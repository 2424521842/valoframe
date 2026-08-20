use std::fmt;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{
    de::{SeqAccess, Visitor},
    Deserialize, Deserializer,
};

use super::{
    truncate_utf8_bytes, ScanExecutionStatus, ScanSummary, MAX_SCAN_ERROR_MESSAGE_BYTES,
    MAX_SCAN_ERROR_SAMPLES,
};
use crate::db::DbResult;

pub fn latest_scan_summary(connection: &Connection) -> DbResult<Option<ScanSummary>> {
    connection
        .query_row(
            "
            SELECT root_path, source_dir_count, clip_group_count, new_clip_count,
                   updated_clip_count, missing_clip_count, pending_clip_count,
                   cover_missing_count,
                   metadata_match_count, metadata_enriched_clip_count,
                   metadata_event_count, metadata_warning_count,
                   diagnostic_omitted_count, errors_json, message
            FROM scan_runs
            WHERE summary_available = 1
              AND status IN ('completed', 'partial')
            ORDER BY id DESC
            LIMIT 1
            ",
            [],
            map_scan_summary,
        )
        .optional()
        .map_err(|error| format!("Database reading scan summary failed: {error}"))
}

/// Returns an exact persisted summary for one job. A terminal fallback row deliberately returns
/// `None`: its default zero counters are not evidence that the job added zero clips.
pub fn scan_summary_for_job(
    connection: &Connection,
    job_id: &str,
) -> DbResult<Option<ScanSummary>> {
    connection
        .query_row(
            "
            SELECT root_path, source_dir_count, clip_group_count, new_clip_count,
                   updated_clip_count, missing_clip_count, pending_clip_count,
                   cover_missing_count,
                   metadata_match_count, metadata_enriched_clip_count,
                   metadata_event_count, metadata_warning_count,
                   diagnostic_omitted_count, errors_json, message
            FROM scan_runs
            WHERE job_id = ?1
              AND summary_available = 1
            LIMIT 1
            ",
            params![job_id],
            map_scan_summary,
        )
        .optional()
        .map_err(|error| format!("Database reading scan summary for job failed: {error}"))
}

fn map_scan_summary(row: &Row<'_>) -> rusqlite::Result<ScanSummary> {
    let errors_json: String = row.get(13)?;
    let bounded = parse_bounded_scan_errors(&errors_json);
    Ok(ScanSummary {
        root_path: row.get(0)?,
        source_dir_count: row.get(1)?,
        clip_group_count: row.get(2)?,
        new_clip_count: row.get(3)?,
        updated_clip_count: row.get(4)?,
        missing_clip_count: row.get(5)?,
        pending_clip_count: row.get(6)?,
        cover_missing_count: row.get(7)?,
        metadata_match_count: row.get(8)?,
        metadata_enriched_clip_count: row.get(9)?,
        metadata_event_count: row.get(10)?,
        metadata_warning_count: row.get(11)?,
        omitted_error_count: row.get::<_, i64>(12)?.saturating_add(bounded.omitted_count),
        errors: bounded.errors,
        message: row.get(14)?,
    })
}

pub fn ensure_scan_run_started(
    connection: &Connection,
    job_id: &str,
    root_path: &str,
) -> DbResult<i64> {
    start_scan_run(connection, Some(job_id), root_path)
}

pub fn mark_scan_run_cancelling(connection: &Connection, job_id: &str) -> DbResult<bool> {
    connection
        .execute(
            "
            UPDATE scan_runs
            SET status = 'cancelling',
                message = '正在取消扫描'
            WHERE job_id = ?1
              AND status = 'running'
            ",
            params![job_id],
        )
        .map(|changed| changed > 0)
        .map_err(|error| format!("Database marking scan run cancelling failed: {error}"))
}

/// Finalizes scan rows left active when a previous application process exited unexpectedly.
pub fn recover_interrupted_scan_runs(connection: &Connection) -> DbResult<usize> {
    connection
        .execute(
            "
            UPDATE scan_runs
            SET status = 'failed',
                message = '上次应用异常退出，扫描已中断',
                summary_available = 0,
                finished_at = CURRENT_TIMESTAMP
            WHERE status IN ('running', 'cancelling')
            ",
            [],
        )
        .map_err(|error| format!("Database recovering interrupted scan runs failed: {error}"))
}

pub fn ensure_scan_run_terminal(
    connection: &Connection,
    job_id: &str,
    root_path: &str,
    status: &str,
    message: &str,
) -> DbResult<()> {
    if !matches!(status, "completed" | "partial" | "failed" | "cancelled") {
        return Err(format!("Unsupported terminal scan status: {status}"));
    }

    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Database starting terminal scan update failed: {error}"))?;
    let changed = transaction
        .execute(
            "
            UPDATE scan_runs
            SET root_path = CASE
                    WHEN NULLIF(TRIM(root_path), '') IS NULL THEN ?2
                    ELSE root_path
                END,
                status = ?3,
                message = ?4,
                summary_available = 0,
                finished_at = CURRENT_TIMESTAMP
            WHERE job_id = ?1
              AND status IN ('running', 'cancelling')
            ",
            params![job_id, root_path, status, message],
        )
        .map_err(|error| format!("Database finalizing scan run failed: {error}"))?;
    if changed == 0 {
        let existing_status = transaction
            .query_row(
                "SELECT status FROM scan_runs WHERE job_id = ?1",
                params![job_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Database checking terminal scan run failed: {error}"))?;
        if existing_status.is_none() {
            transaction
                .execute(
                    "
            INSERT INTO scan_runs (job_id, root_path, status, message)
            VALUES (?1, ?2, ?3, ?4)
            ",
                    params![job_id, root_path, status, message],
                )
                .map_err(|error| format!("Database recording terminal scan run failed: {error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("Database committing terminal scan update failed: {error}"))
}

pub fn finalize_scan_run_for_job(
    connection: &Connection,
    job_id: &str,
    status: ScanExecutionStatus,
    summary: &ScanSummary,
) -> DbResult<()> {
    let scan_run_id = connection
        .query_row(
            "SELECT id FROM scan_runs WHERE job_id = ?1",
            params![job_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Database finding scan job failed: {error}"))?;
    finish_scan_run(connection, scan_run_id, status.as_str(), summary)
}

fn start_scan_run(connection: &Connection, job_id: Option<&str>, root_path: &str) -> DbResult<i64> {
    if let Some(job_id) = job_id {
        let existing = connection
            .query_row(
                "SELECT id, status FROM scan_runs WHERE job_id = ?1",
                params![job_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| format!("Database reading scan run failed: {error}"))?;
        if let Some((scan_run_id, status)) = existing {
            if !matches!(status.as_str(), "running" | "cancelling") {
                return Err(format!(
                    "Scan job {job_id} is already terminal with status {status}"
                ));
            }
            connection
                .execute(
                    "UPDATE scan_runs SET root_path = ?2 WHERE id = ?1",
                    params![scan_run_id, root_path],
                )
                .map_err(|error| format!("Database updating scan root failed: {error}"))?;
            return Ok(scan_run_id);
        }
    }

    connection
        .execute(
            "
            INSERT INTO scan_runs (job_id, root_path, status, message)
            VALUES (?1, ?2, 'running', '正在扫描')
            ",
            params![job_id, root_path],
        )
        .map_err(|error| format!("Database starting scan run failed: {error}"))?;
    Ok(connection.last_insert_rowid())
}

fn finish_scan_run(
    connection: &Connection,
    scan_run_id: i64,
    status: &str,
    summary: &ScanSummary,
) -> DbResult<()> {
    let errors_json = serde_json::to_string(&summary.errors)
        .map_err(|error| format!("Serializing scan errors failed: {error}"))?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Database starting scan finalization failed: {error}"))?;
    let changed = transaction
        .execute(
            "
            UPDATE scan_runs
            SET root_path = ?2,
                status = ?3,
                source_dir_count = ?4,
                clip_group_count = ?5,
                new_clip_count = ?6,
                updated_clip_count = ?7,
                missing_clip_count = ?8,
                pending_clip_count = ?9,
                cover_missing_count = ?10,
                metadata_match_count = ?11,
                metadata_enriched_clip_count = ?12,
                metadata_event_count = ?13,
                metadata_warning_count = ?14,
                diagnostic_omitted_count = ?15,
                errors_json = ?16,
                message = ?17,
                summary_available = 1,
                finished_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            ",
            params![
                scan_run_id,
                summary.root_path,
                status,
                summary.source_dir_count,
                summary.clip_group_count,
                summary.new_clip_count,
                summary.updated_clip_count,
                summary.missing_clip_count,
                summary.pending_clip_count,
                summary.cover_missing_count,
                summary.metadata_match_count,
                summary.metadata_enriched_clip_count,
                summary.metadata_event_count,
                summary.metadata_warning_count,
                summary.omitted_error_count,
                errors_json,
                summary.message,
            ],
        )
        .map_err(|error| format!("Database saving scan summary failed: {error}"))?;
    if changed == 0 {
        return Err(format!(
            "Scan run {scan_run_id} disappeared before finalization"
        ));
    }
    transaction
        .commit()
        .map_err(|error| format!("Database committing scan finalization failed: {error}"))
}

pub(super) struct ScanRunGuard<'a> {
    connection: &'a Connection,
    scan_run_id: i64,
    finished: bool,
}

impl<'a> ScanRunGuard<'a> {
    pub(super) fn start(
        connection: &'a Connection,
        job_id: Option<&str>,
        root_path: &str,
    ) -> DbResult<Self> {
        Ok(Self {
            connection,
            scan_run_id: start_scan_run(connection, job_id, root_path)?,
            finished: false,
        })
    }

    pub(super) fn finish(mut self, status: &str, summary: &ScanSummary) -> DbResult<()> {
        finish_scan_run(self.connection, self.scan_run_id, status, summary)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for ScanRunGuard<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = self.connection.execute(
            "
            UPDATE scan_runs
            SET status = 'failed',
                message = '扫描任务意外终止',
                summary_available = 0,
                finished_at = CURRENT_TIMESTAMP
            WHERE id = ?1
              AND status IN ('running', 'cancelling')
            ",
            params![self.scan_run_id],
        );
    }
}

struct BoundedScanErrors {
    errors: Vec<String>,
    omitted_count: i64,
}

impl<'de> Deserialize<'de> for BoundedScanErrors {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(BoundedScanErrorsVisitor)
    }
}

struct BoundedScanErrorsVisitor;

impl<'de> Visitor<'de> for BoundedScanErrorsVisitor {
    type Value = BoundedScanErrors;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an array of scan error strings")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut errors = Vec::new();
        let mut omitted_count = 0_i64;
        while let Some(value) = sequence.next_element::<serde_json::Value>()? {
            let Some(error) = value.as_str() else {
                omitted_count = omitted_count.saturating_add(1);
                continue;
            };
            let error = truncate_utf8_bytes(error.to_string(), MAX_SCAN_ERROR_MESSAGE_BYTES);
            if errors.contains(&error) {
                continue;
            }
            if errors.len() < MAX_SCAN_ERROR_SAMPLES {
                errors.push(error);
            } else {
                omitted_count = omitted_count.saturating_add(1);
            }
        }
        Ok(BoundedScanErrors {
            errors,
            omitted_count,
        })
    }
}

fn parse_bounded_scan_errors(value: &str) -> BoundedScanErrors {
    serde_json::from_str(value).unwrap_or_else(|_| BoundedScanErrors {
        errors: Vec::new(),
        omitted_count: i64::from(!value.trim().is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn recover_interrupted_scan_runs_fails_only_active_rows_and_is_idempotent() {
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        connection
            .execute_batch(
                "
                INSERT INTO scan_runs (job_id, root_path, status, message, finished_at) VALUES
                    ('active-running', 'D:/running', 'running', '正在扫描', '2000-01-01 00:00:00'),
                    ('active-cancelling', 'D:/cancelling', 'cancelling', '正在取消扫描', '2000-01-01 00:00:00'),
                    ('terminal-completed', 'D:/completed', 'completed', '完成', '2000-01-01 00:00:00'),
                    ('terminal-partial', 'D:/partial', 'partial', '部分完成', '2000-01-01 00:00:00'),
                    ('terminal-failed', 'D:/failed', 'failed', '已失败', '2000-01-01 00:00:00'),
                    ('terminal-cancelled', 'D:/cancelled', 'cancelled', '已取消', '2000-01-01 00:00:00');
                ",
            )
            .expect("scan run fixtures should insert");

        assert_eq!(recover_interrupted_scan_runs(&connection).unwrap(), 2);

        for job_id in ["active-running", "active-cancelling"] {
            let (status, message, finished_at): (String, String, String) = connection
                .query_row(
                    "SELECT status, message, finished_at FROM scan_runs WHERE job_id = ?1",
                    [job_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("recovered scan run should load");
            assert_eq!(status, "failed");
            assert_eq!(message, "上次应用异常退出，扫描已中断");
            assert_ne!(finished_at, "2000-01-01 00:00:00");
        }

        for (job_id, expected_status, expected_message) in [
            ("terminal-completed", "completed", "完成"),
            ("terminal-partial", "partial", "部分完成"),
            ("terminal-failed", "failed", "已失败"),
            ("terminal-cancelled", "cancelled", "已取消"),
        ] {
            let (status, message, finished_at): (String, String, String) = connection
                .query_row(
                    "SELECT status, message, finished_at FROM scan_runs WHERE job_id = ?1",
                    [job_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("terminal scan run should load");
            assert_eq!(status, expected_status);
            assert_eq!(message, expected_message);
            assert_eq!(finished_at, "2000-01-01 00:00:00");
        }

        assert_eq!(recover_interrupted_scan_runs(&connection).unwrap(), 0);
    }

    #[test]
    fn exact_job_summary_distinguishes_a_real_zero_from_a_fallback_row() {
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        ensure_scan_run_started(&connection, "exact-zero", "D:/clips")
            .expect("scan run should start");
        let mut zero_summary = ScanSummary::empty("D:/clips".to_string());
        zero_summary.pending_clip_count = 2;
        zero_summary.message = Some("completed".to_string());
        finalize_scan_run_for_job(
            &connection,
            "exact-zero",
            ScanExecutionStatus::Completed,
            &zero_summary,
        )
        .expect("full summary should finalize");
        let restored = scan_summary_for_job(&connection, "exact-zero")
            .unwrap()
            .expect("real zero summary should load");
        assert_eq!(restored.new_clip_count, 0);
        assert_eq!(restored.pending_clip_count, 2);

        ensure_scan_run_terminal(
            &connection,
            "fallback-only",
            "D:/fallback",
            "cancelled",
            "cancelled before a summary was available",
        )
        .expect("fallback terminal row should persist");
        assert_eq!(
            scan_summary_for_job(&connection, "fallback-only").unwrap(),
            None
        );
    }

    #[test]
    fn late_terminal_fallback_cannot_erase_a_completed_summary() {
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        ensure_scan_run_started(&connection, "late-fallback", "D:/clips")
            .expect("scan run should start");
        let mut summary = ScanSummary::empty("D:/clips".to_string());
        summary.new_clip_count = 5;
        finalize_scan_run_for_job(
            &connection,
            "late-fallback",
            ScanExecutionStatus::Completed,
            &summary,
        )
        .expect("summary should finalize");

        ensure_scan_run_terminal(
            &connection,
            "late-fallback",
            "D:/wrong-root",
            "failed",
            "late fallback",
        )
        .expect("late fallback should be an idempotent no-op");

        let (status, available): (String, i64) = connection
            .query_row(
                "SELECT status, summary_available FROM scan_runs WHERE job_id = 'late-fallback'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("scan row should load");
        assert_eq!(status, "completed");
        assert_eq!(available, 1);
        assert_eq!(
            scan_summary_for_job(&connection, "late-fallback")
                .unwrap()
                .expect("summary should remain")
                .new_clip_count,
            5,
        );
    }

    #[test]
    fn latest_compatibility_query_ignores_newer_cancelled_summaries() {
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        for (job_id, status, new_clip_count) in [
            ("completed-job", ScanExecutionStatus::Completed, 3),
            ("cancelled-job", ScanExecutionStatus::Cancelled, 9),
        ] {
            ensure_scan_run_started(&connection, job_id, "D:/clips")
                .expect("scan run should start");
            let mut summary = ScanSummary::empty("D:/clips".to_string());
            summary.new_clip_count = new_clip_count;
            finalize_scan_run_for_job(&connection, job_id, status, &summary)
                .expect("scan run should finalize");
        }

        assert_eq!(
            latest_scan_summary(&connection)
                .unwrap()
                .expect("latest successful summary should load")
                .new_clip_count,
            3,
        );
        assert_eq!(
            scan_summary_for_job(&connection, "cancelled-job")
                .unwrap()
                .expect("exact cancelled summary should load")
                .new_clip_count,
            9,
        );
    }
}
