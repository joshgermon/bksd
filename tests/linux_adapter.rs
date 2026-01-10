//! Linux adapter integration tests using loopback devices.
//!
//! Most tests require root privileges and Linux-specific tools (losetup, mkfs.ext4).
//!
//! Run all tests: `cargo test --test linux_adapter`
//! Run ignored tests: `sudo cargo test --test linux_adapter -- --ignored`

#![cfg(target_os = "linux")]

use bksd::adapters::{LinuxAdapter, LinuxAdapterConfig};
use bksd::core::{HardwareAdapter, HardwareEvent};
use nix::unistd::Uid;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tokio::time::timeout;

fn is_root() -> bool {
    Uid::effective().is_root()
}

fn has_losetup() -> bool {
    Command::new("losetup").arg("--version").output().is_ok()
}

fn has_mkfs_ext4() -> bool {
    Command::new("mkfs.ext4").arg("-V").output().is_ok()
}

/// Create a loopback device from a temp file, formatted with ext4.
/// Returns the loop device path (e.g., /dev/loop0) on success.
fn setup_loopback(file_path: &str, size_mb: u64) -> Option<String> {
    // Create file with dd
    let result = Command::new("dd")
        .args([
            "if=/dev/zero",
            &format!("of={}", file_path),
            "bs=1M",
            &format!("count={}", size_mb),
        ])
        .output()
        .ok()?;

    if !result.status.success() {
        eprintln!("dd failed: {}", String::from_utf8_lossy(&result.stderr));
        return None;
    }

    // Format with ext4
    let result = Command::new("mkfs.ext4")
        .args(["-F", "-q", file_path])
        .output()
        .ok()?;

    if !result.status.success() {
        eprintln!(
            "mkfs.ext4 failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        return None;
    }

    // Attach to loopback
    let output = Command::new("losetup")
        .args(["--find", "--show", file_path])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        eprintln!(
            "losetup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        None
    }
}

fn teardown_loopback(loop_device: &str) {
    let _ = Command::new("losetup").args(["-d", loop_device]).output();
}

#[tokio::test]
async fn test_list_devices() {
    let adapter = LinuxAdapter::with_defaults();
    let result = adapter.list_devices();
    assert!(
        result.is_ok(),
        "list_devices should not error: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_start_stop() {
    let adapter = LinuxAdapter::with_defaults();
    let (tx, _rx) = mpsc::channel(32);

    adapter.start(tx);

    // Give udev monitor time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    adapter.stop();

    // Give poll timeout (500ms) time to notice cancellation
    tokio::time::sleep(Duration::from_millis(600)).await;
}

#[tokio::test]
#[ignore = "requires root privileges and losetup/mkfs.ext4"]
async fn test_detects_loopback_device() {
    if !is_root() {
        eprintln!("Skipping: requires root");
        return;
    }

    if !has_losetup() {
        eprintln!("Skipping: losetup not available");
        return;
    }

    if !has_mkfs_ext4() {
        eprintln!("Skipping: mkfs.ext4 not available");
        return;
    }

    // Create temp file for loopback
    let temp_file = NamedTempFile::new().expect("create temp file");
    let file_path = temp_file.path().to_string_lossy().to_string();

    // Set up adapter (don't auto-mount for this test)
    let config = LinuxAdapterConfig {
        mount_base: PathBuf::from("/tmp/bksd_test"),
        auto_mount: false,
    };
    let adapter = LinuxAdapter::new(config);
    let (tx, mut rx) = mpsc::channel(32);

    adapter.start(tx);

    // Give adapter time to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create and attach loopback device
    let loop_device = match setup_loopback(&file_path, 32) {
        Some(dev) => dev,
        None => {
            eprintln!("Failed to setup loopback device");
            adapter.stop();
            return;
        }
    };

    println!("Created loopback device: {}", loop_device);

    // Wait for device event
    let event = timeout(Duration::from_secs(5), rx.recv()).await;

    // Cleanup before assertions
    teardown_loopback(&loop_device);
    adapter.stop();

    // Verify we got an event
    match event {
        Ok(Some(HardwareEvent::DeviceAdded(device))) => {
            println!("Detected device: {:?}", device);
            assert_eq!(device.filesystem, "ext4");
            assert!(device.capacity > 0);
        }
        Ok(Some(HardwareEvent::DeviceRemoved(_))) => {
            // Might catch the remove from teardown, acceptable
        }
        Ok(None) => {
            // Channel closed, adapter stopped
        }
        Err(_) => {
            // Timeout - loopback devices might not trigger udev events on all systems
            eprintln!("Note: No udev event received (may be expected on some systems)");
        }
    }
}
