use anyhow::{Context, Result};
use bksd::core::transfer_engine::TransferEngineType;
use bksd::logging::{self, LogConfig};
use bksd::rpc::{RpcClient, RpcServer};
use bksd::{config, context, core::Orchestrator, db};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "bksd")]
#[command(about = "Automated SD Card Backup System", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(long, global = true)]
    simulation: Option<bool>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the backup daemon
    Daemon(ServerArgs),
    /// Query daemon status and active jobs
    Status(StatusArgs),
}

#[derive(Args)]
struct StatusArgs {
    /// Address of the daemon RPC server
    #[arg(short, long, default_value = "127.0.0.1:9847")]
    addr: SocketAddr,
}

#[derive(Args, Serialize)]
struct ServerArgs {
    #[arg(short = 'd', long)]
    backup_directory: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(short = 'e', long)]
    transfer_engine: Option<TransferEngineType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(short = 'r', long)]
    retry_attempts: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(short = 'v', long)]
    verbose: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(short = 's', long)]
    simulation: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(short = 'm', long)]
    mount_base: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = match &cli.command {
        Commands::Daemon(args) => config::AppConfig::new(Some(args))?,
        _ => config::AppConfig::new(None::<&ServerArgs>)?,
    };

    logging::init(LogConfig {
        json: config.log_json,
        verbose: config.verbose,
    });

    match &cli.command {
        Commands::Daemon(_) => {
            #[cfg(target_os = "linux")]
            if !config.simulation {
                check_root_privileges()?;
            }

            let db_conn = db::init().await?;
            let ctx = context::AppContext::new(config, db_conn);
            run_daemon(ctx).await.context("Failed to start daemon")?
        }
        Commands::Status(args) => run_status(args.addr)
            .await
            .context("Failed to check status of daemon")?,
    }

    Ok(())
}

async fn run_daemon(ctx: context::AppContext) -> Result<()> {
    let rpc_server = if ctx.config.rpc_enabled {
        let server = Arc::new(RpcServer::new(ctx.clone(), ctx.config.rpc_bind));
        let server_clone = server.clone();
        let server_handle = tokio::spawn(async move {
            if let Err(e) = server_clone.start().await {
                tracing::error!(error = %e, "RPC server error");
            }
        });
        Some((server, server_handle))
    } else {
        None
    };

    let result = Orchestrator::new(ctx).start().await;

    // Shutdown RPC server
    if let Some((server, handle)) = rpc_server {
        server.shutdown();
        handle.abort();
    }

    result
}

async fn run_status(addr: SocketAddr) -> Result<()> {
    let client = RpcClient::new(addr);

    #[derive(Deserialize)]
    struct DaemonStatus {
        version: String,
        uptime_secs: u64,
        active_jobs: usize,
        simulation: bool,
    }

    let status: DaemonStatus = client
        .call_no_params("daemon.status")
        .await
        .context("Failed to connect to daemon. Is it running?")?;

    println!("Daemon Status");
    println!("  Version:     {}", status.version);
    println!("  Uptime:      {}s", status.uptime_secs);
    println!(
        "  Mode:        {}",
        if status.simulation {
            "simulation"
        } else {
            "production"
        }
    );
    println!("  Active Jobs: {}", status.active_jobs);

    // If there are active jobs, show their progress
    if status.active_jobs > 0 {
        #[derive(Deserialize)]
        struct ActiveProgress {
            jobs: HashMap<String, serde_json::Value>,
            #[allow(dead_code)]
            count: usize,
        }

        let progress: ActiveProgress = client.call_no_params("progress.active").await?;

        println!("\nActive Transfers:");
        for (job_id, status) in progress.jobs {
            let state = status
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");

            let job_short = &job_id[..8];

            match state {
                "in_progress" => {
                    let pct = status
                        .get("percentage")
                        .and_then(|p| p.as_u64())
                        .unwrap_or(0) as u8;
                    let file = status
                        .get("current_file")
                        .and_then(|f| f.as_str())
                        .unwrap_or("");
                    let bar = progress_bar(pct, 20);
                    println!("  {} {} {:>3}% {}", job_short, bar, pct, file);
                }
                "verifying" => {
                    let current = status.get("current").and_then(|c| c.as_u64()).unwrap_or(0);
                    let total = status.get("total").and_then(|t| t.as_u64()).unwrap_or(0);
                    let pct = if total > 0 {
                        ((current * 100) / total) as u8
                    } else {
                        100
                    };
                    let bar = progress_bar(pct, 20);
                    println!(
                        "  {} {} {:>3}% verifying {}/{}",
                        job_short, bar, pct, current, total
                    );
                }
                _ => {
                    println!("  {} - {}", job_short, state);
                }
            }
        }
    }

    Ok(())
}

/// Generate an ASCII progress bar
fn progress_bar(percentage: u8, width: usize) -> String {
    let percentage = percentage.min(100) as usize;
    let filled = (percentage * width) / 100;
    let empty = width - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

#[cfg(target_os = "linux")]
fn check_root_privileges() -> Result<()> {
    use nix::unistd::Uid;

    if !Uid::effective().is_root() {
        anyhow::bail!(
            "The daemon requires root privileges for mounting devices.\n\
             Run with: sudo cargo run daemon\n\
             Or use --simulation for testing without real devices."
        );
    }
    Ok(())
}
