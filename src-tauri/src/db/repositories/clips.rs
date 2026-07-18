//! Clip lookup and state-mutation persistence.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};

use super::super::{
    attach_clip_events, bool_to_integer, ensure_row_changed, list_tags_for_clip, map_clip,
    normalize_optional, readable_error, BatchClipMutationResult, Clip, ClipDetail, ClipFileTarget,
    ClipMediaPaths, DbResult, CLIP_SELECT_SQL,
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
