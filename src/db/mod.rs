use anyhow::Result;
use std::path::Path;
use tokio_rusqlite::Connection;

pub mod jobs;

/// Default directory for bksd persistent data (database).
pub const DATA_DIR: &str = "/var/lib/bksd";

/// Database filename within the data directory.
const DB_FILENAME: &str = "bksd.db";

pub async fn init() -> Result<Connection> {
    let data_dir = Path::new(DATA_DIR);

    // Create data directory if it doesn't exist
    if !data_dir.exists() {
        std::fs::create_dir_all(data_dir)?;
    }

    let db_path = data_dir.join(DB_FILENAME);
    let conn = Connection::open(&db_path).await?;

    conn.call(|conn| {
        let schema = include_str!("schema.sql");
        conn.execute_batch(schema)?;

        // Enable foreign keys (SQLite disables them by default!)
        conn.execute("PRAGMA foreign_keys = ON;", [])?;

        Ok::<(), tokio_rusqlite::rusqlite::Error>(())
    })
    .await?;

    Ok(conn)
}
