//! RPC method handlers.
//!
//! Dispatches JSON-RPC method calls to the appropriate handler functions.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

use crate::context::AppContext;
use crate::core::transfer_engine::TransferStatus;
use crate::db;

use super::protocol::{Request, Response};

/// Handles RPC method dispatch and execution.
pub struct MethodHandler {
    ctx: AppContext,
    start_time: Instant,
}

impl MethodHandler {
    pub fn new(ctx: AppContext) -> Self {
        Self {
            ctx,
            start_time: Instant::now(),
        }
    }

    /// Handle an RPC request and return a response.
    pub async fn handle(&self, request: Request) -> Response {
        let id = request.id.clone().unwrap_or(Value::Null);
        let params = request.params.unwrap_or(Value::Null);

        match request.method.as_str() {
            "daemon.status" => self.daemon_status(id).await,
            "jobs.list" => self.jobs_list(id, params).await,
            "jobs.get" => self.jobs_get(id, params).await,
            "progress.active" => self.progress_active(id).await,
            "progress.get" => self.progress_get(id, params).await,
            _ => Response::method_not_found(id, &request.method),
        }
    }

    /// Get daemon status/health information.
    async fn daemon_status(&self, id: Value) -> Response {
        let active_jobs = self.ctx.progress.active_count().await;
        let uptime_secs = self.start_time.elapsed().as_secs();

        #[derive(Serialize)]
        struct DaemonStatus {
            version: &'static str,
            uptime_secs: u64,
            active_jobs: usize,
            rpc_bind: String,
            simulation: bool,
        }

        Response::success(
            id,
            DaemonStatus {
                version: env!("CARGO_PKG_VERSION"),
                uptime_secs,
                active_jobs,
                rpc_bind: self.ctx.config.rpc_bind.to_string(),
                simulation: self.ctx.config.simulation,
            },
        )
    }

    /// List jobs with optional filtering and pagination.
    async fn jobs_list(&self, id: Value, params: Value) -> Response {
        #[derive(Deserialize, Default)]
        struct Params {
            #[serde(default)]
            limit: Option<u32>,
            #[serde(default)]
            offset: Option<u32>,
            #[serde(default)]
            status: Option<String>,
        }

        let params: Params = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => return Response::invalid_params(id, e.to_string()),
        };

        let limit = params.limit.unwrap_or(50);
        let offset = params.offset.unwrap_or(0);

        match db::jobs::list(&self.ctx.db, limit, offset, params.status).await {
            Ok(jobs) => Response::success(id, jobs),
            Err(e) => Response::internal_error(id, e.to_string()),
        }
    }

    /// Get a single job with its full status history.
    async fn jobs_get(&self, id: Value, params: Value) -> Response {
        #[derive(Deserialize)]
        struct Params {
            id: String,
        }

        let params: Params = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => return Response::invalid_params(id, e.to_string()),
        };

        match db::jobs::get_with_history(&self.ctx.db, params.id).await {
            Ok(job) => Response::success(id, job),
            Err(e) => Response::internal_error(id, e.to_string()),
        }
    }

    /// Get all active jobs with their current progress.
    async fn progress_active(&self, id: Value) -> Response {
        let progress = self.ctx.progress.get_all().await;

        #[derive(Serialize)]
        struct ActiveProgress {
            jobs: HashMap<String, TransferStatus>,
            count: usize,
        }

        let count = progress.len();
        Response::success(
            id,
            ActiveProgress {
                jobs: progress,
                count,
            },
        )
    }

    /// Get progress for a single job.
    async fn progress_get(&self, id: Value, params: Value) -> Response {
        #[derive(Deserialize)]
        struct Params {
            id: String,
        }

        let params: Params = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => return Response::invalid_params(id, e.to_string()),
        };

        match self.ctx.progress.get(&params.id).await {
            Some(status) => Response::success(id, status),
            None => Response::error(
                id,
                -32000,
                format!("Job not found or not active: {}", params.id),
            ),
        }
    }
}
