use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;

use crate::display_names;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    Parsed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedVideoType {
    Triple,
    Quad,
    Five,
    Six,
    KillCompilation,
    Death,
}

/// Describes how WonderfulDb's `event_sTime` is positioned inside an exported video.
///
/// Compilation exports already store an absolute video offset. Ordinary highlights
/// store an offset relative to their containing round segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineEventTimeSemantics {
    SegmentRelative,
    VideoAbsolute,
}

/// Classifies the event-time convention used by a WonderfulDb video record.
///
/// Positive numeric highlight types are authoritative,
/// while names/types provide compatibility with older records that did not persist
/// `highLightType`. Unknown records deliberately remain unclassified so callers do
/// not guess whether an event offset is relative or absolute.
pub fn classify_timeline_event_time(
    highlight_type: Option<i64>,
    video_name: &str,
    video_type: &str,
) -> Option<TimelineEventTimeSemantics> {
    let descriptive_text = format!("{video_name} {video_type}");

    if matches!(highlight_type, Some(2 | 3))
        || is_kill_collection_text(&descriptive_text)
        || is_death_moment_text(&descriptive_text)
    {
        return Some(TimelineEventTimeSemantics::VideoAbsolute);
    }

    if highlight_type.is_some_and(|highlight_type| highlight_type > 0)
        || contains_any(&descriptive_text, &["三杀", "3杀", "三连杀"])
        || contains_any(&descriptive_text, &["四杀", "4杀", "四连杀"])
        || is_five_kill_text(&descriptive_text)
        || is_six_kill_text(&descriptive_text)
    {
        return Some(TimelineEventTimeSemantics::SegmentRelative);
    }

    None
}

impl DetectedVideoType {
    pub fn highlight_type(self) -> i64 {
        match self {
            Self::KillCompilation => 2,
            Self::Death => 3,
            Self::Triple => 4,
            Self::Quad => 6,
            Self::Five | Self::Six => 10,
        }
    }

    pub fn kill_count(self) -> Option<i64> {
        match self {
            Self::Triple => Some(3),
            Self::Quad => Some(4),
            Self::Five => Some(5),
            Self::Six => Some(6),
            Self::KillCompilation | Self::Death => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoExportConfigMetadata {
    pub json_path: String,
    pub extracted_text: String,
    pub parse_status: ParseStatus,
    pub is_mvp: bool,
    pub is_triple_kill: bool,
    pub is_quadra_kill: bool,
    pub is_ace: bool,
    pub kda: Option<String>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub agent_name: Option<String>,
    pub player_name: Option<String>,
    pub parse_error: Option<String>,
}

impl VideoExportConfigMetadata {
    pub fn detected_video_type(&self) -> Option<DetectedVideoType> {
        if is_death_moment_text(&self.extracted_text) {
            return Some(DetectedVideoType::Death);
        }
        if is_kill_collection_text(&self.extracted_text) {
            return Some(DetectedVideoType::KillCompilation);
        }
        if is_six_kill_text(&self.extracted_text) {
            return Some(DetectedVideoType::Six);
        }
        if is_five_kill_text(&self.extracted_text) || self.is_ace {
            return Some(DetectedVideoType::Five);
        }
        if self.is_quadra_kill {
            return Some(DetectedVideoType::Quad);
        }
        if self.is_triple_kill {
            return Some(DetectedVideoType::Triple);
        }

        None
    }
}

pub fn parse_video_export_configs(
    source_dir: impl AsRef<Path>,
) -> Result<Vec<VideoExportConfigMetadata>, String> {
    let config_dir = source_dir.as_ref().join("videoExportTmp");
    if !config_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut config_paths = fs::read_dir(&config_dir)
        .map_err(|error| {
            format!(
                "Failed to read metadata directory {}: {error}",
                path_to_string(&config_dir)
            )
        })?
        .map(|entry| {
            entry.map(|entry| entry.path()).map_err(|error| {
                format!(
                    "Failed to read metadata entry {}: {error}",
                    path_to_string(&config_dir)
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    config_paths.retain(|path| is_video_export_config(path));
    config_paths.sort_by_key(|path| path_to_string(path).to_lowercase());

    Ok(config_paths
        .into_iter()
        .map(parse_video_export_config)
        .collect())
}

fn parse_video_export_config(path: PathBuf) -> VideoExportConfigMetadata {
    let json_path = path_to_string(&path);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            return failed_metadata(json_path, format!("Failed to read JSON: {error}"));
        }
    };

    let value = match serde_json::from_str::<Value>(&content) {
        Ok(value) => value,
        Err(error) => {
            return failed_metadata(json_path, format!("Failed to parse JSON: {error}"));
        }
    };

    let mut strings = Vec::new();
    collect_strings(&value, &mut strings);

    let extracted_text = strings
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if extracted_text.is_empty() {
        return failed_metadata(json_path, "No string fields found".to_string());
    }

    let is_mvp = contains_case_insensitive(&extracted_text, "MVP");
    let is_triple_kill = contains_any(&extracted_text, &["三杀", "3杀", "三连杀"]);
    let is_quadra_kill = contains_any(&extracted_text, &["四杀", "4杀", "四连杀"]);
    let is_five_kill = is_five_kill_text(&extracted_text);
    let is_six_kill = is_six_kill_text(&extracted_text);
    let is_ace = contains_case_insensitive(&extracted_text, "ACE");
    let kda = find_kda(&extracted_text);
    let map_name = find_labeled_value(&strings, &["地图名", "地图名称", "地图", "map name", "map"])
        .or_else(|| find_known_value(&strings, MAP_NAMES))
        .and_then(|value| display_names::map_name_for_display(&value));
    let game_mode = find_labeled_value(
        &strings,
        &[
            "模式名",
            "模式名称",
            "游戏模式",
            "对局模式",
            "模式",
            "game mode",
            "mode",
        ],
    )
    .or_else(|| find_known_value(&strings, GAME_MODES))
    .and_then(|value| display_names::game_mode_for_display(&value));
    let agent_name = find_labeled_value(
        &strings,
        &[
            "英雄名",
            "英雄名称",
            "英雄",
            "agent name",
            "agentName",
            "agent_name",
            "heroName",
            "hero_name",
        ],
    )
    .and_then(|value| display_names::agent_name_for_display(&value))
    .or_else(|| display_names::agent_name_from_export_text(&extracted_text));
    let player_name = find_riot_id_value(&strings).or_else(|| {
        find_labeled_value(
            &strings,
            &[
                "玩家昵称",
                "玩家名",
                "玩家名称",
                "昵称",
                "riot id",
                "riotId",
                "gameNameWithTag",
                "player name",
                "playerName",
                "player_name",
                "nickName",
                "player",
            ],
        )
        .and_then(|value| display_names::player_name_for_display(&value))
    });

    let has_structured_value = is_mvp
        || is_triple_kill
        || is_quadra_kill
        || is_five_kill
        || is_six_kill
        || is_ace
        || kda.is_some()
        || map_name.is_some()
        || game_mode.is_some()
        || agent_name.is_some()
        || player_name.is_some();
    let parse_status = if has_structured_value {
        ParseStatus::Parsed
    } else {
        ParseStatus::Partial
    };

    VideoExportConfigMetadata {
        json_path,
        extracted_text,
        parse_status,
        is_mvp,
        is_triple_kill,
        is_quadra_kill,
        is_ace,
        kda,
        map_name,
        game_mode,
        agent_name,
        player_name,
        parse_error: None,
    }
}

fn failed_metadata(json_path: String, parse_error: String) -> VideoExportConfigMetadata {
    VideoExportConfigMetadata {
        json_path,
        extracted_text: String::new(),
        parse_status: ParseStatus::Failed,
        is_mvp: false,
        is_triple_kill: false,
        is_quadra_kill: false,
        is_ace: false,
        kda: None,
        map_name: None,
        game_mode: None,
        agent_name: None,
        player_name: None,
        parse_error: Some(parse_error),
    }
}

fn collect_strings(value: &Value, strings: &mut Vec<String>) {
    match value {
        Value::String(value) => {
            let repaired = repair_mojibake(value).unwrap_or_else(|| value.to_string());
            let trimmed = repaired.trim();
            if !trimmed.is_empty() {
                strings.push(trimmed.to_string());
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_strings(value, strings);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if let Value::String(value) = value {
                    let repaired = repair_mojibake(value).unwrap_or_else(|| value.to_string());
                    let trimmed = repaired.trim();
                    if !trimmed.is_empty() {
                        strings.push(format!("{key}: {trimmed}"));
                    }
                    continue;
                }

                collect_strings(value, strings);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn repair_mojibake(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut has_non_ascii_byte = false;

    for character in value.chars() {
        let byte = mojibake_byte(character)?;
        if byte > 0x7f {
            has_non_ascii_byte = true;
        }
        bytes.push(byte);
    }

    if !has_non_ascii_byte {
        return None;
    }

    String::from_utf8(bytes)
        .ok()
        .filter(|repaired| repaired != value)
}

fn mojibake_byte(character: char) -> Option<u8> {
    let codepoint = character as u32;
    if codepoint <= u8::MAX as u32 {
        return Some(codepoint as u8);
    }

    Some(match character {
        '\u{20AC}' => 0x80,
        '\u{201A}' => 0x82,
        '\u{0192}' => 0x83,
        '\u{201E}' => 0x84,
        '\u{2026}' => 0x85,
        '\u{2020}' => 0x86,
        '\u{2021}' => 0x87,
        '\u{02C6}' => 0x88,
        '\u{2030}' => 0x89,
        '\u{0160}' => 0x8a,
        '\u{2039}' => 0x8b,
        '\u{0152}' => 0x8c,
        '\u{017D}' => 0x8e,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201C}' => 0x93,
        '\u{201D}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{02DC}' => 0x98,
        '\u{2122}' => 0x99,
        '\u{0161}' => 0x9a,
        '\u{203A}' => 0x9b,
        '\u{0153}' => 0x9c,
        '\u{017E}' => 0x9e,
        '\u{0178}' => 0x9f,
        _ => return None,
    })
}

fn is_video_export_config(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("config-"))
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn find_kda(text: &str) -> Option<String> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        if !chars[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        if let Some((candidate, _next_index)) = read_kda_at(&chars, index) {
            return Some(candidate);
        }

        index += 1;
    }

    None
}

fn read_kda_at(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut index = start;
    let kills = read_digits(chars, &mut index)?;
    if chars.get(index) != Some(&'/') {
        return None;
    }
    index += 1;
    let deaths = read_digits(chars, &mut index)?;
    if chars.get(index) != Some(&'/') {
        return None;
    }
    index += 1;
    let assists = read_digits(chars, &mut index)?;

    Some((format!("{kills}/{deaths}/{assists}"), index))
}

fn read_digits(chars: &[char], index: &mut usize) -> Option<String> {
    let start = *index;
    while *index < chars.len() && chars[*index].is_ascii_digit() {
        *index += 1;
    }

    if *index == start {
        None
    } else {
        Some(chars[start..*index].iter().collect())
    }
}

fn find_labeled_value(strings: &[String], labels: &[&str]) -> Option<String> {
    for value in strings {
        for label in labels {
            if let Some(candidate) = value_after_label(value, label) {
                return Some(candidate);
            }
        }
    }

    None
}

fn find_riot_id_value(strings: &[String]) -> Option<String> {
    for value in strings {
        for segment in value.split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    ',' | '，' | ';' | '；' | '|' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}'
                )
        }) {
            if let Some(riot_id) = normalize_riot_id_segment(segment) {
                return Some(riot_id);
            }
        }
    }

    None
}

fn normalize_riot_id_segment(segment: &str) -> Option<String> {
    let candidate = segment
        .rsplit([':', '：', '='])
        .next()
        .unwrap_or(segment)
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    ',' | '，' | ';' | '；' | '|' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}'
                )
        });

    let (_name, tag) = candidate.split_once('#')?;
    let tag = tag.trim().trim_start_matches('#');
    if tag.is_empty()
        || tag.len() > 12
        || !tag
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }

    display_names::player_name_for_display(candidate)
}

fn value_after_label(value: &str, label: &str) -> Option<String> {
    let lower_value = value.to_lowercase();
    let lower_label = label.to_lowercase();
    let label_index = lower_value.find(&lower_label)?;
    let remainder_start = label_index + lower_label.len();
    let candidate = clean_labeled_value(&value[remainder_start..]);

    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

fn clean_labeled_value(value: &str) -> String {
    let trimmed = value.trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, ':' | '：' | '=' | '-' | '_' | ' ' | '、')
    });
    let mut candidate = trimmed
        .split(|character: char| {
            matches!(
                character,
                ',' | '，' | ';' | '；' | '|' | '\n' | '\r' | '\t'
            )
        })
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();

    for label in SECONDARY_LABELS {
        if let Some(index) = secondary_label_index(&candidate, label) {
            candidate.truncate(index);
            candidate = candidate.trim().to_string();
        }
    }

    candidate
}

fn secondary_label_index(value: &str, label: &str) -> Option<usize> {
    let lower_value = value.to_lowercase();
    let lower_label = label.to_lowercase();
    let mut search_start = 0;

    while let Some(relative_index) = lower_value[search_start..].find(&lower_label) {
        let index = search_start + relative_index;
        if index > 0 && is_boundary_before(value, index) {
            return Some(index);
        }
        search_start = index + lower_label.len();
    }

    None
}

fn is_boundary_before(value: &str, byte_index: usize) -> bool {
    value[..byte_index]
        .chars()
        .next_back()
        .is_some_and(|character| {
            character.is_whitespace()
                || matches!(character, ',' | '，' | ';' | '；' | '|' | ':' | '：' | '=')
        })
}

fn find_known_value(strings: &[String], known_values: &[&str]) -> Option<String> {
    for value in strings {
        for known_value in known_values {
            if contains_case_insensitive(value, known_value) {
                return Some((*known_value).to_string());
            }
        }
    }

    None
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| contains_case_insensitive(text, needle))
}

fn is_kill_collection_text(text: &str) -> bool {
    contains_any(text, KILL_COLLECTION_KEYWORDS)
}

fn is_death_moment_text(text: &str) -> bool {
    contains_any(
        text,
        &["死亡时刻", "死亡集锦", "death moment", "death compilation"],
    )
}

fn is_five_kill_text(text: &str) -> bool {
    contains_highlight_marker(text, &["五杀", "5杀", "五连杀"])
}

fn is_six_kill_text(text: &str) -> bool {
    contains_highlight_marker(text, &["六杀", "6杀", "六连杀"])
}

fn contains_highlight_marker(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| {
        text.match_indices(marker)
            .any(|(index, _)| has_highlight_marker_boundary(&text[index + marker.len()..]))
    })
}

fn has_highlight_marker_boundary(remainder: &str) -> bool {
    if remainder.is_empty()
        || HIGHLIGHT_MARKER_SUFFIXES
            .iter()
            .any(|suffix| remainder.starts_with(suffix))
    {
        return true;
    }

    remainder.chars().next().is_some_and(|character| {
        character.is_whitespace()
            || character.is_ascii_punctuation()
            || matches!(
                character,
                '，' | '。'
                    | '！'
                    | '？'
                    | '：'
                    | '；'
                    | '、'
                    | '—'
                    | '…'
                    | '（'
                    | '）'
                    | '【'
                    | '】'
                    | '《'
                    | '》'
                    | '“'
                    | '”'
                    | '‘'
                    | '’'
            )
    })
}

fn contains_case_insensitive(text: &str, needle: &str) -> bool {
    text.to_lowercase().contains(&needle.to_lowercase())
}

fn path_to_string(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

const SECONDARY_LABELS: &[&str] = &[
    "地图名",
    "地图名称",
    "地图",
    "模式名",
    "模式名称",
    "游戏模式",
    "对局模式",
    "模式",
    "玩家昵称",
    "玩家名",
    "玩家名称",
    "昵称",
    "map name",
    "game mode",
    "player name",
    "map",
    "mode",
    "player",
];

const MAP_NAMES: &[&str] = &[
    "源工重镇",
    "隐世修所",
    "霓虹町",
    "双塔迷城",
    "亚海悬城",
    "森寒冬港",
    "微风岛屿",
    "裂变峡谷",
    "深海明珠",
    "莲华古城",
    "日落之城",
    "迷邃幽境",
    "Ascent",
    "Haven",
    "Split",
    "Bind",
    "Icebox",
    "Breeze",
    "Fracture",
    "Pearl",
    "Lotus",
    "Sunset",
    "Abyss",
];

const GAME_MODES: &[&str] = &[
    "竞技模式",
    "未评级",
    "普通模式",
    "极速模式",
    "乱斗模式",
    "死斗模式",
    "团队乱斗",
    "复制乱战",
    "自定义游戏",
    "排位",
    "Competitive",
    "Unrated",
    "Swiftplay",
    "Spike Rush",
    "Deathmatch",
    "Team Deathmatch",
    "Custom Game",
];

const KILL_COLLECTION_KEYWORDS: &[&str] = &[
    "击杀合集",
    "击杀集锦",
    "击杀剪辑",
    "击杀片段合集",
    "高光合集",
    "精彩击杀合集",
    "kill compilation",
    "kill collection",
    "kill montage",
    "kills montage",
    "highlight compilation",
    "highlight reel",
];

const HIGHLIGHT_MARKER_SUFFIXES: &[&str] =
    &["时刻", "集锦", "高光", "片段", "剪辑", "回放", "合集"];
