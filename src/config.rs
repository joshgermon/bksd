use crate::core::transfer_engine::TransferEngineType;
use figment::{
    Figment,
    providers::{Env, Serialized},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub backup_directory: PathBuf,
    pub transfer_engine: TransferEngineType,
    pub retry_attempts: u32,
    pub verbose: bool,
    pub simulation: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            backup_directory: PathBuf::from("/tmp/bksd"),
            transfer_engine: TransferEngineType::Rsync,
            retry_attempts: 3,
            verbose: false,
            simulation: false,
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
