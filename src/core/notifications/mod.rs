mod slack;

use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::{NotificationChannelType, NotificationConfig};

/// Events that trigger notifications
#[derive(Debug, Clone)]
pub enum JobEvent {
    Started {
        job_id: String,
        device_label: String,
        device_uuid: String,
        source: PathBuf,
        destination: PathBuf,
    },
    Completed {
        job_id: String,
        device_label: String,
        total_bytes: u64,
        duration_secs: u64,
    },
    Failed {
        job_id: String,
        device_label: String,
        error: String,
    },
}

/// Trait for notification channel implementations (Slack, Discord, etc.)
#[async_trait]
pub trait NotificationChannel: Send + Sync {
    async fn notify(&self, event: JobEvent) -> Result<()>;
}

/// Factory function to create a notifier based on config
pub fn create_notifier(config: &NotificationConfig) -> Option<Arc<dyn NotificationChannel>> {
    match &config.channel {
        NotificationChannelType::None => None,
        NotificationChannelType::Slack => {
            let webhook = config.slack_webhook.as_ref()?;
            if webhook.is_empty() {
                return None;
            }
            Some(Arc::new(slack::SlackNotifier::new(webhook.clone())))
        }
    }
}
