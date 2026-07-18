use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;

const BATTLE_LIST_PREFIX: &[u8] = b"acloshighlight_battle_list_";
const ACCOUNT_ROLES_KEY: &[u8] = b"ACLOS_USER_ROLES_INFO";
const MAX_JSON_START_DISTANCE: usize = 512;
const MAX_JSON_BYTES: usize = 256 * 1024;
const MAX_ACCOUNT_ROLE_SCAN_BYTES: usize = 8 * 1024;
const MAX_NOISY_BATTLE_SCAN_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelDbBattleListResult {
    pub battles: Vec<LevelDbBattleRecord>,
    pub account_roles: Vec<LevelDbAccountRoleRecord>,
    pub bad_record_count: usize,
    pub warning_count: usize,
    pub copied_file_count: usize,
    pub used_snapshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelDbBattleRecord {
    pub account_id: String,
    pub battle_id: Option<String>,
    pub match_id: Option<String>,
    pub player_name: Option<String>,
    pub kda: Option<String>,
    pub match_date: Option<String>,
    pub agent_avatar_url: Option<String>,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelDbAccountRoleRecord {
    pub account_id: String,
    pub player_name: Option<String>,
    pub nick_name: Option<String>,
    pub tag_line: Option<String>,
    pub raw_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonEncoding {
    Utf16Le,
    Utf8,
}

pub fn read_leveldb_battle_lists(
    leveldb_dir: impl AsRef<Path>,
) -> Result<LevelDbBattleListResult, String> {
    let leveldb_dir = leveldb_dir.as_ref();
    let mut result = LevelDbBattleListResult::default();

    if !leveldb_dir.is_dir() {
        return Ok(result);
    }

    let snapshot_dir = if leveldb_dir.join("LOCK").exists() {
        create_leveldb_snapshot(leveldb_dir, &mut result)
    } else {
        None
    };
    let read_dir = snapshot_dir.as_deref().unwrap_or(leveldb_dir);
    parse_leveldb_dir(read_dir, &mut result)?;

    if let Some(snapshot_dir) = snapshot_dir {
        let _ = fs::remove_dir_all(snapshot_dir);
    }

    Ok(result)
}

fn create_leveldb_snapshot(
    leveldb_dir: &Path,
    result: &mut LevelDbBattleListResult,
) -> Option<PathBuf> {
    let snapshot_dir = std::env::temp_dir().join(format!(
        "vhm-leveldb-snapshot-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));

    if fs::create_dir_all(&snapshot_dir).is_err() {
        result.warning_count += 1;
        return None;
    }

    result.used_snapshot = true;

    let entries = match fs::read_dir(leveldb_dir) {
        Ok(entries) => entries,
        Err(_) => {
            result.warning_count += 1;
            return Some(snapshot_dir);
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                result.warning_count += 1;
                continue;
            }
        };
        let source_path = entry.path();
        if !source_path.is_file() || !is_leveldb_candidate_file(&source_path) {
            continue;
        }

        let destination_path = snapshot_dir.join(entry.file_name());
        match fs::copy(&source_path, destination_path) {
            Ok(_) => result.copied_file_count += 1,
            Err(_) => result.warning_count += 1,
        }
    }

    Some(snapshot_dir)
}

fn parse_leveldb_dir(
    leveldb_dir: &Path,
    result: &mut LevelDbBattleListResult,
) -> Result<(), String> {
    let mut seen_battles = HashSet::new();
    let mut seen_account_roles = HashSet::new();
    let mut paths = fs::read_dir(leveldb_dir)
        .map_err(|error| {
            format!(
                "Failed to read LevelDB directory {}: {error}",
                path_to_string(leveldb_dir)
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file() && is_leveldb_candidate_file(path))
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| path_to_string(path).to_lowercase());

    for path in paths {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                result.warning_count += 1;
                continue;
            }
        };
        parse_leveldb_bytes(&bytes, result, &mut seen_battles, &mut seen_account_roles);
    }

    Ok(())
}

fn parse_leveldb_bytes(
    bytes: &[u8],
    result: &mut LevelDbBattleListResult,
    seen_battles: &mut HashSet<String>,
    seen_account_roles: &mut HashSet<String>,
) {
    let mut position = 0;
    while let Some(prefix_offset) = find_subslice(&bytes[position..], BATTLE_LIST_PREFIX) {
        let prefix_start = position + prefix_offset;
        let account_start = prefix_start + BATTLE_LIST_PREFIX.len();
        let account_end = read_ascii_digits_end(bytes, account_start);
        if account_end == account_start {
            position = prefix_start + 1;
            continue;
        }

        let account_id = String::from_utf8_lossy(&bytes[account_start..account_end]).to_string();
        let strict_result =
            find_json_array_start(bytes, account_end).and_then(|(json_start, encoding)| {
                extract_json_array(bytes, json_start, encoding)
                    .map(|json| parse_battle_list_json(&account_id, &json, result, seen_battles))
            });
        let parsed_strict = matches!(strict_result, Some(Ok(true)));
        let parsed_noisy = if parsed_strict {
            false
        } else {
            parse_noisy_battle_list(
                bytes,
                prefix_start,
                account_end,
                &account_id,
                result,
                seen_battles,
            )
        };
        if !parsed_strict && !parsed_noisy && !matches!(strict_result, Some(Ok(_))) {
            result.bad_record_count += 1;
        }

        position = account_end;
    }

    parse_account_role_entries(bytes, result, seen_account_roles);
}

fn parse_battle_list_json(
    account_id: &str,
    json: &str,
    result: &mut LevelDbBattleListResult,
    seen: &mut HashSet<String>,
) -> Result<bool, ()> {
    let value = match serde_json::from_str::<Value>(json) {
        Ok(value) => value,
        Err(_) => return Err(()),
    };

    let Value::Array(records) = value else {
        return Err(());
    };

    let mut parsed_any = false;
    for record in records {
        let Some(battle) = parse_battle_record(account_id, &record) else {
            result.bad_record_count += 1;
            continue;
        };

        parsed_any |= push_battle_record(result, seen, battle);
    }

    Ok(parsed_any)
}

fn parse_battle_record(account_id: &str, value: &Value) -> Option<LevelDbBattleRecord> {
    if !value.is_object() {
        return None;
    }

    let battle_id = find_string_by_keys(value, &["battleId", "battle_id", "battleID"]);
    let match_id = find_string_by_keys(value, &["matchId", "match_id", "gameId", "game_id"]);
    if battle_id.is_none() && match_id.is_none() {
        return None;
    }

    let kills = find_i64_by_keys(value, &["kills", "killCount", "kill_count"]);
    let deaths = find_i64_by_keys(value, &["deaths", "deathCount", "death_count"]);
    let assists = find_i64_by_keys(value, &["assists", "assistCount", "assist_count"]);

    Some(LevelDbBattleRecord {
        account_id: account_id.to_string(),
        battle_id,
        match_id,
        player_name: find_player_name(value),
        kda: find_string_by_keys(value, &["kda", "KDA"])
            .or_else(|| build_kda(kills, deaths, assists)),
        match_date: find_string_by_keys(
            value,
            &[
                "date",
                "matchDate",
                "match_date",
                "battleDate",
                "startedAt",
                "startTime",
                "createTime",
            ],
        ),
        agent_avatar_url: find_string_by_keys(
            value,
            &[
                "heroAvatarUrl",
                "hero_avatar_url",
                "agentAvatarUrl",
                "agent_avatar_url",
                "characterAvatarUrl",
                "avatarUrl",
                "英雄头像",
            ],
        ),
        raw_json: serde_json::to_string(value).unwrap_or_default(),
    })
}

fn parse_noisy_battle_list(
    bytes: &[u8],
    prefix_start: usize,
    account_end: usize,
    account_id: &str,
    result: &mut LevelDbBattleListResult,
    seen: &mut HashSet<String>,
) -> bool {
    let max_end = bytes
        .len()
        .min(prefix_start.saturating_add(MAX_NOISY_BATTLE_SCAN_BYTES));
    let next_prefix = find_subslice(&bytes[account_end..], BATTLE_LIST_PREFIX)
        .map(|offset| account_end + offset)
        .unwrap_or(max_end);
    let scan_end = max_end.min(next_prefix);
    let raw_text = compact_ascii_text(&bytes[prefix_start..scan_end]);

    let mut candidates = find_uuid_mentions_after_keywords(
        &raw_text,
        &[
            "match_id",
            "matchid",
            "matches_id",
            "matchesid",
            "game_id",
            "gameid",
        ],
    );
    if candidates.is_empty() {
        if let Some(candidate) = find_uuid_in(&raw_text, 0, raw_text.len()) {
            candidates.push(candidate);
        }
    }

    let single_candidate = candidates.len() == 1;
    let mut parsed_any = false;
    let mut local_seen = HashSet::new();
    for (match_id, position) in candidates {
        if !local_seen.insert(match_id.clone()) {
            continue;
        }

        let context_start = position.saturating_sub(1024);
        let context_end = raw_text.len().min(position.saturating_add(2048));
        let context = &raw_text[context_start..context_end];
        let kda = find_kda_in(context).or_else(|| {
            if single_candidate {
                find_kda_in(&raw_text)
            } else {
                None
            }
        });
        let agent_avatar_url = find_avatar_url_in(context).or_else(|| {
            if single_candidate {
                find_avatar_url_in(&raw_text)
            } else {
                None
            }
        });
        let battle_id = find_uuid_mentions_after_keywords(context, &["battle_id", "battleid"])
            .into_iter()
            .map(|(uuid, _)| uuid)
            .find(|uuid| uuid != &match_id);

        if kda.is_none() && agent_avatar_url.is_none() && battle_id.is_none() {
            continue;
        }

        let battle = LevelDbBattleRecord {
            account_id: account_id.to_string(),
            battle_id,
            match_id: Some(match_id),
            player_name: None,
            kda,
            match_date: None,
            agent_avatar_url,
            raw_json: context.to_string(),
        };
        parsed_any |= push_battle_record(result, seen, battle);
    }

    parsed_any
}

fn push_battle_record(
    result: &mut LevelDbBattleListResult,
    seen: &mut HashSet<String>,
    battle: LevelDbBattleRecord,
) -> bool {
    let dedupe_key = format!(
        "{}|{}|{}",
        battle.account_id,
        battle.battle_id.as_deref().unwrap_or_default(),
        battle.match_id.as_deref().unwrap_or_default()
    );
    if seen.insert(dedupe_key) {
        result.battles.push(battle);
        true
    } else {
        false
    }
}

fn parse_account_role_entries(
    bytes: &[u8],
    result: &mut LevelDbBattleListResult,
    seen: &mut HashSet<String>,
) {
    let mut position = 0;
    while let Some(key_offset) = find_subslice(&bytes[position..], ACCOUNT_ROLES_KEY) {
        let key_start = position + key_offset;
        let value_start = key_start + ACCOUNT_ROLES_KEY.len();
        let parsed_strict = find_json_array_start(bytes, value_start)
            .and_then(|(json_start, encoding)| extract_json_array(bytes, json_start, encoding))
            .is_some_and(|json| parse_account_roles_json(&json, result, seen));

        if !parsed_strict {
            parse_noisy_account_roles(bytes, key_start, result, seen);
        }

        position = value_start;
    }
}

fn parse_account_roles_json(
    json: &str,
    result: &mut LevelDbBattleListResult,
    seen: &mut HashSet<String>,
) -> bool {
    let Ok(Value::Array(records)) = serde_json::from_str::<Value>(json) else {
        return false;
    };

    let mut parsed_any = false;
    for record in records {
        let Some(role) = parse_account_role_record(&record, json) else {
            continue;
        };

        parsed_any |= push_account_role(result, seen, role);
    }

    parsed_any
}

fn parse_account_role_record(value: &Value, raw_text: &str) -> Option<LevelDbAccountRoleRecord> {
    if !value.is_object() {
        return None;
    }

    let account_id = find_string_by_keys(
        value,
        &[
            "openid",
            "open_id",
            "gopenid",
            "gopen_id",
            "accountId",
            "account_id",
        ],
    )?;
    if !looks_like_account_id(&account_id) {
        return None;
    }

    let nick_name = find_string_by_keys(
        value,
        &[
            "nick",
            "nickname",
            "nickName",
            "nick_name",
            "gameName",
            "playerName",
            "player_name",
        ],
    );
    let tag_line = find_string_by_keys(
        value,
        &[
            "tag", "tagLine", "tagline", "tag_line", "gameTag", "game_tag",
        ],
    );
    let player_name = role_player_name(nick_name.as_deref(), tag_line.as_deref());

    Some(LevelDbAccountRoleRecord {
        account_id,
        player_name,
        nick_name,
        tag_line,
        raw_text: raw_text.to_string(),
    })
}

fn parse_noisy_account_roles(
    bytes: &[u8],
    key_start: usize,
    result: &mut LevelDbBattleListResult,
    seen: &mut HashSet<String>,
) {
    let scan_end = bytes
        .len()
        .min(key_start.saturating_add(MAX_ACCOUNT_ROLE_SCAN_BYTES));
    let window = &bytes[key_start..scan_end];
    let raw_text = String::from_utf8_lossy(window).into_owned();

    let mut position = 0;
    while let Some(anchor_offset) =
        find_subslice_ascii_case_insensitive(&window[position..], b"openi")
    {
        let anchor = position + anchor_offset;
        let Some((account_id, account_end)) = find_digit_run_after(window, anchor, 160, 17, 20)
        else {
            position = anchor.saturating_add(5);
            continue;
        };
        let search_end = window.len().min(account_end.saturating_add(1024));
        let Some((nick_name, nick_end)) =
            find_stringish_value_after(window, b"nick", account_end, search_end)
        else {
            position = account_end;
            continue;
        };

        let tag_line = find_stringish_value_after(window, b"tag", nick_end, search_end)
            .map(|(value, _)| value)
            .filter(|value| looks_like_tag_line(value))
            .or_else(|| find_digit_tag_after_nick(window, nick_end, search_end));
        let player_name = role_player_name(Some(&nick_name), tag_line.as_deref());
        if player_name.is_none() {
            position = account_end;
            continue;
        }

        let role = LevelDbAccountRoleRecord {
            account_id,
            player_name,
            nick_name: Some(nick_name),
            tag_line,
            raw_text: raw_text.clone(),
        };
        push_account_role(result, seen, role);
        position = account_end;
    }
}

fn push_account_role(
    result: &mut LevelDbBattleListResult,
    seen: &mut HashSet<String>,
    role: LevelDbAccountRoleRecord,
) -> bool {
    let Some(player_name) = role.player_name.as_deref() else {
        return false;
    };
    if !player_name.contains('#') {
        return false;
    }

    let dedupe_key = format!("{}|{player_name}", role.account_id);
    if seen.insert(dedupe_key) {
        result.account_roles.push(role);
        true
    } else {
        false
    }
}

fn find_player_name(value: &Value) -> Option<String> {
    find_riot_id(value).or_else(|| {
        find_string_by_keys(
            value,
            &[
                "riotId",
                "riot_id",
                "gameNameWithTag",
                "playerName",
                "player_name",
                "userName",
                "nickName",
                "玩家昵称",
                "玩家名",
            ],
        )
    })
}

fn find_riot_id(value: &Value) -> Option<String> {
    let game_name = find_string_by_keys(
        value,
        &[
            "gameName",
            "GameName",
            "riotGameName",
            "riot_game_name",
            "playerGameName",
        ],
    )?;
    let tag_line = find_string_by_keys(
        value,
        &[
            "tagLine",
            "TagLine",
            "tagline",
            "riotTagLine",
            "riot_tag_line",
            "gameTag",
        ],
    )?;

    format_riot_id(&game_name, &tag_line)
}

fn format_riot_id(game_name: &str, tag_line: &str) -> Option<String> {
    let game_name = game_name.trim();
    let tag_line = tag_line.trim().trim_start_matches('#');

    if game_name.is_empty() {
        return None;
    }

    if game_name.contains('#') || tag_line.is_empty() {
        return Some(game_name.to_string());
    }

    Some(format!("{game_name}#{tag_line}"))
}

fn role_player_name(nick_name: Option<&str>, tag_line: Option<&str>) -> Option<String> {
    let nick_name = nick_name?.trim();
    if nick_name.is_empty() {
        return None;
    }

    if nick_name.contains('#') {
        return Some(nick_name.to_string());
    }

    format_riot_id(nick_name, tag_line?)
}

fn find_json_array_start(bytes: &[u8], from: usize) -> Option<(usize, JsonEncoding)> {
    let limit = bytes
        .len()
        .min(from.saturating_add(MAX_JSON_START_DISTANCE));
    for index in from..limit {
        if bytes.get(index) != Some(&b'[') {
            continue;
        }

        if bytes.get(index + 1) == Some(&0) {
            return Some((index, JsonEncoding::Utf16Le));
        }

        return Some((index, JsonEncoding::Utf8));
    }

    None
}

fn extract_json_array(bytes: &[u8], start: usize, encoding: JsonEncoding) -> Option<String> {
    let end = bytes.len().min(start.saturating_add(MAX_JSON_BYTES));
    let text = match encoding {
        JsonEncoding::Utf16Le => decode_utf16le_lossy(&bytes[start..end]),
        JsonEncoding::Utf8 => String::from_utf8_lossy(&bytes[start..end]).into_owned(),
    };

    json_array_prefix(&text)
}

fn json_array_prefix(text: &str) -> Option<String> {
    let start = text.find('[')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, character) in text[start..].char_indices() {
        let absolute_index = start + index;
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match character {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(text[start..absolute_index + character.len_utf8()].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

fn decode_utf16le_lossy(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&units)
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

fn build_kda(kills: Option<i64>, deaths: Option<i64>, assists: Option<i64>) -> Option<String> {
    Some(format!("{}/{}/{}", kills?, deaths?, assists?))
}

fn compact_ascii_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .filter_map(|byte| match *byte {
            0 => None,
            b'\t' | b'\r' | b'\n' => Some(' '),
            0x20..=0x7e => Some(*byte as char),
            0x01..=0x1f => None,
            _ => Some(' '),
        })
        .collect()
}

fn find_uuid_mentions_after_keywords(text: &str, keywords: &[&str]) -> Vec<(String, usize)> {
    let lower = text.to_ascii_lowercase();
    let mut mentions = Vec::new();
    for keyword in keywords {
        let mut position = 0;
        while let Some(offset) = lower[position..].find(keyword) {
            let keyword_start = position + offset;
            let search_start = keyword_start + keyword.len();
            let search_end = lower.len().min(search_start.saturating_add(320));
            if let Some(mention) = find_uuid_in(text, search_start, search_end) {
                mentions.push(mention);
            }
            position = search_start;
        }
    }
    mentions.sort_by_key(|(_uuid, position)| *position);
    mentions.dedup_by(|left, right| left.0 == right.0);
    mentions
}

fn find_uuid_in(text: &str, start: usize, end: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let limit = bytes.len().min(end);
    let mut index = start.min(limit);
    while index < limit {
        if let Some(uuid) =
            uuid_at(bytes, index, limit).or_else(|| noisy_uuid_at(bytes, index, limit))
        {
            return Some((uuid, index));
        }
        index += 1;
    }
    None
}

fn uuid_at(bytes: &[u8], start: usize, limit: usize) -> Option<String> {
    let groups = [8usize, 4, 4, 4, 12];
    let mut index = start;
    for (group_index, group_len) in groups.iter().enumerate() {
        if index + group_len > limit {
            return None;
        }
        if !bytes[index..index + group_len]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        index += group_len;
        if group_index + 1 < groups.len() {
            if bytes.get(index) != Some(&b'-') {
                return None;
            }
            index += 1;
        }
    }

    Some(String::from_utf8_lossy(&bytes[start..index]).to_string())
}

fn noisy_uuid_at(bytes: &[u8], start: usize, limit: usize) -> Option<String> {
    if !bytes.get(start).is_some_and(u8::is_ascii_hexdigit) {
        return None;
    }

    let groups = [8usize, 4, 4, 4, 12];
    let mut index = start;
    let mut output = String::new();

    for (group_index, group_len) in groups.iter().enumerate() {
        for _ in 0..*group_len {
            index = skip_uuid_noise(bytes, index, limit, false)?;
            if index >= limit || !bytes[index].is_ascii_hexdigit() {
                return None;
            }
            output.push((bytes[index] as char).to_ascii_lowercase());
            index += 1;
        }

        if group_index + 1 < groups.len() {
            index = skip_uuid_noise(bytes, index, limit, true)?;
            if bytes.get(index) != Some(&b'-') {
                return None;
            }
            output.push('-');
            index += 1;
        }
    }

    Some(output)
}

fn skip_uuid_noise(
    bytes: &[u8],
    mut index: usize,
    limit: usize,
    looking_for_hyphen: bool,
) -> Option<usize> {
    let mut skipped = 0usize;
    while index < limit {
        let byte = bytes[index];
        if byte.is_ascii_hexdigit() || byte == b'-' {
            return Some(index);
        }
        if !is_uuid_noise_byte(byte) || skipped >= 8 {
            return None;
        }
        index += 1;
        skipped += 1;
    }

    if looking_for_hyphen {
        None
    } else {
        Some(index)
    }
}

fn is_uuid_noise_byte(byte: u8) -> bool {
    byte.is_ascii_alphabetic()
        || matches!(
            byte,
            b' ' | b'"' | b'\'' | b':' | b'_' | b'{' | b'}' | b',' | b'\\'
        )
}

fn find_kda_in(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    lower
        .find("kda")
        .and_then(|index| find_kda_pattern(&text[index..]))
        .or_else(|| find_kda_pattern(text))
}

fn find_kda_pattern(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        let Some((kills, after_kills)) = read_small_number(bytes, index) else {
            index += 1;
            continue;
        };
        if bytes.get(after_kills) != Some(&b'/') {
            index = after_kills;
            continue;
        }
        let Some((deaths, after_deaths)) = read_small_number(bytes, after_kills + 1) else {
            index = after_kills + 1;
            continue;
        };
        if bytes.get(after_deaths) != Some(&b'/') {
            index = after_deaths;
            continue;
        }
        let Some((assists, _after_assists)) = read_small_number(bytes, after_deaths + 1) else {
            index = after_deaths + 1;
            continue;
        };
        return Some(format!("{kills}/{deaths}/{assists}"));
    }

    None
}

fn read_small_number(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }

    let digit_count = end.saturating_sub(start);
    if (1..=3).contains(&digit_count) {
        let value = String::from_utf8_lossy(&bytes[start..end])
            .parse::<i64>()
            .ok()?
            .to_string();
        Some((value, end))
    } else {
        None
    }
}

fn find_avatar_url_in(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for marker in ["headico/", "headicon/"] {
        let Some(marker_index) = lower.find(marker) else {
            continue;
        };
        let start = lower[..marker_index].rfind("http").unwrap_or(marker_index);
        let mut end = marker_index + marker.len();
        let bytes = text.as_bytes();
        while end < bytes.len()
            && !matches!(
                bytes[end],
                b'"' | b'\'' | b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n'
            )
        {
            end += 1;
        }

        let value = text[start..end]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }

    None
}

fn find_digit_run_after(
    bytes: &[u8],
    start: usize,
    max_distance: usize,
    min_len: usize,
    max_len: usize,
) -> Option<(String, usize)> {
    let limit = bytes.len().min(start.saturating_add(max_distance));
    let mut index = start;
    while index < limit {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        let digit_start = index;
        while index < limit && bytes[index].is_ascii_digit() {
            index += 1;
        }

        let digit_count = index - digit_start;
        if digit_count >= min_len && digit_count <= max_len {
            return Some((
                String::from_utf8_lossy(&bytes[digit_start..index]).to_string(),
                index,
            ));
        }
    }

    None
}

fn find_digit_tag_after_nick(bytes: &[u8], start: usize, end: usize) -> Option<String> {
    let limit = find_subslice_ascii_case_insensitive(&bytes[start..end], b"avatar")
        .map(|offset| start + offset)
        .unwrap_or(end);
    let mut index = start;
    let mut best = None;

    while index < limit {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        let digit_start = index;
        while index < limit && bytes[index].is_ascii_digit() {
            index += 1;
        }

        let digit_count = index - digit_start;
        if (5..=6).contains(&digit_count) {
            best = Some(String::from_utf8_lossy(&bytes[digit_start..index]).to_string());
        }
    }

    best
}

fn find_stringish_value_after(
    bytes: &[u8],
    key: &[u8],
    start: usize,
    end: usize,
) -> Option<(String, usize)> {
    let key_start = start + find_subslice_ascii_case_insensitive(&bytes[start..end], key)?;
    let mut index = key_start + key.len();

    while index < end && bytes[index] != b'"' {
        index += 1;
    }
    index += 1;
    while index < end && bytes[index] != b'"' {
        index += 1;
    }
    index += 1;

    let value_start = index;
    while index < end {
        let byte = bytes[index];
        if byte == b'"'
            || byte == b','
            || byte == b'{'
            || byte == b'}'
            || byte == b'['
            || byte == b']'
            || byte < 0x20
        {
            break;
        }
        index += 1;
    }

    let value = String::from_utf8_lossy(&bytes[value_start..index])
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_string();
    if value.is_empty() {
        None
    } else {
        Some((value, index))
    }
}

fn read_ascii_digits_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    end
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_subslice_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn looks_like_account_id(value: &str) -> bool {
    let trimmed = value.trim();
    (10..=24).contains(&trimmed.len())
        && trimmed.chars().all(|character| character.is_ascii_digit())
}

fn looks_like_tag_line(value: &str) -> bool {
    let trimmed = value.trim().trim_start_matches('#');
    (3..=6).contains(&trimmed.len())
        && trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn is_leveldb_candidate_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name == "CURRENT" || name.starts_with("MANIFEST-") {
        return true;
    }

    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("ldb") || extension.eq_ignore_ascii_case("log")
        })
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

fn path_to_string(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
