use aes::Aes256;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::string::FromUtf8Error;

use crate::display_names;

type Aes256CbcDec = cbc::Decryptor<Aes256>;

#[derive(Debug)]
pub enum WonderfulDbError {
    Io {
        account: String,
        source: io::Error,
    },
    InvalidHex {
        account: String,
        source: hex::FromHexError,
    },
    InvalidKeyMaterial {
        account: String,
    },
    DecryptFailed {
        account: String,
    },
    InvalidUtf8 {
        account: String,
        source: FromUtf8Error,
    },
    InvalidJson {
        account: String,
        source: serde_json::Error,
    },
    InvalidAccountRecord {
        account: String,
    },
}

impl fmt::Display for WonderfulDbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { account, .. } => {
                write!(
                    formatter,
                    "WonderfulDb account file {account} could not be read"
                )
            }
            Self::InvalidHex { account, .. } => {
                write!(
                    formatter,
                    "WonderfulDb account file {account} is not valid hexadecimal"
                )
            }
            Self::InvalidKeyMaterial { account } => write!(
                formatter,
                "WonderfulDb account file {account} has invalid cryptographic key material"
            ),
            Self::DecryptFailed { account } => write!(
                formatter,
                "WonderfulDb account file {account} could not be decrypted"
            ),
            Self::InvalidUtf8 { account, .. } => write!(
                formatter,
                "WonderfulDb account file {account} contains invalid UTF-8"
            ),
            Self::InvalidJson { account, .. } => write!(
                formatter,
                "WonderfulDb account file {account} contains invalid JSON"
            ),
            Self::InvalidAccountRecord { account } => write!(
                formatter,
                "WonderfulDb account file {account} has an invalid account record"
            ),
        }
    }
}

impl Error for WonderfulDbError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidHex { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::InvalidJson { source, .. } => Some(source),
            Self::InvalidKeyMaterial { .. }
            | Self::DecryptFailed { .. }
            | Self::InvalidAccountRecord { .. } => None,
        }
    }
}

pub fn decrypt_wonderful_db_text(
    openid: &str,
    ciphertext_hex: &str,
) -> Result<String, WonderfulDbError> {
    let digest = format!("{:x}", Sha256::digest(openid.as_bytes()));
    let key = &digest.as_bytes()[..32];
    let iv = &digest.as_bytes()[..16];
    let ciphertext =
        hex::decode(ciphertext_hex.trim()).map_err(|source| WonderfulDbError::InvalidHex {
            account: openid.to_owned(),
            source,
        })?;
    let plaintext = Aes256CbcDec::new_from_slices(key, iv)
        .map_err(|_| WonderfulDbError::InvalidKeyMaterial {
            account: openid.to_owned(),
        })?
        .decrypt_padded_vec_mut::<Pkcs7>(&ciphertext)
        .map_err(|_| WonderfulDbError::DecryptFailed {
            account: openid.to_owned(),
        })?;
    String::from_utf8(plaintext).map_err(|source| WonderfulDbError::InvalidUtf8 {
        account: openid.to_owned(),
        source,
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WonderfulMatchRecord {
    pub match_id: String,
    pub battle_id: Option<String>,
    pub match_time: Option<String>,
    pub map_id: Option<String>,
    pub map_name: Option<String>,
    pub agent_name: Option<String>,
    pub agent_avatar_url: Option<String>,
    pub game_mode: Option<String>,
    pub kda: Option<String>,
    pub scoreline: Option<String>,
    pub combat_score: Option<i64>,
    pub has_won: Option<bool>,
    pub career: Option<Value>,
    pub videos: Vec<WonderfulVideoRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WonderfulVideoRecord {
    pub video_id: String,
    pub video_name: String,
    pub video_type: String,
    pub highlight_type: Option<i64>,
    pub video_src: Option<String>,
    pub round_score: Option<i64>,
    pub segments: Vec<WonderfulSegmentRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WonderfulSegmentRecord {
    pub segment_id: String,
    pub round_id: Option<i64>,
    pub clip_start_ms: Option<i64>,
    pub clip_end_ms: Option<i64>,
    pub events: Vec<WonderfulEventRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WonderfulEventRecord {
    pub event_id: String,
    pub event_type: String,
    pub video_time_ms: Option<i64>,
    pub event_time: Option<String>,
    pub round_id: Option<i64>,
    pub player_name: Option<String>,
    pub agent_name: Option<String>,
    pub weapon_name: Option<String>,
    pub killer_name: Option<String>,
    pub killed_name: Option<String>,
    pub killer_is_me: bool,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WonderfulAccountRecord {
    pub openid: String,
    pub matches: Vec<WonderfulMatchRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WonderfulSnapshotRecord {
    pub match_record: WonderfulMatchRecord,
    pub snapshot_id: String,
    pub captured_at: Option<String>,
    pub account_name: Option<String>,
    pub package_path: Option<String>,
    pub thumb_path: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub size_bytes: Option<i64>,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WonderfulSnapshotAccountRecord {
    pub openid: String,
    pub snapshots: Vec<WonderfulSnapshotRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WonderfulDbWarning {
    pub account_filename: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WonderfulDbReadResult {
    pub accounts: Vec<WonderfulAccountRecord>,
    pub snapshot_accounts: Vec<WonderfulSnapshotAccountRecord>,
    pub warnings: Vec<WonderfulDbWarning>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawMatchRecord {
    #[serde(alias = "matchId", alias = "matches_id")]
    match_id: Value,
    #[serde(alias = "matchTime", alias = "match_startTime", alias = "matches_time")]
    match_time: Option<Value>,
    #[serde(alias = "mapName", alias = "match_map")]
    map_name: Option<Value>,
    map: Option<Value>,
    agent: Option<Value>,
    mode: Option<Value>,
    stats: Option<Value>,
    career: Option<Value>,
    #[serde(alias = "video_list", alias = "videoList")]
    videos: Vec<RawVideoRecord>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawVideoRecord {
    #[serde(alias = "videoId")]
    video_id: Value,
    #[serde(alias = "videoName")]
    video_name: Value,
    #[serde(alias = "videoType")]
    video_type: Value,
    #[serde(
        rename = "highLightType",
        alias = "highlightType",
        alias = "highlight_type"
    )]
    highlight_type: Option<Value>,
    #[serde(alias = "videoPath", alias = "video_path")]
    video_src: Option<Value>,
    #[serde(alias = "roundScore")]
    round_score: Option<Value>,
    #[serde(alias = "roundClips", alias = "segments")]
    round_clips: Vec<RawSegmentRecord>,
    rounds: Vec<RawRoundRecord>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawRoundRecord {
    #[serde(alias = "roundId")]
    round_id: Option<Value>,
    #[serde(alias = "roundClips", alias = "segments")]
    round_clips: Vec<RawSegmentRecord>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawSegmentRecord {
    #[serde(alias = "segmentId", alias = "clip_id")]
    segment_id: Value,
    #[serde(alias = "roundId")]
    round_id: Option<Value>,
    #[serde(rename = "clip_sTime", alias = "clip_start_ms")]
    clip_start_ms: Option<Value>,
    #[serde(rename = "clip_eTime", alias = "clip_end_ms")]
    clip_end_ms: Option<Value>,
    #[serde(alias = "clipDuration")]
    clip_duration: Option<Value>,
    #[serde(alias = "clipEvents", alias = "events")]
    clip_events: Vec<Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawEventRecord {
    #[serde(alias = "eventId")]
    event_id: Value,
    #[serde(alias = "eventType")]
    event_type: Value,
    #[serde(rename = "event_sTime", alias = "event_start_ms")]
    event_start_ms: Option<Value>,
    #[serde(alias = "eventTime")]
    event_time: Option<Value>,
    #[serde(alias = "roundId")]
    round_id: Option<Value>,
    #[serde(alias = "playerName")]
    player_name: Option<Value>,
    #[serde(alias = "agentName")]
    agent_name: Option<Value>,
    #[serde(alias = "weaponName")]
    weapon_name: Option<Value>,
    #[serde(alias = "killerName")]
    killer_name: Option<Value>,
    #[serde(alias = "killedName")]
    killed_name: Option<Value>,
    #[serde(alias = "killerIsMe")]
    killer_is_me: bool,
    event_ext: Option<RawEventExt>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawEventExt {
    #[serde(rename = "EventTime")]
    event_time: Option<Value>,
    #[serde(rename = "PlayerName")]
    player_name: Option<Value>,
    #[serde(rename = "AgentName")]
    agent_name: Option<Value>,
    #[serde(rename = "RoundID")]
    round_id: Option<Value>,
    #[serde(rename = "WeaponID")]
    weapon_id: Option<Value>,
    #[serde(rename = "WeaponSkinName")]
    weapon_skin_name: Option<Value>,
    #[serde(rename = "KillerPlayerName")]
    killer_player_name: Option<Value>,
    #[serde(rename = "KilledPlayerName")]
    killed_player_name: Option<Value>,
    #[serde(rename = "KillerIsMe")]
    killer_is_me: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawSnapshotEnvelope {
    snapshot: Option<RawSnapshotData>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawSnapshotData {
    ss_id: Value,
    ss_time: Option<Value>,
    ss_package_src: Option<Value>,
    ss_thumb_src: Option<Value>,
    ss_thumb_path: Option<Value>,
    ss_width: Option<Value>,
    ss_height: Option<Value>,
    ss_size: Option<Value>,
    ss_nick: Option<Value>,
    ss_nick_id: Option<Value>,
}

pub fn parse_wonderful_db_text(
    openid: &str,
    plaintext: &str,
) -> Result<Vec<WonderfulMatchRecord>, WonderfulDbError> {
    let root: Value =
        serde_json::from_str(plaintext).map_err(|source| WonderfulDbError::InvalidJson {
            account: openid.to_owned(),
            source,
        })?;
    let account_key = format!("key_wonderful_list_{openid}");
    let records = root
        .get(&account_key)
        .and_then(Value::as_array)
        .ok_or_else(|| WonderfulDbError::InvalidAccountRecord {
            account: openid.to_owned(),
        })?;

    records
        .iter()
        .cloned()
        .map(|record| normalize_match(openid, record))
        .collect()
}

pub fn parse_wonderful_snapshot_text(
    openid: &str,
    plaintext: &str,
) -> Result<Vec<WonderfulSnapshotRecord>, WonderfulDbError> {
    let root: Value =
        serde_json::from_str(plaintext).map_err(|source| WonderfulDbError::InvalidJson {
            account: openid.to_owned(),
            source,
        })?;
    let snapshot_key = format!("key_snapshot_list{openid}");
    let records = root
        .get(&snapshot_key)
        .and_then(Value::as_array)
        .ok_or_else(|| WonderfulDbError::InvalidAccountRecord {
            account: format!("snapshot{openid}"),
        })?;

    records
        .iter()
        .cloned()
        .map(|record| normalize_snapshot(openid, record))
        .collect()
}

pub fn read_wonderful_db_dir(path: &Path) -> WonderfulDbReadResult {
    let mut result = WonderfulDbReadResult::default();
    let paths = match fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>(),
        Err(_) => {
            result.warnings.push(WonderfulDbWarning {
                account_filename: "<directory>".to_owned(),
                message: "WonderfulDb directory could not be read".to_owned(),
            });
            return result;
        }
    };
    let mut account_paths = paths
        .iter()
        .filter(|entry_path| is_numeric_account_file(entry_path))
        .cloned()
        .collect::<Vec<_>>();
    let mut snapshot_paths = paths
        .into_iter()
        .filter(|entry_path| is_snapshot_account_file(entry_path))
        .collect::<Vec<_>>();
    account_paths.sort();
    snapshot_paths.sort();

    for account_path in account_paths {
        let openid = account_path
            .file_name()
            .and_then(|filename| filename.to_str())
            .unwrap_or_default()
            .to_owned();
        match read_account_file(&account_path, &openid) {
            Ok(matches) => result
                .accounts
                .push(WonderfulAccountRecord { openid, matches }),
            Err(error) => result.warnings.push(WonderfulDbWarning {
                account_filename: openid,
                message: error.to_string(),
            }),
        }
    }

    for snapshot_path in snapshot_paths {
        let account_filename = snapshot_path
            .file_name()
            .and_then(|filename| filename.to_str())
            .unwrap_or_default()
            .to_owned();
        let openid = account_filename
            .strip_prefix("snapshot")
            .unwrap_or_default()
            .to_owned();
        match read_snapshot_file(&snapshot_path, &openid) {
            Ok(snapshots) => result
                .snapshot_accounts
                .push(WonderfulSnapshotAccountRecord { openid, snapshots }),
            Err(error) => result.warnings.push(WonderfulDbWarning {
                account_filename,
                message: error.to_string(),
            }),
        }
    }

    result
}

fn is_numeric_account_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|filename| filename.to_str())
        .is_some_and(|filename| {
            !filename.is_empty() && filename.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn is_snapshot_account_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|filename| filename.to_str())
        .and_then(|filename| filename.strip_prefix("snapshot"))
        .is_some_and(|openid| {
            !openid.is_empty() && openid.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn read_account_file(
    path: &Path,
    openid: &str,
) -> Result<Vec<WonderfulMatchRecord>, WonderfulDbError> {
    let ciphertext_hex = fs::read_to_string(path).map_err(|source| WonderfulDbError::Io {
        account: openid.to_owned(),
        source,
    })?;
    let plaintext = decrypt_wonderful_db_text(openid, &ciphertext_hex)?;
    parse_wonderful_db_text(openid, &plaintext)
}

fn read_snapshot_file(
    path: &Path,
    openid: &str,
) -> Result<Vec<WonderfulSnapshotRecord>, WonderfulDbError> {
    let ciphertext_hex = fs::read_to_string(path).map_err(|source| WonderfulDbError::Io {
        account: format!("snapshot{openid}"),
        source,
    })?;
    let plaintext = decrypt_wonderful_db_text(openid, &ciphertext_hex)?;
    parse_wonderful_snapshot_text(openid, &plaintext)
}

fn normalize_match(openid: &str, value: Value) -> Result<WonderfulMatchRecord, WonderfulDbError> {
    let raw: RawMatchRecord =
        serde_json::from_value(value).map_err(|source| WonderfulDbError::InvalidJson {
            account: openid.to_owned(),
            source,
        })?;

    let career = raw.career.as_ref();
    let stats = raw.stats.as_ref();
    let map_id = object_string(raw.map.as_ref(), &["map_id", "mapId", "id"])
        .or_else(|| raw.map.as_ref().and_then(value_to_string));
    let map_name = raw
        .map_name
        .as_ref()
        .and_then(nonempty_value_to_string)
        .or_else(|| object_string(raw.map.as_ref(), &["map_name", "mapName", "name"]))
        .or_else(|| object_string(career, &["map_name", "mapName"]))
        .and_then(|value| display_names::map_name_for_display(&value))
        .or_else(|| {
            map_id
                .as_deref()
                .and_then(display_names::map_name_for_display)
        });
    let agent_name = object_string(career, &["hero_name", "agent_name", "agentName"])
        .or_else(|| {
            object_string(
                raw.agent.as_ref(),
                &["agent_name", "agentName", "hero_name", "name", "agent_id"],
            )
        })
        .or_else(|| raw.agent.as_ref().and_then(value_to_string))
        .and_then(|value| {
            display_names::agent_name_for_display(&value)
                .or_else(|| display_names::agent_name_from_asset_id(&value))
        });
    let game_mode = object_string(career, &["game_mode", "gameMode"])
        .or_else(|| object_string(stats, &["mode_name", "modeName", "game_mode"]))
        .or_else(|| {
            object_string(
                raw.mode.as_ref(),
                &["mode_name", "modeName", "game_mode", "mode_id", "id"],
            )
        })
        .or_else(|| raw.mode.as_ref().and_then(value_to_string))
        .and_then(|value| display_names::game_mode_for_display(&value));
    let kills = object_i64(stats, &["kills"]);
    let deaths = object_i64(stats, &["deaths"]);
    let assists = object_i64(stats, &["assists"]);
    let rounds_won = object_i64(stats, &["rounds_won", "roundsWon"]);
    let rounds_lost = object_i64(stats, &["rounds_lost", "roundsLost"]);
    let kda = object_string(career, &["kda", "KDA"])
        .or_else(|| format_stat_triplet(kills, deaths, assists));
    let scoreline = object_string(career, &["rounds_score", "roundsScore", "scoreline"])
        .or_else(|| format_scoreline(rounds_won, rounds_lost));
    let combat_score = object_i64(career, &["score", "combat_score", "combatScore"])
        .or_else(|| object_i64(stats, &["score", "combat_score", "combatScore"]));
    let has_won = object_bool(career, &["won_match", "has_won", "hasWon"])
        .or_else(|| object_bool(stats, &["has_won", "hasWon"]));

    Ok(WonderfulMatchRecord {
        match_id: value_to_string(&raw.match_id).unwrap_or_default(),
        battle_id: object_string(career, &["battle_id", "battleId"]),
        match_time: raw
            .match_time
            .as_ref()
            .and_then(normalize_match_time)
            .or_else(|| {
                object_field(career, &["time", "match_time", "matchTime"], |value| {
                    normalize_match_time(value)
                })
            }),
        map_id,
        map_name,
        agent_name,
        agent_avatar_url: object_string(
            career,
            &["hero_image", "agent_avatar_url", "agentAvatarUrl"],
        ),
        game_mode,
        kda,
        scoreline,
        combat_score,
        has_won,
        career: raw.career,
        videos: raw
            .videos
            .into_iter()
            .map(|video| normalize_video(openid, video))
            .collect::<Result<_, _>>()?,
    })
}

fn normalize_snapshot(
    openid: &str,
    value: Value,
) -> Result<WonderfulSnapshotRecord, WonderfulDbError> {
    let raw_json =
        serde_json::to_string(&value).map_err(|source| WonderfulDbError::InvalidJson {
            account: format!("snapshot{openid}"),
            source,
        })?;
    let raw: RawSnapshotEnvelope =
        serde_json::from_value(value.clone()).map_err(|source| WonderfulDbError::InvalidJson {
            account: format!("snapshot{openid}"),
            source,
        })?;
    let snapshot = raw
        .snapshot
        .ok_or_else(|| WonderfulDbError::InvalidAccountRecord {
            account: format!("snapshot{openid}"),
        })?;
    let account_name =
        format_snapshot_account_name(snapshot.ss_nick.as_ref(), snapshot.ss_nick_id.as_ref());

    Ok(WonderfulSnapshotRecord {
        match_record: normalize_match(openid, value)?,
        snapshot_id: nonempty_value_to_string(&snapshot.ss_id).unwrap_or_default(),
        captured_at: snapshot.ss_time.as_ref().and_then(normalize_match_time),
        account_name,
        package_path: snapshot
            .ss_package_src
            .as_ref()
            .and_then(nonempty_value_to_string),
        thumb_path: snapshot
            .ss_thumb_src
            .as_ref()
            .and_then(nonempty_value_to_string)
            .or_else(|| {
                snapshot
                    .ss_thumb_path
                    .as_ref()
                    .and_then(nonempty_value_to_string)
            }),
        width: snapshot.ss_width.as_ref().and_then(value_to_i64),
        height: snapshot.ss_height.as_ref().and_then(value_to_i64),
        size_bytes: snapshot.ss_size.as_ref().and_then(value_to_i64),
        raw_json,
    })
}

fn format_snapshot_account_name(nick: Option<&Value>, tag: Option<&Value>) -> Option<String> {
    let nick = nick.and_then(nonempty_value_to_string)?;
    let tag = tag.and_then(nonempty_value_to_string);
    let combined = match tag {
        Some(tag) if !nick.contains('#') => format!("{nick}#{tag}"),
        _ => nick,
    };
    display_names::player_name_for_display(&combined)
}

fn object_field<T>(
    value: Option<&Value>,
    keys: &[&str],
    convert: impl Fn(&Value) -> Option<T>,
) -> Option<T> {
    let object = value?.as_object()?;
    keys.iter().find_map(|key| convert(object.get(*key)?))
}

fn object_string(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    object_field(value, keys, nonempty_value_to_string)
}

fn object_i64(value: Option<&Value>, keys: &[&str]) -> Option<i64> {
    object_field(value, keys, value_to_i64)
}

fn object_bool(value: Option<&Value>, keys: &[&str]) -> Option<bool> {
    object_field(value, keys, value_to_bool)
}

fn format_stat_triplet(
    kills: Option<i64>,
    deaths: Option<i64>,
    assists: Option<i64>,
) -> Option<String> {
    Some(format!("{}/{}/{}", kills?, deaths?, assists?))
}

fn format_scoreline(rounds_won: Option<i64>, rounds_lost: Option<i64>) -> Option<String> {
    Some(format!("{}/{}", rounds_won?, rounds_lost?))
}

fn normalize_match_time(value: &Value) -> Option<String> {
    let value = value_to_string(value)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let Ok(timestamp) = trimmed.parse::<i64>() else {
        return Some(trimmed.to_string());
    };
    let absolute = timestamp.unsigned_abs();
    let seconds = if absolute >= 100_000_000_000_000_000 {
        timestamp / 1_000_000_000
    } else if absolute >= 100_000_000_000_000 {
        timestamp / 1_000_000
    } else if absolute >= 100_000_000_000 {
        timestamp / 1_000
    } else {
        timestamp
    };
    Some(seconds.to_string())
}

fn normalize_video(
    openid: &str,
    raw: RawVideoRecord,
) -> Result<WonderfulVideoRecord, WonderfulDbError> {
    let segment_records = if raw.rounds.is_empty() {
        raw.round_clips
    } else {
        raw.rounds
            .into_iter()
            .flat_map(|round| {
                let parent_round_id = round.round_id;
                round.round_clips.into_iter().map(move |mut segment| {
                    if segment.round_id.is_none() {
                        segment.round_id = parent_round_id.clone();
                    }
                    segment
                })
            })
            .collect()
    };
    Ok(WonderfulVideoRecord {
        video_id: value_to_string(&raw.video_id).unwrap_or_default(),
        video_name: value_to_string(&raw.video_name).unwrap_or_default(),
        video_type: value_to_string(&raw.video_type).unwrap_or_default(),
        highlight_type: raw.highlight_type.as_ref().and_then(value_to_i64),
        video_src: raw.video_src.as_ref().and_then(value_to_string),
        round_score: raw.round_score.as_ref().and_then(value_to_i64),
        segments: segment_records
            .into_iter()
            .map(|segment| normalize_segment(openid, segment))
            .collect::<Result<_, _>>()?,
    })
}

fn normalize_segment(
    openid: &str,
    raw: RawSegmentRecord,
) -> Result<WonderfulSegmentRecord, WonderfulDbError> {
    let round_id = raw.round_id.as_ref().and_then(value_to_i64);
    let clip_start_ms = raw.clip_start_ms.as_ref().and_then(value_to_i64);
    let clip_duration = raw.clip_duration.as_ref().and_then(value_to_i64);
    let clip_end_ms = raw.clip_end_ms.as_ref().and_then(value_to_i64).or_else(|| {
        clip_start_ms
            .zip(clip_duration)
            .and_then(|(start_ms, duration_ms)| start_ms.checked_add(duration_ms))
    });
    let events = raw
        .clip_events
        .into_iter()
        .map(|event| normalize_event(openid, clip_start_ms, round_id, event))
        .collect::<Result<_, _>>()?;

    Ok(WonderfulSegmentRecord {
        segment_id: value_to_string(&raw.segment_id).unwrap_or_default(),
        round_id,
        clip_start_ms,
        clip_end_ms,
        events,
    })
}

fn normalize_event(
    openid: &str,
    clip_start_ms: Option<i64>,
    segment_round_id: Option<i64>,
    value: Value,
) -> Result<WonderfulEventRecord, WonderfulDbError> {
    let raw_json =
        serde_json::to_string(&value).map_err(|source| WonderfulDbError::InvalidJson {
            account: openid.to_owned(),
            source,
        })?;
    let raw: RawEventRecord =
        serde_json::from_value(value).map_err(|source| WonderfulDbError::InvalidJson {
            account: openid.to_owned(),
            source,
        })?;
    let event_start_ms = raw.event_start_ms.as_ref().and_then(value_to_i64);
    let raw_round_id = raw.round_id.as_ref().and_then(value_to_i64);
    let video_time_ms = clip_start_ms
        .zip(event_start_ms)
        .and_then(|(clip_start, event_start)| clip_start.checked_add(event_start));
    let event_ext = raw.event_ext.as_ref();

    Ok(WonderfulEventRecord {
        event_id: value_to_string(&raw.event_id).unwrap_or_default(),
        event_type: value_to_string(&raw.event_type).unwrap_or_default(),
        video_time_ms,
        event_time: event_ext
            .and_then(|ext| ext.event_time.as_ref())
            .and_then(nonempty_value_to_string)
            .or_else(|| raw.event_time.as_ref().and_then(nonempty_value_to_string)),
        round_id: event_ext
            .and_then(|ext| ext.round_id.as_ref())
            .and_then(value_to_i64)
            .or(raw_round_id)
            .or(segment_round_id),
        player_name: event_ext
            .and_then(|ext| ext.player_name.as_ref())
            .and_then(nonempty_value_to_string)
            .or_else(|| raw.player_name.as_ref().and_then(nonempty_value_to_string)),
        agent_name: event_ext
            .and_then(|ext| ext.agent_name.as_ref())
            .and_then(nonempty_value_to_string)
            .or_else(|| raw.agent_name.as_ref().and_then(nonempty_value_to_string)),
        weapon_name: event_ext
            .and_then(|ext| {
                ext.weapon_id
                    .as_ref()
                    .and_then(normalize_weapon_name)
                    .or_else(|| {
                        ext.weapon_skin_name
                            .as_ref()
                            .and_then(normalize_weapon_name)
                    })
            })
            .or_else(|| raw.weapon_name.as_ref().and_then(normalize_weapon_name)),
        killer_name: event_ext
            .and_then(|ext| ext.killer_player_name.as_ref())
            .and_then(nonempty_value_to_string)
            .or_else(|| raw.killer_name.as_ref().and_then(nonempty_value_to_string)),
        killed_name: event_ext
            .and_then(|ext| ext.killed_player_name.as_ref())
            .and_then(nonempty_value_to_string)
            .or_else(|| raw.killed_name.as_ref().and_then(nonempty_value_to_string)),
        killer_is_me: event_ext
            .and_then(|ext| ext.killer_is_me.as_ref())
            .map(killer_is_me_from_value)
            .unwrap_or(raw.killer_is_me),
        raw_json,
    })
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn nonempty_value_to_string(value: &Value) -> Option<String> {
    value_to_string(value).and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(value) => value
            .as_i64()
            .or_else(|| value.as_f64().and_then(integral_f64_to_i64)),
        Value::String(value) => value.trim().parse().ok().or_else(|| {
            value
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(integral_f64_to_i64)
        }),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

fn value_to_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => value.as_i64().map(|value| value != 0),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn normalize_weapon_name(value: &Value) -> Option<String> {
    value_to_string(value).and_then(|value| display_names::weapon_name_for_display(&value))
}

fn integral_f64_to_i64(value: f64) -> Option<i64> {
    (value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64)
        .then_some(value as i64)
}

fn killer_is_me_from_value(value: &Value) -> bool {
    matches!(value, Value::Number(value) if value.as_i64() == Some(1))
}
