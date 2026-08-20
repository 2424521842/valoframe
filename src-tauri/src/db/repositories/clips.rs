//! Clip lookup and state-mutation persistence.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};

use super::super::{
    attach_clip_events, bool_to_integer, ensure_row_changed, list_tags_for_clip, map_clip,
    normalize_optional, readable_error, BatchClipMutationResult, Clip, ClipDetail, ClipFileTarget,
    ClipMediaPaths, DbResult, FeedbackClipSnapshot, FeedbackSiblingClip, CLIP_SELECT_SQL,
};

pub fn find_clip_by_id(connection: &Connection, clip_id: i64) -> DbResult<Clip> {
    let sql = format!("{CLIP_SELECT_SQL} WHERE clips.id = ?1");
    let mut clip = connection
        .query_row(&sql, params![clip_id], map_clip)
        .map_err(|error| readable_error("reading clip", error))?;
    attach_clip_events(connection, std::slice::from_mut(&mut clip))?;

    Ok(clip)
}

/// Loads one full clip only. Unlike the legacy list command this never scans or hydrates other
/// clips, and an absent id is represented by `None` for a stable command-level not-found error.
pub fn find_clip_detail_by_id(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<Option<ClipDetail>> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting clip detail snapshot", error))?;
    let sql = format!("{CLIP_SELECT_SQL} WHERE clips.id = ?1");
    let Some(mut clip) = transaction
        .query_row(&sql, params![clip_id], map_clip)
        .optional()
        .map_err(|error| readable_error("reading clip detail", error))?
    else {
        transaction
            .commit()
            .map_err(|error| readable_error("finishing empty clip detail snapshot", error))?;
        return Ok(None);
    };
    attach_clip_events(&transaction, std::slice::from_mut(&mut clip))?;
    let tags = list_tags_for_clip(&transaction, clip_id)?;
    transaction
        .commit()
        .map_err(|error| readable_error("finishing clip detail snapshot", error))?;

    Ok(Some(ClipDetail { clip, tags }))
}

pub(crate) fn find_clip_media_paths_by_id(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<ClipMediaPaths> {
    connection
        .query_row(
            "
            SELECT
                clips.id,
                clips.file_path,
                clips.extension,
                clips.cover_path,
                CASE
                    WHEN clips.cover_source = 'file'
                         AND NULLIF(TRIM(clips.cover_path), '') IS NOT NULL
                    THEN 'file'
                    ELSE 'missing'
                END AS effective_cover_source,
                CASE
                    WHEN clips.cover_source = 'file'
                         AND NULLIF(TRIM(clips.cover_path), '') IS NOT NULL
                    THEN NULL
                    WHEN clip_thumbnails.status = 'ready'
                    THEN clip_thumbnails.cache_file
                    ELSE NULL
                END AS generated_cover_file,
                CASE
                    WHEN clips.cover_source = 'file'
                         AND NULLIF(TRIM(clips.cover_path), '') IS NOT NULL
                    THEN NULL
                    WHEN clip_thumbnails.status = 'ready'
                    THEN clip_thumbnails.revision
                    ELSE NULL
                END AS thumbnail_revision
            FROM clips
            LEFT JOIN clip_thumbnails
                ON clip_thumbnails.clip_id = clips.id
            WHERE clips.id = ?1
            ",
            params![clip_id],
            |row| {
                Ok(ClipMediaPaths {
                    id: row.get(0)?,
                    video_path: row.get(1)?,
                    extension: row.get(2)?,
                    cover_path: row.get(3)?,
                    cover_source: row.get(4)?,
                    generated_cover_file: row.get(5)?,
                    thumbnail_revision: row.get(6)?,
                })
            },
        )
        .map_err(|error| readable_error("reading clip media paths", error))
}

pub(crate) fn find_clip_file_target_by_id(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<Option<ClipFileTarget>> {
    connection
        .query_row(
            "
            SELECT
                clips.file_path,
                clips.file_status,
                clips.extension,
                source_dirs.path
            FROM clips
            JOIN source_dirs
                ON source_dirs.id = clips.source_dir_id
            WHERE clips.id = ?1
            ",
            params![clip_id],
            |row| {
                Ok(ClipFileTarget {
                    video_path: row.get(0)?,
                    file_status: row.get(1)?,
                    extension: row.get(2)?,
                    source_dir_path: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| readable_error("reading clip file target", error))
}

/// Loads the sanitized snapshot used in user-submitted feedback packages. The query never
/// selects OpenID/materialized identity keys, personal notes, tags, extracted text, or absolute
/// local paths; the transport-only video_path and clip_group_id are stripped at serialization.
pub(crate) fn find_feedback_clip_snapshot_by_id(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<Option<FeedbackClipSnapshot>> {
    connection
        .query_row(
            "
            SELECT
                clips.id,
                clips.clip_group_id,
                clip_groups.display_name AS clip_group_name,
                clips.file_path,
                clips.file_name,
                clips.extension,
                clips.size_bytes,
                clips.modified_at,
                clips.duration_ms,
                clips.recorded_at,
                clips.cover_source,
                clips.file_status,
                clip_metadata.account_name,
                clip_metadata.player_name,
                clip_metadata.agent_name,
                clip_metadata.map_name,
                clip_metadata.game_mode,
                COALESCE(clip_metadata.metadata_status, 'not_found') AS metadata_status,
                clip_metadata.match_id,
                clip_metadata.scoreline,
                clip_metadata.kda,
                matches.agent_avatar_url,
                clip_metadata.round_label,
                clip_metadata.weapon_name,
                clip_metadata.kill_count,
                matches.started_at AS match_started_at,
                match_stats.combat_score,
                match_stats.has_won,
                clip_metadata.official_video_name,
                clip_metadata.official_video_type,
                CASE
                    WHEN NULLIF(TRIM(clip_metadata.highlight_type), '') IS NOT NULL
                     AND TRIM(clip_metadata.highlight_type) NOT GLOB '*[^0-9]*'
                        THEN CAST(TRIM(clip_metadata.highlight_type) AS INTEGER)
                    ELSE NULL
                END AS highlight_type,
                clip_metadata.metadata_source,
                clip_thumbnails.status AS thumbnail_status,
                source_dirs.name AS source_dir_display_name,
                source_dirs.source_kind,
                source_dirs.scan_mode,
                clips.source_relative_dir
            FROM clips
            JOIN source_dirs
                ON source_dirs.id = clips.source_dir_id
            LEFT JOIN clip_groups
                ON clip_groups.id = clips.clip_group_id
            LEFT JOIN clip_metadata
                ON clip_metadata.clip_id = clips.id
            LEFT JOIN matches
                ON matches.game_id = clip_metadata.match_id
            LEFT JOIN match_stats
                ON match_stats.match_id = matches.id
            LEFT JOIN clip_thumbnails
                ON clip_thumbnails.clip_id = clips.id
            WHERE clips.id = ?1
            ",
            params![clip_id],
            |row| {
                let account_name: Option<String> = row.get(12)?;
                let player_name: Option<String> = row.get(13)?;
                let source_dir_display_name: String = row.get(33)?;
                let account_display_name = normalize_optional(account_name.as_deref())
                    .or_else(|| normalize_optional(player_name.as_deref()))
                    .map(str::to_owned)
                    .unwrap_or_else(|| source_dir_display_name.clone());
                let has_won = row.get::<_, Option<i64>>(27)?.map(|value| value != 0);

                Ok(FeedbackClipSnapshot {
                    id: row.get(0)?,
                    clip_group_id: row.get(1)?,
                    video_path: row.get(3)?,
                    clip_group_name: row.get(2)?,
                    file_name: row.get(4)?,
                    extension: row.get(5)?,
                    file_size: row.get(6)?,
                    modified_at: row.get(7)?,
                    duration_ms: row.get(8)?,
                    recorded_at: row.get(9)?,
                    cover_source: row.get(10)?,
                    thumbnail_status: row.get(32)?,
                    file_status: row.get(11)?,
                    account_display_name,
                    account_name,
                    player_name,
                    agent_name: row.get(14)?,
                    map_name: row.get(15)?,
                    game_mode: row.get(16)?,
                    metadata_status: row.get(17)?,
                    match_id: row.get(18)?,
                    scoreline: row.get(19)?,
                    kda: row.get(20)?,
                    agent_avatar_url: row.get(21)?,
                    round_label: row.get(22)?,
                    weapon_name: row.get(23)?,
                    kill_count: row.get(24)?,
                    match_started_at: row.get(25)?,
                    combat_score: row.get(26)?,
                    has_won,
                    official_video_name: row.get(28)?,
                    official_video_type: row.get(29)?,
                    highlight_type: row.get(30)?,
                    metadata_source: row.get(31)?,
                    source_dir_display_name,
                    source_kind: row.get(34)?,
                    scan_mode: row.get(35)?,
                    source_relative_dir: row.get(36)?,
                })
            },
        )
        .optional()
        .map_err(|error| readable_error("reading feedback clip snapshot", error))
}

/// Sibling clips of the same match (same clip group, or the same match id) for the feedback
/// package. Bounded so the operator sees enough context without shipping the whole library.
pub(crate) fn list_feedback_sibling_clips(
    connection: &Connection,
    clip_id: i64,
    clip_group_id: Option<i64>,
    match_id: Option<&str>,
) -> DbResult<Vec<FeedbackSiblingClip>> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                clips.id,
                clips.file_name,
                clips.size_bytes,
                clips.duration_ms,
                clips.recorded_at,
                clips.modified_at,
                clip_metadata.official_video_name,
                clip_metadata.kill_count,
                clip_metadata.scoreline,
                clip_metadata.agent_name,
                clip_metadata.map_name
            FROM clips
            LEFT JOIN clip_metadata
                ON clip_metadata.clip_id = clips.id
            WHERE clips.id <> ?1
              AND (
                  (?2 IS NOT NULL AND clips.clip_group_id = ?2)
                  OR (?3 IS NOT NULL AND clip_metadata.match_id = ?3)
              )
            ORDER BY COALESCE(clips.recorded_at, clips.modified_at) ASC, clips.id ASC
            LIMIT 20
            ",
        )
        .map_err(|error| readable_error("preparing feedback sibling clip query", error))?;

    let rows = statement
        .query_map(params![clip_id, clip_group_id, match_id], |row| {
            Ok(FeedbackSiblingClip {
                id: row.get(0)?,
                file_name: row.get(1)?,
                file_size: row.get(2)?,
                duration_ms: row.get(3)?,
                recorded_at: row.get(4)?,
                modified_at: row.get(5)?,
                official_video_name: row.get(6)?,
                kill_count: row.get(7)?,
                scoreline: row.get(8)?,
                agent_name: row.get(9)?,
                map_name: row.get(10)?,
            })
        })
        .map_err(|error| readable_error("querying feedback sibling clips", error))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading feedback sibling clips", error))
}

pub fn list_active_clip_paths_for_source(
    connection: &Connection,
    source_dir_id: i64,
) -> DbResult<Vec<String>> {
    let mut statement = connection
        .prepare(
            "
            SELECT normalized_path
            FROM clips
            WHERE source_dir_id = ?1
              AND file_status NOT IN ('missing', 'trashed')
            ",
        )
        .map_err(|error| readable_error("preparing active clip path query", error))?;

    let paths = statement
        .query_map(params![source_dir_id], |row| row.get::<_, String>(0))
        .map_err(|error| readable_error("querying active clip paths", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading active clip paths", error))?;

    Ok(paths)
}

pub fn find_clip_source_id_by_normalized_path(
    connection: &Connection,
    normalized_path: &str,
) -> DbResult<Option<i64>> {
    connection
        .query_row(
            "SELECT source_dir_id FROM clips WHERE normalized_path = ?1",
            params![normalized_path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| readable_error("reading clip source ownership", error))
}

pub fn mark_clip_missing_by_normalized_path(
    connection: &Connection,
    normalized_path: &str,
) -> DbResult<bool> {
    let changed = connection
        .execute(
            "
            UPDATE clips
            SET file_status = 'missing',
                updated_at = CURRENT_TIMESTAMP
            WHERE normalized_path = ?1
              AND file_status NOT IN ('missing', 'trashed')
            ",
            params![normalized_path],
        )
        .map_err(|error| readable_error("marking clip missing", error))?;

    Ok(changed > 0)
}

pub fn set_clips_favorite(
    connection: &Connection,
    clip_ids: &[i64],
    favorite: bool,
) -> DbResult<BatchClipMutationResult> {
    let clip_ids = deduplicate_clip_ids(clip_ids);
    if clip_ids.is_empty() {
        return Ok(empty_batch_clip_mutation_result());
    }

    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting favorite batch transaction", error))?;
    let mut matched_ids = Vec::with_capacity(clip_ids.len());
    let mut missing_ids = Vec::new();
    let mut updated = 0;

    for clip_id in &clip_ids {
        let current = transaction
            .query_row(
                "SELECT is_favorite FROM clips WHERE id = ?1",
                params![clip_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| readable_error("matching clips for favorite batch", error))?;

        let Some(current) = current else {
            missing_ids.push(*clip_id);
            continue;
        };
        matched_ids.push(*clip_id);

        if (current != 0) != favorite {
            updated += transaction
                .execute(
                    "
                    UPDATE clips
                    SET is_favorite = ?1,
                        updated_at = CURRENT_TIMESTAMP
                    WHERE id = ?2
                    ",
                    params![bool_to_integer(favorite), clip_id],
                )
                .map_err(|error| readable_error("updating favorite batch", error))?;
        }
    }

    finish_batch_clip_mutation(
        transaction,
        clip_ids.len(),
        matched_ids,
        updated,
        missing_ids,
        "favorite batch",
    )
}

pub fn set_clips_trashed(
    connection: &Connection,
    clip_ids: &[i64],
    trashed: bool,
) -> DbResult<BatchClipMutationResult> {
    super::deletions::set_clips_trashed_guarded(connection, clip_ids, trashed)
}

pub fn add_tag_to_clips(
    connection: &Connection,
    clip_ids: &[i64],
    tag_id: i64,
) -> DbResult<BatchClipMutationResult> {
    mutate_clip_tags(connection, clip_ids, tag_id, ClipTagMutation::Add)
}

pub fn remove_tag_from_clips(
    connection: &Connection,
    clip_ids: &[i64],
    tag_id: i64,
) -> DbResult<BatchClipMutationResult> {
    mutate_clip_tags(connection, clip_ids, tag_id, ClipTagMutation::Remove)
}

pub fn update_clip_favorite(connection: &Connection, clip_id: i64, favorite: bool) -> DbResult<()> {
    let changed = connection
        .execute(
            "
            UPDATE clips
            SET is_favorite = ?1,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?2
            ",
            params![bool_to_integer(favorite), clip_id],
        )
        .map_err(|error| readable_error("updating favorite", error))?;

    ensure_row_changed(changed, "updating favorite", clip_id)
}

pub fn update_clip_note(connection: &Connection, clip_id: i64, note: Option<&str>) -> DbResult<()> {
    let normalized_note = normalize_optional(note);

    let changed = connection
        .execute(
            "
            UPDATE clips
            SET note = ?1,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?2
            ",
            params![normalized_note, clip_id],
        )
        .map_err(|error| readable_error("updating note", error))?;

    ensure_row_changed(changed, "updating note", clip_id)
}

pub fn update_clip_trashed(connection: &Connection, clip_id: i64, trashed: bool) -> DbResult<()> {
    let result = super::deletions::set_clips_trashed_guarded(connection, &[clip_id], trashed)?;
    if result.matched == 0 {
        return Err(format!(
            "Database updating recycle-bin state failed because id {clip_id} was not found"
        ));
    }
    Ok(())
}

pub fn delete_clip_from_index(connection: &Connection, clip_id: i64) -> DbResult<()> {
    let changed = connection
        .execute("DELETE FROM clips WHERE id = ?1", params![clip_id])
        .map_err(|error| readable_error("removing clip from index", error))?;

    ensure_row_changed(changed, "removing clip from index", clip_id)
}

#[derive(Clone, Copy)]
enum ClipTagMutation {
    Add,
    Remove,
}

fn mutate_clip_tags(
    connection: &Connection,
    clip_ids: &[i64],
    tag_id: i64,
    mutation: ClipTagMutation,
) -> DbResult<BatchClipMutationResult> {
    let clip_ids = deduplicate_clip_ids(clip_ids);
    if clip_ids.is_empty() {
        return Ok(empty_batch_clip_mutation_result());
    }

    let action = match mutation {
        ClipTagMutation::Add => "add-tag batch",
        ClipTagMutation::Remove => "remove-tag batch",
    };
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error(&format!("starting {action} transaction"), error))?;
    let tag_exists = transaction
        .query_row("SELECT 1 FROM tags WHERE id = ?1", params![tag_id], |_| {
            Ok(())
        })
        .optional()
        .map_err(|error| readable_error("validating batch tag id", error))?
        .is_some();
    if !tag_exists {
        return Err(format!(
            "updating clip tags failed: tag id {tag_id} was not found"
        ));
    }

    let mut matched_ids = Vec::with_capacity(clip_ids.len());
    let mut missing_ids = Vec::new();
    let mut updated = 0;

    for clip_id in &clip_ids {
        let clip_exists = transaction
            .query_row(
                "SELECT 1 FROM clips WHERE id = ?1",
                params![clip_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| readable_error("matching clips for tag batch", error))?
            .is_some();
        if !clip_exists {
            missing_ids.push(*clip_id);
            continue;
        }
        matched_ids.push(*clip_id);

        updated += match mutation {
            ClipTagMutation::Add => transaction
                .execute(
                    "
                    INSERT INTO clip_tags (clip_id, tag_id)
                    VALUES (?1, ?2)
                    ON CONFLICT(clip_id, tag_id) DO NOTHING
                    ",
                    params![clip_id, tag_id],
                )
                .map_err(|error| readable_error("adding tag batch", error))?,
            ClipTagMutation::Remove => transaction
                .execute(
                    "DELETE FROM clip_tags WHERE clip_id = ?1 AND tag_id = ?2",
                    params![clip_id, tag_id],
                )
                .map_err(|error| readable_error("removing tag batch", error))?,
        };
    }

    finish_batch_clip_mutation(
        transaction,
        clip_ids.len(),
        matched_ids,
        updated,
        missing_ids,
        action,
    )
}

fn finish_batch_clip_mutation(
    transaction: rusqlite::Transaction<'_>,
    requested: usize,
    matched_ids: Vec<i64>,
    updated: usize,
    missing_ids: Vec<i64>,
    action: &str,
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
        .map_err(|error| readable_error(&format!("committing {action} transaction"), error))?;
    Ok(result)
}

fn deduplicate_clip_ids(clip_ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::with_capacity(clip_ids.len());
    clip_ids
        .iter()
        .copied()
        .filter(|clip_id| seen.insert(*clip_id))
        .collect()
}

pub(in crate::db) fn empty_batch_clip_mutation_result() -> BatchClipMutationResult {
    BatchClipMutationResult {
        requested: 0,
        matched: 0,
        updated: 0,
        missing_ids: Vec::new(),
        clips: Vec::new(),
    }
}

pub(in crate::db) fn find_clip_by_normalized_path(
    connection: &Connection,
    normalized_path: &str,
) -> DbResult<Clip> {
    let sql = format!("{CLIP_SELECT_SQL} WHERE clips.normalized_path = ?1");
    connection
        .query_row(&sql, params![normalized_path], map_clip)
        .map_err(|error| readable_error("reading clip", error))
}

pub(in crate::db) fn find_optional_clip_by_normalized_path(
    connection: &Connection,
    normalized_path: &str,
) -> DbResult<Option<Clip>> {
    let sql = format!("{CLIP_SELECT_SQL} WHERE clips.normalized_path = ?1");
    connection
        .query_row(&sql, params![normalized_path], map_clip)
        .optional()
        .map_err(|error| readable_error("reading optional clip", error))
}
