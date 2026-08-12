//! Source-local scan candidate staging and conservative clip reconnection.
//!
//! Candidates are staged in SQLite TEMP tables so a source can be enumerated completely before
//! any permanent clip row is rebound. This is important for hard links and duplicate legacy
//! fingerprints: seeing the first path is not proof that either side of a match is unique.

use std::{fs, io, path::Path};

use rusqlite::{named_params, params, Connection, OptionalExtension};

use crate::file_identity::StableFileIdentity;

use super::super::{
    extension_from_file_name, normalize_optional, normalize_path, readable_error,
    require_non_empty, source_relative_directory_for_clip, stable_path_for_storage, ClipInput,
    ClipSaveOutcome, DbResult, SavedClip,
};
use super::{clips::find_clip_by_id, thumbnails};

const CANDIDATE_TABLE: &str = "scan_reconnect_candidates_v2";
const PLAN_TABLE: &str = "scan_reconnect_plan_state_v2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanReconnectCandidate {
    pub(crate) candidate_id: i64,
    pub(crate) source_dir_id: i64,
    pub(crate) file_path: String,
    pub(crate) normalized_path: String,
    pub(crate) file_name: String,
    pub(crate) size_bytes: i64,
    pub(crate) modified_at: Option<String>,
    pub(crate) file_identity: Option<StableFileIdentity>,
    pub(crate) validation_token: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanReconnectCandidateInput<'a> {
    pub(crate) source_dir_id: i64,
    pub(crate) file_path: &'a str,
    pub(crate) normalized_path: &'a str,
    pub(crate) file_name: &'a str,
    pub(crate) size_bytes: i64,
    pub(crate) modified_at: Option<&'a str>,
    pub(crate) file_identity: Option<StableFileIdentity>,
    pub(crate) validation_token: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StageScanReconnectCandidateOutcome {
    Staged(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanReconnectMatchKind {
    StableFileIdentity,
    LegacyFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedScanReconnect {
    pub(crate) candidate: ScanReconnectCandidate,
    pub(crate) clip_id: i64,
    pub(crate) old_file_path: String,
    pub(crate) old_normalized_path: String,
    pub(crate) match_kind: ScanReconnectMatchKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanReconnectWarningKind {
    ForeignPathOwner,
    NormalizedPathConflict,
    StableIdentityConflict,
    LegacyFingerprintConflict,
    ProtectedClip,
    OldPathUnverifiable,
}

impl ScanReconnectWarningKind {
    /// Only an I/O error checking the old path makes missing reconciliation unsafe. Candidate or
    /// database ambiguity is still a complete enumeration: the old row must converge to missing.
    pub(crate) const fn blocks_missing_reconciliation(self) -> bool {
        matches!(
            self,
            Self::ForeignPathOwner | Self::NormalizedPathConflict | Self::OldPathUnverifiable
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanReconnectWarning {
    pub(crate) kind: ScanReconnectWarningKind,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScanReconnectDecision {
    ExistingPath {
        candidate: ScanReconnectCandidate,
        clip_id: i64,
    },
    Reconnect(PlannedScanReconnect),
    New(ScanReconnectCandidate),
    NewWithWarning {
        candidate: ScanReconnectCandidate,
        warning: ScanReconnectWarning,
    },
}

impl ScanReconnectDecision {
    pub(crate) fn candidate(&self) -> &ScanReconnectCandidate {
        match self {
            Self::ExistingPath { candidate, .. }
            | Self::New(candidate)
            | Self::NewWithWarning { candidate, .. } => candidate,
            Self::Reconnect(planned) => &planned.candidate,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ScanReconnectPlanStats {
    pub(crate) candidate_count: i64,
    pub(crate) existing_path_count: i64,
    pub(crate) stable_identity_match_count: i64,
    pub(crate) legacy_fingerprint_match_count: i64,
    pub(crate) new_count: i64,
    pub(crate) conflict_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApplyScanReconnectOutcome {
    Reconnected(Box<SavedClip>),
    OldPathPresent,
    OldPathUnverifiable(String),
    StalePlan,
}

/// Starts a fresh connection-local plan for one source.
pub(crate) fn begin_scan_reconnect_plan(
    connection: &Connection,
    source_dir_id: i64,
) -> DbResult<()> {
    if source_dir_id <= 0 {
        return Err("scan reconnect source id must be positive".to_string());
    }
    ensure_temp_schema(connection)?;
    connection
        .execute(&format!("DELETE FROM temp.{CANDIDATE_TABLE}"), [])
        .map_err(|error| readable_error("clearing scan reconnect candidates", error))?;
    connection
        .execute(&format!("DELETE FROM temp.{PLAN_TABLE}"), [])
        .map_err(|error| readable_error("clearing scan reconnect plan state", error))?;
    connection
        .execute(
            &format!("INSERT INTO temp.{PLAN_TABLE} (source_dir_id, finalized) VALUES (?1, 0)"),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("starting scan reconnect plan", error))?;
    Ok(())
}

pub(crate) fn stage_scan_reconnect_candidate(
    connection: &Connection,
    input: ScanReconnectCandidateInput<'_>,
) -> DbResult<StageScanReconnectCandidateOutcome> {
    validate_candidate_input(&input)?;
    require_open_plan(connection, input.source_dir_id)?;
    let file_path = stable_path_for_storage(input.file_path);
    let normalized_path = normalize_path(&file_path);
    let (volume_serial, index_high, index_low) = identity_database_parts(input.file_identity);
    connection
        .execute(
            &format!(
                "
                INSERT INTO temp.{CANDIDATE_TABLE} (
                    source_dir_id,
                    file_path,
                    normalized_path,
                    file_name,
                    normalized_file_name,
                    size_bytes,
                    modified_at,
                    file_volume_serial,
                    file_index_high,
                    file_index_low,
                    validation_token
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                "
            ),
            params![
                input.source_dir_id,
                file_path,
                normalized_path,
                input.file_name,
                normalized_file_name(input.file_name),
                input.size_bytes,
                input.modified_at,
                volume_serial,
                index_high,
                index_low,
                input.validation_token,
            ],
        )
        .map_err(|error| readable_error("staging scan reconnect candidate", error))?;
    Ok(StageScanReconnectCandidateOutcome::Staged(
        connection.last_insert_rowid(),
    ))
}

/// Materializes every decision before callers are allowed to write permanent clip rows.
pub(crate) fn finalize_scan_reconnect_plan(
    connection: &Connection,
    source_dir_id: i64,
) -> DbResult<ScanReconnectPlanStats> {
    require_open_plan(connection, source_dir_id)?;

    // Windows can expose distinct physical files whose normalized paths are identical when a
    // directory has case sensitivity enabled. Keep every candidate in the plan so this ambiguity
    // cannot disappear behind an INSERT conflict and accidentally reconnect user state.
    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'normalized_path_conflict'
                WHERE source_dir_id = ?1
                  AND 1 < (
                      SELECT COUNT(*)
                      FROM temp.{CANDIDATE_TABLE} AS peer
                      WHERE peer.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                        AND peer.normalized_path = temp.{CANDIDATE_TABLE}.normalized_path
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("materializing normalized path conflicts", error))?;

    // Existing normalized paths always win. A global path owned by another source cannot be
    // inserted because clips.normalized_path is unique, so preserve the previous fail-closed rule.
    connection
        .execute_batch(&format!(
            "
            UPDATE temp.{CANDIDATE_TABLE}
            SET decision = CASE
                    WHEN (
                        SELECT clips.source_dir_id
                        FROM clips
                        WHERE clips.normalized_path = temp.{CANDIDATE_TABLE}.normalized_path
                    ) = temp.{CANDIDATE_TABLE}.source_dir_id
                    THEN 'existing'
                    ELSE 'foreign_path'
                END,
                matched_clip_id = (
                    SELECT clips.id
                    FROM clips
                    WHERE clips.normalized_path = temp.{CANDIDATE_TABLE}.normalized_path
                )
            WHERE source_dir_id = {source_dir_id}
              AND decision = 'unplanned'
              AND EXISTS (
                  SELECT 1
                  FROM clips
                  WHERE clips.normalized_path = temp.{CANDIDATE_TABLE}.normalized_path
              );
            "
        ))
        .map_err(|error| readable_error("matching existing scan paths", error))?;

    materialize_stable_identity_decisions(connection, source_dir_id)?;
    materialize_legacy_fingerprint_decisions(connection, source_dir_id)?;

    connection
        .execute(
            &format!(
                "UPDATE temp.{CANDIDATE_TABLE} SET decision = 'new' \
                 WHERE source_dir_id = ?1 AND decision = 'unplanned'"
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("finalizing new scan candidates", error))?;
    connection
        .execute(
            &format!("UPDATE temp.{PLAN_TABLE} SET finalized = 1 WHERE source_dir_id = ?1"),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("finalizing scan reconnect plan state", error))?;

    scan_reconnect_plan_stats(connection, source_dir_id)
}

pub(crate) fn list_staged_scan_reconnect_candidates(
    connection: &Connection,
    source_dir_id: i64,
    after_candidate_id: i64,
    limit: i64,
) -> DbResult<Vec<ScanReconnectCandidate>> {
    require_finalized_plan(connection, source_dir_id)?;
    if after_candidate_id < 0 {
        return Err("scan reconnect cursor must not be negative".to_string());
    }
    if !(1..=1_000).contains(&limit) {
        return Err("scan reconnect page limit must be between 1 and 1000".to_string());
    }
    let mut statement = connection
        .prepare(&format!(
            "
            SELECT
                candidate_id,
                source_dir_id,
                file_path,
                normalized_path,
                file_name,
                size_bytes,
                modified_at,
                file_volume_serial,
                file_index_high,
                file_index_low,
                validation_token
            FROM temp.{CANDIDATE_TABLE}
            WHERE source_dir_id = ?1
              AND candidate_id > ?2
            ORDER BY candidate_id
            LIMIT ?3
            "
        ))
        .map_err(|error| readable_error("preparing staged scan candidate page", error))?;
    let candidates = statement
        .query_map(params![source_dir_id, after_candidate_id, limit], |row| {
            map_candidate(row)
        })
        .map_err(|error| readable_error("querying staged scan candidate page", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading staged scan candidate page", error))?;
    Ok(candidates)
}

/// Resolves a materialized database decision and performs the final filesystem absence check.
pub(crate) fn resolve_scan_reconnect_candidate(
    connection: &Connection,
    source_dir_id: i64,
    candidate_id: i64,
) -> DbResult<ScanReconnectDecision> {
    require_finalized_plan(connection, source_dir_id)?;
    let planned = connection
        .query_row(
            &format!(
                "
                SELECT
                    candidate_id,
                    source_dir_id,
                    file_path,
                    normalized_path,
                    file_name,
                    size_bytes,
                    modified_at,
                    file_volume_serial,
                    file_index_high,
                    file_index_low,
                    validation_token,
                    decision,
                    matched_clip_id,
                    matched_old_path,
                    matched_old_normalized_path,
                    match_kind
                FROM temp.{CANDIDATE_TABLE}
                WHERE source_dir_id = ?1
                  AND candidate_id = ?2
                "
            ),
            params![source_dir_id, candidate_id],
            |row| {
                Ok(MaterializedCandidateDecision {
                    candidate: map_candidate(row)?,
                    decision: row.get(11)?,
                    matched_clip_id: row.get(12)?,
                    matched_old_path: row.get(13)?,
                    matched_old_normalized_path: row.get(14)?,
                    match_kind: row.get(15)?,
                })
            },
        )
        .optional()
        .map_err(|error| readable_error("reading scan reconnect decision", error))?
        .ok_or_else(|| format!("scan reconnect candidate {candidate_id} was not staged"))?;

    decision_from_materialized(planned)
}

/// Rebinds one materialized match while preserving the clip id and every user-owned relation.
///
/// A savepoint makes the clip path change and thumbnail invalidation atomic even when the caller
/// is already inside a larger scan batch transaction. Every matching predicate is repeated here;
/// a TEMP plan is an optimization, not authorization to overwrite a row that changed meanwhile.
pub(crate) fn apply_planned_scan_reconnect(
    connection: &Connection,
    planned: &PlannedScanReconnect,
    input: ClipInput<'_>,
    file_identity: Option<StableFileIdentity>,
) -> DbResult<ApplyScanReconnectOutcome> {
    validate_reconnect_apply_input(planned, &input, file_identity)?;
    match classify_old_path(&planned.old_file_path) {
        OldPathState::Present => return Ok(ApplyScanReconnectOutcome::OldPathPresent),
        OldPathState::Unverifiable(error) => {
            return Ok(ApplyScanReconnectOutcome::OldPathUnverifiable(error))
        }
        OldPathState::Missing => {}
    }

    connection
        .execute_batch("SAVEPOINT apply_scan_reconnect")
        .map_err(|error| readable_error("starting scan reconnect savepoint", error))?;
    let result =
        apply_planned_scan_reconnect_in_savepoint(connection, planned, input, file_identity);
    match result {
        Ok(outcome) => {
            connection
                .execute_batch("RELEASE SAVEPOINT apply_scan_reconnect")
                .map_err(|error| readable_error("committing scan reconnect savepoint", error))?;
            Ok(outcome)
        }
        Err(error) => {
            let rollback = connection.execute_batch(
                "ROLLBACK TO SAVEPOINT apply_scan_reconnect; \
                 RELEASE SAVEPOINT apply_scan_reconnect;",
            );
            match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!(
                    "{error}; additionally failed to roll back scan reconnect: {rollback_error}"
                )),
            }
        }
    }
}

fn apply_planned_scan_reconnect_in_savepoint(
    connection: &Connection,
    planned: &PlannedScanReconnect,
    input: ClipInput<'_>,
    file_identity: Option<StableFileIdentity>,
) -> DbResult<ApplyScanReconnectOutcome> {
    let video_path =
        stable_path_for_storage(require_non_empty(input.video_path, "clip video path")?);
    let file_name = require_non_empty(input.file_name, "clip file name")?;
    let cover_source = require_non_empty(input.cover_source, "cover source")?;
    let normalized_path = normalize_path(&video_path);
    let source_relative_dir = source_relative_directory_for_clip(connection, &input, &video_path)?;
    let extension = extension_from_file_name(file_name);
    let cover_path = normalize_optional(input.cover_path).map(stable_path_for_storage);
    let (volume_serial, index_high, index_low) = identity_database_parts(file_identity);

    let match_predicate = match planned.match_kind {
        ScanReconnectMatchKind::StableFileIdentity => {
            "clips.file_volume_serial = :volume_serial
             AND clips.file_index_high = :index_high
             AND clips.file_index_low = :index_low"
        }
        ScanReconnectMatchKind::LegacyFingerprint => {
            "clips.file_volume_serial IS NULL
             AND clips.file_index_high IS NULL
             AND clips.file_index_low IS NULL
             AND LOWER(clips.file_name) = :legacy_file_name
             AND clips.size_bytes = :size_bytes
             AND clips.modified_at IS :modified_at"
        }
    };
    let sql = format!(
        "
        UPDATE clips
        SET clip_group_id = :clip_group_id,
            file_path = :file_path,
            normalized_path = :normalized_path,
            file_name = :file_name,
            extension = :extension,
            size_bytes = :size_bytes,
            modified_at = :modified_at,
            file_volume_serial = :volume_serial,
            file_index_high = :index_high,
            file_index_low = :index_low,
            duration_ms = CASE
                WHEN :duration_ms IS NULL
                    AND EXISTS (
                        SELECT 1 FROM clip_metadata
                        WHERE clip_metadata.clip_id = clips.id
                          AND clip_metadata.metadata_source = 'wonderful_db'
                    )
                THEN clips.duration_ms
                ELSE :duration_ms
            END,
            recorded_at = CASE
                WHEN :recorded_at IS NULL
                    AND EXISTS (
                        SELECT 1 FROM clip_metadata
                        WHERE clip_metadata.clip_id = clips.id
                          AND clip_metadata.metadata_source = 'wonderful_db'
                    )
                THEN clips.recorded_at
                ELSE :recorded_at
            END,
            source_relative_dir = :source_relative_dir,
            cover_path = :cover_path,
            cover_source = :cover_source,
            file_status = 'available',
            last_seen_at = CURRENT_TIMESTAMP,
            updated_at = CURRENT_TIMESTAMP
        WHERE clips.id = :clip_id
          AND clips.source_dir_id = :source_dir_id
          AND clips.normalized_path = :old_normalized_path
          AND clips.file_status <> 'trashed'
          AND NOT EXISTS (
              SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = clips.id
          )
          AND NOT EXISTS (
              SELECT 1
              FROM clips AS target
              WHERE target.normalized_path = :normalized_path
                AND target.id <> clips.id
          )
          AND LENGTH(:legacy_file_name) > 0
          AND {match_predicate}
        "
    );
    let changed = connection
        .execute(
            &sql,
            named_params! {
                ":clip_group_id": input.clip_group_id,
                ":file_path": video_path,
                ":normalized_path": normalized_path,
                ":file_name": file_name,
                ":extension": extension,
                ":size_bytes": input.file_size,
                ":modified_at": input.modified_at,
                ":volume_serial": volume_serial,
                ":index_high": index_high,
                ":index_low": index_low,
                ":duration_ms": input.duration_ms,
                ":recorded_at": input.recorded_at,
                ":source_relative_dir": source_relative_dir,
                ":cover_path": cover_path,
                ":cover_source": cover_source,
                ":clip_id": planned.clip_id,
                ":source_dir_id": input.source_dir_id,
                ":old_normalized_path": planned.old_normalized_path,
                ":legacy_file_name": normalized_file_name(file_name),
            },
        )
        .map_err(|error| readable_error("reconnecting scanned clip", error))?;
    if changed != 1 {
        return Ok(ApplyScanReconnectOutcome::StalePlan);
    }

    let fingerprint =
        thumbnails::thumbnail_fingerprint(&normalized_path, input.file_size, input.modified_at);
    let has_source_cover = cover_source == "file" && cover_path.is_some();
    thumbnails::reset_thumbnail_state(
        connection,
        planned.clip_id,
        &fingerprint,
        if has_source_cover {
            "suppressed"
        } else {
            "pending"
        },
    )?;
    let clip = find_clip_by_id(connection, planned.clip_id)?;
    Ok(ApplyScanReconnectOutcome::Reconnected(Box::new(
        SavedClip {
            clip,
            outcome: ClipSaveOutcome::Updated,
        },
    )))
}

fn validate_reconnect_apply_input(
    planned: &PlannedScanReconnect,
    input: &ClipInput<'_>,
    file_identity: Option<StableFileIdentity>,
) -> DbResult<()> {
    if input.source_dir_id != planned.candidate.source_dir_id {
        return Err("scan reconnect candidate source changed after planning".to_string());
    }
    if normalize_path(input.video_path) != planned.candidate.normalized_path
        || input.file_name != planned.candidate.file_name
        || input.file_size != planned.candidate.size_bytes
        || input.modified_at != planned.candidate.modified_at.as_deref()
    {
        return Err("scan reconnect candidate changed after planning".to_string());
    }
    if file_identity != planned.candidate.file_identity {
        return Err("file identity changed after reconnect planning".to_string());
    }
    Ok(())
}

pub(crate) fn clear_scan_reconnect_plan(connection: &Connection) -> DbResult<()> {
    ensure_temp_schema(connection)?;
    connection
        .execute(&format!("DELETE FROM temp.{CANDIDATE_TABLE}"), [])
        .map_err(|error| readable_error("clearing scan reconnect candidates", error))?;
    connection
        .execute(&format!("DELETE FROM temp.{PLAN_TABLE}"), [])
        .map_err(|error| readable_error("clearing scan reconnect plan", error))?;
    Ok(())
}

fn ensure_temp_schema(connection: &Connection) -> DbResult<()> {
    connection
        .execute_batch(&format!(
            "
            CREATE TEMP TABLE IF NOT EXISTS {PLAN_TABLE} (
                source_dir_id INTEGER PRIMARY KEY,
                finalized INTEGER NOT NULL DEFAULT 0 CHECK (finalized IN (0, 1))
            );

            CREATE TEMP TABLE IF NOT EXISTS {CANDIDATE_TABLE} (
                candidate_id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_dir_id INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                normalized_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                normalized_file_name TEXT NOT NULL,
                size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
                modified_at TEXT,
                file_volume_serial INTEGER,
                file_index_high INTEGER,
                file_index_low INTEGER,
                validation_token TEXT NOT NULL,
                decision TEXT NOT NULL DEFAULT 'unplanned' CHECK (
                    decision IN (
                        'unplanned',
                        'existing',
                        'foreign_path',
                        'normalized_path_conflict',
                        'stable_candidate_conflict',
                        'stable_old_conflict',
                        'stable_protected',
                        'stable_match',
                        'legacy_candidate_conflict',
                        'legacy_old_conflict',
                        'legacy_protected',
                        'legacy_match',
                        'new'
                    )
                ),
                matched_clip_id INTEGER,
                matched_old_path TEXT,
                matched_old_normalized_path TEXT,
                match_kind TEXT CHECK (match_kind IS NULL OR match_kind IN ('stable', 'legacy')),
                CHECK (
                    (file_volume_serial IS NULL
                        AND file_index_high IS NULL
                        AND file_index_low IS NULL)
                    OR
                    (file_volume_serial IS NOT NULL
                        AND file_index_high IS NOT NULL
                        AND file_index_low IS NOT NULL)
                )
            );

            CREATE INDEX IF NOT EXISTS temp.idx_scan_reconnect_candidate_normalized_path_v2
                ON {CANDIDATE_TABLE}(source_dir_id, normalized_path);
            CREATE INDEX IF NOT EXISTS temp.idx_scan_reconnect_candidate_identity_v2
                ON {CANDIDATE_TABLE}(
                    source_dir_id,
                    file_volume_serial,
                    file_index_high,
                    file_index_low
                );
            CREATE INDEX IF NOT EXISTS temp.idx_scan_reconnect_candidate_fingerprint_v2
                ON {CANDIDATE_TABLE}(
                    source_dir_id,
                    normalized_file_name,
                    size_bytes,
                    modified_at
                );
            "
        ))
        .map_err(|error| readable_error("initializing scan reconnect temp schema", error))
}

fn materialize_stable_identity_decisions(
    connection: &Connection,
    source_dir_id: i64,
) -> DbResult<()> {
    // Candidate-side ambiguity (usually hard links) blocks both stable and legacy matching.
    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'stable_candidate_conflict'
                WHERE source_dir_id = ?1
                  AND decision = 'unplanned'
                  AND file_volume_serial IS NOT NULL
                  AND 1 < (
                      SELECT COUNT(*)
                      FROM temp.{CANDIDATE_TABLE} AS peer
                      WHERE peer.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                        AND peer.file_volume_serial = temp.{CANDIDATE_TABLE}.file_volume_serial
                        AND peer.file_index_high = temp.{CANDIDATE_TABLE}.file_index_high
                        AND peer.file_index_low = temp.{CANDIDATE_TABLE}.file_index_low
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("materializing candidate identity conflicts", error))?;

    // Every stable row in the source participates in uniqueness. Trashed/intent rows are counted
    // so an authorized deletion target can never be bypassed by falling back to a legacy match.
    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'stable_old_conflict'
                WHERE source_dir_id = ?1
                  AND decision = 'unplanned'
                  AND file_volume_serial IS NOT NULL
                  AND 1 < (
                      SELECT COUNT(*)
                      FROM clips AS old
                      WHERE old.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                        AND old.file_volume_serial = temp.{CANDIDATE_TABLE}.file_volume_serial
                        AND old.file_index_high = temp.{CANDIDATE_TABLE}.file_index_high
                        AND old.file_index_low = temp.{CANDIDATE_TABLE}.file_index_low
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("materializing stored identity conflicts", error))?;

    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'stable_protected'
                WHERE source_dir_id = ?1
                  AND decision = 'unplanned'
                  AND file_volume_serial IS NOT NULL
                  AND 1 = (
                      SELECT COUNT(*)
                      FROM clips AS old
                      WHERE old.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                        AND old.file_volume_serial = temp.{CANDIDATE_TABLE}.file_volume_serial
                        AND old.file_index_high = temp.{CANDIDATE_TABLE}.file_index_high
                        AND old.file_index_low = temp.{CANDIDATE_TABLE}.file_index_low
                  )
                  AND NOT EXISTS (
                      SELECT 1
                      FROM clips AS old
                      WHERE old.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                        AND old.file_volume_serial = temp.{CANDIDATE_TABLE}.file_volume_serial
                        AND old.file_index_high = temp.{CANDIDATE_TABLE}.file_index_high
                        AND old.file_index_low = temp.{CANDIDATE_TABLE}.file_index_low
                        AND old.file_status <> 'trashed'
                        AND NOT EXISTS (
                            SELECT 1
                            FROM clip_delete_intents AS intent
                            WHERE intent.clip_id = old.id
                        )
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("protecting authorized identity rows", error))?;

    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'stable_match',
                    matched_clip_id = (
                        SELECT old.id
                        FROM clips AS old
                        WHERE old.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                          AND old.file_volume_serial = temp.{CANDIDATE_TABLE}.file_volume_serial
                          AND old.file_index_high = temp.{CANDIDATE_TABLE}.file_index_high
                          AND old.file_index_low = temp.{CANDIDATE_TABLE}.file_index_low
                          AND old.file_status <> 'trashed'
                          AND NOT EXISTS (
                              SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = old.id
                          )
                    ),
                    matched_old_path = (
                        SELECT old.file_path
                        FROM clips AS old
                        WHERE old.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                          AND old.file_volume_serial = temp.{CANDIDATE_TABLE}.file_volume_serial
                          AND old.file_index_high = temp.{CANDIDATE_TABLE}.file_index_high
                          AND old.file_index_low = temp.{CANDIDATE_TABLE}.file_index_low
                          AND old.file_status <> 'trashed'
                          AND NOT EXISTS (
                              SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = old.id
                          )
                    ),
                    matched_old_normalized_path = (
                        SELECT old.normalized_path
                        FROM clips AS old
                        WHERE old.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                          AND old.file_volume_serial = temp.{CANDIDATE_TABLE}.file_volume_serial
                          AND old.file_index_high = temp.{CANDIDATE_TABLE}.file_index_high
                          AND old.file_index_low = temp.{CANDIDATE_TABLE}.file_index_low
                          AND old.file_status <> 'trashed'
                          AND NOT EXISTS (
                              SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = old.id
                          )
                    ),
                    match_kind = 'stable'
                WHERE source_dir_id = ?1
                  AND decision = 'unplanned'
                  AND file_volume_serial IS NOT NULL
                  AND 1 = (
                      SELECT COUNT(*)
                      FROM clips AS old
                      WHERE old.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
                        AND old.file_volume_serial = temp.{CANDIDATE_TABLE}.file_volume_serial
                        AND old.file_index_high = temp.{CANDIDATE_TABLE}.file_index_high
                        AND old.file_index_low = temp.{CANDIDATE_TABLE}.file_index_low
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("materializing stable identity matches", error))?;
    Ok(())
}

fn materialize_legacy_fingerprint_decisions(
    connection: &Connection,
    source_dir_id: i64,
) -> DbResult<()> {
    let candidate_fingerprint_predicate = format!(
        "peer.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
         AND peer.normalized_file_name = temp.{CANDIDATE_TABLE}.normalized_file_name
         AND peer.size_bytes = temp.{CANDIDATE_TABLE}.size_bytes
         AND peer.modified_at = temp.{CANDIDATE_TABLE}.modified_at"
    );
    let old_fingerprint_predicate = format!(
        "old.source_dir_id = temp.{CANDIDATE_TABLE}.source_dir_id
         AND LOWER(old.file_name) = temp.{CANDIDATE_TABLE}.normalized_file_name
         AND old.size_bytes = temp.{CANDIDATE_TABLE}.size_bytes
         AND old.modified_at = temp.{CANDIDATE_TABLE}.modified_at"
    );
    let old_legacy_target_predicate = format!(
        "{old_fingerprint_predicate}
         AND old.file_volume_serial IS NULL
         AND old.file_index_high IS NULL
         AND old.file_index_low IS NULL"
    );

    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'legacy_candidate_conflict'
                WHERE source_dir_id = ?1
                  AND decision = 'unplanned'
                  AND modified_at IS NOT NULL
                  AND 1 < (
                      SELECT COUNT(*)
                      FROM temp.{CANDIDATE_TABLE} AS peer
                      WHERE {candidate_fingerprint_predicate}
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("materializing candidate fingerprint conflicts", error))?;

    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'legacy_old_conflict'
                WHERE source_dir_id = ?1
                  AND decision = 'unplanned'
                  AND modified_at IS NOT NULL
                  AND 1 < (
                      SELECT COUNT(*)
                      FROM clips AS old
                      WHERE {old_fingerprint_predicate}
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("materializing stored fingerprint conflicts", error))?;

    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'legacy_protected'
                WHERE source_dir_id = ?1
                  AND decision = 'unplanned'
                  AND modified_at IS NOT NULL
                  AND 1 = (
                      SELECT COUNT(*)
                      FROM clips AS old
                      WHERE {old_fingerprint_predicate}
                  )
                  AND EXISTS (
                      SELECT 1
                      FROM clips AS old
                      WHERE {old_legacy_target_predicate}
                  )
                  AND NOT EXISTS (
                      SELECT 1
                      FROM clips AS old
                      WHERE {old_legacy_target_predicate}
                        AND old.file_status <> 'trashed'
                        AND NOT EXISTS (
                            SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = old.id
                        )
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("protecting authorized fingerprint rows", error))?;

    connection
        .execute(
            &format!(
                "
                UPDATE temp.{CANDIDATE_TABLE}
                SET decision = 'legacy_match',
                    matched_clip_id = (
                        SELECT old.id
                        FROM clips AS old
                        WHERE {old_legacy_target_predicate}
                          AND old.file_status <> 'trashed'
                          AND NOT EXISTS (
                              SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = old.id
                          )
                    ),
                    matched_old_path = (
                        SELECT old.file_path
                        FROM clips AS old
                        WHERE {old_legacy_target_predicate}
                          AND old.file_status <> 'trashed'
                          AND NOT EXISTS (
                              SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = old.id
                          )
                    ),
                    matched_old_normalized_path = (
                        SELECT old.normalized_path
                        FROM clips AS old
                        WHERE {old_legacy_target_predicate}
                          AND old.file_status <> 'trashed'
                          AND NOT EXISTS (
                              SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = old.id
                          )
                    ),
                    match_kind = 'legacy'
                WHERE source_dir_id = ?1
                  AND decision = 'unplanned'
                  AND modified_at IS NOT NULL
                  AND 1 = (
                      SELECT COUNT(*)
                      FROM clips AS old
                      WHERE {old_fingerprint_predicate}
                  )
                  AND EXISTS (
                      SELECT 1
                      FROM clips AS old
                      WHERE {old_legacy_target_predicate}
                        AND old.file_status <> 'trashed'
                        AND NOT EXISTS (
                            SELECT 1 FROM clip_delete_intents AS intent WHERE intent.clip_id = old.id
                        )
                  )
                "
            ),
            params![source_dir_id],
        )
        .map_err(|error| readable_error("materializing legacy fingerprint matches", error))?;
    Ok(())
}

fn scan_reconnect_plan_stats(
    connection: &Connection,
    source_dir_id: i64,
) -> DbResult<ScanReconnectPlanStats> {
    connection
        .query_row(
            &format!(
                "
                SELECT
                    COUNT(*),
                    SUM(CASE WHEN decision = 'existing' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN decision = 'stable_match' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN decision = 'legacy_match' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN decision = 'new' THEN 1 ELSE 0 END),
                    SUM(CASE
                        WHEN decision IN (
                            'foreign_path',
                            'normalized_path_conflict',
                            'stable_candidate_conflict',
                            'stable_old_conflict',
                            'stable_protected',
                            'legacy_candidate_conflict',
                            'legacy_old_conflict',
                            'legacy_protected'
                        ) THEN 1 ELSE 0 END)
                FROM temp.{CANDIDATE_TABLE}
                WHERE source_dir_id = ?1
                "
            ),
            params![source_dir_id],
            |row| {
                Ok(ScanReconnectPlanStats {
                    candidate_count: row.get(0)?,
                    existing_path_count: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    stable_identity_match_count: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    legacy_fingerprint_match_count: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    new_count: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    conflict_count: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                })
            },
        )
        .map_err(|error| readable_error("reading scan reconnect plan statistics", error))
}

#[derive(Debug)]
struct MaterializedCandidateDecision {
    candidate: ScanReconnectCandidate,
    decision: String,
    matched_clip_id: Option<i64>,
    matched_old_path: Option<String>,
    matched_old_normalized_path: Option<String>,
    match_kind: Option<String>,
}

fn decision_from_materialized(
    planned: MaterializedCandidateDecision,
) -> DbResult<ScanReconnectDecision> {
    let candidate = planned.candidate;
    match planned.decision.as_str() {
        "existing" => Ok(ScanReconnectDecision::ExistingPath {
            clip_id: required_planned_value(planned.matched_clip_id, "existing clip id")?,
            candidate,
        }),
        "new" => Ok(ScanReconnectDecision::New(candidate)),
        "foreign_path" => Ok(new_with_warning(
            candidate,
            ScanReconnectWarningKind::ForeignPathOwner,
            "candidate path is already owned by source from another registration",
        )),
        "normalized_path_conflict" => Ok(new_with_warning(
            candidate,
            ScanReconnectWarningKind::NormalizedPathConflict,
            "multiple physical files share the same normalized path; skipped without reconnecting",
        )),
        "stable_candidate_conflict" | "stable_old_conflict" => Ok(new_with_warning(
            candidate,
            ScanReconnectWarningKind::StableIdentityConflict,
            "stable file identity is not unique on both sides; indexed without reconnecting",
        )),
        "legacy_candidate_conflict" | "legacy_old_conflict" => Ok(new_with_warning(
            candidate,
            ScanReconnectWarningKind::LegacyFingerprintConflict,
            "legacy filename/size/modified-time fingerprint is not unique on both sides; indexed without reconnecting",
        )),
        "stable_protected" | "legacy_protected" => Ok(new_with_warning(
            candidate,
            ScanReconnectWarningKind::ProtectedClip,
            "matching clip is trashed or has a permanent-delete intent; indexed without reconnecting",
        )),
        "stable_match" | "legacy_match" => {
            let match_kind = match planned.match_kind.as_deref() {
                Some("stable") => ScanReconnectMatchKind::StableFileIdentity,
                Some("legacy") => ScanReconnectMatchKind::LegacyFingerprint,
                _ => return Err("scan reconnect plan has an invalid match kind".to_string()),
            };
            let reconnect = PlannedScanReconnect {
                candidate,
                clip_id: required_planned_value(planned.matched_clip_id, "matched clip id")?,
                old_file_path: required_planned_value(
                    planned.matched_old_path,
                    "matched old file path",
                )?,
                old_normalized_path: required_planned_value(
                    planned.matched_old_normalized_path,
                    "matched old normalized path",
                )?,
                match_kind,
            };
            match classify_old_path(&reconnect.old_file_path) {
                OldPathState::Missing => Ok(ScanReconnectDecision::Reconnect(reconnect)),
                OldPathState::Present => Ok(ScanReconnectDecision::New(reconnect.candidate)),
                OldPathState::Unverifiable(error) => Ok(new_with_warning(
                    reconnect.candidate,
                    ScanReconnectWarningKind::OldPathUnverifiable,
                    &format!("old clip path could not be verified as missing: {error}"),
                )),
            }
        }
        "unplanned" => Err("scan reconnect candidate was read before plan finalization".to_string()),
        value => Err(format!("unsupported scan reconnect decision: {value}")),
    }
}

fn new_with_warning(
    candidate: ScanReconnectCandidate,
    kind: ScanReconnectWarningKind,
    detail: &str,
) -> ScanReconnectDecision {
    let message = format!("{}: {detail}", candidate.file_path);
    ScanReconnectDecision::NewWithWarning {
        candidate,
        warning: ScanReconnectWarning { kind, message },
    }
}

#[derive(Debug, PartialEq, Eq)]
enum OldPathState {
    Missing,
    Present,
    Unverifiable(String),
}

fn classify_old_path(path: &str) -> OldPathState {
    classify_old_path_result(fs::symlink_metadata(Path::new(path)))
}

fn classify_old_path_result(result: io::Result<fs::Metadata>) -> OldPathState {
    match result {
        Ok(_) => OldPathState::Present,
        Err(error) if error.kind() == io::ErrorKind::NotFound => OldPathState::Missing,
        Err(error) => OldPathState::Unverifiable(error.to_string()),
    }
}

fn map_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanReconnectCandidate> {
    let volume_serial = row.get(7)?;
    let index_high = row.get(8)?;
    let index_low = row.get(9)?;
    Ok(ScanReconnectCandidate {
        candidate_id: row.get(0)?,
        source_dir_id: row.get(1)?,
        file_path: row.get(2)?,
        normalized_path: row.get(3)?,
        file_name: row.get(4)?,
        size_bytes: row.get(5)?,
        modified_at: row.get(6)?,
        file_identity: StableFileIdentity::from_database_parts(
            volume_serial,
            index_high,
            index_low,
        ),
        validation_token: row.get(10)?,
    })
}

fn validate_candidate_input(input: &ScanReconnectCandidateInput<'_>) -> DbResult<()> {
    if input.source_dir_id <= 0 {
        return Err("scan reconnect source id must be positive".to_string());
    }
    for (value, label) in [
        (input.file_path, "file path"),
        (input.normalized_path, "normalized path"),
        (input.file_name, "file name"),
    ] {
        if value.trim().is_empty() {
            return Err(format!("scan reconnect candidate {label} cannot be empty"));
        }
    }
    if input.size_bytes < 0 {
        return Err("scan reconnect candidate size must not be negative".to_string());
    }
    if input
        .modified_at
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err("scan reconnect candidate modified time cannot be blank".to_string());
    }
    if input.validation_token.trim().is_empty() {
        return Err("scan reconnect candidate validation token cannot be empty".to_string());
    }
    Ok(())
}

fn normalized_file_name(file_name: &str) -> String {
    file_name.to_lowercase()
}

fn identity_database_parts(
    identity: Option<StableFileIdentity>,
) -> (Option<i64>, Option<i64>, Option<i64>) {
    identity.map_or((None, None, None), |identity| {
        let (volume, high, low) = identity.database_parts();
        (Some(volume), Some(high), Some(low))
    })
}

fn require_open_plan(connection: &Connection, source_dir_id: i64) -> DbResult<()> {
    let state = plan_state(connection, source_dir_id)?;
    match state {
        Some(false) => Ok(()),
        Some(true) => Err("scan reconnect plan was already finalized".to_string()),
        None => Err("scan reconnect plan was not started".to_string()),
    }
}

fn require_finalized_plan(connection: &Connection, source_dir_id: i64) -> DbResult<()> {
    match plan_state(connection, source_dir_id)? {
        Some(true) => Ok(()),
        Some(false) => Err("scan reconnect plan is not finalized".to_string()),
        None => Err("scan reconnect plan was not started".to_string()),
    }
}

fn plan_state(connection: &Connection, source_dir_id: i64) -> DbResult<Option<bool>> {
    ensure_temp_schema(connection)?;
    connection
        .query_row(
            &format!("SELECT finalized FROM temp.{PLAN_TABLE} WHERE source_dir_id = ?1"),
            params![source_dir_id],
            |row| row.get::<_, i64>(0).map(|value| value != 0),
        )
        .optional()
        .map_err(|error| readable_error("reading scan reconnect plan state", error))
}

fn required_planned_value<T>(value: Option<T>, label: &str) -> DbResult<T> {
    value.ok_or_else(|| format!("scan reconnect plan is missing {label}"))
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use rusqlite::{params, Connection};

    use super::*;
    use crate::db::{self, ClipInput, SourceDirInput, SourceKind, SourceProfileInput};

    struct Fixture {
        connection: Connection,
        root: std::path::PathBuf,
        source_id: i64,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "vhm-reconnect-{label}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            let connection = Connection::open_in_memory().unwrap();
            db::initialize_schema(&connection).unwrap();
            let source_path = root.join("source");
            fs::create_dir_all(&source_path).unwrap();
            let source = db::upsert_source_dir(
                &connection,
                SourceDirInput {
                    path: &source_path.display().to_string(),
                    name: "source",
                },
            )
            .unwrap();
            Self {
                connection,
                root,
                source_id: source.id,
            }
        }

        fn insert_clip(
            &self,
            path: &Path,
            file_name: &str,
            size_bytes: i64,
            modified_at: &str,
            identity: Option<StableFileIdentity>,
        ) -> i64 {
            db::upsert_scanned_clip_with_file_identity(
                &self.connection,
                ClipInput {
                    source_dir_id: self.source_id,
                    clip_group_id: None,
                    video_path: &path.display().to_string(),
                    file_name,
                    file_size: size_bytes,
                    modified_at: Some(modified_at),
                    duration_ms: None,
                    recorded_at: None,
                    cover_path: None,
                    cover_source: "missing",
                },
                identity,
            )
            .unwrap()
            .clip
            .id
        }

        fn stage(
            &self,
            path: &Path,
            file_name: &str,
            size_bytes: i64,
            modified_at: &str,
            identity: Option<StableFileIdentity>,
        ) -> i64 {
            let file_path = path.display().to_string();
            let StageScanReconnectCandidateOutcome::Staged(candidate_id) =
                stage_scan_reconnect_candidate(
                    &self.connection,
                    ScanReconnectCandidateInput {
                        source_dir_id: self.source_id,
                        file_path: &file_path,
                        normalized_path: &db::normalize_path(&file_path),
                        file_name,
                        size_bytes,
                        modified_at: Some(modified_at),
                        file_identity: identity,
                        validation_token: "test-stable-token",
                    },
                )
                .unwrap();
            candidate_id
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const IDENTITY: StableFileIdentity = StableFileIdentity {
        volume_serial: 7,
        file_index_high: 8,
        file_index_low: 9,
    };

    #[test]
    fn verbatim_reconnect_candidate_uses_stable_storage_and_nested_relative_directory() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let ordinary_root = format!(
            r"D:\__vhm_reconnect_storage_{}_{}",
            std::process::id(),
            nonce
        );
        let source = db::register_source_dir(
            &connection,
            SourceDirInput {
                path: &ordinary_root,
                name: "Reconnect storage regression",
            },
            SourceProfileInput {
                source_kind: SourceKind::Generic,
                scan_mode: SourceKind::Generic.default_scan_mode(),
                scan_root_path: &ordinary_root,
            },
            true,
        )
        .expect("ordinary production-style source should register");
        assert_eq!(source.path, ordinary_root);
        assert_eq!(source.scan_root_path, ordinary_root);

        let old_path = format!(r"{}\old\clip.mp4", ordinary_root);
        let clip_id = db::upsert_scanned_clip_with_file_identity(
            &connection,
            ClipInput {
                source_dir_id: source.id,
                clip_group_id: None,
                video_path: &old_path,
                file_name: "clip.mp4",
                file_size: 10,
                modified_at: Some("100"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
            Some(IDENTITY),
        )
        .expect("old clip should insert")
        .clip
        .id;

        let verbatim_candidate = format!(r"\\?\{}\nested\clip.mp4", ordinary_root);
        let verbatim_cover = format!(r"\\?\{}\nested\cover.jpg", ordinary_root);
        begin_scan_reconnect_plan(&connection, source.id).unwrap();
        let StageScanReconnectCandidateOutcome::Staged(candidate_id) =
            stage_scan_reconnect_candidate(
                &connection,
                ScanReconnectCandidateInput {
                    source_dir_id: source.id,
                    file_path: &verbatim_candidate,
                    normalized_path: &db::normalize_path(&verbatim_candidate),
                    file_name: "clip.mp4",
                    size_bytes: 10,
                    modified_at: Some("100"),
                    file_identity: Some(IDENTITY),
                    validation_token: "stable-token",
                },
            )
            .unwrap();
        finalize_scan_reconnect_plan(&connection, source.id).unwrap();
        let planned =
            match resolve_scan_reconnect_candidate(&connection, source.id, candidate_id).unwrap() {
                ScanReconnectDecision::Reconnect(planned) => planned,
                other => panic!("expected reconnect plan, got {other:?}"),
            };
        assert_eq!(
            planned.candidate.file_path,
            format!(r"{}\nested\clip.mp4", ordinary_root)
        );

        let outcome = apply_planned_scan_reconnect(
            &connection,
            &planned,
            ClipInput {
                source_dir_id: source.id,
                clip_group_id: None,
                video_path: &verbatim_candidate,
                file_name: "clip.mp4",
                file_size: 10,
                modified_at: Some("100"),
                duration_ms: None,
                recorded_at: None,
                cover_path: Some(&verbatim_cover),
                cover_source: "file",
            },
            Some(IDENTITY),
        )
        .expect("reconnect should apply");
        assert!(matches!(outcome, ApplyScanReconnectOutcome::Reconnected(_)));

        let stored: (String, String, String, Option<String>) = connection
            .query_row(
                "SELECT file_path, normalized_path, source_relative_dir, cover_path
                 FROM clips WHERE id = ?1",
                params![clip_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(stored.0, format!(r"{}\nested\clip.mp4", ordinary_root));
        assert_eq!(stored.1, db::normalize_path(&stored.0));
        assert_eq!(stored.2, "nested");
        assert_eq!(
            stored.3.as_deref(),
            Some(format!(r"{}\nested\cover.jpg", ordinary_root).as_str())
        );
        assert!(!db::has_windows_verbatim_prefix(&stored.0));
    }

    #[test]
    fn stable_identity_reconnect_requires_the_old_path_to_be_not_found() {
        let fixture = Fixture::new("stable");
        let old_path = fixture.root.join("source/old/clip.mp4");
        let clip_id = fixture.insert_clip(&old_path, "clip.mp4", 10, "100", Some(IDENTITY));
        let new_path = fixture.root.join("source/new/clip.mp4");
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        let candidate_id = fixture.stage(&new_path, "clip.mp4", 10, "100", Some(IDENTITY));
        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.stable_identity_match_count, 1);
        match resolve_scan_reconnect_candidate(&fixture.connection, fixture.source_id, candidate_id)
            .unwrap()
        {
            ScanReconnectDecision::Reconnect(reconnect) => {
                assert_eq!(reconnect.clip_id, clip_id);
                assert_eq!(
                    reconnect.match_kind,
                    ScanReconnectMatchKind::StableFileIdentity
                );
            }
            other => panic!("expected stable reconnect, got {other:?}"),
        }

        fs::create_dir_all(old_path.parent().unwrap()).unwrap();
        fs::write(&old_path, b"still here").unwrap();
        match resolve_scan_reconnect_candidate(&fixture.connection, fixture.source_id, candidate_id)
            .unwrap()
        {
            ScanReconnectDecision::New(candidate) => {
                assert_eq!(candidate.candidate_id, candidate_id)
            }
            other => panic!("an existing old path must not reconnect: {other:?}"),
        }
    }

    #[test]
    fn upgraded_identityless_clip_can_use_a_unique_legacy_fingerprint() {
        let fixture = Fixture::new("legacy");
        let old_path = fixture.root.join("source/old/legacy.mp4");
        let clip_id = fixture.insert_clip(&old_path, "legacy.mp4", 25, "200", None);
        let new_path = fixture.root.join("source/new/legacy.mp4");
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        let candidate_id = fixture.stage(&new_path, "legacy.mp4", 25, "200", Some(IDENTITY));
        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.legacy_fingerprint_match_count, 1);
        match resolve_scan_reconnect_candidate(&fixture.connection, fixture.source_id, candidate_id)
            .unwrap()
        {
            ScanReconnectDecision::Reconnect(reconnect) => {
                assert_eq!(reconnect.clip_id, clip_id);
                assert_eq!(
                    reconnect.match_kind,
                    ScanReconnectMatchKind::LegacyFingerprint
                );
            }
            other => panic!("expected legacy reconnect, got {other:?}"),
        }
    }

    #[test]
    fn stable_old_row_blocks_legacy_fallback_even_when_fingerprint_matches() {
        let fixture = Fixture::new("no-downgrade");
        let old_identity = StableFileIdentity {
            volume_serial: 70,
            file_index_high: 80,
            file_index_low: 90,
        };
        let old_path = fixture.root.join("source/old/same.mp4");
        fixture.insert_clip(&old_path, "same.mp4", 25, "200", Some(old_identity));
        let new_path = fixture.root.join("source/new/same.mp4");
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        let candidate_id = fixture.stage(&new_path, "same.mp4", 25, "200", Some(IDENTITY));
        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.stable_identity_match_count, 0);
        assert_eq!(stats.legacy_fingerprint_match_count, 0);
        assert!(matches!(
            resolve_scan_reconnect_candidate(&fixture.connection, fixture.source_id, candidate_id)
                .unwrap(),
            ScanReconnectDecision::New(_)
        ));
    }

    #[test]
    fn stable_and_identityless_old_rows_make_the_legacy_fingerprint_ambiguous() {
        let fixture = Fixture::new("mixed-legacy-ambiguity");
        let other_identity = StableFileIdentity {
            volume_serial: 70,
            file_index_high: 80,
            file_index_low: 90,
        };
        fixture.insert_clip(
            &fixture.root.join("source/stable/same.mp4"),
            "same.mp4",
            25,
            "200",
            Some(other_identity),
        );
        fixture.insert_clip(
            &fixture.root.join("source/legacy/same.mp4"),
            "same.mp4",
            25,
            "200",
            None,
        );
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        let candidate_id = fixture.stage(
            &fixture.root.join("source/new/same.mp4"),
            "same.mp4",
            25,
            "200",
            Some(IDENTITY),
        );
        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.legacy_fingerprint_match_count, 0);
        assert_eq!(stats.conflict_count, 1);
        assert!(matches!(
            resolve_scan_reconnect_candidate(&fixture.connection, fixture.source_id, candidate_id)
                .unwrap(),
            ScanReconnectDecision::NewWithWarning {
                warning: ScanReconnectWarning {
                    kind: ScanReconnectWarningKind::LegacyFingerprintConflict,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn duplicate_candidate_identity_never_reconnects_the_first_path_seen() {
        let fixture = Fixture::new("hardlink-ambiguity");
        let old_path = fixture.root.join("source/old/clip.mp4");
        fixture.insert_clip(&old_path, "clip.mp4", 10, "100", Some(IDENTITY));
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        let first = fixture.stage(
            &fixture.root.join("source/new-a/clip.mp4"),
            "clip.mp4",
            10,
            "100",
            Some(IDENTITY),
        );
        let second = fixture.stage(
            &fixture.root.join("source/new-b/clip.mp4"),
            "clip.mp4",
            10,
            "100",
            Some(IDENTITY),
        );
        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.conflict_count, 2);
        for candidate_id in [first, second] {
            match resolve_scan_reconnect_candidate(
                &fixture.connection,
                fixture.source_id,
                candidate_id,
            )
            .unwrap()
            {
                ScanReconnectDecision::NewWithWarning { warning, .. } => assert_eq!(
                    warning.kind,
                    ScanReconnectWarningKind::StableIdentityConflict
                ),
                other => panic!("ambiguous identity must not reconnect: {other:?}"),
            }
        }
    }

    #[test]
    fn duplicate_old_identity_never_reconnects_the_only_new_candidate() {
        let fixture = Fixture::new("duplicate-old-identity");
        for directory in ["old-a", "old-b"] {
            fixture.insert_clip(
                &fixture.root.join(format!("source/{directory}/clip.mp4")),
                "clip.mp4",
                10,
                "100",
                Some(IDENTITY),
            );
        }
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        let candidate_id = fixture.stage(
            &fixture.root.join("source/new/clip.mp4"),
            "clip.mp4",
            10,
            "100",
            Some(IDENTITY),
        );

        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.stable_identity_match_count, 0);
        assert_eq!(stats.conflict_count, 1);
        assert!(matches!(
            resolve_scan_reconnect_candidate(&fixture.connection, fixture.source_id, candidate_id)
                .unwrap(),
            ScanReconnectDecision::NewWithWarning {
                warning: ScanReconnectWarning {
                    kind: ScanReconnectWarningKind::StableIdentityConflict,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn duplicate_normalized_paths_are_kept_and_never_reconnect_user_state() {
        let fixture = Fixture::new("normalized-path-ambiguity");
        fixture.insert_clip(
            &fixture.root.join("source/old/clip.mp4"),
            "clip.mp4",
            10,
            "100",
            Some(IDENTITY),
        );
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();

        let mut candidate_ids = Vec::new();
        for file_path in [
            fixture.root.join("source/new/Clip.mp4"),
            fixture.root.join("source/new/clip.mp4"),
        ] {
            let file_path = file_path.display().to_string();
            let StageScanReconnectCandidateOutcome::Staged(candidate_id) =
                stage_scan_reconnect_candidate(
                    &fixture.connection,
                    ScanReconnectCandidateInput {
                        source_dir_id: fixture.source_id,
                        file_path: &file_path,
                        normalized_path: "c:\\source\\new\\clip.mp4",
                        file_name: "clip.mp4",
                        size_bytes: 10,
                        modified_at: Some("100"),
                        file_identity: Some(IDENTITY),
                        validation_token: "test-stable-token",
                    },
                )
                .unwrap();
            candidate_ids.push(candidate_id);
        }

        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.candidate_count, 2);
        assert_eq!(stats.stable_identity_match_count, 0);
        assert_eq!(stats.conflict_count, 2);
        for candidate_id in candidate_ids {
            assert!(matches!(
                resolve_scan_reconnect_candidate(
                    &fixture.connection,
                    fixture.source_id,
                    candidate_id,
                )
                .unwrap(),
                ScanReconnectDecision::NewWithWarning {
                    warning: ScanReconnectWarning {
                        kind: ScanReconnectWarningKind::NormalizedPathConflict,
                        ..
                    },
                    ..
                }
            ));
        }
        assert!(ScanReconnectWarningKind::NormalizedPathConflict.blocks_missing_reconciliation());
    }

    #[test]
    fn duplicate_legacy_fingerprint_on_either_side_never_reconnects() {
        let fixture = Fixture::new("legacy-ambiguity");
        fixture.insert_clip(
            &fixture.root.join("source/old-a/clip.mp4"),
            "clip.mp4",
            10,
            "100",
            None,
        );
        fixture.insert_clip(
            &fixture.root.join("source/old-b/clip.mp4"),
            "clip.mp4",
            10,
            "100",
            None,
        );
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        let candidate_id = fixture.stage(
            &fixture.root.join("source/new/clip.mp4"),
            "clip.mp4",
            10,
            "100",
            None,
        );
        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.conflict_count, 1);
        assert!(matches!(
            resolve_scan_reconnect_candidate(&fixture.connection, fixture.source_id, candidate_id)
                .unwrap(),
            ScanReconnectDecision::NewWithWarning {
                warning: ScanReconnectWarning {
                    kind: ScanReconnectWarningKind::LegacyFingerprintConflict,
                    ..
                },
                ..
            }
        ));

        let fixture = Fixture::new("legacy-candidate-ambiguity");
        fixture.insert_clip(
            &fixture.root.join("source/old/clip.mp4"),
            "clip.mp4",
            10,
            "100",
            None,
        );
        begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        let candidates = ["new-a", "new-b"].map(|directory| {
            fixture.stage(
                &fixture.root.join(format!("source/{directory}/clip.mp4")),
                "clip.mp4",
                10,
                "100",
                None,
            )
        });
        let stats = finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
        assert_eq!(stats.legacy_fingerprint_match_count, 0);
        assert_eq!(stats.conflict_count, 2);
        for candidate_id in candidates {
            assert!(matches!(
                resolve_scan_reconnect_candidate(
                    &fixture.connection,
                    fixture.source_id,
                    candidate_id,
                )
                .unwrap(),
                ScanReconnectDecision::NewWithWarning {
                    warning: ScanReconnectWarning {
                        kind: ScanReconnectWarningKind::LegacyFingerprintConflict,
                        ..
                    },
                    ..
                }
            ));
        }
    }

    #[test]
    fn trashed_or_delete_intent_rows_are_never_reconnect_targets() {
        for with_intent in [false, true] {
            let fixture = Fixture::new(if with_intent { "intent" } else { "trashed" });
            let old_path = fixture.root.join("source/old/clip.mp4");
            let clip_id = fixture.insert_clip(&old_path, "clip.mp4", 10, "100", Some(IDENTITY));
            if with_intent {
                fixture
                    .connection
                    .execute(
                        "INSERT INTO clip_delete_intents (
                            clip_id, video_path, source_dir_path, extension, file_existed
                         ) VALUES (?1, ?2, ?3, 'mp4', 0)",
                        params![
                            clip_id,
                            old_path.display().to_string(),
                            fixture.root.join("source").display().to_string(),
                        ],
                    )
                    .unwrap();
            } else {
                fixture
                    .connection
                    .execute(
                        "INSERT INTO clip_trash_snapshots (
                            clip_id, video_path, canonical_video_path, source_dir_path,
                            canonical_source_dir_path, extension, file_existed
                         ) VALUES (?1, ?2, ?2, ?3, ?3, 'mp4', 0)",
                        params![
                            clip_id,
                            old_path.display().to_string(),
                            fixture.root.join("source").display().to_string(),
                        ],
                    )
                    .unwrap();
                fixture
                    .connection
                    .execute(
                        "UPDATE clips SET file_status = 'trashed' WHERE id = ?1",
                        params![clip_id],
                    )
                    .unwrap();
            }
            begin_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
            let candidate_id = fixture.stage(
                &fixture.root.join("source/new/clip.mp4"),
                "clip.mp4",
                10,
                "100",
                Some(IDENTITY),
            );
            finalize_scan_reconnect_plan(&fixture.connection, fixture.source_id).unwrap();
            assert!(matches!(
                resolve_scan_reconnect_candidate(
                    &fixture.connection,
                    fixture.source_id,
                    candidate_id
                )
                .unwrap(),
                ScanReconnectDecision::NewWithWarning {
                    warning: ScanReconnectWarning {
                        kind: ScanReconnectWarningKind::ProtectedClip,
                        ..
                    },
                    ..
                }
            ));
        }
    }

    #[test]
    fn only_not_found_is_accepted_as_old_path_absence() {
        assert_eq!(
            classify_old_path_result(Err(io::Error::new(io::ErrorKind::NotFound, "gone"))),
            OldPathState::Missing
        );
        assert!(matches!(
            classify_old_path_result(Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "denied"
            ))),
            OldPathState::Unverifiable(_)
        ));
        assert!(ScanReconnectWarningKind::OldPathUnverifiable.blocks_missing_reconciliation());
        assert!(!ScanReconnectWarningKind::StableIdentityConflict.blocks_missing_reconciliation());
    }
}
