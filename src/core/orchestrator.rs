use chrono::Local;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::context::AppContext;
use crate::core::TargetDrive;
use crate::core::hardware::{BlockDevice, HardwareAdapter, HardwareEvent};
use crate::core::transfer_engine::{self, TransferRequest, TransferStatus};
use crate::{adapters, db};
use anyhow::Result;

pub struct Orchestrator {
    ctx: AppContext,
    adapter: Arc<dyn HardwareAdapter>,
}

impl Orchestrator {
    pub fn new(ctx: AppContext) -> Self {
        let adapter: Box<dyn HardwareAdapter> = adapters::get_adapter(ctx.config.simulation);
        Self {
            ctx,
            adapter: Arc::from(adapter),
        }
    }

    pub async fn start(&self) -> Result<()> {
        println!(">> Sentinel Daemon Starting...");

        let (tx, mut rx) = mpsc::channel(32);

        self.adapter.start(tx);

        while let Some(event) = rx.recv().await {
            self.handle_device_event(event).await;
        }

        Ok(())
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
        println!(">> NEW CARD: {} ({})", dev.label, dev.uuid);

        let job_id = uuid::Uuid::now_v7().to_string();
        let destination = self.build_destination(&dev.label);

        let target_drive = TargetDrive {
            uuid: dev.uuid.clone(),
            label: dev.label.clone(),
            mount_path: dev.path.to_string_lossy().to_string(),
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
            eprintln!(">> FAILED TO CREATE JOB: {}", e);
            return;
        }

        println!(">> JOB CREATED: {}", job_id);

        let transfer_req = TransferRequest {
            job_id: job_id.clone(),
            source: dev.path.clone(),
            destination,
        };

        let (progress_tx, mut progress_rx) = mpsc::channel(100);
        let db = self.ctx.db.clone();
        let job_id_clone = job_id.clone();
        let adapter = self.adapter.clone();
        let dev_clone = dev.clone();

        tokio::spawn(async move {
            while let Some(status) = progress_rx.recv().await {
                let (status_str, description) = match &status {
                    TransferStatus::Ready => ("Ready", None),
                    TransferStatus::InProgress { percentage, .. } => {
                        println!(">> PROGRESS: {}%", percentage);
                        ("InProgress", Some(format!("{}% complete", percentage)))
                    }
                    TransferStatus::CopyComplete => ("CopyComplete", None),
                    TransferStatus::Verifying { current, total } => {
                        ("Verifying", Some(format!("{}/{}", current, total)))
                    }
                    TransferStatus::Complete => ("Complete", None),
                    TransferStatus::Failed(msg) => ("Failed", Some(msg.clone())),
                };

                let _ = db::jobs::update_status(
                    &db,
                    job_id_clone.clone(),
                    status_str.to_string(),
                    description,
                )
                .await;

                if matches!(status, TransferStatus::Complete | TransferStatus::Failed(_)) {
                    println!(">> JOB FINISHED: {} (Status: {:?})", job_id_clone, status);
                    let _ = adapter.cleanup_device(&dev_clone);
                    break;
                }
            }
        });

        // Start transfer
        tokio::spawn(async move {
            if let Err(e) = transfer_engine.transfer(&transfer_req, progress_tx).await {
                eprintln!(">> TRANSFER ERROR [{}]: {}", job_id, e);
            }
        });
    }

    async fn handle_device_removed(&self, uuid: String) {
        println!(">> REMOVED: {}", uuid);
    }
}
