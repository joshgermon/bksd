use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, error, info, info_span};
use udev::Enumerator;

use crate::core::hardware::{BlockDevice, HardwareAdapter, HardwareEvent, SupportedFilesystem};

/// Configuration for the Linux adapter
#[derive(Debug, Clone)]
pub struct LinuxAdapterConfig {
    /// Base path for mounting devices (e.g., /run/bksd)
    pub mount_base: PathBuf,
    /// Whether to auto-mount devices
    pub auto_mount: bool,
}

impl Default for LinuxAdapterConfig {
    fn default() -> Self {
        Self {
            mount_base: PathBuf::from("/run/bksd"),
            auto_mount: true,
        }
    }
}

/// Internal state for tracking mounted devices
struct MountState {
    /// Map of UUID -> mount point for devices we mounted
    mounted_by_us: HashMap<String, PathBuf>,
}

/// Data extracted from udev event (Send-safe)
#[derive(Debug)]
enum UdevEventData {
    Add {
        uuid: String,
        label: String,
        devnode: PathBuf,
        fs_type: String,
    },
    Remove {
        uuid: String,
    },
}

pub struct LinuxAdapter {
    config: LinuxAdapterConfig,
    cancel_token: CancellationToken,
    mount_state: Arc<Mutex<MountState>>,
    stopped_notify: Arc<Notify>,
}

impl LinuxAdapter {
    pub fn new(config: LinuxAdapterConfig) -> Self {
        Self {
            config,
            cancel_token: CancellationToken::new(),
            mount_state: Arc::new(Mutex::new(MountState {
                mounted_by_us: HashMap::new(),
            })),
            stopped_notify: Arc::new(Notify::new()),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(LinuxAdapterConfig::default())
    }
}

impl HardwareAdapter for LinuxAdapter {
    fn start(&self, event_sender: mpsc::Sender<HardwareEvent>) {
        let cancel_token = self.cancel_token.clone();
        let mount_state = self.mount_state.clone();
        let stopped_notify = self.stopped_notify.clone();
        let config = self.config.clone();

        // Channel for udev events (extracted data is Send-safe)
        let (udev_tx, mut udev_rx) = mpsc::channel::<UdevEventData>(32);

        // Spawn blocking task for udev monitor (udev types are not Send)
        let cancel_for_udev = cancel_token.clone();
        std::thread::spawn(move || {
            if let Err(e) = run_udev_monitor_blocking(udev_tx, cancel_for_udev) {
                error!(error = %e, "udev monitor error");
            }
        });

        // Spawn async task to process events
        let udev_span = info_span!("udev_monitor");
        tokio::spawn(
            async move {
                info!("Event processor started");

                while let Some(event_data) = udev_rx.recv().await {
                    if cancel_token.is_cancelled() {
                        break;
                    }

                    let Some(hw_event) =
                        process_event_data(event_data, &mount_state, &config).await
                    else {
                        continue;
                    };

                    if event_sender.send(hw_event).await.is_err() {
                        break;
                    }
                }

                stopped_notify.notify_one();
            }
            .instrument(udev_span),
        );
    }

    fn stop(&self) {
        self.cancel_token.cancel();
    }

    fn list_devices(&self) -> Result<Vec<BlockDevice>> {
        let mut enumerator = Enumerator::new()?;
        enumerator.match_subsystem("block")?;

        let mut devices = Vec::new();

        for device in enumerator.scan_devices()? {
            let devtype = device
                .property_value("DEVTYPE")
                .and_then(|v: &OsStr| v.to_str());
            let fs_type = device
                .property_value("ID_FS_TYPE")
                .and_then(|v: &OsStr| v.to_str());

            // Skip if not a partition and no filesystem detected
            if devtype != Some("partition") && fs_type.is_none() {
                continue;
            }

            let Some(fs_type) = fs_type else { continue };
            let Some(supported_fs) = SupportedFilesystem::from_str(fs_type) else {
                continue;
            };

            let Some(uuid) = device
                .property_value("ID_FS_UUID")
                .and_then(|v: &OsStr| v.to_str())
            else {
                continue;
            };

            let label = device
                .property_value("ID_FS_LABEL")
                .and_then(|v: &OsStr| v.to_str())
                .unwrap_or(uuid)
                .to_string();

            let Some(devnode) = device.devnode() else {
                continue;
            };

            // Only include mounted devices for enumeration
            let Some(mount_point) = get_mount_point(devnode) else {
                continue;
            };

            let capacity = get_device_capacity(devnode).unwrap_or(0);

            devices.push(BlockDevice {
                uuid: uuid.to_string(),
                label,
                path: devnode.to_path_buf(),
                mount_point,
                capacity,
                filesystem: supported_fs.as_str().to_string(),
            });
        }

        Ok(devices)
    }

    fn cleanup_device(&self, device: &BlockDevice) -> Result<()> {
        debug!(
            label = %device.label,
            uuid = %device.uuid,
            "Cleaning up device"
        );

        // Step 1: Sync the filesystem
        sync_filesystem(&device.mount_point)?;

        // Step 2: Check if we mounted this device
        let should_unmount = {
            let state = self.mount_state.lock().unwrap();
            state.mounted_by_us.contains_key(&device.uuid)
        };

        if should_unmount {
            debug!(
                mount_point = %device.mount_point.display(),
                "Unmounting device"
            );

            // Use lazy unmount to handle busy filesystems gracefully
            umount2(&device.mount_point, MntFlags::MNT_DETACH)
                .with_context(|| format!("Failed to unmount {}", device.mount_point.display()))?;

            // Remove from tracking
            {
                let mut state = self.mount_state.lock().unwrap();
                state.mounted_by_us.remove(&device.uuid);
            }

            // Clean up mount directory
            let _ = fs::remove_dir(&device.mount_point);
        } else {
            debug!("Device was not mounted by us, skipping unmount");
        }

        Ok(())
    }
}

/// Run udev monitor in a blocking thread (udev types are not Send/Sync)
fn run_udev_monitor_blocking(
    tx: mpsc::Sender<UdevEventData>,
    cancel_token: CancellationToken,
) -> Result<()> {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use std::os::unix::io::AsFd;

    let socket = udev::MonitorBuilder::new()?
        .match_subsystem("block")?
        .listen()?;

    info!("udev monitor started");

    loop {
        if cancel_token.is_cancelled() {
            info!("Shutdown requested, stopping udev monitor");
            break;
        }

        let poll_fd = PollFd::new(socket.as_fd(), PollFlags::POLLIN);

        match poll(&mut [poll_fd], PollTimeout::from(500_u16)) {
            Ok(0) => continue, // Timeout, check cancellation
            Ok(_) => {
                let Some(event) = socket.iter().next() else {
                    continue;
                };
                let Some(event_data) = extract_event_data(&event) else {
                    continue;
                };
                if tx.blocking_send(event_data).is_err() {
                    break;
                }
            }
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => anyhow::bail!("poll error: {}", e),
        }
    }

    Ok(())
}

/// Extract data from udev event (must be done in the same thread as the monitor)
fn extract_event_data(event: &udev::Event) -> Option<UdevEventData> {
    let device = event.device();

    match event.event_type() {
        udev::EventType::Add => {
            // Filter: must be a partition or have filesystem type
            let devtype = device.property_value("DEVTYPE").and_then(|v| v.to_str());
            let fs_type = device.property_value("ID_FS_TYPE").and_then(|v| v.to_str());

            if devtype != Some("partition") && fs_type.is_none() {
                return None;
            }

            let fs_type = fs_type?;
            let _ = SupportedFilesystem::from_str(fs_type)?; // Validate supported

            let uuid = device
                .property_value("ID_FS_UUID")
                .and_then(|v| v.to_str())?
                .to_string();

            let label = device
                .property_value("ID_FS_LABEL")
                .and_then(|v| v.to_str())
                .unwrap_or(&uuid)
                .to_string();

            let devnode = device.devnode()?.to_path_buf();

            Some(UdevEventData::Add {
                uuid,
                label,
                devnode,
                fs_type: fs_type.to_string(),
            })
        }

        udev::EventType::Remove => {
            let uuid = device
                .property_value("ID_FS_UUID")
                .and_then(|v| v.to_str())?
                .to_string();

            Some(UdevEventData::Remove { uuid })
        }

        _ => None,
    }
}

/// Process extracted event data (async-safe)
async fn process_event_data(
    event_data: UdevEventData,
    mount_state: &Arc<Mutex<MountState>>,
    config: &LinuxAdapterConfig,
) -> Option<HardwareEvent> {
    match event_data {
        UdevEventData::Add {
            uuid,
            label,
            devnode,
            fs_type,
        } => {
            let supported_fs = SupportedFilesystem::from_str(&fs_type)?;

            // Check if already mounted, mount if needed
            let mount_point = if let Some(existing) = get_mount_point(&devnode) {
                existing
            } else if config.auto_mount {
                match mount_device(&devnode, &uuid, &supported_fs, config).await {
                    Ok(mp) => {
                        mount_state
                            .lock()
                            .unwrap()
                            .mounted_by_us
                            .insert(uuid.clone(), mp.clone());
                        mp
                    }
                    Err(e) => {
                        error!(
                            device = %devnode.display(),
                            error = %e,
                            "Failed to mount device"
                        );
                        return None;
                    }
                }
            } else {
                return None;
            };

            let capacity = get_device_capacity(&devnode).unwrap_or(0);

            let block_device = BlockDevice {
                uuid,
                label,
                path: devnode,
                mount_point,
                capacity,
                filesystem: supported_fs.as_str().to_string(),
            };

            info!(
                label = %block_device.label,
                uuid = %block_device.uuid,
                mount_point = %block_device.mount_point.display(),
                "Device added"
            );

            Some(HardwareEvent::DeviceAdded(block_device))
        }

        UdevEventData::Remove { uuid } => {
            mount_state.lock().unwrap().mounted_by_us.remove(&uuid);
            info!(uuid = %uuid, "Device removed");
            Some(HardwareEvent::DeviceRemoved(uuid))
        }
    }
}

/// Check /proc/mounts to find if device is already mounted
fn get_mount_point(device_path: &Path) -> Option<PathBuf> {
    let file = fs::File::open("/proc/mounts").ok()?;
    let reader = BufReader::new(file);

    let device_str = device_path.to_string_lossy();

    for line in reader.lines().map_while(Result::ok) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[0] == device_str {
            return Some(PathBuf::from(parts[1]));
        }
    }

    None
}

/// Mount a device to /run/bksd/<uuid>
async fn mount_device(
    device_path: &Path,
    uuid: &str,
    fs_type: &SupportedFilesystem,
    config: &LinuxAdapterConfig,
) -> Result<PathBuf> {
    let mount_point = config.mount_base.join(uuid);

    fs::create_dir_all(&mount_point)
        .with_context(|| format!("Failed to create mount point: {}", mount_point.display()))?;

    let flags = MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID;

    // Filesystem-specific options
    let options: Option<&str> = match fs_type {
        SupportedFilesystem::Vfat | SupportedFilesystem::Exfat => {
            Some("utf8,uid=0,gid=0,umask=022")
        }
        SupportedFilesystem::Ntfs => Some("uid=0,gid=0,umask=022"),
        _ => None,
    };

    let device_path = device_path.to_path_buf();
    let fs_str = fs_type.as_str().to_string();
    let options = options.map(|s| s.to_string());

    let err_device = device_path.display().to_string();
    let err_mount = mount_point.display().to_string();

    tokio::task::spawn_blocking({
        let mount_point = mount_point.clone();
        move || {
            mount(
                Some(device_path.as_path()),
                mount_point.as_path(),
                Some(fs_str.as_str()),
                flags,
                options.as_deref(),
            )
        }
    })
    .await?
    .with_context(|| format!("Failed to mount {} to {}", err_device, err_mount))?;

    debug!(
        device = %err_device,
        mount_point = %err_mount,
        "Mounted device"
    );

    Ok(mount_point)
}

/// Sync filesystem buffers for a mount point
fn sync_filesystem(mount_point: &Path) -> Result<()> {
    let file = fs::File::open(mount_point)
        .with_context(|| format!("Failed to open mount point: {}", mount_point.display()))?;

    let fd = file.as_raw_fd();
    let result = unsafe { libc::syncfs(fd) };

    if result != 0 {
        anyhow::bail!("syncfs failed: {}", std::io::Error::last_os_error());
    }

    debug!(
        mount_point = %mount_point.display(),
        "Synced filesystem"
    );

    Ok(())
}

/// Get device capacity from sysfs
fn get_device_capacity(device_path: &Path) -> Option<u64> {
    let device_name = device_path.file_name()?.to_str()?;

    // Extract base device name (e.g., "sdb" from "sdb1")
    let base_device: String = device_name
        .chars()
        .take_while(|c| !c.is_numeric())
        .collect();

    let size_path = if base_device == device_name {
        // Whole disk
        PathBuf::from(format!("/sys/block/{}/size", device_name))
    } else {
        // Partition
        PathBuf::from(format!("/sys/block/{}/{}/size", base_device, device_name))
    };

    let size_str = fs::read_to_string(size_path).ok()?;
    let sectors: u64 = size_str.trim().parse().ok()?;

    // Convert sectors (512 bytes each) to bytes
    Some(sectors * 512)
}
