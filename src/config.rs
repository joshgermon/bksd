use crate::core::transfer_engine::TransferEngineType;
use figment::{
    Figment,
    providers::{Env, Serialized},
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub backup_directory: PathBuf,
    pub transfer_engine: TransferEngineType,
    pub retry_attempts: u32,
    pub verbose: bool,
    pub simulation: bool,
    pub mount_base: PathBuf,
    /// Output logs as JSON instead of pretty console format
    pub log_json: bool,
    /// Enable the RPC server for client connections
    pub rpc_enabled: bool,
    /// Address and port for the RPC server to bind to
    pub rpc_bind: SocketAddr,
    /// Verify file integrity after transfer using BLAKE3 checksums
    pub verify_transfers: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            backup_directory: PathBuf::from("/tmp/bksd"),
            transfer_engine: TransferEngineType::Rsync,
            retry_attempts: 3,
            verbose: false,
            simulation: false,
            mount_base: PathBuf::from("/run/bksd"),
            log_json: false,
            rpc_enabled: true,
            rpc_bind: SocketAddr::from(([127, 0, 0, 1], 9847)),
            verify_transfers: true,
        }
    }
}

impl AppConfig {
    pub fn new(args: Option<&impl Serialize>) -> Result<Self, figment::Error> {
        let mut figment = Figment::new().merge(Serialized::defaults(AppConfig::default()));

        if let Some(args) = args {
            figment = figment.merge(Serialized::defaults(args));
        }

        figment = figment.merge(Env::prefixed("BKSD_"));

        figment.extract()
    }
}
