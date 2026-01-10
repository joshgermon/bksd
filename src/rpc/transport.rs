//! TCP transport layer for the RPC server.
//!
//! Handles TCP connections with newline-delimited JSON framing.
//! Each connection is handled in its own task.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use super::methods::MethodHandler;
use super::protocol::{Request, Response};

/// Manages the TCP transport layer.
pub struct Transport {
    bind_addr: SocketAddr,
    handler: Arc<MethodHandler>,
}

impl Transport {
    pub fn new(bind_addr: SocketAddr, handler: MethodHandler) -> Self {
        Self {
            bind_addr,
            handler: Arc::new(handler),
        }
    }

    /// Start listening for connections. Runs until shutdown signal is received.
    pub async fn listen(&self, mut shutdown: broadcast::Receiver<()>) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.bind_addr).await?;
        info!(addr = %self.bind_addr, "RPC server listening");

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, peer_addr)) => {
                            debug!(peer = %peer_addr, "Client connected");
                            let handler = self.handler.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, peer_addr, handler).await {
                                    debug!(peer = %peer_addr, error = %e, "Connection error");
                                }
                                debug!(peer = %peer_addr, "Client disconnected");
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "Failed to accept connection");
                        }
                    }
                }
                _ = shutdown.recv() => {
                    info!("RPC server shutting down");
                    break;
                }
            }
        }

        Ok(())
    }
}

/// Handle a single client connection.
async fn handle_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    handler: Arc<MethodHandler>,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;

        if bytes_read == 0 {
            // EOF - client disconnected
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(request) => {
                // Validate JSON-RPC 2.0 format
                if let Err(msg) = request.validate() {
                    warn!(peer = %peer_addr, error = msg, "Invalid request");
                    let id = request.id.clone().unwrap_or(serde_json::Value::Null);
                    Response::invalid_request(id)
                } else if request.is_notification() {
                    // Notifications don't get responses
                    debug!(peer = %peer_addr, method = %request.method, "Notification received");
                    handler.handle(request).await;
                    continue;
                } else {
                    // Normal request
                    handler.handle(request).await
                }
            }
            Err(e) => {
                warn!(peer = %peer_addr, error = %e, "Parse error");
                Response::parse_error()
            }
        };

        let mut response_json = serde_json::to_string(&response)?;
        response_json.push('\n');
        writer.write_all(response_json.as_bytes()).await?;
    }

    Ok(())
}
