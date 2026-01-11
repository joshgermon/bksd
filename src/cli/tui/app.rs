//! TUI application state and logic.

use std::collections::HashMap;
use std::net::SocketAddr;

use anyhow::Result;
use serde::Deserialize;

use crate::core::models::{Job, JobWithHistory};
use crate::core::transfer_engine::TransferStatus;
use crate::rpc::RpcClient;

/// Response type for daemon.status RPC call.
#[derive(Debug, Clone, Deserialize)]
pub struct DaemonStatus {
    pub version: String,
    pub uptime_secs: u64,
    pub active_jobs: usize,
    pub simulation: bool,
}

/// Response type for progress.active RPC call.
#[derive(Debug, Clone, Deserialize)]
pub struct ActiveProgress {
    pub jobs: HashMap<String, TransferStatus>,
    pub count: usize,
}

/// Cached data fetched from the daemon via RPC.
#[derive(Debug, Default)]
pub struct AppData {
    pub daemon_status: Option<DaemonStatus>,
    pub active_jobs: HashMap<String, TransferStatus>,
    pub recent_jobs: Vec<Job>,
    pub all_jobs: Vec<Job>,
    pub selected_job: Option<JobWithHistory>,
}

/// Current view being displayed.
#[derive(Debug, Clone)]
pub enum View {
    /// Main dashboard showing active transfer banner and recent jobs list.
    Dashboard {
        /// Selected index in recent jobs list
        selected: usize,
    },
    /// Full job history list.
    History {
        /// Selected job index
        selected: usize,
        /// Pagination offset
        offset: u32,
    },
    /// Single job detail view.
    Detail {
        /// Job ID being viewed
        job_id: String,
        /// Scroll offset for long content
        scroll: u16,
    },
}

impl Default for View {
    fn default() -> Self {
        View::Dashboard { selected: 0 }
    }
}

/// Actions that can be triggered by user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
    Refresh,
    History,
}

/// Main TUI application state.
pub struct TuiApp {
    client: RpcClient,
    pub view: View,
    pub data: AppData,
    pub running: bool,
    pub error: Option<String>,
}

impl TuiApp {
    /// Create a new TUI application connected to the daemon at the given address.
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            client: RpcClient::new(addr),
            view: View::default(),
            data: AppData::default(),
            running: true,
            error: None,
        }
    }

    /// Fetch initial data from the daemon.
    pub async fn init(&mut self) -> Result<()> {
        self.refresh_dashboard().await
    }

    /// Refresh dashboard data (daemon status, active jobs, recent jobs).
    pub async fn refresh_dashboard(&mut self) -> Result<()> {
        self.error = None;

        // Fetch daemon status
        match self.client.call_no_params::<DaemonStatus>("daemon.status").await {
            Ok(status) => self.data.daemon_status = Some(status),
            Err(e) => {
                self.error = Some(format!("Failed to connect: {}", e));
                return Ok(());
            }
        }

        // Fetch active progress
        match self.client.call_no_params::<ActiveProgress>("progress.active").await {
            Ok(progress) => self.data.active_jobs = progress.jobs,
            Err(e) => self.error = Some(format!("Failed to fetch progress: {}", e)),
        }

        // Fetch recent jobs
        match self
            .client
            .call::<Vec<Job>>("jobs.list", Some(serde_json::json!({ "limit": 20 })))
            .await
        {
            Ok(jobs) => self.data.recent_jobs = jobs,
            Err(e) => self.error = Some(format!("Failed to fetch jobs: {}", e)),
        }

        Ok(())
    }

    /// Refresh only active jobs (for polling during dashboard view).
    pub async fn refresh_active_jobs(&mut self) {
        if let Ok(progress) = self.client.call_no_params::<ActiveProgress>("progress.active").await
        {
            self.data.active_jobs = progress.jobs;
        }
    }

    /// Fetch all jobs for the history view.
    pub async fn fetch_history(&mut self, offset: u32) {
        match self
            .client
            .call::<Vec<Job>>(
                "jobs.list",
                Some(serde_json::json!({ "limit": 50, "offset": offset })),
            )
            .await
        {
            Ok(jobs) => self.data.all_jobs = jobs,
            Err(e) => self.error = Some(format!("Failed to fetch history: {}", e)),
        }
    }

    /// Fetch a single job's details.
    pub async fn fetch_job_detail(&mut self, job_id: &str) {
        match self
            .client
            .call::<JobWithHistory>("jobs.get", Some(serde_json::json!({ "id": job_id })))
            .await
        {
            Ok(job) => self.data.selected_job = Some(job),
            Err(e) => self.error = Some(format!("Failed to fetch job: {}", e)),
        }
    }

    /// Handle an action and update state accordingly.
    pub async fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => self.running = false,
            Action::Refresh => {
                let _ = self.refresh_dashboard().await;
            }
            Action::History => {
                self.fetch_history(0).await;
                self.view = View::History {
                    selected: 0,
                    offset: 0,
                };
            }
            Action::Back => {
                self.view = View::Dashboard { selected: 0 };
                let _ = self.refresh_dashboard().await;
            }
            Action::Up => self.navigate_up(),
            Action::Down => self.navigate_down(),
            Action::Left | Action::Right => {} // No-op in simplified UI
            Action::Select => self.select_item().await,
        }
    }

    fn navigate_up(&mut self) {
        match &self.view {
            View::Dashboard { selected } => {
                if *selected > 0 {
                    self.view = View::Dashboard {
                        selected: *selected - 1,
                    };
                }
            }
            View::History { selected, offset } => {
                if *selected > 0 {
                    self.view = View::History {
                        selected: *selected - 1,
                        offset: *offset,
                    };
                }
            }
            View::Detail { job_id, scroll } => {
                self.view = View::Detail {
                    job_id: job_id.clone(),
                    scroll: scroll.saturating_sub(1),
                };
            }
        }
    }

    fn navigate_down(&mut self) {
        match &self.view {
            View::Dashboard { selected } => {
                if *selected + 1 < self.data.recent_jobs.len() {
                    self.view = View::Dashboard {
                        selected: *selected + 1,
                    };
                }
            }
            View::History { selected, offset } => {
                if *selected + 1 < self.data.all_jobs.len() {
                    self.view = View::History {
                        selected: *selected + 1,
                        offset: *offset,
                    };
                }
            }
            View::Detail { job_id, scroll } => {
                self.view = View::Detail {
                    job_id: job_id.clone(),
                    scroll: *scroll + 1,
                };
            }
        }
    }

    async fn select_item(&mut self) {
        match &self.view {
            View::Dashboard { selected } => {
                if let Some(job) = self.data.recent_jobs.get(*selected) {
                    let id = job.id.clone();
                    self.fetch_job_detail(&id).await;
                    self.view = View::Detail {
                        job_id: id,
                        scroll: 0,
                    };
                }
            }
            View::History { selected, .. } => {
                if let Some(job) = self.data.all_jobs.get(*selected) {
                    let id = job.id.clone();
                    self.fetch_job_detail(&id).await;
                    self.view = View::Detail {
                        job_id: id,
                        scroll: 0,
                    };
                }
            }
            View::Detail { .. } => {
                // No action on select in detail view
            }
        }
    }
}
