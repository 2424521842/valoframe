use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::{
    db::{self, DbResult},
    display_names,
    highlight_log_parser::{HighlightLogKillEvent, HighlightLogLineKind, HighlightLogRecord},
    leveldb_reader::LevelDbBattleRecord,
};

#[derive(Debug, Clone, Copy)]
pub struct MetadataIngestInput<'a> {
    pub leveldb_battles: &'a [LevelDbBattleRecord],
    pub log_records: &'a [HighlightLogRecord],
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataIngestSummary {
    pub matches_upserted: usize,
    pub stats_upserted: usize,
    pub events_inserted: usize,
    pub enriched_clip_count: usize,
    pub unmatched_match_count: usize,
}

#[derive(Debug, Clone, Default)]
struct MergedMatch {
    match_id: Option<String>,
    battle_id: Option<String>,
    account_id: Option<String>,
    record_src: Option<String>,
    player_name: Option<String>,
    agent_name: Option<String>,
    agent_avatar_url: Option<String>,
    map_id: Option<String>,
    map_name: Option<String>,
    game_mode: Option<String>,
    started_at: Option<String>,
    kda: Option<String>,
    scoreline: Option<String>,
    has_won: Option<bool>,
    combat_score: Option<i64>,
    kill_events: Vec<HighlightLogKillEvent>,
    source_leveldb: bool,
    source_log: bool,
}

pub fn ingest_match_metadata(
    connection: &Connection,
    input: MetadataIngestInput<'_>,
) -> DbResult<MetadataIngestSummary> {
    let merged_matches = merge_metadata_sources(input);
    let mut summary = MetadataIngestSummary::default();

    for mut merged in merged_matches {
        reconcile_account_from_unique_clip_source(connection, &mut merged)?;
        let Some(game_id) = merged.game_id() else {
            continue;
        };

        let match_row_id = upsert_match(connection, &game_id, &merged)?;
        summary.matches_upserted += 1;

        if upsert_match_stats(connection, match_row_id, &merged)? {
            summary.stats_upserted += 1;
        }
        summary.events_inserted += replace_match_events(connection, match_row_id, &merged)?;

        let enriched = enrich_matching_clips(connection, &game_id, &merged)?;
        if enriched == 0 {
            summary.unmatched_match_count += 1;
        }
        summary.enriched_clip_count += enriched;
    }

    Ok(summary)
}

fn merge_metadata_sources(input: MetadataIngestInput<'_>) -> Vec<MergedMatch> {
    let mut matches = Vec::new();
    let mut aliases = HashMap::new();

    for battle in input.leveldb_battles {
        let index = merged_index_for_leveldb(battle, &mut matches, &mut aliases);
        let merged = &mut matches[index];
        merged.source_leveldb = true;
        merge_string(&mut merged.match_id, battle.match_id.as_deref());
        merge_string(&mut merged.battle_id, battle.battle_id.as_deref());
        merge_string(&mut merged.account_id, Some(&battle.account_id));
        merge_player_name(&mut merged.player_name, battle.player_name.as_deref());
        merge_string(
            &mut merged.agent_avatar_url,
            battle.agent_avatar_url.as_deref(),
        );
        if let Some(agent_name) = battle
            .agent_avatar_url
            .as_deref()
            .and_then(display_names::agent_name_from_avatar_url)
        {
            merge_agent_name(&mut merged.agent_name, Some(&agent_name));
        }
        if battle.kda.is_some() {
            merged.kda = battle.kda.clone();
        }
        if battle.match_date.is_some() {
            merged.started_at = battle.match_date.clone();
        }
        register_aliases(index, merged, &mut aliases);
    }

    for record in input.log_records {
        let index = merged_index_for_log(record, &mut matches, &mut aliases);
        let merged = &mut matches[index];
        let source_account_id = account_id_from_record_src(record);
        let has_source_account = source_account_id.is_some();
        let source_account_conflicts =
            match (source_account_id.as_deref(), merged.account_id.as_deref()) {
                (Some(source_account_id), Some(existing_account_id)) => {
                    source_account_id != existing_account_id
                }
                _ => false,
            };
        let precise_record_src_match = record_src_matches_identity(record);
        let full_match_summary = record_has_full_match_summary(record);
        let is_response_data = matches!(
            record.line_kind,
            HighlightLogLineKind::BattleListResponse | HighlightLogLineKind::PostSnapshot
        );
        merged.source_log = true;
        merge_string(&mut merged.match_id, record.match_id.as_deref());
        merge_string(&mut merged.battle_id, record.battle_id.as_deref());
        merge_string(&mut merged.record_src, record.record_src.as_deref());
        if source_account_conflicts && !precise_record_src_match && !full_match_summary {
            register_aliases(index, merged, &mut aliases);
            continue;
        }
        if source_account_conflicts {
            merged.kill_events.clear();
        }
        if let Some(account_id) = source_account_id.as_deref() {
            replace_string(&mut merged.account_id, Some(account_id));
        }

        if has_source_account || (full_match_summary && !is_response_data) {
            replace_player_name(&mut merged.player_name, record.player_name.as_deref());
            replace_agent_name(&mut merged.agent_name, record.agent_name.as_deref());
            replace_string(&mut merged.map_id, record.map_id.as_deref());
            replace_map_name(
                &mut merged.map_name,
                record.map_name.as_deref().or(record.map_id.as_deref()),
            );
            replace_game_mode(&mut merged.game_mode, record.game_mode.as_deref());
            replace_string(&mut merged.kda, record.kda.as_deref());
            replace_string(&mut merged.scoreline, record.scoreline.as_deref());
        } else {
            merge_player_name(&mut merged.player_name, record.player_name.as_deref());
            merge_agent_name(&mut merged.agent_name, record.agent_name.as_deref());
            merge_string(&mut merged.map_id, record.map_id.as_deref());
            merge_map_name(
                &mut merged.map_name,
                record.map_name.as_deref().or(record.map_id.as_deref()),
            );
            merge_game_mode(&mut merged.game_mode, record.game_mode.as_deref());
            merge_string(&mut merged.kda, record.kda.as_deref());
            merge_string(&mut merged.scoreline, record.scoreline.as_deref());
        }
        if record.has_won.is_some() {
            merged.has_won = record.has_won;
        }
        if record.combat_score.is_some() {
            merged.combat_score = record.combat_score;
        }
        merged
            .kill_events
            .extend(record.kill_events.iter().cloned());
        register_aliases(index, merged, &mut aliases);
    }

    matches
}

fn merged_index_for_leveldb(
    battle: &LevelDbBattleRecord,
    matches: &mut Vec<MergedMatch>,
    aliases: &mut HashMap<String, usize>,
) -> usize {
    for key in [battle.match_id.as_deref(), battle.battle_id.as_deref()]
        .into_iter()
        .flatten()
    {
        if let Some(index) = aliases.get(key) {
            return *index;
        }
    }

    push_merged_match(matches)
}

fn merged_index_for_log(
    record: &HighlightLogRecord,
    matches: &mut Vec<MergedMatch>,
    aliases: &mut HashMap<String, usize>,
) -> usize {
    let identity_keys = [record.match_id.as_deref(), record.battle_id.as_deref()];
    let has_identity_key = identity_keys.iter().any(|key| key.is_some());
    let record_src_key = if has_identity_key {
        None
    } else {
        record.record_src.as_deref().and_then(record_src_group_key)
    };

    for key in identity_keys.into_iter().flatten().chain(record_src_key) {
        if let Some(index) = aliases.get(key) {
            return *index;
        }
    }

    push_merged_match(matches)
}

fn push_merged_match(matches: &mut Vec<MergedMatch>) -> usize {
    let index = matches.len();
    matches.push(MergedMatch::default());
    index
}

fn register_aliases(index: usize, merged: &MergedMatch, aliases: &mut HashMap<String, usize>) {
    for key in [
        merged.match_id.as_deref(),
        merged.battle_id.as_deref(),
        merged.record_src.as_deref().and_then(record_src_group_key),
    ]
    .into_iter()
    .flatten()
    {
        aliases.insert(key.to_string(), index);
    }
}

fn upsert_match(connection: &Connection, game_id: &str, merged: &MergedMatch) -> DbResult<i64> {
    connection
        .execute(
            "
            INSERT INTO matches (
                game_id,
                battle_id,
                account_id,
                player_name,
                agent_name,
                agent_avatar_url,
                map_id,
                map_name,
                game_mode,
                started_at,
                source_leveldb,
                source_log
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(game_id) DO UPDATE SET
                battle_id = COALESCE(excluded.battle_id, matches.battle_id),
                account_id = COALESCE(excluded.account_id, matches.account_id),
                player_name = COALESCE(excluded.player_name, matches.player_name),
                agent_name = COALESCE(excluded.agent_name, matches.agent_name),
                agent_avatar_url = COALESCE(excluded.agent_avatar_url, matches.agent_avatar_url),
                map_id = COALESCE(excluded.map_id, matches.map_id),
                map_name = COALESCE(excluded.map_name, matches.map_name),
                game_mode = COALESCE(excluded.game_mode, matches.game_mode),
                started_at = COALESCE(excluded.started_at, matches.started_at),
                source_leveldb = CASE
                    WHEN excluded.source_leveldb = 1 THEN 1
                    ELSE matches.source_leveldb
                END,
                source_log = CASE
                    WHEN excluded.source_log = 1 THEN 1
                    ELSE matches.source_log
                END,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                game_id,
                merged.battle_id.as_deref(),
                merged.account_id.as_deref(),
                merged.player_name.as_deref(),
                merged.agent_name.as_deref(),
                merged.agent_avatar_url.as_deref(),
                merged.map_id.as_deref(),
                merged.map_name.as_deref(),
                merged.game_mode.as_deref(),
                merged.started_at.as_deref(),
                bool_to_integer(merged.source_leveldb),
                bool_to_integer(merged.source_log),
            ],
        )
        .map_err(|error| format!("Database upserting match failed: {error}"))?;

    connection
        .query_row(
            "SELECT id FROM matches WHERE game_id = ?1",
            params![game_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Database reading match id failed: {error}"))
}

fn upsert_match_stats(
    connection: &Connection,
    match_row_id: i64,
    merged: &MergedMatch,
) -> DbResult<bool> {
    let (kills, deaths, assists) = parse_kda(merged.kda.as_deref());
    let (rounds_won, rounds_lost) = parse_scoreline(merged.scoreline.as_deref());
    let has_stats = kills.is_some()
        || deaths.is_some()
        || assists.is_some()
        || merged.combat_score.is_some()
        || rounds_won.is_some()
        || rounds_lost.is_some()
        || merged.has_won.is_some();

    if !has_stats {
        return Ok(false);
    }

    connection
        .execute(
            "
            INSERT INTO match_stats (
                match_id,
                kills,
                deaths,
                assists,
                combat_score,
                rounds_won,
                rounds_lost,
                rounds_played,
                has_won
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(match_id) DO UPDATE SET
                kills = excluded.kills,
                deaths = excluded.deaths,
                assists = excluded.assists,
                combat_score = excluded.combat_score,
                rounds_won = excluded.rounds_won,
                rounds_lost = excluded.rounds_lost,
                rounds_played = excluded.rounds_played,
                has_won = excluded.has_won,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                match_row_id,
                kills,
                deaths,
                assists,
                merged.combat_score,
                rounds_won,
                rounds_lost,
                rounds_played(rounds_won, rounds_lost),
                merged.has_won.map(bool_to_integer),
            ],
        )
        .map_err(|error| format!("Database upserting match stats failed: {error}"))?;

    Ok(true)
}

fn replace_match_events(
    connection: &Connection,
    match_row_id: i64,
    merged: &MergedMatch,
) -> DbResult<usize> {
    connection
        .execute(
            "DELETE FROM match_events WHERE match_id = ?1",
            params![match_row_id],
        )
        .map_err(|error| format!("Database clearing match events failed: {error}"))?;

    let recordable_events = recordable_kill_events(&merged.kill_events);
    for event in &recordable_events {
        connection
            .execute(
                "
                INSERT INTO match_events (
                    match_id,
                    event_type,
                    event_time,
                    round_id,
                    weapon_name,
                    killer_name,
                    killed_name,
                    raw_json
                )
                VALUES (?1, 'kill', ?2, ?3, ?4, ?5, ?6, ?7)
                ",
                params![
                    match_row_id,
                    event.event_time.as_deref(),
                    event.round_id,
                    event.weapon_name.as_deref(),
                    event.killer_name.as_deref(),
                    event.killed_name.as_deref(),
                    event.raw_json.as_deref(),
                ],
            )
            .map_err(|error| format!("Database inserting match event failed: {error}"))?;
    }

    Ok(recordable_events.len())
}

fn recordable_kill_events(events: &[HighlightLogKillEvent]) -> Vec<&HighlightLogKillEvent> {
    events
        .iter()
        .filter(|event| event.round_id != Some(0))
        .collect()
}

fn enrich_matching_clips(
    connection: &Connection,
    game_id: &str,
    merged: &MergedMatch,
) -> DbResult<usize> {
    if merged.account_id.is_none() {
        return Ok(0);
    }
    let clip_ids = find_matching_clip_ids(connection, merged)?;
    if clip_ids.is_empty() {
        return Ok(0);
    }

    let player_name = merged
        .player_name
        .as_deref()
        .and_then(normalize_player_name_for_display);
    let agent_name = merged
        .agent_name
        .as_deref()
        .and_then(normalize_agent_name_for_display);

    for clip_id in &clip_ids {
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET metadata_status = 'enriched',
                    match_id = ?2,
                    updated_at = CURRENT_TIMESTAMP
                WHERE clip_id = ?1
                  AND COALESCE(metadata_source, 'inferred') <> 'wonderful_db'
                ",
                params![clip_id, game_id],
            )
            .map_err(|error| format!("Database enriching clip metadata failed: {error}"))?;
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET account_name = CASE
                        WHEN metadata_source IN ('wonderful_db', 'video_export') THEN COALESCE(account_name, ?2)
                        ELSE COALESCE(?2, account_name)
                    END,
                    player_name = CASE
                        WHEN metadata_source IN ('wonderful_db', 'video_export') THEN COALESCE(player_name, ?2)
                        ELSE COALESCE(?2, player_name)
                    END,
                    agent_name = CASE
                        WHEN metadata_source IN ('wonderful_db', 'video_export') THEN COALESCE(agent_name, ?3)
                        ELSE COALESCE(?3, agent_name)
                    END,
                    map_name = CASE
                        WHEN metadata_source IN ('wonderful_db', 'video_export') THEN COALESCE(map_name, ?4)
                        ELSE COALESCE(?4, map_name)
                    END,
                    game_mode = CASE
                        WHEN metadata_source IN ('wonderful_db', 'video_export') THEN COALESCE(game_mode, ?5)
                        ELSE COALESCE(?5, game_mode)
                    END,
                    scoreline = CASE
                        WHEN metadata_source IN ('wonderful_db', 'video_export') THEN COALESCE(scoreline, ?6)
                        ELSE COALESCE(?6, scoreline)
                    END,
                    kda = CASE
                        WHEN metadata_source IN ('wonderful_db', 'video_export') THEN COALESCE(kda, ?7)
                        ELSE COALESCE(?7, kda)
                    END,
                    updated_at = CURRENT_TIMESTAMP
                WHERE clip_id = ?1
                ",
                params![
                    clip_id,
                    player_name.as_deref(),
                    agent_name.as_deref(),
                    merged.map_name.as_deref(),
                    merged.game_mode.as_deref(),
                    merged.scoreline.as_deref(),
                    merged.kda.as_deref(),
                ],
            )
            .map_err(|error| format!("Database enriching clip metadata failed: {error}"))?;
    }

    Ok(clip_ids.len())
}

fn find_matching_clip_ids(connection: &Connection, merged: &MergedMatch) -> DbResult<Vec<i64>> {
    let group_keys = group_keys_for_merged(merged);
    let mut clip_ids = Vec::new();
    let mut seen = HashSet::new();
    let Some(account_id) = merged.account_id.as_deref() else {
        return Ok(clip_ids);
    };
    for group_key in group_keys {
        let mut statement = connection
            .prepare(
                "
                SELECT clips.id, source_dirs.name, source_dirs.path
                FROM clips
                JOIN clip_groups
                    ON clip_groups.id = clips.clip_group_id
                JOIN source_dirs
                    ON source_dirs.id = clips.source_dir_id
                WHERE clip_groups.group_key = ?1
                ",
            )
            .map_err(|error| format!("Database preparing clip match query failed: {error}"))?;
        let rows = statement
            .query_map(params![group_key], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| format!("Database querying clip matches failed: {error}"))?;

        for row in rows {
            let (clip_id, source_name, source_path) =
                row.map_err(|error| format!("Database reading clip match failed: {error}"))?;
            if db::source_openid(&source_name, &source_path).as_deref() == Some(account_id)
                && seen.insert(clip_id)
            {
                clip_ids.push(clip_id);
            }
        }
    }

    Ok(clip_ids)
}

fn reconcile_account_from_unique_clip_source(
    connection: &Connection,
    merged: &mut MergedMatch,
) -> DbResult<()> {
    let group_keys = group_keys_for_merged(merged);
    if group_keys.is_empty() {
        return Ok(());
    }

    let mut source_account_ids = HashSet::new();
    for group_key in group_keys {
        let mut statement = connection
            .prepare(
                "
                SELECT DISTINCT source_dirs.name, source_dirs.path
                FROM clips
                JOIN clip_groups
                    ON clip_groups.id = clips.clip_group_id
                JOIN source_dirs
                    ON source_dirs.id = clips.source_dir_id
                WHERE clip_groups.group_key = ?1
                ",
            )
            .map_err(|error| {
                format!("Database preparing source account reconciliation failed: {error}")
            })?;
        let rows = statement
            .query_map(params![group_key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| {
                format!("Database querying source account reconciliation failed: {error}")
            })?;

        for row in rows {
            let (source_dir_name, source_dir_path) =
                row.map_err(|error| format!("Database reading source account failed: {error}"))?;
            if let Some(account_id) = db::source_openid(&source_dir_name, &source_dir_path) {
                source_account_ids.insert(account_id);
            }
        }
    }

    if source_account_ids.len() == 1 {
        merged.account_id = source_account_ids.into_iter().next();
    } else if merged.account_id.is_none() {
        // Multiple source dirs share this group_key — can't resolve
        // unambiguously. Leave account_id as None so enrichment is
        // skipped rather than applied to the wrong account's clips.
    }

    Ok(())
}

fn group_keys_for_merged(merged: &MergedMatch) -> HashSet<String> {
    [
        merged.match_id.as_deref(),
        merged.battle_id.as_deref(),
        merged.record_src.as_deref().and_then(record_src_group_key),
    ]
    .into_iter()
    .flatten()
    .map(str::to_string)
    .collect()
}

fn record_src_matches_identity(record: &HighlightLogRecord) -> bool {
    let Some(group_key) = record.record_src.as_deref().and_then(record_src_group_key) else {
        return false;
    };

    [record.match_id.as_deref(), record.battle_id.as_deref()]
        .into_iter()
        .flatten()
        .any(|identity_key| identity_key == group_key)
}

fn record_has_full_match_summary(record: &HighlightLogRecord) -> bool {
    record.scoreline.is_some()
        || matches!(
            record.line_kind,
            HighlightLogLineKind::BattleListResponse | HighlightLogLineKind::PostSnapshot
        )
}

impl MergedMatch {
    fn game_id(&self) -> Option<String> {
        self.match_id
            .clone()
            .or_else(|| self.battle_id.clone())
            .or_else(|| {
                self.record_src
                    .as_deref()
                    .and_then(record_src_group_key)
                    .map(str::to_string)
            })
    }
}

fn merge_string(target: &mut Option<String>, value: Option<&str>) {
    if target.is_none() {
        *target = normalize_optional(value).map(str::to_string);
    }
}

fn replace_string(target: &mut Option<String>, value: Option<&str>) {
    if let Some(value) = normalize_optional(value) {
        *target = Some(value.to_string());
    }
}

fn merge_player_name(target: &mut Option<String>, value: Option<&str>) {
    if target.is_none() {
        *target = value.and_then(normalize_player_name_for_display);
    }
}

fn replace_player_name(target: &mut Option<String>, value: Option<&str>) {
    if let Some(value) = value.and_then(normalize_player_name_for_display) {
        *target = Some(value);
    }
}

fn merge_agent_name(target: &mut Option<String>, value: Option<&str>) {
    if target.is_none() {
        *target = value.and_then(normalize_agent_name_for_display);
    }
}

fn replace_agent_name(target: &mut Option<String>, value: Option<&str>) {
    if let Some(value) = value.and_then(normalize_agent_name_for_display) {
        *target = Some(value);
    }
}

fn merge_map_name(target: &mut Option<String>, value: Option<&str>) {
    if target.is_none() {
        *target = value.and_then(display_names::map_name_for_display);
    }
}

fn replace_map_name(target: &mut Option<String>, value: Option<&str>) {
    if let Some(value) = value.and_then(display_names::map_name_for_display) {
        *target = Some(value);
    }
}

fn merge_game_mode(target: &mut Option<String>, value: Option<&str>) {
    if target.is_none() {
        *target = value.and_then(display_names::game_mode_for_display);
    }
}

fn replace_game_mode(target: &mut Option<String>, value: Option<&str>) {
    if let Some(value) = value.and_then(display_names::game_mode_for_display) {
        *target = Some(value);
    }
}

fn normalize_optional(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_player_name_for_display(value: &str) -> Option<String> {
    let player_name = display_names::player_name_for_display(value)?;
    if player_name.contains('#') {
        Some(player_name)
    } else {
        None
    }
}

fn normalize_agent_name_for_display(value: &str) -> Option<String> {
    display_names::agent_name_for_display(value)
}

fn account_id_from_record_src(record: &HighlightLogRecord) -> Option<String> {
    let record_src = record.record_src.as_deref()?;
    let marker = "wonderfulVideos";
    let marker_index = record_src.find(marker)?;
    let start = marker_index + marker.len();
    let digits = record_src[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

fn record_src_group_key(record_src: &str) -> Option<&str> {
    let mut parts = record_src
        .trim_end_matches('/')
        .rsplit('/')
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let leaf = parts.next()?;
    let group_key = if leaf.to_ascii_lowercase().ends_with(".mp4") {
        parts.next()?
    } else {
        leaf
    };

    if group_key.is_empty() || group_key.eq_ignore_ascii_case("record") {
        None
    } else {
        Some(group_key)
    }
}

fn parse_kda(kda: Option<&str>) -> (Option<i64>, Option<i64>, Option<i64>) {
    let Some(kda) = kda else {
        return (None, None, None);
    };
    let parts = kda.split('/').collect::<Vec<_>>();
    if parts.len() != 3 {
        return (None, None, None);
    }

    (
        parts[0].trim().parse::<i64>().ok(),
        parts[1].trim().parse::<i64>().ok(),
        parts[2].trim().parse::<i64>().ok(),
    )
}

fn parse_scoreline(scoreline: Option<&str>) -> (Option<i64>, Option<i64>) {
    let Some(scoreline) = scoreline else {
        return (None, None);
    };
    let parts = scoreline.split('/').collect::<Vec<_>>();
    if parts.len() != 2 {
        return (None, None);
    }

    (
        parts[0].trim().parse::<i64>().ok(),
        parts[1].trim().parse::<i64>().ok(),
    )
}

fn rounds_played(rounds_won: Option<i64>, rounds_lost: Option<i64>) -> Option<i64> {
    Some(rounds_won? + rounds_lost?)
}

fn bool_to_integer(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}
