mod rsync;
mod simulated;

use crate::core::ownership::FileOwner;
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
    /// Owner for transferred files. If None, files will be owned by the process user (root).
    pub owner: Option<FileOwner>,
}

/// Result returned by transfer engines on successful completion
#[derive(Debug, Clone)]
pub struct TransferResult {
    /// Total bytes transferred
    pub total_bytes: u64,
    /// Duration of the transfer in seconds
    pub duration_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
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
    Complete {
        /// Total bytes transferred during the backup
        total_bytes: u64,
        /// Duration of the actual transfer in seconds (not including queue time)
        duration_secs: u64,
    },
    Failed(String),
}

pub trait TransferEngine: Send + Sync {
    fn transfer(
        &self,
        req: &TransferRequest,
        tx: mpsc::Sender<TransferStatus>,
    ) -> Pin<Box<dyn Future<Output = Result<TransferResult>> + Send>>;
}

pub fn create_engine(engine_type: TransferEngineType) -> Box<dyn TransferEngine> {
    match engine_type {
        TransferEngineType::Rsync => Box::new(rsync::RsyncEngine),
        TransferEngineType::Simulated => Box::new(simulated::SimulatedEngine::default()),
    }
}
