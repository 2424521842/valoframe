use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::{
    db::{self, ClipEventInput, ClipSegmentInput, DbResult},
    highlight_log_parser::HighlightLogRoundScore,
    wonderful_db::{
        WonderfulAccountRecord, WonderfulEventNormalizationWarning, WonderfulEventRecord,
        WonderfulMatchRecord, WonderfulSnapshotAccountRecord, WonderfulVideoRecord,
    },
};

const MAX_INGEST_WARNING_SAMPLES: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WonderfulIngestSummary {
    pub matched_video_count: usize,
    pub unmatched_video_count: usize,
    pub event_count: usize,
    pub round_score_backfilled_count: usize,
    pub warning_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WonderfulSnapshotIngestSummary {
    pub snapshot_count: usize,
    pub match_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AccountNameCandidate {
    account_name: String,
    observed_at: Option<String>,
    source_priority: u8,
    traversal_order: usize,
}

pub fn ingest_wonderful_metadata(
    connection: &Connection,
    accounts: &[WonderfulAccountRecord],
) -> DbResult<WonderfulIngestSummary> {
    ingest_wonderful_metadata_with_round_scores(connection, accounts, &[])
}

/// Resolves Riot ID history across the main WonderfulDb records and snapshots by observation
/// time. Calling the two ingest functions in a fixed order is not sufficient: snapshots are often
/// newer than the last video event after a player renames their Riot ID.
pub fn propagate_latest_wonderful_account_names(
    connection: &Connection,
    accounts: &[WonderfulAccountRecord],
    snapshot_accounts: &[WonderfulSnapshotAccountRecord],
) -> DbResult<usize> {
    let hints = latest_wonderful_account_name_hints(accounts, snapshot_accounts);
    db::propagate_authoritative_account_name_hints(connection, &hints, None)
}

fn latest_wonderful_account_name_hints(
    accounts: &[WonderfulAccountRecord],
    snapshot_accounts: &[WonderfulSnapshotAccountRecord],
) -> Vec<db::AccountNameHint> {
    let mut latest_by_account = HashMap::<String, AccountNameCandidate>::new();

    for account in accounts {
        if let Some(candidate) = latest_valid_account_name_candidate(account) {
            insert_latest_account_name_candidate(
                &mut latest_by_account,
                &account.openid,
                candidate,
            );
        }
    }
    for account in snapshot_accounts {
        if let Some(candidate) = latest_valid_snapshot_account_name_candidate(account) {
            insert_latest_account_name_candidate(
                &mut latest_by_account,
                &account.openid,
                candidate,
            );
        }
    }

    latest_by_account
        .into_iter()
        .map(|(account_id, candidate)| db::AccountNameHint {
            account_id,
            account_name: candidate.account_name,
        })
        .collect()
}

fn insert_latest_account_name_candidate(
    latest_by_account: &mut HashMap<String, AccountNameCandidate>,
    account_id: &str,
    candidate: AccountNameCandidate,
) {
    let should_replace = latest_by_account
        .get(account_id)
        .is_none_or(|current| account_name_candidate_is_newer(&candidate, current));
    if should_replace {
        latest_by_account.insert(account_id.to_string(), candidate);
    }
}

pub fn ingest_wonderful_metadata_with_round_scores(
    connection: &Connection,
    accounts: &[WonderfulAccountRecord],
    round_scores: &[HighlightLogRoundScore],
) -> DbResult<WonderfulIngestSummary> {
    let mut summary = WonderfulIngestSummary::default();
    let mut claimed_clip_ids = HashSet::new();
    let round_score_lookup = build_round_score_lookup(round_scores);

    for account in accounts {
        let latest_account_name = latest_valid_account_name(account);

        for match_record in &account.matches {
            upsert_wonderful_match(connection, &account.openid, match_record)?;

            for video in &match_record.videos {
                let Some(clip_id) = find_clip_for_video(
                    connection,
                    &account.openid,
                    &match_record.match_id,
                    video,
                )?
                else {
                    summary.unmatched_video_count += 1;
                    continue;
                };
                if !claimed_clip_ids.insert(clip_id) {
                    summary.unmatched_video_count += 1;
                    continue;
                }

                let official_duration_ms = video_duration_ms(video);
                let indexed_duration_ms = connection
                    .query_row(
                        "SELECT duration_ms FROM clips WHERE id = ?1",
                        params![clip_id],
                        |row| row.get::<_, Option<i64>>(0),
                    )
                    .map_err(|error| {
                        format!("Database reading indexed clip duration failed: {error}")
                    })?;
                let timeline = build_timeline(video, official_duration_ms.or(indexed_duration_ms));
                summary.warning_count =
                    summary.warning_count.saturating_add(timeline.warning_count);
                for warning in &timeline.warnings {
                    if summary.warnings.len() >= MAX_INGEST_WARNING_SAMPLES {
                        break;
                    }
                    summary.warnings.push(warning.clone());
                }
                let kill_count = timeline
                    .events
                    .iter()
                    .filter(|event| event.event_type == "kill" && event.killer_is_me)
                    .count() as i64;
                let highlight_type = video
                    .highlight_type
                    .or_else(|| video.video_type.trim().parse::<i64>().ok());
                let duration_ms = official_duration_ms;
                let (round_score, round_score_source) =
                    resolved_round_score(&account.openid, match_record, video, &round_score_lookup);
                let can_preserve_reconstructed_score =
                    supports_official_round_score(match_record, video);

                let metadata_transaction = connection.unchecked_transaction().map_err(|error| {
                    format!("Database starting official metadata transaction failed: {error}")
                })?;
                metadata_transaction
                    .execute(
                        "
                        UPDATE clips
                        SET duration_ms = COALESCE(?2, duration_ms),
                            recorded_at = COALESCE(?3, recorded_at),
                            updated_at = CURRENT_TIMESTAMP
                        WHERE id = ?1
                        ",
                        params![clip_id, duration_ms, match_record.match_time.as_deref()],
                    )
                    .map_err(|error| {
                        format!("Database updating official clip fields failed: {error}")
                    })?;

                metadata_transaction
                    .execute(
                        "
                        INSERT INTO clip_metadata (
                            clip_id,
                            metadata_status,
                            agent_name,
                            map_name,
                            game_mode,
                            match_id,
                            scoreline,
                            kda,
                            kill_count,
                            official_video_id,
                            official_video_name,
                            official_video_type,
                            highlight_type,
                            round_score,
                            round_score_source,
                            metadata_source
                        )
                        VALUES (
                            ?1, 'enriched', ?2, ?3, ?4, ?5, ?6, ?7,
                            ?8, ?9, ?10, ?11, ?12, ?13, ?14, 'wonderful_db'
                        )
                        ON CONFLICT(clip_id) DO UPDATE SET
                            metadata_status = 'enriched',
                            agent_name = COALESCE(excluded.agent_name, clip_metadata.agent_name),
                            map_name = COALESCE(excluded.map_name, clip_metadata.map_name),
                            game_mode = COALESCE(excluded.game_mode, clip_metadata.game_mode),
                            match_id = excluded.match_id,
                            scoreline = COALESCE(excluded.scoreline, clip_metadata.scoreline),
                            kda = COALESCE(excluded.kda, clip_metadata.kda),
                            kill_count = excluded.kill_count,
                            official_video_id = excluded.official_video_id,
                            official_video_name = excluded.official_video_name,
                            official_video_type = excluded.official_video_type,
                            highlight_type = excluded.highlight_type,
                            round_score = CASE
                                WHEN excluded.round_score_source = 'wonderful_db'
                                    THEN excluded.round_score
                                WHEN NULLIF(TRIM(excluded.official_video_id), '') IS NOT NULL
                                 AND LOWER(TRIM(clip_metadata.official_video_id)) =
                                     LOWER(TRIM(excluded.official_video_id))
                                 AND NULLIF(TRIM(excluded.match_id), '') IS NOT NULL
                                 AND LOWER(TRIM(clip_metadata.match_id)) =
                                     LOWER(TRIM(excluded.match_id))
                                 AND clip_metadata.round_score_source = 'wonderful_db'
                                    THEN clip_metadata.round_score
                                WHEN excluded.round_score IS NOT NULL
                                    THEN excluded.round_score
                                WHEN NULLIF(TRIM(excluded.official_video_id), '') IS NOT NULL
                                 AND LOWER(TRIM(clip_metadata.official_video_id)) =
                                     LOWER(TRIM(excluded.official_video_id))
                                 AND NULLIF(TRIM(excluded.match_id), '') IS NOT NULL
                                 AND LOWER(TRIM(clip_metadata.match_id)) =
                                     LOWER(TRIM(excluded.match_id))
                                 AND clip_metadata.round_score_source = 'highlight_log_delta'
                                 AND ?15
                                    THEN clip_metadata.round_score
                                ELSE NULL
                            END,
                            round_score_source = CASE
                                WHEN excluded.round_score_source = 'wonderful_db'
                                    THEN excluded.round_score_source
                                WHEN NULLIF(TRIM(excluded.official_video_id), '') IS NOT NULL
                                 AND LOWER(TRIM(clip_metadata.official_video_id)) =
                                     LOWER(TRIM(excluded.official_video_id))
                                 AND NULLIF(TRIM(excluded.match_id), '') IS NOT NULL
                                 AND LOWER(TRIM(clip_metadata.match_id)) =
                                     LOWER(TRIM(excluded.match_id))
                                 AND clip_metadata.round_score_source = 'wonderful_db'
                                    THEN clip_metadata.round_score_source
                                WHEN excluded.round_score IS NOT NULL
                                    THEN excluded.round_score_source
                                WHEN NULLIF(TRIM(excluded.official_video_id), '') IS NOT NULL
                                 AND LOWER(TRIM(clip_metadata.official_video_id)) =
                                     LOWER(TRIM(excluded.official_video_id))
                                 AND NULLIF(TRIM(excluded.match_id), '') IS NOT NULL
                                 AND LOWER(TRIM(clip_metadata.match_id)) =
                                     LOWER(TRIM(excluded.match_id))
                                 AND clip_metadata.round_score_source = 'highlight_log_delta'
                                 AND ?15
                                    THEN clip_metadata.round_score_source
                                ELSE NULL
                            END,
                            metadata_source = 'wonderful_db',
                            updated_at = CURRENT_TIMESTAMP
                        ",
                        params![
                            clip_id,
                            match_record.agent_name.as_deref(),
                            match_record.map_name.as_deref(),
                            match_record.game_mode.as_deref(),
                            match_record.match_id,
                            match_record.scoreline.as_deref(),
                            match_record.kda.as_deref(),
                            kill_count,
                            video.video_id,
                            video.video_name,
                            video.video_type,
                            highlight_type,
                            round_score,
                            round_score_source,
                            can_preserve_reconstructed_score,
                        ],
                    )
                    .map_err(|error| {
                        format!("Database updating official clip metadata failed: {error}")
                    })?;
                let stored_log_score = if round_score_source == Some("highlight_log_delta") {
                    metadata_transaction
                        .query_row(
                            "SELECT round_score_source = 'highlight_log_delta' FROM clip_metadata WHERE clip_id = ?1",
                            params![clip_id],
                            |row| row.get::<_, bool>(0),
                        )
                        .map_err(|error| {
                            format!("Database reading official score provenance failed: {error}")
                        })?
                } else {
                    false
                };
                metadata_transaction.commit().map_err(|error| {
                    format!("Database committing official clip metadata failed: {error}")
                })?;

                let segments = timeline
                    .segments
                    .iter()
                    .map(|segment| ClipSegmentInput {
                        segment_key: &segment.segment_key,
                        round_id: segment.round_id,
                        start_ms: segment.start_ms,
                        duration_ms: segment.duration_ms,
                        game_start_ms: None,
                        game_end_ms: None,
                    })
                    .collect::<Vec<_>>();
                let events = timeline
                    .events
                    .iter()
                    .map(|event| ClipEventInput {
                        segment_key: Some(&event.segment_key),
                        event_key: &event.event_key,
                        event_type: &event.event_type,
                        video_time_ms: event.video_time_ms,
                        event_time: event.event_time.as_deref(),
                        round_id: event.round_id,
                        player_name: event.player_name.as_deref(),
                        agent_name: event.agent_name.as_deref(),
                        weapon_name: event.weapon_name.as_deref(),
                        killer_name: event.killer_name.as_deref(),
                        killed_name: event.killed_name.as_deref(),
                        killer_is_me: event.killer_is_me,
                        killed_is_me: event.killed_is_me,
                        raw_json: Some(&event.raw_json),
                    })
                    .collect::<Vec<_>>();
                db::replace_clip_timeline(connection, clip_id, &segments, &events)?;

                summary.matched_video_count += 1;
                summary.event_count += events.len();
                if stored_log_score {
                    summary.round_score_backfilled_count += 1;
                }
            }
        }

        if let Some(account_name) = latest_account_name {
            db::propagate_authoritative_account_name_hints(
                connection,
                &[db::AccountNameHint {
                    account_id: account.openid.clone(),
                    account_name,
                }],
                None,
            )?;
        }
    }

    Ok(summary)
}

fn build_round_score_lookup(
    round_scores: &[HighlightLogRoundScore],
) -> HashMap<(String, String, i64), i64> {
    let mut candidates = HashMap::<(String, String, i64), Option<i64>>::new();
    for score in round_scores {
        let account_id = normalize_account_id(&score.account_id);
        let match_id = normalize_match_id(&score.match_id);
        if account_id.is_empty() || match_id.is_empty() || score.round_id < 0 || score.score < 0 {
            continue;
        }

        let entry = candidates
            .entry((account_id, match_id, score.round_id))
            .or_insert(Some(score.score));
        if entry.is_some_and(|existing| existing != score.score) {
            *entry = None;
        }
    }

    candidates
        .into_iter()
        .filter_map(|(key, score)| score.map(|score| (key, score)))
        .collect()
}

fn resolved_round_score(
    account_id: &str,
    match_record: &WonderfulMatchRecord,
    video: &WonderfulVideoRecord,
    round_scores: &HashMap<(String, String, i64), i64>,
) -> (Option<i64>, Option<&'static str>) {
    if let Some(score) = video.round_score {
        return (Some(score), Some("wonderful_db"));
    }
    if !supports_official_round_score(match_record, video) {
        return (None, None);
    }
    let Some(round_id) = single_round_id(video) else {
        return (None, None);
    };
    let key = (
        normalize_account_id(account_id),
        normalize_match_id(&match_record.match_id),
        round_id,
    );
    match round_scores.get(&key).copied() {
        Some(score) => (Some(score), Some("highlight_log_delta")),
        None => (None, None),
    }
}

fn supports_official_round_score(
    match_record: &WonderfulMatchRecord,
    video: &WonderfulVideoRecord,
) -> bool {
    let supported_mode = matches!(
        match_record.game_mode.as_deref().map(str::trim),
        Some("普通模式" | "极速模式" | "竞技模式")
    );
    if !supported_mode {
        return false;
    }

    video.video_type.trim().ends_with("杀时刻")
        || matches!(
            video
                .highlight_type
                .or_else(|| video.video_type.trim().parse::<i64>().ok()),
            Some(4 | 6 | 10)
        )
}

fn single_round_id(video: &WonderfulVideoRecord) -> Option<i64> {
    let mut round_id = None;
    for segment in &video.segments {
        let segment_round_id = segment.round_id?;
        match round_id {
            Some(existing) if existing != segment_round_id => return None,
            Some(_) => {}
            None => round_id = Some(segment_round_id),
        }
    }
    round_id
}

fn normalize_match_id(match_id: &str) -> String {
    match_id.trim().to_ascii_lowercase()
}

fn normalize_account_id(account_id: &str) -> String {
    account_id.trim().to_ascii_lowercase()
}

pub fn ingest_wonderful_snapshots(
    connection: &Connection,
    accounts: &[WonderfulSnapshotAccountRecord],
) -> DbResult<WonderfulSnapshotIngestSummary> {
    let mut summary = WonderfulSnapshotIngestSummary::default();
    let mut matched_rows = HashSet::new();

    for account in accounts {
        for snapshot in &account.snapshots {
            let match_id = snapshot.match_record.match_id.trim();
            let snapshot_id = snapshot.snapshot_id.trim();
            if snapshot_id.is_empty() {
                continue;
            }

            let match_row_id = if match_id.is_empty() {
                None
            } else {
                upsert_wonderful_match(connection, &account.openid, &snapshot.match_record)?;
                Some(
                    connection
                        .query_row(
                            "SELECT id FROM matches WHERE game_id = ?1",
                            params![match_id],
                            |row| row.get::<_, i64>(0),
                        )
                        .map_err(|error| {
                            format!("Database reading snapshot match id failed: {error}")
                        })?,
                )
            };
            if let (Some(match_row_id), Some(account_name)) =
                (match_row_id, snapshot.account_name.as_deref())
            {
                connection
                    .execute(
                        "
                        UPDATE matches
                        SET player_name = COALESCE(player_name, ?2),
                            updated_at = CURRENT_TIMESTAMP
                        WHERE id = ?1
                        ",
                        params![match_row_id, account_name],
                    )
                    .map_err(|error| {
                        format!("Database backfilling snapshot account name failed: {error}")
                    })?;
            }
            connection
                .execute(
                    "
                    INSERT INTO match_snapshots (
                        match_id,
                        snapshot_id,
                        account_id,
                        captured_at,
                        account_name,
                        package_path,
                        thumb_path,
                        width,
                        height,
                        size_bytes,
                        raw_json
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    ON CONFLICT(snapshot_id) DO UPDATE SET
                        match_id = excluded.match_id,
                        account_id = excluded.account_id,
                        captured_at = COALESCE(excluded.captured_at, match_snapshots.captured_at),
                        account_name = COALESCE(excluded.account_name, match_snapshots.account_name),
                        package_path = COALESCE(excluded.package_path, match_snapshots.package_path),
                        thumb_path = COALESCE(excluded.thumb_path, match_snapshots.thumb_path),
                        width = COALESCE(excluded.width, match_snapshots.width),
                        height = COALESCE(excluded.height, match_snapshots.height),
                        size_bytes = COALESCE(excluded.size_bytes, match_snapshots.size_bytes),
                        raw_json = excluded.raw_json,
                        updated_at = CURRENT_TIMESTAMP
                    ",
                    params![
                        match_row_id,
                        snapshot_id,
                        account.openid,
                        snapshot.captured_at.as_deref(),
                        snapshot.account_name.as_deref(),
                        snapshot.package_path.as_deref(),
                        snapshot.thumb_path.as_deref(),
                        snapshot.width,
                        snapshot.height,
                        snapshot.size_bytes,
                        snapshot.raw_json,
                    ],
                )
                .map_err(|error| format!("Database upserting WonderfulDb snapshot failed: {error}"))?;
            summary.snapshot_count += 1;
            if let Some(match_row_id) = match_row_id {
                matched_rows.insert(match_row_id);
            }
        }
        if let Some(account_name) = latest_valid_snapshot_account_name(account) {
            db::propagate_authoritative_account_name_hints(
                connection,
                &[db::AccountNameHint {
                    account_id: account.openid.clone(),
                    account_name,
                }],
                None,
            )?;
        }
    }

    summary.match_count = matched_rows.len();
    Ok(summary)
}

fn latest_valid_snapshot_account_name(account: &WonderfulSnapshotAccountRecord) -> Option<String> {
    latest_valid_snapshot_account_name_candidate(account).map(|candidate| candidate.account_name)
}

fn latest_valid_snapshot_account_name_candidate(
    account: &WonderfulSnapshotAccountRecord,
) -> Option<AccountNameCandidate> {
    account
        .snapshots
        .iter()
        .enumerate()
        .filter_map(|(index, snapshot)| {
            let account_name = snapshot
                .account_name
                .as_deref()
                .and_then(valid_wonderful_player_name)?;
            let timestamp = nonempty_trimmed(snapshot.captured_at.as_deref())
                .or_else(|| nonempty_trimmed(snapshot.match_record.match_time.as_deref()));
            Some(AccountNameCandidate {
                account_name: account_name.to_string(),
                observed_at: timestamp.map(str::to_string),
                source_priority: 3,
                traversal_order: index,
            })
        })
        .max_by(account_name_candidate_ordering)
}

fn upsert_wonderful_match(
    connection: &Connection,
    openid: &str,
    match_record: &WonderfulMatchRecord,
) -> DbResult<()> {
    if match_record.match_id.trim().is_empty() {
        return Ok(());
    }

    connection
        .execute(
            "
            INSERT INTO matches (
                game_id,
                battle_id,
                account_id,
                agent_name,
                agent_avatar_url,
                map_id,
                map_name,
                game_mode,
                started_at,
                source_leveldb,
                source_log
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0)
            ON CONFLICT(game_id) DO UPDATE SET
                battle_id = COALESCE(excluded.battle_id, matches.battle_id),
                account_id = excluded.account_id,
                agent_name = COALESCE(excluded.agent_name, matches.agent_name),
                agent_avatar_url = COALESCE(excluded.agent_avatar_url, matches.agent_avatar_url),
                map_id = COALESCE(excluded.map_id, matches.map_id),
                map_name = COALESCE(excluded.map_name, matches.map_name),
                game_mode = COALESCE(excluded.game_mode, matches.game_mode),
                started_at = COALESCE(excluded.started_at, matches.started_at),
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                match_record.match_id,
                match_record.battle_id.as_deref(),
                openid,
                match_record.agent_name.as_deref(),
                match_record.agent_avatar_url.as_deref(),
                match_record.map_id.as_deref(),
                match_record.map_name.as_deref(),
                match_record.game_mode.as_deref(),
                match_record.match_time.as_deref(),
            ],
        )
        .map_err(|error| format!("Database upserting WonderfulDb match failed: {error}"))?;

    connection
        .execute(
            "
            UPDATE clip_metadata
            SET agent_name = COALESCE(?2, agent_name),
                map_name = COALESCE(?3, map_name),
                game_mode = COALESCE(?4, game_mode),
                scoreline = COALESCE(?5, scoreline),
                kda = COALESCE(?6, kda),
                updated_at = CURRENT_TIMESTAMP
            WHERE match_id = ?1
              AND metadata_source = 'wonderful_db'
            ",
            params![
                match_record.match_id,
                match_record.agent_name.as_deref(),
                match_record.map_name.as_deref(),
                match_record.game_mode.as_deref(),
                match_record.scoreline.as_deref(),
                match_record.kda.as_deref(),
            ],
        )
        .map_err(|error| {
            format!("Database backfilling WonderfulDb clip metadata failed: {error}")
        })?;
    connection
        .execute(
            "
            UPDATE clips
            SET recorded_at = COALESCE(?2, recorded_at),
                updated_at = CURRENT_TIMESTAMP
            WHERE id IN (
                SELECT clip_id
                FROM clip_metadata
                WHERE match_id = ?1
                  AND metadata_source = 'wonderful_db'
            )
            ",
            params![match_record.match_id, match_record.match_time.as_deref()],
        )
        .map_err(|error| format!("Database backfilling WonderfulDb clip time failed: {error}"))?;

    let match_row_id = connection
        .query_row(
            "SELECT id FROM matches WHERE game_id = ?1",
            params![match_record.match_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Database reading WonderfulDb match id failed: {error}"))?;
    let (kills, deaths, assists) = parse_stat_triplet(match_record.kda.as_deref());
    let (rounds_won, rounds_lost) = parse_scoreline(match_record.scoreline.as_deref());
    let has_stats = kills.is_some()
        || deaths.is_some()
        || assists.is_some()
        || match_record.combat_score.is_some()
        || rounds_won.is_some()
        || rounds_lost.is_some()
        || match_record.has_won.is_some();
    if !has_stats {
        return Ok(());
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
                kills = COALESCE(excluded.kills, match_stats.kills),
                deaths = COALESCE(excluded.deaths, match_stats.deaths),
                assists = COALESCE(excluded.assists, match_stats.assists),
                combat_score = COALESCE(excluded.combat_score, match_stats.combat_score),
                rounds_won = COALESCE(excluded.rounds_won, match_stats.rounds_won),
                rounds_lost = COALESCE(excluded.rounds_lost, match_stats.rounds_lost),
                rounds_played = COALESCE(excluded.rounds_played, match_stats.rounds_played),
                has_won = COALESCE(excluded.has_won, match_stats.has_won),
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                match_row_id,
                kills,
                deaths,
                assists,
                match_record.combat_score,
                rounds_won,
                rounds_lost,
                rounds_won.zip(rounds_lost).map(|(won, lost)| won + lost),
                match_record.has_won.map(i64::from),
            ],
        )
        .map_err(|error| format!("Database upserting WonderfulDb match stats failed: {error}"))?;

    Ok(())
}

fn parse_stat_triplet(value: Option<&str>) -> (Option<i64>, Option<i64>, Option<i64>) {
    let mut parts = value.unwrap_or_default().split('/');
    let kills = parts.next().and_then(|part| part.trim().parse().ok());
    let deaths = parts.next().and_then(|part| part.trim().parse().ok());
    let assists = parts.next().and_then(|part| part.trim().parse().ok());
    (kills, deaths, assists)
}

fn parse_scoreline(value: Option<&str>) -> (Option<i64>, Option<i64>) {
    let mut parts = value.unwrap_or_default().split('/');
    let rounds_won = parts.next().and_then(|part| part.trim().parse().ok());
    let rounds_lost = parts.next().and_then(|part| part.trim().parse().ok());
    (rounds_won, rounds_lost)
}

fn latest_valid_account_name(account: &WonderfulAccountRecord) -> Option<String> {
    latest_valid_account_name_candidate(account).map(|candidate| candidate.account_name)
}

fn latest_valid_account_name_candidate(
    account: &WonderfulAccountRecord,
) -> Option<AccountNameCandidate> {
    let mut best = None;
    let mut traversal_order = 0usize;

    for match_record in &account.matches {
        let match_time = nonempty_trimmed(match_record.match_time.as_deref());
        traversal_order += 1;
        if let Some(account_name) = match_record
            .account_name
            .as_deref()
            .and_then(valid_wonderful_player_name)
        {
            replace_with_newer_account_name_candidate(
                &mut best,
                AccountNameCandidate {
                    account_name: account_name.to_string(),
                    observed_at: match_time.map(str::to_string),
                    source_priority: 1,
                    traversal_order,
                },
            );
        }
        for video in &match_record.videos {
            for segment in &video.segments {
                for event in &segment.events {
                    traversal_order += 1;
                    let Some(account_name) = event
                        .player_name
                        .as_deref()
                        .and_then(valid_wonderful_player_name)
                    else {
                        continue;
                    };
                    let event_time = nonempty_trimmed(event.event_time.as_deref());
                    replace_with_newer_account_name_candidate(
                        &mut best,
                        AccountNameCandidate {
                            account_name: account_name.to_string(),
                            observed_at: match_time.or(event_time).map(str::to_string),
                            source_priority: 2,
                            traversal_order,
                        },
                    );
                }
            }
        }
    }

    best
}

fn replace_with_newer_account_name_candidate(
    current: &mut Option<AccountNameCandidate>,
    candidate: AccountNameCandidate,
) {
    if current
        .as_ref()
        .is_none_or(|current| account_name_candidate_is_newer(&candidate, current))
    {
        *current = Some(candidate);
    }
}

fn account_name_candidate_is_newer(
    candidate: &AccountNameCandidate,
    current: &AccountNameCandidate,
) -> bool {
    account_name_candidate_ordering(candidate, current).is_gt()
}

fn account_name_candidate_ordering(
    left: &AccountNameCandidate,
    right: &AccountNameCandidate,
) -> std::cmp::Ordering {
    db::compare_account_name_observed_at(left.observed_at.as_deref(), right.observed_at.as_deref())
        .then_with(|| left.source_priority.cmp(&right.source_priority))
        .then_with(|| left.traversal_order.cmp(&right.traversal_order))
}

fn nonempty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn valid_wonderful_player_name(value: &str) -> Option<&str> {
    db::valid_tagged_account_name(value)
}

fn find_clip_for_video(
    connection: &Connection,
    openid: &str,
    match_id: &str,
    video: &WonderfulVideoRecord,
) -> DbResult<Option<i64>> {
    if let Some(video_src) = video.video_src.as_deref() {
        let normalized_src = db::normalize_path(video_src);
        if !normalized_src.is_empty() {
            let matched = connection
                .query_row(
                    "
                    SELECT clips.id, source_dirs.name, source_dirs.path
                    FROM clips
                    JOIN source_dirs ON source_dirs.id = clips.source_dir_id
                    JOIN clip_groups ON clip_groups.id = clips.clip_group_id
                    WHERE clips.normalized_path = ?1
                      AND (?2 = '' OR clip_groups.group_key = ?2)
                    ",
                    params![normalized_src, match_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| {
                    format!("Database matching official video path failed: {error}")
                })?;
            if let Some((clip_id, source_name, source_path)) = matched {
                if db::source_openid(&source_name, &source_path).as_deref() == Some(openid) {
                    return Ok(Some(clip_id));
                }
            }
        }
    }

    let mut statement = connection
        .prepare(
            "
            SELECT clips.id, clips.file_name, source_dirs.name, source_dirs.path
            FROM clips
            JOIN clip_groups ON clip_groups.id = clips.clip_group_id
            JOIN source_dirs ON source_dirs.id = clips.source_dir_id
            WHERE clip_groups.group_key = ?1
            ORDER BY clips.id
            ",
        )
        .map_err(|error| {
            format!("Database preparing official video fallback match failed: {error}")
        })?;
    let candidates = statement
        .query_map(params![match_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("Database matching official video fallback failed: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Database reading official video fallback failed: {error}"))?;
    let matching_ids = candidates
        .into_iter()
        .filter_map(|(clip_id, file_name, source_name, source_path)| {
            (db::source_openid(&source_name, &source_path).as_deref() == Some(openid)
                && Path::new(&file_name)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    == Some(video.video_id.as_str()))
            .then_some(clip_id)
        })
        .collect::<Vec<_>>();

    Ok(match matching_ids.as_slice() {
        [clip_id] => Some(*clip_id),
        _ => None,
    })
}

fn video_duration_ms(video: &WonderfulVideoRecord) -> Option<i64> {
    video
        .segments
        .iter()
        .filter_map(|segment| segment.clip_end_ms)
        .max()
}

#[derive(Debug)]
struct OwnedTimeline {
    segments: Vec<OwnedSegment>,
    events: Vec<OwnedEvent>,
    warning_count: usize,
    warnings: Vec<String>,
}

#[derive(Debug)]
struct OwnedSegment {
    segment_key: String,
    round_id: Option<i64>,
    start_ms: i64,
    duration_ms: i64,
}

#[derive(Debug)]
struct OwnedEvent {
    segment_key: String,
    event_key: String,
    event_type: String,
    video_time_ms: Option<i64>,
    event_time: Option<String>,
    round_id: Option<i64>,
    player_name: Option<String>,
    agent_name: Option<String>,
    weapon_name: Option<String>,
    killer_name: Option<String>,
    killed_name: Option<String>,
    killer_is_me: bool,
    killed_is_me: Option<bool>,
    raw_json: String,
}

fn build_timeline(video: &WonderfulVideoRecord, known_duration_ms: Option<i64>) -> OwnedTimeline {
    let mut segment_keys = HashSet::new();
    let mut event_keys = HashSet::new();
    let mut segments = Vec::new();
    let mut events = Vec::new();
    let mut warning_count = 0usize;
    let mut warnings = Vec::new();

    for (segment_index, segment) in video.segments.iter().enumerate() {
        let segment_base = if segment.segment_id.trim().is_empty() {
            format!("segment-{segment_index}")
        } else {
            segment.segment_id.clone()
        };
        let segment_key = unique_key(segment_base, segment_index, &mut segment_keys);
        let start_ms = segment.clip_start_ms.unwrap_or(0);
        let duration_ms = segment
            .clip_end_ms
            .map(|end_ms| end_ms.saturating_sub(start_ms).max(0))
            .unwrap_or(0);
        segments.push(OwnedSegment {
            segment_key: segment_key.clone(),
            round_id: segment.round_id,
            start_ms,
            duration_ms,
        });

        for event in &segment.events {
            let event_base = if event.event_id.trim().is_empty() {
                event_content_key(&segment_key, event)
            } else {
                event.event_id.clone()
            };
            if !event_keys.insert(event_base.clone()) {
                continue;
            }
            let mut event_warnings = event.normalization_warnings.clone();
            let video_time_ms = validated_timeline_event_time(
                event.video_time_ms,
                known_duration_ms,
                &mut event_warnings,
            );
            warning_count = warning_count.saturating_add(event_warnings.len());
            for warning in event_warnings {
                if warnings.len() >= MAX_INGEST_WARNING_SAMPLES {
                    break;
                }
                warnings.push(format_timeline_warning(video, event, warning));
            }
            events.push(OwnedEvent {
                segment_key: segment_key.clone(),
                event_key: event_base,
                event_type: event.event_type.clone(),
                video_time_ms,
                event_time: event.event_time.clone(),
                round_id: event.round_id,
                player_name: event.player_name.clone(),
                agent_name: event.agent_name.clone(),
                weapon_name: event.weapon_name.clone(),
                killer_name: event.killer_name.clone(),
                killed_name: event.killed_name.clone(),
                killer_is_me: event.killer_is_me,
                killed_is_me: event.killed_is_me,
                raw_json: event.raw_json.clone(),
            });
        }
    }

    OwnedTimeline {
        segments,
        events,
        warning_count,
        warnings,
    }
}

fn validated_timeline_event_time(
    video_time_ms: Option<i64>,
    duration_ms: Option<i64>,
    warnings: &mut Vec<WonderfulEventNormalizationWarning>,
) -> Option<i64> {
    let video_time_ms = video_time_ms?;
    if video_time_ms < 0 || duration_ms.is_some_and(|duration_ms| video_time_ms > duration_ms) {
        if !warnings.contains(&WonderfulEventNormalizationWarning::VideoTimeOutOfBounds) {
            warnings.push(WonderfulEventNormalizationWarning::VideoTimeOutOfBounds);
        }
        return None;
    }
    Some(video_time_ms)
}

fn format_timeline_warning(
    video: &WonderfulVideoRecord,
    event: &WonderfulEventRecord,
    warning: WonderfulEventNormalizationWarning,
) -> String {
    let warning_code = match warning {
        WonderfulEventNormalizationWarning::InvalidTopLevelKilledIsMe => {
            "invalid-top-level-killed-is-me"
        }
        WonderfulEventNormalizationWarning::InvalidExtendedKilledIsMe => {
            "invalid-extended-killed-is-me"
        }
        WonderfulEventNormalizationWarning::VideoTimeOverflow => "video-time-overflow",
        WonderfulEventNormalizationWarning::VideoTimeOutOfBounds => "video-time-out-of-bounds",
    };
    format!(
        "WonderfulDb video {} event {}: {warning_code}",
        bounded_warning_identifier(&video.video_id),
        bounded_warning_identifier(&event.event_id),
    )
}

fn bounded_warning_identifier(value: &str) -> String {
    const MAX_IDENTIFIER_CHARS: usize = 64;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "<missing>".to_string();
    }
    let mut characters = trimmed.chars();
    let mut bounded = characters
        .by_ref()
        .take(MAX_IDENTIFIER_CHARS)
        .collect::<String>();
    if characters.next().is_some() {
        bounded.push('…');
    }
    bounded
}

fn unique_key(base: String, index: usize, used: &mut HashSet<String>) -> String {
    let mut candidate = base.clone();
    let mut suffix = index;
    loop {
        if used.insert(candidate.clone()) {
            return candidate;
        }
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
}

fn event_content_key(segment_key: &str, event: &WonderfulEventRecord) -> String {
    let mut hasher = Sha256::new();
    hash_text(&mut hasher, segment_key);
    hash_text(&mut hasher, &event.event_type);
    hash_optional_text(&mut hasher, event.event_time.as_deref());
    hash_optional_i64(&mut hasher, event.round_id);
    hash_optional_i64(&mut hasher, event.video_time_ms);
    hash_optional_text(&mut hasher, event.player_name.as_deref());
    hash_optional_text(&mut hasher, event.agent_name.as_deref());
    hash_optional_text(&mut hasher, event.killer_name.as_deref());
    hash_optional_text(&mut hasher, event.killed_name.as_deref());
    hash_optional_text(&mut hasher, event.weapon_name.as_deref());
    hasher.update([u8::from(event.killer_is_me)]);
    hash_optional_bool(&mut hasher, event.killed_is_me);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn hash_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn hash_optional_text(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_text(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_i64(hasher: &mut Sha256, value: Option<i64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_le_bytes());
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_bool(hasher: &mut Sha256, value: Option<bool>) {
    match value {
        Some(value) => hasher.update([1, u8::from(value)]),
        None => hasher.update([0]),
    }
}

#[cfg(test)]
mod account_name_tests {
    // Account IDs and player names below are synthetic fixtures, not captured user data.
    use super::{
        latest_valid_account_name, latest_wonderful_account_name_hints, valid_wonderful_player_name,
    };
    use crate::wonderful_db::{
        WonderfulAccountRecord, WonderfulEventRecord, WonderfulMatchRecord, WonderfulSegmentRecord,
        WonderfulSnapshotAccountRecord, WonderfulSnapshotRecord, WonderfulVideoRecord,
    };

    #[test]
    fn validates_wonderful_player_names_without_rewriting_unicode() {
        assert_eq!(
            valid_wonderful_player_name("  测试玩家甲 #0001  "),
            Some("测试玩家甲 #0001")
        );
        for invalid in [
            "",
            "undefined",
            "NULL",
            "wonderfulVideos90000000000000000006",
            "90000000000000000006",
            "90000000000000000006#0000",
            "undefined#0000",
            "Cards/D3018FBE.png#1001",
            r"C:\assets\card.png#1001",
            "https://assets.example/card.png#1001",
            "card.png#1001",
            "MissingTag#",
            "#MissingName",
            "Too#Many#Parts",
        ] {
            assert_eq!(valid_wonderful_player_name(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn selects_latest_name_by_match_then_event_time_then_traversal_order() {
        let account = WonderfulAccountRecord {
            openid: "90000000000000000006".to_string(),
            matches: vec![
                named_match(
                    "old-match",
                    Some("2026-07-01T12:00:00Z"),
                    &[(Some("2026-07-01T12:00:02Z"), "OldName#1001")],
                ),
                named_match(
                    "new-match",
                    Some("2026-07-04T12:00:00Z"),
                    &[
                        (Some("2026-07-04T12:00:01Z"), "NewName#2002"),
                        (Some("2026-07-04T12:00:03Z"), "NewestName#3003"),
                    ],
                ),
            ],
        };

        assert_eq!(
            latest_valid_account_name(&account).as_deref(),
            Some("NewestName#3003")
        );

        let traversal_tie = WonderfulAccountRecord {
            openid: account.openid.clone(),
            matches: vec![named_match(
                "tie-match",
                None,
                &[(None, "First#1001"), (None, "Second#2002")],
            )],
        };
        assert_eq!(
            latest_valid_account_name(&traversal_tie).as_deref(),
            Some("Second#2002")
        );
    }

    #[test]
    fn uses_match_envelope_name_when_events_omit_player_name() {
        let mut older = named_match("old-match", Some("2026-07-01T12:00:00Z"), &[]);
        older.account_name = Some("OldName#1001".to_string());
        let mut newer = named_match("new-match", Some("2026-07-04T12:00:00Z"), &[]);
        newer.account_name = Some("CurrentName#2002".to_string());
        let account = WonderfulAccountRecord {
            openid: "90000000000000000006".to_string(),
            // Deliberately reverse chronological order to prove selection uses timestamps rather
            // than whichever database row happened to be traversed last.
            matches: vec![newer, older],
        };

        assert_eq!(
            latest_valid_account_name(&account).as_deref(),
            Some("CurrentName#2002")
        );
    }

    #[test]
    fn resolves_snapshot_and_match_names_by_observed_time_instead_of_ingest_order() {
        let openid = "90000000000000000006";
        let matches = vec![WonderfulAccountRecord {
            openid: openid.to_string(),
            matches: vec![named_match(
                "older-match",
                Some("100"),
                &[(None, "OlderMatchName#1001")],
            )],
        }];
        let snapshots = vec![WonderfulSnapshotAccountRecord {
            openid: openid.to_string(),
            snapshots: vec![WonderfulSnapshotRecord {
                match_record: named_match("snapshot-match", Some("150"), &[]),
                snapshot_id: "snapshot-1".to_string(),
                captured_at: Some("200".to_string()),
                account_name: Some("NewerSnapshotName#2002".to_string()),
                package_path: None,
                thumb_path: None,
                width: None,
                height: None,
                size_bytes: None,
                raw_json: "{}".to_string(),
            }],
        }];

        let snapshot_wins = latest_wonderful_account_name_hints(&matches, &snapshots);
        assert_eq!(snapshot_wins.len(), 1);
        assert_eq!(snapshot_wins[0].account_name, "NewerSnapshotName#2002");

        let mut newer_matches = matches;
        newer_matches[0].matches[0].match_time = Some("300".to_string());
        let match_wins = latest_wonderful_account_name_hints(&newer_matches, &snapshots);
        assert_eq!(match_wins.len(), 1);
        assert_eq!(match_wins[0].account_name, "OlderMatchName#1001");
    }

    fn named_match(
        match_id: &str,
        match_time: Option<&str>,
        events: &[(Option<&str>, &str)],
    ) -> WonderfulMatchRecord {
        WonderfulMatchRecord {
            match_id: match_id.to_string(),
            match_time: match_time.map(str::to_string),
            map_name: None,
            career: None,
            videos: vec![WonderfulVideoRecord {
                video_id: format!("{match_id}-video"),
                video_name: "测试视频".to_string(),
                video_type: "1".to_string(),
                highlight_type: Some(1),
                video_src: None,
                round_score: None,
                segments: vec![WonderfulSegmentRecord {
                    segment_id: "segment-1".to_string(),
                    round_id: Some(1),
                    clip_start_ms: Some(0),
                    clip_end_ms: Some(1_000),
                    events: events
                        .iter()
                        .enumerate()
                        .map(|(index, (event_time, player_name))| WonderfulEventRecord {
                            event_id: format!("event-{index}"),
                            event_type: "kill".to_string(),
                            video_time_ms: Some(index as i64 * 100),
                            event_time: event_time.map(str::to_string),
                            round_id: Some(1),
                            player_name: Some((*player_name).to_string()),
                            agent_name: None,
                            weapon_name: None,
                            killer_name: None,
                            killed_name: None,
                            killer_is_me: true,
                            killed_is_me: None,
                            normalization_warnings: Vec::new(),
                            raw_json: "{}".to_string(),
                        })
                        .collect(),
                }],
            }],
            ..WonderfulMatchRecord::default()
        }
    }
}
