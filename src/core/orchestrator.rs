use tokio::sync::mpsc;

use crate::adapters;
use crate::context::AppContext;
use crate::core::hardware::{BlockDevice, HardwareEvent};
use anyhow::Result;

pub struct Orchestrator {
    ctx: AppContext,
}

impl Orchestrator {
    pub fn new(ctx: AppContext) -> Self {
        Self { ctx }
    }

    pub async fn start(&self) -> Result<()> {
        println!(">> Sentinel Daemon Starting...");

        let (tx, mut rx) = mpsc::channel(32);
        let monitor = adapters::get_monitor();

        monitor.start(tx);

        while let Some(event) = rx.recv().await {
            self.handle_device_event(event);
        }

        Ok(())
    }

    pub fn handle_device_event(&self, event: HardwareEvent) {
        match event {
            HardwareEvent::DeviceAdded(dev) => self.handle_device_added(dev),
            HardwareEvent::DeviceRemoved(uuid) => self.handle_device_removed(uuid),
        }
    }

    fn handle_device_added(&self, dev: BlockDevice) {
        println!(">> NEW CARD: {} ({})", dev.label, dev.uuid);
    }

    fn handle_device_removed(&self, uuid: String) {
        println!(">> REMOVED: {}", uuid);
    }
}
