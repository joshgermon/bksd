#[derive(Debug, Clone)]
pub struct TargetDrive {
    pub uuid: String,
    pub label: String,
    pub mount_path: String,
    pub raw_size: u64,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub target_id: String,
    pub destination_path: Option<String>,
    pub created_at: String,
    pub status: String,
}
