mod rsync;
mod simulated;

use anyhow::Result;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
pub enum TransferEngineType {
    Rsync,
    Simulated,
}

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub job_id: String,
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Debug, Clone)]
pub enum TransferStatus {
    Ready,
    InProgress {
        total_bytes: u64,
        bytes_copied: u64,
        current_file: String,
        percentage: u8,
    },
    CopyComplete,
    Verifying {
        current: u64,
        total: u64,
    },
    Complete,
    Failed(String),
}

pub trait TransferEngine: Send + Sync {
    fn transfer(
        &self,
        req: &TransferRequest,
        tx: mpsc::Sender<TransferStatus>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;
}

pub fn create_engine(engine_type: TransferEngineType) -> Box<dyn TransferEngine> {
    match engine_type {
        TransferEngineType::Rsync => Box::new(rsync::RsyncEngine),
        TransferEngineType::Simulated => Box::new(simulated::SimulatedEngine::default()),
    }
}
