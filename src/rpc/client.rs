//! RPC client for connecting to the daemon.
//!
//! Provides a simple client for sending JSON-RPC requests to the daemon.

use std::net::SocketAddr;

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::protocol::{Request, Response, RpcError};

/// RPC client for communicating with the daemon.
pub struct RpcClient {
    addr: SocketAddr,
}

/// Error returned by RPC client operations.
#[derive(Debug)]
pub enum ClientError {
    /// Failed to connect to daemon
    Connect(std::io::Error),
    /// Failed to send/receive data
    Io(std::io::Error),
    /// Failed to serialize request
    Serialize(serde_json::Error),
    /// Failed to parse response
    Parse(serde_json::Error),
    /// Server returned an error
    Rpc(RpcError),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Connect(e) => write!(f, "Failed to connect to daemon: {}", e),
            ClientError::Io(e) => write!(f, "Communication error: {}", e),
            ClientError::Serialize(e) => write!(f, "Failed to serialize request: {}", e),
            ClientError::Parse(e) => write!(f, "Failed to parse response: {}", e),
            ClientError::Rpc(e) => write!(f, "RPC error {}: {}", e.code, e.message),
        }
    }
}

impl std::error::Error for ClientError {}

impl RpcClient {
    /// Create a new client that will connect to the given address.
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr }
    }

    /// Call an RPC method and return the result.
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T, ClientError> {
        let mut stream = TcpStream::connect(self.addr)
            .await
            .map_err(ClientError::Connect)?;

        let request = Request {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(Value::Number(1.into())),
        };

        let mut request_json = serde_json::to_string(&request).map_err(ClientError::Serialize)?;
        request_json.push('\n');

        stream
            .write_all(request_json.as_bytes())
            .await
            .map_err(ClientError::Io)?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .await
            .map_err(ClientError::Io)?;

        let response: Response =
            serde_json::from_str(&response_line).map_err(ClientError::Parse)?;

        if let Some(error) = response.error {
            return Err(ClientError::Rpc(error));
        }

        let result = response.result.unwrap_or(Value::Null);
        serde_json::from_value(result).map_err(ClientError::Parse)
    }

    /// Call an RPC method with no parameters.
    pub async fn call_no_params<T: DeserializeOwned>(
        &self,
        method: &str,
    ) -> Result<T, ClientError> {
        self.call(method, None).await
    }
}
