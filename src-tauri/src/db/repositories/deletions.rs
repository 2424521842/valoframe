//! Durable, crash-recoverable permanent deletion.
//!
//! SQLite cannot roll back a filesystem deletion. This module therefore records an immutable
//! delete intent in a committed transaction before touching the video, removes the verified file
//! outside any database transaction, and finally removes the intent and clip row atomically.

use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::{
    super::{
        ensure_row_changed, readable_error, BatchClipMutationResult, ClipFileTarget, DbResult,
    },
    clips::{find_clip_by_id, find_clip_file_target_by_id},
};

const DELETE_LEASE_DURATION_SQL: &str = "+5 minutes";
const DELETE_PENDING_CODE: &str = "delete-pending";
static DELETE_WORKER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipDeleteIssue {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClipDeleteItemOutcome {
    Deleted,
    Missing,
    Pending(ClipDeleteIssue),
    Blocked(ClipDeleteIssue),
    Rejected(ClipDeleteIssue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipDeleteRecoveryFailure {
    pub clip_id: i64,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ClipDeleteRecoveryResult {
    pub attempted: usize,
    pub deleted_ids: Vec<i64>,
    pub pending_ids: Vec<i64>,
    pub blocked_ids: Vec<i64>,
    pub failures: Vec<ClipDeleteRecoveryFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipDeleteIntent {
    id: i64,
    clip_id: i64,
    state: String,
    video_path: String,
    canonical_video_path: Option<String>,
    source_dir_path: String,
    canonical_source_dir_path: Option<String>,
    extension: String,
    file_existed: bool,
    file_size_bytes: Option<i64>,
    file_modified_ticks: Option<i64>,
    file_volume_serial: Option<i64>,
    file_index_high: Option<i64>,
    file_index_low: Option<i64>,
    source_volume_serial: Option<i64>,
    source_file_index_high: Option<i64>,
    source_file_index_low: Option<i64>,
    lease_owner: Option<String>,
    last_error_code: Option<String>,
    last_error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrashIdentitySnapshot {
    video_path: String,
    canonical_video_path: String,
    source_dir_path: String,
    canonical_source_dir_path: String,
    extension: String,
    file_existed: bool,
    file_size_bytes: Option<i64>,
    file_modified_ticks: Option<i64>,
    file_identity: StableFileIdentity,
    source_identity: StableFileIdentity,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StableFileIdentity {
    volume_serial: Option<i64>,
    file_index_high: Option<i64>,
    file_index_low: Option<i64>,
}

struct StableFileSnapshot {
    size_bytes: i64,
    modified_ticks: i64,
    identity: StableFileIdentity,
}

struct AccessibleDirectory {
    canonical_path: PathBuf,
    identity: StableFileIdentity,
    #[cfg(windows)]
    _handle: fs::File,
}

enum StageDeleteOutcome {
    Intent(Box<ClipDeleteIntent>),
    Missing,
    Rejected(ClipDeleteIssue),
}

enum TrashPreparation {
    Missing,
    AlreadyTrashed,
    Snapshot(Box<TrashIdentitySnapshot>),
}

enum ClaimDeleteOutcome {
    Claimed(ClipDeleteIntent),
    Busy(ClipDeleteIntent),
    Blocked(ClipDeleteIntent),
    Missing,
}

enum VerifiedDeleteTarget {
    Present(VerifiedDeleteFile),
    Missing(AccessibleDirectory),
}

struct VerifiedDeleteFile {
    path: PathBuf,
    #[cfg(windows)]
    handle: fs::File,
}

#[derive(Debug)]
enum VerificationFailure {
    Pending(ClipDeleteIssue),
    Blocked(ClipDeleteIssue),
}

/// Stages and attempts one permanent deletion. Every filesystem deletion performed by this
/// function is backed by a committed `clip_delete_intents` row first.
pub(crate) fn delete_clip_permanently(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<ClipDeleteItemOutcome> {
    if let Some(intent) = find_delete_intent_by_clip_id(connection, clip_id)? {
        return process_delete_intent(connection, intent.id);
    }

    match stage_delete_intent(connection, clip_id)? {
        StageDeleteOutcome::Intent(intent) => process_delete_intent(connection, intent.id),
        StageDeleteOutcome::Missing => Ok(ClipDeleteItemOutcome::Missing),
        StageDeleteOutcome::Rejected(problem) => Ok(ClipDeleteItemOutcome::Rejected(problem)),
    }
}

/// Retries every pending intent and every processing intent whose lease expired. Blocked intents
/// require an explicit future user decision and are deliberately not retried at startup.
pub(crate) fn recover_pending_clip_deletions(
    connection: &Connection,
) -> DbResult<ClipDeleteRecoveryResult> {
    let recoverable = list_recoverable_delete_intents(connection)?;
    let mut result = ClipDeleteRecoveryResult {
        attempted: recoverable.len(),
        ..ClipDeleteRecoveryResult::default()
    };

    for (intent_id, clip_id) in recoverable {
        match process_delete_intent(connection, intent_id) {
            Ok(ClipDeleteItemOutcome::Deleted | ClipDeleteItemOutcome::Missing) => {
                result.deleted_ids.push(clip_id);
            }
            Ok(ClipDeleteItemOutcome::Pending(_)) => result.pending_ids.push(clip_id),
            Ok(ClipDeleteItemOutcome::Blocked(_)) => result.blocked_ids.push(clip_id),
            Ok(ClipDeleteItemOutcome::Rejected(problem)) => {
                result.failures.push(ClipDeleteRecoveryFailure {
                    clip_id,
                    message: problem.message,
                });
            }
            Err(message) => result
                .failures
                .push(ClipDeleteRecoveryFailure { clip_id, message }),
        }
    }

    Ok(result)
}

/// Restores clips using the same transaction that verifies no durable delete authorization
/// exists. This closes the race between a command-level preflight check and the status update.
pub(crate) fn set_clips_trashed_guarded(
    connection: &Connection,
    clip_ids: &[i64],
    trashed: bool,
) -> DbResult<BatchClipMutationResult> {
    let clip_ids = deduplicate_clip_ids(clip_ids);
    if clip_ids.is_empty() {
        return Ok(BatchClipMutationResult {
            requested: 0,
            matched: 0,
            updated: 0,
            missing_ids: Vec::new(),
            clips: Vec::new(),
        });
    }

    if trashed {
        move_clips_to_trash(connection, clip_ids)
    } else {
        restore_clips_from_trash(connection, clip_ids)
    }
}

fn move_clips_to_trash(
    connection: &Connection,
    clip_ids: Vec<i64>,
) -> DbResult<BatchClipMutationResult> {
    // Filesystem identity is captured before the write transaction. The transaction then checks
    // that the indexed target is unchanged and commits the immutable snapshot with the status
    // transition. A later replacement can never refresh this authorization.
    let preparations = clip_ids
        .iter()
        .map(|clip_id| {
            let preparation = match find_clip_file_target_by_id(connection, *clip_id)? {
                None => TrashPreparation::Missing,
                Some(target) if target.file_status == "trashed" => TrashPreparation::AlreadyTrashed,
                Some(target) => TrashPreparation::Snapshot(Box::new(
                    prepare_trash_snapshot(&target)
                        .map_err(|problem| format!("{}: {}", problem.code, problem.message))?,
                )),
            };
            Ok((*clip_id, preparation))
        })
        .collect::<DbResult<Vec<_>>>()?;

    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| readable_error("starting guarded trash transaction", error))?;
    let mut matched_ids = Vec::with_capacity(clip_ids.len());
    let mut missing_ids = Vec::new();
    let mut updated = 0;

    for (clip_id, preparation) in preparations {
        let Some(current) = find_clip_file_target_by_id(&transaction, clip_id)? else {
            missing_ids.push(clip_id);
            continue;
        };
        matched_ids.push(clip_id);

        if current.file_status == "trashed" {
            // In particular, do not silently grant legacy trashed rows that lack a v13 snapshot.
            // They must be restored and explicitly moved to trash again.
            continue;
        }

        let TrashPreparation::Snapshot(snapshot) = preparation else {
            return Err(format!(
                "Recycle-bin target {clip_id} changed while its identity was being captured; retry the operation."
            ));
        };
        if current.video_path != snapshot.video_path
            || current.source_dir_path != snapshot.source_dir_path
            || !current.extension.eq_ignore_ascii_case(&snapshot.extension)
        {
            return Err(format!(
                "Recycle-bin target {clip_id} changed while its identity was being captured; retry the operation."
            ));
        }
        if find_trash_snapshot_by_clip_id(&transaction, clip_id)?.is_some() {
            return Err(format!(
                "Recycle-bin target {clip_id} already has an identity snapshot in a non-trashed state; restore and retry."
            ));
        }
        insert_trash_snapshot(&transaction, clip_id, &snapshot)?;
        updated += transaction
            .execute(
                "
                UPDATE clips
                SET file_status = 'trashed',
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                  AND file_status <> 'trashed'
                ",
                params![clip_id],
            )
            .map_err(|error| readable_error("moving clip to recycle bin", error))?;
    }

    finish_trash_mutation(
        transaction,
        clip_ids.len(),
        matched_ids,
        updated,
        missing_ids,
        "guarded trash transaction",
    )
}

fn restore_clips_from_trash(
    connection: &Connection,
    clip_ids: Vec<i64>,
) -> DbResult<BatchClipMutationResult> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| readable_error("starting guarded restore transaction", error))?;
    let mut matched_ids = Vec::with_capacity(clip_ids.len());
    let mut missing_ids = Vec::new();
    let mut updated = 0;

    for clip_id in &clip_ids {
        let current = transaction
            .query_row(
                "
                SELECT
                    clips.file_path,
                    clips.file_status,
                    EXISTS (
                        SELECT 1
                        FROM clip_delete_intents
                        WHERE clip_delete_intents.clip_id = clips.id
                    )
                FROM clips
                WHERE clips.id = ?1
                ",
                params![clip_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    ))
                },
            )
            .optional()
            .map_err(|error| readable_error("matching clips for guarded restore", error))?;

        let Some((video_path, current_status, has_delete_intent)) = current else {
            missing_ids.push(*clip_id);
            continue;
        };
        if has_delete_intent {
            return Err(format!(
                "{DELETE_PENDING_CODE}: 素材 {clip_id} 已进入永久删除队列，无法恢复"
            ));
        }
        matched_ids.push(*clip_id);

        let next_status = if Path::new(&video_path).is_file() {
            "available"
        } else {
            "missing"
        };
        if current_status != next_status {
            updated += transaction
                .execute(
                    "
                    UPDATE clips
                    SET file_status = ?1,
                        updated_at = CURRENT_TIMESTAMP
                    WHERE id = ?2
                    ",
                    params![next_status, clip_id],
                )
                .map_err(|error| readable_error("updating guarded restore batch", error))?;
        }
        transaction
            .execute(
                "DELETE FROM clip_trash_snapshots WHERE clip_id = ?1",
                params![clip_id],
            )
            .map_err(|error| readable_error("clearing restored trash snapshot", error))?;
    }

    finish_trash_mutation(
        transaction,
        clip_ids.len(),
        matched_ids,
        updated,
        missing_ids,
        "guarded restore transaction",
    )
}

fn finish_trash_mutation(
    transaction: Transaction<'_>,
    requested: usize,
    matched_ids: Vec<i64>,
    updated: usize,
    missing_ids: Vec<i64>,
    operation: &str,
) -> DbResult<BatchClipMutationResult> {
    let clips = matched_ids
        .iter()
        .map(|clip_id| find_clip_by_id(&transaction, *clip_id))
        .collect::<DbResult<Vec<_>>>()?;
    let result = BatchClipMutationResult {
        requested,
        matched: matched_ids.len(),
        updated,
        missing_ids,
        clips,
    };
    transaction
        .commit()
        .map_err(|error| readable_error(&format!("committing {operation}"), error))?;
    Ok(result)
}

/// Removes an index row only when no durable delete authorization exists. The check and delete
/// share one write transaction; the schema's `ON DELETE RESTRICT` remains a second line of defense.
pub(crate) fn delete_clip_from_index_guarded(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<()> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| readable_error("starting guarded index removal", error))?;
    let has_delete_intent = transaction
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM clip_delete_intents WHERE clip_id = ?1)",
            params![clip_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| readable_error("checking pending permanent deletion", error))?
        != 0;
    if has_delete_intent {
        return Err(format!(
            "{DELETE_PENDING_CODE}: 素材 {clip_id} 已进入永久删除队列，无法仅移除索引"
        ));
    }

    let changed = transaction
        .execute("DELETE FROM clips WHERE id = ?1", params![clip_id])
        .map_err(|error| readable_error("removing clip from index", error))?;
    ensure_row_changed(changed, "removing clip from index", clip_id)?;
    transaction
        .commit()
        .map_err(|error| readable_error("committing guarded index removal", error))
}

fn prepare_trash_snapshot(
    target: &ClipFileTarget,
) -> Result<TrashIdentitySnapshot, ClipDeleteIssue> {
    let video_path = Path::new(&target.video_path);
    let source = inspect_accessible_directory(Path::new(&target.source_dir_path))?;
    let target_metadata = match fs::symlink_metadata(video_path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(issue(
                fs_error_code(&error),
                format!("无法验证本地视频路径“{}”：{error}", video_path.display()),
                true,
            ));
        }
    };

    let (canonical_video_path, file_existed, file_size_bytes, file_modified_ticks, file_identity) =
        if let Some(metadata) = target_metadata {
            if !metadata.is_file() || metadata_is_reparse_or_symlink(&metadata) {
                return Err(issue(
                    "unsafe-path",
                    "视频目标不是普通文件或包含重解析点，已拒绝永久删除",
                    false,
                ));
            }
            let canonical_video_path = fs::canonicalize(video_path).map_err(|error| {
                issue(
                    fs_error_code(&error),
                    format!("无法规范化本地视频路径：{error}"),
                    true,
                )
            })?;
            if !path_is_within(&canonical_video_path, &source.canonical_path) {
                return Err(issue(
                    "unsafe-path",
                    "视频路径不在已索引的来源目录内，已拒绝永久删除",
                    false,
                ));
            }
            let snapshot = snapshot_existing_file(video_path, &metadata)?;
            (
                canonical_video_path,
                true,
                Some(snapshot.size_bytes),
                Some(snapshot.modified_ticks),
                snapshot.identity,
            )
        } else {
            let canonical_video_path = canonicalize_missing_target(
                video_path,
                Path::new(&target.source_dir_path),
                &source.canonical_path,
            )?;
            (
                canonical_video_path,
                false,
                None,
                None,
                StableFileIdentity::default(),
            )
        };

    Ok(TrashIdentitySnapshot {
        video_path: target.video_path.clone(),
        canonical_video_path: canonical_video_path.to_string_lossy().into_owned(),
        source_dir_path: target.source_dir_path.clone(),
        canonical_source_dir_path: source.canonical_path.to_string_lossy().into_owned(),
        extension: target.extension.clone(),
        file_existed,
        file_size_bytes,
        file_modified_ticks,
        file_identity,
        source_identity: source.identity,
    })
}

fn insert_trash_snapshot(
    connection: &Connection,
    clip_id: i64,
    snapshot: &TrashIdentitySnapshot,
) -> DbResult<()> {
    connection
        .execute(
            "
            INSERT INTO clip_trash_snapshots (
                clip_id,
                video_path,
                canonical_video_path,
                source_dir_path,
                canonical_source_dir_path,
                extension,
                file_existed,
                file_size_bytes,
                file_modified_ticks,
                file_volume_serial,
                file_index_high,
                file_index_low,
                source_volume_serial,
                source_file_index_high,
                source_file_index_low
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
            )
            ",
            params![
                clip_id,
                snapshot.video_path,
                snapshot.canonical_video_path,
                snapshot.source_dir_path,
                snapshot.canonical_source_dir_path,
                snapshot.extension,
                i64::from(snapshot.file_existed),
                snapshot.file_size_bytes,
                snapshot.file_modified_ticks,
                snapshot.file_identity.volume_serial,
                snapshot.file_identity.file_index_high,
                snapshot.file_identity.file_index_low,
                snapshot.source_identity.volume_serial,
                snapshot.source_identity.file_index_high,
                snapshot.source_identity.file_index_low,
            ],
        )
        .map_err(|error| readable_error("recording recycle-bin identity snapshot", error))?;
    Ok(())
}

fn find_trash_snapshot_by_clip_id(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<Option<TrashIdentitySnapshot>> {
    connection
        .query_row(
            "
            SELECT
                video_path,
                canonical_video_path,
                source_dir_path,
                canonical_source_dir_path,
                extension,
                file_existed,
                file_size_bytes,
                file_modified_ticks,
                file_volume_serial,
                file_index_high,
                file_index_low,
                source_volume_serial,
                source_file_index_high,
                source_file_index_low
            FROM clip_trash_snapshots
            WHERE clip_id = ?1
            ",
            params![clip_id],
            |row| {
                Ok(TrashIdentitySnapshot {
                    video_path: row.get(0)?,
                    canonical_video_path: row.get(1)?,
                    source_dir_path: row.get(2)?,
                    canonical_source_dir_path: row.get(3)?,
                    extension: row.get(4)?,
                    file_existed: row.get::<_, i64>(5)? != 0,
                    file_size_bytes: row.get(6)?,
                    file_modified_ticks: row.get(7)?,
                    file_identity: StableFileIdentity {
                        volume_serial: row.get(8)?,
                        file_index_high: row.get(9)?,
                        file_index_low: row.get(10)?,
                    },
                    source_identity: StableFileIdentity {
                        volume_serial: row.get(11)?,
                        file_index_high: row.get(12)?,
                        file_index_low: row.get(13)?,
                    },
                })
            },
        )
        .optional()
        .map_err(|error| readable_error("reading recycle-bin identity snapshot", error))
}

fn stage_delete_intent(connection: &Connection, clip_id: i64) -> DbResult<StageDeleteOutcome> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| readable_error("starting permanent-delete intent transaction", error))?;
    if let Some(intent) = find_delete_intent_by_clip_id(&transaction, clip_id)? {
        transaction
            .commit()
            .map_err(|error| readable_error("finishing existing delete intent lookup", error))?;
        return Ok(StageDeleteOutcome::Intent(Box::new(intent)));
    }

    let Some(current) = find_clip_file_target_by_id(&transaction, clip_id)? else {
        transaction
            .commit()
            .map_err(|error| readable_error("finishing missing delete target lookup", error))?;
        return Ok(StageDeleteOutcome::Missing);
    };
    if current.file_status != "trashed" {
        return Ok(StageDeleteOutcome::Rejected(issue(
            "not-trashed",
            "素材不在回收站，无法永久删除本地视频",
            false,
        )));
    }
    if !current.extension.eq_ignore_ascii_case("mp4")
        || !Path::new(&current.video_path)
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("mp4"))
    {
        return Ok(StageDeleteOutcome::Rejected(issue(
            "not-mp4",
            "目标不是已索引的 MP4 视频，已拒绝永久删除",
            false,
        )));
    }
    let Some(snapshot) = find_trash_snapshot_by_clip_id(&transaction, clip_id)? else {
        return Ok(StageDeleteOutcome::Rejected(issue(
            "trash-snapshot-missing",
            "该素材没有移入回收站时的文件身份快照；为避免删除同路径替换文件，请先恢复，再重新移入回收站",
            false,
        )));
    };
    if current.video_path != snapshot.video_path
        || current.source_dir_path != snapshot.source_dir_path
        || !current.extension.eq_ignore_ascii_case(&snapshot.extension)
    {
        return Ok(StageDeleteOutcome::Rejected(issue(
            "target-changed",
            "索引目标在授权永久删除前发生变化，请刷新后重试",
            true,
        )));
    }

    transaction
        .execute(
            "
            INSERT INTO clip_delete_intents (
                clip_id,
                state,
                video_path,
                canonical_video_path,
                source_dir_path,
                canonical_source_dir_path,
                extension,
                file_existed,
                file_size_bytes,
                file_modified_ticks,
                file_volume_serial,
                file_index_high,
                file_index_low,
                source_volume_serial,
                source_file_index_high,
                source_file_index_low
            )
            VALUES (
                ?1, 'pending', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15
            )
            ",
            params![
                clip_id,
                snapshot.video_path,
                snapshot.canonical_video_path,
                snapshot.source_dir_path,
                snapshot.canonical_source_dir_path,
                snapshot.extension,
                i64::from(snapshot.file_existed),
                snapshot.file_size_bytes,
                snapshot.file_modified_ticks,
                snapshot.file_identity.volume_serial,
                snapshot.file_identity.file_index_high,
                snapshot.file_identity.file_index_low,
                snapshot.source_identity.volume_serial,
                snapshot.source_identity.file_index_high,
                snapshot.source_identity.file_index_low,
            ],
        )
        .map_err(|error| readable_error("recording permanent-delete intent", error))?;
    let intent = find_delete_intent_by_clip_id(&transaction, clip_id)?.ok_or_else(|| {
        "Database recording permanent-delete intent failed: inserted row was not readable"
            .to_string()
    })?;
    transaction
        .commit()
        .map_err(|error| readable_error("committing permanent-delete intent", error))?;
    Ok(StageDeleteOutcome::Intent(Box::new(intent)))
}

fn process_delete_intent(
    connection: &Connection,
    intent_id: i64,
) -> DbResult<ClipDeleteItemOutcome> {
    let worker_id = next_delete_worker_id();
    let intent = match claim_delete_intent(connection, intent_id, &worker_id)? {
        ClaimDeleteOutcome::Claimed(intent) => intent,
        ClaimDeleteOutcome::Busy(intent) => {
            return Ok(ClipDeleteItemOutcome::Pending(intent_problem(
                &intent,
                DELETE_PENDING_CODE,
                "永久删除已由另一个任务处理",
                true,
            )));
        }
        ClaimDeleteOutcome::Blocked(intent) => {
            let problem = intent_problem(
                &intent,
                "delete-blocked",
                "永久删除因目标安全校验失败而暂停",
                false,
            );
            let problem = cancel_blocked_delete_authorization(connection, &intent, problem)?;
            return Ok(ClipDeleteItemOutcome::Blocked(problem));
        }
        ClaimDeleteOutcome::Missing => return Ok(ClipDeleteItemOutcome::Deleted),
    };

    let verified_target = match verify_delete_target(&intent) {
        Ok(target) => target,
        Err(VerificationFailure::Pending(problem)) => {
            persist_delete_problem(connection, &intent, "pending", &problem)?;
            return Ok(ClipDeleteItemOutcome::Pending(problem));
        }
        Err(VerificationFailure::Blocked(problem)) => {
            let problem = cancel_blocked_delete_authorization(connection, &intent, problem)?;
            return Ok(ClipDeleteItemOutcome::Blocked(problem));
        }
    };

    let _source_guard = match verified_target {
        VerifiedDeleteTarget::Present(target) => {
            let display_path = target.path.clone();
            match delete_verified_file(target) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    if let Err(failure) = verify_delete_target_is_missing(&intent) {
                        let (state, problem) = verification_failure_parts(failure);
                        return Ok(if state == "blocked" {
                            let problem =
                                cancel_blocked_delete_authorization(connection, &intent, problem)?;
                            ClipDeleteItemOutcome::Blocked(problem)
                        } else {
                            persist_delete_problem(connection, &intent, state, &problem)?;
                            ClipDeleteItemOutcome::Pending(problem)
                        });
                    }
                }
                Err(error) => {
                    let problem = issue(
                        fs_error_code(&error),
                        format!("无法删除本地视频“{}”：{error}", display_path.display()),
                        true,
                    );
                    persist_delete_problem(connection, &intent, "pending", &problem)?;
                    return Ok(ClipDeleteItemOutcome::Pending(problem));
                }
            }
            None
        }
        VerifiedDeleteTarget::Missing(source_guard) => Some(source_guard),
    };

    match finalize_delete_intent(connection, &intent) {
        Ok(()) => Ok(ClipDeleteItemOutcome::Deleted),
        Err(finalize_error) => {
            // A local SQLite commit error is uncommon but can be ambiguous. Confirm success before
            // reporting a pending retry; never recreate an intent from a possibly stale snapshot.
            let intent_still_exists = find_delete_intent_by_id(connection, intent.id)?.is_some();
            let clip_still_exists = clip_exists(connection, intent.clip_id)?;
            if !intent_still_exists && !clip_still_exists {
                return Ok(ClipDeleteItemOutcome::Deleted);
            }

            let problem = issue(
                "database-finalize-pending",
                format!("本地视频已删除，索引收尾将在恢复流程中重试：{finalize_error}"),
                true,
            );
            if intent_still_exists {
                persist_delete_problem(connection, &intent, "pending", &problem)?;
            }
            Ok(ClipDeleteItemOutcome::Pending(problem))
        }
    }
}

fn claim_delete_intent(
    connection: &Connection,
    intent_id: i64,
    worker_id: &str,
) -> DbResult<ClaimDeleteOutcome> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| readable_error("starting permanent-delete lease transaction", error))?;
    let Some(existing) = find_delete_intent_by_id(&transaction, intent_id)? else {
        transaction
            .commit()
            .map_err(|error| readable_error("finishing missing delete lease lookup", error))?;
        return Ok(ClaimDeleteOutcome::Missing);
    };
    if existing.state == "blocked" {
        transaction
            .commit()
            .map_err(|error| readable_error("finishing blocked delete lease lookup", error))?;
        return Ok(ClaimDeleteOutcome::Blocked(existing));
    }

    let changed = transaction
        .execute(
            "
            UPDATE clip_delete_intents
            SET state = 'processing',
                lease_owner = ?1,
                lease_expires_at = datetime('now', ?2),
                attempt_count = attempt_count + 1,
                last_attempt_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?3
              AND (
                  state = 'pending'
                  OR (
                      state = 'processing'
                      AND (lease_expires_at IS NULL OR lease_expires_at <= CURRENT_TIMESTAMP)
                  )
              )
            ",
            params![worker_id, DELETE_LEASE_DURATION_SQL, intent_id],
        )
        .map_err(|error| readable_error("claiming permanent-delete intent", error))?;
    let current = find_delete_intent_by_id(&transaction, intent_id)?.ok_or_else(|| {
        "Database claiming permanent-delete intent failed: intent disappeared".to_string()
    })?;
    transaction
        .commit()
        .map_err(|error| readable_error("committing permanent-delete lease", error))?;

    if changed == 1 {
        Ok(ClaimDeleteOutcome::Claimed(current))
    } else {
        Ok(ClaimDeleteOutcome::Busy(current))
    }
}

fn verify_delete_target(
    intent: &ClipDeleteIntent,
) -> Result<VerifiedDeleteTarget, VerificationFailure> {
    let source = verify_intent_source(intent)?;
    let video_path = Path::new(&intent.video_path);
    let metadata = match fs::symlink_metadata(video_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            verify_missing_target_location(intent, &source.canonical_path)?;
            return Ok(VerifiedDeleteTarget::Missing(source));
        }
        Err(error) => {
            return Err(VerificationFailure::Pending(issue(
                fs_error_code(&error),
                format!("暂时无法验证待删除视频“{}”：{error}", video_path.display()),
                true,
            )));
        }
    };

    if !intent.file_existed {
        return Err(VerificationFailure::Blocked(issue(
            "target-replaced",
            "永久删除授权后原缺失路径出现了新文件，已阻止删除",
            false,
        )));
    }
    if !metadata.is_file() || metadata_is_reparse_or_symlink(&metadata) {
        return Err(VerificationFailure::Blocked(issue(
            "target-replaced",
            "待删除目标已变为非普通文件或重解析点，已阻止删除",
            false,
        )));
    }
    let canonical_video = fs::canonicalize(video_path).map_err(|error| {
        VerificationFailure::Pending(issue(
            fs_error_code(&error),
            format!("暂时无法规范化待删除视频路径：{error}"),
            true,
        ))
    })?;
    let Some(expected_video) = intent.canonical_video_path.as_deref() else {
        return Err(VerificationFailure::Blocked(issue(
            "invalid-delete-intent",
            "永久删除意图缺少规范化视频路径，已阻止删除",
            false,
        )));
    };
    if !paths_equal(&canonical_video, Path::new(expected_video))
        || !path_is_within(&canonical_video, &source.canonical_path)
        || !canonical_video
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("mp4"))
        || !intent.extension.eq_ignore_ascii_case("mp4")
    {
        return Err(VerificationFailure::Blocked(issue(
            "target-replaced",
            "待删除视频路径或类型与已授权快照不一致，已阻止删除",
            false,
        )));
    }

    let (verified_file, snapshot) =
        open_verified_delete_file(video_path, &metadata).map_err(|problem| {
            if problem.retryable {
                VerificationFailure::Pending(problem)
            } else {
                VerificationFailure::Blocked(problem)
            }
        })?;
    if Some(snapshot.size_bytes) != intent.file_size_bytes
        || Some(snapshot.modified_ticks) != intent.file_modified_ticks
        || snapshot.identity != target_identity(intent)
    {
        return Err(VerificationFailure::Blocked(issue(
            "target-replaced",
            "待删除视频的文件身份与已授权快照不一致，已阻止删除",
            false,
        )));
    }

    Ok(VerifiedDeleteTarget::Present(verified_file))
}

fn verify_delete_target_is_missing(intent: &ClipDeleteIntent) -> Result<(), VerificationFailure> {
    let source = verify_intent_source(intent)?;
    match fs::symlink_metadata(&intent.video_path) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            verify_missing_target_location(intent, &source.canonical_path)
        }
        Ok(_) => Err(VerificationFailure::Blocked(issue(
            "target-replaced",
            "删除过程中目标路径重新出现，已阻止继续收尾",
            false,
        ))),
        Err(error) => Err(VerificationFailure::Pending(issue(
            fs_error_code(&error),
            format!("无法确认视频是否已删除：{error}"),
            true,
        ))),
    }
}

fn verify_intent_source(
    intent: &ClipDeleteIntent,
) -> Result<AccessibleDirectory, VerificationFailure> {
    let source =
        inspect_accessible_directory(Path::new(&intent.source_dir_path)).map_err(|problem| {
            if problem.retryable {
                VerificationFailure::Pending(issue("source-unavailable", problem.message, true))
            } else {
                VerificationFailure::Blocked(issue("source-changed", problem.message, false))
            }
        })?;

    let Some(expected_source) = intent.canonical_source_dir_path.as_deref() else {
        return Err(VerificationFailure::Blocked(issue(
            "invalid-delete-intent",
            "永久删除意图缺少规范化来源路径，已阻止删除",
            false,
        )));
    };
    if !paths_equal(&source.canonical_path, Path::new(expected_source))
        || source.identity != source_identity(intent)
    {
        return Err(VerificationFailure::Blocked(issue(
            "source-changed",
            "素材来源目录的路径或文件身份与永久删除授权快照不一致，已阻止删除",
            false,
        )));
    }
    Ok(source)
}

fn verify_missing_target_location(
    intent: &ClipDeleteIntent,
    canonical_source: &Path,
) -> Result<(), VerificationFailure> {
    let Some(expected_video) = intent.canonical_video_path.as_deref() else {
        return Err(VerificationFailure::Blocked(issue(
            "invalid-delete-intent",
            "永久删除意图缺少规范化视频路径，无法确认缺失状态",
            false,
        )));
    };
    let resolved_missing_path = canonicalize_missing_target(
        Path::new(&intent.video_path),
        Path::new(&intent.source_dir_path),
        canonical_source,
    )
    .map_err(|problem| {
        if problem.retryable {
            VerificationFailure::Pending(problem)
        } else {
            VerificationFailure::Blocked(problem)
        }
    })?;
    if !paths_equal(&resolved_missing_path, Path::new(expected_video)) {
        return Err(VerificationFailure::Blocked(issue(
            "target-replaced",
            "缺失视频的现存祖先或目标路径与授权快照不一致，已阻止收尾",
            false,
        )));
    }
    Ok(())
}

fn finalize_delete_intent(connection: &Connection, intent: &ClipDeleteIntent) -> DbResult<()> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| readable_error("starting permanent-delete finalizer", error))?;
    let clip_status = transaction
        .query_row(
            "SELECT file_status FROM clips WHERE id = ?1",
            params![intent.clip_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| readable_error("checking clip before delete finalizer", error))?;
    if clip_status.as_deref() != Some("trashed") {
        return Err(format!(
            "Database permanent-delete finalizer failed: clip {} is no longer trashed",
            intent.clip_id
        ));
    }

    let intent_changed = transaction
        .execute(
            "DELETE FROM clip_delete_intents WHERE id = ?1 AND lease_owner = ?2",
            params![intent.id, intent.lease_owner],
        )
        .map_err(|error| readable_error("removing completed delete intent", error))?;
    ensure_row_changed(
        intent_changed,
        "removing completed delete intent",
        intent.id,
    )?;
    let clip_changed = transaction
        .execute(
            "DELETE FROM clips WHERE id = ?1 AND file_status = 'trashed'",
            params![intent.clip_id],
        )
        .map_err(|error| readable_error("removing permanently deleted clip", error))?;
    ensure_row_changed(
        clip_changed,
        "removing permanently deleted clip",
        intent.clip_id,
    )?;
    transaction
        .commit()
        .map_err(|error| readable_error("committing permanent-delete finalizer", error))
}

fn persist_delete_problem(
    connection: &Connection,
    intent: &ClipDeleteIntent,
    state: &str,
    problem: &ClipDeleteIssue,
) -> DbResult<()> {
    let changed = connection
        .execute(
            "
            UPDATE clip_delete_intents
            SET state = ?1,
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error_code = ?2,
                last_error_message = ?3,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?4
              AND lease_owner = ?5
            ",
            params![
                state,
                problem.code,
                problem.message,
                intent.id,
                intent.lease_owner,
            ],
        )
        .map_err(|error| readable_error("persisting permanent-delete problem", error))?;
    if changed == 0 {
        // A lease may legitimately expire and be reclaimed. In that case the newer worker owns
        // convergence, so the stale worker must not overwrite its state.
        return Ok(());
    }
    Ok(())
}

fn cancel_blocked_delete_authorization(
    connection: &Connection,
    intent: &ClipDeleteIntent,
    mut problem: ClipDeleteIssue,
) -> DbResult<ClipDeleteIssue> {
    let changed = connection
        .execute(
            "
            DELETE FROM clip_delete_intents
            WHERE id = ?1
              AND (
                  (state = 'processing' AND lease_owner = ?2)
                  OR (state = 'blocked' AND lease_owner IS NULL)
              )
            ",
            params![intent.id, intent.lease_owner],
        )
        .map_err(|error| readable_error("cancelling blocked delete authorization", error))?;
    if changed == 0 && find_delete_intent_by_id(connection, intent.id)?.is_some() {
        return Err(format!(
            "Database cancelling blocked delete authorization failed: intent {} changed owner",
            intent.id
        ));
    }
    problem.message = format!(
        "{}；安全校验已取消本次永久删除授权，文件未删除",
        problem.message
    );
    Ok(problem)
}

fn find_delete_intent_by_clip_id(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<Option<ClipDeleteIntent>> {
    connection
        .query_row(
            &format!("{} WHERE clip_id = ?1", delete_intent_select_sql()),
            params![clip_id],
            map_delete_intent,
        )
        .optional()
        .map_err(|error| readable_error("reading permanent-delete intent by clip", error))
}

fn find_delete_intent_by_id(
    connection: &Connection,
    intent_id: i64,
) -> DbResult<Option<ClipDeleteIntent>> {
    connection
        .query_row(
            &format!("{} WHERE id = ?1", delete_intent_select_sql()),
            params![intent_id],
            map_delete_intent,
        )
        .optional()
        .map_err(|error| readable_error("reading permanent-delete intent", error))
}

fn delete_intent_select_sql() -> &'static str {
    "
    SELECT
        id,
        clip_id,
        state,
        video_path,
        canonical_video_path,
        source_dir_path,
        canonical_source_dir_path,
        extension,
        file_existed,
        file_size_bytes,
        file_modified_ticks,
        file_volume_serial,
        file_index_high,
        file_index_low,
        source_volume_serial,
        source_file_index_high,
        source_file_index_low,
        lease_owner,
        last_error_code,
        last_error_message
    FROM clip_delete_intents
    "
}

fn map_delete_intent(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipDeleteIntent> {
    Ok(ClipDeleteIntent {
        id: row.get(0)?,
        clip_id: row.get(1)?,
        state: row.get(2)?,
        video_path: row.get(3)?,
        canonical_video_path: row.get(4)?,
        source_dir_path: row.get(5)?,
        canonical_source_dir_path: row.get(6)?,
        extension: row.get(7)?,
        file_existed: row.get::<_, i64>(8)? != 0,
        file_size_bytes: row.get(9)?,
        file_modified_ticks: row.get(10)?,
        file_volume_serial: row.get(11)?,
        file_index_high: row.get(12)?,
        file_index_low: row.get(13)?,
        source_volume_serial: row.get(14)?,
        source_file_index_high: row.get(15)?,
        source_file_index_low: row.get(16)?,
        lease_owner: row.get(17)?,
        last_error_code: row.get(18)?,
        last_error_message: row.get(19)?,
    })
}

fn list_recoverable_delete_intents(connection: &Connection) -> DbResult<Vec<(i64, i64)>> {
    let mut statement = connection
        .prepare(
            "
            SELECT id, clip_id
            FROM clip_delete_intents
            WHERE state = 'pending'
               OR (
                   state = 'processing'
                   AND (lease_expires_at IS NULL OR lease_expires_at <= CURRENT_TIMESTAMP)
               )
            ORDER BY id
            ",
        )
        .map_err(|error| readable_error("preparing permanent-delete recovery", error))?;
    let intents = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| readable_error("querying permanent-delete recovery", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading permanent-delete recovery", error))?;
    Ok(intents)
}

fn clip_exists(connection: &Connection, clip_id: i64) -> DbResult<bool> {
    connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM clips WHERE id = ?1)",
            params![clip_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(|error| readable_error("checking permanent-delete clip existence", error))
}

fn inspect_accessible_directory(path: &Path) -> Result<AccessibleDirectory, ClipDeleteIssue> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        issue(
            "source-unavailable",
            format!("无法验证素材来源目录“{}”：{error}", path.display()),
            true,
        )
    })?;
    if !metadata.is_dir() || metadata_is_reparse_or_symlink(&metadata) {
        return Err(issue(
            "unsafe-path",
            "素材来源目录不是普通目录或包含重解析点，已拒绝永久删除",
            false,
        ));
    }
    let canonical = fs::canonicalize(path).map_err(|error| {
        issue(
            "source-unavailable",
            format!("无法规范化素材来源目录：{error}"),
            true,
        )
    })?;
    fs::read_dir(&canonical).map_err(|error| {
        issue(
            "source-unavailable",
            format!("无法读取素材来源目录：{error}"),
            true,
        )
    })?;
    #[cfg(windows)]
    {
        let (handle, identity) = open_windows_directory(&canonical).map_err(|error| {
            issue(
                fs_error_code(&error),
                format!("无法安全打开素材来源目录句柄：{error}"),
                true,
            )
        })?;
        Ok(AccessibleDirectory {
            canonical_path: canonical,
            identity,
            _handle: handle,
        })
    }

    #[cfg(not(windows))]
    {
        Ok(AccessibleDirectory {
            canonical_path: canonical,
            identity: StableFileIdentity::default(),
        })
    }
}

fn canonicalize_missing_target(
    video_path: &Path,
    source_path: &Path,
    canonical_source: &Path,
) -> Result<PathBuf, ClipDeleteIssue> {
    if !path_is_within(video_path, source_path) {
        return Err(issue(
            "unsafe-path",
            "缺失视频路径不在已索引的来源目录内，已拒绝永久删除",
            false,
        ));
    }

    let mut cursor = video_path.to_path_buf();
    let mut missing_segments = Vec::new();
    let existing_ancestor = loop {
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata_is_reparse_or_symlink(&metadata) {
                    return Err(issue(
                        "unsafe-path",
                        "缺失视频路径的现存祖先不是普通目录或包含重解析点",
                        false,
                    ));
                }
                break cursor;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let segment = cursor.file_name().ok_or_else(|| {
                    issue("unsafe-path", "缺失视频路径没有可验证的现存祖先", false)
                })?;
                missing_segments.push(segment.to_os_string());
                cursor = cursor.parent().map(Path::to_path_buf).ok_or_else(|| {
                    issue("unsafe-path", "缺失视频路径没有可验证的现存祖先", false)
                })?;
            }
            Err(error) => {
                return Err(issue(
                    "source-unavailable",
                    format!(
                        "缺失视频路径的祖先暂不可访问“{}”：{error}",
                        cursor.display()
                    ),
                    true,
                ));
            }
        }
    };

    if !path_is_within(&existing_ancestor, source_path) {
        return Err(issue(
            "unsafe-path",
            "缺失视频路径的现存祖先越过了已索引来源目录",
            false,
        ));
    }
    let mut ancestor_check = existing_ancestor.as_path();
    loop {
        let metadata = fs::symlink_metadata(ancestor_check).map_err(|error| {
            issue(
                "source-unavailable",
                format!("缺失视频路径的祖先暂不可复核：{error}"),
                true,
            )
        })?;
        if !metadata.is_dir() || metadata_is_reparse_or_symlink(&metadata) {
            return Err(issue(
                "unsafe-path",
                "缺失视频路径的现存祖先链包含非目录或重解析点",
                false,
            ));
        }
        if paths_equal(ancestor_check, source_path) {
            break;
        }
        ancestor_check = ancestor_check.parent().ok_or_else(|| {
            issue(
                "unsafe-path",
                "缺失视频路径的祖先链未到达已索引来源目录",
                false,
            )
        })?;
    }

    let accessible_ancestor = inspect_accessible_directory(&existing_ancestor)?;
    let mut canonical_video = accessible_ancestor.canonical_path;
    for segment in missing_segments.iter().rev() {
        canonical_video.push(segment);
    }
    if !path_is_within(&canonical_video, canonical_source) {
        return Err(issue(
            "unsafe-path",
            "缺失视频路径不在已索引的来源目录内，已拒绝永久删除",
            false,
        ));
    }
    let file_name = video_path.file_name().ok_or_else(|| {
        issue(
            "unsafe-path",
            "缺失视频路径没有可验证的文件名，已拒绝永久删除",
            false,
        )
    })?;
    if canonical_video.file_name() != Some(file_name) {
        return Err(issue(
            "unsafe-path",
            "缺失视频路径的受控拼接结果与目标文件名不一致",
            false,
        ));
    }
    Ok(canonical_video)
}

fn target_identity(intent: &ClipDeleteIntent) -> StableFileIdentity {
    StableFileIdentity {
        volume_serial: intent.file_volume_serial,
        file_index_high: intent.file_index_high,
        file_index_low: intent.file_index_low,
    }
}

fn source_identity(intent: &ClipDeleteIntent) -> StableFileIdentity {
    StableFileIdentity {
        volume_serial: intent.source_volume_serial,
        file_index_high: intent.source_file_index_high,
        file_index_low: intent.source_file_index_low,
    }
}

#[cfg(windows)]
fn snapshot_existing_file(
    path: &Path,
    _metadata: &fs::Metadata,
) -> Result<StableFileSnapshot, ClipDeleteIssue> {
    let handle = open_windows_file(path, false).map_err(|error| {
        issue(
            fs_error_code(&error),
            format!("无法安全打开视频句柄并记录文件身份：{error}"),
            true,
        )
    })?;
    windows_file_snapshot(&handle, false)
}

#[cfg(not(windows))]
fn snapshot_existing_file(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<StableFileSnapshot, ClipDeleteIssue> {
    let size_bytes = i64::try_from(metadata.len()).map_err(|_| {
        issue(
            "unsafe-metadata",
            "视频大小超出可安全记录的范围，已拒绝永久删除",
            false,
        )
    })?;
    let modified_ticks = file_modified_ticks(metadata).ok_or_else(|| {
        issue(
            "unsafe-metadata",
            "无法记录视频修改时间，已拒绝永久删除",
            false,
        )
    })?;
    Ok(StableFileSnapshot {
        size_bytes,
        modified_ticks,
        identity: StableFileIdentity::default(),
    })
}

#[cfg(windows)]
fn open_verified_delete_file(
    path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(VerifiedDeleteFile, StableFileSnapshot), ClipDeleteIssue> {
    let handle = open_windows_file(path, true).map_err(|error| {
        issue(
            fs_error_code(&error),
            format!("无法取得待删除视频的独占删除句柄：{error}"),
            true,
        )
    })?;
    let snapshot = windows_file_snapshot(&handle, false)?;
    Ok((
        VerifiedDeleteFile {
            path: path.to_path_buf(),
            handle,
        },
        snapshot,
    ))
}

#[cfg(not(windows))]
fn open_verified_delete_file(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(VerifiedDeleteFile, StableFileSnapshot), ClipDeleteIssue> {
    let snapshot = snapshot_existing_file(path, metadata)?;
    Ok((
        VerifiedDeleteFile {
            path: path.to_path_buf(),
        },
        snapshot,
    ))
}

#[cfg(windows)]
fn delete_verified_file(target: VerifiedDeleteFile) -> std::io::Result<()> {
    use std::{mem::size_of, os::windows::io::AsRawHandle};

    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };

    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let deleted = unsafe {
        SetFileInformationByHandle(
            target.handle.as_raw_handle(),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            u32::try_from(size_of::<FILE_DISPOSITION_INFO>()).expect("structure size fits u32"),
        )
    };
    if deleted == 0 {
        return Err(std::io::Error::last_os_error());
    }
    drop(target);
    Ok(())
}

#[cfg(not(windows))]
fn delete_verified_file(target: VerifiedDeleteFile) -> std::io::Result<()> {
    fs::remove_file(target.path)
}

#[cfg(windows)]
fn open_windows_file(path: &Path, for_delete: bool) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = fs::OpenOptions::new();
    let access = FILE_READ_ATTRIBUTES | if for_delete { DELETE } else { 0 };
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | if for_delete { 0 } else { FILE_SHARE_DELETE };
    options
        .read(true)
        .access_mode(access)
        .share_mode(share)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(windows)]
fn open_windows_directory(path: &Path) -> std::io::Result<(fs::File, StableFileIdentity)> {
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = fs::OpenOptions::new();
    let handle = options
        .read(true)
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let snapshot = windows_file_snapshot(&handle, true)
        .map_err(|problem| std::io::Error::other(problem.message))?;
    Ok((handle, snapshot.identity))
}

#[cfg(windows)]
fn windows_file_snapshot(
    handle: &fs::File,
    expect_directory: bool,
) -> Result<StableFileSnapshot, ClipDeleteIssue> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let succeeded = unsafe {
        GetFileInformationByHandle(
            handle.as_raw_handle(),
            &mut information as *mut BY_HANDLE_FILE_INFORMATION,
        )
    };
    if succeeded == 0 {
        let error = std::io::Error::last_os_error();
        return Err(issue(
            fs_error_code(&error),
            format!("无法从句柄读取文件身份：{error}"),
            true,
        ));
    }
    let is_directory = information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || is_directory != expect_directory
    {
        return Err(issue(
            "unsafe-path",
            "句柄目标的类型或重解析点属性不符合永久删除安全要求",
            false,
        ));
    }

    let size = (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow);
    let modified = (u64::from(information.ftLastWriteTime.dwHighDateTime) << 32)
        | u64::from(information.ftLastWriteTime.dwLowDateTime);
    let size_bytes = i64::try_from(size)
        .map_err(|_| issue("unsafe-metadata", "文件大小超出可安全记录的范围", false))?;
    let modified_ticks = i64::try_from(modified)
        .map_err(|_| issue("unsafe-metadata", "文件修改时间超出可安全记录的范围", false))?;
    Ok(StableFileSnapshot {
        size_bytes,
        modified_ticks,
        identity: StableFileIdentity {
            volume_serial: Some(i64::from(information.dwVolumeSerialNumber)),
            file_index_high: Some(i64::from(information.nFileIndexHigh)),
            file_index_low: Some(i64::from(information.nFileIndexLow)),
        },
    })
}

fn metadata_is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }

    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(not(windows))]
fn file_modified_ticks(metadata: &fs::Metadata) -> Option<i64> {
    let duration = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_nanos()).ok()
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path = comparable_path(path);
    let root = comparable_path(root);
    path == root || path.strip_prefix(&format!("{root}/")).is_some()
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    comparable_path(left) == comparable_path(right)
}

fn comparable_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    #[cfg(windows)]
    {
        normalized.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized.to_string()
    }
}

fn fs_error_code(error: &std::io::Error) -> &'static str {
    if matches!(error.raw_os_error(), Some(32 | 33)) {
        "file-in-use"
    } else {
        match error.kind() {
            ErrorKind::NotFound => "file-not-found",
            ErrorKind::PermissionDenied => "permission-denied",
            _ => "filesystem-error",
        }
    }
}

fn next_delete_worker_id() -> String {
    let sequence = DELETE_WORKER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("delete-{}-{timestamp}-{sequence}", std::process::id())
}

fn issue(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> ClipDeleteIssue {
    ClipDeleteIssue {
        code: code.into(),
        message: message.into(),
        retryable,
    }
}

fn intent_problem(
    intent: &ClipDeleteIntent,
    fallback_code: &str,
    fallback_message: &str,
    retryable: bool,
) -> ClipDeleteIssue {
    issue(
        intent.last_error_code.as_deref().unwrap_or(fallback_code),
        intent
            .last_error_message
            .as_deref()
            .unwrap_or(fallback_message),
        retryable,
    )
}

fn verification_failure_parts(failure: VerificationFailure) -> (&'static str, ClipDeleteIssue) {
    match failure {
        VerificationFailure::Pending(problem) => ("pending", problem),
        VerificationFailure::Blocked(problem) => ("blocked", problem),
    }
}

fn deduplicate_clip_ids(clip_ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::with_capacity(clip_ids.len());
    clip_ids
        .iter()
        .copied()
        .filter(|clip_id| seen.insert(*clip_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use rusqlite::Connection;

    use super::{
        delete_clip_from_index_guarded, delete_clip_permanently, finalize_delete_intent,
        find_delete_intent_by_clip_id, find_trash_snapshot_by_clip_id,
        recover_pending_clip_deletions, set_clips_trashed_guarded, stage_delete_intent,
        verify_delete_target, ClipDeleteItemOutcome, StageDeleteOutcome, VerifiedDeleteTarget,
    };
    use crate::db::{self, ClipInput, SourceDirInput};

    #[test]
    fn committed_intent_survives_interruption_and_startup_recovery_converges() {
        let fixture = DeleteFixture::with_file("recover-me.mp4", b"original video");
        let intent = fixture.stage_intent();

        assert!(fixture.clip_path.is_file());
        assert!(
            find_delete_intent_by_clip_id(&fixture.connection, fixture.clip_id)
                .unwrap()
                .is_some()
        );

        let recovered = recover_pending_clip_deletions(&fixture.connection).unwrap();

        assert_eq!(recovered.attempted, 1);
        assert_eq!(recovered.deleted_ids, vec![fixture.clip_id]);
        assert!(!fixture.clip_path.exists());
        assert!(!fixture.clip_exists());
        assert!(
            find_delete_intent_by_clip_id(&fixture.connection, fixture.clip_id)
                .unwrap()
                .is_none()
        );
        assert!(intent.id > 0);
    }

    #[test]
    fn recovery_finishes_when_crash_happened_after_file_delete_before_finalizer() {
        let fixture = DeleteFixture::with_file("post-delete-crash.mp4", b"original video");
        let intent = fixture.stage_intent();
        let verified = verify_delete_target(&intent).unwrap();
        let VerifiedDeleteTarget::Present(target) = verified else {
            panic!("fixture target should exist");
        };
        super::delete_verified_file(target).unwrap();

        let recovered = recover_pending_clip_deletions(&fixture.connection).unwrap();

        assert_eq!(recovered.deleted_ids, vec![fixture.clip_id]);
        assert!(!fixture.clip_exists());
    }

    #[test]
    fn replacement_is_blocked_and_cancels_the_stale_authorization() {
        let fixture = DeleteFixture::with_file("replace-me.mp4", b"original video");
        let intent = fixture.stage_intent();
        assert!(intent.file_size_bytes.is_some());
        fs::write(&fixture.clip_path, b"replacement with different bytes").unwrap();

        let outcome = delete_clip_permanently(&fixture.connection, fixture.clip_id).unwrap();

        let ClipDeleteItemOutcome::Blocked(problem) = outcome else {
            panic!("replacement should be blocked");
        };
        assert!(problem.message.contains("本次永久删除授权"));
        assert_eq!(
            fs::read(&fixture.clip_path).unwrap(),
            b"replacement with different bytes"
        );
        assert!(
            find_delete_intent_by_clip_id(&fixture.connection, fixture.clip_id)
                .unwrap()
                .is_none()
        );
        let restored =
            set_clips_trashed_guarded(&fixture.connection, &[fixture.clip_id], false).unwrap();
        assert_eq!(restored.clips[0].status, "available");
        assert!(fixture.clip_exists());
    }

    #[test]
    fn replacement_before_permanent_delete_is_not_treated_as_new_authorization() {
        let fixture = DeleteFixture::with_file("replace-before-command.mp4", b"original video");
        let authorized = fixture.trash_snapshot();
        fs::remove_file(&fixture.clip_path).unwrap();
        fs::write(&fixture.clip_path, b"replacement video must survive").unwrap();

        let outcome = delete_clip_permanently(&fixture.connection, fixture.clip_id).unwrap();

        let ClipDeleteItemOutcome::Blocked(problem) = outcome else {
            panic!("a path replacement after trashing must be blocked");
        };
        assert_eq!(problem.code, "target-replaced");
        assert_eq!(
            fs::read(&fixture.clip_path).unwrap(),
            b"replacement video must survive"
        );
        assert_eq!(fixture.trash_snapshot(), authorized);
        assert!(
            find_delete_intent_by_clip_id(&fixture.connection, fixture.clip_id)
                .unwrap()
                .is_none()
        );
        assert!(fixture.clip_exists());
    }

    #[test]
    fn legacy_trashed_clip_without_snapshot_fails_closed_until_restore_and_retrash() {
        let fixture = DeleteFixture::with_file("legacy-trash.mp4", b"legacy video");
        fixture
            .connection
            .execute(
                "DELETE FROM clip_trash_snapshots WHERE clip_id = ?1",
                [fixture.clip_id],
            )
            .unwrap();

        let outcome = delete_clip_permanently(&fixture.connection, fixture.clip_id).unwrap();
        let ClipDeleteItemOutcome::Rejected(problem) = outcome else {
            panic!("legacy trash without a snapshot must fail closed");
        };
        assert_eq!(problem.code, "trash-snapshot-missing");
        assert!(problem.message.contains("先恢复"));
        assert!(fixture.clip_path.is_file());
        assert!(fixture.clip_exists());

        set_clips_trashed_guarded(&fixture.connection, &[fixture.clip_id], false).unwrap();
        assert!(
            find_trash_snapshot_by_clip_id(&fixture.connection, fixture.clip_id)
                .unwrap()
                .is_none()
        );
        set_clips_trashed_guarded(&fixture.connection, &[fixture.clip_id], true).unwrap();
        assert!(
            find_trash_snapshot_by_clip_id(&fixture.connection, fixture.clip_id)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            delete_clip_permanently(&fixture.connection, fixture.clip_id).unwrap(),
            ClipDeleteItemOutcome::Deleted
        );
        assert!(!fixture.clip_path.exists());
    }

    #[test]
    fn scan_upsert_cannot_refresh_the_trash_authorization() {
        let fixture = DeleteFixture::with_file("scan-replacement.mp4", b"original scan video");
        let authorized = fixture.trash_snapshot();
        fs::remove_file(&fixture.clip_path).unwrap();
        let replacement = b"replacement discovered by scan";
        fs::write(&fixture.clip_path, replacement).unwrap();
        let source_dir_id = fixture
            .connection
            .query_row(
                "SELECT source_dir_id FROM clips WHERE id = ?1",
                [fixture.clip_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        let rescanned = db::upsert_clip(
            &fixture.connection,
            ClipInput {
                source_dir_id,
                clip_group_id: None,
                video_path: fixture.clip_path.to_string_lossy().as_ref(),
                file_name: fixture
                    .clip_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref(),
                file_size: i64::try_from(replacement.len()).unwrap(),
                modified_at: None,
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .unwrap();

        assert_eq!(rescanned.status, "trashed");
        assert_eq!(fixture.trash_snapshot(), authorized);
        assert!(matches!(
            delete_clip_permanently(&fixture.connection, fixture.clip_id).unwrap(),
            ClipDeleteItemOutcome::Blocked(_)
        ));
        assert_eq!(fs::read(&fixture.clip_path).unwrap(), replacement);
    }

    #[test]
    fn trash_snapshot_is_database_immutable_and_restore_removes_it() {
        let fixture = DeleteFixture::with_file("immutable-snapshot.mp4", b"original video");

        let error = fixture
            .connection
            .execute(
                "UPDATE clip_trash_snapshots SET file_size_bytes = file_size_bytes + 1 WHERE clip_id = ?1",
                [fixture.clip_id],
            )
            .unwrap_err();
        assert!(error.to_string().contains("immutable"));

        set_clips_trashed_guarded(&fixture.connection, &[fixture.clip_id], false).unwrap();
        assert!(
            find_trash_snapshot_by_clip_id(&fixture.connection, fixture.clip_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn missing_target_is_not_finalized_while_source_directory_is_unavailable() {
        let fixture = DeleteFixture::with_file("offline-source.mp4", b"original video");
        fixture.stage_intent();
        fs::remove_file(&fixture.clip_path).unwrap();
        let offline_root = fixture.root.with_extension("offline");
        fs::rename(&fixture.root, &offline_root).unwrap();

        let recovered = recover_pending_clip_deletions(&fixture.connection).unwrap();

        assert_eq!(recovered.pending_ids, vec![fixture.clip_id]);
        assert!(fixture.clip_exists());
        fs::rename(&offline_root, &fixture.root).unwrap();
    }

    #[test]
    fn missing_target_finalizes_when_nested_parent_was_removed_but_source_is_online() {
        let fixture = DeleteFixture::with_file("match-a/nested/recover.mp4", b"original video");
        fixture.stage_intent();
        fs::remove_dir_all(fixture.root.join("match-a")).unwrap();

        let recovered = recover_pending_clip_deletions(&fixture.connection).unwrap();

        assert_eq!(recovered.deleted_ids, vec![fixture.clip_id]);
        assert!(!fixture.clip_exists());
    }

    #[test]
    fn pending_intent_atomically_blocks_restore_and_index_only_removal() {
        let fixture = DeleteFixture::with_file("guard-me.mp4", b"original video");
        fixture.stage_intent();

        let restore_error =
            set_clips_trashed_guarded(&fixture.connection, &[fixture.clip_id], false).unwrap_err();
        let remove_error =
            delete_clip_from_index_guarded(&fixture.connection, fixture.clip_id).unwrap_err();

        assert!(restore_error.contains("delete-pending"));
        assert!(remove_error.contains("delete-pending"));
        assert!(fixture.clip_exists());
        assert!(fixture.clip_path.is_file());
    }

    #[test]
    fn finalizer_removes_intent_and_clip_in_one_transaction() {
        let fixture = DeleteFixture::with_file("finalize.mp4", b"original video");
        let staged = fixture.stage_intent();
        let claimed =
            match super::claim_delete_intent(&fixture.connection, staged.id, "test-finalizer")
                .unwrap()
            {
                super::ClaimDeleteOutcome::Claimed(intent) => intent,
                _ => panic!("intent should be claimable"),
            };
        fs::remove_file(&fixture.clip_path).unwrap();

        finalize_delete_intent(&fixture.connection, &claimed).unwrap();

        assert!(!fixture.clip_exists());
        assert!(
            find_delete_intent_by_clip_id(&fixture.connection, fixture.clip_id)
                .unwrap()
                .is_none()
        );
    }

    struct DeleteFixture {
        connection: Connection,
        root: PathBuf,
        clip_path: PathBuf,
        clip_id: i64,
    }

    impl DeleteFixture {
        fn with_file(file_name: &str, bytes: &[u8]) -> Self {
            let root = unique_temp_dir();
            fs::create_dir_all(&root).unwrap();
            let clip_path = root.join(file_name);
            fs::create_dir_all(clip_path.parent().unwrap()).unwrap();
            fs::write(&clip_path, bytes).unwrap();
            let connection = Connection::open_in_memory().unwrap();
            db::initialize_schema(&connection).unwrap();
            let source = db::upsert_source_dir(
                &connection,
                SourceDirInput {
                    path: root.to_string_lossy().as_ref(),
                    name: "Delete fixture",
                },
            )
            .unwrap();
            let clip = db::upsert_clip(
                &connection,
                ClipInput {
                    source_dir_id: source.id,
                    clip_group_id: None,
                    video_path: clip_path.to_string_lossy().as_ref(),
                    file_name: clip_path.file_name().unwrap().to_string_lossy().as_ref(),
                    file_size: i64::try_from(bytes.len()).unwrap(),
                    modified_at: None,
                    duration_ms: None,
                    recorded_at: None,
                    cover_path: None,
                    cover_source: "missing",
                },
            )
            .unwrap();
            set_clips_trashed_guarded(&connection, &[clip.id], true).unwrap();
            Self {
                connection,
                root,
                clip_path,
                clip_id: clip.id,
            }
        }

        fn stage_intent(&self) -> super::ClipDeleteIntent {
            match stage_delete_intent(&self.connection, self.clip_id).unwrap() {
                StageDeleteOutcome::Intent(intent) => *intent,
                _ => panic!("fixture intent should stage"),
            }
        }

        fn trash_snapshot(&self) -> super::TrashIdentitySnapshot {
            find_trash_snapshot_by_clip_id(&self.connection, self.clip_id)
                .unwrap()
                .expect("fixture should have a trash identity snapshot")
        }

        fn clip_exists(&self) -> bool {
            self.connection
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM clips WHERE id = ?1)",
                    [self.clip_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                != 0
        }
    }

    impl Drop for DeleteFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
            let _ = fs::remove_dir_all(self.root.with_extension("offline"));
        }
    }

    fn unique_temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "valoframe-delete-test-{}-{}",
            std::process::id(),
            super::next_delete_worker_id()
        ))
    }
}
