use crate::config::AppConfig;
use tokio_rusqlite::Connection;

#[derive(Clone)]
pub struct AppContext {
    pub config: std::sync::Arc<AppConfig>,
    pub db: Connection,
}

impl AppContext {
    pub fn new(config: AppConfig, db: Connection) -> Self {
        Self {
            config: std::sync::Arc::new(config),
            db,
        }
    }
}
