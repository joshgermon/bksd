use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetDrive {
    pub uuid: String,
    pub label: String,
    pub mount_path: String,
    pub raw_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub target_id: String,
    pub destination_path: Option<String>,
    pub created_at: String,
    pub status: String,
}

/// A single entry from the job status log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatusEntry {
    pub id: String,
    pub status: String,
    pub description: Option<String>,
    pub total_bytes: Option<u64>,
    pub duration_secs: Option<u64>,
    pub created_at: String,
}

/// Job with full status history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobWithHistory {
    #[serde(flatten)]
    pub job: Job,
    pub history: Vec<JobStatusEntry>,
}
