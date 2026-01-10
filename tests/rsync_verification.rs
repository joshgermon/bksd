//! Integration tests for rsync transfer engine with BLAKE3 verification.
//!
//! These tests exercise the complete backup pipeline:
//! 1. rsync copies files from source to destination
//! 2. verifier confirms all files match via BLAKE3 checksums

use bksd::core::transfer_engine::{
    TransferEngineType, TransferRequest, TransferStatus, create_engine,
};
use bksd::core::verifier::{VerifyRequest, verify_transfer};
use std::os::unix::fs::PermissionsExt;
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

/// Helper to run rsync transfer and collect progress updates
async fn run_transfer(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> (
    anyhow::Result<bksd::core::transfer_engine::TransferResult>,
    Vec<TransferStatus>,
) {
    let engine = create_engine(TransferEngineType::Rsync);
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

/// Helper to run verification and collect progress updates
async fn run_verification(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> (
    anyhow::Result<bksd::core::verifier::VerifyResult>,
    Vec<TransferStatus>,
) {
    let (tx, mut rx) = mpsc::channel(100);

    let req = VerifyRequest {
        job_id: "test-job".to_string(),
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
    };

    let handle = tokio::spawn(async move { verify_transfer(&req, tx).await });

    let mut updates = Vec::new();
    while let Some(status) = rx.recv().await {
        updates.push(status);
    }

    let result = handle.await.unwrap();
    (result, updates)
}

#[tokio::test]
async fn test_rsync_transfer_and_verification_success() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create test files
    create_file(&source.join("small.txt"), b"hello world");
    create_file(&source.join("medium.bin"), &vec![0xAB; 10 * 1024]); // 10KB

    // Run transfer
    let (transfer_result, transfer_updates) = run_transfer(&source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    let result = transfer_result.unwrap();
    assert!(result.total_bytes > 0, "Should have transferred bytes");

    // Verify we got progress updates
    assert!(!transfer_updates.is_empty(), "Should have progress updates");
    assert!(
        transfer_updates
            .iter()
            .any(|s| matches!(s, TransferStatus::Ready)),
        "Should have Ready status"
    );

    // Run verification
    let (verify_result, verify_updates) = run_verification(&source, &dest).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    assert_eq!(verify.files_verified, 2, "Should verify 2 files");
    assert!(verify.bytes_verified > 0, "Should have verified bytes");

    // Verify we got verification progress updates
    assert!(
        verify_updates
            .iter()
            .any(|s| matches!(s, TransferStatus::Verifying { .. })),
        "Should have Verifying status updates"
    );
}

#[tokio::test]
async fn test_rsync_transfer_verification_detects_corruption() {
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
    let (transfer_result, _) = run_transfer(&source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    // CORRUPT the destination file
    std::fs::write(dest.join("data.txt"), b"corrupted content!!!").unwrap();

    // Run verification - should FAIL
    let (verify_result, _) = run_verification(&source, &dest).await;
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
async fn test_rsync_transfer_empty_directory() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();
    // Source is empty - no files

    // Run transfer
    let (transfer_result, _) = run_transfer(&source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    // Run verification
    let (verify_result, _) = run_verification(&source, &dest).await;
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
async fn test_rsync_transfer_nested_directories() {
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
    let (transfer_result, _) = run_transfer(&source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    // Run verification
    let (verify_result, _) = run_verification(&source, &dest).await;
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
async fn test_rsync_transfer_symlinks() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create a regular file and a symlink to it
    create_file(&source.join("target.txt"), b"I am the target file");
    std::os::unix::fs::symlink("target.txt", source.join("link.txt")).unwrap();

    // Create a directory symlink
    std::fs::create_dir_all(source.join("real_dir")).unwrap();
    create_file(&source.join("real_dir/inside.txt"), b"inside directory");
    std::os::unix::fs::symlink("real_dir", source.join("dir_link")).unwrap();

    // Run transfer
    let (transfer_result, _) = run_transfer(&source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    // Verify symlinks were copied as symlinks (not followed)
    let link_path = dest.join("link.txt");
    assert!(
        link_path
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "link.txt should be a symlink in destination"
    );

    let dir_link_path = dest.join("dir_link");
    assert!(
        dir_link_path
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "dir_link should be a symlink in destination"
    );

    // Run verification - should succeed
    // Verifier only checks regular files, symlinks are skipped
    let (verify_result, _) = run_verification(&source, &dest).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    // Should only verify regular files: target.txt and real_dir/inside.txt
    assert_eq!(
        verify.files_verified, 2,
        "Should verify 2 regular files (not symlinks)"
    );
}

#[tokio::test]
async fn test_rsync_transfer_preserves_permissions() {
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
    let (transfer_result, _) = run_transfer(&source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    // Check permissions were preserved
    // Note: rsync with --chmod=u+rw,g+r,o+r modifies permissions slightly
    // The important thing is that executable bit is preserved where set

    let dest_exec = dest.join("executable.sh");
    let exec_mode = std::fs::metadata(&dest_exec).unwrap().permissions().mode();
    assert!(
        exec_mode & 0o100 != 0,
        "Executable should have user execute bit set, got {:o}",
        exec_mode
    );

    // Run verification
    let (verify_result, _) = run_verification(&source, &dest).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    assert_eq!(verify.files_verified, 3, "Should verify 3 files");
}

#[tokio::test]
async fn test_rsync_transfer_large_file() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");

    std::fs::create_dir_all(&source).unwrap();

    // Create a 1MB file with random-ish content
    let large_content: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
    create_file(&source.join("large.bin"), &large_content);

    // Run transfer
    let (transfer_result, _) = run_transfer(&source, &dest).await;
    assert!(
        transfer_result.is_ok(),
        "Transfer failed: {:?}",
        transfer_result
    );

    // Run verification
    let (verify_result, _) = run_verification(&source, &dest).await;
    assert!(
        verify_result.is_ok(),
        "Verification failed: {:?}",
        verify_result
    );

    let verify = verify_result.unwrap();
    assert_eq!(verify.files_verified, 1);
    assert_eq!(verify.bytes_verified, 1024 * 1024, "Should verify 1MB");
}
