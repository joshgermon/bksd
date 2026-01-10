//! In-memory progress tracking for active jobs.
//!
//! This module provides a thread-safe store for live transfer progress.
//! Progress is updated frequently during transfers but is NOT persisted to the database.
//! Only state transitions are written to the database for historical records.
//!
//! Future IPC/TCP endpoints can query this tracker for real-time progress updates.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::transfer_engine::TransferStatus;

/// Thread-safe in-memory store for active job progress.
///
/// This is designed to be shared across the application via `AppContext`.
/// It holds the current `TransferStatus` for all active jobs, allowing
/// real-time progress queries without database access.
#[derive(Clone, Default)]
pub struct ProgressTracker {
    inner: Arc<RwLock<HashMap<String, TransferStatus>>>,
}

impl ProgressTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Update the progress for a job. Called on every progress tick from transfer engines.
    pub async fn update(&self, job_id: &str, status: TransferStatus) {
        let mut map = self.inner.write().await;
        map.insert(job_id.to_string(), status);
    }

    /// Get the current progress for a specific job.
    pub async fn get(&self, job_id: &str) -> Option<TransferStatus> {
        let map = self.inner.read().await;
        map.get(job_id).cloned()
    }

    /// Remove a job from tracking (called when job completes or fails).
    pub async fn remove(&self, job_id: &str) {
        let mut map = self.inner.write().await;
        map.remove(job_id);
    }

    /// Get all currently active jobs and their progress.
    pub async fn get_all(&self) -> HashMap<String, TransferStatus> {
        let map = self.inner.read().await;
        map.clone()
    }

    /// Get the number of currently active jobs.
    pub async fn active_count(&self) -> usize {
        let map = self.inner.read().await;
        map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_progress_tracker_basic_operations() {
        let tracker = ProgressTracker::new();

        // Initially empty
        assert_eq!(tracker.active_count().await, 0);
        assert!(tracker.get("job-1").await.is_none());

        // Add a job
        tracker
            .update(
                "job-1",
                TransferStatus::InProgress {
                    total_bytes: 1000,
                    bytes_copied: 500,
                    current_file: "test.txt".to_string(),
                    percentage: 50,
                },
            )
            .await;

        assert_eq!(tracker.active_count().await, 1);
        let status = tracker.get("job-1").await.unwrap();
        match status {
            TransferStatus::InProgress { percentage, .. } => assert_eq!(percentage, 50),
            _ => panic!("Expected InProgress status"),
        }

        tracker
            .update(
                "job-1",
                TransferStatus::InProgress {
                    total_bytes: 1000,
                    bytes_copied: 750,
                    current_file: "test.txt".to_string(),
                    percentage: 75,
                },
            )
            .await;

        let status = tracker.get("job-1").await.unwrap();
        match status {
            TransferStatus::InProgress { percentage, .. } => assert_eq!(percentage, 75),
            _ => panic!("Expected InProgress status"),
        }

        // Remove the job
        tracker.remove("job-1").await;
        assert_eq!(tracker.active_count().await, 0);
        assert!(tracker.get("job-1").await.is_none());
    }

    #[tokio::test]
    async fn test_progress_tracker_multiple_jobs() {
        let tracker = ProgressTracker::new();

        tracker.update("job-1", TransferStatus::Ready).await;
        tracker
            .update(
                "job-2",
                TransferStatus::InProgress {
                    total_bytes: 1000,
                    bytes_copied: 500,
                    current_file: "file.txt".to_string(),
                    percentage: 50,
                },
            )
            .await;
        tracker.update("job-3", TransferStatus::CopyComplete).await;

        assert_eq!(tracker.active_count().await, 3);

        let all = tracker.get_all().await;
        assert_eq!(all.len(), 3);
        assert!(all.contains_key("job-1"));
        assert!(all.contains_key("job-2"));
        assert!(all.contains_key("job-3"));
    }
}
