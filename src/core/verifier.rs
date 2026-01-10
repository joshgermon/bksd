use anyhow::{Result, bail};
use std::io::Read;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::core::transfer_engine::TransferStatus;

/// Request to verify a completed transfer
pub struct VerifyRequest {
    pub job_id: String,
    pub source: PathBuf,
    pub destination: PathBuf,
}

/// Result of a successful verification
#[derive(Debug)]
pub struct VerifyResult {
    pub files_verified: u64,
    pub bytes_verified: u64,
}

/// Details of a file that failed verification
#[derive(Debug)]
pub struct FileMismatch {
    pub relative_path: PathBuf,
    pub reason: MismatchReason,
}

/// Reason a file failed verification
#[derive(Debug)]
pub enum MismatchReason {
    /// BLAKE3 hash of source and destination differ
    HashMismatch,
    /// File exists in source but not in destination
    MissingInDestination,
}

/// Verify all files in source exist in destination with matching BLAKE3 checksums.
///
/// Processes files sequentially to avoid overwhelming slow storage devices.
/// Collects all mismatches before returning an error.
///
/// Sends `TransferStatus::Verifying { current, total }` progress updates.
pub async fn verify_transfer(
    req: &VerifyRequest,
    tx: mpsc::Sender<TransferStatus>,
) -> Result<VerifyResult> {
    // Collect all files in source directory
    let files = collect_files(&req.source).await?;
    let total = files.len() as u64;

    info!(job_id = %req.job_id, total_files = total, "Starting verification");

    if total == 0 {
        debug!(job_id = %req.job_id, "No files to verify");
        return Ok(VerifyResult {
            files_verified: 0,
            bytes_verified: 0,
        });
    }

    let _ = tx
        .send(TransferStatus::Verifying { current: 0, total })
        .await;

    let mut mismatches: Vec<FileMismatch> = Vec::new();
    let mut bytes_verified: u64 = 0;

    for (i, source_path) in files.iter().enumerate() {
        let relative = source_path
            .strip_prefix(&req.source)
            .expect("file should be under source directory");
        let dest_path = req.destination.join(relative);

        debug!(file = %relative.display(), "Verifying file");

        if !dest_path.exists() {
            mismatches.push(FileMismatch {
                relative_path: relative.to_path_buf(),
                reason: MismatchReason::MissingInDestination,
            });

            // Still send progress update
            let current = (i + 1) as u64;
            let _ = tx.send(TransferStatus::Verifying { current, total }).await;
            continue;
        }

        let source_path_clone = source_path.clone();
        let dest_path_clone = dest_path.clone();

        let (source_hash, dest_hash) =
            tokio::try_join!(hash_file(&source_path_clone), hash_file(&dest_path_clone),)?;

        if source_hash != dest_hash {
            mismatches.push(FileMismatch {
                relative_path: relative.to_path_buf(),
                reason: MismatchReason::HashMismatch,
            });
        } else {
            // Only count bytes for successfully verified files
            if let Ok(metadata) = std::fs::metadata(source_path) {
                bytes_verified += metadata.len();
            }
        }

        // Send progress update
        let current = (i + 1) as u64;
        let _ = tx.send(TransferStatus::Verifying { current, total }).await;
    }

    // Report results
    if !mismatches.is_empty() {
        let error_msg = format_mismatch_error(&mismatches);
        info!(
            job_id = %req.job_id,
            mismatches = mismatches.len(),
            "Verification failed"
        );
        bail!(error_msg);
    }

    info!(
        job_id = %req.job_id,
        files_verified = total,
        bytes_verified = bytes_verified,
        "Verification complete"
    );

    Ok(VerifyResult {
        files_verified: total,
        bytes_verified,
    })
}

/// Collect all file paths under a directory (recursive)
async fn collect_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        collect_files_recursive(&dir, &mut files)?;
        Ok(files)
    })
    .await?
}

fn collect_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            // Directory might not exist or be readable
            bail!("Failed to read directory {}: {}", dir.display(), e);
        }
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // Use symlink_metadata to avoid following symlinks
        let metadata = match path.symlink_metadata() {
            Ok(m) => m,
            Err(_) => continue, // Skip entries we can't read
        };

        if metadata.is_dir() {
            collect_files_recursive(&path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
        // Skip symlinks and other special files
    }

    Ok(())
}

/// Hash a file using BLAKE3, streaming in chunks to handle large files
async fn hash_file(path: &Path) -> Result<blake3::Hash> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path)
            .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", path.display(), e))?;

        let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
        let mut hasher = blake3::Hasher::new();

        let mut buffer = [0u8; 64 * 1024];
        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        Ok(hasher.finalize())
    })
    .await?
}

/// Format mismatch errors into a human-readable message
fn format_mismatch_error(mismatches: &[FileMismatch]) -> String {
    let mut msg = format!(
        "Verification failed: {} file(s) did not match",
        mismatches.len()
    );

    // Show details for first 10 mismatches
    for m in mismatches.iter().take(10) {
        let reason = match &m.reason {
            MismatchReason::HashMismatch => "hash mismatch",
            MismatchReason::MissingInDestination => "missing in destination",
        };
        msg.push_str(&format!("\n  - {}: {}", m.relative_path.display(), reason));
    }

    if mismatches.len() > 10 {
        msg.push_str(&format!("\n  ... and {} more", mismatches.len() - 10));
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_verify_success() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        // Create matching files
        std::fs::write(source.join("file1.txt"), b"hello world").unwrap();
        std::fs::write(dest.join("file1.txt"), b"hello world").unwrap();

        std::fs::create_dir_all(source.join("subdir")).unwrap();
        std::fs::create_dir_all(dest.join("subdir")).unwrap();
        std::fs::write(source.join("subdir/nested.txt"), b"nested content").unwrap();
        std::fs::write(dest.join("subdir/nested.txt"), b"nested content").unwrap();

        let (tx, mut rx) = mpsc::channel(10);
        let req = VerifyRequest {
            job_id: "test-job".to_string(),
            source,
            destination: dest,
        };

        let handle = tokio::spawn(async move { verify_transfer(&req, tx).await });

        // Collect progress updates
        let mut updates = Vec::new();
        while let Some(status) = rx.recv().await {
            updates.push(status);
        }

        let result = handle.await.unwrap();
        assert!(result.is_ok());

        let verify_result = result.unwrap();
        assert_eq!(verify_result.files_verified, 2);
        assert!(verify_result.bytes_verified > 0);

        // Should have progress updates
        assert!(!updates.is_empty());
    }

    #[tokio::test]
    async fn test_verify_hash_mismatch() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        // Create files with different content
        std::fs::write(source.join("file.txt"), b"original content").unwrap();
        std::fs::write(dest.join("file.txt"), b"corrupted content").unwrap();

        let (tx, _rx) = mpsc::channel(10);
        let req = VerifyRequest {
            job_id: "test-job".to_string(),
            source,
            destination: dest,
        };

        let result = verify_transfer(&req, tx).await;
        assert!(result.is_err());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("hash mismatch"));
        assert!(err.contains("file.txt"));
    }

    #[tokio::test]
    async fn test_verify_missing_file() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        // Create file in source only
        std::fs::write(source.join("missing.txt"), b"this file is missing").unwrap();

        let (tx, _rx) = mpsc::channel(10);
        let req = VerifyRequest {
            job_id: "test-job".to_string(),
            source,
            destination: dest,
        };

        let result = verify_transfer(&req, tx).await;
        assert!(result.is_err());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing in destination"));
        assert!(err.contains("missing.txt"));
    }

    #[tokio::test]
    async fn test_verify_collects_all_mismatches() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        // Create multiple mismatches
        std::fs::write(source.join("a.txt"), b"content a").unwrap();
        std::fs::write(dest.join("a.txt"), b"wrong a").unwrap();

        std::fs::write(source.join("b.txt"), b"content b").unwrap();
        // b.txt missing in dest

        std::fs::write(source.join("c.txt"), b"content c").unwrap();
        std::fs::write(dest.join("c.txt"), b"wrong c").unwrap();

        let (tx, _rx) = mpsc::channel(10);
        let req = VerifyRequest {
            job_id: "test-job".to_string(),
            source,
            destination: dest,
        };

        let result = verify_transfer(&req, tx).await;
        assert!(result.is_err());

        let err = result.unwrap_err().to_string();
        // Should report all 3 mismatches
        assert!(err.contains("3 file(s) did not match"));
    }

    #[tokio::test]
    async fn test_verify_empty_directory() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        let (tx, _rx) = mpsc::channel(10);
        let req = VerifyRequest {
            job_id: "test-job".to_string(),
            source,
            destination: dest,
        };

        let result = verify_transfer(&req, tx).await;
        assert!(result.is_ok());

        let verify_result = result.unwrap();
        assert_eq!(verify_result.files_verified, 0);
        assert_eq!(verify_result.bytes_verified, 0);
    }
}
