use crate::core::transfer_engine::{
    TransferEngine, TransferRequest, TransferResult, TransferStatus,
};
use anyhow::{Result, anyhow, bail};
use nix::unistd::{Gid, Group, Uid, User, chown};
use std::fs::{self, File, Permissions};
use std::future::Future;
use std::io::{self, BufReader, BufWriter, ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{Instrument, debug, error, info, info_span, warn};

/// Buffer size for file I/O operations (128KB for optimal throughput)
const BUFFER_SIZE: usize = 128 * 1024;

/// How often to send progress updates (bytes between updates)
const PROGRESS_UPDATE_INTERVAL: u64 = 1024 * 1024; // 1MB

/// Native file copy transfer engine.
///
/// Implements a safe, resilient file transfer with:
/// - Pre-scan for accurate progress reporting
/// - Large buffered I/O for performance
/// - Metadata preservation (permissions, timestamps)
/// - Optional ownership transfer
/// - Graceful handling of device removal
/// - Per-file fsync for durability
pub struct NativeCopyEngine {
    /// Whether to fsync each file after writing (safer but slower)
    pub sync_files: bool,
}

impl Default for NativeCopyEngine {
    fn default() -> Self {
        Self { sync_files: true }
    }
}

impl TransferEngine for NativeCopyEngine {
    fn transfer(
        &self,
        req: &TransferRequest,
        tx: mpsc::Sender<TransferStatus>,
    ) -> Pin<Box<dyn Future<Output = Result<TransferResult>> + Send>> {
        let req = req.clone();
        let sync_files = self.sync_files;

        Box::pin(async move {
            let _ = tx.send(TransferStatus::Ready).await;

            let source = req.source.clone();
            let destination = req.destination.clone();
            let owner = req.owner.clone();

            // Safety check: fail if destination already exists to prevent overwrites
            if destination.exists() {
                let msg = format!(
                    "Destination already exists: {}. Refusing to overwrite.",
                    destination.display()
                );
                let _ = tx.send(TransferStatus::Failed(msg.clone())).await;
                return Err(anyhow!(msg));
            }

            // Create destination directory
            if let Err(e) = fs::create_dir_all(&destination) {
                let msg = format!("Failed to create destination directory: {}", e);
                let _ = tx.send(TransferStatus::Failed(msg.clone())).await;
                return Err(anyhow!(msg));
            }

            let span = info_span!(
                "native_copy_transfer",
                source = %source.display(),
                destination = %destination.display()
            );

            async {
                info!("Starting native copy transfer");
                let start_time = Instant::now();

                // Phase 1: Scan source directory for files and total size
                info!("Scanning source directory");
                let scan_result = match scan_directory(&source).await {
                    Ok(result) => result,
                    Err(e) => {
                        let msg = format!("Failed to scan source directory: {}", e);
                        let _ = tx.send(TransferStatus::Failed(msg.clone())).await;
                        return Err(anyhow!(msg));
                    }
                };

                info!(
                    total_files = scan_result.files.len(),
                    total_bytes = scan_result.total_bytes,
                    total_dirs = scan_result.directories.len(),
                    "Scan complete"
                );

                // Resolve owner UID/GID if specified
                let owner_ids = match &owner {
                    Some(o) => match resolve_owner(o) {
                        Ok(ids) => Some(ids),
                        Err(e) => {
                            warn!(error = %e, "Failed to resolve owner, files will be owned by process user");
                            None
                        }
                    },
                    None => None,
                };

                // Phase 2: Create directory structure
                if let Err(e) = create_directory_structure(
                    &source,
                    &destination,
                    &scan_result.directories,
                    owner_ids.as_ref(),
                )
                .await
                {
                    let msg = format!("Failed to create directory structure: {}", e);
                    let _ = tx.send(TransferStatus::Failed(msg.clone())).await;
                    return Err(anyhow!(msg));
                }

                // Phase 3: Copy files with progress reporting
                let copy_options = CopyOptions {
                    sync_files,
                    owner_ids,
                };

                let result = copy_files_with_progress(
                    &source,
                    &destination,
                    &scan_result.files,
                    scan_result.total_bytes,
                    &copy_options,
                    tx.clone(),
                )
                .await;

                match result {
                    Ok(bytes_copied) => {
                        let duration_secs = start_time.elapsed().as_secs();
                        let speed_mbps = if duration_secs > 0 {
                            bytes_copied as f64 / (1024.0 * 1024.0) / duration_secs as f64
                        } else {
                            0.0
                        };

                        info!(
                            total_bytes = bytes_copied,
                            duration_secs = duration_secs,
                            speed_mbps = format!("{:.2}", speed_mbps),
                            "Native copy transfer complete"
                        );

                        Ok(TransferResult {
                            total_bytes: bytes_copied,
                            duration_secs,
                        })
                    }
                    Err(e) => {
                        let msg = format!("Transfer failed: {}", e);
                        let _ = tx.send(TransferStatus::Failed(msg.clone())).await;
                        Err(anyhow!(msg))
                    }
                }
            }
            .instrument(span)
            .await
        })
    }
}

/// Result of scanning a directory
struct ScanResult {
    /// All files found (absolute paths)
    files: Vec<FileInfo>,
    /// All directories found (absolute paths), in creation order (parents before children)
    directories: Vec<PathBuf>,
    /// Total size of all files in bytes
    total_bytes: u64,
}

/// Information about a file to copy
#[derive(Clone)]
struct FileInfo {
    /// Absolute path to the file
    path: PathBuf,
    /// File size in bytes
    size: u64,
}

/// Resolved owner UID and GID
#[derive(Clone)]
struct OwnerIds {
    uid: Uid,
    gid: Gid,
}

/// Options for the copy operation
struct CopyOptions {
    /// Whether to fsync each file after writing
    sync_files: bool,
    /// Owner UID/GID if ownership should be changed
    owner_ids: Option<OwnerIds>,
}

/// Scan a directory recursively, collecting files and directories.
async fn scan_directory(source: &Path) -> Result<ScanResult> {
    let source = source.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut total_bytes: u64 = 0;

        scan_directory_recursive(
            &source,
            &source,
            &mut files,
            &mut directories,
            &mut total_bytes,
        )?;

        Ok(ScanResult {
            files,
            directories,
            total_bytes,
        })
    })
    .await?
}

fn scan_directory_recursive(
    base: &Path,
    current: &Path,
    files: &mut Vec<FileInfo>,
    directories: &mut Vec<PathBuf>,
    total_bytes: &mut u64,
) -> Result<()> {
    let entries = fs::read_dir(current).map_err(|e| {
        if is_device_removed_error(&e) {
            anyhow!("Device appears to have been removed: {}", e)
        } else {
            anyhow!("Failed to read directory {}: {}", current.display(), e)
        }
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            if is_device_removed_error(&e) {
                anyhow!("Device appears to have been removed: {}", e)
            } else {
                anyhow!("Failed to read directory entry: {}", e)
            }
        })?;

        let path = entry.path();

        // Use symlink_metadata to avoid following symlinks
        let metadata = match path.symlink_metadata() {
            Ok(m) => m,
            Err(e) => {
                if is_device_removed_error(&e) {
                    bail!("Device appears to have been removed: {}", e);
                }
                warn!(path = %path.display(), error = %e, "Skipping unreadable entry");
                continue;
            }
        };

        if metadata.is_dir() {
            directories.push(path.clone());
            scan_directory_recursive(base, &path, files, directories, total_bytes)?;
        } else if metadata.is_file() {
            let size = metadata.len();
            *total_bytes += size;
            files.push(FileInfo { path, size });
        }
        // Skip symlinks and other special files
    }

    Ok(())
}

/// Resolve a FileOwner to UID/GID
fn resolve_owner(owner: &crate::core::ownership::FileOwner) -> Result<OwnerIds> {
    let user = User::from_name(&owner.user)
        .map_err(|e| anyhow!("Failed to lookup user '{}': {}", owner.user, e))?
        .ok_or_else(|| anyhow!("User '{}' not found", owner.user))?;

    let group = Group::from_name(&owner.group)
        .map_err(|e| anyhow!("Failed to lookup group '{}': {}", owner.group, e))?
        .ok_or_else(|| anyhow!("Group '{}' not found", owner.group))?;

    Ok(OwnerIds {
        uid: user.uid,
        gid: group.gid,
    })
}

/// Create all directories in the destination, preserving structure and permissions
async fn create_directory_structure(
    source: &Path,
    destination: &Path,
    directories: &[PathBuf],
    owner_ids: Option<&OwnerIds>,
) -> Result<()> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    let directories = directories.to_vec();
    let owner_ids = owner_ids.cloned();

    tokio::task::spawn_blocking(move || {
        for dir_path in &directories {
            let relative = dir_path
                .strip_prefix(&source)
                .expect("directory should be under source");
            let dest_dir = destination.join(relative);

            // Get source directory metadata for permissions
            let metadata = fs::metadata(dir_path)?;
            let permissions = metadata.permissions();

            // Create directory
            fs::create_dir_all(&dest_dir)?;

            // Set permissions
            fs::set_permissions(&dest_dir, permissions)?;

            // Set ownership if specified
            if let Some(ref ids) = owner_ids {
                if let Err(e) = chown(&dest_dir, Some(ids.uid), Some(ids.gid)) {
                    warn!(
                        path = %dest_dir.display(),
                        error = %e,
                        "Failed to set directory ownership"
                    );
                }
            }
        }
        Ok(())
    })
    .await?
}

/// Copy all files with progress reporting
async fn copy_files_with_progress(
    source: &Path,
    destination: &Path,
    files: &[FileInfo],
    total_bytes: u64,
    options: &CopyOptions,
    tx: mpsc::Sender<TransferStatus>,
) -> Result<u64> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    let files = files.to_vec();
    let sync_files = options.sync_files;
    let owner_ids = options.owner_ids.clone();

    tokio::task::spawn_blocking(move || {
        let mut bytes_copied: u64 = 0;
        let mut last_progress_update: u64 = 0;
        let mut errors: Vec<CopyError> = Vec::new();

        for file_info in &files {
            let relative = file_info
                .path
                .strip_prefix(&source)
                .expect("file should be under source");
            let dest_path = destination.join(relative);
            let current_file = relative.to_string_lossy().to_string();

            debug!(file = %current_file, size = file_info.size, "Copying file");

            match copy_single_file(&file_info.path, &dest_path, sync_files, owner_ids.as_ref()) {
                Ok(file_bytes) => {
                    bytes_copied += file_bytes;

                    // Send progress update if enough bytes have been copied
                    if bytes_copied - last_progress_update >= PROGRESS_UPDATE_INTERVAL
                        || bytes_copied == total_bytes
                    {
                        let percentage = if total_bytes > 0 {
                            ((bytes_copied as f64 / total_bytes as f64) * 100.0) as u8
                        } else {
                            100
                        };

                        let _ = tx.blocking_send(TransferStatus::InProgress {
                            total_bytes,
                            bytes_copied,
                            current_file: current_file.clone(),
                            percentage,
                        });

                        last_progress_update = bytes_copied;
                    }
                }
                Err(e) => {
                    // Check if this is a device removal error - if so, fail immediately
                    if e.is_device_removed {
                        return Err(anyhow!(
                            "Device removed during transfer at file: {}",
                            current_file
                        ));
                    }

                    error!(
                        file = %current_file,
                        error = %e.message,
                        "Failed to copy file"
                    );

                    errors.push(CopyError {
                        file: current_file,
                        message: e.message,
                    });
                }
            }
        }

        // Report any non-fatal errors
        if !errors.is_empty() {
            let error_summary = format!(
                "Transfer completed with {} error(s):\n{}",
                errors.len(),
                errors
                    .iter()
                    .take(10)
                    .map(|e| format!("  - {}: {}", e.file, e.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            );

            if errors.len() > 10 {
                return Err(anyhow!(
                    "{}\n  ... and {} more errors",
                    error_summary,
                    errors.len() - 10
                ));
            }
            return Err(anyhow!(error_summary));
        }

        Ok(bytes_copied)
    })
    .await?
}

/// Error information from a file copy operation
struct FileCopyError {
    message: String,
    is_device_removed: bool,
}

/// Error tracking for copy operations
struct CopyError {
    file: String,
    message: String,
}

/// Copy a single file with metadata preservation
fn copy_single_file(
    source: &Path,
    dest: &Path,
    sync_file: bool,
    owner_ids: Option<&OwnerIds>,
) -> Result<u64, FileCopyError> {
    // Read source file metadata first
    let source_metadata = fs::metadata(source).map_err(|e| FileCopyError {
        message: format!("Failed to read source metadata: {}", e),
        is_device_removed: is_device_removed_error(&e),
    })?;

    // Open source file
    let source_file = File::open(source).map_err(|e| FileCopyError {
        message: format!("Failed to open source file: {}", e),
        is_device_removed: is_device_removed_error(&e),
    })?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, source_file);

    // Create destination file
    let dest_file = File::create(dest).map_err(|e| FileCopyError {
        message: format!("Failed to create destination file: {}", e),
        is_device_removed: is_device_removed_error(&e),
    })?;
    let mut writer = BufWriter::with_capacity(BUFFER_SIZE, dest_file);

    // Copy data in chunks
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut bytes_written: u64 = 0;

    loop {
        let bytes_read = reader.read(&mut buffer).map_err(|e| FileCopyError {
            message: format!("Failed to read from source: {}", e),
            is_device_removed: is_device_removed_error(&e),
        })?;

        if bytes_read == 0 {
            break;
        }

        writer
            .write_all(&buffer[..bytes_read])
            .map_err(|e| FileCopyError {
                message: format!("Failed to write to destination: {}", e),
                is_device_removed: is_device_removed_error(&e),
            })?;

        bytes_written += bytes_read as u64;
    }

    // Flush and optionally sync
    writer.flush().map_err(|e| FileCopyError {
        message: format!("Failed to flush destination file: {}", e),
        is_device_removed: is_device_removed_error(&e),
    })?;

    if sync_file {
        let inner = writer.into_inner().map_err(|e| FileCopyError {
            message: format!("Failed to get inner file handle: {}", e.error()),
            is_device_removed: is_device_removed_error(&e.error()),
        })?;

        inner.sync_all().map_err(|e| FileCopyError {
            message: format!("Failed to sync file: {}", e),
            is_device_removed: is_device_removed_error(&e),
        })?;
    }

    // Preserve permissions
    let permissions = source_metadata.permissions();
    if let Err(e) = fs::set_permissions(dest, permissions) {
        // Log but don't fail - permission errors might happen on some filesystems
        debug!(
            dest = %dest.display(),
            error = %e,
            "Failed to set file permissions"
        );
    }

    // Preserve timestamps
    if let Err(e) = preserve_timestamps(source, dest) {
        debug!(
            dest = %dest.display(),
            error = %e,
            "Failed to preserve file timestamps"
        );
    }

    // Set ownership if specified
    if let Some(ids) = owner_ids {
        if let Err(e) = chown(dest, Some(ids.uid), Some(ids.gid)) {
            debug!(
                dest = %dest.display(),
                error = %e,
                "Failed to set file ownership"
            );
        }
    }

    Ok(bytes_written)
}

/// Preserve access and modification timestamps from source to destination
fn preserve_timestamps(source: &Path, dest: &Path) -> Result<()> {
    let source_metadata = fs::metadata(source)?;

    // Get timestamps (Unix)
    let atime = filetime::FileTime::from_last_access_time(&source_metadata);
    let mtime = filetime::FileTime::from_last_modification_time(&source_metadata);

    filetime::set_file_times(dest, atime, mtime)?;
    Ok(())
}

/// Check if an I/O error indicates the device has been removed
fn is_device_removed_error(error: &io::Error) -> bool {
    match error.kind() {
        // Common error kinds when device is removed
        ErrorKind::NotFound => true,
        ErrorKind::PermissionDenied => false, // Usually not device removal
        ErrorKind::BrokenPipe => true,
        ErrorKind::ConnectionReset => true,
        ErrorKind::ConnectionAborted => true,
        ErrorKind::NotConnected => true,
        _ => {
            // Check for specific errno values that indicate device issues
            if let Some(os_error) = error.raw_os_error() {
                matches!(
                    os_error,
                    libc::EIO       // I/O error
                    | libc::ENODEV  // No such device
                    | libc::ENXIO   // No such device or address
                    | libc::ENOMEDIUM // No medium found
                    | libc::EMEDIUMTYPE // Wrong medium type
                )
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_is_device_removed_error_eio() {
        let error = io::Error::from_raw_os_error(libc::EIO);
        assert!(is_device_removed_error(&error));
    }

    #[test]
    fn test_is_device_removed_error_enodev() {
        let error = io::Error::from_raw_os_error(libc::ENODEV);
        assert!(is_device_removed_error(&error));
    }

    #[test]
    fn test_is_device_removed_error_not_found() {
        let error = io::Error::new(ErrorKind::NotFound, "not found");
        assert!(is_device_removed_error(&error));
    }

    #[test]
    fn test_is_device_removed_error_permission_denied() {
        let error = io::Error::new(ErrorKind::PermissionDenied, "permission denied");
        assert!(!is_device_removed_error(&error));
    }

    #[tokio::test]
    async fn test_scan_empty_directory() {
        let temp = tempdir().unwrap();
        let result = scan_directory(temp.path()).await.unwrap();

        assert!(result.files.is_empty());
        assert!(result.directories.is_empty());
        assert_eq!(result.total_bytes, 0);
    }

    #[tokio::test]
    async fn test_scan_with_files() {
        let temp = tempdir().unwrap();

        // Create some files
        fs::write(temp.path().join("file1.txt"), b"hello").unwrap();
        fs::write(temp.path().join("file2.txt"), b"world!!!").unwrap();
        fs::create_dir(temp.path().join("subdir")).unwrap();
        fs::write(temp.path().join("subdir/nested.txt"), b"nested").unwrap();

        let result = scan_directory(temp.path()).await.unwrap();

        assert_eq!(result.files.len(), 3);
        assert_eq!(result.directories.len(), 1);
        assert_eq!(result.total_bytes, 5 + 8 + 6); // hello + world!!! + nested
    }

    #[tokio::test]
    async fn test_native_copy_engine() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        // Create source files
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file1.txt"), b"hello world").unwrap();
        fs::create_dir(source.join("subdir")).unwrap();
        fs::write(source.join("subdir/file2.txt"), b"nested content").unwrap();

        // Set specific permissions
        fs::set_permissions(source.join("file1.txt"), Permissions::from_mode(0o644)).unwrap();

        let engine = NativeCopyEngine::default();
        let (tx, mut rx) = mpsc::channel(100);

        let req = TransferRequest {
            job_id: "test-job".to_string(),
            source: source.clone(),
            destination: dest.clone(),
            owner: None,
        };

        let handle = tokio::spawn(async move { engine.transfer(&req, tx).await.await });

        // Collect progress updates
        let mut updates = Vec::new();
        while let Some(status) = rx.recv().await {
            updates.push(status);
        }

        let result = handle.await.unwrap();
        assert!(result.is_ok());

        let transfer_result = result.unwrap();
        assert_eq!(transfer_result.total_bytes, 11 + 14); // hello world + nested content

        // Verify files were copied
        assert!(dest.join("file1.txt").exists());
        assert!(dest.join("subdir/file2.txt").exists());

        // Verify content
        let content1 = fs::read_to_string(dest.join("file1.txt")).unwrap();
        assert_eq!(content1, "hello world");

        let content2 = fs::read_to_string(dest.join("subdir/file2.txt")).unwrap();
        assert_eq!(content2, "nested content");

        // Should have received Ready status
        assert!(matches!(updates.first(), Some(TransferStatus::Ready)));
    }

    #[tokio::test]
    async fn test_native_copy_refuses_existing_destination() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        fs::create_dir(&source).unwrap();
        fs::create_dir(&dest).unwrap(); // Pre-create destination

        let engine = NativeCopyEngine::default();
        let (tx, _rx) = mpsc::channel(100);

        let req = TransferRequest {
            job_id: "test-job".to_string(),
            source,
            destination: dest,
            owner: None,
        };

        let result = engine.transfer(&req, tx).await.await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_copy_single_file_preserves_content() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.txt");
        let dest = temp.path().join("dest.txt");

        let content = b"test file content for copying";
        fs::write(&source, content).unwrap();

        let result = copy_single_file(&source, &dest, true, None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), content.len() as u64);

        let copied_content = fs::read(&dest).unwrap();
        assert_eq!(copied_content, content);
    }
}
