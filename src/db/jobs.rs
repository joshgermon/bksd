use anyhow::{Result, anyhow};
use tokio_rusqlite::{Connection, params, rusqlite};
use uuid::Uuid;

use crate::core::{Job, TargetDrive};

pub async fn create(
    conn: &Connection,
    job_id: String,
    drive: TargetDrive,
    destination_path: String,
) -> Result<()> {
    conn.call(move |c| {
        let tx = c.transaction()?;

        tx.execute(
            "INSERT INTO targets (id, label, raw_size, adapter, source)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                label = excluded.label,
                source = excluded.source,
                created_at = CURRENT_TIMESTAMP",
            params![
                &drive.uuid,
                &drive.label,
                drive.raw_size,
                "SIMULATED",
                &drive.mount_path
            ],
        )?;

        tx.execute(
            "INSERT INTO jobs (id, target_id, destination_path)
             VALUES (?1, ?2, ?3)",
            params![&job_id, &drive.uuid, &destination_path],
        )?;

        let log_id = Uuid::now_v7().to_string();
        tx.execute(
            "INSERT INTO job_status_log (id, job_id, status, description)
             VALUES (?1, ?2, 'Ready', 'Job created waiting for processor')",
            params![log_id, &job_id],
        )?;

        tx.commit()?;
        Ok::<(), rusqlite::Error>(())
    })
    .await?;

    Ok(())
}

pub async fn get(conn: &Connection, job_id: String) -> Result<Job> {
    conn.call(move |c| {
        let mut stmt = c.prepare(
            "SELECT j.id, j.target_id, j.destination_path, j.created_at,
             COALESCE((SELECT status FROM job_status_log WHERE job_id = j.id ORDER BY created_at DESC LIMIT 1), 'Unknown') as status
             FROM jobs j
             WHERE j.id = ?1"
        )?;

        stmt.query_row(params![job_id], |row| {
            Ok(Job {
                id: row.get(0)?,
                target_id: row.get(1)?,
                destination_path: row.get(2)?,
                created_at: row.get(3)?,
                status: row.get(4)?,
            })
        })
    })
    .await
    .map_err(|e| anyhow!("Failed to get job: {}", e))
}

pub async fn update_status(
    conn: &Connection,
    job_id: String,
    status: String,
    description: Option<String>,
) -> Result<()> {
    conn.call(move |c| {
        let log_id = Uuid::now_v7().to_string();
        c.execute(
            "INSERT INTO job_status_log (id, job_id, status, description)
             VALUES (?1, ?2, ?3, ?4)",
            params![log_id, job_id, status, description],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await?;

    Ok(())
}
