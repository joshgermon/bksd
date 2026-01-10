use crate::core::transfer_engine::{
    TransferEngine, TransferRequest, TransferResult, TransferStatus,
};
use anyhow::{Result, anyhow};
use regex::Regex;
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;
use tokio::{
    io::{AsyncReadExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tracing::{Instrument, info, info_span};

pub struct RsyncEngine;

impl TransferEngine for RsyncEngine {
    fn transfer(
        &self,
        req: &TransferRequest,
        tx: mpsc::Sender<TransferStatus>,
    ) -> Pin<Box<dyn Future<Output = Result<TransferResult>> + Send>> {
        let req = req.clone();
        Box::pin(async move {
            let _ = tx.send(TransferStatus::Ready).await;

            let source = req.source.to_string_lossy().to_string();
            let destination = req.destination.to_string_lossy().to_string();

            // Safety check: fail if destination already exists to prevent overwrites
            if req.destination.exists() {
                let msg = format!(
                    "Destination already exists: {}. Refusing to overwrite.",
                    req.destination.display()
                );
                let _ = tx.send(TransferStatus::Failed(msg.clone())).await;
                return Err(anyhow!(msg));
            }

            if let Err(e) = std::fs::create_dir_all(&req.destination) {
                let msg = format!("Failed to create destination directory: {}", e);
                let _ = tx.send(TransferStatus::Failed(msg.clone())).await;
                return Err(anyhow!(msg));
            }

            let span = info_span!(
                "rsync_transfer",
                source = %source,
                destination = %destination
            );

            async {
                info!("Starting rsync transfer");

                let start_time = Instant::now();
                let mut last_bytes_copied: u64 = 0;

                let mut cmd = Command::new("rsync");
                cmd.arg("-av")
                    .arg("--chmod=u+rw,g+r,o+r")
                    .arg("--info=progress2")
                    .arg("--no-inc-recursive");

                if let Some(ref owner) = req.owner {
                    cmd.arg(format!("--chown={}", owner.as_chown_arg()));
                    info!(owner = %owner.as_chown_arg(), "Setting file ownership");
                }

                let mut child_process = cmd
                    .arg(format!("{}/", source)) // trailing slash to copy contents
                    .arg(destination.as_str())
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| anyhow!("Failed to spawn rsync process: {}", e))?;

                let stdout = child_process
                    .stdout
                    .take()
                    .ok_or_else(|| anyhow!("Failed to get stdout"))?;
                let mut reader = BufReader::new(stdout);

                // Regex: "  12,345,678   45%  10.2MB/s ..."
                let re = Regex::new(r"^\s*([\d,]+)\s+(\d+)%").unwrap();

                let mut line_buffer = Vec::new();
                let mut byte_buffer = [0u8; 1];

                while let Ok(n) = reader.read(&mut byte_buffer).await {
                    if n == 0 {
                        break;
                    }

                    let b = byte_buffer[0];

                    if b == b'\r' || b == b'\n' {
                        if line_buffer.is_empty() {
                            continue;
                        }

                        let line = String::from_utf8_lossy(&line_buffer);

                        if let Some(capts) = re.captures(&line) {
                            let bytes_copied: u64 = capts
                                .get(1)
                                .unwrap()
                                .as_str()
                                .replace(",", "")
                                .parse()
                                .unwrap_or(0);

                            let percentage: u8 =
                                capts.get(2).unwrap().as_str().parse().unwrap_or(0);

                            last_bytes_copied = bytes_copied;

                            let _ = tx
                                .send(TransferStatus::InProgress {
                                    total_bytes: 0,
                                    bytes_copied,
                                    current_file: String::new(),
                                    percentage,
                                })
                                .await;
                        }

                        line_buffer.clear();
                    } else {
                        line_buffer.push(b);
                    }
                }

                let status = child_process
                    .wait()
                    .await
                    .map_err(|e| anyhow!("Failed to wait for rsync: {}", e))?;

                if status.success() {
                    let duration_secs = start_time.elapsed().as_secs();
                    info!(
                        total_bytes = last_bytes_copied,
                        duration_secs = duration_secs,
                        "Rsync transfer finished"
                    );
                    Ok(TransferResult {
                        total_bytes: last_bytes_copied,
                        duration_secs,
                    })
                } else {
                    let _ = tx
                        .send(TransferStatus::Failed(format!(
                            "Rsync failed with status: {}",
                            status
                        )))
                        .await;
                    Err(anyhow!("Rsync failed with status: {}", status))
                }
            }
            .instrument(span)
            .await
        })
    }
}
