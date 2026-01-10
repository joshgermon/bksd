use chrono::Local;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{Instrument, error, info, info_span};

use crate::context::AppContext;
use crate::core::TargetDrive;
use crate::core::hardware::{BlockDevice, HardwareAdapter, HardwareEvent};
use crate::core::ownership::get_backup_owner;
use crate::core::transfer_engine::{self, TransferRequest, TransferStatus};
use crate::core::verifier::{VerifyRequest, verify_transfer};
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
        tokio::spawn(async move {
            let transfer_result = transfer_engine
                .transfer(&transfer_req, progress_tx.clone())
                .await;

            match transfer_result {
                Ok(result) => {
                    let _ = progress_tx.send(TransferStatus::CopyComplete).await;

                    if config.verify_transfers && !config.simulation {
                        let verify_req = VerifyRequest {
                            job_id: job_id.clone(),
                            source: transfer_req.source.clone(),
                            destination: transfer_req.destination.clone(),
                        };

                        match verify_transfer(&verify_req, progress_tx.clone()).await {
                            Ok(_verify_result) => {
                                let _ = progress_tx
                                    .send(TransferStatus::Complete {
                                        total_bytes: result.total_bytes,
                                        duration_secs: result.duration_secs,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = progress_tx
                                    .send(TransferStatus::Failed(e.to_string()))
                                    .await;
                            }
                        }
                    } else {
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
                }
            }
        });
    }

    async fn handle_device_removed(&self, uuid: String) {
        info!(uuid = %uuid, "Device removed");
    }
}
