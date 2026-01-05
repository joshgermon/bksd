use crate::core::hardware::{BlockDevice, HardwareAdapter, HardwareEvent};
use tokio::sync::mpsc;

pub struct LinuxAdapter;

impl HardwareAdapter for LinuxAdapter {
    fn start(&self, _tx: mpsc::Sender<HardwareEvent>) {
        println!("(LinuxAdapter) Starting udev listener thread...");
    }

    fn cleanup_device(&self, device: &BlockDevice) -> anyhow::Result<()> {
        println!("(LinuxAdapter) Would unmount: {:?}", device.path);
        Ok(())
    }
}
