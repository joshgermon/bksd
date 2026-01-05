use crate::core::transfer_engine::{TransferEngine, TransferRequest, TransferStatus};
use anyhow::{anyhow, Result};
use regex::Regex;
use std::future::Future;
use std::pin::Pin;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::mpsc,
};

pub struct RsyncEngine;

impl TransferEngine for RsyncEngine {
    fn transfer(
        &self,
        req: &TransferRequest,
        tx: mpsc::Sender<TransferStatus>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        let req = req.clone();
        Box::pin(async move {
            let _ = tx.send(TransferStatus::Ready).await;

            let source = req.source.to_string_lossy();
            let destination = req.destination.to_string_lossy();

            // Create destination directory if it doesn't exist
            if let Err(e) = std::fs::create_dir_all(&req.destination) {
                let msg = format!("Failed to create destination directory: {}", e);
                let _ = tx.send(TransferStatus::Failed(msg.clone())).await;
                return Err(anyhow!(msg));
            }

            println!("(Rsync) Transferring {} to {}", source, destination);

            let mut child_process = Command::new("rsync")
                .arg("-av")
                .arg("--info=progress2")
                .arg("--no-inc-recursive")
                .arg(format!("{}/", source))  // trailing slash to copy contents
                .arg(destination.as_ref())
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

                        let percentage: u8 = capts.get(2).unwrap().as_str().parse().unwrap_or(0);

                        let _ = tx.send(TransferStatus::InProgress {
                            total_bytes: 0,
                            bytes_copied,
                            current_file: String::new(),
                            percentage,
                        }).await;
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
                let _ = tx.send(TransferStatus::Complete).await;
                Ok(())
            } else {
                let _ = tx.send(TransferStatus::Failed(format!(
                    "Rsync failed with status: {}",
                    status
                ))).await;
                Err(anyhow!("Rsync failed with status: {}", status))
            }
        })
    }
}
