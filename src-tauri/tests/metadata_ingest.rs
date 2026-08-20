// Privacy note: account IDs, player names, match IDs, and paths in this file are synthetic fixtures.
use std::collections::HashSet;

use rusqlite::{params, Connection};

use valorant_highlight_manager_lib::{
    db::{self, ClipGroupInput, ClipInput, SourceDirInput},
    highlight_log_parser::{HighlightLogKillEvent, HighlightLogLineKind, HighlightLogRecord},
    leveldb_reader::LevelDbBattleRecord,
    metadata_ingest::{ingest_match_metadata, MetadataIngestInput},
};

#[test]
fn merges_leveldb_and_log_data_into_matches_and_clip_metadata() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-a-001", "ace.mp4");

    let leveldb_battles = vec![LevelDbBattleRecord {
        account_id: "1001".to_string(),
        battle_id: Some("battle-a-001".to_string()),
        match_id: Some("match-a-001".to_string()),
        player_name: None,
        kda: Some("18/7/3".to_string()),
        match_date: Some("2026-07-01T10:00:00Z".to_string()),
        agent_avatar_url: Some("https://assets.example/jett.png".to_string()),
        raw_json: r#"{"matchId":"match-a-001"}"#.to_string(),
    }];
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::FirstRequestData,
        match_id: Some("match-a-001".to_string()),
        battle_id: Some("battle-a-001".to_string()),
        record_src: Some("D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-a-001".to_string()),
        player_name: Some("PlayerOne#0000".to_string()),
        map_id: Some("maps/ascent".to_string()),
        map_name: Some("Ascent".to_string()),
        game_mode: Some("Competitive".to_string()),
        agent_name: Some("Jett".to_string()),
        kda: Some("1/1/1".to_string()),
        scoreline: Some("13/11".to_string()),
        has_won: Some(true),
        combat_score: Some(287),
        kill_events: vec![HighlightLogKillEvent {
            event_time: Some("2026-07-01T10:00:31Z".to_string()),
            round_id: Some(3),
            weapon_name: Some("Vandal".to_string()),
            killer_name: Some("PlayerOne#0000".to_string()),
            killed_name: Some("Opponent#0000".to_string()),
            raw_json: Some(r#"{"weaponName":"Vandal"}"#.to_string()),
        }],
        has_gzip_event: false,
        raw_json: r#"{"matchId":"match-a-001"}"#.to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &leveldb_battles,
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.matches_upserted, 1);
    assert_eq!(summary.stats_upserted, 1);
    assert_eq!(summary.events_inserted, 1);
    assert_eq!(summary.enriched_clip_count, 1);
    assert_eq!(summary.unmatched_match_count, 0);

    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.player_name.as_deref(), Some("PlayerOne#0000"));
    assert_eq!(clip.agent_name.as_deref(), Some("Jett"));
    assert_eq!(clip.map_name.as_deref(), Some("亚海悬城"));
    assert_eq!(clip.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(clip.scoreline.as_deref(), Some("13/11"));
    assert_eq!(clip.kda.as_deref(), Some("1/1/1"));
    assert_eq!(clip.recorded_at, None);

    let metadata: (String, String, Option<i64>, Option<String>, Option<String>) = connection
        .query_row(
            "
            SELECT metadata_status, match_id, kill_count, weapon_name, round_label
            FROM clip_metadata
            WHERE clip_id = ?1
            ",
            params![clip_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("clip metadata should be readable");
    assert_eq!(
        metadata,
        (
            "enriched".to_string(),
            "match-a-001".to_string(),
            None,
            None,
            None
        )
    );

    let stored_match: (String, String, String, String, String, i64, i64) = connection
        .query_row(
            "
            SELECT game_id, battle_id, account_id, player_name, agent_avatar_url, source_leveldb, source_log
            FROM matches
            WHERE game_id = 'match-a-001'
            ",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("match should be readable");
    assert_eq!(
        stored_match,
        (
            "match-a-001".to_string(),
            "battle-a-001".to_string(),
            "1001".to_string(),
            "PlayerOne#0000".to_string(),
            "https://assets.example/jett.png".to_string(),
            1,
            1
        )
    );

    let stats: (i64, i64, i64, i64, i64, i64, i64) = connection
        .query_row(
            "
            SELECT kills, deaths, assists, combat_score, rounds_won, rounds_lost, has_won
            FROM match_stats
            JOIN matches ON matches.id = match_stats.match_id
            WHERE matches.game_id = 'match-a-001'
            ",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("stats should be readable");
    assert_eq!(stats, (1, 1, 1, 287, 13, 11, 1));
}

#[test]
fn match_events_do_not_derive_clip_timeline_metadata_or_tags() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-a", "clip.mp4");
    let log_records = vec![log_record_with_kill_events("match-a", 26)];

    ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    let tags = clip_tag_names(&connection, &clip);

    assert_eq!(clip.kill_count, None);
    assert_eq!(clip.recorded_at, None);
    assert!(!tags.contains("三杀"));
    assert!(!tags.contains("四杀"));
    assert_eq!(list_match_events_for_test(&connection, "match-a").len(), 26);
}

#[test]
fn fallback_fills_missing_match_fields_without_replacing_wonderful_values() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-official", "clip.mp4");
    db::upsert_clip_metadata(
        &connection,
        db::ClipMetadataInput {
            clip_id,
            metadata_status: "enriched",
            json_path: None,
            account_name: None,
            player_name: None,
            agent_name: None,
            map_name: Some("隐世修所"),
            game_mode: None,
            scoreline: None,
            kda: None,
            extracted_text: None,
            parse_error: None,
        },
    )
    .expect("official metadata should seed");
    connection
        .execute(
            "
            UPDATE clip_metadata
            SET match_id = 'match-official',
                kill_count = 6,
                metadata_source = 'wonderful_db'
            WHERE clip_id = ?1
            ",
            params![clip_id],
        )
        .expect("official source should seed");
    let mut log_record = log_record_with_kill_events("match-official", 3);
    log_record.map_name = Some("Ascent".to_string());
    log_record.kda = Some("18/7/3".to_string());

    ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &[log_record],
        },
    )
    .expect("fallback ingest should run");

    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(clip.kda.as_deref(), Some("18/7/3"));
    assert_eq!(clip.kill_count, Some(6));
}

#[test]
fn export_fields_are_not_replaced_by_log_or_leveldb_metadata() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-export", "clip.mp4");
    db::upsert_clip_metadata(
        &connection,
        db::ClipMetadataInput {
            clip_id,
            metadata_status: "parsed",
            json_path: Some("D:/exports/config-export.json"),
            account_name: Some("ExportPlayer#1001"),
            player_name: Some("ExportPlayer#1001"),
            agent_name: Some("ExportAgent"),
            map_name: Some("ExportMap"),
            game_mode: Some("ExportMode"),
            scoreline: Some("13/2"),
            kda: Some("30/2/7"),
            extracted_text: Some("export payload"),
            parse_error: None,
        },
    )
    .expect("export metadata should seed");
    let mut log_record = log_record_with_kill_events("match-export", 1);
    log_record.player_name = Some("LogPlayer#9999".to_string());
    log_record.agent_name = Some("Jett".to_string());
    log_record.map_name = Some("Ascent".to_string());
    log_record.game_mode = Some("Competitive".to_string());
    log_record.scoreline = Some("1/13".to_string());
    log_record.kda = Some("1/13/0".to_string());
    let leveldb_battles = vec![LevelDbBattleRecord {
        account_id: "1001".to_string(),
        battle_id: None,
        match_id: Some("match-export".to_string()),
        player_name: Some("LevelDbPlayer#8888".to_string()),
        kda: Some("2/12/1".to_string()),
        match_date: None,
        agent_avatar_url: None,
        raw_json: "{}".to_string(),
    }];

    ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &leveldb_battles,
            log_records: &[log_record],
        },
    )
    .expect("lower-priority metadata ingest should run");

    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.player_name.as_deref(), Some("ExportPlayer#1001"));
    assert_eq!(clip.agent_name.as_deref(), Some("ExportAgent"));
    assert_eq!(clip.map_name.as_deref(), Some("ExportMap"));
    assert_eq!(clip.game_mode.as_deref(), Some("ExportMode"));
    assert_eq!(clip.scoreline.as_deref(), Some("13/2"));
    assert_eq!(clip.kda.as_deref(), Some("30/2/7"));
    assert_eq!(clip.metadata_source.as_deref(), Some("video_export"));
}

#[test]
fn lower_priority_metadata_fills_fields_missing_from_export() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-export-partial", "clip.mp4");
    db::upsert_clip_metadata(
        &connection,
        db::ClipMetadataInput {
            clip_id,
            metadata_status: "partial",
            json_path: Some("D:/exports/config-partial.json"),
            account_name: Some("ExportPlayer#1001"),
            player_name: Some("ExportPlayer#1001"),
            agent_name: None,
            map_name: None,
            game_mode: None,
            scoreline: None,
            kda: None,
            extracted_text: Some("partial export payload"),
            parse_error: None,
        },
    )
    .expect("partial export metadata should seed");
    let mut log_record = log_record_with_kill_events("match-export-partial", 1);
    log_record.agent_name = Some("Jett".to_string());
    log_record.map_name = Some("Ascent".to_string());
    log_record.game_mode = Some("Competitive".to_string());
    log_record.scoreline = Some("13/11".to_string());
    log_record.kda = Some("1/1/1".to_string());
    let leveldb_battles = vec![LevelDbBattleRecord {
        account_id: "1001".to_string(),
        battle_id: None,
        match_id: Some("match-export-partial".to_string()),
        player_name: None,
        kda: Some("18/7/3".to_string()),
        match_date: None,
        agent_avatar_url: None,
        raw_json: "{}".to_string(),
    }];

    ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &leveldb_battles,
            log_records: &[log_record],
        },
    )
    .expect("fallback metadata ingest should run");

    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.player_name.as_deref(), Some("ExportPlayer#1001"));
    assert_eq!(clip.agent_name.as_deref(), Some("Jett"));
    assert_eq!(clip.map_name.as_deref(), Some("亚海悬城"));
    assert_eq!(clip.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(clip.scoreline.as_deref(), Some("13/11"));
    assert_eq!(clip.kda.as_deref(), Some("1/1/1"));
}

#[test]
fn round_zero_match_events_are_not_stored_or_counted() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-round-zero", "clip.mp4");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::EventParser,
        match_id: Some("match-round-zero".to_string()),
        battle_id: None,
        record_src: Some(
            "D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-round-zero/clip.mp4".to_string(),
        ),
        player_name: Some("PlayerOne#0000".to_string()),
        map_id: None,
        map_name: Some("Ascent".to_string()),
        game_mode: Some("Competitive".to_string()),
        agent_name: Some("Jett".to_string()),
        kda: None,
        scoreline: None,
        has_won: None,
        combat_score: None,
        kill_events: vec![
            HighlightLogKillEvent {
                event_time: Some("2026-07-03T12:11:00Z".to_string()),
                round_id: Some(0),
                weapon_name: Some("BasePistol".to_string()),
                killer_name: None,
                killed_name: None,
                raw_json: Some(r#"{"EventName":"Shot","RoundID":0}"#.to_string()),
            },
            HighlightLogKillEvent {
                event_time: Some("2026-07-03T12:12:00Z".to_string()),
                round_id: Some(3),
                weapon_name: Some("Vandal".to_string()),
                killer_name: Some("PlayerOne#0000".to_string()),
                killed_name: Some("Opponent#0000".to_string()),
                raw_json: Some(r#"{"EventName":"Kill","RoundID":3}"#.to_string()),
            },
        ],
        has_gzip_event: true,
        raw_json: "{}".to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.events_inserted, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.kill_count, None);
    assert_eq!(clip.event_count, 0);
    assert!(clip.clip_events.is_empty());

    let stored_match_events: (i64, i64, Option<i64>) = connection
        .query_row(
            "
            SELECT
                COUNT(*),
                COALESCE(SUM(CASE WHEN match_events.round_id = 0 THEN 1 ELSE 0 END), 0),
                MIN(match_events.round_id)
            FROM match_events
            JOIN matches ON matches.id = match_events.match_id
            WHERE matches.game_id = 'match-round-zero'
            ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("stored match events should be readable directly");
    assert_eq!(stored_match_events, (1, 0, Some(3)));
}

#[test]
fn leveldb_only_match_is_stored_without_touching_unmatched_clips() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let unrelated_clip_id =
        insert_clip_for_group(&connection, "1001", "unrelated-match", "clip.mp4");
    let leveldb_battles = vec![LevelDbBattleRecord {
        account_id: "1001".to_string(),
        battle_id: Some("battle-only".to_string()),
        match_id: Some("match-only".to_string()),
        player_name: None,
        kda: Some("4/2/1".to_string()),
        match_date: Some("2026-07-02T11:00:00Z".to_string()),
        agent_avatar_url: Some("https://assets.example/phoenix.png".to_string()),
        raw_json: r#"{"matchId":"match-only"}"#.to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &leveldb_battles,
            log_records: &[],
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.matches_upserted, 1);
    assert_eq!(summary.stats_upserted, 1);
    assert_eq!(summary.events_inserted, 0);
    assert_eq!(summary.enriched_clip_count, 0);
    assert_eq!(summary.unmatched_match_count, 1);

    let metadata_status: String = connection
        .query_row(
            "SELECT metadata_status FROM clip_metadata WHERE clip_id = ?1",
            params![unrelated_clip_id],
            |row| row.get(0),
        )
        .expect("clip metadata should be readable");
    assert_eq!(metadata_status, "not_found");

    let stored_match: (String, String, i64, i64) = connection
        .query_row(
            "
            SELECT game_id, account_id, source_leveldb, source_log
            FROM matches
            WHERE game_id = 'match-only'
            ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("match should be readable");
    assert_eq!(
        stored_match,
        ("match-only".to_string(), "1001".to_string(), 1, 0)
    );
}

#[test]
fn leveldb_player_name_enriches_matching_clip_account_metadata() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-player", "clip.mp4");
    let leveldb_battles = vec![LevelDbBattleRecord {
        account_id: "1001".to_string(),
        battle_id: Some("battle-player".to_string()),
        match_id: Some("match-player".to_string()),
        player_name: Some("FixtureAlpha#0001".to_string()),
        kda: Some("24/13/7".to_string()),
        match_date: Some("2026-07-04T12:00:00Z".to_string()),
        agent_avatar_url: None,
        raw_json: r#"{"matchId":"match-player","GameName":"FixtureAlpha","TagLine":"0001"}"#
            .to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &leveldb_battles,
            log_records: &[],
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);

    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.account_name.as_deref(), Some("FixtureAlpha#0001"));
    assert_eq!(clip.player_name.as_deref(), Some("FixtureAlpha#0001"));
    assert_eq!(clip.kda.as_deref(), Some("24/13/7"));
}

#[test]
fn leveldb_headico_avatar_enriches_matching_clip_agent_name() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-headico", "clip.mp4");
    let leveldb_battles = vec![LevelDbBattleRecord {
        account_id: "1001".to_string(),
        battle_id: None,
        match_id: Some("match-headico".to_string()),
        player_name: None,
        kda: Some("22/13/2".to_string()),
        match_date: None,
        agent_avatar_url: Some(
            "https://game.gtimg.cn/images/actdaoju/act/a20230301valorant/headico/02.png"
                .to_string(),
        ),
        raw_json: r#"{"matchId":"match-headico"}"#.to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &leveldb_battles,
            log_records: &[],
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.agent_name.as_deref(), Some("Jett"));
    assert_eq!(clip.kda.as_deref(), Some("22/13/2"));
}

#[test]
fn match_metadata_requires_source_account_when_group_key_exists_in_multiple_sources() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let matching_clip_id = insert_clip_for_group(&connection, "1001", "shared-match", "ace.mp4");
    let other_clip_id = insert_clip_for_group(&connection, "2002", "shared-match", "ace.mp4");
    let leveldb_battles = vec![LevelDbBattleRecord {
        account_id: "1001".to_string(),
        battle_id: None,
        match_id: Some("shared-match".to_string()),
        player_name: Some("PlayerOne#0000".to_string()),
        kda: Some("24/13/7".to_string()),
        match_date: Some("2026-07-04T12:00:00Z".to_string()),
        agent_avatar_url: None,
        raw_json: r#"{"matchId":"shared-match"}"#.to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &leveldb_battles,
            log_records: &[],
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let matching_clip =
        db::find_clip_by_id(&connection, matching_clip_id).expect("matching clip should reload");
    let other_clip =
        db::find_clip_by_id(&connection, other_clip_id).expect("other clip should reload");
    assert_eq!(
        matching_clip.account_name.as_deref(),
        Some("PlayerOne#0000")
    );
    assert_eq!(matching_clip.kda.as_deref(), Some("24/13/7"));
    assert_eq!(other_clip.metadata_status, "not_found");
    assert_eq!(other_clip.account_name, None);
    assert_eq!(other_clip.kda, None);
}

#[test]
fn unique_source_group_corrects_stale_match_account_on_rescan() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id =
        insert_clip_for_group(&connection, "90000000000000001", "match-stale", "clip.mp4");
    connection
        .execute(
            "UPDATE source_dirs SET name = '朋友的素材库' WHERE id = (SELECT source_dir_id FROM clips WHERE id = ?1)",
            params![clip_id],
        )
        .expect("source display name should be customized");
    connection
        .execute(
            "
            INSERT INTO matches (
                game_id,
                account_id,
                player_name,
                agent_name,
                map_name,
                game_mode,
                source_log
            )
            VALUES (
                'match-stale',
                '9000000000000000002',
                'Wrong#0000',
                'Jett',
                '隐世修所',
                '竞技模式',
                1
            )
            ",
            [],
        )
        .expect("stale match should seed");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::BattleListResponse,
        match_id: Some("match-stale".to_string()),
        battle_id: Some("battle-stale".to_string()),
        record_src: None,
        player_name: Some("FixtureAlpha#0001".to_string()),
        map_id: None,
        map_name: Some("日落之城".to_string()),
        game_mode: Some("竞技模式".to_string()),
        agent_name: Some("猎枭".to_string()),
        kda: Some("14/16/5".to_string()),
        scoreline: Some("11/13".to_string()),
        has_won: Some(false),
        combat_score: Some(157),
        kill_events: Vec::new(),
        has_gzip_event: false,
        raw_json: r#"{"match_id":"match-stale"}"#.to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.match_account_id.as_deref(), Some("90000000000000001"));
    assert_eq!(clip.account_name.as_deref(), Some("FixtureAlpha#0001"));
    assert_eq!(clip.agent_name.as_deref(), Some("猎枭"));
    assert_eq!(clip.map_name.as_deref(), Some("日落之城"));
    assert_eq!(clip.kda.as_deref(), Some("14/16/5"));
}

#[test]
fn record_src_account_log_record_overrides_prior_ambiguous_log_record() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(
        &connection,
        "9000000000000000002",
        "44444444-4444-4444-8444-444444444402",
        "clip.mp4",
    );
    let log_records = vec![
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::BattleListResponse,
            match_id: Some("44444444-4444-4444-8444-444444444402".to_string()),
            battle_id: Some("44444444-4444-4444-8444-444444444401".to_string()),
            record_src: None,
            player_name: Some("FixtureAlpha#0001".to_string()),
            map_id: None,
            map_name: Some("霓虹町".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Phoenix".to_string()),
            kda: Some("16/16/1".to_string()),
            scoreline: Some("9/13".to_string()),
            has_won: Some(false),
            combat_score: Some(208),
            kill_events: Vec::new(),
            has_gzip_event: false,
            raw_json: r#"{"match_id":"44444444-4444-4444-8444-444444444402"}"#.to_string(),
        },
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::TemplateParam,
            match_id: Some("44444444-4444-4444-8444-444444444402".to_string()),
            battle_id: Some("44444444-4444-4444-8444-444444444401".to_string()),
            record_src: Some(
                "C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/44444444-4444-4444-8444-444444444402/clip.mp4"
                    .to_string(),
            ),
            player_name: Some("FixtureBravo#0002".to_string()),
            map_id: None,
            map_name: Some("霓虹町".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("芮娜".to_string()),
            kda: Some("31/15/6".to_string()),
            scoreline: Some("13/10".to_string()),
            has_won: Some(true),
            combat_score: Some(385),
            kill_events: Vec::new(),
            has_gzip_event: false,
            raw_json: r#"{"video_src":"C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/44444444-4444-4444-8444-444444444402/clip.mp4"}"#.to_string(),
        },
    ];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(
        clip.match_account_id.as_deref(),
        Some("9000000000000000002")
    );
    assert_eq!(clip.account_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.player_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.agent_name.as_deref(), Some("芮娜"));
    assert_eq!(clip.map_name.as_deref(), Some("霓虹町"));
    assert_eq!(clip.kda.as_deref(), Some("31/15/6"));

    let stored_match: (String, String, String, String, i64, i64, i64, i64) = connection
        .query_row(
            "
            SELECT
                matches.account_id,
                matches.player_name,
                matches.agent_name,
                matches.map_name,
                match_stats.kills,
                match_stats.deaths,
                match_stats.assists,
                match_stats.combat_score
            FROM matches
            JOIN match_stats ON match_stats.match_id = matches.id
            WHERE matches.game_id = '44444444-4444-4444-8444-444444444402'
            ",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .expect("stored match should be readable");
    assert_eq!(
        stored_match,
        (
            "9000000000000000002".to_string(),
            "FixtureBravo#0002".to_string(),
            "芮娜".to_string(),
            "霓虹町".to_string(),
            31,
            15,
            6,
            385
        )
    );
}

#[test]
fn generic_record_directory_is_not_used_as_match_alias() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "90000000000000001", "match-a", "clip.mp4");
    let log_records = vec![
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::FirstRequestData,
            match_id: Some("match-a".to_string()),
            battle_id: None,
            record_src: Some(
                "C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos90000000000000001/record/20260702-173530.mp4"
                    .to_string(),
            ),
            player_name: Some("FixtureAlpha#0001".to_string()),
            map_id: None,
            map_name: Some("隐世修所".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Sova".to_string()),
            kda: Some("14/16/5".to_string()),
            scoreline: Some("11/13".to_string()),
            has_won: Some(false),
            combat_score: Some(157),
            kill_events: Vec::new(),
            has_gzip_event: false,
            raw_json: r#"{"match_id":"match-a"}"#.to_string(),
        },
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::TemplateParam,
            match_id: None,
            battle_id: None,
            record_src: Some(
                "C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/record/20260701-182826.mp4"
                    .to_string(),
            ),
            player_name: Some("FixtureBravo#0002".to_string()),
            map_id: None,
            map_name: Some("霓虹町".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Reyna".to_string()),
            kda: Some("31/15/6".to_string()),
            scoreline: Some("13/10".to_string()),
            has_won: Some(true),
            combat_score: Some(385),
            kill_events: Vec::new(),
            has_gzip_event: false,
            raw_json: r#"{"recordSrc":"C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/record/20260701-182826.mp4"}"#.to_string(),
        },
    ];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    assert_eq!(summary.matches_upserted, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.match_account_id.as_deref(), Some("90000000000000001"));
    assert_eq!(clip.account_name.as_deref(), Some("FixtureAlpha#0001"));
    assert_eq!(clip.agent_name.as_deref(), Some("Sova"));
    assert_eq!(clip.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(clip.kda.as_deref(), Some("14/16/5"));
}

#[test]
fn conflicting_record_src_account_does_not_override_existing_source_account_match() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "90000000000000001", "match-a", "clip.mp4");
    let log_records = vec![
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::FirstRequestData,
            match_id: Some("match-a".to_string()),
            battle_id: None,
            record_src: Some(
                "C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos90000000000000001/record/20260702-173530.mp4"
                    .to_string(),
            ),
            player_name: Some("FixtureAlpha#0001".to_string()),
            map_id: None,
            map_name: Some("隐世修所".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Sova".to_string()),
            kda: Some("14/16/5".to_string()),
            scoreline: Some("11/13".to_string()),
            has_won: Some(false),
            combat_score: Some(157),
            kill_events: vec![HighlightLogKillEvent {
                event_time: Some("2026-07-02T17:36:37Z".to_string()),
                round_id: Some(0),
                weapon_name: Some("Ghost".to_string()),
                killer_name: Some("FixtureAlpha".to_string()),
                killed_name: Some("Jett".to_string()),
                raw_json: None,
            }],
            has_gzip_event: false,
            raw_json: r#"{"match_id":"match-a"}"#.to_string(),
        },
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::EventParser,
            match_id: Some("match-a".to_string()),
            battle_id: None,
            record_src: Some(
                "C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/record/20260702-211945.mp4"
                    .to_string(),
            ),
            player_name: Some("FixtureBravo#0002".to_string()),
            map_id: None,
            map_name: Some("隐世修所".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Jett".to_string()),
            kda: Some("21/14/1".to_string()),
            scoreline: None,
            has_won: None,
            combat_score: Some(30),
            kill_events: vec![HighlightLogKillEvent {
                event_time: Some("2026-07-02T21:19:45Z".to_string()),
                round_id: Some(4),
                weapon_name: Some("Vandal".to_string()),
                killer_name: Some("FixtureBravo".to_string()),
                killed_name: Some("Sova".to_string()),
                raw_json: None,
            }],
            has_gzip_event: true,
            raw_json: r#"{"match_id":"match-a"}"#.to_string(),
        },
    ];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.match_account_id.as_deref(), Some("90000000000000001"));
    assert_eq!(clip.account_name.as_deref(), Some("FixtureAlpha#0001"));
    assert_eq!(clip.player_name.as_deref(), Some("FixtureAlpha#0001"));
    assert_eq!(clip.agent_name.as_deref(), Some("Sova"));
    assert_eq!(clip.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(clip.kda.as_deref(), Some("14/16/5"));

    let event_count: i64 = connection
        .query_row(
            "
            SELECT COUNT(*)
            FROM match_events
            JOIN matches ON matches.id = match_events.match_id
            WHERE matches.game_id = 'match-a'
            ",
            [],
            |row| row.get(0),
        )
        .expect("event count should be readable");
    assert_eq!(event_count, 0);
}

#[test]
fn specific_record_src_group_can_correct_prior_conflicting_record_source() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "9000000000000000002", "match-a", "clip.mp4");
    let log_records = vec![
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::EventParser,
            match_id: Some("match-a".to_string()),
            battle_id: None,
            record_src: Some(
                "C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos90000000000000001/record/20260702-173530.mp4"
                    .to_string(),
            ),
            player_name: Some("FixtureAlpha#0001".to_string()),
            map_id: None,
            map_name: Some("霓虹町".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Phoenix".to_string()),
            kda: Some("16/16/1".to_string()),
            scoreline: Some("9/13".to_string()),
            has_won: Some(false),
            combat_score: Some(208),
            kill_events: Vec::new(),
            has_gzip_event: true,
            raw_json: r#"{"match_id":"match-a"}"#.to_string(),
        },
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::TemplateParam,
            match_id: Some("match-a".to_string()),
            battle_id: Some("battle-a".to_string()),
            record_src: Some(
                "C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/match-a/clip.mp4"
                    .to_string(),
            ),
            player_name: Some("FixtureBravo#0002".to_string()),
            map_id: None,
            map_name: Some("霓虹町".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Reyna".to_string()),
            kda: Some("31/15/6".to_string()),
            scoreline: Some("13/10".to_string()),
            has_won: Some(true),
            combat_score: Some(385),
            kill_events: Vec::new(),
            has_gzip_event: false,
            raw_json: r#"{"match_id":"match-a","video_src":"C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/match-a/clip.mp4"}"#.to_string(),
        },
    ];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(
        clip.match_account_id.as_deref(),
        Some("9000000000000000002")
    );
    assert_eq!(clip.account_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.player_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.agent_name.as_deref(), Some("Reyna"));
    assert_eq!(clip.map_name.as_deref(), Some("霓虹町"));
    assert_eq!(clip.kda.as_deref(), Some("31/15/6"));
}

#[test]
fn source_less_full_summary_overrides_prior_generic_record_source() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "9000000000000000002", "match-a", "clip.mp4");
    let log_records = vec![
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::EventParser,
            match_id: Some("match-a".to_string()),
            battle_id: None,
            record_src: Some(
                "C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos90000000000000001/record/20260702-155135.mp4"
                    .to_string(),
            ),
            player_name: Some("FixtureAlpha#0001".to_string()),
            map_id: None,
            map_name: Some("霓虹町".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Phoenix".to_string()),
            kda: Some("16/16/1".to_string()),
            scoreline: None,
            has_won: None,
            combat_score: Some(80),
            kill_events: Vec::new(),
            has_gzip_event: true,
            raw_json: r#"{"match_id":"match-a"}"#.to_string(),
        },
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::TemplateParam,
            match_id: Some("match-a".to_string()),
            battle_id: Some("battle-a".to_string()),
            record_src: None,
            player_name: Some("FixtureBravo#0002".to_string()),
            map_id: None,
            map_name: Some("霓虹町".to_string()),
            game_mode: Some("竞技模式".to_string()),
            agent_name: Some("Reyna".to_string()),
            kda: Some("31/15/6".to_string()),
            scoreline: Some("13/10".to_string()),
            has_won: Some(true),
            combat_score: Some(385),
            kill_events: Vec::new(),
            has_gzip_event: false,
            raw_json: r#"{"match_id":"match-a","scoreline":"13/10"}"#.to_string(),
        },
    ];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(
        clip.match_account_id.as_deref(),
        Some("9000000000000000002")
    );
    assert_eq!(clip.account_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.player_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.agent_name.as_deref(), Some("Reyna"));
    assert_eq!(clip.map_name.as_deref(), Some("霓虹町"));
    assert_eq!(clip.kda.as_deref(), Some("31/15/6"));
}

#[test]
fn invalid_log_player_name_does_not_replace_existing_account_label() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-player-path", "clip.mp4");
    db::upsert_clip_metadata(
        &connection,
        db::ClipMetadataInput {
            clip_id,
            metadata_status: "parsed",
            json_path: None,
            account_name: Some("FixtureBravo#0002"),
            player_name: Some("FixtureBravo#0002"),
            agent_name: None,
            map_name: Some("隐世修所"),
            game_mode: Some("竞技模式"),
            scoreline: None,
            kda: Some("27/22/6"),
            extracted_text: None,
            parse_error: None,
        },
    )
    .expect("metadata should seed");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::EventParser,
        match_id: Some("match-player-path".to_string()),
        battle_id: None,
        record_src: Some(
            "D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-player-path/clip.mp4".to_string(),
        ),
        player_name: Some(
            "/Game/Maps/Jam/Jam.Jam:PersistentLevel.Vampire_PC_C_2146862510".to_string(),
        ),
        map_id: Some("/Game/Maps/Jam/Jam".to_string()),
        map_name: Some("Jam".to_string()),
        game_mode: Some("Deathmatch".to_string()),
        agent_name: Some("Reyna".to_string()),
        kda: Some("27/25/4".to_string()),
        scoreline: Some("13/8".to_string()),
        has_won: Some(true),
        combat_score: Some(300),
        kill_events: Vec::new(),
        has_gzip_event: true,
        raw_json: "{}".to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.account_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.player_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.agent_name.as_deref(), Some("Reyna"));
    assert_eq!(clip.map_name.as_deref(), Some("莲华古城"));
}

#[test]
fn untagged_log_player_name_does_not_replace_existing_account_label() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(
        &connection,
        "9000000000000000002",
        "match-opponent-name",
        "clip.mp4",
    );
    db::upsert_clip_metadata(
        &connection,
        db::ClipMetadataInput {
            clip_id,
            metadata_status: "parsed",
            json_path: None,
            account_name: Some("FixtureBravo#0002"),
            player_name: Some("FixtureBravo#0002"),
            agent_name: None,
            map_name: Some("隐世修所"),
            game_mode: Some("竞技模式"),
            scoreline: None,
            kda: Some("27/22/6"),
            extracted_text: None,
            parse_error: None,
        },
    )
    .expect("metadata should seed");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::EventParser,
        match_id: Some("match-opponent-name".to_string()),
        battle_id: None,
        record_src: Some(
            "D:/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/match-opponent-name/clip.mp4"
                .to_string(),
        ),
        player_name: Some("测试玩家甲".to_string()),
        map_id: Some("/Game/Maps/Triad/Triad".to_string()),
        map_name: Some("Triad".to_string()),
        game_mode: Some("Competitive".to_string()),
        agent_name: Some("Sova".to_string()),
        kda: Some("13/17/4".to_string()),
        scoreline: Some("12/14".to_string()),
        has_won: Some(false),
        combat_score: Some(7661),
        kill_events: vec![HighlightLogKillEvent {
            event_time: Some("2026-06-30 21:00:14.878".to_string()),
            round_id: Some(9),
            weapon_name: Some("Vandal".to_string()),
            killer_name: Some("测试玩家甲".to_string()),
            killed_name: Some("FixtureBravo".to_string()),
            raw_json: Some(r#"{"PlayerName":"测试玩家甲"}"#.to_string()),
        }],
        has_gzip_event: true,
        raw_json: r#"{"PlayerName":"测试玩家甲"}"#.to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.account_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.player_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.agent_name.as_deref(), Some("Sova"));
    assert_eq!(clip.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(clip.kda.as_deref(), Some("13/17/4"));
    assert_eq!(clip.kill_count, None);
}

#[test]
fn asset_like_log_player_name_does_not_replace_existing_account_label() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-player-card", "clip.mp4");
    db::upsert_clip_metadata(
        &connection,
        db::ClipMetadataInput {
            clip_id,
            metadata_status: "parsed",
            json_path: None,
            account_name: Some("FixtureBravo#0002"),
            player_name: Some("FixtureBravo#0002"),
            agent_name: None,
            map_name: Some("隐世修所"),
            game_mode: Some("竞技模式"),
            scoreline: None,
            kda: Some("27/22/6"),
            extracted_text: None,
            parse_error: None,
        },
    )
    .expect("metadata should seed");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::EventParser,
        match_id: Some("match-player-card".to_string()),
        battle_id: None,
        record_src: Some(
            "D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-player-card/clip.mp4".to_string(),
        ),
        player_name: Some("Cards/D3018FBE-45CD-786A-DD6C-BCAF429F7096.png".to_string()),
        map_id: None,
        map_name: Some("Jam".to_string()),
        game_mode: Some("DeathmatchGameMode".to_string()),
        agent_name: Some("Reyna".to_string()),
        kda: Some("27/25/4".to_string()),
        scoreline: Some("13/8".to_string()),
        has_won: Some(true),
        combat_score: Some(300),
        kill_events: Vec::new(),
        has_gzip_event: true,
        raw_json: "{}".to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.account_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.player_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(clip.agent_name.as_deref(), Some("Reyna"));
}

#[test]
fn normalizes_internal_map_and_mode_names_during_enrichment() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-display-names", "clip.mp4");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::EventParser,
        match_id: Some("match-display-names".to_string()),
        battle_id: None,
        record_src: Some(
            "D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-display-names/clip.mp4".to_string(),
        ),
        player_name: Some("FixtureBravo#0002".to_string()),
        map_id: Some("/Game/Maps/Jam/Jam".to_string()),
        map_name: Some("Jam".to_string()),
        game_mode: Some("DeathmatchGameMode".to_string()),
        agent_name: Some("Reyna".to_string()),
        kda: Some("27/25/4".to_string()),
        scoreline: Some("13/8".to_string()),
        has_won: Some(true),
        combat_score: Some(300),
        kill_events: Vec::new(),
        has_gzip_event: true,
        raw_json: "{}".to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.map_name.as_deref(), Some("莲华古城"));
    assert_eq!(clip.game_mode.as_deref(), Some("死斗模式"));
}

#[test]
fn invalid_log_agent_name_does_not_replace_existing_agent_label() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-agent-path", "clip.mp4");
    db::upsert_clip_metadata(
        &connection,
        db::ClipMetadataInput {
            clip_id,
            metadata_status: "parsed",
            json_path: None,
            account_name: Some("FixtureBravo#0002"),
            player_name: Some("FixtureBravo#0002"),
            agent_name: Some("Clove"),
            map_name: Some("隐世修所"),
            game_mode: Some("竞技模式"),
            scoreline: None,
            kda: Some("27/22/6"),
            extracted_text: None,
            parse_error: None,
        },
    )
    .expect("metadata should seed");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::EventParser,
        match_id: Some("match-agent-path".to_string()),
        battle_id: None,
        record_src: Some(
            "D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-agent-path/clip.mp4".to_string(),
        ),
        player_name: Some("FixtureBravo#0002".to_string()),
        map_id: None,
        map_name: Some("Jam".to_string()),
        game_mode: Some("Deathmatch".to_string()),
        agent_name: Some("FixtureBravo #0002".to_string()),
        kda: Some("27/25/4".to_string()),
        scoreline: Some("13/8".to_string()),
        has_won: Some(true),
        combat_score: Some(300),
        kill_events: Vec::new(),
        has_gzip_event: true,
        raw_json: "{}".to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.agent_name.as_deref(), Some("Clove"));
}

#[test]
fn log_record_src_file_path_matches_clip_group_parent_directory() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-c-003", "ace.mp4");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::EventParser,
        match_id: Some("match-c-003".to_string()),
        battle_id: None,
        record_src: Some(
            "D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-c-003/ace.mp4".to_string(),
        ),
        player_name: Some("PlayerOne#0000".to_string()),
        map_id: Some("/Game/Maps/Ascent/Ascent".to_string()),
        map_name: Some("Ascent".to_string()),
        game_mode: Some("Competitive".to_string()),
        agent_name: Some("Jett".to_string()),
        kda: Some("18/7/3".to_string()),
        scoreline: None,
        has_won: None,
        combat_score: Some(287),
        kill_events: vec![HighlightLogKillEvent {
            event_time: Some("2026-07-01T10:00:31Z".to_string()),
            round_id: None,
            weapon_name: Some("Vandal".to_string()),
            killer_name: None,
            killed_name: None,
            raw_json: None,
        }],
        has_gzip_event: true,
        raw_json: r#"{"games":[{"event":"H4sI"}]}"#.to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.matches_upserted, 1);
    assert_eq!(summary.enriched_clip_count, 1);
    assert_eq!(summary.unmatched_match_count, 0);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.player_name.as_deref(), Some("PlayerOne#0000"));
    assert_eq!(clip.agent_name.as_deref(), Some("Jett"));
    assert_eq!(clip.map_name.as_deref(), Some("亚海悬城"));
    assert_eq!(clip.kda.as_deref(), Some("18/7/3"));
}

#[test]
fn log_record_without_match_id_uses_record_src_group_as_game_id() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_clip_for_group(&connection, "1001", "match-from-src", "ace.mp4");
    let log_records = vec![HighlightLogRecord {
        line_kind: HighlightLogLineKind::TemplateParam,
        match_id: None,
        battle_id: None,
        record_src: Some("D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-from-src".to_string()),
        player_name: Some("PlayerOne#0000".to_string()),
        map_id: None,
        map_name: Some("Ascent".to_string()),
        game_mode: Some("Competitive".to_string()),
        agent_name: Some("Jett".to_string()),
        kda: Some("18/7/3".to_string()),
        scoreline: Some("13/11".to_string()),
        has_won: Some(true),
        combat_score: Some(287),
        kill_events: Vec::new(),
        has_gzip_event: false,
        raw_json: r#"{"recordSrc":"D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-from-src"}"#
            .to_string(),
    }];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.matches_upserted, 1);
    assert_eq!(summary.enriched_clip_count, 1);
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.player_name.as_deref(), Some("PlayerOne#0000"));
    assert_eq!(clip.kda.as_deref(), Some("18/7/3"));
    let match_id: String = connection
        .query_row(
            "SELECT match_id FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| row.get(0),
        )
        .expect("clip metadata should include match id");
    assert_eq!(match_id, "match-from-src");
}

#[test]
fn distinct_match_ids_do_not_merge_only_because_record_src_matches() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let shared_record_src =
        "D:/ACLOS/aclos-highlight/wonderfulVideos1001/shared-folder/ace.mp4".to_string();
    let log_records = vec![
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::EventParser,
            match_id: Some("match-one".to_string()),
            battle_id: None,
            record_src: Some(shared_record_src.clone()),
            player_name: Some("PlayerOne#0000".to_string()),
            map_id: None,
            map_name: Some("Ascent".to_string()),
            game_mode: None,
            agent_name: Some("Jett".to_string()),
            kda: None,
            scoreline: None,
            has_won: None,
            combat_score: None,
            kill_events: Vec::new(),
            has_gzip_event: true,
            raw_json: r#"{"gameId":"match-one"}"#.to_string(),
        },
        HighlightLogRecord {
            line_kind: HighlightLogLineKind::EventParser,
            match_id: Some("match-two".to_string()),
            battle_id: None,
            record_src: Some(shared_record_src),
            player_name: Some("PlayerOne#0000".to_string()),
            map_id: None,
            map_name: Some("Haven".to_string()),
            game_mode: None,
            agent_name: Some("Sage".to_string()),
            kda: None,
            scoreline: None,
            has_won: None,
            combat_score: None,
            kill_events: Vec::new(),
            has_gzip_event: true,
            raw_json: r#"{"gameId":"match-two"}"#.to_string(),
        },
    ];

    let summary = ingest_match_metadata(
        &connection,
        MetadataIngestInput {
            leveldb_battles: &[],
            log_records: &log_records,
        },
    )
    .expect("metadata ingest should run");

    assert_eq!(summary.matches_upserted, 2);
    let match_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM matches", [], |row| row.get(0))
        .expect("match count should be readable");
    assert_eq!(match_count, 2);
}

fn insert_clip_for_group(
    connection: &Connection,
    account_id: &str,
    group_key: &str,
    file_name: &str,
) -> i64 {
    let source_dir = db::upsert_source_dir(
        connection,
        SourceDirInput {
            path: &format!("D:/ACLOS/aclos-highlight/wonderfulVideos{account_id}"),
            name: &format!("wonderfulVideos{account_id}"),
        },
    )
    .expect("source dir should upsert");
    let clip_group = db::upsert_clip_group(
        connection,
        ClipGroupInput {
            source_dir_id: source_dir.id,
            group_key,
            display_name: group_key,
        },
    )
    .expect("clip group should upsert");
    let clip = db::upsert_clip(
        connection,
        ClipInput {
            source_dir_id: source_dir.id,
            clip_group_id: Some(clip_group.id),
            video_path: &format!(
                "D:/ACLOS/aclos-highlight/wonderfulVideos{account_id}/{group_key}/{file_name}"
            ),
            file_name,
            file_size: 42,
            modified_at: Some("1782634272"),
            duration_ms: Some(12_000),
            recorded_at: None,
            cover_path: None,
            cover_source: "missing",
        },
    )
    .expect("clip should upsert");

    clip.id
}

fn log_record_with_kill_events(match_id: &str, count: usize) -> HighlightLogRecord {
    HighlightLogRecord {
        line_kind: HighlightLogLineKind::FirstRequestData,
        match_id: Some(match_id.to_string()),
        battle_id: None,
        record_src: Some(format!(
            "D:/ACLOS/aclos-highlight/wonderfulVideos1001/{match_id}"
        )),
        player_name: Some("PlayerOne#0000".to_string()),
        map_id: Some("maps/ascent".to_string()),
        map_name: Some("Ascent".to_string()),
        game_mode: Some("Competitive".to_string()),
        agent_name: Some("Jett".to_string()),
        kda: None,
        scoreline: None,
        has_won: None,
        combat_score: None,
        kill_events: (0..count)
            .map(|index| HighlightLogKillEvent {
                event_time: Some(format!("2026-07-01T10:00:{:02}Z", 30 + index)),
                round_id: Some(3),
                weapon_name: Some("Vandal".to_string()),
                killer_name: Some("PlayerOne#0000".to_string()),
                killed_name: Some(format!("Opponent{index}#0000")),
                raw_json: Some(format!(r#"{{"index":{index}}}"#)),
            })
            .collect(),
        has_gzip_event: false,
        raw_json: format!(r#"{{"matchId":"{match_id}"}}"#),
    }
}

fn list_match_events_for_test(connection: &Connection, game_id: &str) -> Vec<i64> {
    let mut statement = connection
        .prepare(
            "
            SELECT match_events.id
            FROM match_events
            JOIN matches ON matches.id = match_events.match_id
            WHERE matches.game_id = ?1
            ORDER BY match_events.id
            ",
        )
        .expect("match event query should prepare");
    statement
        .query_map(params![game_id], |row| row.get(0))
        .expect("match events should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("match events should load")
}

fn clip_tag_names(connection: &Connection, clip: &db::Clip) -> HashSet<String> {
    let tag_ids = clip.tag_ids.iter().copied().collect::<HashSet<_>>();
    db::list_tags(connection)
        .expect("tags should list")
        .into_iter()
        .filter(|tag| tag_ids.contains(&tag.id))
        .map(|tag| tag.name)
        .collect()
}
