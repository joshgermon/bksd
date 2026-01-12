//! Web dashboard for BKSD.
//!
//! Serves a minimal terminal-like web UI for monitoring backup jobs.
//! Uses WebSocket for real-time updates with JSON-RPC 2.0 protocol.
//!
//! ## Architecture
//!
//! - `routes`: Axum router with HTTP and WebSocket endpoints
//! - `websocket`: WebSocket handler that dispatches to RPC method handlers
//!
//! ## Endpoints
//!
//! - `GET /` - Serves the embedded SPA dashboard
//! - `WS /ws` - WebSocket endpoint for JSON-RPC communication

mod websocket;

use axum::{Router, response::Html, routing::get};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::context::AppContext;
use crate::rpc::MethodHandler;

/// Embedded HTML dashboard
const INDEX_HTML: &str = include_str!("assets/index.html");

/// Shared state for the web server
#[derive(Clone)]
pub struct WebState {
    pub handler: Arc<MethodHandler>,
}

/// Web server for the dashboard UI.
pub struct WebServer {
    bind_addr: SocketAddr,
    state: WebState,
    shutdown_tx: broadcast::Sender<()>,
}

impl WebServer {
    /// Create a new web server bound to the given address.
    pub fn new(ctx: AppContext, bind_addr: SocketAddr) -> Self {
        let handler = Arc::new(MethodHandler::new(ctx));
        let state = WebState { handler };
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            bind_addr,
            state,
            shutdown_tx,
        }
    }

    /// Start the web server. Runs until shutdown() is called.
    pub async fn start(&self) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/", get(serve_index))
            .route("/ws", get(websocket::ws_handler))
            .with_state(self.state.clone());

        let listener = tokio::net::TcpListener::bind(self.bind_addr).await?;
        tracing::info!(addr = %self.bind_addr, "Web dashboard listening");

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.recv().await;
            })
            .await?;

        Ok(())
    }

    /// Signal the server to shut down gracefully.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

/// Serve the embedded index.html
async fn serve_index() -> Html<&'static str> {
    Html(INDEX_HTML)
}
