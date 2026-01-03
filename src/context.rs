use crate::config::AppConfig;
use tokio_rusqlite::Connection;

#[derive(Clone)]
pub struct AppContext {
    // AppConfig is simple enough to clone, or we can Arc it if it gets large.
    // For now, since AppConfig was struct with owned fields, we might need to wrap it in Arc
    // if we want cheap clones for AppContext, or derive Clone for AppConfig.
    // The previous AppConfig had PathBuf which is heap alloc, so Arc is better.
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
