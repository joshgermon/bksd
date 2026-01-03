use tokio_rusqlite::Connection;
use anyhow::Result;

pub async fn init() -> Result<Connection> {
    let conn = Connection::open("backup_system.db").await?;

    conn.call(|conn| {
        let schema = include_str!("schema.sql");
        conn.execute_batch(schema)?;
        
        // Enable foreign keys (SQLite disables them by default!)
        conn.execute("PRAGMA foreign_keys = ON;", [])?;
        
        Ok::<(), tokio_rusqlite::rusqlite::Error>(())
    }).await?;

    Ok(conn)
}
