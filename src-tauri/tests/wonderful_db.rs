use aes::Aes256;
use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use valorant_highlight_manager_lib::wonderful_db::{
    decrypt_wonderful_db_text, parse_wonderful_db_text, parse_wonderful_snapshot_text,
    read_wonderful_db_dir,
};

type Aes256CbcEnc = cbc::Encryptor<Aes256>;

#[test]
fn decrypts_aclos_aes_256_cbc_known_vector() {
    let ciphertext = "1bef612e667f7841956e8a82b030f1fdcf43b773e4c60361f37f7d769dc3b23d";
    let plaintext = decrypt_wonderful_db_text("1001", ciphertext).expect("vector should decrypt");
    assert_eq!(plaintext, r#"{"key_wonderful_list_1001":[]}"#);
}

const TOLERANT_RECORD_FIXTURE: &str = r#"
{
  "key_wonderful_list_1001": [
    {
      "match_id": "match-1",
      "unknown_match_field": {"future": true},
      "videos": [
        {
          "video_id": "video-1",
          "video_name": "六杀时刻",
          "video_type": "五杀时刻",
          "unknown_video_field": 42,
          "round_clips": [
            {
              "round_id": 7,
              "clip_sTime": 1200,
              "clip_events": [
                {
                  "event_id": "event-1",
                  "event_type": "kill",
                  "event_sTime": 340,
                  "killer_name": "synthetic-killer",
                  "killed_name": "synthetic-target",
                  "killer_is_me": true,
                  "unknown_event_field": "ignored"
                }
              ]
            }
          ]
        }
      ]
    }
  ],
  "unknown_root_field": "ignored"
}
"#;

#[test]
fn keeps_video_name_separate_from_video_type() {
    let matches = parse_wonderful_db_text("1001", TOLERANT_RECORD_FIXTURE)
        .expect("synthetic record should parse");

    assert_eq!(matches[0].videos[0].video_name, "六杀时刻");
    assert_eq!(matches[0].videos[0].video_type, "五杀时刻");
}

#[test]
fn parses_recoverable_match_fields_from_current_wonderful_schema() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "matches_id": "match-current-schema",
        "matches_time": "2026-03-21T12:00:00Z",
        "map": {"map_id": "/Game/Maps/Triad/Triad"},
        "agent": {"agent_id": "08"},
        "mode": {"mode_id": "/Game/GameModes/Bomb/competitive"},
        "stats": {
          "kills": 36,
          "deaths": 17,
          "assists": 5,
          "score": 394,
          "rounds_won": 14,
          "rounds_lost": 12,
          "has_won": true
        },
        "career": {
          "battle_id": "battle-current-schema",
          "hero_name": "Sova",
          "hero_image": "https://assets.example/sova.png",
          "map_name": "隐世修所",
          "game_mode": "竞技模式",
          "kda": "36/17/5",
          "rounds_score": "14/12",
          "score": "394",
          "won_match": true
        },
        "videos": []
      }]
    }
    "#;

    let matches =
        parse_wonderful_db_text("1001", fixture).expect("current WonderfulDb schema should parse");
    let record = &matches[0];

    assert_eq!(record.match_id, "match-current-schema");
    assert_eq!(record.battle_id.as_deref(), Some("battle-current-schema"));
    assert_eq!(record.match_time.as_deref(), Some("2026-03-21T12:00:00Z"));
    assert_eq!(record.map_id.as_deref(), Some("/Game/Maps/Triad/Triad"));
    assert_eq!(record.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(record.agent_name.as_deref(), Some("Sova"));
    assert_eq!(
        record.agent_avatar_url.as_deref(),
        Some("https://assets.example/sova.png")
    );
    assert_eq!(record.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(record.kda.as_deref(), Some("36/17/5"));
    assert_eq!(record.scoreline.as_deref(), Some("14/12"));
    assert_eq!(record.combat_score, Some(394));
    assert_eq!(record.has_won, Some(true));
}

#[test]
fn empty_career_scoreline_falls_back_to_round_stats() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "matches_id": "match-empty-career-score",
        "stats": {"rounds_won": 14, "rounds_lost": 12},
        "career": {"rounds_score": ""},
        "videos": []
      }]
    }
    "#;

    let matches =
        parse_wonderful_db_text("1001", fixture).expect("empty career score should still parse");

    assert_eq!(matches[0].scoreline.as_deref(), Some("14/12"));
}

#[test]
fn normalizes_real_millisecond_match_time_to_unix_seconds() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "matches_id": "match-millisecond-time",
        "matches_time": 1772892634798,
        "career": {"time": "2026-03-07 22:10:34"},
        "videos": []
      }]
    }
    "#;

    let matches =
        parse_wonderful_db_text("1001", fixture).expect("millisecond timestamp should parse");

    assert_eq!(matches[0].match_time.as_deref(), Some("1772892634"));
}

#[test]
fn blank_match_time_falls_back_to_career_time() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "matches_id": "match-career-time",
        "matches_time": "  ",
        "career": {"time": "2026-03-07 22:10:34"},
        "videos": []
      }]
    }
    "#;

    let matches = parse_wonderful_db_text("1001", fixture).expect("career timestamp should parse");

    assert_eq!(
        matches[0].match_time.as_deref(),
        Some("2026-03-07 22:10:34")
    );
}

#[test]
fn invalid_primary_aliases_do_not_block_later_valid_aliases() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "matches_id": "match-alias-fallbacks",
        "map": {"map_id": null, "mapId": "/Game/Maps/Plummet/Plummet"},
        "stats": {
          "rounds_won": " ",
          "roundsWon": "13",
          "rounds_lost": null,
          "roundsLost": 9
        },
        "career": {
          "battle_id": " ",
          "battleId": "battle-fallback",
          "rounds_score": "",
          "roundsScore": "13/9",
          "score": "invalid",
          "combat_score": "321",
          "won_match": "unknown",
          "hasWon": true,
          "time": "",
          "matchTime": "2026-03-07 22:10:34"
        },
        "videos": []
      }]
    }
    "#;

    let matches = parse_wonderful_db_text("1001", fixture).expect("aliases should parse");
    let record = &matches[0];

    assert_eq!(record.battle_id.as_deref(), Some("battle-fallback"));
    assert_eq!(record.map_id.as_deref(), Some("/Game/Maps/Plummet/Plummet"));
    assert_eq!(record.map_name.as_deref(), Some("天枢云阙"));
    assert_eq!(record.scoreline.as_deref(), Some("13/9"));
    assert_eq!(record.combat_score, Some(321));
    assert_eq!(record.has_won, Some(true));
    assert_eq!(record.match_time.as_deref(), Some("2026-03-07 22:10:34"));
}

#[test]
fn blank_event_ext_strings_fall_back_to_legacy_event_fields() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "videos": [{
          "round_clips": [{
            "clip_events": [{
              "event_time": "legacy-time",
              "player_name": "legacy-player",
              "agent_name": "legacy-agent",
              "killer_name": "legacy-killer",
              "killed_name": "legacy-target",
              "event_ext": {
                "EventTime": " ",
                "PlayerName": "",
                "AgentName": null,
                "KillerPlayerName": " ",
                "KilledPlayerName": ""
              }
            }]
          }]
        }]
      }]
    }
    "#;

    let matches = parse_wonderful_db_text("1001", fixture).expect("event aliases should parse");
    let event = &matches[0].videos[0].segments[0].events[0];

    assert_eq!(event.event_time.as_deref(), Some("legacy-time"));
    assert_eq!(event.player_name.as_deref(), Some("legacy-player"));
    assert_eq!(event.agent_name.as_deref(), Some("legacy-agent"));
    assert_eq!(event.killer_name.as_deref(), Some("legacy-killer"));
    assert_eq!(event.killed_name.as_deref(), Some("legacy-target"));
}

#[test]
fn ignores_unknown_fields_and_calculates_video_time() {
    let matches = parse_wonderful_db_text("1001", TOLERANT_RECORD_FIXTURE)
        .expect("synthetic record should parse");
    let event = &matches[0].videos[0].segments[0].events[0];

    assert_eq!(event.video_time_ms, Some(1540));
    assert_eq!(event.round_id, Some(7));
    assert_eq!(event.killer_name.as_deref(), Some("synthetic-killer"));
    assert!(event.killer_is_me);
}

#[test]
fn normalizes_official_event_ext_fields_with_priority_over_legacy_top_level_fields() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "videos": [{
          "round_clips": [{
            "round_id": 3,
            "clip_events": [{
              "event_id": "event-official",
              "event_type": "kill",
              "event_time": "legacy-time",
              "round_id": 4,
              "player_name": "legacy-player",
              "agent_name": "legacy-agent",
              "weapon_name": "legacy-weapon",
              "killer_name": "legacy-killer",
              "killed_name": "legacy-target",
              "killer_is_me": false,
              "event_ext": {
                "EventTime": "official-time",
                "PlayerName": "official-player",
                "AgentName": "official-agent",
                "RoundID": 9,
                "WeaponID": "/Game/Maps/Pitt/Pitt.Pitt:PersistentLevel.AssaultRifle_AK_C_2147000127",
                "WeaponSkinName": "AK_Ashen_PrimaryAsset_C /Game/Equippables/Guns/Rifles/AK/Ashen/AK_Ashen_PrimaryAsset.Default__AK_Ashen_PrimaryAsset_C",
                "KillerPlayerName": "official-killer",
                "KilledPlayerName": "official-target",
                "KillerIsMe": 1
              }
            }]
          }]
        }]
      }]
    }
    "#;
    let matches =
        parse_wonderful_db_text("1001", fixture).expect("official event_ext should parse");
    let event = &matches[0].videos[0].segments[0].events[0];

    assert_eq!(event.event_time.as_deref(), Some("official-time"));
    assert_eq!(event.player_name.as_deref(), Some("official-player"));
    assert_eq!(event.agent_name.as_deref(), Some("official-agent"));
    assert_eq!(event.round_id, Some(9));
    assert_eq!(event.weapon_name.as_deref(), Some("狂徒"));
    assert_eq!(event.killer_name.as_deref(), Some("official-killer"));
    assert_eq!(event.killed_name.as_deref(), Some("official-target"));
    assert!(event.killer_is_me);
}

#[test]
fn treats_only_numeric_one_as_official_event_ext_killer_is_me() {
    for (killer_is_me, expected) in [(1, true), (2, false), (-1, false)] {
        let fixture = format!(
            r#"{{
              "key_wonderful_list_1001": [{{
                "videos": [{{
                  "round_clips": [{{
                    "clip_events": [{{
                      "killer_is_me": true,
                      "event_ext": {{"KillerIsMe": {killer_is_me}}}
                    }}]
                  }}]
                }}]
              }}]
            }}"#
        );
        let matches = parse_wonderful_db_text("1001", &fixture)
            .expect("official KillerIsMe boundary fixture should parse");
        let event = &matches[0].videos[0].segments[0].events[0];

        assert_eq!(event.killer_is_me, expected, "KillerIsMe={killer_is_me}");
    }
}

#[test]
fn falls_back_to_top_level_killer_is_me_when_event_ext_omits_the_field() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "videos": [{
          "round_clips": [{
            "clip_events": [{
              "killerIsMe": true,
              "event_ext": {"PlayerName": "official-player"}
            }]
          }]
        }]
      }]
    }
    "#;

    let matches = parse_wonderful_db_text("1001", fixture).expect("fixture should parse");
    assert!(matches[0].videos[0].segments[0].events[0].killer_is_me);
}

#[test]
fn explicit_event_ext_zero_overrides_top_level_killer_is_me() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "videos": [{
          "round_clips": [{
            "clip_events": [{
              "killerIsMe": true,
              "event_ext": {"KillerIsMe": 0}
            }]
          }]
        }]
      }]
    }
    "#;

    let matches = parse_wonderful_db_text("1001", fixture).expect("fixture should parse");
    assert!(!matches[0].videos[0].segments[0].events[0].killer_is_me);
}

#[test]
fn preserves_official_match_id_and_video_source_fields() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "matches_id": "match-official",
        "videos": [{
          "video_src": "D:\\synthetic\\clip.mp4"
        }]
      }]
    }
    "#;
    let matches = parse_wonderful_db_text("1001", fixture)
        .expect("official match and video fields should parse");

    assert_eq!(matches[0].match_id, "match-official");
    assert_eq!(
        matches[0].videos[0].video_src.as_deref(),
        Some(r"D:\synthetic\clip.mp4")
    );
}

#[test]
fn accepts_match_without_optional_career() {
    let matches = parse_wonderful_db_text("1001", TOLERANT_RECORD_FIXTURE)
        .expect("missing career should be accepted");

    assert_eq!(matches.len(), 1);
    assert!(matches[0].career.is_none());
}

#[test]
fn parses_string_round_score_as_integer() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "videos": [{
          "video_id": "round-score-video",
          "round_score": "13"
        }, {
          "video_id": "numeric-round-score-video",
          "round_score": 11
        }]
      }]
    }
    "#;

    let matches =
        parse_wonderful_db_text("1001", fixture).expect("round score fixture should parse");

    assert_eq!(matches[0].videos[0].round_score, Some(13));
    assert_eq!(matches[0].videos[1].round_score, Some(11));
}

#[test]
fn parses_official_rounds_shape_into_segments_and_events() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "matches_id": "official-rounds-match",
        "videos": [{
          "video_id": "official-six-kill",
          "video_name": "六杀时刻",
          "video_type": "高光时刻",
          "highLightType": 10,
          "round_score": "13",
          "rounds": [
            {
              "round_id": 1,
              "round_clips": [{
                "clip_id": "segment-1",
                "clip_sTime": 0,
                "clip_duration": 1000,
                "clip_events": [
                  {"event_id":"event-1","event_type":"kill","event_sTime":100,"event_ext":{"KillerIsMe":1}},
                  {"event_id":"event-2","event_type":"kill","event_sTime":200,"event_ext":{"KillerIsMe":1}}
                ]
              }]
            },
            {
              "round_id": 2,
              "round_clips": [{
                "clip_id": "segment-2",
                "clip_sTime": 1000,
                "clip_duration": 1000,
                "clip_events": [
                  {"event_id":"event-3","event_type":"kill","event_sTime":100,"event_ext":{"KillerIsMe":1}},
                  {"event_id":"event-4","event_type":"kill","event_sTime":200,"event_ext":{"KillerIsMe":1}}
                ]
              }]
            },
            {
              "round_id": 3,
              "round_clips": [{
                "clip_id": "segment-3",
                "clip_sTime": 2000,
                "clip_duration": 1000,
                "clip_events": [
                  {"event_id":"event-5","event_type":"kill","event_sTime":100,"event_ext":{"KillerIsMe":1}},
                  {"event_id":"event-6","event_type":"kill","event_sTime":200,"event_ext":{"KillerIsMe":1}}
                ]
              }]
            }
          ]
        }]
      }]
    }
    "#;

    let matches =
        parse_wonderful_db_text("1001", fixture).expect("official rounds fixture should parse");
    let video = &matches[0].videos[0];
    let events = video
        .segments
        .iter()
        .flat_map(|segment| &segment.events)
        .collect::<Vec<_>>();

    assert_eq!(video.segments.len(), 3);
    assert_eq!(events.len(), 6);
    assert!(events.iter().all(|event| event.killer_is_me));
    assert_eq!(video.highlight_type, Some(10));
}

#[test]
fn parses_snapshot_records_without_mixing_them_into_video_records() {
    let fixture = r#"
    {
      "key_snapshot_list1001": [{
        "matches_id": "snapshot-match",
        "matches_time": 1772892634798,
        "map": {"map_id": "/Game/Maps/Plummet/Plummet"},
        "agent": {"agent_name": "Neon"},
        "stats": {
          "kills": 22,
          "deaths": 16,
          "assists": 1,
          "rounds_won": 13,
          "rounds_lost": 9,
          "score": 321,
          "has_won": true,
          "mode_name": "/Game/GameModes/Bomb/competitive"
        },
        "snapshot": {
          "ss_id": "snapshot-1",
          "ss_time": 1772892627947,
          "ss_package_src": "D:\\snapshot\\package.jpeg",
          "ss_thumb_src": "D:\\snapshot\\thumb.jpeg",
          "ss_width": "1920",
          "ss_height": 1080,
          "ss_size": 12345,
          "ss_nick": "FixtureBravo",
          "ss_nick_id": "0002"
        }
      }]
    }
    "#;

    let snapshots = parse_wonderful_snapshot_text("1001", fixture).expect("snapshot should parse");
    let snapshot = &snapshots[0];

    assert_eq!(snapshot.snapshot_id, "snapshot-1");
    assert_eq!(snapshot.captured_at.as_deref(), Some("1772892627"));
    assert_eq!(snapshot.account_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(snapshot.width, Some(1920));
    assert_eq!(snapshot.height, Some(1080));
    assert_eq!(snapshot.size_bytes, Some(12345));
    assert_eq!(snapshot.match_record.match_id, "snapshot-match");
    assert_eq!(snapshot.match_record.map_name.as_deref(), Some("天枢云阙"));
    assert_eq!(snapshot.match_record.kda.as_deref(), Some("22/16/1"));
    assert_eq!(snapshot.match_record.scoreline.as_deref(), Some("13/9"));
    assert!(snapshot.match_record.videos.is_empty());
}

#[test]
fn tolerates_mixed_numeric_shapes_in_official_rounds() {
    let fixture = r#"
    {
      "key_wonderful_list_1001": [{
        "matches_id": "mixed-number-match",
        "videos": [{
          "video_id": "mixed-number-video",
          "highLightType": "10.0",
          "rounds": [{
            "round_id": "1",
            "round_clips": [{
              "clip_id": "mixed-segment",
              "clip_sTime": "1000",
              "clip_duration": 500.0,
              "clip_events": [{
                "event_id": "mixed-event",
                "event_type": "kill",
                "event_sTime": "250.0",
                "round_id": 2.0,
                "event_ext": {"KillerIsMe": 1}
              }]
            }]
          }]
        }]
      }]
    }
    "#;

    let matches =
        parse_wonderful_db_text("1001", fixture).expect("mixed numeric fixture should parse");
    let video = &matches[0].videos[0];
    let segment = &video.segments[0];
    let event = &segment.events[0];

    assert_eq!(video.highlight_type, Some(10));
    assert_eq!(segment.round_id, Some(1));
    assert_eq!(segment.clip_start_ms, Some(1_000));
    assert_eq!(segment.clip_end_ms, Some(1_500));
    assert_eq!(event.video_time_ms, Some(1_250));
    assert_eq!(event.round_id, Some(2));
    assert!(event.killer_is_me);
}

#[test]
fn isolates_corrupt_accounts_and_ignores_non_numeric_files() {
    let temp = SyntheticTempDir::new();
    fs::write(
        temp.path().join("1001"),
        encrypt_wonderful_db_text("1001", TOLERANT_RECORD_FIXTURE),
    )
    .expect("synthetic valid account file should be written");
    fs::write(temp.path().join("1002"), "not-hex")
        .expect("synthetic corrupt account file should be written");
    fs::write(temp.path().join("snapshot-notnumeric"), "ignored")
        .expect("synthetic snapshot file should be written");

    let result = read_wonderful_db_dir(temp.path());

    assert_eq!(result.accounts.len(), 1);
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(result.accounts[0].openid, "1001");
}

fn encrypt_wonderful_db_text(openid: &str, plaintext: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(openid.as_bytes()));
    let key = &digest.as_bytes()[..32];
    let iv = &digest.as_bytes()[..16];
    let ciphertext = Aes256CbcEnc::new_from_slices(key, iv)
        .expect("synthetic key material should be valid")
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext.as_bytes());
    hex::encode(ciphertext)
}

struct SyntheticTempDir {
    path: PathBuf,
}

impl SyntheticTempDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "valorant-highlight-manager-wonderful-db-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("synthetic temporary directory should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SyntheticTempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("synthetic temporary directory should be removed");
    }
}
