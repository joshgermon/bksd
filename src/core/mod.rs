pub mod hardware;
pub mod models;
pub mod notifications;
pub mod orchestrator;
pub mod ownership;
pub mod progress;
pub mod transfer_engine;
pub mod verifier;

pub use hardware::{BlockDevice, HardwareAdapter, HardwareEvent};
pub use models::{Job, JobStatusEntry, JobWithHistory, TargetDrive};
pub use orchestrator::Orchestrator;
pub use ownership::{FileOwner, get_backup_owner};
pub use progress::ProgressTracker;
pub use verifier::{VerifyResult, verify_from_hashes};
