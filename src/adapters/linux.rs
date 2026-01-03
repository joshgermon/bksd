use crate::core::hardware::{HardwareEvent, HardwareMonitor};
use tokio::sync::mpsc;

pub struct LinuxMonitor;

impl HardwareMonitor for LinuxMonitor {
    fn start(&self, _tx: mpsc::Sender<HardwareEvent>) {
        println!("(LinuxMonitor) Starting udev listener thread...");
    }
}
