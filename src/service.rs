use crate::config::AppConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

const SERVICE_PATH: &str = "/etc/systemd/system/bksd.service";
const CONFIG_DIR: &str = "/etc/bksd";
const CONFIG_PATH: &str = "/etc/bksd/config.toml";
const DATA_DIR: &str = "/var/lib/bksd";

const SERVICE_TEMPLATE: &str = r#"[Unit]
Description=Backup Sentinel Daemon
After=local-fs.target

[Service]
Type=simple
ExecStart={binary_path} start {backup_dir} --foreground
Restart=always
RestartSec=5

StartLimitBurst=5
StartLimitIntervalSec=60

ProtectSystem=strict
PrivateTmp=true
ReadWritePaths=/run/bksd /var/lib/bksd {backup_dir}

[Install]
WantedBy=multi-user.target
"#;

pub struct ServiceManager {
    service_path: PathBuf,
    config_path: PathBuf,
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceManager {
    pub fn new() -> Self {
        Self {
            service_path: PathBuf::from(SERVICE_PATH),
            config_path: PathBuf::from(CONFIG_PATH),
        }
    }

    pub fn is_installed(&self) -> bool {
        self.service_path.exists()
    }

    pub fn is_running(&self) -> Result<bool> {
        let output = Command::new("systemctl")
            .args(["is-active", "bksd"])
            .output()
            .context("Failed to check service status")?;

        Ok(output.status.success())
    }

    pub fn load_current_config(&self) -> Result<Option<AppConfig>> {
        if !self.config_path.exists() {
            return Ok(None);
        }

        let content =
            std::fs::read_to_string(&self.config_path).context("Failed to read config file")?;

        let config: AppConfig = toml::from_str(&content).context("Failed to parse config file")?;

        Ok(Some(config))
    }

    pub fn install_and_start(&self, config: &AppConfig) -> Result<()> {
        self.create_directories()?;
        self.write_config(config)?;
        self.write_service_file(config)?;
        self.daemon_reload()?;
        self.enable()?;
        self.start()?;
        Ok(())
    }

    pub fn update_config_and_restart(&self, config: &AppConfig) -> Result<()> {
        self.write_config(config)?;
        self.write_service_file(config)?;
        self.daemon_reload()?;
        self.restart()?;
        Ok(())
    }

    pub fn start(&self) -> Result<()> {
        let status = Command::new("systemctl")
            .args(["start", "bksd"])
            .status()
            .context("Failed to start service")?;

        if !status.success() {
            anyhow::bail!("systemctl start bksd failed");
        }
        Ok(())
    }

    fn restart(&self) -> Result<()> {
        let status = Command::new("systemctl")
            .args(["restart", "bksd"])
            .status()
            .context("Failed to restart service")?;

        if !status.success() {
            anyhow::bail!("systemctl restart bksd failed");
        }
        Ok(())
    }

    fn create_directories(&self) -> Result<()> {
        std::fs::create_dir_all(CONFIG_DIR).context("Failed to create /etc/bksd directory")?;
        std::fs::create_dir_all(DATA_DIR).context("Failed to create /var/lib/bksd directory")?;
        Ok(())
    }

    fn write_config(&self, config: &AppConfig) -> Result<()> {
        let content = toml::to_string_pretty(config).context("Failed to serialize config")?;

        std::fs::write(&self.config_path, content).context("Failed to write config file")?;

        Ok(())
    }

    fn write_service_file(&self, config: &AppConfig) -> Result<()> {
        let binary_path = std::env::current_exe().context("Failed to determine binary path")?;

        let backup_dir = config.backup_directory.display().to_string();

        let service_content = SERVICE_TEMPLATE
            .replace("{binary_path}", &binary_path.display().to_string())
            .replace("{backup_dir}", &backup_dir);

        std::fs::write(&self.service_path, service_content)
            .context("Failed to write service file")?;

        Ok(())
    }

    fn daemon_reload(&self) -> Result<()> {
        let status = Command::new("systemctl")
            .arg("daemon-reload")
            .status()
            .context("Failed to reload systemd")?;

        if !status.success() {
            anyhow::bail!("systemctl daemon-reload failed");
        }
        Ok(())
    }

    fn enable(&self) -> Result<()> {
        let status = Command::new("systemctl")
            .args(["enable", "bksd"])
            .status()
            .context("Failed to enable service")?;

        if !status.success() {
            anyhow::bail!("systemctl enable bksd failed");
        }
        Ok(())
    }
}

pub fn configs_differ(a: &AppConfig, b: &AppConfig) -> bool {
    a.backup_directory != b.backup_directory
        || a.transfer_engine != b.transfer_engine
        || a.verify_transfers != b.verify_transfers
        || a.simulation != b.simulation
}

pub fn prompt_restart(current: &AppConfig, new: &AppConfig) -> Result<bool> {
    use std::io::{Write, stdin, stdout};

    println!("bksd is already running with a different configuration.\n");
    println!("  Current: {}", current.backup_directory.display());
    println!("  New:     {}", new.backup_directory.display());
    println!();
    print!("Restart with new config? [y/N] ");
    stdout().flush()?;

    let mut input = String::new();
    stdin().read_line(&mut input)?;

    Ok(input.trim().eq_ignore_ascii_case("y"))
}
