//! Persistent thumbnail queue state and atomic job transitions.
//!
//! This repository deliberately knows nothing about the cache filesystem or FFmpeg. It keeps
//! generated artifacts separate from `clips.cover_path`, which remains owned by the scanner and
//! therefore safe to refresh from read-only source directories.

use std::collections::BTreeSet;

use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use super::super::{
    readable_error, DbResult, ThumbnailCacheRef, ThumbnailEnsureResult, ThumbnailJob,
    ThumbnailQueueStatus, ThumbnailReconcileResult, ThumbnailStatus,
};

const MAX_COMMAND_CLIP_IDS: usize = 200;
const THUMBNAIL_FINGERPRINT_VERSION: &str = "thumb-v1-jpeg-w480-seek500ms";

#[derive(Debug)]
struct ReconcileCandidate {
    clip_id: i64,
    normalized_path: String,
    size_bytes: i64,
    modified_at: Option<String>,
    file_status: String,
    cover_path: Option<String>,
    cover_source: String,
    thumbnail_fingerprint: Option<String>,
    thumbnail_status: Option<String>,
    thumbnail_revision: Option<String>,
    thumbnail_error_code: Option<String>,
}

/// Computes the stable source/output fingerprint used for invalidation and stale-result guards.
pub fn thumbnail_fingerprint(
    normalized_path: &str,
    size_bytes: i64,
    modified_at: Option<&str>,
) -> String {
    let mut digest = Sha256::new();
    for part in [
        THUMBNAIL_FINGERPRINT_VERSION,
        normalized_path,
        &size_bytes.to_string(),
        modified_at.unwrap_or_default(),
    ] {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

/// Reconciles every clip, or a bounded selected set, against its persisted thumbnail state.
///
/// A source-owned cover always suppresses generation. Available coverless clips are queued when
/// first seen, when their fingerprint changes, when they were evicted, or when `force_retry`
/// explicitly resets a terminal failure. Ready artifacts with an unchanged fingerprint are left
/// untouched.
pub fn reconcile_clip_thumbnails(
    connection: &Connection,
    clip_ids: Option<&[i64]>,
    force_retry: bool,
) -> DbResult<ThumbnailReconcileResult> {
    let selected_ids = clip_ids.map(normalized_clip_ids).transpose()?;
    if selected_ids.as_ref().is_some_and(Vec::is_empty) {
        return Ok(ThumbnailReconcileResult::default());
    }

    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting thumbnail reconciliation", error))?;
    let candidates = load_reconcile_candidates(&transaction, selected_ids.as_deref())?;
    let requested = selected_ids
        .as_ref()
        .map_or(candidates.len(), |ids| ids.len());
    let is_selected_reconcile = selected_ids.is_some();
    let mut result = ThumbnailReconcileResult {
        counts: ThumbnailEnsureResult {
            requested,
            skipped: requested.saturating_sub(candidates.len()),
            ..ThumbnailEnsureResult::default()
        },
        changed: Vec::new(),
    };

    for candidate in candidates {
        reconcile_candidate(
            &transaction,
            candidate,
            is_selected_reconcile,
            force_retry,
            &mut result,
        )?;
    }

    transaction
        .commit()
        .map_err(|error| readable_error("finishing thumbnail reconciliation", error))?;
    Ok(result)
}

pub fn ensure_clip_thumbnails(
    connection: &Connection,
    clip_ids: &[i64],
) -> DbResult<ThumbnailReconcileResult> {
    reconcile_clip_thumbnails(connection, Some(clip_ids), false)
}

pub fn retry_clip_thumbnails(
    connection: &Connection,
    clip_ids: &[i64],
) -> DbResult<ThumbnailReconcileResult> {
    reconcile_clip_thumbnails(connection, Some(clip_ids), true)
}

/// Atomically claims the next due job. The single UPDATE chooses and transitions the row while
/// SQLite holds its write lock, so concurrent workers cannot receive the same clip.
pub fn claim_next_thumbnail_job(
    connection: &Connection,
    now: &str,
) -> DbResult<Option<ThumbnailJob>> {
    if now.trim().is_empty() {
        return Err("thumbnail claim timestamp must not be empty".to_string());
    }

    loop {
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| readable_error("starting thumbnail claim", error))?;
        let claimed_clip_id = transaction
            .query_row(
                "
            UPDATE clip_thumbnails
            SET status = 'running',
                attempt_count = attempt_count + 1,
                next_attempt_at = NULL,
                error_code = NULL,
                last_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE clip_id = (
                SELECT thumbnail.clip_id
                FROM clip_thumbnails thumbnail
                JOIN clips ON clips.id = thumbnail.clip_id
                WHERE thumbnail.status = 'pending'
                  AND (
                      thumbnail.next_attempt_at IS NULL
                      OR julianday(thumbnail.next_attempt_at) <= julianday(?1)
                  )
                  AND clips.file_status = 'available'
                  AND NOT (
                      clips.cover_source = 'file'
                      AND NULLIF(TRIM(clips.cover_path), '') IS NOT NULL
                  )
                ORDER BY
                    CASE WHEN thumbnail.next_attempt_at IS NULL THEN 0 ELSE 1 END,
                    thumbnail.next_attempt_at,
                    thumbnail.clip_id
                LIMIT 1
            )
              AND status = 'pending'
            RETURNING clip_id
            ",
                params![now],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| readable_error("claiming thumbnail job", error))?;

        let Some(clip_id) = claimed_clip_id else {
            transaction
                .commit()
                .map_err(|error| readable_error("finishing empty thumbnail claim", error))?;
            return Ok(None);
        };
        let job = transaction
            .query_row(
                "
                    SELECT
                        thumbnail.clip_id,
                        clips.file_path,
                        clips.normalized_path,
                        clips.size_bytes,
                        clips.modified_at,
                        thumbnail.fingerprint,
                        thumbnail.attempt_count,
                        thumbnail.revision,
                        thumbnail.cache_file
                    FROM clip_thumbnails thumbnail
                    JOIN clips ON clips.id = thumbnail.clip_id
                    WHERE thumbnail.clip_id = ?1
                    ",
                params![clip_id],
                |row| {
                    let clip_id = row.get(0)?;
                    Ok(ThumbnailJob {
                        id: clip_id,
                        clip_id,
                        video_path: row.get(1)?,
                        normalized_path: row.get(2)?,
                        size_bytes: row.get(3)?,
                        modified_at: row.get(4)?,
                        fingerprint: row.get(5)?,
                        attempt_count: row.get(6)?,
                        revision: row.get(7)?,
                        cache_file: row.get(8)?,
                    })
                },
            )
            .map_err(|error| readable_error("reading claimed thumbnail job", error))?;

        let current_fingerprint = thumbnail_fingerprint(
            &job.normalized_path,
            job.size_bytes,
            job.modified_at.as_deref(),
        );
        if current_fingerprint != job.fingerprint {
            transaction
                .execute(
                    "
                    UPDATE clip_thumbnails
                    SET fingerprint = ?3,
                        cache_file = NULL,
                        status = 'pending',
                        attempt_count = 0,
                        next_attempt_at = NULL,
                        error_code = NULL,
                        last_error = NULL,
                        byte_size = NULL,
                        revision = NULL,
                        generated_at = NULL,
                        updated_at = CURRENT_TIMESTAMP
                    WHERE clip_id = ?1
                      AND fingerprint = ?2
                      AND status = 'running'
                    ",
                    params![clip_id, job.fingerprint, current_fingerprint],
                )
                .map_err(|error| readable_error("refreshing stale thumbnail claim", error))?;
            transaction
                .commit()
                .map_err(|error| readable_error("finishing stale thumbnail claim", error))?;
            continue;
        }

        transaction
            .commit()
            .map_err(|error| readable_error("finishing thumbnail claim", error))?;
        return Ok(Some(job));
    }
}

/// Marks a generated artifact ready only if the claimed source fingerprint is still current.
pub fn complete_thumbnail_job_if_current(
    connection: &Connection,
    job: &ThumbnailJob,
    cache_file: &str,
    byte_size: i64,
    revision: &str,
) -> DbResult<bool> {
    validate_cache_basename(cache_file)?;
    if job.fingerprint.trim().is_empty() {
        return Err("thumbnail fingerprint must not be empty".to_string());
    }
    if revision.trim().is_empty() {
        return Err("thumbnail revision must not be empty".to_string());
    }
    if revision != job.fingerprint {
        return Err("thumbnail revision must match the current fingerprint".to_string());
    }
    let expected_cache_file = format!("{}-{}.jpg", job.clip_id, job.fingerprint);
    if cache_file != expected_cache_file {
        return Err("thumbnail cache file must match the clip fingerprint".to_string());
    }
    if byte_size <= 0 {
        return Err("thumbnail byte size must be positive".to_string());
    }

    let changed = connection
        .execute(
            "
            UPDATE clip_thumbnails
            SET cache_file = ?7,
                status = 'ready',
                next_attempt_at = NULL,
                error_code = NULL,
                last_error = NULL,
                byte_size = ?8,
                revision = ?9,
                generated_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            WHERE clip_id = ?1
              AND fingerprint = ?2
              AND status = 'running'
              AND EXISTS (
                  SELECT 1
                  FROM clips
                  WHERE clips.id = clip_thumbnails.clip_id
                    AND clips.file_path = ?3
                    AND clips.normalized_path = ?4
                    AND clips.size_bytes = ?5
                    AND clips.modified_at IS ?6
                    AND clips.file_status = 'available'
                    AND NOT (
                        clips.cover_source = 'file'
                        AND NULLIF(TRIM(clips.cover_path), '') IS NOT NULL
                    )
              )
            ",
            params![
                job.clip_id,
                job.fingerprint,
                job.video_path,
                job.normalized_path,
                job.size_bytes,
                job.modified_at,
                cache_file,
                byte_size,
                revision
            ],
        )
        .map_err(|error| readable_error("completing thumbnail job", error))?;
    Ok(changed == 1)
}

/// Records a retryable or terminal generator failure only for the still-current claimed job.
pub fn fail_thumbnail_job_if_current(
    connection: &Connection,
    clip_id: i64,
    fingerprint: &str,
    next_status: &str,
    error_code: &str,
    last_error: Option<&str>,
    next_attempt_at: Option<&str>,
) -> DbResult<bool> {
    if !matches!(next_status, "pending" | "failed" | "unavailable") {
        return Err(format!(
            "thumbnail failure status must be pending, failed, or unavailable; got {next_status:?}"
        ));
    }
    if next_status == "pending" && next_attempt_at.is_none() {
        return Err("retryable thumbnail failures require next_attempt_at".to_string());
    }
    if fingerprint.trim().is_empty() || error_code.trim().is_empty() {
        return Err("thumbnail fingerprint and error code must not be empty".to_string());
    }

    let changed = connection
        .execute(
            "
            UPDATE clip_thumbnails
            SET cache_file = NULL,
                status = ?3,
                next_attempt_at = ?4,
                error_code = ?5,
                last_error = ?6,
                byte_size = NULL,
                revision = NULL,
                generated_at = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE clip_id = ?1
              AND fingerprint = ?2
              AND status = 'running'
            ",
            params![
                clip_id,
                fingerprint,
                next_status,
                next_attempt_at,
                error_code,
                last_error
            ],
        )
        .map_err(|error| readable_error("failing thumbnail job", error))?;
    Ok(changed == 1)
}

/// Returns interrupted work to the pending state during startup recovery.
pub fn recover_running_thumbnail_jobs(connection: &Connection) -> DbResult<usize> {
    connection
        .execute(
            "
            UPDATE clip_thumbnails
            SET status = 'pending',
                next_attempt_at = NULL,
                error_code = NULL,
                last_error = NULL,
                revision = NULL,
                cache_file = NULL,
                byte_size = NULL,
                generated_at = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE status = 'running'
            ",
            [],
        )
        .map_err(|error| readable_error("recovering running thumbnail jobs", error))
}

/// Applies a single global generator-unavailable diagnosis without claiming or emitting one
/// failure per clip.
pub fn mark_pending_thumbnails_unavailable(
    connection: &Connection,
    error_code: &str,
) -> DbResult<usize> {
    if error_code.trim().is_empty() {
        return Err("thumbnail unavailable error code must not be empty".to_string());
    }
    connection
        .execute(
            "
            UPDATE clip_thumbnails
            SET status = 'unavailable',
                next_attempt_at = NULL,
                error_code = ?1,
                last_error = NULL,
                cache_file = NULL,
                byte_size = NULL,
                revision = NULL,
                generated_at = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE status = 'pending'
            ",
            params![error_code],
        )
        .map_err(|error| readable_error("marking pending thumbnails unavailable", error))
}

/// Requeues the global unavailable state after a later startup successfully detects a generator.
pub fn recover_unavailable_thumbnail_jobs(connection: &Connection) -> DbResult<usize> {
    connection
        .execute(
            "
            UPDATE clip_thumbnails
            SET status = 'pending',
                attempt_count = 0,
                next_attempt_at = NULL,
                error_code = NULL,
                last_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE status = 'unavailable'
            ",
            [],
        )
        .map_err(|error| readable_error("recovering unavailable thumbnail jobs", error))
}

pub fn get_thumbnail_status(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<Option<ThumbnailStatus>> {
    connection
        .query_row(
            "
            SELECT clip_id, status, revision, error_code
            FROM clip_thumbnails
            WHERE clip_id = ?1
            ",
            params![clip_id],
            map_thumbnail_status,
        )
        .optional()
        .map_err(|error| readable_error("reading thumbnail status", error))
}

pub fn list_thumbnail_statuses(
    connection: &Connection,
    clip_ids: &[i64],
) -> DbResult<Vec<ThumbnailStatus>> {
    let clip_ids = normalized_clip_ids(clip_ids)?;
    if clip_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", clip_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        SELECT clip_id, status, revision, error_code
        FROM clip_thumbnails
        WHERE clip_id IN ({placeholders})
        ORDER BY clip_id
        "
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| readable_error("preparing thumbnail status list", error))?;
    let statuses = statement
        .query_map(params_from_iter(clip_ids), map_thumbnail_status)
        .map_err(|error| readable_error("querying thumbnail status list", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading thumbnail status list", error))?;
    Ok(statuses)
}

pub fn get_thumbnail_queue_status(connection: &Connection) -> DbResult<ThumbnailQueueStatus> {
    connection
        .query_row(
            "
            SELECT
                COALESCE(SUM(status = 'pending'), 0),
                COALESCE(SUM(status = 'running'), 0),
                COALESCE(SUM(status = 'ready'), 0),
                COALESCE(SUM(status = 'failed'), 0),
                COALESCE(SUM(status = 'unavailable'), 0),
                COALESCE(SUM(status = 'evicted'), 0),
                COALESCE(SUM(CASE WHEN status = 'ready' THEN byte_size ELSE 0 END), 0)
            FROM clip_thumbnails
            ",
            [],
            |row| {
                Ok(ThumbnailQueueStatus {
                    pending: row.get(0)?,
                    running: row.get(1)?,
                    ready: row.get(2)?,
                    failed: row.get(3)?,
                    unavailable: row.get(4)?,
                    evicted: row.get(5)?,
                    cache_bytes: row.get(6)?,
                })
            },
        )
        .map_err(|error| readable_error("reading thumbnail queue status", error))
}

pub fn list_ready_thumbnail_cache_refs(
    connection: &Connection,
) -> DbResult<Vec<ThumbnailCacheRef>> {
    let mut statement = connection
        .prepare(
            "
            SELECT clip_id, cache_file, revision, byte_size, generated_at
            FROM clip_thumbnails
            WHERE status = 'ready'
            ORDER BY generated_at, clip_id
            ",
        )
        .map_err(|error| readable_error("preparing ready thumbnail cache list", error))?;
    let cache_refs = statement
        .query_map([], |row| {
            Ok(ThumbnailCacheRef {
                clip_id: row.get(0)?,
                cache_file: row.get(1)?,
                revision: row.get(2)?,
                byte_size: row.get(3)?,
                generated_at: row.get(4)?,
            })
        })
        .map_err(|error| readable_error("querying ready thumbnail cache list", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading ready thumbnail cache list", error))?;
    Ok(cache_refs)
}

/// Marks a ready artifact evicted only if cleanup observed the same revision.
pub fn mark_thumbnail_evicted_if_current(
    connection: &Connection,
    clip_id: i64,
    revision: &str,
) -> DbResult<bool> {
    let changed = connection
        .execute(
            "
            UPDATE clip_thumbnails
            SET status = 'evicted',
                cache_file = NULL,
                byte_size = NULL,
                revision = NULL,
                generated_at = NULL,
                error_code = NULL,
                last_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE clip_id = ?1
              AND status = 'ready'
              AND revision = ?2
            ",
            params![clip_id, revision],
        )
        .map_err(|error| readable_error("marking thumbnail evicted", error))?;
    Ok(changed == 1)
}

/// Requeues a ready DB row when its referenced cache file is absent on disk.
pub fn mark_thumbnail_cache_missing_if_current(
    connection: &Connection,
    clip_id: i64,
    revision: &str,
) -> DbResult<bool> {
    let changed = connection
        .execute(
            "
            UPDATE clip_thumbnails
            SET status = 'pending',
                cache_file = NULL,
                byte_size = NULL,
                revision = NULL,
                generated_at = NULL,
                attempt_count = 0,
                next_attempt_at = NULL,
                error_code = NULL,
                last_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE clip_id = ?1
              AND status = 'ready'
              AND revision = ?2
            ",
            params![clip_id, revision],
        )
        .map_err(|error| readable_error("requeueing missing thumbnail cache file", error))?;
    Ok(changed == 1)
}

/// Defensive repair for databases previously opened without foreign-key enforcement. Normal
/// configured connections already remove these rows through `ON DELETE CASCADE`.
pub fn delete_orphan_thumbnail_rows(connection: &Connection) -> DbResult<usize> {
    connection
        .execute(
            "
            DELETE FROM clip_thumbnails
            WHERE NOT EXISTS (
                SELECT 1 FROM clips WHERE clips.id = clip_thumbnails.clip_id
            )
            ",
            [],
        )
        .map_err(|error| readable_error("deleting orphan thumbnail rows", error))
}

fn load_reconcile_candidates(
    connection: &Connection,
    clip_ids: Option<&[i64]>,
) -> DbResult<Vec<ReconcileCandidate>> {
    let mut sql = String::from(
        "
        SELECT
            clips.id,
            clips.normalized_path,
            clips.size_bytes,
            clips.modified_at,
            clips.file_status,
            clips.cover_path,
            clips.cover_source,
            thumbnail.fingerprint,
            thumbnail.status,
            thumbnail.revision,
            thumbnail.error_code
        FROM clips
        LEFT JOIN clip_thumbnails thumbnail ON thumbnail.clip_id = clips.id
        ",
    );
    let params = if let Some(clip_ids) = clip_ids {
        let placeholders = std::iter::repeat_n("?", clip_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" WHERE clips.id IN ({placeholders})"));
        clip_ids.iter().copied().map(Value::Integer).collect()
    } else {
        Vec::new()
    };
    sql.push_str(" ORDER BY clips.id");

    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| readable_error("preparing thumbnail reconciliation", error))?;
    let candidates = statement
        .query_map(params_from_iter(params), |row| {
            Ok(ReconcileCandidate {
                clip_id: row.get(0)?,
                normalized_path: row.get(1)?,
                size_bytes: row.get(2)?,
                modified_at: row.get(3)?,
                file_status: row.get(4)?,
                cover_path: row.get(5)?,
                cover_source: row.get(6)?,
                thumbnail_fingerprint: row.get(7)?,
                thumbnail_status: row.get(8)?,
                thumbnail_revision: row.get(9)?,
                thumbnail_error_code: row.get(10)?,
            })
        })
        .map_err(|error| readable_error("querying thumbnail reconciliation", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading thumbnail reconciliation", error))?;
    Ok(candidates)
}

fn reconcile_candidate(
    connection: &Connection,
    candidate: ReconcileCandidate,
    is_selected_reconcile: bool,
    force_retry: bool,
    result: &mut ThumbnailReconcileResult,
) -> DbResult<()> {
    let fingerprint = thumbnail_fingerprint(
        &candidate.normalized_path,
        candidate.size_bytes,
        candidate.modified_at.as_deref(),
    );
    let has_source_cover = candidate.cover_source == "file"
        && candidate
            .cover_path
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty());
    let is_eligible = candidate.file_status == "available" && !has_source_cover;

    if !is_eligible {
        result.counts.skipped += 1;
        let already_suppressed = candidate.thumbnail_status.as_deref() == Some("suppressed")
            && candidate.thumbnail_fingerprint.as_deref() == Some(fingerprint.as_str())
            && candidate.thumbnail_revision.is_none()
            && candidate.thumbnail_error_code.is_none();
        if !already_suppressed {
            reset_thumbnail_state(connection, candidate.clip_id, &fingerprint, "suppressed")?;
            result.changed.push(ThumbnailStatus {
                clip_id: candidate.clip_id,
                status: "suppressed".to_string(),
                revision: None,
                error_code: None,
            });
        }
        return Ok(());
    }

    let fingerprint_changed = candidate.thumbnail_fingerprint.as_deref() != Some(&fingerprint);
    let should_queue = fingerprint_changed
        || candidate.thumbnail_status.is_none()
        || candidate.thumbnail_status.as_deref() == Some("suppressed")
        || (is_selected_reconcile && candidate.thumbnail_status.as_deref() == Some("evicted"))
        || (force_retry
            && matches!(
                candidate.thumbnail_status.as_deref(),
                Some("failed" | "unavailable")
            ));

    if should_queue {
        reset_thumbnail_state(connection, candidate.clip_id, &fingerprint, "pending")?;
        result.counts.queued += 1;
        result.changed.push(ThumbnailStatus {
            clip_id: candidate.clip_id,
            status: "pending".to_string(),
            revision: None,
            error_code: None,
        });
    } else if matches!(
        candidate.thumbnail_status.as_deref(),
        Some("pending" | "running")
    ) {
        result.counts.already_queued += 1;
    } else {
        result.counts.skipped += 1;
    }
    Ok(())
}

fn reset_thumbnail_state(
    connection: &Connection,
    clip_id: i64,
    fingerprint: &str,
    status: &str,
) -> DbResult<()> {
    connection
        .execute(
            "
            INSERT INTO clip_thumbnails (
                clip_id,
                fingerprint,
                status,
                attempt_count,
                updated_at
            )
            VALUES (?1, ?2, ?3, 0, CURRENT_TIMESTAMP)
            ON CONFLICT(clip_id) DO UPDATE SET
                fingerprint = excluded.fingerprint,
                cache_file = NULL,
                status = excluded.status,
                attempt_count = 0,
                next_attempt_at = NULL,
                error_code = NULL,
                last_error = NULL,
                byte_size = NULL,
                revision = NULL,
                generated_at = NULL,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![clip_id, fingerprint, status],
        )
        .map_err(|error| readable_error("resetting thumbnail queue state", error))?;
    Ok(())
}

fn normalized_clip_ids(clip_ids: &[i64]) -> DbResult<Vec<i64>> {
    let ids = clip_ids.iter().copied().collect::<BTreeSet<_>>();
    if ids.len() > MAX_COMMAND_CLIP_IDS {
        return Err(format!(
            "thumbnail commands accept at most {MAX_COMMAND_CLIP_IDS} unique clip ids"
        ));
    }
    Ok(ids.into_iter().collect())
}

fn validate_cache_basename(cache_file: &str) -> DbResult<()> {
    let trimmed = cache_file.trim();
    if trimmed.is_empty()
        || trimmed != cache_file
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.len() > 255
    {
        return Err("thumbnail cache file must be a safe basename".to_string());
    }
    Ok(())
}

fn map_thumbnail_status(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThumbnailStatus> {
    Ok(ThumbnailStatus {
        clip_id: row.get(0)?,
        status: row.get(1)?,
        revision: row.get(2)?,
        error_code: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::db::{
        find_clip_by_id, find_clip_media_paths_by_id, initialize_schema, list_clip_page,
        migrations::SCHEMA_VERSION, normalize_path, upsert_clip, upsert_source_dir, ClipInput,
        ClipListQuery, SourceDirInput,
    };

    #[test]
    fn current_schema_preserves_v8_thumbnail_constraints_and_indexes_idempotently() {
        let connection = Connection::open_in_memory().expect("fixture database should open");
        initialize_schema(&connection).expect("schema should initialize");
        initialize_schema(&connection).expect("schema should initialize repeatedly");

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version should be readable");
        assert_eq!(version, SCHEMA_VERSION);
        for name in [
            "idx_clip_thumbnails_status_due",
            "idx_clip_thumbnails_cache_file",
        ] {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                    params![name],
                    |row| row.get(0),
                )
                .expect("index should be readable");
            assert_eq!(count, 1, "missing index {name}");
        }

        let clip_id = insert_clip(&connection, "schema.mp4", None);
        let unsafe_path = connection.execute(
            "INSERT INTO clip_thumbnails (clip_id, fingerprint, cache_file, status, byte_size, revision) VALUES (?1, 'fp', '../escape.jpg', 'ready', 1, 'r')",
            params![clip_id],
        );
        assert!(unsafe_path.is_err(), "cache files must be basenames");
    }

    #[test]
    fn reconcile_preserves_source_covers_and_exposes_generated_state_separately() {
        let connection = fixture_connection();
        let generated_id = insert_clip(&connection, "generated.mp4", None);
        let source_id = insert_clip(
            &connection,
            "source-cover.mp4",
            Some("D:\\ReadOnlySource\\cover-source-cover.jpeg"),
        );

        let reconciled = reconcile_clip_thumbnails(&connection, None, false)
            .expect("thumbnail state should reconcile");
        assert_eq!(
            reconciled.counts,
            ThumbnailEnsureResult {
                requested: 2,
                queued: 1,
                already_queued: 0,
                skipped: 1,
            }
        );
        assert_eq!(
            get_thumbnail_status(&connection, source_id)
                .expect("status should load")
                .expect("source status should exist")
                .status,
            "suppressed"
        );

        let job = claim_next_thumbnail_job(&connection, "2026-07-16T00:00:00Z")
            .expect("claim should succeed")
            .expect("generated clip should be queued");
        assert_eq!(job.clip_id, generated_id);
        let cache_file = format!("{}-{}.jpg", job.clip_id, job.fingerprint);
        assert!(complete_thumbnail_job_if_current(
            &connection,
            &job,
            &cache_file,
            1234,
            &job.fingerprint,
        )
        .expect("completion should update"));

        let clip = find_clip_by_id(&connection, generated_id).expect("clip should load");
        assert_eq!(clip.cover_path, None);
        assert_eq!(clip.cover_source, "missing");
        assert_eq!(clip.thumbnail_status.as_deref(), Some("ready"));
        assert_eq!(
            clip.thumbnail_revision.as_deref(),
            Some(job.fingerprint.as_str())
        );
        let serialized = serde_json::to_value(&clip).expect("clip should serialize");
        assert_eq!(serialized["thumbnailStatus"], "ready");
        assert_eq!(serialized["thumbnailRevision"], job.fingerprint);
        let page =
            list_clip_page(&connection, &ClipListQuery::default()).expect("clip page should load");
        let summary = page
            .items
            .iter()
            .find(|item| item.id == generated_id)
            .expect("summary should be present");
        assert_eq!(summary.thumbnail_status.as_deref(), Some("ready"));
        assert_eq!(
            summary.thumbnail_revision.as_deref(),
            Some(job.fingerprint.as_str())
        );

        let generated_media = find_clip_media_paths_by_id(&connection, generated_id)
            .expect("generated media should load");
        assert_eq!(generated_media.cover_path, None);
        assert_eq!(
            generated_media.generated_cover_file.as_deref(),
            Some(cache_file.as_str())
        );
        let source_media =
            find_clip_media_paths_by_id(&connection, source_id).expect("source media should load");
        assert!(source_media.generated_cover_file.is_none());
        assert_eq!(source_media.cover_source, "file");
    }

    #[test]
    fn claim_completion_failure_retry_and_stale_fingerprint_are_deterministic() {
        let connection = fixture_connection();
        let clip_id = insert_clip(&connection, "lifecycle.mp4", None);
        ensure_clip_thumbnails(&connection, &[clip_id]).expect("clip should queue");
        let original_fingerprint = thumbnail_fingerprint(
            &normalize_path("D:\\ReadOnlySource\\lifecycle.mp4"),
            100,
            Some("100"),
        );
        connection
            .execute(
                "UPDATE clips SET size_bytes = 101, modified_at = '101' WHERE id = ?1",
                params![clip_id],
            )
            .unwrap();
        let job = claim_next_thumbnail_job(&connection, "2026-07-16T00:00:00Z")
            .expect("claim should run")
            .expect("job should exist");
        assert_eq!(job.attempt_count, 1);
        assert_ne!(job.fingerprint, original_fingerprint);
        assert_eq!(
            job.fingerprint,
            thumbnail_fingerprint(&job.normalized_path, 101, Some("101"))
        );
        let stale_cache_file = format!("{clip_id}-stale-fingerprint.jpg");
        assert!(!complete_thumbnail_job_if_current(
            &connection,
            &ThumbnailJob {
                fingerprint: "stale-fingerprint".to_string(),
                ..job.clone()
            },
            &stale_cache_file,
            1,
            "stale-fingerprint",
        )
        .expect("stale completion should be ignored"));
        assert!(fail_thumbnail_job_if_current(
            &connection,
            clip_id,
            &job.fingerprint,
            "pending",
            "ffmpeg-failed",
            Some("decoder error"),
            Some("2026-07-16T00:10:00Z"),
        )
        .expect("failure should persist"));
        assert!(
            claim_next_thumbnail_job(&connection, "2026-07-16T00:05:00Z")
                .expect("early claim should run")
                .is_none()
        );
        let retry = claim_next_thumbnail_job(&connection, "2026-07-16T00:11:00Z")
            .expect("due claim should run")
            .expect("retry should be due");
        assert_eq!(retry.attempt_count, 2);
        assert_eq!(recover_running_thumbnail_jobs(&connection).unwrap(), 1);
        assert_eq!(
            get_thumbnail_status(&connection, clip_id)
                .unwrap()
                .unwrap()
                .status,
            "pending"
        );

        let claimed = claim_next_thumbnail_job(&connection, "2026-07-16T00:12:00Z")
            .unwrap()
            .unwrap();
        assert!(fail_thumbnail_job_if_current(
            &connection,
            clip_id,
            &claimed.fingerprint,
            "failed",
            "ffmpeg-failed",
            None,
            None,
        )
        .unwrap());
        let ensured = ensure_clip_thumbnails(&connection, &[clip_id]).unwrap();
        assert_eq!(ensured.counts.skipped, 1);
        let retried = retry_clip_thumbnails(&connection, &[clip_id]).unwrap();
        assert_eq!(retried.counts.queued, 1);
    }

    #[test]
    fn ready_commit_rejects_clip_changes_that_arrive_before_reconciliation() {
        let connection = fixture_connection();
        let clip_id = insert_clip(&connection, "scan-race.mp4", None);
        ensure_clip_thumbnails(&connection, &[clip_id]).unwrap();
        let job = claim_next_thumbnail_job(&connection, "2026-07-16T00:00:00Z")
            .unwrap()
            .unwrap();
        connection
            .execute(
                "
                UPDATE clips
                SET size_bytes = size_bytes + 1,
                    modified_at = 'changed-during-generation',
                    cover_path = 'D:\\ReadOnlySource\\cover-scan-race.jpeg',
                    cover_source = 'file'
                WHERE id = ?1
                ",
                params![clip_id],
            )
            .unwrap();
        let cache_file = format!("{clip_id}-{}.jpg", job.fingerprint);

        assert!(!complete_thumbnail_job_if_current(
            &connection,
            &job,
            &cache_file,
            1024,
            &job.fingerprint,
        )
        .unwrap());
        assert_eq!(
            get_thumbnail_status(&connection, clip_id)
                .unwrap()
                .unwrap()
                .status,
            "running"
        );

        let reconciled = reconcile_clip_thumbnails(&connection, Some(&[clip_id]), false).unwrap();
        assert_eq!(reconciled.changed[0].status, "suppressed");
        assert_eq!(
            get_thumbnail_status(&connection, clip_id)
                .unwrap()
                .unwrap()
                .status,
            "suppressed"
        );
    }

    #[test]
    fn fingerprint_changes_requeue_ready_rows_and_eviction_tracks_cache_bytes() {
        let connection = fixture_connection();
        let clip_id = insert_clip(&connection, "invalidate.mp4", None);
        ensure_clip_thumbnails(&connection, &[clip_id]).unwrap();
        let job = claim_next_thumbnail_job(&connection, "2026-07-16T00:00:00Z")
            .unwrap()
            .unwrap();
        let first_cache_file = format!("{}-{}.jpg", clip_id, job.fingerprint);
        complete_thumbnail_job_if_current(
            &connection,
            &job,
            &first_cache_file,
            4096,
            &job.fingerprint,
        )
        .unwrap();
        assert_eq!(
            get_thumbnail_queue_status(&connection).unwrap().cache_bytes,
            4096
        );
        assert_eq!(
            list_ready_thumbnail_cache_refs(&connection).unwrap().len(),
            1
        );

        connection
            .execute(
                "UPDATE clips SET size_bytes = size_bytes + 1, modified_at = '200' WHERE id = ?1",
                params![clip_id],
            )
            .unwrap();
        let changed = reconcile_clip_thumbnails(&connection, Some(&[clip_id]), false).unwrap();
        assert_eq!(changed.counts.queued, 1);
        let status = get_thumbnail_status(&connection, clip_id).unwrap().unwrap();
        assert_eq!(status.status, "pending");
        assert_eq!(status.revision, None);

        let refreshed = claim_next_thumbnail_job(&connection, "2026-07-16T00:01:00Z")
            .unwrap()
            .unwrap();
        let refreshed_cache_file = format!("{}-{}.jpg", clip_id, refreshed.fingerprint);
        complete_thumbnail_job_if_current(
            &connection,
            &refreshed,
            &refreshed_cache_file,
            2048,
            &refreshed.fingerprint,
        )
        .unwrap();
        assert!(
            mark_thumbnail_evicted_if_current(&connection, clip_id, &refreshed.fingerprint)
                .unwrap()
        );
        let status = get_thumbnail_queue_status(&connection).unwrap();
        assert_eq!(status.evicted, 1);
        assert_eq!(status.cache_bytes, 0);
        assert!(list_ready_thumbnail_cache_refs(&connection)
            .unwrap()
            .is_empty());

        let global = reconcile_clip_thumbnails(&connection, None, false).unwrap();
        assert_eq!(
            global.counts.queued, 0,
            "global scans must not undo eviction"
        );
        assert_eq!(
            get_thumbnail_status(&connection, clip_id)
                .unwrap()
                .unwrap()
                .status,
            "evicted"
        );
        let visible = ensure_clip_thumbnails(&connection, &[clip_id]).unwrap();
        assert_eq!(visible.counts.queued, 1);
        assert_eq!(
            get_thumbnail_status(&connection, clip_id)
                .unwrap()
                .unwrap()
                .status,
            "pending"
        );
    }

    #[test]
    fn global_generator_availability_transitions_are_set_based_and_recoverable() {
        let connection = fixture_connection();
        let first = insert_clip(&connection, "unavailable-1.mp4", None);
        let second = insert_clip(&connection, "unavailable-2.mp4", None);
        ensure_clip_thumbnails(&connection, &[first, second]).unwrap();

        assert_eq!(
            mark_pending_thumbnails_unavailable(&connection, "ffmpeg-unavailable").unwrap(),
            2
        );
        let unavailable = get_thumbnail_queue_status(&connection).unwrap();
        assert_eq!(unavailable.pending, 0);
        assert_eq!(unavailable.unavailable, 2);
        assert!(list_thumbnail_statuses(&connection, &[second, first])
            .unwrap()
            .iter()
            .all(|status| status.error_code.as_deref() == Some("ffmpeg-unavailable")));

        assert_eq!(recover_unavailable_thumbnail_jobs(&connection).unwrap(), 2);
        let recovered = get_thumbnail_queue_status(&connection).unwrap();
        assert_eq!(recovered.pending, 2);
        assert_eq!(recovered.unavailable, 0);
    }

    fn fixture_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("fixture database should open");
        initialize_schema(&connection).expect("fixture schema should initialize");
        connection
    }

    fn insert_clip(connection: &Connection, file_name: &str, cover_path: Option<&str>) -> i64 {
        let source = upsert_source_dir(
            connection,
            SourceDirInput {
                path: "D:\\ReadOnlySource",
                name: "Read-only source",
            },
        )
        .expect("fixture source should upsert");
        let video_path = format!("D:\\ReadOnlySource\\{file_name}");
        upsert_clip(
            connection,
            ClipInput {
                source_dir_id: source.id,
                clip_group_id: None,
                video_path: &video_path,
                file_name,
                file_size: 100,
                modified_at: Some("100"),
                duration_ms: Some(1_000),
                recorded_at: None,
                cover_path,
                cover_source: if cover_path.is_some() {
                    "file"
                } else {
                    "missing"
                },
            },
        )
        .expect("fixture clip should upsert")
        .id
    }
}
