use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::core::hardware::{BlockDevice, HardwareAdapter, HardwareEvent};

enum SimulatedCommand {
    InjectAdd(BlockDevice),
    InjectRemove(String),
}

#[derive(Clone)]
pub struct Simulator {
    tx: mpsc::UnboundedSender<SimulatedCommand>,
}

impl Simulator {
    pub fn add_device(&self, uuid: &str, size_gb: u64) {
        let device = BlockDevice {
            uuid: uuid.to_string(),
            label: format!("TEST_DEVICE_{}", uuid),
            path: PathBuf::from(format!("/tmp/test_{}", uuid)),
            mount_point: PathBuf::from(format!("/tmp/mnt_{}", uuid)),
            capacity: size_gb * 1024 * 1024 * 1024,
            filesystem: "ext4".to_string(),
        };

        let _ = self.tx.send(SimulatedCommand::InjectAdd(device));
    }

    pub fn remove_device(&self, uuid: &str) {
        let _ = self
            .tx
            .send(SimulatedCommand::InjectRemove(uuid.to_string()));
    }
}

pub struct SimulatedAdapter {
    // We wrap the receiver in a Mutex so we can move it out inside `start()`
    // which takes &self. (Start is only called once).
    cmd_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<SimulatedCommand>>>>,
}

impl SimulatedAdapter {
    pub fn new() -> (Self, Simulator) {
        let (tx, rx) = mpsc::unbounded_channel();

        (
            Self {
                cmd_rx: Arc::new(Mutex::new(Some(rx))),
            },
            Simulator { tx },
        )
    }
}

impl HardwareAdapter for SimulatedAdapter {
    fn start(&self, daemon_tx: mpsc::Sender<HardwareEvent>) {
        // Steal the receiver from the mutex
        let mut rx = self
            .cmd_rx
            .lock()
            .unwrap()
            .take()
            .expect("SimulatedAdapter::start() called twice");

        info!("SimulatedAdapter listening for controller commands");

        // Bridge task
        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                let event = match cmd {
                    SimulatedCommand::InjectAdd(device) => HardwareEvent::DeviceAdded(device),
                    SimulatedCommand::InjectRemove(uuid) => HardwareEvent::DeviceRemoved(uuid),
                };

                if daemon_tx.send(event).await.is_err() {
                    break;
                }
            }
        });
    }

    fn stop(&self) {
        // No-op for simulated adapter
        info!("SimulatedAdapter stop requested");
    }

    fn list_devices(&self) -> Result<Vec<BlockDevice>> {
        // Simulated adapter doesn't track devices persistently
        Ok(vec![])
    }

    fn cleanup_device(&self, device: &BlockDevice) -> Result<()> {
        debug!(
            label = %device.label,
            uuid = %device.uuid,
            "SimulatedAdapter cleaning up device"
        );
        Ok(())
    }
}
