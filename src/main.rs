use anyhow::{Context, Result};
use bksd::{config, context, core::Orchestrator, db};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "bksd")]
#[command(about = "Automated SD Card Backup System", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Daemon,
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = config::AppConfig {
        backup_directory: PathBuf::from("/tmp/bksd"),
        retry_attempts: 3,
        http_port: 8080,
        simulation: false,
        verbose: false,
    };

    let db_conn = db::init().await?;
    let ctx = context::AppContext::new(config, db_conn);

    match &cli.command {
        Commands::Daemon => run_daemon(ctx).await.context("Failed to start daemon")?,
        Commands::Status => run_status().context("Failed to check status of daemon")?,
    }

    Ok(())
}

async fn run_daemon(ctx: context::AppContext) -> Result<()> {
    Orchestrator::new(ctx).start().await
}

fn run_status() -> Result<()> {
    // Later, this will connect to the Unix Socket
    println!("Checking status of the daemon...");
    println!("(TODO: Implement IPC Client)");
    Ok(())
}
