use std::path::PathBuf;

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
    pub capacity: u64,
}

pub trait HardwareAdapter: Send + Sync {
    fn start(&self, event_sender: tokio::sync::mpsc::Sender<HardwareEvent>);
    fn cleanup_device(&self, device: &BlockDevice) -> anyhow::Result<()>;
}
