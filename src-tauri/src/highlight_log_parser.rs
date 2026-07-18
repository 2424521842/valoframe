use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose, Engine as _};
use flate2::read::GzDecoder;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::display_names;

const HIGHLIGHT_LOG_CANDIDATES: &[&str] = &[
    "highlight.log",
    "highlight.old.log",
    "main.log",
    "main.old.log",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HighlightLogLineKind {
    FirstRequestData,
    TemplateParam,
    EventParser,
    TlogRequest,
    BattleListResponse,
    VideoListData,
    PostSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightLogParseResult {
    pub records: Vec<HighlightLogRecord>,
    pub round_scores: Vec<HighlightLogRoundScore>,
    pub account_name_hints: Vec<HighlightLogAccountNameHint>,
    pub bad_line_count: usize,
    pub gzip_event_count: usize,
    #[serde(skip)]
    round_score_conflicts: HashSet<(String, String, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightLogRoundScore {
    pub account_id: String,
    pub match_id: String,
    pub round_id: i64,
    pub score: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightLogAccountNameHint {
    pub account_id: String,
    pub account_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightLogRecord {
    pub line_kind: HighlightLogLineKind,
    pub match_id: Option<String>,
    pub battle_id: Option<String>,
    pub record_src: Option<String>,
    pub player_name: Option<String>,
    pub map_id: Option<String>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub agent_name: Option<String>,
    pub kda: Option<String>,
    pub scoreline: Option<String>,
    pub has_won: Option<bool>,
    pub combat_score: Option<i64>,
    pub kill_events: Vec<HighlightLogKillEvent>,
    pub has_gzip_event: bool,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightLogKillEvent {
    pub event_time: Option<String>,
    pub round_id: Option<i64>,
    pub weapon_name: Option<String>,
    pub killer_name: Option<String>,
    pub killed_name: Option<String>,
    pub raw_json: Option<String>,
}

pub fn parse_highlight_log(path: impl AsRef<Path>) -> Result<HighlightLogParseResult, String> {
    let path = path.as_ref();
    if !path.is_file() {
        return Ok(HighlightLogParseResult::default());
    }

    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read highlight log {}: {error}",
            path_to_string(path)
        )
    })?;

    parse_highlight_log_content(&content)
}

pub fn parse_highlight_logs(logs_dir: impl AsRef<Path>) -> Result<HighlightLogParseResult, String> {
    let logs_dir = logs_dir.as_ref();
    let mut merged = HighlightLogParseResult::default();
    let mut seen_records = HashSet::new();
    let mut seen_account_hints = HashSet::new();

    if !logs_dir.is_dir() {
        return Ok(merged);
    }

    for file_name in HIGHLIGHT_LOG_CANDIDATES {
        let path = logs_dir.join(file_name);
        if !path.is_file() {
            continue;
        }

        let parsed = parse_highlight_log(&path)?;
        merged.bad_line_count += parsed.bad_line_count;
        for hint in parsed.account_name_hints {
            let dedupe_key = format!(
                "{}|{}",
                hint.account_id,
                hint.account_name.as_deref().unwrap_or_default()
            );
            if seen_account_hints.insert(dedupe_key) {
                merged.account_name_hints.push(hint);
            }
        }
        merged.round_scores.extend(parsed.round_scores);
        merged
            .round_score_conflicts
            .extend(parsed.round_score_conflicts);
        for record in parsed.records {
            let dedupe_key = highlight_record_dedupe_key(&record);
            if !seen_records.insert(dedupe_key) {
                continue;
            }
            if record.has_gzip_event {
                merged.gzip_event_count += 1;
            }
            merged.records.push(record);
        }
    }

    finalize_round_scores(&mut merged);
    Ok(merged)
}

pub fn parse_highlight_log_content(content: &str) -> Result<HighlightLogParseResult, String> {
    let mut result = HighlightLogParseResult::default();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(payload) = payload_after_marker(trimmed, "first request data is") {
            parse_payload(payload, HighlightLogLineKind::FirstRequestData, &mut result);
            continue;
        }

        if let Some(payload) = payload_after_marker(trimmed, "template param ==") {
            parse_payload(payload, HighlightLogLineKind::TemplateParam, &mut result);
            continue;
        }

        if let Some(payload) = payload_after_marker(trimmed, "now event parser params is:") {
            parse_event_parser_payload(payload, &mut result);
            continue;
        }

        if let Some(payload) = payload_after_marker(trimmed, "postSnapshot start data") {
            parse_post_snapshot_payload(payload, &mut result);
            continue;
        }

        if let Some(payload) = payload_after_marker(trimmed, "获取视频列表。data:") {
            parse_video_list_data_payload(payload, &mut result);
            continue;
        }

        if let Some(payload) = payload_after_marker(trimmed, "REQUEST：")
            .or_else(|| payload_after_marker(trimmed, "REQUEST:"))
        {
            parse_tlog_request_payload(payload, &mut result);
            continue;
        }

        if let Some(payload) = payload_after_marker(trimmed, "RESPONSE：")
            .or_else(|| payload_after_marker(trimmed, "RESPONSE:"))
        {
            parse_battle_list_response_payload(payload, &mut result);
        }
    }

    finalize_round_scores(&mut result);
    Ok(result)
}

fn highlight_record_dedupe_key(record: &HighlightLogRecord) -> String {
    format!(
        "{:?}|{}|{}|{}|{}",
        record.line_kind,
        record.match_id.as_deref().unwrap_or_default(),
        record.battle_id.as_deref().unwrap_or_default(),
        record.record_src.as_deref().unwrap_or_default(),
        record.raw_json
    )
}

fn finalize_round_scores(result: &mut HighlightLogParseResult) {
    let mut resolved = BTreeMap::<(String, String, i64), Option<i64>>::new();
    for conflict in std::mem::take(&mut result.round_score_conflicts) {
        resolved.insert(conflict, None);
    }

    for candidate in std::mem::take(&mut result.round_scores) {
        let account_id = candidate.account_id.trim().to_ascii_lowercase();
        let match_id = candidate.match_id.trim().to_ascii_lowercase();
        if account_id.is_empty()
            || match_id.is_empty()
            || candidate.round_id < 0
            || candidate.score < 0
        {
            continue;
        }

        let key = (account_id, match_id, candidate.round_id);
        match resolved.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(candidate.score));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if entry.get().is_some_and(|score| score != candidate.score) {
                    entry.insert(None);
                }
            }
        }
    }

    result.round_score_conflicts = resolved
        .iter()
        .filter_map(|(key, score)| score.is_none().then_some(key.clone()))
        .collect();
    result.round_scores = resolved
        .into_iter()
        .filter_map(|((account_id, match_id, round_id), score)| {
            score.map(|score| HighlightLogRoundScore {
                account_id,
                match_id,
                round_id,
                score,
            })
        })
        .collect();
}

fn parse_payload(
    payload: &str,
    line_kind: HighlightLogLineKind,
    result: &mut HighlightLogParseResult,
) {
    let value = match serde_json::from_str::<Value>(payload) {
        Ok(value) => value,
        Err(_) => {
            result.bad_line_count += 1;
            return;
        }
    };

    match value {
        Value::Array(values) => {
            for value in values {
                if !value.is_object() {
                    continue;
                }
                push_record(value, line_kind.clone(), result);
            }
        }
        Value::Object(_) => push_record(value, line_kind, result),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            result.bad_line_count += 1;
        }
    }
}

fn push_record(
    value: Value,
    line_kind: HighlightLogLineKind,
    result: &mut HighlightLogParseResult,
) {
    let record = parse_record(&value, line_kind);
    if record.has_gzip_event {
        result.gzip_event_count += 1;
    }
    result.records.push(record);
}

fn parse_event_parser_payload(payload: &str, result: &mut HighlightLogParseResult) {
    let value = match serde_json::from_str::<Value>(payload) {
        Ok(value) => value,
        Err(_) => {
            result.bad_line_count += 1;
            return;
        }
    };

    let Some(games) = find_array_by_keys(&value, &["games"]) else {
        result.bad_line_count += 1;
        return;
    };

    for game in games {
        let Some(encoded_event) = find_string_by_keys(game, &["event"]) else {
            continue;
        };
        if !is_gzip_payload(&encoded_event) {
            continue;
        }

        result.gzip_event_count += 1;
        match decode_gzip_event_payload(&encoded_event) {
            Some(events) => {
                let record = parse_event_parser_record(game, &events);
                result
                    .round_scores
                    .extend(find_event_parser_round_scores(&events));
                result.records.push(record);
            }
            None => result.bad_line_count += 1,
        }
    }
}

fn parse_tlog_request_payload(payload: &str, result: &mut HighlightLogParseResult) {
    let value = match serde_json::from_str::<Value>(payload) {
        Ok(value) => expand_embedded_json_strings(&value),
        Err(_) => {
            if payload.contains("wonderfulsdk_ui_show") {
                result.bad_line_count += 1;
            }
            return;
        }
    };

    let Some(event_id) = find_string_by_keys(&value, &["eventId", "event_id"]) else {
        return;
    };
    if event_id != "wonderfulsdk_ui_show" {
        return;
    }

    push_record(value, HighlightLogLineKind::TlogRequest, result);
}

fn parse_battle_list_response_payload(payload: &str, result: &mut HighlightLogParseResult) {
    if !payload.contains("battle_list")
        && !payload.contains("battleList")
        && !payload.contains("g_open_id")
    {
        return;
    }

    let value = match serde_json::from_str::<Value>(payload) {
        Ok(value) => expand_and_collect_hints(&value, &mut result.account_name_hints),
        Err(_) => {
            result.bad_line_count += 1;
            return;
        }
    };

    let Some(records) = find_array_by_keys(&value, &["battleList", "battle_list"]) else {
        return;
    };

    for record in records {
        if record.is_object() {
            push_record(
                record.clone(),
                HighlightLogLineKind::BattleListResponse,
                result,
            );
        }
    }
}

fn parse_video_list_data_payload(payload: &str, result: &mut HighlightLogParseResult) {
    let value = match serde_json::from_str::<Value>(payload) {
        Ok(value) => expand_embedded_json_strings(&value),
        Err(_) => {
            result.bad_line_count += 1;
            return;
        }
    };

    let Some(records) = direct_array_by_keys(&value, &["data"]) else {
        result.bad_line_count += 1;
        return;
    };

    for record in records {
        if record.is_object() {
            push_record(record.clone(), HighlightLogLineKind::VideoListData, result);
        }
    }
}

fn account_name_hint_from_object(
    object: &Map<String, Value>,
) -> Option<HighlightLogAccountNameHint> {
    let account_id = direct_string_by_keys(
        object,
        &[
            "g_open_id",
            "gOpenId",
            "open_id",
            "openId",
            "openid",
            "OpenID",
        ],
    )?;
    if !looks_like_source_account_id(&account_id) {
        return None;
    }

    let role_name = direct_string_by_keys(
        object,
        &[
            "role_name",
            "roleName",
            "user_name",
            "userName",
            "gameName",
            "GameName",
            "nick",
        ],
    )?;
    let nick_id = direct_string_by_keys(
        object,
        &[
            "nick_id",
            "nickId",
            "user_nick_id",
            "userNickId",
            "tagLine",
            "TagLine",
            "tag",
        ],
    )?;
    let account_name = display_names::player_name_for_display(&format!("{role_name}#{nick_id}"));

    Some(HighlightLogAccountNameHint {
        account_id,
        account_name,
    })
}

fn parse_post_snapshot_payload(payload: &str, result: &mut HighlightLogParseResult) {
    let value = match serde_json::from_str::<Value>(payload) {
        Ok(value) => expand_embedded_json_strings(&value),
        Err(_) => {
            result.bad_line_count += 1;
            return;
        }
    };
    let mut records = Vec::new();
    collect_post_snapshot_records(&value, &mut records);

    if records.is_empty() {
        result.bad_line_count += 1;
        return;
    }

    for record in records {
        push_record(record, HighlightLogLineKind::PostSnapshot, result);
    }
}

fn collect_post_snapshot_records(value: &Value, records: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            if let Some(record) = compact_post_snapshot_record(value) {
                records.push(record);
                return;
            }

            for child in object.values() {
                collect_post_snapshot_records(child, records);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_post_snapshot_records(child, records);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn compact_post_snapshot_record(value: &Value) -> Option<Value> {
    let career = object_child_by_keys(value, &["career"])?;
    if !career.is_object() {
        return None;
    }
    find_string_by_keys(
        career,
        &[
            "matchId",
            "match_id",
            "matchesId",
            "matches_id",
            "gameId",
            "game_id",
        ],
    )?;

    let mut object = Map::new();
    object.insert("career".to_string(), career.clone());
    if let Some(kill_infos) = object_child_by_keys(
        value,
        &["killInfos", "kill_infos", "killEvents", "kill_events"],
    ) {
        object.insert("killInfos".to_string(), kill_infos.clone());
    }
    if let Some(record_src) =
        object_child_by_keys(value, &["recordSrc", "record_src", "videoSrc", "video_src"])
    {
        object.insert("recordSrc".to_string(), record_src.clone());
    }

    Some(Value::Object(object))
}

fn parse_event_parser_record(game: &Value, events: &[Value]) -> HighlightLogRecord {
    let game_context = expand_embedded_json_strings(game);
    let event_values = Value::Array(events.to_vec());
    let context = Value::Array(vec![game_context.clone(), event_values.clone()]);
    let map_id = find_map_id(&context);
    let game_mode = find_game_mode(&context);
    let kills = find_i64_by_keys(&context, &["kills", "killCount", "kill_count", "击杀"]);
    let deaths = find_i64_by_keys(&context, &["deaths", "deathCount", "death_count", "死亡"]);
    let assists = find_i64_by_keys(
        &context,
        &["assists", "assistCount", "assist_count", "助攻"],
    );
    let rounds_won = find_i64_by_keys(
        &context,
        &["roundsWon", "rounds_won", "winRound", "roundWin"],
    );
    let rounds_lost = find_i64_by_keys(
        &context,
        &["roundsLost", "rounds_lost", "lostRound", "roundLoss"],
    );

    HighlightLogRecord {
        line_kind: HighlightLogLineKind::EventParser,
        match_id: find_string_by_keys(
            &context,
            &[
                "matchId",
                "match_id",
                "gameId",
                "game_id",
                "matchesId",
                "matches_id",
            ],
        ),
        battle_id: find_string_by_keys(&context, &["battleId", "battle_id"]),
        record_src: find_video_list_path(game).map(normalize_path_string),
        player_name: find_player_name(&game_context)
            .or_else(|| find_current_player_name_from_events(events)),
        map_name: find_map_name(&context).or_else(|| {
            map_id
                .as_deref()
                .and_then(display_names::map_name_for_display)
        }),
        map_id,
        game_mode,
        agent_name: find_agent_name(&context),
        kda: find_string_by_keys(&context, &["kda", "KDA"])
            .or_else(|| build_kda(kills, deaths, assists)),
        scoreline: find_scoreline(&context, rounds_won, rounds_lost),
        has_won: find_bool_by_keys(
            &context,
            &["hasWon", "has_won", "isWin", "win", "wonMatch", "won_match"],
        )
        .or_else(|| parse_win_text(&context)),
        combat_score: find_i64_by_keys(
            &context,
            &[
                "combatScore",
                "combat_score",
                "MatchCombatScore",
                "score",
                "战斗分",
            ],
        ),
        kill_events: find_event_parser_kill_events(events),
        has_gzip_event: true,
        raw_json: serde_json::to_string(game).unwrap_or_default(),
    }
}

fn find_event_parser_round_scores(events: &[Value]) -> Vec<HighlightLogRoundScore> {
    let event_objects = events
        .iter()
        .filter_map(event_params_object)
        .collect::<Vec<_>>();

    let mut round_scores = Vec::new();
    let mut active_match_id = None;
    let mut active_events = Vec::new();

    for object in event_objects {
        if event_name_is(object, "GameStart") {
            active_match_id = event_game_id(object);
            active_events.clear();
            if active_match_id.is_some() {
                active_events.push(object);
            }
            continue;
        }

        if event_name_is(object, "GameSettle") {
            let Some(match_id) = active_match_id.take() else {
                active_events.clear();
                continue;
            };
            let settle_match_id = event_game_id(object);
            if settle_match_id.as_deref() == Some(match_id.as_str()) {
                active_events.push(object);
                if let Some(scores) = find_segment_round_scores(&active_events, match_id.as_str()) {
                    round_scores.extend(scores);
                }
            }
            active_events.clear();
            continue;
        }

        let Some(match_id) = active_match_id.as_deref() else {
            continue;
        };
        if event_game_id(object).is_some_and(|event_match_id| event_match_id != match_id) {
            active_match_id = None;
            active_events.clear();
            continue;
        }
        active_events.push(object);
    }

    round_scores
}

fn find_segment_round_scores(
    events: &[&Map<String, Value>],
    match_id: &str,
) -> Option<Vec<HighlightLogRoundScore>> {
    let settle = events.last().copied()?;
    if !event_name_is(settle, "GameSettle") || event_game_id(settle).as_deref() != Some(match_id) {
        return None;
    }
    let account_id = event_account_id(settle)?;
    let total_score = direct_i64_by_keys(settle, &["TotalScore", "totalScore", "total_score"])?;
    if total_score < 0 {
        return None;
    }

    let mut contextual_player_names = Vec::new();
    for object in events {
        if !event_name_is(object, "GameStart")
            && !event_name_is(object, "GameSettle")
            && !event_name_is(object, "RoundStart")
            && !event_name_is(object, "RoundEnd")
        {
            continue;
        }
        let event_account_id = event_account_id(object);
        if (event_name_is(object, "GameStart")
            || event_name_is(object, "GameSettle")
            || event_name_is(object, "RoundStart")
            || event_name_is(object, "RoundEnd"))
            && event_account_id.is_none()
        {
            return None;
        }
        if event_account_id
            .as_deref()
            .is_some_and(|event_account_id| event_account_id != account_id)
        {
            return None;
        }
        if event_account_id.as_deref() == Some(account_id.as_str()) {
            if let Some(player_name) =
                direct_string_by_keys(object, &["PlayerName", "playerName", "player_name"])
                    .as_deref()
                    .and_then(normalize_player_identity)
            {
                contextual_player_names.push(player_name);
            }
        }
    }

    let mut final_scores_by_player = BTreeMap::new();
    for object in events {
        if !event_name_is(object, "CombatScore") {
            continue;
        }
        if event_game_id(object).as_deref() != Some(match_id) {
            return None;
        }
        let player_name = combat_score_player_name(object)
            .as_deref()
            .and_then(normalize_player_identity)?;
        let match_score = direct_i64_by_keys(
            object,
            &["MatchCombatScore", "matchCombatScore", "match_combat_score"],
        )?;
        if match_score < 0 {
            return None;
        }
        final_scores_by_player.insert(player_name, match_score);
    }

    let mut matching_players = final_scores_by_player
        .iter()
        .filter_map(|(player_name, score)| (*score == total_score).then_some(player_name));
    let current_player = matching_players.next().cloned()?;
    if matching_players.next().is_some() {
        return None;
    }
    if contextual_player_names.first().is_some_and(|first| {
        contextual_player_names
            .iter()
            .skip(1)
            .any(|player_name| !player_identities_match(first, player_name))
    }) {
        return None;
    }
    if contextual_player_names
        .iter()
        .any(|player_name| !player_identities_match(player_name, &current_player))
    {
        return None;
    }

    let mut latest_match_score = 0i64;
    let mut previous_round_end_score = 0i64;
    let mut previous_round_id = None;
    let mut round_scores = Vec::new();
    for object in events {
        if event_name_is(object, "CombatScore") {
            let event_player = combat_score_player_name(object)
                .as_deref()
                .and_then(normalize_player_identity);
            if event_player.as_deref() != Some(current_player.as_str()) {
                continue;
            }
            let Some(match_score) = direct_i64_by_keys(
                object,
                &["MatchCombatScore", "matchCombatScore", "match_combat_score"],
            ) else {
                continue;
            };
            if match_score < latest_match_score {
                return None;
            }
            latest_match_score = match_score;
            continue;
        }

        if !event_name_is(object, "RoundEnd") {
            continue;
        }
        let round_id = direct_i64_by_keys(object, &["RoundID", "roundId", "round_id", "round"])?;
        let expected_round_id = previous_round_id.map_or(0, |round_id| round_id + 1);
        if round_id != expected_round_id {
            return None;
        }
        let round_score = latest_match_score.checked_sub(previous_round_end_score)?;
        round_scores.push(HighlightLogRoundScore {
            account_id: account_id.clone(),
            match_id: match_id.to_string(),
            round_id,
            score: round_score,
        });
        previous_round_id = Some(round_id);
        previous_round_end_score = latest_match_score;
    }

    if round_scores.is_empty() {
        return None;
    }
    Some(round_scores)
}

fn event_game_id(object: &Map<String, Value>) -> Option<String> {
    direct_string_by_keys(object, &["GameID", "gameId", "game_id"])
        .map(|game_id| game_id.trim().to_ascii_lowercase())
        .filter(|game_id| !game_id.is_empty())
}

fn event_account_id(object: &Map<String, Value>) -> Option<String> {
    direct_string_by_keys(
        object,
        &[
            "OpenID",
            "openID",
            "openId",
            "openid",
            "open_id",
            "g_open_id",
        ],
    )
    .map(|account_id| account_id.trim().to_ascii_lowercase())
    .filter(|account_id| !account_id.is_empty())
}

fn event_params_object(event: &Value) -> Option<&Map<String, Value>> {
    let object = event.as_object()?;
    object
        .iter()
        .find_map(|(key, child)| {
            key_matches(key, &["Params", "params"])
                .then(|| child.as_object())
                .flatten()
        })
        .or(Some(object))
}

fn event_name_is(object: &Map<String, Value>, expected: &str) -> bool {
    direct_string_by_keys(
        object,
        &["EventName", "eventName", "event_type", "eventType"],
    )
    .is_some_and(|event_name| event_name.eq_ignore_ascii_case(expected))
}

fn combat_score_player_name(object: &Map<String, Value>) -> Option<String> {
    if let Some((_, value)) = object
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("__PlayerName__"))
    {
        return string_from_value(value);
    }

    object.iter().find_map(|(key, value)| {
        ["PlayerName", "playerName", "player_name"]
            .iter()
            .any(|candidate| key.eq_ignore_ascii_case(candidate))
            .then(|| string_from_value(value))
            .flatten()
    })
}

fn normalize_player_identity(player_name: &str) -> Option<String> {
    let player_name = player_name.trim().to_lowercase();
    (!player_name.is_empty()).then_some(player_name)
}

fn player_identities_match(expected: &str, candidate: &str) -> bool {
    if expected == candidate {
        return true;
    }

    let expected_base = expected
        .split_once('#')
        .map_or(expected, |(name, _)| name)
        .trim();
    let candidate_base = candidate
        .split_once('#')
        .map_or(candidate, |(name, _)| name)
        .trim();
    (!expected.contains('#') || !candidate.contains('#'))
        && !expected_base.is_empty()
        && expected_base == candidate_base
}

fn direct_i64_by_keys(object: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    object.iter().find_map(|(key, child)| {
        key_matches(key, keys)
            .then(|| i64_from_value(child))
            .flatten()
    })
}

fn parse_record(value: &Value, line_kind: HighlightLogLineKind) -> HighlightLogRecord {
    let kills = find_i64_by_keys(value, &["kills", "killCount", "kill_count", "击杀"]);
    let deaths = find_i64_by_keys(value, &["deaths", "deathCount", "death_count", "死亡"]);
    let assists = find_i64_by_keys(value, &["assists", "assistCount", "assist_count", "助攻"]);
    let rounds_won = find_i64_by_keys(value, &["roundsWon", "rounds_won", "winRound", "roundWin"]);
    let rounds_lost = find_i64_by_keys(
        value,
        &["roundsLost", "rounds_lost", "lostRound", "roundLoss"],
    );

    HighlightLogRecord {
        line_kind,
        match_id: find_string_by_keys(
            value,
            &[
                "matchId",
                "match_id",
                "matchesId",
                "matches_id",
                "gameId",
                "game_id",
            ],
        ),
        battle_id: find_string_by_keys(value, &["battleId", "battle_id"]),
        record_src: find_string_by_keys(
            value,
            &[
                "recordSrc",
                "record_src",
                "recordSource",
                "record_source",
                "videoSrc",
                "video_src",
                "videoSource",
                "video_source",
            ],
        )
        .map(normalize_path_string),
        player_name: find_player_name(value),
        map_id: find_map_id(value),
        map_name: find_map_name(value),
        game_mode: find_game_mode(value),
        agent_name: find_agent_name(value),
        kda: find_string_by_keys(value, &["kda", "KDA"])
            .or_else(|| build_kda(kills, deaths, assists)),
        scoreline: find_scoreline(value, rounds_won, rounds_lost),
        has_won: find_bool_by_keys(
            value,
            &["hasWon", "has_won", "isWin", "win", "wonMatch", "won_match"],
        )
        .or_else(|| parse_win_text(value)),
        combat_score: find_i64_by_keys(
            value,
            &["combatScore", "combat_score", "acs", "score", "战斗分"],
        ),
        kill_events: find_kill_events(value),
        has_gzip_event: has_gzip_event(value),
        raw_json: serde_json::to_string(value).unwrap_or_default(),
    }
}

fn find_player_name(value: &Value) -> Option<String> {
    find_riot_id(value)
        .or_else(|| {
            find_direct_player_name(value).or_else(|| find_direct_nested_player_name(value))
        })
        .and_then(|value| normalize_player_name(&value))
}

fn find_riot_id(value: &Value) -> Option<String> {
    let Value::Object(object) = value else {
        return None;
    };

    riot_id_from_object(object)
}

fn riot_id_from_object(object: &Map<String, Value>) -> Option<String> {
    let game_name = direct_string_by_keys(
        object,
        &[
            "gameName",
            "GameName",
            "riotGameName",
            "riot_game_name",
            "playerGameName",
            "userName",
            "user_name",
        ],
    )?;
    let tag_line = direct_string_by_keys(
        object,
        &[
            "tagLine",
            "TagLine",
            "tagline",
            "riotTagLine",
            "riot_tag_line",
            "gameTag",
            "userNickId",
            "user_nick_id",
            "userNickID",
            "nickId",
            "nick_id",
        ],
    )?;

    format_riot_id(&game_name, &tag_line)
}

fn find_current_player_name_from_events(events: &[Value]) -> Option<String> {
    events.iter().find_map(|event| {
        let Value::Object(object) = event else {
            return None;
        };
        let event_object = object
            .iter()
            .find_map(|(key, child)| {
                if key_matches(key, &["Params", "params"]) {
                    child.as_object()
                } else {
                    None
                }
            })
            .unwrap_or(object);
        let killer_is_me = direct_event_flag(event_object, &["KillerIsMe", "killerIsMe"]);
        let killed_is_me = direct_event_flag(event_object, &["KilledIsMe", "killedIsMe"]);
        let is_identity_event = direct_string_by_keys(
            event_object,
            &["EventName", "eventName", "event_type", "eventType"],
        )
        .is_some_and(|name| name.eq_ignore_ascii_case("RoundStart"));
        if !killer_is_me && !killed_is_me && !is_identity_event {
            return None;
        }

        if let Some(riot_id) = riot_id_from_object(event_object) {
            return normalize_player_name(&riot_id);
        }

        if is_identity_event {
            return direct_string_by_keys(
                event_object,
                &["playerName", "player_name", "riotId", "riot_id"],
            )
            .filter(|name| name.contains('#'))
            .and_then(|name| normalize_player_name(&name));
        }

        let participant_name = if killer_is_me {
            direct_string_by_keys(
                event_object,
                &["killerName", "killer_name", "killer", "KillerPlayerName"],
            )
        } else {
            direct_string_by_keys(
                event_object,
                &[
                    "killedName",
                    "killed_name",
                    "victimName",
                    "KilledPlayerName",
                ],
            )
        }?;

        participant_name
            .contains('#')
            .then_some(participant_name)
            .and_then(|name| normalize_player_name(&name))
    })
}

fn direct_event_flag(object: &Map<String, Value>, keys: &[&str]) -> bool {
    direct_string_by_keys(object, keys).is_some_and(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        normalized == "1" || normalized == "true"
    })
}

fn find_direct_player_name(value: &Value) -> Option<String> {
    let Value::Object(object) = value else {
        return None;
    };

    direct_string_by_keys(
        object,
        &[
            "riotId",
            "riot_id",
            "gameNameWithTag",
            "playerName",
            "player_name",
            "userName",
            "user_name",
            "nickName",
            "玩家昵称",
            "玩家名",
        ],
    )
}

fn find_direct_nested_player_name(value: &Value) -> Option<String> {
    let player = object_child_by_keys(value, &["player"])?;
    let Value::Object(object) = player else {
        return None;
    };

    direct_string_by_keys(object, &["name", "nickName", "playerName"])
}

fn format_riot_id(game_name: &str, tag_line: &str) -> Option<String> {
    let game_name = game_name.trim();
    let tag_line = tag_line.trim().trim_start_matches('#');

    if game_name.is_empty() {
        return None;
    }

    if game_name.contains('#') || tag_line.is_empty() {
        return normalize_player_name(game_name);
    }

    normalize_player_name(&format!("{game_name}#{tag_line}"))
}

fn find_map_id(value: &Value) -> Option<String> {
    find_string_by_keys(value, &["mapId", "map_id", "地图ID", "地图Id"])
        .or_else(|| find_nested_string(value, &["map"], &["id", "mapId"]))
}

fn find_map_name(value: &Value) -> Option<String> {
    find_string_by_keys(value, &["mapName", "map_name", "地图", "地图名"])
        .and_then(|value| display_names::map_name_for_display(&value))
        .or_else(|| find_nested_string(value, &["map"], &["name", "mapName"]))
        .and_then(|value| display_names::map_name_for_display(&value))
        .or_else(|| {
            find_map_id(value)
                .as_deref()
                .and_then(display_names::map_name_for_display)
        })
}

fn find_game_mode(value: &Value) -> Option<String> {
    find_mapped_string_by_keys(
        value,
        &[
            "gameMode",
            "game_mode",
            "mode",
            "FullModeId",
            "full_mode_id",
            "modeId",
            "mode_id",
            "modelId",
            "model_id",
            "模式",
        ],
        normalize_game_mode,
    )
    .or_else(|| find_mapped_string_by_keys(value, &["modeName", "mode_name"], normalize_game_mode))
}

fn find_agent_name(value: &Value) -> Option<String> {
    find_mapped_string_by_keys(value, &["agentName", "agent_name"], normalize_agent_name)
        .or_else(|| {
            find_nested_string(
                value,
                &["agent", "hero", "character"],
                &["name", "agentName"],
            )
            .and_then(|value| normalize_agent_name(&value))
        })
        .or_else(|| {
            find_mapped_string_by_keys(
                value,
                &["heroName", "heroId", "hero_id", "characterName", "英雄"],
                normalize_agent_name,
            )
        })
        .or_else(|| {
            find_mapped_string_by_keys(
                value,
                &[
                    "heroImage",
                    "hero_image",
                    "agentUrl",
                    "agent_url",
                    "agentAvatarUrl",
                    "agent_avatar_url",
                ],
                display_names::agent_name_from_avatar_url,
            )
        })
        .or_else(|| find_mapped_string_by_keys(value, &["roleName"], normalize_agent_name))
}

fn find_kill_events(value: &Value) -> Vec<HighlightLogKillEvent> {
    if let Some(events) = find_array_by_keys(
        value,
        &[
            "killEvents",
            "kill_events",
            "killInfos",
            "kill_infos",
            "击杀事件",
        ],
    ) {
        return events.iter().filter_map(parse_kill_event).collect();
    }

    if let Some(events) = find_array_by_keys(value, &["击杀事件时间", "killEventTimes"]) {
        return events.iter().filter_map(parse_kill_event_time).collect();
    }

    Vec::new()
}

fn parse_kill_event(value: &Value) -> Option<HighlightLogKillEvent> {
    match value {
        Value::Object(_) => Some(HighlightLogKillEvent {
            event_time: find_string_by_keys(
                value,
                &["eventTime", "EventTime", "event_time", "time", "时间"],
            ),
            round_id: find_i64_by_keys(value, &["roundId", "RoundID", "round_id", "round", "回合"]),
            weapon_name: find_string_by_keys(
                value,
                &[
                    "weaponName",
                    "WeaponName",
                    "weapon_name",
                    "weapon",
                    "WeaponSkinName",
                    "WeaponID",
                    "武器",
                ],
            )
            .and_then(|value| display_name_from_internal_path(&value).or(Some(value))),
            killer_name: find_string_by_keys(
                value,
                &["killerName", "KillerPlayerName", "killer_name", "killer"],
            ),
            killed_name: find_string_by_keys(
                value,
                &[
                    "killedName",
                    "KilledPlayerName",
                    "killed_name",
                    "victimName",
                ],
            ),
            raw_json: serde_json::to_string(value).ok(),
        }),
        Value::String(_) => parse_kill_event_time(value),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
    }
}

fn parse_kill_event_time(value: &Value) -> Option<HighlightLogKillEvent> {
    string_from_value(value).map(|event_time| HighlightLogKillEvent {
        event_time: Some(event_time),
        round_id: None,
        weapon_name: None,
        killer_name: None,
        killed_name: None,
        raw_json: None,
    })
}

fn find_event_parser_kill_events(events: &[Value]) -> Vec<HighlightLogKillEvent> {
    events
        .iter()
        .filter(|event| is_event_parser_kill_event(event))
        .map(|event| HighlightLogKillEvent {
            event_time: find_string_by_keys(event, &["eventTime", "EventTime", "time", "时间"]),
            round_id: find_i64_by_keys(event, &["roundId", "round_id", "round", "Round", "回合"]),
            weapon_name: find_string_by_keys(
                event,
                &[
                    "weaponName",
                    "WeaponName",
                    "weapon",
                    "WeaponSkinName",
                    "WeaponID",
                    "武器",
                ],
            )
            .and_then(|value| display_name_from_internal_path(&value).or(Some(value))),
            killer_name: find_string_by_keys(
                event,
                &["killerName", "killer_name", "killer", "KillerPlayerName"],
            ),
            killed_name: find_string_by_keys(
                event,
                &[
                    "killedName",
                    "killed_name",
                    "victimName",
                    "KilledPlayerName",
                ],
            ),
            raw_json: serde_json::to_string(event).ok(),
        })
        .collect()
}

fn is_event_parser_kill_event(event: &Value) -> bool {
    let Some(event_name) = find_string_by_keys(
        event,
        &["EventName", "eventName", "event_type", "eventType"],
    ) else {
        return false;
    };
    let normalized = event_name.trim().to_lowercase();

    normalized == "killstreak"
        || normalized == "kill"
        || (normalized == "shot" && is_real_shot_kill_event(event))
}

fn is_real_shot_kill_event(event: &Value) -> bool {
    let Some(round_id) =
        find_i64_by_keys(event, &["roundId", "RoundID", "round_id", "round", "回合"])
    else {
        return false;
    };
    if round_id <= 0 {
        return false;
    }

    let has_kill_participants = find_string_by_keys(
        event,
        &["killerName", "killer_name", "killer", "KillerPlayerName"],
    )
    .is_some()
        && find_string_by_keys(
            event,
            &[
                "killedName",
                "killed_name",
                "victimName",
                "KilledPlayerName",
            ],
        )
        .is_some();
    let involves_current_player = find_i64_by_keys(event, &["KillerIsMe"])
        .is_some_and(|value| value == 1)
        || find_i64_by_keys(event, &["KilledIsMe"]).is_some_and(|value| value == 1);

    has_kill_participants && involves_current_player
}

fn find_video_list_path(value: &Value) -> Option<String> {
    let video_list = find_array_by_keys(value, &["videoList", "video_list"])?;
    video_list.iter().find_map(|video| {
        find_string_by_keys(
            video,
            &[
                "path",
                "recordSrc",
                "record_src",
                "videoSrc",
                "video_src",
                "videoSource",
                "video_source",
            ],
        )
    })
}

fn expand_embedded_json_strings(value: &Value) -> Value {
    expand_json_strings_inner(value, &mut |_| {})
}

fn expand_and_collect_hints(value: &Value, hints: &mut Vec<HighlightLogAccountNameHint>) -> Value {
    expand_json_strings_inner(value, &mut |obj| {
        if let Some(hint) = account_name_hint_from_object(obj) {
            hints.push(hint);
        }
    })
}

fn expand_json_strings_inner(
    value: &Value,
    on_object: &mut dyn FnMut(&Map<String, Value>),
) -> Value {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if (trimmed.starts_with('{') || trimmed.starts_with('[')) && trimmed.len() < 1_000_000 {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    return expand_json_strings_inner(&parsed, on_object);
                }
            }
            value.clone()
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|v| expand_json_strings_inner(v, on_object))
                .collect(),
        ),
        Value::Object(object) => {
            on_object(object);
            Value::Object(
                object
                    .iter()
                    .map(|(k, v)| (k.clone(), expand_json_strings_inner(v, on_object)))
                    .collect(),
            )
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

fn decode_gzip_event_payload(encoded: &str) -> Option<Vec<Value>> {
    let mut base64_text = encoded.trim().replace("\\\\", "\\");
    let missing_padding = base64_text.len() % 4;
    if missing_padding != 0 {
        base64_text.push_str(&"=".repeat(4 - missing_padding));
    }

    let compressed = general_purpose::STANDARD.decode(&base64_text).ok()?;
    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut decoded = String::with_capacity(compressed.len() * 4);
    decoder.read_to_string(&mut decoded).ok()?;

    let Value::Array(values) = serde_json::from_str::<Value>(&decoded).ok()? else {
        return None;
    };

    let events = values
        .into_iter()
        .filter_map(|value| match value {
            Value::String(text) => serde_json::from_str::<Value>(&text).ok(),
            Value::Object(_) => Some(value),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
        })
        .collect::<Vec<_>>();

    Some(events)
}

fn payload_after_marker<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let marker_index = line.find(marker)?;
    let payload = line[marker_index + marker.len()..].trim();
    if payload.is_empty() {
        None
    } else {
        Some(payload)
    }
}

fn find_nested_string(value: &Value, object_keys: &[&str], value_keys: &[&str]) -> Option<String> {
    find_object_by_keys(value, object_keys)
        .and_then(|object| find_string_by_keys(object, value_keys))
}

fn find_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key_matches(key, keys) {
                    if let Some(value) = string_from_value(child) {
                        return Some(value);
                    }
                }
            }

            for child in object.values() {
                if let Some(value) = find_string_by_keys(child, keys) {
                    return Some(value);
                }
            }

            None
        }
        Value::Array(values) => values
            .iter()
            .find_map(|child| find_string_by_keys(child, keys)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn direct_string_by_keys(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    object.iter().find_map(|(key, child)| {
        if key_matches(key, keys) {
            string_from_value(child)
        } else {
            None
        }
    })
}

fn direct_array_by_keys<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    let Value::Object(object) = value else {
        return None;
    };

    object.iter().find_map(|(key, child)| {
        if key_matches(key, keys) {
            if let Value::Array(values) = child {
                Some(values)
            } else {
                None
            }
        } else {
            None
        }
    })
}

fn looks_like_source_account_id(value: &str) -> bool {
    let trimmed = value.trim();
    (8..=32).contains(&trimmed.len()) && trimmed.bytes().all(|byte| byte.is_ascii_digit())
}

fn find_mapped_string_by_keys<F>(value: &Value, keys: &[&str], map: F) -> Option<String>
where
    F: Fn(&str) -> Option<String> + Copy,
{
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key_matches(key, keys) {
                    if let Some(value) = string_from_value(child).and_then(|value| map(&value)) {
                        return Some(value);
                    }
                }
            }

            for child in object.values() {
                if let Some(value) = find_mapped_string_by_keys(child, keys, map) {
                    return Some(value);
                }
            }

            None
        }
        Value::Array(values) => values
            .iter()
            .find_map(|child| find_mapped_string_by_keys(child, keys, map)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn find_i64_by_keys(value: &Value, keys: &[&str]) -> Option<i64> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key_matches(key, keys) {
                    if let Some(value) = i64_from_value(child) {
                        return Some(value);
                    }
                }
            }

            for child in object.values() {
                if let Some(value) = find_i64_by_keys(child, keys) {
                    return Some(value);
                }
            }

            None
        }
        Value::Array(values) => values
            .iter()
            .find_map(|child| find_i64_by_keys(child, keys)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn find_bool_by_keys(value: &Value, keys: &[&str]) -> Option<bool> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key_matches(key, keys) {
                    if let Some(value) = bool_from_value(child) {
                        return Some(value);
                    }
                }
            }

            for child in object.values() {
                if let Some(value) = find_bool_by_keys(child, keys) {
                    return Some(value);
                }
            }

            None
        }
        Value::Array(values) => values
            .iter()
            .find_map(|child| find_bool_by_keys(child, keys)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn find_array_by_keys<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key_matches(key, keys) {
                    if let Value::Array(values) = child {
                        return Some(values);
                    }
                }
            }

            for child in object.values() {
                if let Some(values) = find_array_by_keys(child, keys) {
                    return Some(values);
                }
            }

            None
        }
        Value::Array(values) => values
            .iter()
            .find_map(|child| find_array_by_keys(child, keys)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn find_object_by_keys<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key_matches(key, keys) && child.is_object() {
                    return Some(child);
                }
            }

            for child in object.values() {
                if let Some(value) = find_object_by_keys(child, keys) {
                    return Some(value);
                }
            }

            None
        }
        Value::Array(values) => values
            .iter()
            .find_map(|child| find_object_by_keys(child, keys)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn object_child_by_keys<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let Value::Object(object) = value else {
        return None;
    };

    object
        .iter()
        .find_map(|(key, child)| key_matches(key, keys).then_some(child))
}

fn parse_win_text(value: &Value) -> Option<bool> {
    let text = find_string_by_keys(
        value,
        &[
            "hasWon",
            "has_won",
            "result",
            "matchResult",
            "match_result",
            "win",
            "胜负",
        ],
    )?;
    let normalized = text.to_lowercase();

    if normalized.contains('胜')
        || normalized.contains("成功")
        || normalized.contains("win")
        || normalized.contains("victory")
    {
        Some(true)
    } else if normalized.contains('负')
        || normalized.contains("失败")
        || normalized.contains("loss")
        || normalized.contains("defeat")
        || normalized.contains("lose")
    {
        Some(false)
    } else {
        None
    }
}

fn has_gzip_event(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, child)| {
            (key_matches(key, &["event"])
                && string_from_value(child).is_some_and(|value| is_gzip_payload(&value)))
                || has_gzip_event(child)
        }),
        Value::Array(values) => values.iter().any(has_gzip_event),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn string_from_value(value: &Value) -> Option<String> {
    let value = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => return None,
    };

    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn i64_from_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

fn bool_from_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "win" | "won" | "victory" | "胜" | "胜利" => Some(true),
            "false" | "0" | "no" | "loss" | "lost" | "defeat" | "负" | "失败" => Some(false),
            _ => None,
        },
        Value::Number(value) => value.as_i64().and_then(|value| match value {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn normalize_player_name(value: &str) -> Option<String> {
    display_names::player_name_for_display(value)
}

fn normalize_agent_name(value: &str) -> Option<String> {
    let candidate = agent_name_from_role_name(value)
        .or_else(|| display_name_from_internal_path(value))
        .unwrap_or_else(|| value.trim().to_string());
    display_names::agent_name_for_display(&candidate)
}

fn normalize_game_mode(value: &str) -> Option<String> {
    display_names::game_mode_for_display(value)
}

fn build_kda(kills: Option<i64>, deaths: Option<i64>, assists: Option<i64>) -> Option<String> {
    Some(format!("{}/{}/{}", kills?, deaths?, assists?))
}

fn find_scoreline(
    value: &Value,
    rounds_won: Option<i64>,
    rounds_lost: Option<i64>,
) -> Option<String> {
    find_string_by_keys(
        value,
        &[
            "scoreline",
            "scoreLine",
            "roundsScore",
            "rounds_score",
            "roundsResult",
            "rounds_result",
            "比分",
        ],
    )
    .and_then(|value| normalize_scoreline(&value))
    .or_else(|| build_scoreline(rounds_won, rounds_lost))
}

fn normalize_scoreline(value: &str) -> Option<String> {
    for separator in ['/', '-', ':'] {
        let parts = value.split(separator).collect::<Vec<_>>();
        if parts.len() != 2 {
            continue;
        }

        let left = parts[0].trim().parse::<i64>().ok()?;
        let right = parts[1].trim().parse::<i64>().ok()?;
        return Some(format!("{left}/{right}"));
    }

    None
}

fn build_scoreline(rounds_won: Option<i64>, rounds_lost: Option<i64>) -> Option<String> {
    Some(format!("{}/{}", rounds_won?, rounds_lost?))
}

fn display_name_from_internal_path(value: &str) -> Option<String> {
    display_names::internal_asset_name(value)
}

fn agent_name_from_role_name(value: &str) -> Option<String> {
    let role_name = value
        .split("PersistentLevel.")
        .nth(1)
        .unwrap_or(value)
        .rsplit('/')
        .next()
        .unwrap_or(value)
        .split('.')
        .next_back()
        .unwrap_or(value)
        .split('_')
        .next()
        .unwrap_or(value)
        .trim();

    if role_name.is_empty() || role_name == value {
        None
    } else {
        Some(role_name.to_string())
    }
}

fn key_matches(key: &str, candidates: &[&str]) -> bool {
    let normalized_key = normalize_key(key);
    candidates
        .iter()
        .any(|candidate| normalized_key == normalize_key(candidate))
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '_' | '-' | ' ' | '\t' | '\r' | '\n'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_path_string(value: String) -> String {
    value.replace('\\', "/")
}

fn is_gzip_payload(value: &str) -> bool {
    value.trim_start().starts_with("H4sI")
}

fn path_to_string(path: &Path) -> String {
    PathBuf::from(path).display().to_string().replace('\\', "/")
}
