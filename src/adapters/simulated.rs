use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::core::hardware::{BlockDevice, HardwareEvent, HardwareMonitor};
use tokio::sync::mpsc;

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
            capacity: size_gb * 1024 * 1024 * 1024,
        };

        let _ = self.tx.send(SimulatedCommand::InjectAdd(device));
    }

    pub fn remove_device(&self, uuid: &str) {
        let _ = self
            .tx
            .send(SimulatedCommand::InjectRemove(uuid.to_string()));
    }
}

pub struct SimulatedMonitor {
    // We wrap the receiver in a Mutex so we can move it out inside `start()`
    // which takes &self. (Start is only called once).
    cmd_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<SimulatedCommand>>>>,
}

impl SimulatedMonitor {
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

impl HardwareMonitor for SimulatedMonitor {
    fn start(&self, daemon_tx: mpsc::Sender<HardwareEvent>) {
        // Steal the receiver from the mutex
        let mut rx = self
            .cmd_rx
            .lock()
            .unwrap()
            .take()
            .expect("SimulatedMonitor::start() called twice");

        println!("(SimulatedMonitor) Starting listening for controller commands...");

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
}
