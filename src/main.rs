use anyhow::{Context, Result};
use bksd::core::transfer_engine::TransferEngineType;
use bksd::{config, context, core::Orchestrator, db};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;

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
    Daemon(ServerArgs),
    Status,
}

#[derive(Args, Serialize)]
struct ServerArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    backup_directory: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    transfer_engine: Option<TransferEngineType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    retry_attempts: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    verbose: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    simulation: Option<bool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = match &cli.command {
        Commands::Daemon(args) => config::AppConfig::new(Some(args))?,
        _ => config::AppConfig::new(None::<&ServerArgs>)?,
    };

    match &cli.command {
        Commands::Daemon(_) => {
            let db_conn = db::init().await?;
            let ctx = context::AppContext::new(config, db_conn);
            run_daemon(ctx).await.context("Failed to start daemon")?
        }
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
