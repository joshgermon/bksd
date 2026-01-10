use std::path::PathBuf;

use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum HardwareEvent {
    DeviceAdded(BlockDevice),
    DeviceRemoved(String),
}

#[derive(Debug, Clone)]
pub struct BlockDevice {
    pub uuid: String,
    pub label: String,
    pub path: PathBuf,
    pub mount_point: PathBuf,
    pub capacity: u64,
    pub filesystem: String,
}

/// Supported filesystems for backup operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedFilesystem {
    Ext4,
    Exfat,
    Vfat,
    Ntfs,
    Btrfs,
}

impl SupportedFilesystem {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ext4" => Some(Self::Ext4),
            "exfat" => Some(Self::Exfat),
            "vfat" | "fat32" | "fat16" => Some(Self::Vfat),
            "ntfs" => Some(Self::Ntfs),
            "btrfs" => Some(Self::Btrfs),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ext4 => "ext4",
            Self::Exfat => "exfat",
            Self::Vfat => "vfat",
            Self::Ntfs => "ntfs",
            Self::Btrfs => "btrfs",
        }
    }
}

pub trait HardwareAdapter: Send + Sync {
    /// Start listening for hardware events.
    /// Spawns internal tasks that send events to the provided channel.
    fn start(&self, event_sender: mpsc::Sender<HardwareEvent>);

    /// Stop the hardware monitor gracefully.
    fn stop(&self);

    /// List all currently connected and valid devices.
    fn list_devices(&self) -> anyhow::Result<Vec<BlockDevice>>;

    /// Cleanup a device: sync filesystem and unmount.
    /// NOTE: This method performs blocking I/O (syncfs, umount) and should be
    /// called from a blocking context (e.g., via spawn_blocking).
    fn cleanup_device(&self, device: &BlockDevice) -> anyhow::Result<()>;
}
