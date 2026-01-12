use anyhow::{Result, bail};
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use crate::core::transfer_engine::FileHash;

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

/// Verify destination files against pre-computed hashes from the transfer.
///
/// This is the fast verification path - it only reads destination files
/// since source files were already hashed during the copy operation.
///
/// Returns Ok if all files match, Err with details if any mismatches found.
pub async fn verify_from_hashes(
    job_id: &str,
    destination: &Path,
    file_hashes: &[FileHash],
) -> Result<VerifyResult> {
    let total = file_hashes.len() as u64;

    info!(job_id = %job_id, total_files = total, "Starting hash verification");

    if total == 0 {
        debug!(job_id = %job_id, "No files to verify");
        return Ok(VerifyResult {
            files_verified: 0,
            bytes_verified: 0,
        });
    }

    let destination = destination.to_path_buf();
    let file_hashes = file_hashes.to_vec();
    let job_id = job_id.to_string();

    // Run verification in a blocking task since it's I/O heavy
    tokio::task::spawn_blocking(move || {
        let mut mismatches: Vec<FileMismatch> = Vec::new();
        let mut bytes_verified: u64 = 0;

        for fh in &file_hashes {
            let dest_path = destination.join(&fh.relative_path);

            debug!(file = %fh.relative_path.display(), "Verifying file");

            if !dest_path.exists() {
                mismatches.push(FileMismatch {
                    relative_path: fh.relative_path.clone(),
                    reason: MismatchReason::MissingInDestination,
                });
                continue;
            }

            // Hash the destination file
            match hash_file_sync(&dest_path) {
                Ok(dest_hash) => {
                    if dest_hash.as_bytes() != &fh.hash {
                        mismatches.push(FileMismatch {
                            relative_path: fh.relative_path.clone(),
                            reason: MismatchReason::HashMismatch,
                        });
                    } else {
                        bytes_verified += fh.size;
                    }
                }
                Err(e) => {
                    debug!(
                        file = %fh.relative_path.display(),
                        error = %e,
                        "Failed to hash destination file"
                    );
                    mismatches.push(FileMismatch {
                        relative_path: fh.relative_path.clone(),
                        reason: MismatchReason::HashMismatch,
                    });
                }
            }
        }

        // Report results
        if !mismatches.is_empty() {
            let error_msg = format_mismatch_error(&mismatches);
            info!(
                job_id = %job_id,
                mismatches = mismatches.len(),
                "Verification failed"
            );
            bail!(error_msg);
        }

        info!(
            job_id = %job_id,
            files_verified = total,
            bytes_verified = bytes_verified,
            "Verification complete"
        );

        Ok(VerifyResult {
            files_verified: total,
            bytes_verified,
        })
    })
    .await?
}

/// Hash a file using BLAKE3, streaming in chunks to handle large files (sync version)
fn hash_file_sync(path: &Path) -> Result<blake3::Hash> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", path.display(), e))?;

    let mut reader = std::io::BufReader::with_capacity(128 * 1024, file);
    let mut hasher = blake3::Hasher::new();

    let mut buffer = [0u8; 128 * 1024];
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize())
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

    /// Helper to create a FileHash from content
    fn make_hash(relative_path: &str, content: &[u8]) -> FileHash {
        let hash = blake3::hash(content);
        FileHash {
            relative_path: PathBuf::from(relative_path),
            hash: *hash.as_bytes(),
            size: content.len() as u64,
        }
    }

    #[tokio::test]
    async fn test_verify_from_hashes_success() {
        let temp = tempdir().unwrap();
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&dest).unwrap();
        std::fs::create_dir_all(dest.join("subdir")).unwrap();

        // Create destination files
        std::fs::write(dest.join("file1.txt"), b"hello world").unwrap();
        std::fs::write(dest.join("subdir/nested.txt"), b"nested content").unwrap();

        // Create hashes that match the destination content
        let file_hashes = vec![
            make_hash("file1.txt", b"hello world"),
            make_hash("subdir/nested.txt", b"nested content"),
        ];

        let result = verify_from_hashes("test-job", &dest, &file_hashes).await;
        assert!(result.is_ok());

        let verify_result = result.unwrap();
        assert_eq!(verify_result.files_verified, 2);
        assert!(verify_result.bytes_verified > 0);
    }

    #[tokio::test]
    async fn test_verify_from_hashes_mismatch() {
        let temp = tempdir().unwrap();
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&dest).unwrap();

        // Create destination file with different content than expected
        std::fs::write(dest.join("file.txt"), b"corrupted content").unwrap();

        // Hash is for "original content" but file contains "corrupted content"
        let file_hashes = vec![make_hash("file.txt", b"original content")];

        let result = verify_from_hashes("test-job", &dest, &file_hashes).await;
        assert!(result.is_err());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("hash mismatch"));
        assert!(err.contains("file.txt"));
    }

    #[tokio::test]
    async fn test_verify_from_hashes_missing_file() {
        let temp = tempdir().unwrap();
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&dest).unwrap();

        // Hash for a file that doesn't exist in destination
        let file_hashes = vec![make_hash("missing.txt", b"this file is missing")];

        let result = verify_from_hashes("test-job", &dest, &file_hashes).await;
        assert!(result.is_err());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing in destination"));
        assert!(err.contains("missing.txt"));
    }

    #[tokio::test]
    async fn test_verify_from_hashes_collects_all_mismatches() {
        let temp = tempdir().unwrap();
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&dest).unwrap();

        // Create some files with wrong content
        std::fs::write(dest.join("a.txt"), b"wrong a").unwrap();
        // b.txt is missing
        std::fs::write(dest.join("c.txt"), b"wrong c").unwrap();

        let file_hashes = vec![
            make_hash("a.txt", b"content a"),
            make_hash("b.txt", b"content b"),
            make_hash("c.txt", b"content c"),
        ];

        let result = verify_from_hashes("test-job", &dest, &file_hashes).await;
        assert!(result.is_err());

        let err = result.unwrap_err().to_string();
        // Should report all 3 mismatches
        assert!(err.contains("3 file(s) did not match"));
    }

    #[tokio::test]
    async fn test_verify_from_hashes_empty() {
        let temp = tempdir().unwrap();
        let dest = temp.path().join("dest");

        std::fs::create_dir_all(&dest).unwrap();

        let file_hashes: Vec<FileHash> = vec![];

        let result = verify_from_hashes("test-job", &dest, &file_hashes).await;
        assert!(result.is_ok());

        let verify_result = result.unwrap();
        assert_eq!(verify_result.files_verified, 0);
        assert_eq!(verify_result.bytes_verified, 0);
    }
}
