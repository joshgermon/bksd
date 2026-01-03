#[derive(Debug, Clone)]
pub struct TargetDrive {
    pub uuid: String,
    pub backup_state: BackupState,
    pub label: String,
    pub mount_path: String,
    pub raw_size: u64,
}

#[derive(Debug, Clone)]
pub enum BackupState {
    Ready,
    InProgress { current: u64, total: u64 },
    CopyComplete,
    Verifying { current: u64, total: u64 },
    Complete,
    Failed,
}
