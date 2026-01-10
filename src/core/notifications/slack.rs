use super::{JobEvent, NotificationChannel};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

pub struct SlackNotifier {
    webhook_url: String,
    client: reqwest::Client,
}

impl SlackNotifier {
    pub fn new(webhook_url: String) -> Self {
        Self {
            webhook_url,
            client: reqwest::Client::new(),
        }
    }

    fn format_message(&self, event: &JobEvent) -> serde_json::Value {
        match event {
            JobEvent::Started {
                job_id,
                device_label,
                source,
                destination,
                ..
            } => {
                let short_id = &job_id[..8.min(job_id.len())];
                json!({
                    "blocks": [
                        {
                            "type": "header",
                            "text": {
                                "type": "plain_text",
                                "text": "Backup Started",
                                "emoji": true
                            }
                        },
                        {
                            "type": "section",
                            "fields": [
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Device:*\n{}", device_label)
                                },
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Job ID:*\n`{}`", short_id)
                                },
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Source:*\n`{}`", source.display())
                                },
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Destination:*\n`{}`", destination.display())
                                }
                            ]
                        }
                    ]
                })
            }
            JobEvent::Completed {
                job_id,
                device_label,
                total_bytes,
                duration_secs,
            } => {
                let short_id = &job_id[..8.min(job_id.len())];
                let size_mb = *total_bytes as f64 / (1024.0 * 1024.0);
                let speed_mbps = if *duration_secs > 0 {
                    size_mb / *duration_secs as f64
                } else {
                    0.0
                };
                json!({
                    "blocks": [
                        {
                            "type": "header",
                            "text": {
                                "type": "plain_text",
                                "text": "Backup Complete",
                                "emoji": true
                            }
                        },
                        {
                            "type": "section",
                            "fields": [
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Device:*\n{}", device_label)
                                },
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Job ID:*\n`{}`", short_id)
                                },
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Size:*\n{:.1} MB", size_mb)
                                },
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Duration:*\n{}s ({:.1} MB/s)", duration_secs, speed_mbps)
                                }
                            ]
                        }
                    ]
                })
            }
            JobEvent::Failed {
                job_id,
                device_label,
                error,
            } => {
                let short_id = &job_id[..8.min(job_id.len())];
                json!({
                    "blocks": [
                        {
                            "type": "header",
                            "text": {
                                "type": "plain_text",
                                "text": "Backup Failed",
                                "emoji": true
                            }
                        },
                        {
                            "type": "section",
                            "fields": [
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Device:*\n{}", device_label)
                                },
                                {
                                    "type": "mrkdwn",
                                    "text": format!("*Job ID:*\n`{}`", short_id)
                                }
                            ]
                        },
                        {
                            "type": "section",
                            "text": {
                                "type": "mrkdwn",
                                "text": format!("*Error:*\n```{}```", error)
                            }
                        }
                    ]
                })
            }
        }
    }
}

#[async_trait]
impl NotificationChannel for SlackNotifier {
    async fn notify(&self, event: JobEvent) -> Result<()> {
        let payload = self.format_message(&event);
        self.client
            .post(&self.webhook_url)
            .json(&payload)
            .send()
            .await?;
        Ok(())
    }
}
