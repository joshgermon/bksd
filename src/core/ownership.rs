use nix::unistd::{Gid, Group, Uid, User};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use tracing::{debug, warn};

/// Represents file ownership as user:group
#[derive(Debug, Clone)]
pub struct FileOwner {
    pub user: String,
    pub group: String,
}

impl FileOwner {
    /// Format as "user:group" for use with rsync --chown
    pub fn as_chown_arg(&self) -> String {
        format!("{}:{}", self.user, self.group)
    }
}

/// Determine the appropriate owner for backup files.
///
/// Detection order:
/// 1. `SUDO_USER` environment variable - the user who invoked sudo
/// 2. Owner of the backup directory - fallback if SUDO_USER not set
///
/// Returns None if ownership cannot be determined (files will be owned by root).
pub fn get_backup_owner(backup_dir: &Path) -> Option<FileOwner> {
    // Try SUDO_USER first - this is the user who ran "sudo bksd"
    if let Some(owner) = get_owner_from_sudo_user() {
        debug!(user = %owner.user, group = %owner.group, "Detected backup owner from SUDO_USER");
        return Some(owner);
    }

    // Fallback: use owner of the backup directory
    if let Some(owner) = get_owner_from_path(backup_dir) {
        debug!(
            user = %owner.user,
            group = %owner.group,
            path = %backup_dir.display(),
            "Detected backup owner from backup directory"
        );
        return Some(owner);
    }

    warn!("Could not determine backup owner - files will be owned by root");
    None
}

/// Get owner from SUDO_USER environment variable
fn get_owner_from_sudo_user() -> Option<FileOwner> {
    let sudo_user = std::env::var("SUDO_USER").ok()?;

    if sudo_user.is_empty() {
        return None;
    }

    // Look up the user to get their primary group
    let user = User::from_name(&sudo_user).ok()??;
    let group = Group::from_gid(user.gid).ok()??;

    Some(FileOwner {
        user: sudo_user,
        group: group.name,
    })
}

/// Get owner from the uid/gid of a filesystem path
fn get_owner_from_path(path: &Path) -> Option<FileOwner> {
    let metadata = std::fs::metadata(path).ok()?;

    let uid = Uid::from_raw(metadata.uid());
    let gid = Gid::from_raw(metadata.gid());

    let user = User::from_uid(uid).ok()??;
    let group = Group::from_gid(gid).ok()??;

    Some(FileOwner {
        user: user.name,
        group: group.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_owner_as_chown_arg() {
        let owner = FileOwner {
            user: "joshua".to_string(),
            group: "users".to_string(),
        };
        assert_eq!(owner.as_chown_arg(), "joshua:users");
    }

    #[test]
    fn test_get_owner_from_path_current_dir() {
        // Current directory should have a valid owner
        let owner = get_owner_from_path(Path::new("."));
        assert!(owner.is_some());
    }

    #[test]
    fn test_get_owner_from_path_nonexistent() {
        let owner = get_owner_from_path(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(owner.is_none());
    }
}
