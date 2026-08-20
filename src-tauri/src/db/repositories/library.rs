//! Read-only library queries, facets, pagination, and full clip hydration.

use std::{cmp::Ordering, collections::HashMap};

use rusqlite::{params, params_from_iter, types::Value, Connection, Row};

use super::super::{
    normalize_optional, readable_error, source_openid, AccountIdentitySource, Clip, ClipEvent,
    ClipListQuery, ClipPage, ClipSort, ClipSummary, DbResult, FavoriteFilter, HighlightFilter,
    LibraryAccountFacet, LibraryFacetValue, LibraryFacets, LibrarySourceFacet, LibraryTagFacet,
    DEFAULT_CLIP_PAGE_LIMIT, MAX_CLIP_PAGE_LIMIT,
};

pub fn list_clip_events_for_clip(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<Vec<ClipEvent>> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                clip_events.id,
                clip_events.clip_id,
                clip_events.segment_id,
                clip_segments.segment_key,
                clip_events.event_key,
                clip_events.event_type,
                clip_events.video_time_ms,
                clip_events.event_time,
                clip_events.round_id,
                clip_events.player_name,
                clip_events.agent_name,
                clip_events.weapon_name,
                clip_events.killer_name,
                clip_events.killed_name,
                clip_events.killer_is_me,
                clip_events.killed_is_me,
                clip_events.raw_json,
                clip_events.created_at
            FROM clip_events
            LEFT JOIN clip_segments ON clip_segments.id = clip_events.segment_id
            WHERE clip_events.clip_id = ?1
            ORDER BY COALESCE(clip_events.video_time_ms, 9223372036854775807), clip_events.id
            ",
        )
        .map_err(|error| readable_error("preparing clip event list", error))?;
    let rows = statement
        .query_map(params![clip_id], map_clip_event)
        .map_err(|error| readable_error("querying clip events", error))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading clip events", error))
}

/// Legacy all-at-once clip contract retained until the production UI migrates to
/// [`list_clip_page`]. It deliberately preserves full event and tag-id hydration.
pub fn list_clips(connection: &Connection) -> DbResult<Vec<Clip>> {
    let mut clips = {
        let mut statement = connection
            .prepare(CLIP_SELECT_SQL)
            .map_err(|error| readable_error("preparing clip list query", error))?;

        let clips = statement
            .query_map([], map_clip)
            .map_err(|error| readable_error("querying clip list", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading clip list", error))?;
        clips
    };
    attach_clip_events(connection, &mut clips)?;

    Ok(clips)
}

pub fn list_clip_page(connection: &Connection, query: &ClipListQuery) -> DbResult<ClipPage> {
    let (offset, limit) = validate_clip_pagination(query)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting clip page snapshot", error))?;
    let filter = build_clip_list_filter(&transaction, query)?;
    let count_sql = format!("SELECT COUNT(*) {CLIP_LIST_FROM_SQL} {}", filter.where_sql);
    let total_count = transaction
        .query_row(&count_sql, params_from_iter(filter.params.iter()), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| readable_error("counting filtered clips", error))?;

    let order_by = clip_list_order_by(query.sort_by.unwrap_or_default());
    let page_sql = format!(
        "{CLIP_SUMMARY_SELECT_SQL} {CLIP_LIST_FROM_SQL} {} ORDER BY {order_by} LIMIT ? OFFSET ?",
        filter.where_sql
    );
    let mut page_params = filter.params;
    page_params.push(Value::Integer(limit));
    page_params.push(Value::Integer(offset));
    let mut items = {
        let mut statement = transaction
            .prepare(&page_sql)
            .map_err(|error| readable_error("preparing clip page query", error))?;
        let items = statement
            .query_map(params_from_iter(page_params.iter()), map_clip_summary)
            .map_err(|error| readable_error("querying clip page", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading clip page", error))?;
        items
    };
    attach_clip_summary_tags(&transaction, &mut items)?;
    transaction
        .commit()
        .map_err(|error| readable_error("finishing clip page snapshot", error))?;

    debug_assert!(items.len() <= limit as usize);
    let consumed = offset.saturating_add(items.len() as i64);
    let has_more = consumed < total_count;

    Ok(ClipPage {
        items,
        offset,
        limit,
        total_count,
        has_more,
        next_offset: has_more.then_some(consumed),
    })
}

#[derive(Debug)]
struct LibraryFacetOverview {
    total_count: i64,
    active_count: i64,
    favorite_count: i64,
    active_favorite_count: i64,
    trashed_count: i64,
    tagged_count: i64,
    active_tagged_count: i64,
    total_size_bytes: i64,
    active_size_bytes: i64,
    size_bytes_min: Option<i64>,
    size_bytes_max: Option<i64>,
    recent_count: i64,
    recorded_at_min: Option<i64>,
    recorded_at_max: Option<i64>,
    modified_at_min: Option<i64>,
    modified_at_max: Option<i64>,
}

#[derive(Debug)]
struct SourceFacetAggregate {
    source_dir_id: i64,
    source_name: String,
    source_path: String,
    count: i64,
    active_count: i64,
}

/// Returns exact facets for the entire clip index in one read snapshot.
///
/// The implementation deliberately uses a fixed seven aggregate statements. It never selects
/// event tables, raw metadata JSON, or a full [`Clip`] / [`ClipSummary`] row. Facet arrays grow
/// only with values observed on clips; the separate tag catalog supplies zero-use tag metadata.
pub fn get_library_facets(connection: &Connection) -> DbResult<LibraryFacets> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting library facet snapshot", error))?;

    let overview_sql = format!(
        "
        SELECT
            COUNT(*),
            COALESCE(SUM(CASE WHEN clips.file_status <> 'trashed' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN clips.is_favorite = 1 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE
                WHEN clips.is_favorite = 1 AND clips.file_status <> 'trashed' THEN 1 ELSE 0
            END), 0),
            COALESCE(SUM(CASE WHEN clips.file_status = 'trashed' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN EXISTS (
                SELECT 1 FROM clip_tags overview_clip_tags
                WHERE overview_clip_tags.clip_id = clips.id
            ) THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE
                WHEN clips.file_status <> 'trashed' AND EXISTS (
                    SELECT 1 FROM clip_tags active_overview_clip_tags
                    WHERE active_overview_clip_tags.clip_id = clips.id
                ) THEN 1 ELSE 0
            END), 0),
            COALESCE(SUM(clips.size_bytes), 0),
            COALESCE(SUM(CASE
                WHEN clips.file_status <> 'trashed' THEN clips.size_bytes ELSE 0
            END), 0),
            COALESCE(SUM(CASE
                WHEN clips.file_status <> 'trashed'
                  AND {CLIP_MODIFIED_UNIX_SQL} >= unixepoch(
                      'now', 'localtime', 'start of day', 'utc'
                  )
                  AND {CLIP_MODIFIED_UNIX_SQL} < unixepoch(
                      'now', 'localtime', 'start of day', '+1 day', 'utc'
                  )
                THEN 1 ELSE 0
            END), 0),
            MIN({CLIP_RECORDED_UNIX_SQL}),
            MAX({CLIP_RECORDED_UNIX_SQL}),
            MIN(NULLIF({CLIP_MODIFIED_UNIX_SQL}, 0)),
            MAX(NULLIF({CLIP_MODIFIED_UNIX_SQL}, 0)),
            MIN(clips.size_bytes),
            MAX(clips.size_bytes)
        FROM clips
        "
    );
    let overview = transaction
        .query_row(&overview_sql, [], |row| {
            Ok(LibraryFacetOverview {
                total_count: row.get(0)?,
                active_count: row.get(1)?,
                favorite_count: row.get(2)?,
                active_favorite_count: row.get(3)?,
                trashed_count: row.get(4)?,
                tagged_count: row.get(5)?,
                active_tagged_count: row.get(6)?,
                total_size_bytes: row.get(7)?,
                active_size_bytes: row.get(8)?,
                recent_count: row.get(9)?,
                recorded_at_min: row.get(10)?,
                recorded_at_max: row.get(11)?,
                modified_at_min: row.get(12)?,
                modified_at_max: row.get(13)?,
                size_bytes_min: row.get(14)?,
                size_bytes_max: row.get(15)?,
            })
        })
        .map_err(|error| readable_error("querying library facet overview", error))?;

    let (mut file_statuses, mut metadata_statuses) = {
        let mut statement = transaction
            .prepare(
                "
                SELECT 'file', clips.file_status, COUNT(*),
                    COALESCE(SUM(CASE
                        WHEN clips.file_status <> 'trashed' THEN 1 ELSE 0
                    END), 0)
                FROM clips
                GROUP BY clips.file_status

                UNION ALL

                SELECT 'metadata',
                    COALESCE(NULLIF(TRIM(clip_metadata.metadata_status), ''), 'not_found'),
                    COUNT(*),
                    COALESCE(SUM(CASE
                        WHEN clips.file_status <> 'trashed' THEN 1 ELSE 0
                    END), 0)
                FROM clips
                LEFT JOIN clip_metadata ON clip_metadata.clip_id = clips.id
                GROUP BY COALESCE(NULLIF(TRIM(clip_metadata.metadata_status), ''), 'not_found')
                ",
            )
            .map_err(|error| readable_error("preparing library status facets", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    LibraryFacetValue {
                        value: row.get(1)?,
                        count: row.get(2)?,
                        active_count: row.get(3)?,
                    },
                ))
            })
            .map_err(|error| readable_error("querying library status facets", error))?;
        let mut file_statuses = Vec::new();
        let mut metadata_statuses = Vec::new();
        for row in rows {
            let (kind, facet) =
                row.map_err(|error| readable_error("reading library status facets", error))?;
            if kind == "file" {
                file_statuses.push(facet);
            } else {
                metadata_statuses.push(facet);
            }
        }
        (file_statuses, metadata_statuses)
    };
    sort_value_facets(&mut file_statuses);
    sort_value_facets(&mut metadata_statuses);

    let source_aggregates = {
        let mut statement = transaction
            .prepare(
                "
                SELECT
                    source_dirs.id,
                    source_dirs.name,
                    source_dirs.path,
                    COUNT(clips.id),
                    COALESCE(SUM(CASE
                        WHEN clips.file_status <> 'trashed' THEN 1 ELSE 0
                    END), 0)
                FROM source_dirs
                JOIN clips ON clips.source_dir_id = source_dirs.id
                GROUP BY source_dirs.id, source_dirs.name, source_dirs.path
                ORDER BY COUNT(clips.id) DESC, source_dirs.id ASC
                ",
            )
            .map_err(|error| readable_error("preparing library source facets", error))?;
        let sources = statement
            .query_map([], |row| {
                Ok(SourceFacetAggregate {
                    source_dir_id: row.get(0)?,
                    source_name: row.get(1)?,
                    source_path: row.get(2)?,
                    count: row.get(3)?,
                    active_count: row.get(4)?,
                })
            })
            .map_err(|error| readable_error("querying library source facets", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading library source facets", error))?;
        sources
    };
    let source_dirs = source_aggregates
        .iter()
        .map(|source| LibrarySourceFacet {
            source_dir_id: source.source_dir_id,
            count: source.count,
            active_count: source.active_count,
        })
        .collect::<Vec<_>>();

    let accounts = {
        let identity_expression = account_facet_identity_expression(&source_aggregates);
        let sql = format!(
            "
            WITH account_rows AS (
                SELECT
                    clips.id,
                    clips.file_status,
                    {CLIP_MODIFIED_UNIX_SQL} AS modified_unix,
                    {identity_expression} AS account_identity_key,
                    COALESCE(
                        NULLIF(TRIM(clip_metadata.account_name), ''),
                        NULLIF(TRIM(clip_metadata.player_name), '')
                    ) AS observed_display_name,
                    source_dirs.name AS source_name
                FROM clips
                JOIN source_dirs ON source_dirs.id = clips.source_dir_id
                LEFT JOIN clip_metadata ON clip_metadata.clip_id = clips.id
                LEFT JOIN matches ON matches.game_id = clip_metadata.match_id
            ),
            ranked_names AS (
                SELECT
                    account_rows.*,
                    ROW_NUMBER() OVER (
                        PARTITION BY account_identity_key
                        ORDER BY
                            CASE WHEN observed_display_name IS NULL THEN 1 ELSE 0 END,
                            modified_unix DESC,
                            id DESC
                    ) AS display_rank
                FROM account_rows
            ),
            account_counts AS (
                SELECT
                    account_identity_key,
                    COUNT(*) AS clip_count,
                    COALESCE(SUM(CASE
                        WHEN file_status <> 'trashed' THEN 1 ELSE 0
                    END), 0) AS active_clip_count
                FROM account_rows
                GROUP BY account_identity_key
            )
            SELECT
                account_counts.account_identity_key,
                COALESCE(
                    ranked_names.observed_display_name,
                    CASE
                        WHEN account_counts.account_identity_key LIKE 'match-account-%'
                            THEN '账号 ' || SUBSTR(account_counts.account_identity_key, 15)
                        ELSE ranked_names.source_name
                    END
                ) AS account_display_name,
                account_counts.clip_count,
                account_counts.active_clip_count
            FROM account_counts
            JOIN ranked_names
              ON ranked_names.account_identity_key = account_counts.account_identity_key
             AND ranked_names.display_rank = 1
            ORDER BY
                account_counts.clip_count DESC,
                account_display_name COLLATE NOCASE ASC,
                account_counts.account_identity_key ASC
            "
        );
        let mut statement = transaction
            .prepare(&sql)
            .map_err(|error| readable_error("preparing library account facets", error))?;
        let accounts = statement
            .query_map([], |row| {
                Ok(LibraryAccountFacet {
                    account_identity_key: row.get(0)?,
                    account_display_name: row.get(1)?,
                    count: row.get(2)?,
                    active_count: row.get(3)?,
                })
            })
            .map_err(|error| readable_error("querying library account facets", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading library account facets", error))?;
        accounts
    };

    let (mut agents, mut maps, mut game_modes) = {
        let mut statement = transaction
            .prepare(
                "
                SELECT
                    'agent',
                    TRIM(clip_metadata.agent_name),
                    NULL,
                    NULL,
                    COUNT(*),
                    COALESCE(SUM(CASE
                        WHEN clips.file_status <> 'trashed' THEN 1 ELSE 0
                    END), 0)
                FROM clips
                JOIN clip_metadata ON clip_metadata.clip_id = clips.id
                WHERE NULLIF(TRIM(clip_metadata.agent_name), '') IS NOT NULL
                GROUP BY TRIM(clip_metadata.agent_name)

                UNION ALL

                SELECT
                    'map',
                    NULLIF(TRIM(clip_metadata.map_name), ''),
                    NULLIF(TRIM(matches.map_id), ''),
                    clip_metadata.metadata_source,
                    COUNT(*),
                    COALESCE(SUM(CASE
                        WHEN clips.file_status <> 'trashed' THEN 1 ELSE 0
                    END), 0)
                FROM clips
                LEFT JOIN clip_metadata ON clip_metadata.clip_id = clips.id
                LEFT JOIN matches ON matches.game_id = clip_metadata.match_id
                WHERE NULLIF(TRIM(clip_metadata.map_name), '') IS NOT NULL
                   OR NULLIF(TRIM(matches.map_id), '') IS NOT NULL
                GROUP BY
                    NULLIF(TRIM(clip_metadata.map_name), ''),
                    NULLIF(TRIM(matches.map_id), ''),
                    clip_metadata.metadata_source

                UNION ALL

                SELECT
                    'game-mode',
                    TRIM(clip_metadata.game_mode),
                    NULL,
                    NULL,
                    COUNT(*),
                    COALESCE(SUM(CASE
                        WHEN clips.file_status <> 'trashed' THEN 1 ELSE 0
                    END), 0)
                FROM clips
                JOIN clip_metadata ON clip_metadata.clip_id = clips.id
                WHERE NULLIF(TRIM(clip_metadata.game_mode), '') IS NOT NULL
                GROUP BY TRIM(clip_metadata.game_mode)
                ",
            )
            .map_err(|error| readable_error("preparing library dimension facets", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|error| readable_error("querying library dimension facets", error))?;
        let mut agent_counts = HashMap::<String, (i64, i64)>::new();
        let mut map_counts = HashMap::<String, (i64, i64)>::new();
        let mut game_mode_counts = HashMap::<String, (i64, i64)>::new();
        for row in rows {
            let (kind, value, auxiliary, metadata_source, count, active_count) =
                row.map_err(|error| readable_error("reading library dimension facets", error))?;
            let resolved = match kind.as_str() {
                "agent" => value
                    .as_deref()
                    .and_then(crate::display_names::localized_agent_name_for_display),
                "map" => resolved_clip_map_name(
                    value.as_deref(),
                    auxiliary.as_deref(),
                    metadata_source.as_deref(),
                ),
                _ => value.and_then(|value| normalize_optional(Some(&value)).map(str::to_owned)),
            };
            let Some(resolved) = resolved else {
                continue;
            };
            let counts = match kind.as_str() {
                "agent" => &mut agent_counts,
                "map" => &mut map_counts,
                _ => &mut game_mode_counts,
            };
            let entry = counts.entry(resolved).or_insert((0, 0));
            entry.0 += count;
            entry.1 += active_count;
        }
        (
            facet_values_from_counts(agent_counts),
            facet_values_from_counts(map_counts),
            facet_values_from_counts(game_mode_counts),
        )
    };
    sort_value_facets(&mut agents);
    sort_value_facets(&mut maps);
    sort_value_facets(&mut game_modes);

    let mut kill_types = {
        let triple = clip_highlight_condition(HighlightFilter::Triple)
            .expect("triple filter must have a condition");
        let quad = clip_highlight_condition(HighlightFilter::Quad)
            .expect("quad filter must have a condition");
        let five = clip_highlight_condition(HighlightFilter::Five)
            .expect("five-kill filter must have a condition");
        let six = clip_highlight_condition(HighlightFilter::Six)
            .expect("six-kill filter must have a condition");
        let compilation = clip_highlight_condition(HighlightFilter::KillCompilation)
            .expect("kill compilation filter must have a condition");
        let death = clip_highlight_condition(HighlightFilter::Death)
            .expect("death filter must have a condition");
        let sql = format!(
            "
            SELECT
                COALESCE(SUM(CASE WHEN {triple} THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE
                    WHEN clips.file_status <> 'trashed' AND {triple} THEN 1 ELSE 0
                END), 0),
                COALESCE(SUM(CASE WHEN {quad} THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE
                    WHEN clips.file_status <> 'trashed' AND {quad} THEN 1 ELSE 0
                END), 0),
                COALESCE(SUM(CASE WHEN {five} THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE
                    WHEN clips.file_status <> 'trashed' AND {five} THEN 1 ELSE 0
                END), 0),
                COALESCE(SUM(CASE WHEN {six} THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE
                    WHEN clips.file_status <> 'trashed' AND {six} THEN 1 ELSE 0
                END), 0),
                COALESCE(SUM(CASE WHEN {compilation} THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE
                    WHEN clips.file_status <> 'trashed' AND {compilation} THEN 1 ELSE 0
                END), 0),
                COALESCE(SUM(CASE WHEN {death} THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE
                    WHEN clips.file_status <> 'trashed' AND {death} THEN 1 ELSE 0
                END), 0)
            {CLIP_LIST_FROM_SQL}
            "
        );
        transaction
            .query_row(&sql, [], |row| {
                Ok(vec![
                    LibraryFacetValue {
                        value: "triple".to_string(),
                        count: row.get(0)?,
                        active_count: row.get(1)?,
                    },
                    LibraryFacetValue {
                        value: "quad".to_string(),
                        count: row.get(2)?,
                        active_count: row.get(3)?,
                    },
                    LibraryFacetValue {
                        value: "five".to_string(),
                        count: row.get(4)?,
                        active_count: row.get(5)?,
                    },
                    LibraryFacetValue {
                        value: "six".to_string(),
                        count: row.get(6)?,
                        active_count: row.get(7)?,
                    },
                    LibraryFacetValue {
                        value: "kill-compilation".to_string(),
                        count: row.get(8)?,
                        active_count: row.get(9)?,
                    },
                    LibraryFacetValue {
                        value: "death".to_string(),
                        count: row.get(10)?,
                        active_count: row.get(11)?,
                    },
                ])
            })
            .map_err(|error| readable_error("querying library video-type facets", error))?
    };
    kill_types.retain(|facet| facet.count > 0);

    let mut tags = {
        let mut statement = transaction
            .prepare(
                "
                SELECT
                    tags.id,
                    tags.name,
                    tags.color,
                    COUNT(clips.id),
                    COALESCE(SUM(CASE
                        WHEN clips.file_status <> 'trashed' THEN 1 ELSE 0
                    END), 0)
                FROM tags
                LEFT JOIN clip_tags ON clip_tags.tag_id = tags.id
                LEFT JOIN clips ON clips.id = clip_tags.clip_id
                GROUP BY tags.id, tags.name, tags.color
                HAVING COUNT(clips.id) > 0
                ",
            )
            .map_err(|error| readable_error("preparing library tag facets", error))?;
        let tag_facets = statement
            .query_map([], |row| {
                Ok(LibraryTagFacet {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    count: row.get(3)?,
                    active_count: row.get(4)?,
                })
            })
            .map_err(|error| readable_error("querying library tag facets", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading library tag facets", error))?;
        tag_facets
    };
    tags.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| compare_facet_names(&left.name, &right.name))
            .then_with(|| left.id.cmp(&right.id))
    });

    transaction
        .commit()
        .map_err(|error| readable_error("finishing library facet snapshot", error))?;

    Ok(LibraryFacets {
        total_count: overview.total_count,
        active_count: overview.active_count,
        favorite_count: overview.favorite_count,
        active_favorite_count: overview.active_favorite_count,
        trashed_count: overview.trashed_count,
        tagged_count: overview.tagged_count,
        active_tagged_count: overview.active_tagged_count,
        total_size_bytes: overview.total_size_bytes,
        active_size_bytes: overview.active_size_bytes,
        size_bytes_min: overview.size_bytes_min,
        size_bytes_max: overview.size_bytes_max,
        recent_count: overview.recent_count,
        recorded_at_min: overview.recorded_at_min,
        recorded_at_max: overview.recorded_at_max,
        modified_at_min: overview.modified_at_min,
        modified_at_max: overview.modified_at_max,
        file_statuses,
        metadata_statuses,
        accounts,
        source_dirs,
        agents,
        maps,
        game_modes,
        kill_types,
        tags,
    })
}

fn account_facet_identity_expression(sources: &[SourceFacetAggregate]) -> String {
    let source_cases = sources
        .iter()
        .filter_map(|source| {
            source_openid(&source.source_name, &source.source_path).map(|openid| {
                format!(
                    "WHEN {} THEN 'match-account-{}'",
                    source.source_dir_id, openid
                )
            })
        })
        .collect::<Vec<_>>();
    let source_identity = if source_cases.is_empty() {
        "('source-' || clips.source_dir_id)".to_string()
    } else {
        format!(
            "(CASE clips.source_dir_id {} ELSE 'source-' || clips.source_dir_id END)",
            source_cases.join(" ")
        )
    };

    format!(
        "(CASE
            WHEN NULLIF(TRIM(matches.account_id), '') IS NOT NULL
                THEN 'match-account-' || TRIM(matches.account_id)
            ELSE {source_identity}
        END)"
    )
}

fn facet_values_from_counts(counts: HashMap<String, (i64, i64)>) -> Vec<LibraryFacetValue> {
    counts
        .into_iter()
        .map(|(value, (count, active_count))| LibraryFacetValue {
            value,
            count,
            active_count,
        })
        .collect()
}

fn sort_value_facets(facets: &mut [LibraryFacetValue]) {
    facets.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| compare_facet_names(&left.value, &right.value))
            .then_with(|| left.value.cmp(&right.value))
    });
}

fn compare_facet_names(left: &str, right: &str) -> Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

const CLIP_MODIFIED_UNIX_SQL: &str = "COALESCE(
    CASE
        WHEN NULLIF(TRIM(clips.modified_at), '') IS NULL THEN NULL
        WHEN TRIM(clips.modified_at) NOT GLOB '*[^0-9]*'
            THEN CAST(clips.modified_at AS INTEGER)
        ELSE unixepoch(clips.modified_at)
    END,
    CASE
        WHEN NULLIF(TRIM(clips.recorded_at), '') IS NULL THEN NULL
        WHEN TRIM(clips.recorded_at) NOT GLOB '*[^0-9]*'
            THEN CAST(clips.recorded_at AS INTEGER)
        ELSE unixepoch(clips.recorded_at)
    END,
    0
)";

const CLIP_RECORDED_UNIX_SQL: &str = "CASE
    WHEN NULLIF(TRIM(clips.recorded_at), '') IS NULL THEN NULL
    WHEN TRIM(clips.recorded_at) NOT GLOB '*[^0-9]*'
        THEN CAST(clips.recorded_at AS INTEGER)
    ELSE unixepoch(clips.recorded_at)
END";

pub(crate) const CLIP_LIST_FROM_SQL: &str = "
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
";

pub(crate) const CLIP_SUMMARY_SELECT_SQL: &str = "
    SELECT
        clips.id,
        clips.source_dir_id,
        clips.clip_group_id,
        clip_groups.display_name AS clip_group_name,
        clips.file_path,
        clips.file_name,
        clips.size_bytes,
        clips.modified_at,
        clips.duration_ms,
        clips.recorded_at,
        clips.cover_path,
        clips.cover_source,
        clips.file_status,
        clips.is_favorite,
        clip_metadata.account_name,
        clip_metadata.player_name,
        clip_metadata.agent_name,
        clip_metadata.map_name,
        clip_metadata.game_mode,
        COALESCE(clip_metadata.metadata_status, 'not_found') AS metadata_status,
        clip_metadata.match_id,
        matches.account_id AS match_account_id,
        clip_metadata.scoreline,
        clip_metadata.kda,
        matches.agent_avatar_url,
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
        CAST(NULLIF(TRIM(clip_metadata.round_score), '') AS INTEGER) AS round_score,
        clip_metadata.metadata_source,
        matches.map_id AS match_map_id,
        source_dirs.path AS source_dir_path,
        source_dirs.name AS source_dir_name,
        clip_thumbnails.status AS thumbnail_status,
        CASE
            WHEN clip_thumbnails.status = 'ready' THEN clip_thumbnails.revision
            ELSE NULL
        END AS thumbnail_revision,
        source_dirs.source_kind,
        source_dirs.scan_mode,
        source_dirs.scan_root_path,
        clips.source_relative_dir,
        clips.review_decision,
        clips.reviewed_at
";

pub(crate) struct ClipListFilter {
    pub(crate) where_sql: String,
    pub(crate) params: Vec<Value>,
}

fn validate_clip_pagination(query: &ClipListQuery) -> DbResult<(i64, i64)> {
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(DEFAULT_CLIP_PAGE_LIMIT);

    if offset < 0 {
        return Err("clip list offset must be non-negative".to_string());
    }
    if !(1..=MAX_CLIP_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "clip list limit must be between 1 and {MAX_CLIP_PAGE_LIMIT}"
        ));
    }
    validate_clip_filter(query)?;

    Ok((offset, limit))
}

pub(crate) fn validate_clip_filter(query: &ClipListQuery) -> DbResult<()> {
    if query.size_min_bytes.is_some_and(|value| value < 0)
        || query.size_max_bytes.is_some_and(|value| value < 0)
    {
        return Err("clip list size bounds must be non-negative".to_string());
    }
    if matches!(
        (query.size_min_bytes, query.size_max_bytes),
        (Some(minimum), Some(maximum)) if minimum > maximum
    ) {
        return Err("clip list minimum size cannot exceed maximum size".to_string());
    }
    if matches!(
        (query.modified_from, query.modified_to),
        (Some(from), Some(to)) if from > to
    ) {
        return Err("clip list modified-from cannot exceed modified-to".to_string());
    }

    Ok(())
}

pub(crate) fn build_clip_list_filter(
    connection: &Connection,
    query: &ClipListQuery,
) -> DbResult<ClipListFilter> {
    let mut conditions = Vec::new();
    let mut values = Vec::new();

    if let Some(search_term) = normalized_filter_value(query.query.as_deref()) {
        let pattern = escaped_like_pattern(&search_term.to_lowercase());
        let searchable_expressions = [
            "clips.file_name",
            "clips.file_path",
            "source_dirs.name",
            "source_dirs.path",
            "clip_metadata.account_name",
            "clip_metadata.player_name",
            "matches.account_id",
            "('账号 ' || COALESCE(matches.account_id, ''))",
            "(CASE
                WHEN LOWER(SUBSTR(source_dirs.name, 1, 15)) = 'wonderfulvideos'
                  AND SUBSTR(source_dirs.name, 16) <> ''
                  AND SUBSTR(source_dirs.name, 16) NOT GLOB '*[^0-9]*'
                THEN '账号 ' || SUBSTR(source_dirs.name, 16)
                ELSE ''
            END)",
            "clip_metadata.agent_name",
            "clip_metadata.map_name",
            "clip_metadata.game_mode",
            "clip_metadata.scoreline",
            "clip_metadata.kda",
            "clips.note",
            "clip_metadata.extracted_text",
        ];
        let mut search_conditions = searchable_expressions
            .iter()
            .map(|expression| format!("LOWER(COALESCE({expression}, '')) LIKE ? ESCAPE '\\'"))
            .collect::<Vec<_>>();
        values.extend(std::iter::repeat_n(
            Value::Text(pattern.clone()),
            searchable_expressions.len(),
        ));
        let source_account_ids = source_dir_ids_matching_account_display(connection, search_term)?;
        if !source_account_ids.is_empty() {
            let placeholders = std::iter::repeat_n("?", source_account_ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            search_conditions.push(format!("clips.source_dir_id IN ({placeholders})"));
            values.extend(source_account_ids.into_iter().map(Value::Integer));
        }
        search_conditions.push(
            "EXISTS (
                SELECT 1
                FROM clip_tags search_clip_tags
                JOIN tags search_tags ON search_tags.id = search_clip_tags.tag_id
                WHERE search_clip_tags.clip_id = clips.id
                  AND LOWER(search_tags.name) LIKE ? ESCAPE '\\'
            )"
            .to_string(),
        );
        values.push(Value::Text(pattern));
        conditions.push(format!("({})", search_conditions.join(" OR ")));
    }

    append_account_filter(
        connection,
        query.account_id.as_deref(),
        &mut conditions,
        &mut values,
    )?;

    if let Some(source_dir_id) = query.source_dir_id {
        conditions.push("clips.source_dir_id = ?".to_string());
        values.push(Value::Integer(source_dir_id));
    }
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
    if let Some(tag_id) = query.tag_id {
        conditions.push(
            "EXISTS (
                SELECT 1
                FROM clip_tags filtered_clip_tags
                WHERE filtered_clip_tags.clip_id = clips.id
                  AND filtered_clip_tags.tag_id = ?
            )"
            .to_string(),
        );
        values.push(Value::Integer(tag_id));
    }

    if let Some(highlight_condition) =
        clip_highlight_condition(query.highlight_filter.unwrap_or_default())
    {
        conditions.push(highlight_condition.to_string());
    }

    match query.favorite_filter.unwrap_or_default() {
        FavoriteFilter::All => {}
        FavoriteFilter::Favorite => conditions.push("clips.is_favorite = 1".to_string()),
        FavoriteFilter::NotFavorite => conditions.push("clips.is_favorite = 0".to_string()),
    }

    if let Some(review_decision) = query.review_decision {
        conditions.push("clips.review_decision = ?".to_string());
        values.push(Value::Text(review_decision.as_str().to_string()));
    }

    if let Some(file_status) = normalized_filter_value(query.file_status.as_deref()) {
        conditions.push("clips.file_status = ?".to_string());
        values.push(Value::Text(file_status.to_string()));
    } else {
        // This is the current production `all` scope: recycled clips live in an explicit mode.
        conditions.push("clips.file_status <> 'trashed'".to_string());
    }
    if let Some(metadata_status) = normalized_filter_value(query.metadata_status.as_deref()) {
        conditions.push("COALESCE(clip_metadata.metadata_status, 'not_found') = ?".to_string());
        values.push(Value::Text(metadata_status.to_string()));
    }
    if let Some(modified_from) = query.modified_from {
        conditions.push(format!("{CLIP_MODIFIED_UNIX_SQL} >= ?"));
        values.push(Value::Integer(modified_from));
    }
    if let Some(modified_to) = query.modified_to {
        conditions.push(format!("{CLIP_MODIFIED_UNIX_SQL} <= ?"));
        values.push(Value::Integer(modified_to));
    }
    if let Some(size_min_bytes) = query.size_min_bytes {
        conditions.push("clips.size_bytes >= ?".to_string());
        values.push(Value::Integer(size_min_bytes));
    }
    if let Some(size_max_bytes) = query.size_max_bytes {
        conditions.push("clips.size_bytes <= ?".to_string());
        values.push(Value::Integer(size_max_bytes));
    }

    Ok(ClipListFilter {
        where_sql: format!("WHERE {}", conditions.join(" AND ")),
        params: values,
    })
}

fn source_dir_ids_matching_account_display(
    connection: &Connection,
    search_term: &str,
) -> DbResult<Vec<i64>> {
    let search_term = search_term.to_lowercase();
    let mut statement = connection
        .prepare("SELECT id, name, path FROM source_dirs ORDER BY id")
        .map_err(|error| readable_error("preparing source account search", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| readable_error("querying source account search", error))?;
    let mut matching_ids = Vec::new();
    for row in rows {
        let (source_id, source_name, source_path) =
            row.map_err(|error| readable_error("reading source account search", error))?;
        if source_openid(&source_name, &source_path).is_some_and(|openid| {
            format!("账号 {openid}")
                .to_lowercase()
                .contains(&search_term)
        }) {
            matching_ids.push(source_id);
        }
    }
    Ok(matching_ids)
}

pub(crate) fn append_account_filter(
    connection: &Connection,
    account_id: Option<&str>,
    conditions: &mut Vec<String>,
    values: &mut Vec<Value>,
) -> DbResult<()> {
    let Some(account_id) = normalized_filter_value(account_id) else {
        return Ok(());
    };

    if let Some(source_dir_id) = account_id
        .strip_prefix("source-")
        .and_then(|value| value.parse::<i64>().ok())
    {
        conditions.push(
            "clips.source_dir_id = ? AND NULLIF(TRIM(matches.account_id), '') IS NULL".to_string(),
        );
        values.push(Value::Integer(source_dir_id));
        return Ok(());
    }

    let identity_value = account_id
        .strip_prefix("match-account-")
        .unwrap_or(account_id);
    let source_dir_ids = source_dir_ids_for_openid(connection, identity_value)?;
    let mut condition = "(NULLIF(TRIM(matches.account_id), '') = ?".to_string();
    values.push(Value::Text(identity_value.to_string()));
    if !source_dir_ids.is_empty() {
        let placeholders = std::iter::repeat_n("?", source_dir_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        condition.push_str(&format!(
            " OR (NULLIF(TRIM(matches.account_id), '') IS NULL AND clips.source_dir_id IN ({placeholders}))"
        ));
        values.extend(source_dir_ids.into_iter().map(Value::Integer));
    }
    condition.push(')');
    conditions.push(condition);

    Ok(())
}

fn source_dir_ids_for_openid(connection: &Connection, openid: &str) -> DbResult<Vec<i64>> {
    let mut statement = connection
        .prepare("SELECT id, name, path FROM source_dirs ORDER BY id")
        .map_err(|error| readable_error("preparing source identity query", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| readable_error("querying source identities", error))?;
    let mut source_dir_ids = Vec::new();
    for row in rows {
        let (source_dir_id, source_name, source_path) =
            row.map_err(|error| readable_error("reading source identities", error))?;
        if source_openid(&source_name, &source_path).as_deref() == Some(openid) {
            source_dir_ids.push(source_dir_id);
        }
    }
    Ok(source_dir_ids)
}

pub(crate) fn append_exact_text_filter(
    expression: &str,
    selected_value: Option<&str>,
    conditions: &mut Vec<String>,
    values: &mut Vec<Value>,
) {
    if let Some(selected_value) = normalized_filter_value(selected_value) {
        conditions.push(format!("{expression} = ?"));
        values.push(Value::Text(selected_value.to_string()));
    }
}

pub(crate) fn append_agent_filter(
    selected_value: Option<&str>,
    conditions: &mut Vec<String>,
    values: &mut Vec<Value>,
) {
    let Some(selected_value) = normalized_filter_value(selected_value) else {
        return;
    };
    let filter_values = crate::display_names::agent_name_filter_values(selected_value);
    if filter_values.is_empty() {
        append_exact_text_filter(
            "clip_metadata.agent_name",
            Some(selected_value),
            conditions,
            values,
        );
        return;
    }

    let placeholders = std::iter::repeat_n("?", filter_values.len())
        .collect::<Vec<_>>()
        .join(", ");
    conditions.push(format!(
        "LOWER(TRIM(clip_metadata.agent_name)) IN ({placeholders})"
    ));
    values.extend(
        filter_values
            .into_iter()
            .map(|value| Value::Text(value.to_lowercase())),
    );
}

pub(crate) fn append_map_filter(
    connection: &Connection,
    selected_value: Option<&str>,
    conditions: &mut Vec<String>,
    values: &mut Vec<Value>,
) -> DbResult<()> {
    let Some(selected_value) = normalized_filter_value(selected_value) else {
        return Ok(());
    };
    let known_map_ids = known_match_map_ids(connection)?;
    let selected_map_ids = known_map_ids
        .iter()
        .filter(|(_, display_name)| display_name == selected_value)
        .map(|(map_id, _)| map_id.as_str())
        .collect::<Vec<_>>();
    let mut fallback_display_conditions = Vec::new();
    let mut fallback_values = Vec::new();
    if !selected_map_ids.is_empty() {
        let placeholders = std::iter::repeat_n("?", selected_map_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        fallback_display_conditions.push(format!("matches.map_id IN ({placeholders})"));
        fallback_values.extend(
            selected_map_ids
                .into_iter()
                .map(|map_id| Value::Text(map_id.to_string())),
        );
    }
    if known_map_ids.is_empty() {
        fallback_display_conditions.push("clip_metadata.map_name = ?".to_string());
        fallback_values.push(Value::Text(selected_value.to_string()));
    } else {
        let placeholders = std::iter::repeat_n("?", known_map_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        fallback_display_conditions.push(format!(
            "((matches.map_id IS NULL OR matches.map_id NOT IN ({placeholders})) AND clip_metadata.map_name = ?)"
        ));
        fallback_values.extend(
            known_map_ids
                .iter()
                .map(|(map_id, _)| Value::Text(map_id.clone())),
        );
        fallback_values.push(Value::Text(selected_value.to_string()));
    }
    values.push(Value::Text(selected_value.to_string()));
    values.extend(fallback_values);
    conditions.push(format!(
        "(
            (
                clip_metadata.metadata_source IN ('wonderful_db', 'video_export')
                AND clip_metadata.map_name IS NOT NULL
                AND clip_metadata.map_name NOT IN ('幽邃迷境', '迷邃幽境')
                AND clip_metadata.map_name = ?
            )
            OR (
                (
                    clip_metadata.map_name IS NULL
                    OR clip_metadata.map_name IN ('幽邃迷境', '迷邃幽境')
                    OR COALESCE(clip_metadata.metadata_source, '') NOT IN ('wonderful_db', 'video_export')
                )
                AND ({})
            )
        )",
        fallback_display_conditions.join(" OR ")
    ));

    Ok(())
}

fn known_match_map_ids(connection: &Connection) -> DbResult<Vec<(String, String)>> {
    let mut statement = connection
        .prepare(
            "
            SELECT DISTINCT map_id
            FROM matches
            WHERE NULLIF(TRIM(map_id), '') IS NOT NULL
            ORDER BY map_id
            ",
        )
        .map_err(|error| readable_error("preparing match map identity query", error))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| readable_error("querying match map identities", error))?;
    let mut known = Vec::new();
    for row in rows {
        let map_id = row.map_err(|error| readable_error("reading match map identities", error))?;
        if let Some(display_name) = crate::display_names::known_map_name_for_display(&map_id) {
            known.push((map_id, display_name));
        }
    }
    Ok(known)
}

pub(crate) fn normalized_filter_value(value: Option<&str>) -> Option<&str> {
    normalize_optional(value).filter(|value| !value.eq_ignore_ascii_case("all"))
}

fn escaped_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('%');
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('%');
    escaped
}

fn clip_highlight_condition(filter: HighlightFilter) -> Option<String> {
    let category = match filter {
        HighlightFilter::All => return None,
        HighlightFilter::Triple => "triple",
        HighlightFilter::Quad => "quad",
        HighlightFilter::Five => "five",
        HighlightFilter::Six => "six",
        HighlightFilter::KillCompilation => "kill-compilation",
        HighlightFilter::Death => "death",
    };
    Some(format!("{} = '{category}'", clip_highlight_category_sql()))
}

/// Produces exactly one product video type for a clip.
///
/// `kill_count` is a clip-scoped event total, so a compilation may legitimately
/// contain four, five, or six kills. Positive official numeric types therefore
/// win over event counts and descriptive text. Weaker signals are considered
/// only when no positive numeric type is available.
fn clip_highlight_category_sql() -> String {
    let numeric_type = "(
        CASE
            WHEN NULLIF(TRIM(clip_metadata.highlight_type), '') IS NOT NULL
             AND TRIM(clip_metadata.highlight_type) NOT GLOB '*[^0-9]*'
                THEN CAST(TRIM(clip_metadata.highlight_type) AS INTEGER)
            WHEN NULLIF(TRIM(clip_metadata.official_video_type), '') IS NOT NULL
             AND TRIM(clip_metadata.official_video_type) NOT GLOB '*[^0-9]*'
                THEN CAST(TRIM(clip_metadata.official_video_type) AS INTEGER)
            ELSE NULL
        END
    )";
    // The paginated ClipSummary DTO intentionally omits extracted_text. Keep
    // category inputs aligned so server-side filters and card badges cannot drift.
    let text = "LOWER(REPLACE(REPLACE(REPLACE(
        COALESCE(clip_metadata.official_video_name, '') || ' ' ||
        COALESCE(clip_metadata.official_video_type, ''),
        CHAR(9), ' '), CHAR(10), ' '), CHAR(13), ' '))";
    let kill_compilation = format!(
        "({text} LIKE '%击杀合集%'
          OR {text} LIKE '%击杀集锦%'
          OR {text} LIKE '%击杀剪辑%'
          OR {text} LIKE '%kill compilation%'
          OR {text} LIKE '%kill montage%')"
    );
    let death = format!(
        "({text} LIKE '%死亡时刻%'
          OR {text} LIKE '%死亡集锦%'
          OR {text} LIKE '%death moment%'
          OR {text} LIKE '%death compilation%')"
    );
    let marker = |value: &str| {
        format!(
            "({text} LIKE '%{value}'
              OR {text} LIKE '%{value} %'
              OR {text} GLOB '*{value}[!\"#$%&''()*+,./:;<=>?@\\^_`{{|}}~，。！？：；、—…（）【】《》“”‘’]*'
              OR {text} LIKE '%{value}[%'
              OR {text} LIKE '%{value}]%'
              OR {text} LIKE '%{value}-%'
              OR {text} LIKE '%{value}时刻%'
              OR {text} LIKE '%{value}集锦%'
              OR {text} LIKE '%{value}高光%'
              OR {text} LIKE '%{value}片段%'
              OR {text} LIKE '%{value}剪辑%'
              OR {text} LIKE '%{value}回放%'
              OR {text} LIKE '%{value}合集%')"
        )
    };
    let triple = format!(
        "({} OR {} OR {})",
        marker("三杀"),
        marker("3杀"),
        marker("三连杀")
    );
    let quad = format!(
        "({} OR {} OR {})",
        marker("四杀"),
        marker("4杀"),
        marker("四连杀")
    );
    let five = format!(
        "({text} = 'ace'
          OR {text} LIKE 'ace %'
          OR {text} LIKE '% ace'
          OR {text} LIKE '% ace %'
          OR {}
          OR {}
          OR {})",
        marker("五杀"),
        marker("5杀"),
        marker("五连杀")
    );
    let six = format!(
        "({} OR {} OR {})",
        marker("六杀"),
        marker("6杀"),
        marker("六连杀")
    );

    format!(
        "(CASE
            WHEN {numeric_type} = 2 THEN 'kill-compilation'
            WHEN {numeric_type} = 3 THEN 'death'
            WHEN {numeric_type} = 4 THEN 'triple'
            WHEN {numeric_type} = 6 THEN 'quad'
            WHEN {numeric_type} = 10 AND clip_metadata.kill_count = 6 THEN 'six'
            WHEN {numeric_type} = 10 AND clip_metadata.kill_count = 5 THEN 'five'
            WHEN {numeric_type} = 10 AND {six} THEN 'six'
            WHEN {numeric_type} = 10 AND {five} THEN 'five'
            WHEN {numeric_type} IS NOT NULL AND {numeric_type} > 0 THEN NULL
            WHEN {kill_compilation} AND {death} THEN NULL
            WHEN {kill_compilation} AND NOT {death} THEN 'kill-compilation'
            WHEN {death} AND NOT {kill_compilation} THEN 'death'
            WHEN {six} THEN 'six'
            WHEN {five} THEN 'five'
            WHEN {quad} THEN 'quad'
            WHEN {triple} THEN 'triple'
            WHEN clip_metadata.kill_count = 6 THEN 'six'
            WHEN clip_metadata.kill_count = 5 THEN 'five'
            WHEN clip_metadata.kill_count = 4 THEN 'quad'
            WHEN clip_metadata.kill_count = 3 THEN 'triple'
            ELSE NULL
        END)"
    )
}

pub(crate) fn clip_list_order_by(sort: ClipSort) -> String {
    match sort {
        ClipSort::ModifiedDesc => format!("{CLIP_MODIFIED_UNIX_SQL} DESC, clips.id DESC"),
        ClipSort::ModifiedAsc => format!("{CLIP_MODIFIED_UNIX_SQL} ASC, clips.id ASC"),
        ClipSort::SizeDesc => "clips.size_bytes DESC, clips.id DESC".to_string(),
        ClipSort::SizeAsc => "clips.size_bytes ASC, clips.id ASC".to_string(),
        ClipSort::NameAsc => "clips.file_name COLLATE VHM_CLIP_NAME ASC, clips.id ASC".to_string(),
    }
}

pub(in crate::db) const CLIP_SELECT_SQL: &str = "
    SELECT
        clips.id,
        clips.source_dir_id,
        clips.clip_group_id,
        clip_groups.display_name AS clip_group_name,
        clips.file_path,
        clips.normalized_path,
        clips.file_name,
        clips.extension,
        clips.size_bytes,
        clips.modified_at,
        clips.duration_ms,
        clips.recorded_at,
        clips.cover_path,
        clips.cover_source,
        clips.file_status,
        clips.is_favorite,
        clips.note,
        COALESCE(clip_metadata.extracted_text, '') AS extracted_text,
        clip_metadata.account_name,
        clip_metadata.player_name,
        clip_metadata.agent_name,
        clip_metadata.map_name,
        clip_metadata.game_mode,
        COALESCE(clip_metadata.metadata_status, 'not_found') AS metadata_status,
        clip_metadata.match_id,
        matches.account_id AS match_account_id,
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
        CAST(NULLIF(TRIM(clip_metadata.round_score), '') AS INTEGER) AS round_score,
        clip_metadata.metadata_source,
        0 AS event_count,
        COALESCE((
            SELECT GROUP_CONCAT(tag_id, ',')
            FROM (
                SELECT tag_id
                FROM clip_tags
                WHERE clip_id = clips.id
                ORDER BY tag_id
            )
        ), '') AS tag_ids,
        matches.map_id AS match_map_id,
        source_dirs.path AS source_dir_path,
        source_dirs.name AS source_dir_display_name,
        clip_thumbnails.status AS thumbnail_status,
        CASE
            WHEN clip_thumbnails.status = 'ready' THEN clip_thumbnails.revision
            ELSE NULL
        END AS thumbnail_revision,
        source_dirs.source_kind,
        source_dirs.scan_mode,
        source_dirs.scan_root_path,
        clips.source_relative_dir,
        clips.review_decision,
        clips.reviewed_at
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
";

fn resolved_clip_map_name(
    stored_map_name: Option<&str>,
    match_map_id: Option<&str>,
    metadata_source: Option<&str>,
) -> Option<String> {
    let map_name_from_id = match_map_id.and_then(crate::display_names::known_map_name_for_display);
    let preserves_official_map_name =
        matches!(metadata_source, Some("wonderful_db" | "video_export"));
    let has_obsolete_map_name =
        stored_map_name.is_some_and(crate::display_names::is_obsolete_map_display_name);

    if stored_map_name.is_none() || has_obsolete_map_name || !preserves_official_map_name {
        map_name_from_id.or_else(|| stored_map_name.map(str::to_owned))
    } else {
        stored_map_name.map(str::to_owned)
    }
}

pub(in crate::db) fn map_clip(row: &Row<'_>) -> rusqlite::Result<Clip> {
    let favorite: i64 = row.get(15)?;
    let has_won = row.get::<_, Option<i64>>(34)?.map(|value| value != 0);
    let tag_ids_raw: String = row.get(41)?;
    let stored_map_name: Option<String> = row.get(21)?;
    let match_map_id: Option<String> = row.get(42)?;
    let metadata_source: Option<String> = row.get(39)?;
    let source_dir_id: i64 = row.get(1)?;
    let source_dir_path: String = row.get(43)?;
    let source_dir_display_name: String = row.get(44)?;
    let account_name: Option<String> = row.get(18)?;
    let player_name: Option<String> = row.get(19)?;
    let match_account_id = row
        .get::<_, Option<String>>(25)?
        .and_then(|value| normalize_optional(Some(&value)).map(str::to_owned));
    let openid = source_openid(&source_dir_display_name, &source_dir_path);
    let (account_identity_key, account_identity_source, identity_display_value) = account_identity(
        match_account_id.as_deref(),
        openid.as_deref(),
        source_dir_id,
    );
    let account_display_name = normalize_optional(account_name.as_deref())
        .or_else(|| normalize_optional(player_name.as_deref()))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            identity_display_value
                .map(|value| format!("账号 {value}"))
                .unwrap_or_else(|| source_dir_display_name.clone())
        });
    let map_name = resolved_clip_map_name(
        stored_map_name.as_deref(),
        match_map_id.as_deref(),
        metadata_source.as_deref(),
    );

    Ok(Clip {
        id: row.get(0)?,
        source_dir_id,
        source_kind: row.get(47)?,
        scan_mode: row.get(48)?,
        scan_root_path: row.get(49)?,
        source_relative_dir: row.get(50)?,
        clip_group_id: row.get(2)?,
        clip_group_name: row.get(3)?,
        video_path: row.get(4)?,
        normalized_path: row.get(5)?,
        file_name: row.get(6)?,
        extension: row.get(7)?,
        file_size: row.get(8)?,
        modified_at: row.get(9)?,
        duration_ms: row.get(10)?,
        recorded_at: row.get(11)?,
        cover_path: row.get(12)?,
        cover_source: row.get(13)?,
        thumbnail_status: row.get(45)?,
        thumbnail_revision: row.get(46)?,
        status: row.get(14)?,
        favorite: favorite != 0,
        review_decision: row.get(51)?,
        reviewed_at: row.get(52)?,
        note: row.get(16)?,
        extracted_text: row.get(17)?,
        account_identity_key,
        account_identity_source,
        account_display_name,
        openid,
        account_name,
        player_name,
        agent_name: row.get(20)?,
        map_name,
        game_mode: row.get(22)?,
        metadata_status: row.get(23)?,
        match_id: row.get(24)?,
        match_account_id,
        scoreline: row.get(26)?,
        kda: row.get(27)?,
        agent_avatar_url: row.get(28)?,
        round_label: row.get(29)?,
        weapon_name: row.get(30)?,
        kill_count: row.get(31)?,
        match_started_at: row.get(32)?,
        combat_score: row.get(33)?,
        has_won,
        official_video_name: row.get(35)?,
        official_video_type: row.get(36)?,
        highlight_type: row.get(37)?,
        round_score: row.get(38)?,
        metadata_source,
        event_count: row.get(40)?,
        clip_events: Vec::new(),
        tag_ids: parse_tag_ids(&tag_ids_raw)?,
    })
}

pub(crate) fn map_clip_summary(row: &Row<'_>) -> rusqlite::Result<ClipSummary> {
    let source_dir_id: i64 = row.get(1)?;
    let source_dir_path: String = row.get(35)?;
    let source_dir_name: String = row.get(36)?;
    let account_name: Option<String> = row.get(14)?;
    let player_name: Option<String> = row.get(15)?;
    let stored_map_name: Option<String> = row.get(17)?;
    let match_account_id = row
        .get::<_, Option<String>>(21)?
        .and_then(|value| normalize_optional(Some(&value)).map(str::to_owned));
    let metadata_source: Option<String> = row.get(33)?;
    let match_map_id: Option<String> = row.get(34)?;
    let openid = source_openid(&source_dir_name, &source_dir_path);
    let (account_identity_key, account_identity_source, identity_display_value) = account_identity(
        match_account_id.as_deref(),
        openid.as_deref(),
        source_dir_id,
    );
    let account_display_name = normalize_optional(account_name.as_deref())
        .or_else(|| normalize_optional(player_name.as_deref()))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            identity_display_value
                .map(|value| format!("账号 {value}"))
                .unwrap_or_else(|| source_dir_name.clone())
        });
    let map_name = resolved_clip_map_name(
        stored_map_name.as_deref(),
        match_map_id.as_deref(),
        metadata_source.as_deref(),
    );

    Ok(ClipSummary {
        id: row.get(0)?,
        source_dir_id,
        source_dir_path,
        source_dir_name,
        source_kind: row.get(39)?,
        scan_mode: row.get(40)?,
        scan_root_path: row.get(41)?,
        source_relative_dir: row.get(42)?,
        clip_group_id: row.get(2)?,
        clip_group_name: row.get(3)?,
        video_path: row.get(4)?,
        file_name: row.get(5)?,
        file_size: row.get(6)?,
        modified_at: row.get(7)?,
        duration_ms: row.get(8)?,
        recorded_at: row.get(9)?,
        cover_path: row.get(10)?,
        cover_source: row.get(11)?,
        thumbnail_status: row.get(37)?,
        thumbnail_revision: row.get(38)?,
        status: row.get(12)?,
        favorite: row.get::<_, i64>(13)? != 0,
        review_decision: row.get(43)?,
        reviewed_at: row.get(44)?,
        account_identity_key,
        account_identity_source,
        account_display_name,
        openid,
        account_name,
        player_name,
        agent_name: row.get(16)?,
        map_name,
        game_mode: row.get(18)?,
        metadata_status: row.get(19)?,
        match_id: row.get(20)?,
        match_account_id,
        scoreline: row.get(22)?,
        kda: row.get(23)?,
        agent_avatar_url: row.get(24)?,
        kill_count: row.get(25)?,
        match_started_at: row.get(26)?,
        combat_score: row.get(27)?,
        has_won: row.get::<_, Option<i64>>(28)?.map(|value| value != 0),
        official_video_name: row.get(29)?,
        official_video_type: row.get(30)?,
        highlight_type: row.get(31)?,
        round_score: row.get(32)?,
        metadata_source,
        tag_ids: Vec::new(),
    })
}

pub(crate) fn attach_clip_summary_tags(
    connection: &Connection,
    summaries: &mut [ClipSummary],
) -> DbResult<()> {
    if summaries.is_empty() {
        return Ok(());
    }

    let clip_ids = summaries
        .iter()
        .map(|summary| summary.id)
        .collect::<Vec<_>>();
    let placeholders = std::iter::repeat_n("?", clip_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        SELECT clip_id, tag_id
        FROM clip_tags
        WHERE clip_id IN ({placeholders})
        ORDER BY clip_id, tag_id
        "
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| readable_error("preparing clip summary tag query", error))?;
    let rows = statement
        .query_map(params_from_iter(clip_ids.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| readable_error("querying clip summary tags", error))?;
    let mut tags_by_clip_id = HashMap::<i64, Vec<i64>>::new();
    for row in rows {
        let (clip_id, tag_id) =
            row.map_err(|error| readable_error("reading clip summary tags", error))?;
        tags_by_clip_id.entry(clip_id).or_default().push(tag_id);
    }
    for summary in summaries {
        summary.tag_ids = tags_by_clip_id.remove(&summary.id).unwrap_or_default();
    }

    Ok(())
}

pub(in crate::db) fn attach_clip_events(
    connection: &Connection,
    clips: &mut [Clip],
) -> DbResult<()> {
    if clips.is_empty() {
        return Ok(());
    }

    let clip_ids = clips.iter().map(|clip| clip.id).collect::<Vec<_>>();
    let mut events_by_clip_id = list_clip_events_for_clips(connection, &clip_ids)?;
    for clip in clips {
        let events = events_by_clip_id.remove(&clip.id).unwrap_or_default();
        clip.event_count = events.len() as i64;
        clip.clip_events = events;
    }

    Ok(())
}

fn list_clip_events_for_clips(
    connection: &Connection,
    clip_ids: &[i64],
) -> DbResult<HashMap<i64, Vec<ClipEvent>>> {
    if clip_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = std::iter::repeat_n("?", clip_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        SELECT
            clip_events.id,
            clip_events.clip_id,
            clip_events.segment_id,
            clip_segments.segment_key,
            clip_events.event_key,
            clip_events.event_type,
            clip_events.video_time_ms,
            clip_events.event_time,
            clip_events.round_id,
            clip_events.player_name,
            clip_events.agent_name,
            clip_events.weapon_name,
            clip_events.killer_name,
            clip_events.killed_name,
            clip_events.killer_is_me,
            clip_events.killed_is_me,
            clip_events.raw_json,
            clip_events.created_at
        FROM clip_events
        LEFT JOIN clip_segments ON clip_segments.id = clip_events.segment_id
        WHERE clip_events.clip_id IN ({placeholders})
        ORDER BY
            clip_events.clip_id,
            COALESCE(clip_events.video_time_ms, 9223372036854775807),
            clip_events.id
        "
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| readable_error("preparing clip event batch list", error))?;
    let events = statement
        .query_map(params_from_iter(clip_ids.iter()), map_clip_event)
        .map_err(|error| readable_error("querying clip event batch", error))?;
    let mut events_by_clip_id = HashMap::new();
    for event in events {
        let event = event.map_err(|error| readable_error("reading clip event batch", error))?;
        events_by_clip_id
            .entry(event.clip_id)
            .or_insert_with(Vec::new)
            .push(event);
    }

    Ok(events_by_clip_id)
}

fn map_clip_event(row: &Row<'_>) -> rusqlite::Result<ClipEvent> {
    Ok(ClipEvent {
        id: row.get(0)?,
        clip_id: row.get(1)?,
        segment_id: row.get(2)?,
        segment_key: row.get(3)?,
        event_key: row.get(4)?,
        event_type: row.get(5)?,
        video_time_ms: row.get(6)?,
        event_time: row.get(7)?,
        round_id: row.get(8)?,
        player_name: row.get(9)?,
        agent_name: row.get(10)?,
        weapon_name: row.get(11)?,
        killer_name: row.get(12)?,
        killed_name: row.get(13)?,
        killer_is_me: row.get::<_, i64>(14)? != 0,
        killed_is_me: row.get::<_, Option<i64>>(15)?.map(|value| value != 0),
        raw_json: row.get(16)?,
        created_at: row.get(17)?,
    })
}

fn account_identity(
    match_account_id: Option<&str>,
    openid: Option<&str>,
    source_dir_id: i64,
) -> (String, AccountIdentitySource, Option<String>) {
    if let Some(match_account_id) = normalize_optional(match_account_id) {
        return (
            format!("match-account-{match_account_id}"),
            AccountIdentitySource::MatchAccountId,
            Some(match_account_id.to_string()),
        );
    }

    if let Some(openid) = normalize_optional(openid) {
        return (
            format!("match-account-{openid}"),
            AccountIdentitySource::Openid,
            Some(openid.to_string()),
        );
    }

    (
        format!("source-{source_dir_id}"),
        AccountIdentitySource::SourceDir,
        None,
    )
}

fn parse_tag_ids(raw: &str) -> rusqlite::Result<Vec<i64>> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    Ok(raw
        .split(',')
        .filter_map(|value| value.parse::<i64>().ok())
        .collect())
}
