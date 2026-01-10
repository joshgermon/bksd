//! JSON-RPC 2.0 protocol types.
//!
//! Implements the JSON-RPC 2.0 specification for request/response messaging.
//! See: https://www.jsonrpc.org/specification

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Must be exactly "2.0"
    pub jsonrpc: String,
    /// Method name to invoke
    pub method: String,
    /// Optional parameters (can be object or array)
    #[serde(default)]
    pub params: Option<Value>,
    /// Request identifier. If None, this is a notification (no response expected).
    #[serde(default)]
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 response object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Always "2.0"
    pub jsonrpc: String,
    /// Result on success (mutually exclusive with error)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error on failure (mutually exclusive with result)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    /// Request identifier (echoed from request)
    pub id: Value,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// Error code (see standard codes below)
    pub code: i32,
    /// Short error description
    pub message: String,
    /// Optional additional error data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// Standard JSON-RPC 2.0 error codes
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

impl Response {
    /// Create a success response with the given result.
    pub fn success(id: Value, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(result).unwrap_or(Value::Null)),
            error: None,
            id,
        }
    }

    /// Create an error response.
    pub fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }

    /// Create an error response with additional data.
    pub fn error_with_data(
        id: Value,
        code: i32,
        message: impl Into<String>,
        data: impl Serialize,
    ) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: Some(serde_json::to_value(data).unwrap_or(Value::Null)),
            }),
            id,
        }
    }

    /// Create a parse error response (used when request ID is unknown).
    pub fn parse_error() -> Self {
        Self::error(Value::Null, PARSE_ERROR, "Parse error")
    }

    /// Create an invalid request response.
    pub fn invalid_request(id: Value) -> Self {
        Self::error(id, INVALID_REQUEST, "Invalid request")
    }

    /// Create a method not found response.
    pub fn method_not_found(id: Value, method: &str) -> Self {
        Self::error(
            id,
            METHOD_NOT_FOUND,
            format!("Method not found: {}", method),
        )
    }

    /// Create an invalid params response.
    pub fn invalid_params(id: Value, details: impl Into<String>) -> Self {
        Self::error(id, INVALID_PARAMS, details.into())
    }

    /// Create an internal error response.
    pub fn internal_error(id: Value, details: impl Into<String>) -> Self {
        Self::error(id, INTERNAL_ERROR, details.into())
    }
}

impl Request {
    /// Check if this request is a notification (no response expected).
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }

    /// Validate the request conforms to JSON-RPC 2.0.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.jsonrpc != "2.0" {
            return Err("jsonrpc must be \"2.0\"");
        }
        if self.method.is_empty() {
            return Err("method must not be empty");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_request() {
        let json = r#"{"jsonrpc":"2.0","method":"jobs.list","params":{"limit":10},"id":1}"#;
        let req: Request = serde_json::from_str(json).unwrap();

        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "jobs.list");
        assert!(req.params.is_some());
        assert_eq!(req.id, Some(Value::Number(1.into())));
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_parse_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"ping"}"#;
        let req: Request = serde_json::from_str(json).unwrap();

        assert!(req.is_notification());
        assert!(req.params.is_none());
    }

    #[test]
    fn test_serialize_success_response() {
        let resp = Response::success(Value::Number(1.into()), "ok");
        let json = serde_json::to_string(&resp).unwrap();

        assert!(json.contains(r#""jsonrpc":"2.0""#));
        assert!(json.contains(r#""result":"ok""#));
        assert!(json.contains(r#""id":1"#));
        assert!(!json.contains("error"));
    }

    #[test]
    fn test_serialize_error_response() {
        let resp = Response::method_not_found(Value::String("abc".into()), "unknown.method");
        let json = serde_json::to_string(&resp).unwrap();

        assert!(json.contains(r#""code":-32601"#));
        assert!(json.contains("Method not found"));
        assert!(!json.contains("result"));
    }
}
