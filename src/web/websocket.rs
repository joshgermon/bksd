//! WebSocket handler for JSON-RPC communication.

use axum::{
    extract::ws::{Message, WebSocket},
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
};

use super::WebState;
use crate::rpc::Request;

/// Handle WebSocket upgrade requests
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WebState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle an established WebSocket connection
async fn handle_socket(mut socket: WebSocket, state: WebState) {
    while let Some(msg) = socket.recv().await {
        let msg = match msg {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Ok(_) => continue, // Ignore binary, ping, pong
            Err(e) => {
                tracing::debug!(error = %e, "WebSocket receive error");
                break;
            }
        };

        // Parse JSON-RPC request
        let response = match serde_json::from_str::<Request>(&msg) {
            Ok(request) => {
                let response = state.handler.handle(request).await;
                serde_json::to_string(&response).unwrap_or_else(|_| {
                    r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Serialization error"},"id":null}"#.to_string()
                })
            }
            Err(e) => {
                // Invalid JSON or malformed request
                format!(
                    r#"{{"jsonrpc":"2.0","error":{{"code":-32700,"message":"Parse error: {}"}},"id":null}}"#,
                    e.to_string().replace('"', "'")
                )
            }
        };

        if socket.send(Message::Text(response.into())).await.is_err() {
            break;
        }
    }

    tracing::debug!("WebSocket connection closed");
}
