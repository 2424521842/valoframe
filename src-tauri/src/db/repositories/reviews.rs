//! Stable quick-review queue reads and transactional review decisions.

use std::collections::BTreeSet;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::{
    readable_error, ClipReviewMutationResult, ClipReviewState, DbResult, ReviewClipPage,
    ReviewDecision, ReviewQueueQuery, DEFAULT_CLIP_PAGE_LIMIT, MAX_CLIP_PAGE_LIMIT,
};
use super::library::{
    append_account_filter, append_agent_filter, append_exact_text_filter, append_map_filter,
    attach_clip_summary_tags, map_clip_summary, normalized_filter_value, CLIP_LIST_FROM_SQL,
    CLIP_SUMMARY_SELECT_SQL,
};

const REVIEW_CURSOR_VERSION: u8 = 2;
const MAX_REVIEW_FILTER_IDS: usize = 10_000;
const MAX_REVIEW_CURSOR_BYTES: usize = 1_024;

/// SQLite expression for the queue's effective recording timestamp. Scanner-owned fields contain
/// a mix of Unix-second strings and ISO UTC values, so each fallback must be normalized before the
/// first usable value is selected.
const REVIEW_EFFECTIVE_UNIX_SQL: &str = "COALESCE(
    CASE
        WHEN NULLIF(TRIM(clips.recorded_at), '') IS NULL THEN NULL
        WHEN TRIM(clips.recorded_at) NOT GLOB '*[^0-9]*'
            THEN CAST(clips.recorded_at AS INTEGER)
        ELSE unixepoch(clips.recorded_at)
    END,
    CASE
        WHEN NULLIF(TRIM(clips.modified_at), '') IS NULL THEN NULL
        WHEN TRIM(clips.modified_at) NOT GLOB '*[^0-9]*'
            THEN CAST(clips.modified_at AS INTEGER)
        ELSE unixepoch(clips.modified_at)
    END,
    CASE
        WHEN NULLIF(TRIM(clips.first_indexed_at), '') IS NULL THEN NULL
        WHEN TRIM(clips.first_indexed_at) NOT GLOB '*[^0-9]*'
            THEN CAST(clips.first_indexed_at AS INTEGER)
        ELSE unixepoch(clips.first_indexed_at)
    END,
    0
)";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewCursor {
    version: u8,
    snapshot_max_clip_id: i64,
    query_fingerprint: String,
    effective_time: i64,
    clip_id: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewQueryFingerprint<'a> {
    source_dir_ids: &'a [i64],
    tag_ids: &'a [i64],
    account_id: Option<&'a str>,
    agent_name: Option<&'a str>,
    map_name: Option<&'a str>,
    game_mode: Option<&'a str>,
    recorded_from: Option<i64>,
    recorded_to: Option<i64>,
}

#[derive(Debug)]
struct ReviewFilter {
    where_sql: String,
    params: Vec<Value>,
}

pub fn list_review_clip_page(
    connection: &Connection,
    query: &ReviewQueueQuery,
) -> DbResult<ReviewClipPage> {
    let limit = validate_review_query(query)?;
    let decoded_cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let query_fingerprint = review_query_fingerprint(query)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting review queue snapshot", error))?;

    let snapshot_max_clip_id = resolve_snapshot_max_clip_id(
        &transaction,
        query.snapshot_max_clip_id,
        decoded_cursor.as_ref(),
    )?;
    let count_filter = build_review_filter(&transaction, query, snapshot_max_clip_id, None, None)?;
    let count_sql = format!(
        "SELECT COUNT(*) {CLIP_LIST_FROM_SQL} {}",
        count_filter.where_sql
    );
    let candidate_count = transaction
        .query_row(
            &count_sql,
            params_from_iter(count_filter.params.iter()),
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| readable_error("counting review queue candidates", error))?;

    let page_filter = build_review_filter(
        &transaction,
        query,
        snapshot_max_clip_id,
        decoded_cursor.as_ref(),
        Some(&query_fingerprint),
    )?;
    let page_sql = format!(
        "{CLIP_SUMMARY_SELECT_SQL},
         {REVIEW_EFFECTIVE_UNIX_SQL} AS review_effective_time
         {CLIP_LIST_FROM_SQL}
         {}
         ORDER BY review_effective_time DESC, clips.id DESC
         LIMIT ?",
        page_filter.where_sql
    );
    let mut page_params = page_filter.params;
    page_params.push(Value::Integer(limit.saturating_add(1)));
    let mut rows = {
        let mut statement = transaction
            .prepare(&page_sql)
            .map_err(|error| readable_error("preparing review queue page", error))?;
        let mapped = statement
            .query_map(params_from_iter(page_params.iter()), |row| {
                Ok((map_clip_summary(row)?, row.get::<_, i64>(45)?))
            })
            .map_err(|error| readable_error("querying review queue page", error))?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading review queue page", error))?
    };

    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_cursor = if has_more {
        rows.last()
            .map(|(clip, effective_time)| {
                encode_cursor(ReviewCursor {
                    version: REVIEW_CURSOR_VERSION,
                    snapshot_max_clip_id,
                    query_fingerprint: query_fingerprint.clone(),
                    effective_time: *effective_time,
                    clip_id: clip.id,
                })
            })
            .transpose()?
    } else {
        None
    };
    let mut items = rows
        .into_iter()
        .map(|(clip, _effective_time)| clip)
        .collect::<Vec<_>>();
    attach_clip_summary_tags(&transaction, &mut items)?;
    transaction
        .commit()
        .map_err(|error| readable_error("finishing review queue snapshot", error))?;

    Ok(ReviewClipPage {
        items,
        snapshot_max_clip_id,
        candidate_count,
        limit,
        has_more,
        next_cursor,
    })
}

/// Applies a user review action. A normal liked action synchronizes favorite=true in the same
/// statement; disliked and reset preserve the existing favorite value.
pub fn set_clip_review_decision(
    connection: &Connection,
    clip_id: i64,
    review_decision: ReviewDecision,
) -> DbResult<ClipReviewMutationResult> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting clip review transaction", error))?;
    let before = find_clip_review_state(&transaction, clip_id)?;
    let reviewed_at = if review_decision == ReviewDecision::Unreviewed {
        None
    } else if review_decision == before.review_decision {
        before.reviewed_at.clone()
    } else {
        Some(
            transaction
                .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|error| readable_error("creating clip review timestamp", error))?,
        )
    };
    let after = ClipReviewState {
        clip_id,
        review_decision,
        reviewed_at,
        favorite: if review_decision == ReviewDecision::Liked {
            true
        } else {
            before.favorite
        },
    };
    finish_review_mutation(transaction, before, after, "updating clip review decision")
}

pub fn reset_clip_review_decision(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<ClipReviewMutationResult> {
    set_clip_review_decision(connection, clip_id, ReviewDecision::Unreviewed)
}

/// Restores the exact `before` state returned by a prior mutation. This intentionally restores the
/// original review timestamp as well as favorite state instead of synthesizing a new decision.
pub fn restore_clip_review_state(
    connection: &Connection,
    expected_current: &ClipReviewState,
    restore_state: &ClipReviewState,
) -> DbResult<ClipReviewMutationResult> {
    if expected_current.clip_id != restore_state.clip_id {
        return Err("clip review restore states belong to different clips".to_string());
    }
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting clip review restore transaction", error))?;
    let before = find_clip_review_state(&transaction, restore_state.clip_id)?;
    if &before != expected_current {
        return Err(
            "clip review state changed after the decision; refusing stale undo".to_string(),
        );
    }
    finish_review_mutation(
        transaction,
        before,
        restore_state.clone(),
        "restoring clip review state",
    )
}

fn validate_review_query(query: &ReviewQueueQuery) -> DbResult<i64> {
    let limit = query.limit.unwrap_or(DEFAULT_CLIP_PAGE_LIMIT);
    if !(1..=MAX_CLIP_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "review queue limit must be between 1 and {MAX_CLIP_PAGE_LIMIT}"
        ));
    }
    if matches!(
        (query.recorded_from, query.recorded_to),
        (Some(from), Some(to)) if from > to
    ) {
        return Err("review queue recorded-from cannot exceed recorded-to".to_string());
    }
    validate_filter_ids(query.source_dir_ids.as_deref(), "source directory")?;
    validate_filter_ids(query.tag_ids.as_deref(), "tag")?;
    if query.snapshot_max_clip_id.is_some_and(|value| value < 0) {
        return Err("review queue snapshot max clip id must be non-negative".to_string());
    }
    Ok(limit)
}

fn validate_filter_ids(values: Option<&[i64]>, label: &str) -> DbResult<()> {
    let Some(values) = values else {
        return Ok(());
    };
    if values.len() > MAX_REVIEW_FILTER_IDS {
        return Err(format!(
            "review queue accepts at most {MAX_REVIEW_FILTER_IDS} {label} ids"
        ));
    }
    if values.iter().any(|value| *value <= 0) {
        return Err(format!("review queue {label} ids must be positive"));
    }
    Ok(())
}

fn resolve_snapshot_max_clip_id(
    connection: &Connection,
    requested_snapshot: Option<i64>,
    cursor: Option<&ReviewCursor>,
) -> DbResult<i64> {
    if let (Some(requested), Some(cursor)) = (requested_snapshot, cursor) {
        if requested != cursor.snapshot_max_clip_id {
            return Err(
                "review queue cursor does not belong to the requested snapshot".to_string(),
            );
        }
    }
    if let Some(cursor) = cursor {
        return Ok(cursor.snapshot_max_clip_id);
    }
    if let Some(requested) = requested_snapshot {
        return Ok(requested);
    }
    connection
        .query_row("SELECT COALESCE(MAX(id), 0) FROM clips", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| readable_error("freezing review queue clip upper bound", error))
}

fn build_review_filter(
    connection: &Connection,
    query: &ReviewQueueQuery,
    snapshot_max_clip_id: i64,
    cursor: Option<&ReviewCursor>,
    expected_query_fingerprint: Option<&str>,
) -> DbResult<ReviewFilter> {
    let source_dir_ids = normalized_filter_ids(query.source_dir_ids.as_deref());
    let tag_ids = normalized_filter_ids(query.tag_ids.as_deref());
    let mut conditions = vec![
        "clips.id <= ?".to_string(),
        "clips.file_status = 'available'".to_string(),
        "clips.review_decision = 'unreviewed'".to_string(),
    ];
    let mut values = vec![Value::Integer(snapshot_max_clip_id)];

    if !source_dir_ids.is_empty() {
        conditions.push(
            "clips.source_dir_id IN (
                SELECT CAST(value AS INTEGER)
                FROM json_each(?)
                WHERE type = 'integer'
            )"
            .to_string(),
        );
        values.push(Value::Text(serialize_filter_ids(
            &source_dir_ids,
            "source directory",
        )?));
    }
    if !tag_ids.is_empty() {
        conditions.push(
            "EXISTS (
                SELECT 1
                FROM clip_tags review_clip_tags
                WHERE review_clip_tags.clip_id = clips.id
                  AND review_clip_tags.tag_id IN (
                      SELECT CAST(value AS INTEGER)
                      FROM json_each(?)
                      WHERE type = 'integer'
                  )
            )"
            .to_string(),
        );
        values.push(Value::Text(serialize_filter_ids(&tag_ids, "tag")?));
    }
    append_account_filter(
        connection,
        query.account_id.as_deref(),
        &mut conditions,
        &mut values,
    )?;
    append_agent_filter(query.agent_name.as_deref(), &mut conditions, &mut values);
    append_map_filter(
        connection,
        query.map_name.as_deref(),
        &mut conditions,
        &mut values,
    )?;
    append_exact_text_filter(
        "clip_metadata.game_mode",
        query.game_mode.as_deref(),
        &mut conditions,
        &mut values,
    );
    if let Some(recorded_from) = query.recorded_from {
        conditions.push(format!("{REVIEW_EFFECTIVE_UNIX_SQL} >= ?"));
        values.push(Value::Integer(recorded_from));
    }
    if let Some(recorded_to) = query.recorded_to {
        conditions.push(format!("{REVIEW_EFFECTIVE_UNIX_SQL} <= ?"));
        values.push(Value::Integer(recorded_to));
    }
    if let Some(cursor) = cursor {
        if cursor.version != REVIEW_CURSOR_VERSION
            || cursor.snapshot_max_clip_id != snapshot_max_clip_id
            || expected_query_fingerprint
                .is_none_or(|fingerprint| fingerprint != cursor.query_fingerprint)
            || cursor.clip_id <= 0
            || cursor.clip_id > snapshot_max_clip_id
        {
            return Err("review queue cursor is invalid for this snapshot".to_string());
        }
        conditions.push(format!(
            "({REVIEW_EFFECTIVE_UNIX_SQL} < ? OR
              ({REVIEW_EFFECTIVE_UNIX_SQL} = ? AND clips.id < ?))"
        ));
        values.push(Value::Integer(cursor.effective_time));
        values.push(Value::Integer(cursor.effective_time));
        values.push(Value::Integer(cursor.clip_id));
    }

    Ok(ReviewFilter {
        where_sql: format!("WHERE {}", conditions.join(" AND ")),
        params: values,
    })
}

fn normalized_filter_ids(values: Option<&[i64]>) -> Vec<i64> {
    values
        .unwrap_or_default()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn review_query_fingerprint(query: &ReviewQueueQuery) -> DbResult<String> {
    let source_dir_ids = normalized_filter_ids(query.source_dir_ids.as_deref());
    let tag_ids = normalized_filter_ids(query.tag_ids.as_deref());
    let canonical = ReviewQueryFingerprint {
        source_dir_ids: &source_dir_ids,
        tag_ids: &tag_ids,
        account_id: normalized_filter_value(query.account_id.as_deref()),
        agent_name: normalized_filter_value(query.agent_name.as_deref()),
        map_name: normalized_filter_value(query.map_name.as_deref()),
        game_mode: normalized_filter_value(query.game_mode.as_deref()),
        recorded_from: query.recorded_from,
        recorded_to: query.recorded_to,
    };
    let encoded = serde_json::to_vec(&canonical)
        .map_err(|error| readable_error("fingerprinting review queue query", error))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn serialize_filter_ids(values: &[i64], label: &str) -> DbResult<String> {
    serde_json::to_string(values)
        .map_err(|error| readable_error(&format!("encoding review queue {label} ids"), error))
}

fn encode_cursor(cursor: ReviewCursor) -> DbResult<String> {
    let json = serde_json::to_vec(&cursor)
        .map_err(|error| readable_error("encoding review queue cursor", error))?;
    Ok(URL_SAFE_NO_PAD.encode(json))
}

fn decode_cursor(value: &str) -> DbResult<ReviewCursor> {
    if value.is_empty() || value.len() > MAX_REVIEW_CURSOR_BYTES {
        return Err("review queue cursor is malformed".to_string());
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "review queue cursor is malformed".to_string())?;
    let cursor = serde_json::from_slice::<ReviewCursor>(&decoded)
        .map_err(|_| "review queue cursor is malformed".to_string())?;
    if cursor.version != REVIEW_CURSOR_VERSION
        || cursor.snapshot_max_clip_id < 0
        || cursor.query_fingerprint.len() != 64
        || !cursor
            .query_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || cursor.clip_id <= 0
        || cursor.clip_id > cursor.snapshot_max_clip_id
    {
        return Err("review queue cursor is malformed".to_string());
    }
    Ok(cursor)
}

fn find_clip_review_state(connection: &Connection, clip_id: i64) -> DbResult<ClipReviewState> {
    connection
        .query_row(
            "
            SELECT review_decision, reviewed_at, is_favorite
            FROM clips
            WHERE id = ?1
            ",
            params![clip_id],
            |row| {
                Ok(ClipReviewState {
                    clip_id,
                    review_decision: row.get(0)?,
                    reviewed_at: row.get(1)?,
                    favorite: row.get::<_, i64>(2)? != 0,
                })
            },
        )
        .optional()
        .map_err(|error| readable_error("reading clip review state", error))?
        .ok_or_else(|| format!("clip id {clip_id} was not found"))
}

fn finish_review_mutation(
    transaction: rusqlite::Transaction<'_>,
    before: ClipReviewState,
    after: ClipReviewState,
    action: &str,
) -> DbResult<ClipReviewMutationResult> {
    if before.clip_id != after.clip_id {
        return Err("clip review restore state belongs to a different clip".to_string());
    }
    let changed = before != after;
    if changed {
        transaction
            .execute(
                "
                UPDATE clips
                SET review_decision = ?1,
                    reviewed_at = ?2,
                    is_favorite = ?3,
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = ?4
                ",
                params![
                    after.review_decision.as_str(),
                    after.reviewed_at,
                    i64::from(after.favorite),
                    after.clip_id
                ],
            )
            .map_err(|error| readable_error(action, error))?;
    }
    let result = ClipReviewMutationResult {
        before,
        after,
        changed,
    };
    transaction
        .commit()
        .map_err(|error| readable_error(&format!("committing {action}"), error))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};

    use super::*;
    use crate::db::{
        assign_tag_to_clip, create_tag, initialize_schema, upsert_clip, upsert_source_dir,
        ClipInput, SourceDir, SourceDirInput,
    };

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("review fixture database should open");
        initialize_schema(&connection).expect("review fixture schema should initialize");
        connection
    }

    fn seed_source(connection: &Connection, key: &str) -> SourceDir {
        let path = format!("C:\\ReviewQueue\\{key}");
        upsert_source_dir(
            connection,
            SourceDirInput {
                path: &path,
                name: key,
            },
        )
        .expect("review source should seed")
    }

    fn seed_clip(
        connection: &Connection,
        source: &SourceDir,
        key: &str,
        recorded_at: Option<&str>,
        modified_at: Option<&str>,
        first_indexed_at: &str,
    ) -> i64 {
        let video_path = format!("{}\\{key}.mp4", source.path);
        let file_name = format!("{key}.mp4");
        let clip = upsert_clip(
            connection,
            ClipInput {
                source_dir_id: source.id,
                clip_group_id: None,
                video_path: &video_path,
                file_name: &file_name,
                file_size: 42,
                modified_at,
                duration_ms: Some(12_000),
                recorded_at,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("review clip should seed");
        connection
            .execute(
                "UPDATE clips SET first_indexed_at = ?2 WHERE id = ?1",
                params![clip.id, first_indexed_at],
            )
            .expect("review first-indexed timestamp should seed");
        clip.id
    }

    fn seed_clip_metadata(
        connection: &Connection,
        clip_id: i64,
        account_id: &str,
        agent_name: &str,
        map_name: &str,
        game_mode: &str,
    ) {
        let match_id = format!("review-match-{clip_id}");
        connection
            .execute(
                "
                INSERT INTO matches (game_id, account_id)
                VALUES (?1, ?2)
                ON CONFLICT(game_id) DO UPDATE SET account_id = excluded.account_id
                ",
                params![match_id, account_id],
            )
            .expect("review match metadata should seed");
        connection
            .execute(
                "
                INSERT INTO clip_metadata (
                    clip_id,
                    metadata_status,
                    account_name,
                    agent_name,
                    map_name,
                    game_mode,
                    match_id,
                    metadata_source
                )
                VALUES (?1, 'enriched', ?2, ?3, ?4, ?5, ?6, 'wonderful_db')
                ON CONFLICT(clip_id) DO UPDATE SET
                    metadata_status = excluded.metadata_status,
                    account_name = excluded.account_name,
                    agent_name = excluded.agent_name,
                    map_name = excluded.map_name,
                    game_mode = excluded.game_mode,
                    match_id = excluded.match_id,
                    metadata_source = excluded.metadata_source
                ",
                params![
                    clip_id,
                    format!("账号 {account_id}"),
                    agent_name,
                    map_name,
                    game_mode,
                    match_id,
                ],
            )
            .expect("review clip metadata should seed");
    }

    #[test]
    fn queue_combines_dimensions_and_uses_effective_time_fallbacks() {
        let connection = test_connection();
        let source_a = seed_source(&connection, "source-a");
        let source_b = seed_source(&connection, "source-b");
        let source_c = seed_source(&connection, "source-c");
        let tag_a = create_tag(&connection, "tag-a", None).expect("tag a should seed");
        let tag_b = create_tag(&connection, "tag-b", None).expect("tag b should seed");

        let recorded_wins = seed_clip(
            &connection,
            &source_a,
            "recorded-wins",
            Some("300"),
            Some("900"),
            "100",
        );
        let modified_fallback = seed_clip(
            &connection,
            &source_b,
            "modified-fallback",
            None,
            Some("250"),
            "100",
        );
        let indexed_fallback = seed_clip(
            &connection,
            &source_a,
            "indexed-fallback",
            Some("invalid-recorded-time"),
            None,
            "200",
        );
        let wrong_source = seed_clip(
            &connection,
            &source_c,
            "wrong-source",
            Some("280"),
            None,
            "100",
        );
        let missing_tag = seed_clip(
            &connection,
            &source_a,
            "missing-tag",
            Some("275"),
            None,
            "100",
        );
        let missing_file = seed_clip(
            &connection,
            &source_a,
            "missing-file",
            Some("260"),
            None,
            "100",
        );
        let already_reviewed = seed_clip(
            &connection,
            &source_b,
            "already-reviewed",
            Some("240"),
            None,
            "100",
        );
        assign_tag_to_clip(&connection, recorded_wins, tag_a.id).expect("tag should attach");
        assign_tag_to_clip(&connection, modified_fallback, tag_b.id).expect("tag should attach");
        assign_tag_to_clip(&connection, indexed_fallback, tag_a.id).expect("tag should attach");
        assign_tag_to_clip(&connection, wrong_source, tag_a.id).expect("tag should attach");
        assign_tag_to_clip(&connection, missing_file, tag_a.id).expect("tag should attach");
        assign_tag_to_clip(&connection, already_reviewed, tag_b.id).expect("tag should attach");
        connection
            .execute(
                "UPDATE clips SET file_status = 'missing' WHERE id = ?1",
                [missing_file],
            )
            .expect("missing status should seed");
        set_clip_review_decision(&connection, already_reviewed, ReviewDecision::Disliked)
            .expect("reviewed status should seed");

        let page = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                source_dir_ids: Some(vec![source_b.id, source_a.id, source_a.id]),
                tag_ids: Some(vec![tag_b.id, tag_a.id, tag_a.id]),
                recorded_from: Some(190),
                recorded_to: Some(300),
                limit: Some(20),
                ..ReviewQueueQuery::default()
            },
        )
        .expect("combined review queue should load");

        assert_eq!(
            page.items.iter().map(|clip| clip.id).collect::<Vec<_>>(),
            vec![recorded_wins, modified_fallback, indexed_fallback]
        );
        assert_eq!(page.candidate_count, 3);
        assert!(!page.has_more);
        assert_eq!(page.next_cursor, None);
        assert!(page.items.iter().all(|clip| !clip.tag_ids.is_empty()));
        assert!(!page.items.iter().any(|clip| clip.id == missing_tag));
        assert!(!page.items.iter().any(|clip| clip.id == missing_file));
        assert!(!page.items.iter().any(|clip| clip.id == already_reviewed));
    }

    #[test]
    fn queue_combines_account_agent_map_and_game_mode_with_library_semantics() {
        let connection = test_connection();
        let source = seed_source(&connection, "metadata-filter-source");
        let target = seed_clip(
            &connection,
            &source,
            "metadata-target",
            Some("500"),
            None,
            "1",
        );
        seed_clip_metadata(&connection, target, "1001", "Jett", "天枢之阙", "竞技模式");

        let mismatches = [
            ("wrong-account", "2002", "Jett", "天枢之阙", "竞技模式"),
            ("wrong-agent", "1001", "Sage", "天枢之阙", "竞技模式"),
            ("wrong-map", "1001", "Jett", "双塔迷城", "竞技模式"),
            ("wrong-mode", "1001", "Jett", "天枢之阙", "极速模式"),
        ];
        for (key, account_id, agent_name, map_name, game_mode) in mismatches {
            let clip_id = seed_clip(&connection, &source, key, Some("400"), None, "1");
            seed_clip_metadata(
                &connection,
                clip_id,
                account_id,
                agent_name,
                map_name,
                game_mode,
            );
        }

        let page = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                account_id: Some("match-account-1001".to_string()),
                // The stored source value is `Jett`; the localized library selection is `捷风`.
                agent_name: Some("捷风".to_string()),
                map_name: Some("天枢之阙".to_string()),
                game_mode: Some("竞技模式".to_string()),
                limit: Some(20),
                ..ReviewQueueQuery::default()
            },
        )
        .expect("metadata-filtered review queue should load");

        assert_eq!(page.candidate_count, 1);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, target);
    }

    #[test]
    fn snapshot_and_keyset_survive_decisions_new_inserts_and_normalized_filters() {
        let connection = test_connection();
        let source = seed_source(&connection, "snapshot-source");
        let old_ids = [500, 400, 300, 200, 100]
            .into_iter()
            .map(|timestamp| {
                seed_clip(
                    &connection,
                    &source,
                    &format!("old-{timestamp}"),
                    Some(&timestamp.to_string()),
                    None,
                    "1",
                )
            })
            .collect::<Vec<_>>();
        let first = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                source_dir_ids: Some(vec![source.id, source.id]),
                limit: Some(2),
                ..ReviewQueueQuery::default()
            },
        )
        .expect("first review page should load");
        assert_eq!(first.candidate_count, 5);
        assert_eq!(
            first.items.iter().map(|clip| clip.id).collect::<Vec<_>>(),
            old_ids[..2]
        );
        let cursor = first
            .next_cursor
            .clone()
            .expect("first page should have cursor");

        for clip in &first.items {
            set_clip_review_decision(&connection, clip.id, ReviewDecision::Disliked)
                .expect("review decision should persist");
        }
        let new_clip = seed_clip(
            &connection,
            &source,
            "new-after-snapshot",
            Some("600"),
            None,
            "1",
        );
        assert!(new_clip > first.snapshot_max_clip_id);

        let second = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                source_dir_ids: Some(vec![source.id]),
                snapshot_max_clip_id: Some(first.snapshot_max_clip_id),
                cursor: Some(cursor.clone()),
                limit: Some(2),
                ..ReviewQueueQuery::default()
            },
        )
        .expect("second keyset page should load");
        assert_eq!(
            second.items.iter().map(|clip| clip.id).collect::<Vec<_>>(),
            old_ids[2..4]
        );
        assert_eq!(second.candidate_count, 3);
        assert!(!second.items.iter().any(|clip| clip.id == new_clip));

        let third = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                source_dir_ids: Some(vec![source.id]),
                snapshot_max_clip_id: Some(first.snapshot_max_clip_id),
                cursor: second.next_cursor,
                limit: Some(2),
                ..ReviewQueueQuery::default()
            },
        )
        .expect("final keyset page should load");
        assert_eq!(
            third.items.iter().map(|clip| clip.id).collect::<Vec<_>>(),
            old_ids[4..]
        );
        assert!(!third.has_more);

        let changed_filter = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                source_dir_ids: Some(vec![source.id]),
                recorded_from: Some(1),
                snapshot_max_clip_id: Some(first.snapshot_max_clip_id),
                cursor: Some(cursor.clone()),
                limit: Some(2),
                ..ReviewQueueQuery::default()
            },
        )
        .expect_err("cursor must bind the frozen filters");
        assert!(changed_filter.contains("cursor"));

        let changed_snapshot = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                source_dir_ids: Some(vec![source.id]),
                snapshot_max_clip_id: Some(first.snapshot_max_clip_id + 1),
                cursor: Some(cursor),
                limit: Some(2),
                ..ReviewQueueQuery::default()
            },
        )
        .expect_err("cursor must bind the frozen snapshot");
        assert!(changed_snapshot.contains("snapshot"));
    }

    #[test]
    fn cursor_binds_every_metadata_filter() {
        let connection = test_connection();
        let source = seed_source(&connection, "metadata-cursor-source");
        for (index, timestamp) in [300, 200, 100].into_iter().enumerate() {
            let clip_id = seed_clip(
                &connection,
                &source,
                &format!("metadata-cursor-{index}"),
                Some(&timestamp.to_string()),
                None,
                "1",
            );
            seed_clip_metadata(&connection, clip_id, "1001", "Jett", "天枢之阙", "竞技模式");
        }

        let base_query = ReviewQueueQuery {
            account_id: Some("match-account-1001".to_string()),
            agent_name: Some("捷风".to_string()),
            map_name: Some("天枢之阙".to_string()),
            game_mode: Some("竞技模式".to_string()),
            limit: Some(1),
            ..ReviewQueueQuery::default()
        };
        let first = list_review_clip_page(&connection, &base_query)
            .expect("first metadata-filtered review page should load");
        let cursor = first
            .next_cursor
            .clone()
            .expect("metadata-filtered page should return a cursor");

        let mut unchanged = base_query.clone();
        unchanged.snapshot_max_clip_id = Some(first.snapshot_max_clip_id);
        unchanged.cursor = Some(cursor.clone());
        let second = list_review_clip_page(&connection, &unchanged)
            .expect("cursor should accept unchanged metadata filters");
        assert_eq!(second.items.len(), 1);

        let mut changed_queries = Vec::new();
        let mut changed_account = base_query.clone();
        changed_account.account_id = Some("match-account-2002".to_string());
        changed_queries.push(("account", changed_account));
        let mut changed_agent = base_query.clone();
        changed_agent.agent_name = Some("贤者".to_string());
        changed_queries.push(("agent", changed_agent));
        let mut changed_map = base_query.clone();
        changed_map.map_name = Some("双塔迷城".to_string());
        changed_queries.push(("map", changed_map));
        let mut changed_mode = base_query;
        changed_mode.game_mode = Some("极速模式".to_string());
        changed_queries.push(("game mode", changed_mode));

        for (label, mut changed_query) in changed_queries {
            changed_query.snapshot_max_clip_id = Some(first.snapshot_max_clip_id);
            changed_query.cursor = Some(cursor.clone());
            let error = list_review_clip_page(&connection, &changed_query)
                .expect_err("cursor must reject changed metadata filters");
            assert!(
                error.contains("cursor"),
                "changing {label} should invalidate the cursor: {error}"
            );
        }
    }

    #[test]
    fn queue_rejects_malformed_cursor_and_invalid_bounds() {
        let connection = test_connection();
        let malformed = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                cursor: Some("not-a-valid-cursor".to_string()),
                ..ReviewQueueQuery::default()
            },
        )
        .expect_err("malformed cursor should fail");
        assert!(malformed.contains("cursor"));

        let invalid_dates = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                recorded_from: Some(20),
                recorded_to: Some(10),
                ..ReviewQueueQuery::default()
            },
        )
        .expect_err("inverted review dates should fail");
        assert!(invalid_dates.contains("recorded-from"));

        let invalid_limit = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                limit: Some(0),
                ..ReviewQueueQuery::default()
            },
        )
        .expect_err("zero review limit should fail");
        assert!(invalid_limit.contains("limit"));

        let too_many_ids = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                source_dir_ids: Some((1..=MAX_REVIEW_FILTER_IDS as i64 + 1).collect()),
                ..ReviewQueueQuery::default()
            },
        )
        .expect_err("oversized review filters should fail before SQL execution");
        assert!(too_many_ids.contains("at most"));
    }

    #[test]
    fn large_filter_sets_use_two_json_parameters_instead_of_sql_bind_expansion() {
        let connection = test_connection();
        let source = seed_source(&connection, "json-filter-source");
        let clip_id = seed_clip(
            &connection,
            &source,
            "json-filter-clip",
            Some("100"),
            None,
            "1",
        );
        let tag = create_tag(&connection, "json-filter-tag", None).expect("tag should seed");
        assign_tag_to_clip(&connection, clip_id, tag.id).expect("tag should attach");

        let source_ids = std::iter::once(source.id)
            .chain(10_000..11_200)
            .collect::<Vec<_>>();
        let tag_ids = std::iter::once(tag.id)
            .chain(20_000..21_200)
            .collect::<Vec<_>>();
        let query = ReviewQueueQuery {
            source_dir_ids: Some(source_ids),
            tag_ids: Some(tag_ids),
            limit: Some(3),
            ..ReviewQueueQuery::default()
        };
        let filter = build_review_filter(&connection, &query, clip_id, None, None)
            .expect("large JSON-backed filters should build");
        assert_eq!(filter.params.len(), 3, "snapshot plus two JSON parameters");
        assert_eq!(filter.where_sql.matches("json_each(?)").count(), 2);

        let page = list_review_clip_page(&connection, &query)
            .expect("large JSON-backed filters should execute");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, clip_id);
    }

    #[test]
    fn ten_thousand_clip_queue_keeps_the_materialized_page_bounded() {
        let mut connection = test_connection();
        let source = seed_source(&connection, "ten-thousand-review-clips");
        let transaction = connection
            .transaction()
            .expect("large review fixture transaction should start");
        for index in 0..10_000 {
            seed_clip(
                &transaction,
                &source,
                &format!("clip-{index:05}"),
                Some(&(10_000 - index).to_string()),
                None,
                "1",
            );
        }
        transaction
            .commit()
            .expect("large review fixture should commit");

        let page = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                limit: Some(3),
                ..ReviewQueueQuery::default()
            },
        )
        .expect("large review queue should load one bounded page");
        assert_eq!(page.candidate_count, 10_000);
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.limit, 3);
        assert!(page.has_more);
        assert!(page.next_cursor.is_some());
    }

    #[test]
    fn review_contract_uses_camel_case_command_payload_fields() {
        let query = serde_json::from_value::<ReviewQueueQuery>(serde_json::json!({
            "sourceDirIds": [2, 1],
            "tagIds": [4, 3],
            "accountId": "match-account-1001",
            "agentName": "捷风",
            "mapName": "天枢之阙",
            "gameMode": "竞技模式",
            "recordedFrom": 10,
            "recordedTo": 20,
            "snapshotMaxClipId": 9,
            "limit": 1
        }))
        .expect("camel-case review query should deserialize");
        assert_eq!(query.source_dir_ids, Some(vec![2, 1]));
        assert_eq!(query.account_id.as_deref(), Some("match-account-1001"));
        assert_eq!(query.agent_name.as_deref(), Some("捷风"));
        assert_eq!(query.map_name.as_deref(), Some("天枢之阙"));
        assert_eq!(query.game_mode.as_deref(), Some("竞技模式"));
        assert_eq!(query.snapshot_max_clip_id, Some(9));
        let query_json = serde_json::to_value(&query).expect("review query should serialize");
        assert_eq!(query_json["accountId"], "match-account-1001");
        assert_eq!(query_json["agentName"], "捷风");
        assert_eq!(query_json["mapName"], "天枢之阙");
        assert_eq!(query_json["gameMode"], "竞技模式");
        assert!(query_json.get("account_id").is_none());

        let connection = test_connection();
        let page = list_review_clip_page(
            &connection,
            &ReviewQueueQuery {
                limit: Some(1),
                ..ReviewQueueQuery::default()
            },
        )
        .expect("empty review page should load");
        let page_json = serde_json::to_value(page).expect("review page should serialize");
        assert_eq!(page_json["snapshotMaxClipId"], 0);
        assert_eq!(page_json["candidateCount"], 0);
        assert_eq!(page_json["nextCursor"], serde_json::Value::Null);

        let mutation_json = serde_json::to_value(ClipReviewMutationResult {
            before: ClipReviewState {
                clip_id: 1,
                review_decision: ReviewDecision::Unreviewed,
                reviewed_at: None,
                favorite: false,
            },
            after: ClipReviewState {
                clip_id: 1,
                review_decision: ReviewDecision::Liked,
                reviewed_at: Some("2026-08-09T00:00:00Z".to_string()),
                favorite: true,
            },
            changed: true,
        })
        .expect("review mutation should serialize");
        assert_eq!(mutation_json["after"]["clipId"], 1);
        assert_eq!(mutation_json["after"]["reviewDecision"], "liked");
        assert_eq!(mutation_json["after"]["favorite"], true);
    }

    #[test]
    fn mutations_are_idempotent_preserve_fields_and_restore_exact_state() {
        let connection = test_connection();
        let source = seed_source(&connection, "mutation-source");
        let clip_id = seed_clip(&connection, &source, "mutation", Some("100"), None, "1");
        let tag = create_tag(&connection, "preserved-tag", None).expect("tag should seed");
        assign_tag_to_clip(&connection, clip_id, tag.id).expect("tag should attach");
        connection
            .execute(
                "UPDATE clips SET note = 'preserved note' WHERE id = ?1",
                [clip_id],
            )
            .expect("note should seed");

        let liked = set_clip_review_decision(&connection, clip_id, ReviewDecision::Liked)
            .expect("liked decision should persist");
        assert!(liked.changed);
        assert_eq!(liked.before.review_decision, ReviewDecision::Unreviewed);
        assert!(!liked.before.favorite);
        assert_eq!(liked.after.review_decision, ReviewDecision::Liked);
        assert!(liked.after.favorite);
        assert!(liked.after.reviewed_at.is_some());

        let repeated = set_clip_review_decision(&connection, clip_id, ReviewDecision::Liked)
            .expect("repeated liked decision should succeed");
        assert!(!repeated.changed);
        assert_eq!(repeated.before, liked.after);
        assert_eq!(repeated.after.reviewed_at, liked.after.reviewed_at);

        let disliked = set_clip_review_decision(&connection, clip_id, ReviewDecision::Disliked)
            .expect("disliked decision should persist");
        assert!(disliked.after.favorite, "disliked must preserve favorite");
        let restored = restore_clip_review_state(&connection, &disliked.after, &disliked.before)
            .expect("undo should restore the exact previous state");
        assert_eq!(restored.after, liked.after);

        let current = set_clip_review_decision(&connection, clip_id, ReviewDecision::Disliked)
            .expect("new decision should persist");
        let stale = restore_clip_review_state(&connection, &liked.after, &liked.before)
            .expect_err("stale undo must not overwrite a newer decision");
        assert!(stale.contains("stale undo"));
        assert_eq!(
            find_clip_review_state(&connection, clip_id).unwrap(),
            current.after
        );

        let reset =
            reset_clip_review_decision(&connection, clip_id).expect("review decision should reset");
        assert_eq!(reset.after.review_decision, ReviewDecision::Unreviewed);
        assert_eq!(reset.after.reviewed_at, None);
        assert!(reset.after.favorite, "reset must preserve favorite");

        let preserved: (String, String, i64) = connection
            .query_row(
                "
                SELECT note, file_status,
                    (SELECT COUNT(*) FROM clip_tags WHERE clip_id = clips.id)
                FROM clips
                WHERE id = ?1
                ",
                [clip_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("preserved fields should load");
        assert_eq!(
            preserved,
            ("preserved note".to_string(), "available".to_string(), 1)
        );
    }

    #[test]
    fn failed_review_write_rolls_back_favorite_and_decision() {
        let connection = test_connection();
        let source = seed_source(&connection, "failure-source");
        let clip_id = seed_clip(&connection, &source, "failure", Some("100"), None, "1");
        let before = find_clip_review_state(&connection, clip_id).expect("state should load");
        connection
            .execute_batch(
                "
                CREATE TRIGGER fail_review_update
                BEFORE UPDATE OF review_decision ON clips
                BEGIN
                    SELECT RAISE(ABORT, 'forced review failure');
                END;
                ",
            )
            .expect("failure trigger should install");

        let error = set_clip_review_decision(&connection, clip_id, ReviewDecision::Liked)
            .expect_err("forced write failure should surface");
        assert!(error.contains("forced review failure"));
        assert_eq!(
            find_clip_review_state(&connection, clip_id).unwrap(),
            before
        );
    }
}
