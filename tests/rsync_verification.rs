//! Integration tests for transfer engines with inline verification.
//!
//! These tests exercise the complete backup pipeline:
//! - NativeCopy: hashes files during copy, then verifies destination
//! - Rsync: uses --checksum flag for internal verification

use bksd::core::transfer_engine::{
    FileHash, TransferEngineType, TransferRequest, TransferStatus, create_engine,
};
use bksd::core::verifier::verify_from_hashes;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tempfile::tempdir;
use tokio::sync::mpsc;

/// Helper to create test files with specific content
fn create_file(path: &std::path::Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// Helper to create test files with specific permissions
fn create_file_with_mode(path: &std::path::Path, content: &[u8], mode: u32) {
    create_file(path, content);
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms).unwrap();
}

/// Helper to create a FileHash from content
fn make_hash(relative_path: &str, content: &[u8]) -> FileHash {
    let hash = blake3::hash(content);
    FileHash {
        relative_path: PathBuf::from(relative_path),
        hash: *hash.as_bytes(),
        size: content.len() as u64,
    }
}

/// Helper to run transfer and collect progress updates
async fn run_transfer(
    engine_type: TransferEngineType,
    source: &std::path::Path,
    destination: &std::path::Path,
) -> (
    anyhow::Result<bksd::core::transfer_engine::TransferResult>,
    Vec<TransferStatus>,
) {
    let engine = create_engine(engine_type);
    let (tx, mut rx) = mpsc::channel(100);

    let req = TransferRequest {
        job_id: "test-job".to_string(),
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        owner: None,
    };

    let handle = tokio::spawn({
        let req = req.clone();
        async move { engine.transfer(&req, tx).await }
    });

    let mut updates = Vec::new();
    while let Some(status) = rx.recv().await {
        updates.push(status);
    }

    let result = handle.await.unwrap();
    (result, updates)
}

#[tokio::test]
async fn test_native_copy_with_inline_verification() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create test files
    create_file(&source.join("small.txt"), b"hello world");
    create_file(&source.join("medium.bin"), &vec![0xAB; 10 * 1024]); // 10KB

    // Run transfer with NativeCopy engine
    let (transfer_result, transfer_updates) =
        run_transfer(TransferEngineType::NativeCopy, &source, &dest).await;

    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    let result = transfer_result.unwrap();
    assert!(result.total_bytes > 0, "Should have transferred bytes");

    // NativeCopy should return file hashes
    assert!(
        result.file_hashes.is_some(),
        "NativeCopy should return file hashes"
    );
    let hashes = result.file_hashes.unwrap();
    assert_eq!(hashes.len(), 2, "Should have hashes for 2 files");

    // Verify we got progress updates
    assert!(!transfer_updates.is_empty(), "Should have progress updates");
    assert!(
        transfer_updates
            .iter()
            .any(|s| matches!(s, TransferStatus::Ready)),
        "Should have Ready status"
    );

    // Run verification using the hashes from transfer
    let verify_result = verify_from_hashes("test-job", &dest, &hashes).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    assert_eq!(verify.files_verified, 2, "Should verify 2 files");
    assert!(verify.bytes_verified > 0, "Should have verified bytes");
}

#[tokio::test]
async fn test_native_copy_verification_detects_corruption() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create test file
    create_file(
        &source.join("data.txt"),
        b"original content that should match",
    );

    // Run transfer
    let (transfer_result, _) = run_transfer(TransferEngineType::NativeCopy, &source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    let result = transfer_result.unwrap();
    let hashes = result.file_hashes.expect("Should have hashes");

    // CORRUPT the destination file after transfer
    std::fs::write(dest.join("data.txt"), b"corrupted content!!!").unwrap();

    // Run verification - should FAIL
    let verify_result = verify_from_hashes("test-job", &dest, &hashes).await;
    assert!(
        verify_result.is_err(),
        "Verification should fail on corrupted file"
    );

    let err = verify_result.unwrap_err().to_string();
    assert!(
        err.contains("hash mismatch"),
        "Error should mention hash mismatch: {}",
        err
    );
    assert!(
        err.contains("data.txt"),
        "Error should mention the corrupted file: {}",
        err
    );
}

#[tokio::test]
async fn test_rsync_transfer_with_checksum() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create test files
    create_file(&source.join("small.txt"), b"hello world");
    create_file(&source.join("medium.bin"), &vec![0xAB; 10 * 1024]); // 10KB

    // Run transfer with Rsync engine
    let (transfer_result, transfer_updates) =
        run_transfer(TransferEngineType::Rsync, &source, &dest).await;

    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    let result = transfer_result.unwrap();
    assert!(result.total_bytes > 0, "Should have transferred bytes");

    // Rsync should NOT return file hashes (it uses --checksum internally)
    assert!(
        result.file_hashes.is_none(),
        "Rsync should not return file hashes (uses --checksum internally)"
    );

    // Verify we got progress updates
    assert!(!transfer_updates.is_empty(), "Should have progress updates");
    assert!(
        transfer_updates
            .iter()
            .any(|s| matches!(s, TransferStatus::Ready)),
        "Should have Ready status"
    );

    // Verify files exist and have correct content
    assert!(dest.join("small.txt").exists(), "small.txt should exist");
    assert!(dest.join("medium.bin").exists(), "medium.bin should exist");

    let content = std::fs::read(dest.join("small.txt")).unwrap();
    assert_eq!(content, b"hello world");
}

#[tokio::test]
async fn test_native_copy_empty_directory() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();
    // Source is empty - no files

    // Run transfer
    let (transfer_result, _) = run_transfer(TransferEngineType::NativeCopy, &source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    let result = transfer_result.unwrap();
    let hashes = result.file_hashes.expect("Should have hashes (empty vec)");
    assert!(hashes.is_empty(), "Should have no hashes for empty dir");

    // Run verification
    let verify_result = verify_from_hashes("test-job", &dest, &hashes).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    assert_eq!(verify.files_verified, 0, "Should verify 0 files");
    assert_eq!(verify.bytes_verified, 0, "Should have 0 bytes verified");
}

#[tokio::test]
async fn test_native_copy_nested_directories() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create deeply nested structure
    create_file(&source.join("level1.txt"), b"level 1");
    create_file(&source.join("a/level2.txt"), b"level 2");
    create_file(&source.join("a/b/level3.txt"), b"level 3");
    create_file(&source.join("a/b/c/level4.txt"), b"level 4");
    create_file(&source.join("a/b/c/d/level5.txt"), b"level 5");

    // Also create sibling directories
    create_file(&source.join("x/sibling.txt"), b"sibling");
    create_file(&source.join("a/x/nested_sibling.txt"), b"nested sibling");

    // Run transfer
    let (transfer_result, _) = run_transfer(TransferEngineType::NativeCopy, &source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    let result = transfer_result.unwrap();
    let hashes = result.file_hashes.expect("Should have hashes");
    assert_eq!(hashes.len(), 7, "Should have hashes for 7 files");

    // Run verification
    let verify_result = verify_from_hashes("test-job", &dest, &hashes).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    assert_eq!(verify.files_verified, 7, "Should verify 7 files");

    // Verify directory structure was created correctly
    assert!(
        dest.join("a/b/c/d/level5.txt").exists(),
        "Deep nested file should exist"
    );
    assert!(
        dest.join("x/sibling.txt").exists(),
        "Sibling file should exist"
    );
}

#[tokio::test]
async fn test_verify_from_hashes_detects_missing_file() {
    let temp = tempdir().unwrap();
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&dest).unwrap();

    // Create one file but have hashes for two
    std::fs::write(dest.join("exists.txt"), b"I exist").unwrap();

    let hashes = vec![
        make_hash("exists.txt", b"I exist"),
        make_hash("missing.txt", b"I am missing"),
    ];

    let verify_result = verify_from_hashes("test-job", &dest, &hashes).await;
    assert!(
        verify_result.is_err(),
        "Verification should fail for missing file"
    );

    let err = verify_result.unwrap_err().to_string();
    assert!(
        err.contains("missing in destination"),
        "Error should mention missing file: {}",
        err
    );
    assert!(
        err.contains("missing.txt"),
        "Error should name the missing file: {}",
        err
    );
}

#[tokio::test]
async fn test_native_copy_large_file() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create a 1MB file with pattern content
    let large_content: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
    create_file(&source.join("large.bin"), &large_content);

    // Run transfer
    let (transfer_result, _) = run_transfer(TransferEngineType::NativeCopy, &source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    let result = transfer_result.unwrap();
    let hashes = result.file_hashes.expect("Should have hashes");

    // Run verification
    let verify_result = verify_from_hashes("test-job", &dest, &hashes).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    assert_eq!(verify.files_verified, 1);
    assert_eq!(verify.bytes_verified, 1024 * 1024, "Should verify 1MB");
}

#[tokio::test]
async fn test_native_copy_preserves_permissions() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create files with different permissions
    create_file_with_mode(&source.join("normal.txt"), b"normal file", 0o644);
    create_file_with_mode(
        &source.join("executable.sh"),
        b"#!/bin/bash\necho hello",
        0o755,
    );
    create_file_with_mode(&source.join("readonly.txt"), b"read only", 0o444);

    // Run transfer
    let (transfer_result, _) = run_transfer(TransferEngineType::NativeCopy, &source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    // Check permissions were preserved
    let dest_exec = dest.join("executable.sh");
    let exec_mode = std::fs::metadata(&dest_exec).unwrap().permissions().mode();
    assert!(
        exec_mode & 0o100 != 0,
        "Executable should have user execute bit set, got {:o}",
        exec_mode
    );

    let result = transfer_result.unwrap();
    let hashes = result.file_hashes.expect("Should have hashes");

    // Run verification
    let verify_result = verify_from_hashes("test-job", &dest, &hashes).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    assert_eq!(verify.files_verified, 3, "Should verify 3 files");
}
