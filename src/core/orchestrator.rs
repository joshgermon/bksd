use chrono::Local;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{Instrument, error, info, info_span, warn};

use crate::context::AppContext;
use crate::core::TargetDrive;
use crate::core::hardware::{BlockDevice, HardwareAdapter, HardwareEvent};
use crate::core::notifications::JobEvent;
use crate::core::ownership::get_backup_owner;
use crate::core::transfer_engine::{self, TransferRequest, TransferStatus};
use crate::core::verifier::verify_from_hashes;
use crate::logging::LogThrottle;
use crate::{adapters, db};
use anyhow::Result;

pub struct Orchestrator {
    ctx: AppContext,
    adapter: Arc<dyn HardwareAdapter>,
}

impl Orchestrator {
    pub fn new(ctx: AppContext) -> Self {
        let adapter: Box<dyn HardwareAdapter> = adapters::get_adapter(&ctx.config);
        Self {
            ctx,
            adapter: Arc::from(adapter),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let backup_dir = self.ctx.config.backup_directory.display().to_string();
        let simulation = self.ctx.config.simulation;

        let span = info_span!(
            "daemon",
            backup_dir = %backup_dir,
            simulation = simulation
        );

        async {
            info!("Sentinel Daemon starting");

            let (tx, mut rx) = mpsc::channel(32);

            self.adapter.start(tx);

            while let Some(event) = rx.recv().await {
                self.handle_device_event(event).await;
            }

            Ok(())
        }
        .instrument(span)
        .await
    }

    pub async fn handle_device_event(&self, event: HardwareEvent) {
        match event {
            HardwareEvent::DeviceAdded(dev) => self.handle_device_added(dev).await,
            HardwareEvent::DeviceRemoved(uuid) => self.handle_device_removed(uuid).await,
        }
    }

    fn build_destination(&self, label: &str) -> PathBuf {
        let timestamp = Local::now().format("%Y-%m-%d_T%H%M_%S").to_string();
        self.ctx.config.backup_directory.join(label).join(timestamp)
    }

    async fn handle_device_added(&self, dev: BlockDevice) {
        let device_span = info_span!(
            "device",
            uuid = %dev.uuid,
            label = %dev.label,
            filesystem = %dev.filesystem
        );
        let _guard = device_span.enter();

        info!(
            path = %dev.path.display(),
            mount_point = %dev.mount_point.display(),
            capacity_mb = dev.capacity / (1024 * 1024),
            "New device detected"
        );

        let job_id = uuid::Uuid::now_v7().to_string();
        let destination = self.build_destination(&dev.label);

        let target_drive = TargetDrive {
            uuid: dev.uuid.clone(),
            label: dev.label.clone(),
            mount_path: dev.mount_point.to_string_lossy().to_string(),
            raw_size: dev.capacity,
        };

        let transfer_engine =
            transfer_engine::create_engine(self.ctx.config.transfer_engine.clone());

        if let Err(e) = db::jobs::create(
            &self.ctx.db,
            job_id.clone(),
            target_drive,
            destination.to_string_lossy().to_string(),
        )
        .await
        {
            error!(error = %e, "Failed to create job in database");
            return;
        }

        info!(
            job_id = %job_id,
            source = %dev.mount_point.display(),
            destination = %destination.display(),
            "Job created"
        );

        // Send "Started" notification
        if let Some(ref notifier) = self.ctx.notifier {
            let event = JobEvent::Started {
                job_id: job_id.clone(),
                device_label: dev.label.clone(),
                device_uuid: dev.uuid.clone(),
                source: dev.mount_point.clone(),
                destination: destination.clone(),
            };
            let notifier = notifier.clone();
            tokio::spawn(async move {
                if let Err(e) = notifier.notify(event).await {
                    warn!(error = %e, "Failed to send start notification");
                }
            });
        }

        let transfer_req = TransferRequest {
            job_id: job_id.clone(),
            source: dev.mount_point.clone(),
            destination,
            owner: get_backup_owner(&self.ctx.config.backup_directory),
        };

        let (progress_tx, mut progress_rx) = mpsc::channel(100);
        let db = self.ctx.db.clone();
        let adapter = self.adapter.clone();
        let progress_tracker = self.ctx.progress.clone();

        // Progress throttle: only log every 500ms
        let throttle = LogThrottle::new(Duration::from_millis(500));

        let job_span = info_span!(
            "job",
            job_id = %job_id,
            source = %dev.mount_point.display(),
            destination = %transfer_req.destination.display()
        );

        let config = self.ctx.config.clone();
        let notifier = self.ctx.notifier.clone();
        let device_label = dev.label.clone();
        let job_id_for_consumer = job_id.clone();

        // Spawn transfer task
        tokio::spawn(async move {
            let transfer_result = transfer_engine
                .transfer(&transfer_req, progress_tx.clone())
                .await;

            match transfer_result {
                Ok(result) => {
                    let _ = progress_tx.send(TransferStatus::CopyComplete).await;

                    // Verify if enabled and we have file hashes from the transfer
                    let verification_passed = if config.verify_transfers && !config.simulation {
                        match &result.file_hashes {
                            Some(hashes) => {
                                // Fast path: verify using hashes computed during copy
                                match verify_from_hashes(&job_id, &transfer_req.destination, hashes)
                                    .await
                                {
                                    Ok(_) => true,
                                    Err(e) => {
                                        let _ = progress_tx
                                            .send(TransferStatus::Failed(e.to_string()))
                                            .await;
                                        false
                                    }
                                }
                            }
                            None => {
                                // Engine handles verification internally (e.g., rsync --checksum)
                                // or doesn't support it (simulated) - trust the transfer
                                true
                            }
                        }
                    } else {
                        true
                    };

                    if verification_passed {
                        let _ = progress_tx
                            .send(TransferStatus::Complete {
                                total_bytes: result.total_bytes,
                                duration_secs: result.duration_secs,
                            })
                            .await;
                    }
                }
                Err(e) => {
                    error!(job_id = %job_id, error = %e, "Transfer error");
                    let _ = progress_tx
                        .send(TransferStatus::Failed(e.to_string()))
                        .await;
                }
            }
        });

        // Spawn progress consumer task
        tokio::spawn(
            async move {
                while let Some(status) = progress_rx.recv().await {
                    // Log progress with throttling
                    if let TransferStatus::InProgress { percentage, .. } = &status {
                        if throttle.should_log() {
                            info!(percentage = %percentage, "Transfer progress");
                        }
                    }

                    // Update in-memory tracker
                    progress_tracker
                        .update(&job_id_for_consumer, status.clone())
                        .await;

                    // Persist and notify based on status
                    match &status {
                        TransferStatus::CopyComplete => {
                            let _ = db::jobs::update_status(
                                &db,
                                job_id_for_consumer.clone(),
                                "copy_complete".to_string(),
                                None,
                                None,
                                None,
                            )
                            .await;
                        }
                        TransferStatus::Complete {
                            total_bytes,
                            duration_secs,
                        } => {
                            let _ = db::jobs::update_status(
                                &db,
                                job_id_for_consumer.clone(),
                                "complete".to_string(),
                                None,
                                Some(*total_bytes),
                                Some(*duration_secs),
                            )
                            .await;

                            // Send completion notification
                            if let Some(ref notifier) = notifier {
                                let event = JobEvent::Completed {
                                    job_id: job_id_for_consumer.clone(),
                                    device_label: device_label.clone(),
                                    total_bytes: *total_bytes,
                                    duration_secs: *duration_secs,
                                };
                                if let Err(e) = notifier.notify(event).await {
                                    warn!(error = %e, "Failed to send completion notification");
                                }
                            }

                            // Cleanup: unmount device if we mounted it
                            if let Err(e) = adapter.cleanup_device(&dev) {
                                error!(error = %e, "Failed to cleanup device");
                            }

                            progress_tracker.remove(&job_id_for_consumer).await;
                            break;
                        }
                        TransferStatus::Failed(error) => {
                            let _ = db::jobs::update_status(
                                &db,
                                job_id_for_consumer.clone(),
                                "failed".to_string(),
                                Some(error.clone()),
                                None,
                                None,
                            )
                            .await;

                            // Send failure notification
                            if let Some(ref notifier) = notifier {
                                let event = JobEvent::Failed {
                                    job_id: job_id_for_consumer.clone(),
                                    device_label: device_label.clone(),
                                    error: error.clone(),
                                };
                                if let Err(e) = notifier.notify(event).await {
                                    warn!(error = %e, "Failed to send failure notification");
                                }
                            }

                            progress_tracker.remove(&job_id_for_consumer).await;
                            break;
                        }
                        _ => {}
                    }
                }
            }
            .instrument(job_span),
        );
    }

    async fn handle_device_removed(&self, uuid: String) {
        info!(uuid = %uuid, "Device removed");
    }
}
