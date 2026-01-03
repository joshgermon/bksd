use std::path::PathBuf;

pub struct AppConfig {
    pub backup_directory: PathBuf,
    pub retry_attempts: u32,
    pub http_port: u16,
    pub simulation: bool,
    pub verbose: bool,
}
