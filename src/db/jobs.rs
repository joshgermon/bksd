use anyhow::{Result, anyhow};
use tokio_rusqlite::{Connection, params, rusqlite};
use uuid::Uuid;

use crate::core::{Job, JobStatusEntry, JobWithHistory, TargetDrive};

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
    total_bytes: Option<u64>,
    duration_secs: Option<u64>,
) -> Result<()> {
    conn.call(move |c| {
        let log_id = Uuid::now_v7().to_string();
        c.execute(
            "INSERT INTO job_status_log (id, job_id, status, description, total_bytes, duration_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![log_id, job_id, status, description, total_bytes, duration_secs],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await?;

    Ok(())
}

/// List jobs with optional filtering and pagination.
/// Returns jobs ordered by creation date (newest first).
pub async fn list(
    conn: &Connection,
    limit: u32,
    offset: u32,
    status_filter: Option<String>,
) -> Result<Vec<Job>> {
    conn.call(move |c| {
        let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(ref status) = status_filter {
            (
                "SELECT j.id, j.target_id, j.destination_path, j.created_at,
                 COALESCE((SELECT status FROM job_status_log WHERE job_id = j.id ORDER BY created_at DESC LIMIT 1), 'Unknown') as status
                 FROM jobs j
                 WHERE (SELECT status FROM job_status_log WHERE job_id = j.id ORDER BY created_at DESC LIMIT 1) = ?1
                 ORDER BY j.created_at DESC
                 LIMIT ?2 OFFSET ?3",
                vec![
                    Box::new(status.clone()) as Box<dyn rusqlite::ToSql>,
                    Box::new(limit),
                    Box::new(offset),
                ],
            )
        } else {
            (
                "SELECT j.id, j.target_id, j.destination_path, j.created_at,
                 COALESCE((SELECT status FROM job_status_log WHERE job_id = j.id ORDER BY created_at DESC LIMIT 1), 'Unknown') as status
                 FROM jobs j
                 ORDER BY j.created_at DESC
                 LIMIT ?1 OFFSET ?2",
                vec![
                    Box::new(limit) as Box<dyn rusqlite::ToSql>,
                    Box::new(offset),
                ],
            )
        };

        let mut stmt = c.prepare(sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let jobs = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(Job {
                    id: row.get(0)?,
                    target_id: row.get(1)?,
                    destination_path: row.get(2)?,
                    created_at: row.get(3)?,
                    status: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok::<_, rusqlite::Error>(jobs)
    })
    .await
    .map_err(|e| anyhow!("Failed to list jobs: {}", e))
}

/// Get a job with its full status history.
pub async fn get_with_history(conn: &Connection, job_id: String) -> Result<JobWithHistory> {
    conn.call(move |c| {
        // First get the job
        let job = {
            let mut stmt = c.prepare(
                "SELECT j.id, j.target_id, j.destination_path, j.created_at,
                 COALESCE((SELECT status FROM job_status_log WHERE job_id = j.id ORDER BY created_at DESC LIMIT 1), 'Unknown') as status
                 FROM jobs j
                 WHERE j.id = ?1",
            )?;

            stmt.query_row(params![&job_id], |row| {
                Ok(Job {
                    id: row.get(0)?,
                    target_id: row.get(1)?,
                    destination_path: row.get(2)?,
                    created_at: row.get(3)?,
                    status: row.get(4)?,
                })
            })?
        };

        // Then get the status history
        let history = {
            let mut stmt = c.prepare(
                "SELECT id, status, description, total_bytes, duration_secs, created_at
                 FROM job_status_log
                 WHERE job_id = ?1
                 ORDER BY created_at ASC",
            )?;

            stmt.query_map(params![&job_id], |row| {
                Ok(JobStatusEntry {
                    id: row.get(0)?,
                    status: row.get(1)?,
                    description: row.get(2)?,
                    total_bytes: row.get(3)?,
                    duration_secs: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        };

        Ok::<_, rusqlite::Error>(JobWithHistory { job, history })
    })
    .await
    .map_err(|e| anyhow!("Failed to get job with history: {}", e))
}
