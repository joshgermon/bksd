use std::sync::Arc;

use crate::config::AppConfig;
use crate::core::ProgressTracker;
use crate::core::notifications::{self, NotificationChannel};
use tokio_rusqlite::Connection;

#[derive(Clone)]
pub struct AppContext {
    pub config: Arc<AppConfig>,
    pub db: Connection,
    pub progress: ProgressTracker,
    pub notifier: Option<Arc<dyn NotificationChannel>>,
}

impl AppContext {
    pub fn new(config: AppConfig, db: Connection) -> Self {
        let notifier = notifications::create_notifier(&config.notifications);
        Self {
            config: Arc::new(config),
            db,
            progress: ProgressTracker::new(),
            notifier,
        }
    }
}
