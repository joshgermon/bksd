use crate::core::transfer_engine::{
    TransferEngine, TransferRequest, TransferResult, TransferStatus,
};
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

pub struct SimulatedEngine {
    pub speed_mb_per_sec: u64,
}

impl Default for SimulatedEngine {
    fn default() -> Self {
        Self {
            speed_mb_per_sec: 100,
        }
    }
}

impl TransferEngine for SimulatedEngine {
    fn transfer(
        &self,
        req: &TransferRequest,
        tx: mpsc::Sender<TransferStatus>,
    ) -> Pin<Box<dyn Future<Output = Result<TransferResult>> + Send>> {
        let _req = req.clone();
        let speed = self.speed_mb_per_sec;

        Box::pin(async move {
            let start_time = Instant::now();

            let _ = tx.send(TransferStatus::Ready).await;
            sleep(Duration::from_millis(500)).await;

            let total_size: u64 = 1024 * 1024 * 500; // 500 MB
            let chunk_size = speed * 1024 * 1024 / 2; // update twice per second
            let mut copied: u64 = 0;

            while copied < total_size {
                copied += chunk_size;
                if copied > total_size {
                    copied = total_size;
                }

                let percentage = ((copied as f64 / total_size as f64) * 100.0) as u8;

                let _ = tx
                    .send(TransferStatus::InProgress {
                        total_bytes: total_size,
                        bytes_copied: copied,
                        current_file: "simulated_file.dat".to_string(),
                        percentage,
                    })
                    .await;

                sleep(Duration::from_millis(500)).await;
            }

            let duration_secs = start_time.elapsed().as_secs();

            // Return transfer result - orchestrator handles CopyComplete and verification
            Ok(TransferResult {
                total_bytes: total_size,
                duration_secs,
            })
        })
    }
}
