//! RPC server for client communication.
//!
//! Provides a JSON-RPC 2.0 interface over TCP for querying job status,
//! active progress, and daemon information.
//!
//! ## Architecture
//!
//! - `protocol`: JSON-RPC 2.0 request/response types
//! - `transport`: TCP listener with newline-delimited JSON framing
//! - `methods`: Method dispatcher and handlers
//! - `client`: Client for connecting to the daemon
//!
//! ## Extensibility
//!
//! The architecture supports future enhancements:
//! - Push notifications: Server can send JSON-RPC notifications to connected clients
//! - Subscriptions: Add `subscribe.*` methods for real-time event streaming

pub mod client;
pub mod methods;
mod protocol;
mod transport;

use std::net::SocketAddr;
use tokio::sync::broadcast;

use crate::context::AppContext;
use transport::Transport;

pub use client::RpcClient;
pub use methods::MethodHandler;
pub use protocol::{Request, Response, RpcError};

/// RPC server that exposes daemon functionality to clients.
pub struct RpcServer {
    transport: Transport,
    shutdown_tx: broadcast::Sender<()>,
}

impl RpcServer {
    /// Create a new RPC server bound to the given address.
    pub fn new(ctx: AppContext, bind_addr: SocketAddr) -> Self {
        let handler = MethodHandler::new(ctx);
        let transport = Transport::new(bind_addr, handler);
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            transport,
            shutdown_tx,
        }
    }

    /// Start the RPC server. Runs until shutdown() is called.
    pub async fn start(&self) -> anyhow::Result<()> {
        let shutdown_rx = self.shutdown_tx.subscribe();
        self.transport.listen(shutdown_rx).await
    }

    /// Signal the server to shut down gracefully.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}
