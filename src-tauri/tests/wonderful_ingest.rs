// Privacy note: account IDs, player names, match IDs, and paths in this file are synthetic fixtures.
use rusqlite::{params, Connection};
use valorant_highlight_manager_lib::{
    db::{self, ClipGroupInput, ClipInput, SourceDirInput},
    highlight_log_parser::{
        HighlightLogKillEvent, HighlightLogLineKind, HighlightLogRecord, HighlightLogRoundScore,
    },
    metadata_ingest::{ingest_match_metadata, MetadataIngestInput},
    wonderful_db::{
        parse_wonderful_db_text, WonderfulAccountRecord, WonderfulEventRecord,
        WonderfulMatchRecord, WonderfulSegmentRecord, WonderfulSnapshotAccountRecord,
        WonderfulSnapshotRecord, WonderfulVideoRecord,
    },
    wonderful_ingest::{
        ingest_wonderful_metadata, ingest_wonderful_metadata_with_round_scores,
        ingest_wonderful_snapshots,
    },
};

#[derive(Debug)]
struct OfficialMetadata {
    kill_count: Option<i64>,
    official_video_name: Option<String>,
    official_video_id: Option<String>,
    official_video_type: Option<String>,
    highlight_type: Option<i64>,
    metadata_source: Option<String>,
}

#[test]
fn snapshot_ingest_preserves_raw_records_and_match_stats_idempotently() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let mut accounts = vec![WonderfulSnapshotAccountRecord {
        openid: "1001".to_string(),
        snapshots: vec![WonderfulSnapshotRecord {
            match_record: WonderfulMatchRecord {
                match_id: "snapshot-match".to_string(),
                match_time: Some("1772892634".to_string()),
                map_name: Some("天枢云阙".to_string()),
                agent_name: Some("Neon".to_string()),
                game_mode: Some("竞技模式".to_string()),
                kda: Some("22/16/1".to_string()),
                scoreline: Some("13/9".to_string()),
                combat_score: Some(321),
                has_won: Some(true),
                ..WonderfulMatchRecord::default()
            },
            snapshot_id: "snapshot-1".to_string(),
            captured_at: Some("1772892627".to_string()),
            account_name: Some("FixtureBravo#0002".to_string()),
            package_path: Some(r"D:\snapshot\package.jpeg".to_string()),
            thumb_path: Some(r"D:\snapshot\thumb.jpeg".to_string()),
            width: Some(1920),
            height: Some(1080),
            size_bytes: Some(12345),
            raw_json: r#"{"snapshot":{"ss_id":"snapshot-1"}}"#.to_string(),
        }],
    }];
    let mut orphan_snapshot = accounts[0].snapshots[0].clone();
    orphan_snapshot.snapshot_id = "snapshot-without-match".to_string();
    orphan_snapshot.match_record.match_id.clear();
    accounts[0].snapshots.push(orphan_snapshot);

    let first =
        ingest_wonderful_snapshots(&connection, &accounts).expect("snapshot ingest should succeed");
    let second = ingest_wonderful_snapshots(&connection, &accounts)
        .expect("repeated snapshot ingest should succeed");

    assert_eq!(first.snapshot_count, 2);
    assert_eq!(first.match_count, 1);
    assert_eq!(second, first);
    let stored: (i64, String, String, i64, i64, i64, i64, i64, i64) = connection
        .query_row(
            "
            SELECT
                COUNT(*),
                matches.player_name,
                match_snapshots.account_name,
                match_snapshots.width,
                match_snapshots.height,
                match_stats.kills,
                match_stats.deaths,
                match_stats.assists,
                match_stats.rounds_won
            FROM match_snapshots
            JOIN matches ON matches.id = match_snapshots.match_id
            JOIN match_stats ON match_stats.match_id = matches.id
            WHERE match_snapshots.snapshot_id = 'snapshot-1'
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
                    row.get(8)?,
                ))
            },
        )
        .expect("stored snapshot should be readable");
    assert_eq!(
        stored,
        (
            1,
            "FixtureBravo#0002".to_string(),
            "FixtureBravo#0002".to_string(),
            1920,
            1080,
            22,
            16,
            1,
            13,
        )
    );
    let total_snapshots: i64 = connection
        .query_row("SELECT COUNT(*) FROM match_snapshots", [], |row| row.get(0))
        .expect("snapshot count should be readable");
    assert_eq!(total_snapshots, 2);
}

#[test]
fn current_wonderful_schema_recovers_match_metadata_for_existing_clips() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let openid = "1001";
    let match_id = "match-current-schema";
    let clip_path = "D:/synthetic/wonderfulVideos1001/match-current-schema/recoverable.mp4";
    let clip_id = insert_indexed_clip(&connection, openid, match_id, clip_path, "recoverable.mp4");
    let fixture = format!(
        r#"{{
          "key_wonderful_list_{openid}": [{{
            "matches_id": "{match_id}",
            "matches_time": "2026-03-21T12:00:00Z",
            "map": {{"map_id": "/Game/Maps/Triad/Triad"}},
            "stats": {{
              "kills": 36,
              "deaths": 17,
              "assists": 5,
              "score": 394,
              "rounds_won": 14,
              "rounds_lost": 12,
              "has_won": true
            }},
            "career": {{
              "battle_id": "battle-current-schema",
              "hero_name": "Sova",
              "hero_image": "https://assets.example/sova.png",
              "map_name": "隐世修所",
              "game_mode": "竞技模式",
              "kda": "36/17/5",
              "rounds_score": "14/12",
              "score": "394",
              "won_match": true
            }},
            "videos": [{{
              "video_id": "recoverable",
              "video_name": "三杀时刻",
              "video_type": "4",
              "video_src": "{clip_path}",
              "round_clips": []
            }}]
          }}]
        }}"#
    );
    let account = WonderfulAccountRecord {
        openid: openid.to_string(),
        matches: parse_wonderful_db_text(openid, &fixture)
            .expect("current WonderfulDb schema should parse"),
    };

    ingest_wonderful_metadata(&connection, &[account]).expect("official metadata should import");

    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.metadata_source.as_deref(), Some("wonderful_db"));
    assert_eq!(clip.match_id.as_deref(), Some(match_id));
    assert_eq!(clip.match_account_id.as_deref(), Some(openid));
    assert_eq!(clip.agent_name.as_deref(), Some("Sova"));
    assert_eq!(clip.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(clip.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(clip.kda.as_deref(), Some("36/17/5"));
    assert_eq!(clip.scoreline.as_deref(), Some("14/12"));
    assert_eq!(clip.combat_score, Some(394));
    assert_eq!(clip.has_won, Some(true));
    assert_eq!(
        clip.match_started_at.as_deref(),
        Some("2026-03-21T12:00:00Z")
    );
}

#[test]
fn current_wonderful_schema_backfills_an_already_matched_official_clip() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-backfill",
        "D:/synthetic/wonderfulVideos1001/match-backfill/legacy.mp4",
        "legacy.mp4",
    );
    db::upsert_clip_metadata(
        &connection,
        db::ClipMetadataInput {
            clip_id,
            metadata_status: "enriched",
            json_path: None,
            account_name: None,
            player_name: None,
            agent_name: None,
            map_name: None,
            game_mode: None,
            scoreline: None,
            kda: None,
            extracted_text: None,
            parse_error: None,
        },
    )
    .expect("legacy metadata should seed");
    connection
        .execute(
            "UPDATE clip_metadata SET match_id = 'match-backfill', metadata_source = 'wonderful_db' WHERE clip_id = ?1",
            params![clip_id],
        )
        .expect("official identity should seed");
    let account = WonderfulAccountRecord {
        openid: "1001".to_string(),
        matches: vec![WonderfulMatchRecord {
            match_id: "match-backfill".to_string(),
            map_id: Some("/Game/Maps/Triad/Triad".to_string()),
            map_name: Some("隐世修所".to_string()),
            agent_name: Some("Sova".to_string()),
            game_mode: Some("竞技模式".to_string()),
            kda: Some("36/17/5".to_string()),
            scoreline: Some("14/12".to_string()),
            combat_score: Some(394),
            has_won: Some(true),
            ..WonderfulMatchRecord::default()
        }],
    };

    ingest_wonderful_metadata(&connection, &[account]).expect("official metadata should backfill");

    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert_eq!(clip.agent_name.as_deref(), Some("Sova"));
    assert_eq!(clip.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(clip.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(clip.kda.as_deref(), Some("36/17/5"));
    assert_eq!(clip.scoreline.as_deref(), Some("14/12"));
}

#[test]
fn assigns_events_to_their_exact_video() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let openid = "1001";
    let match_id = "match-with-two-videos";
    let three_path = r"D:\synthetic\wonderfulVideos1001\match-with-two-videos\three.mp4";
    let six_path = r"D:\synthetic\wonderfulVideos1001\match-with-two-videos\six.mp4";
    let three_clip_id = insert_indexed_clip(&connection, openid, match_id, three_path, "three.mp4");
    let six_clip_id = insert_indexed_clip(&connection, openid, match_id, six_path, "six.mp4");
    let accounts = vec![account_with_videos(
        openid,
        match_id,
        vec![
            video_record("three", "三杀时刻", "4", Some(three_path), 3),
            video_record("six", "六杀时刻", "10", Some(six_path), 6),
        ],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    let three = load_official_metadata(&connection, three_clip_id);
    let three_events = db::list_clip_events_for_clip(&connection, three_clip_id)
        .expect("three-kill events should load");
    assert_eq!(three.kill_count, Some(3));
    assert_eq!(three.official_video_name.as_deref(), Some("三杀时刻"));
    assert_eq!(three.official_video_id.as_deref(), Some("three"));
    assert_eq!(three.official_video_type.as_deref(), Some("4"));
    assert_eq!(three.highlight_type, Some(4));
    assert_eq!(three.metadata_source.as_deref(), Some("wonderful_db"));
    assert_eq!(three_events.len(), 3);
    assert!(three_events
        .iter()
        .all(|event| event.event_key.starts_with("three-event-")));

    let six = load_official_metadata(&connection, six_clip_id);
    let six_events = db::list_clip_events_for_clip(&connection, six_clip_id)
        .expect("six-kill events should load");
    assert_eq!(six.kill_count, Some(6));
    assert_eq!(six.official_video_name.as_deref(), Some("六杀时刻"));
    assert_eq!(six_events.len(), 6);
    assert!(six_events
        .iter()
        .all(|event| event.event_key.starts_with("six-event-")));
    let three_clip =
        db::find_clip_by_id(&connection, three_clip_id).expect("three-kill clip should reload");
    assert_eq!(three_clip.duration_ms, Some(9_000));
    assert_eq!(
        three_clip.recorded_at.as_deref(),
        Some("2026-07-04T12:00:00Z")
    );
}

#[test]
fn official_video_types_do_not_create_or_delete_user_tags() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-compilation\compilation.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-compilation",
        clip_path,
        "compilation.mp4",
    );
    let custom =
        db::create_tag(&connection, "复盘", Some("blue")).expect("custom tag should exist");
    let video_named_custom =
        db::create_tag(&connection, "三杀", Some("gold")).expect("custom tag should exist");
    db::assign_tag_to_clip(&connection, clip_id, custom.id).expect("custom tag should assign");
    db::assign_tag_to_clip(&connection, clip_id, video_named_custom.id)
        .expect("video-named custom tag should assign");
    db::update_clip_favorite(&connection, clip_id, true).expect("favorite should seed");
    db::update_clip_note(&connection, clip_id, Some("保留用户备注")).expect("note should seed");
    let accounts = vec![account_with_videos(
        "1001",
        "match-compilation",
        vec![video_record(
            "compilation",
            "击杀合集",
            "2",
            Some(clip_path),
            75,
        )],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    let tags = assigned_tag_names(&connection, clip_id);
    assert!(tags.contains(&"复盘".to_string()));
    assert!(tags.contains(&"三杀".to_string()));
    assert!(!tags.contains(&"击杀合集".to_string()));
    assert!(!tags.contains(&"75杀".to_string()));
    assert_eq!(
        load_official_metadata(&connection, clip_id).highlight_type,
        Some(2)
    );
    let clip = db::find_clip_by_id(&connection, clip_id).expect("clip should reload");
    assert!(clip.favorite);
    assert_eq!(clip.note.as_deref(), Some("保留用户备注"));
}

#[test]
fn rescanning_updates_video_type_without_touching_user_tags() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-retag\retag.mp4";
    let clip_id = insert_indexed_clip(&connection, "1001", "match-retag", clip_path, "retag.mp4");
    let custom =
        db::create_tag(&connection, "复盘", Some("blue")).expect("custom tag should exist");
    db::assign_tag_to_clip(&connection, clip_id, custom.id).expect("custom tag should assign");
    let kill_compilation = vec![account_with_videos(
        "1001",
        "match-retag",
        vec![video_record("retag", "击杀合集", "2", Some(clip_path), 3)],
    )];
    ingest_wonderful_metadata(&connection, &kill_compilation).expect("first ingest should succeed");
    assert_eq!(
        load_official_metadata(&connection, clip_id).highlight_type,
        Some(2)
    );
    assert_eq!(
        assigned_tag_names(&connection, clip_id),
        vec!["复盘".to_string()]
    );

    let death_compilation = vec![account_with_videos(
        "1001",
        "match-retag",
        vec![video_record("retag", "死亡集锦", "3", Some(clip_path), 0)],
    )];
    ingest_wonderful_metadata(&connection, &death_compilation).expect("rescan should succeed");

    let tags = assigned_tag_names(&connection, clip_id);
    assert_eq!(tags, vec!["复盘".to_string()]);
    assert_eq!(
        load_official_metadata(&connection, clip_id).highlight_type,
        Some(3)
    );
}

#[test]
fn official_classification_never_interprets_user_tag_names_as_video_types() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-fallback-tag\clip.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-fallback-tag",
        clip_path,
        "clip.mp4",
    );
    for label in ["ACE", "三杀", "击杀集锦", "复盘"] {
        let tag = db::create_tag(&connection, label, Some("blue")).expect("tag should exist");
        db::assign_tag_to_clip(&connection, clip_id, tag.id).expect("tag should assign");
    }
    let accounts = vec![account_with_videos(
        "1001",
        "match-fallback-tag",
        vec![video_record("clip", "四杀时刻", "6", Some(clip_path), 4)],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    let tags = assigned_tag_names(&connection, clip_id);
    assert_eq!(
        tags,
        vec![
            "ACE".to_string(),
            "三杀".to_string(),
            "击杀集锦".to_string(),
            "复盘".to_string()
        ]
    );
    assert_eq!(
        load_official_metadata(&connection, clip_id).highlight_type,
        Some(6)
    );
}

#[test]
fn lower_priority_log_ingest_cannot_replace_official_clip_data() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-priority\six.mp4";
    let clip_id = insert_indexed_clip(&connection, "1001", "match-priority", clip_path, "six.mp4");
    let accounts = vec![account_with_videos(
        "1001",
        "match-priority",
        vec![video_record("six", "六杀时刻", "10", Some(clip_path), 6)],
    )];
    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");
    let original_event_keys = db::list_clip_events_for_clip(&connection, clip_id)
        .expect("official events should load")
        .into_iter()
        .map(|event| event.event_key)
        .collect::<Vec<_>>();
    for log_kill_count in [3, 4] {
        let log_records = vec![priority_log_record(clip_path, log_kill_count)];
        ingest_match_metadata(
            &connection,
            MetadataIngestInput {
                leveldb_battles: &[],
                log_records: &log_records,
            },
        )
        .expect("lower-priority log ingest should run");
    }

    let metadata = load_official_metadata(&connection, clip_id);
    let reloaded_event_keys = db::list_clip_events_for_clip(&connection, clip_id)
        .expect("official events should remain")
        .into_iter()
        .map(|event| event.event_key)
        .collect::<Vec<_>>();
    assert_eq!(metadata.kill_count, Some(6));
    assert_eq!(metadata.official_video_name.as_deref(), Some("六杀时刻"));
    assert_eq!(reloaded_event_keys, original_event_keys);
    let kda: Option<String> = connection
        .query_row(
            "SELECT kda FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| row.get(0),
        )
        .expect("complementary log metadata should load");
    assert_eq!(kda.as_deref(), Some("1/1/1"));
    assert!(assigned_tag_names(&connection, clip_id).is_empty());
}

#[test]
fn repeated_identical_ingest_does_not_duplicate_segments_or_events() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-idempotent\six.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-idempotent",
        clip_path,
        "six.mp4",
    );
    let accounts = vec![account_with_videos(
        "1001",
        "match-idempotent",
        vec![video_record("six", "六杀时刻", "10", Some(clip_path), 6)],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("first ingest should succeed");
    ingest_wonderful_metadata(&connection, &accounts).expect("second ingest should succeed");

    let segment_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM clip_segments WHERE clip_id = ?1",
            params![clip_id],
            |row| row.get(0),
        )
        .expect("segment count should load");
    let events =
        db::list_clip_events_for_clip(&connection, clip_id).expect("official events should load");
    let unique_event_keys = events
        .iter()
        .map(|event| event.event_key.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(segment_count, 1);
    assert_eq!(events.len(), 6);
    assert_eq!(unique_event_keys.len(), 6);
}

#[test]
fn unmatched_official_video_is_reported_without_enriching_another_clip() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let match_id = "match-unmatched";
    let first_clip_id = insert_indexed_clip(
        &connection,
        "1001",
        match_id,
        r"D:\synthetic\wonderfulVideos1001\match-unmatched\first.mp4",
        "first.mp4",
    );
    let second_clip_id = insert_indexed_clip(
        &connection,
        "1001",
        match_id,
        r"D:\synthetic\wonderfulVideos1001\match-unmatched\second.mp4",
        "second.mp4",
    );
    let accounts = vec![account_with_videos(
        "1001",
        match_id,
        vec![video_record(
            "missing-video",
            "不应匹配",
            "10",
            Some(r"D:\synthetic\elsewhere\missing-video.mp4"),
            6,
        )],
    )];

    let summary = ingest_wonderful_metadata(&connection, &accounts)
        .expect("unmatched official ingest should complete");

    assert_eq!(summary.matched_video_count, 0);
    assert_eq!(summary.unmatched_video_count, 1);
    for clip_id in [first_clip_id, second_clip_id] {
        let metadata = load_official_metadata(&connection, clip_id);
        assert_eq!(metadata.kill_count, None);
        assert_eq!(metadata.official_video_name, None);
        assert!(db::list_clip_events_for_clip(&connection, clip_id)
            .expect("clip events should load")
            .is_empty());
    }
}

#[test]
fn ambiguous_scoped_stem_fallback_is_unmatched() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let match_id = "match-ambiguous-stem";
    let first_clip_id = insert_indexed_clip(
        &connection,
        "1001",
        match_id,
        r"D:\synthetic\wonderfulVideos1001\match-ambiguous-stem\a\shared.mp4",
        "shared.mp4",
    );
    let second_clip_id = insert_indexed_clip(
        &connection,
        "1001",
        match_id,
        r"D:\synthetic\wonderfulVideos1001\match-ambiguous-stem\b\shared.mp4",
        "shared.mp4",
    );
    let accounts = vec![account_with_videos(
        "1001",
        match_id,
        vec![video_record("shared", "不得猜测", "4", None, 3)],
    )];

    let summary = ingest_wonderful_metadata(&connection, &accounts)
        .expect("ambiguous fallback should complete");

    assert_eq!(summary.matched_video_count, 0);
    assert_eq!(summary.unmatched_video_count, 1);
    for clip_id in [first_clip_id, second_clip_id] {
        assert_eq!(
            load_official_metadata(&connection, clip_id).kill_count,
            None
        );
    }
}

#[test]
fn official_ingest_replaces_stale_kill_count() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-stale\six.mp4";
    let clip_id = insert_indexed_clip(&connection, "1001", "match-stale", clip_path, "six.mp4");
    connection
        .execute(
            "
            UPDATE clip_metadata
            SET kill_count = 26,
                official_video_name = '旧记录',
                metadata_source = 'wonderful_db'
            WHERE clip_id = ?1
            ",
            params![clip_id],
        )
        .expect("stale metadata should seed");
    let accounts = vec![account_with_videos(
        "1001",
        "match-stale",
        vec![video_record("six", "六杀时刻", "10", Some(clip_path), 6)],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    let metadata = load_official_metadata(&connection, clip_id);
    assert_eq!(metadata.kill_count, Some(6));
    assert_eq!(metadata.official_video_name.as_deref(), Some("六杀时刻"));
}

#[test]
fn changed_official_video_replaces_only_its_clip_timeline() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let match_id = "match-changed-video";
    let changed_path = r"D:\synthetic\wonderfulVideos1001\match-changed-video\changed.mp4";
    let untouched_path = r"D:\synthetic\wonderfulVideos1001\match-changed-video\untouched.mp4";
    let changed_clip_id =
        insert_indexed_clip(&connection, "1001", match_id, changed_path, "changed.mp4");
    let untouched_clip_id = insert_indexed_clip(
        &connection,
        "1001",
        match_id,
        untouched_path,
        "untouched.mp4",
    );
    let initial = vec![account_with_videos(
        "1001",
        match_id,
        vec![
            video_record("changed", "三杀时刻", "4", Some(changed_path), 3),
            video_record("untouched", "六杀时刻", "10", Some(untouched_path), 6),
        ],
    )];
    ingest_wonderful_metadata(&connection, &initial).expect("initial ingest should succeed");
    let custom_tag =
        db::create_tag(&connection, "复盘", Some("blue")).expect("custom tag should exist");
    db::assign_tag_to_clip(&connection, changed_clip_id, custom_tag.id)
        .expect("custom tag should assign");
    db::update_clip_favorite(&connection, changed_clip_id, true).expect("favorite should seed");
    db::update_clip_note(&connection, changed_clip_id, Some("保留 changed 备注"))
        .expect("note should seed");
    let untouched_event_keys = db::list_clip_events_for_clip(&connection, untouched_clip_id)
        .expect("untouched events should load")
        .into_iter()
        .map(|event| event.event_key)
        .collect::<Vec<_>>();
    let changed = vec![account_with_videos(
        "1001",
        match_id,
        vec![video_record(
            "changed",
            "四杀时刻",
            "6",
            Some(changed_path),
            4,
        )],
    )];

    ingest_wonderful_metadata(&connection, &changed).expect("changed ingest should succeed");

    let changed_events = db::list_clip_events_for_clip(&connection, changed_clip_id)
        .expect("changed events should load");
    assert_eq!(changed_events.len(), 4);
    assert_eq!(
        load_official_metadata(&connection, changed_clip_id).kill_count,
        Some(4)
    );
    let changed_tags = assigned_tag_names(&connection, changed_clip_id);
    assert_eq!(changed_tags, vec!["复盘".to_string()]);
    let changed_clip = db::find_clip_by_id(&connection, changed_clip_id)
        .expect("changed clip user state should reload");
    assert!(changed_clip.favorite);
    assert_eq!(changed_clip.note.as_deref(), Some("保留 changed 备注"));
    let reloaded_untouched_event_keys =
        db::list_clip_events_for_clip(&connection, untouched_clip_id)
            .expect("untouched events should remain")
            .into_iter()
            .map(|event| event.event_key)
            .collect::<Vec<_>>();
    assert_eq!(reloaded_untouched_event_keys, untouched_event_keys);
    assert_eq!(
        load_official_metadata(&connection, untouched_clip_id).kill_count,
        Some(6)
    );
}

#[test]
fn official_death_five_and_six_types_do_not_create_tags() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let match_id = "match-tag-matrix";
    let death_path = r"D:\synthetic\wonderfulVideos1001\match-tag-matrix\death.mp4";
    let five_path = r"D:\synthetic\wonderfulVideos1001\match-tag-matrix\five.mp4";
    let six_path = r"D:\synthetic\wonderfulVideos1001\match-tag-matrix\six.mp4";
    let death_clip_id = insert_indexed_clip(&connection, "1001", match_id, death_path, "death.mp4");
    let five_clip_id = insert_indexed_clip(&connection, "1001", match_id, five_path, "five.mp4");
    let six_clip_id = insert_indexed_clip(&connection, "1001", match_id, six_path, "six.mp4");
    let accounts = vec![account_with_videos(
        "1001",
        match_id,
        vec![
            video_record("death", "死亡集锦", "3", Some(death_path), 0),
            video_record("five", "五杀时刻", "10", Some(five_path), 5),
            video_record("six", "六杀时刻", "10", Some(six_path), 6),
        ],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    assert!(assigned_tag_names(&connection, death_clip_id).is_empty());
    assert!(assigned_tag_names(&connection, five_clip_id).is_empty());
    assert!(assigned_tag_names(&connection, six_clip_id).is_empty());
    assert_eq!(
        load_official_metadata(&connection, death_clip_id).highlight_type,
        Some(3)
    );
    assert_eq!(
        load_official_metadata(&connection, five_clip_id).kill_count,
        Some(5)
    );
    assert_eq!(
        load_official_metadata(&connection, six_clip_id).kill_count,
        Some(6)
    );
}

#[test]
fn normalized_video_src_precedes_exact_scoped_stem_fallback() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let match_id = "match-order";
    let path_target = r"D:\synthetic\wonderfulVideos1001\match-order\path-target.mp4";
    let stem_decoy = r"D:\synthetic\wonderfulVideos1001\match-order\stem-decoy.mp4";
    let fallback_target = r"D:\synthetic\wonderfulVideos1001\match-order\fallback.mp4";
    let path_target_id = insert_indexed_clip(
        &connection,
        "1001",
        match_id,
        path_target,
        "path-target.mp4",
    );
    let stem_decoy_id =
        insert_indexed_clip(&connection, "1001", match_id, stem_decoy, "stem-decoy.mp4");
    let fallback_target_id = insert_indexed_clip(
        &connection,
        "1001",
        match_id,
        fallback_target,
        "fallback.mp4",
    );
    let accounts = vec![account_with_videos(
        "1001",
        match_id,
        vec![
            video_record("stem-decoy", "路径优先", "4", Some(path_target), 3),
            video_record("fallback", "Stem 回退", "6", None, 4),
        ],
    )];

    let summary = ingest_wonderful_metadata(&connection, &accounts)
        .expect("ordered official matching should succeed");

    assert_eq!(summary.matched_video_count, 2);
    assert_eq!(summary.unmatched_video_count, 0);
    assert_eq!(
        load_official_metadata(&connection, path_target_id)
            .official_video_name
            .as_deref(),
        Some("路径优先")
    );
    assert_eq!(
        load_official_metadata(&connection, stem_decoy_id).kill_count,
        None
    );
    assert_eq!(
        load_official_metadata(&connection, fallback_target_id)
            .official_video_name
            .as_deref(),
        Some("Stem 回退")
    );
}

#[test]
fn exact_video_src_recovers_legacy_record_without_match_id() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\legacy-video.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "legacy-video",
        clip_path,
        "legacy-video.mp4",
    );
    let accounts = vec![account_with_videos(
        "1001",
        "",
        vec![video_record(
            "legacy-video",
            "旧版击杀集锦",
            "2",
            Some(clip_path),
            2,
        )],
    )];

    let summary = ingest_wonderful_metadata(&connection, &accounts)
        .expect("legacy official matching should succeed");

    assert_eq!(summary.matched_video_count, 1);
    assert_eq!(summary.unmatched_video_count, 0);
    let metadata = load_official_metadata(&connection, clip_id);
    assert_eq!(
        metadata.official_video_name.as_deref(),
        Some("旧版击杀集锦")
    );
    assert_eq!(metadata.kill_count, Some(2));
}

#[test]
fn kill_count_uses_unique_self_kill_events_only() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-event-filter\filtered.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-event-filter",
        clip_path,
        "filtered.mp4",
    );
    let mut video = video_record("filtered", "三杀时刻", "4", Some(clip_path), 3);
    let duplicate = video.segments[0].events[0].clone();
    let mut death = duplicate.clone();
    death.event_id = "filtered-death".to_string();
    death.event_type = "death".to_string();
    let mut other_killer = duplicate;
    other_killer.event_id = "filtered-other-killer".to_string();
    other_killer.killer_is_me = false;
    let duplicate_again = video.segments[0].events[0].clone();
    video.segments[0].events.extend([death, other_killer]);
    video.segments[0].events.push(duplicate_again);
    let accounts = vec![account_with_videos(
        "1001",
        "match-event-filter",
        vec![video],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    assert_eq!(
        load_official_metadata(&connection, clip_id).kill_count,
        Some(3)
    );
    assert_eq!(
        db::list_clip_events_for_clip(&connection, clip_id)
            .expect("deduplicated events should load")
            .len(),
        5
    );
    assert!(assigned_tag_names(&connection, clip_id).is_empty());
}

#[test]
fn exact_video_src_requires_matching_account_and_match() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let wrong_account_path = r"D:\synthetic\wonderfulVideos2002\expected-match\wrong-account.mp4";
    let wrong_match_path = r"D:\synthetic\wonderfulVideos1001\foreign-match\wrong-match.mp4";
    let wrong_account_clip_id = insert_indexed_clip(
        &connection,
        "2002",
        "expected-match",
        wrong_account_path,
        "wrong-account.mp4",
    );
    let wrong_match_clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "foreign-match",
        wrong_match_path,
        "wrong-match.mp4",
    );
    let accounts = vec![account_with_videos(
        "1001",
        "expected-match",
        vec![
            video_record(
                "wrong-account",
                "不得跨账号认领",
                "4",
                Some(wrong_account_path),
                3,
            ),
            video_record(
                "wrong-match",
                "不得跨比赛认领",
                "4",
                Some(wrong_match_path),
                3,
            ),
        ],
    )];

    let summary = ingest_wonderful_metadata(&connection, &accounts)
        .expect("scoped official matching should complete");

    assert_eq!(summary.matched_video_count, 0);
    assert_eq!(summary.unmatched_video_count, 2);
    for clip_id in [wrong_account_clip_id, wrong_match_clip_id] {
        let metadata = load_official_metadata(&connection, clip_id);
        assert_eq!(metadata.kill_count, None);
        assert_eq!(metadata.official_video_name, None);
    }
}

#[test]
fn two_official_records_cannot_claim_the_same_clip() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-claimed\shared.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-claimed",
        clip_path,
        "shared.mp4",
    );
    let accounts = vec![account_with_videos(
        "1001",
        "match-claimed",
        vec![
            video_record("first", "首次认领", "4", Some(clip_path), 3),
            video_record("second", "重复认领", "6", Some(clip_path), 4),
        ],
    )];

    let summary = ingest_wonderful_metadata(&connection, &accounts)
        .expect("duplicate claim ingest should complete");

    assert_eq!(summary.matched_video_count, 1);
    assert_eq!(summary.unmatched_video_count, 1);
    let metadata = load_official_metadata(&connection, clip_id);
    assert_eq!(metadata.official_video_name.as_deref(), Some("首次认领"));
    assert_eq!(metadata.kill_count, Some(3));
}

#[test]
fn identical_events_without_ids_are_deduplicated_by_content() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-no-id-duplicate\no-id.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-no-id-duplicate",
        clip_path,
        "no-id.mp4",
    );
    let duplicate = event_without_id("Opponent#0001", 1_500, 7);
    let mut video = video_record("no-id", "单杀", "1", Some(clip_path), 0);
    video.segments[0].events = vec![duplicate.clone(), duplicate];
    let accounts = vec![account_with_videos(
        "1001",
        "match-no-id-duplicate",
        vec![video],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    assert_eq!(
        load_official_metadata(&connection, clip_id).kill_count,
        Some(1)
    );
    assert_eq!(
        db::list_clip_events_for_clip(&connection, clip_id)
            .expect("deduplicated events should load")
            .len(),
        1
    );
}

#[test]
fn event_keys_without_ids_remain_stable_when_events_are_reordered() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-no-id-order\no-id.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-no-id-order",
        clip_path,
        "no-id.mp4",
    );
    let first = event_without_id("Opponent#0001", 1_500, 7);
    let second = event_without_id("Opponent#0002", 2_000, 8);
    let mut video = video_record("no-id", "双杀", "1", Some(clip_path), 0);
    video.segments[0].events = vec![first.clone(), second.clone()];
    let initial = vec![account_with_videos(
        "1001",
        "match-no-id-order",
        vec![video.clone()],
    )];
    ingest_wonderful_metadata(&connection, &initial).expect("initial ingest should succeed");
    let initial_keys = event_keys_by_victim(&connection, clip_id);
    video.segments[0].events = vec![second, first];
    let reordered = vec![account_with_videos(
        "1001",
        "match-no-id-order",
        vec![video],
    )];

    ingest_wonderful_metadata(&connection, &reordered).expect("reordered ingest should succeed");

    assert_eq!(event_keys_by_victim(&connection, clip_id), initial_keys);
}

#[test]
fn persists_official_round_score() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-round-score\score.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-round-score",
        clip_path,
        "score.mp4",
    );
    let mut video = video_record("score", "比分记录", "1", Some(clip_path), 1);
    video.round_score = Some(13);
    let accounts = vec![account_with_videos(
        "1001",
        "match-round-score",
        vec![video],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    let round_score: Option<i64> = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER) FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| row.get(0),
        )
        .expect("round score should load");
    assert_eq!(round_score, Some(13));
}

#[test]
fn missing_official_round_score_does_not_erase_a_previously_recovered_score() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-round-score-retry\score.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-round-score-retry",
        clip_path,
        "score.mp4",
    );
    let mut scored_video = video_record("score", "四杀时刻", "高光时刻", Some(clip_path), 4);
    scored_video.round_score = Some(846);
    ingest_wonderful_metadata(
        &connection,
        &[account_with_videos(
            "1001",
            "match-round-score-retry",
            vec![scored_video],
        )],
    )
    .expect("scored ingest should succeed");

    let missing_video = video_record("score", "四杀时刻", "高光时刻", Some(clip_path), 4);
    ingest_wonderful_metadata(
        &connection,
        &[account_with_videos(
            "1001",
            "match-round-score-retry",
            vec![missing_video],
        )],
    )
    .expect("missing-score ingest should succeed");

    let round_score: Option<i64> = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER) FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| row.get(0),
        )
        .expect("preserved round score should load");
    assert_eq!(round_score, Some(846));
}

#[test]
fn recovers_missing_round_score_from_verified_official_log_delta() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-log-score\score.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-log-score",
        clip_path,
        "score.mp4",
    );
    let video = video_record("score", "精准预判", "三杀时刻", Some(clip_path), 3);
    let mut account = account_with_videos("1001", "match-log-score", vec![video]);
    account.matches[0].game_mode = Some("竞技模式".to_string());
    let scores = vec![
        HighlightLogRoundScore {
            account_id: "1001".to_string(),
            match_id: " MATCH-LOG-SCORE ".to_string(),
            round_id: 7,
            score: 846,
        },
        HighlightLogRoundScore {
            account_id: "different-account".to_string(),
            match_id: "match-log-score".to_string(),
            round_id: 7,
            score: 999,
        },
    ];

    let summary = ingest_wonderful_metadata_with_round_scores(&connection, &[account], &scores)
        .expect("verified log score ingest should succeed");

    let stored: (Option<i64>, Option<String>) = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER), round_score_source FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("recovered round score should load");
    assert_eq!(stored, (Some(846), Some("highlight_log_delta".to_string())));
    assert_eq!(summary.round_score_backfilled_count, 1);
}

#[test]
fn wonderful_db_round_score_wins_over_reconstructed_log_score() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-official-score-priority\score.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-official-score-priority",
        clip_path,
        "score.mp4",
    );
    let mut video = video_record("score", "四杀时刻", "四杀时刻", Some(clip_path), 4);
    video.round_score = Some(921);
    let mut account = account_with_videos("1001", "match-official-score-priority", vec![video]);
    account.matches[0].game_mode = Some("竞技模式".to_string());
    let scores = vec![HighlightLogRoundScore {
        account_id: "1001".to_string(),
        match_id: "match-official-score-priority".to_string(),
        round_id: 7,
        score: 846,
    }];

    let summary = ingest_wonderful_metadata_with_round_scores(&connection, &[account], &scores)
        .expect("official score priority ingest should succeed");

    let stored: (Option<i64>, Option<String>) = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER), round_score_source FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("official round score should load");
    assert_eq!(stored, (Some(921), Some("wonderful_db".to_string())));
    assert_eq!(summary.round_score_backfilled_count, 0);
}

#[test]
fn previously_stored_wonderful_db_score_is_not_downgraded_to_log_delta() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-score-provenance\score.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-score-provenance",
        clip_path,
        "score.mp4",
    );
    let mut official_video = video_record("score", "四杀时刻", "四杀时刻", Some(clip_path), 4);
    official_video.round_score = Some(921);
    let mut official_account =
        account_with_videos("1001", "match-score-provenance", vec![official_video]);
    official_account.matches[0].game_mode = Some("竞技模式".to_string());
    ingest_wonderful_metadata(&connection, &[official_account])
        .expect("official score should seed");

    let missing_video = video_record("score", "四杀时刻", "四杀时刻", Some(clip_path), 4);
    let mut missing_account =
        account_with_videos("1001", "match-score-provenance", vec![missing_video]);
    missing_account.matches[0].game_mode = Some("竞技模式".to_string());
    let summary = ingest_wonderful_metadata_with_round_scores(
        &connection,
        &[missing_account],
        &[HighlightLogRoundScore {
            account_id: "1001".to_string(),
            match_id: "match-score-provenance".to_string(),
            round_id: 7,
            score: 846,
        }],
    )
    .expect("later log score ingest should succeed");

    let stored: (Option<i64>, Option<String>) = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER), round_score_source FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("preserved official score should load");
    assert_eq!(stored, (Some(921), Some("wonderful_db".to_string())));
    assert_eq!(summary.round_score_backfilled_count, 0);
}

#[test]
fn conflicting_case_insensitive_log_score_candidates_are_discarded() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-log-conflict\score.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-log-conflict",
        clip_path,
        "score.mp4",
    );
    let video = video_record("score", "三杀时刻", "三杀时刻", Some(clip_path), 3);
    let mut account = account_with_videos("1001", "match-log-conflict", vec![video]);
    account.matches[0].game_mode = Some("竞技模式".to_string());

    let summary = ingest_wonderful_metadata_with_round_scores(
        &connection,
        &[account],
        &[
            HighlightLogRoundScore {
                account_id: "1001".to_string(),
                match_id: "match-log-conflict".to_string(),
                round_id: 7,
                score: 734,
            },
            HighlightLogRoundScore {
                account_id: " 1001 ".to_string(),
                match_id: " MATCH-LOG-CONFLICT ".to_string(),
                round_id: 7,
                score: 735,
            },
        ],
    )
    .expect("conflicting log score ingest should succeed without a score");

    let stored: (Option<i64>, Option<String>) = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER), round_score_source FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("discarded score state should load");
    assert_eq!(stored, (None, None));
    assert_eq!(summary.round_score_backfilled_count, 0);
}

#[test]
fn reconstructed_score_is_strictly_scoped_and_survives_log_rotation() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-log-rotation\score.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-log-rotation",
        clip_path,
        "score.mp4",
    );
    let video = video_record("score", "三杀时刻", "三杀时刻", Some(clip_path), 3);
    let mut account = account_with_videos("1001", "match-log-rotation", vec![video]);
    account.matches[0].game_mode = Some("竞技模式".to_string());
    let scores = vec![HighlightLogRoundScore {
        account_id: "1001".to_string(),
        match_id: "match-log-rotation".to_string(),
        round_id: 7,
        score: 734,
    }];
    ingest_wonderful_metadata_with_round_scores(&connection, &[account.clone()], &scores)
        .expect("initial reconstructed score ingest should succeed");

    ingest_wonderful_metadata(&connection, &[account])
        .expect("rescan after log rotation should succeed");

    let stored: (Option<i64>, Option<String>) = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER), round_score_source FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("preserved reconstructed score should load");
    assert_eq!(stored, (Some(734), Some("highlight_log_delta".to_string())));

    let mut changed_identity = account_with_videos(
        "1001",
        "match-log-rotation",
        vec![video_record(
            "replacement-score",
            "三杀时刻",
            "三杀时刻",
            Some(clip_path),
            3,
        )],
    );
    changed_identity.matches[0].game_mode = Some("竞技模式".to_string());
    ingest_wonderful_metadata(&connection, &[changed_identity])
        .expect("changed official identity ingest should succeed");
    let cleared: (Option<i64>, Option<String>) = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER), round_score_source FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("cleared score should load");
    assert_eq!(cleared, (None, None));
}

#[test]
fn reconstructed_score_is_not_reused_when_stored_match_identity_disagrees() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-score-a\score.mp4";
    let clip_id = insert_indexed_clip(&connection, "1001", "match-score-a", clip_path, "score.mp4");
    let video = video_record("score", "三杀时刻", "三杀时刻", Some(clip_path), 3);
    let mut first_account = account_with_videos("1001", "match-score-a", vec![video.clone()]);
    first_account.matches[0].game_mode = Some("竞技模式".to_string());
    ingest_wonderful_metadata_with_round_scores(
        &connection,
        &[first_account],
        &[HighlightLogRoundScore {
            account_id: "1001".to_string(),
            match_id: "match-score-a".to_string(),
            round_id: 7,
            score: 734,
        }],
    )
    .expect("initial reconstructed score ingest should succeed");

    connection
        .execute(
            "UPDATE clip_metadata SET match_id = 'match-score-b' WHERE clip_id = ?1",
            params![clip_id],
        )
        .expect("stale stored match identity should seed");
    let mut replacement_account = account_with_videos("1001", "match-score-a", vec![video]);
    replacement_account.matches[0].game_mode = Some("竞技模式".to_string());
    ingest_wonderful_metadata(&connection, &[replacement_account])
        .expect("replacement match ingest should succeed");

    let stored: (Option<i64>, Option<String>) = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER), round_score_source FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("replacement score state should load");
    assert_eq!(stored, (None, None));
}

#[test]
fn does_not_reconstruct_scores_for_wrong_round_or_unscored_video_types() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-unscored\score.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-unscored",
        clip_path,
        "score.mp4",
    );
    let video = video_record("score", "击杀集锦", "击杀集锦", Some(clip_path), 8);
    let mut account = account_with_videos("1001", "match-unscored", vec![video]);
    account.matches[0].game_mode = Some("竞技模式".to_string());
    let scores = vec![
        HighlightLogRoundScore {
            account_id: "1001".to_string(),
            match_id: "match-unscored".to_string(),
            round_id: 7,
            score: 1_200,
        },
        HighlightLogRoundScore {
            account_id: "1001".to_string(),
            match_id: "another-match".to_string(),
            round_id: 7,
            score: 1_300,
        },
    ];

    ingest_wonderful_metadata_with_round_scores(&connection, &[account], &scores)
        .expect("unscored video ingest should succeed");

    let stored: (Option<i64>, Option<String>) = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER), round_score_source FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("unscored video metadata should load");
    assert_eq!(stored, (None, None));
}

#[test]
fn multiple_segment_key_collisions_receive_unique_keys() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\match-segment-collision\segments.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "match-segment-collision",
        clip_path,
        "segments.mp4",
    );
    let mut video = video_record("segments", "分段冲突", "1", Some(clip_path), 0);
    let mut first = video.segments[0].clone();
    first.segment_id = "duplicate".to_string();
    let mut occupied_suffix = first.clone();
    occupied_suffix.segment_id = "duplicate-2".to_string();
    occupied_suffix.clip_start_ms = Some(10_000);
    occupied_suffix.clip_end_ms = Some(19_000);
    let mut repeated = first.clone();
    repeated.clip_start_ms = Some(20_000);
    repeated.clip_end_ms = Some(29_000);
    video.segments = vec![first, occupied_suffix, repeated];
    let accounts = vec![account_with_videos(
        "1001",
        "match-segment-collision",
        vec![video],
    )];

    ingest_wonderful_metadata(&connection, &accounts).expect("segment ingest should succeed");

    let segment_count: i64 = connection
        .query_row(
            "SELECT COUNT(DISTINCT segment_key) FROM clip_segments WHERE clip_id = ?1",
            params![clip_id],
            |row| row.get(0),
        )
        .expect("unique segment count should load");
    assert_eq!(segment_count, 3);
}

#[test]
fn ingests_relative_and_absolute_times_and_rebuilds_killed_state_on_rescan() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let ordinary_path = r"D:\synthetic\wonderfulVideos1001\timeline-rescan\ordinary.mp4";
    let death_path = r"D:\synthetic\wonderfulVideos1001\timeline-rescan\death.mp4";
    let ordinary_clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "timeline-rescan",
        ordinary_path,
        "ordinary.mp4",
    );
    let death_clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "timeline-rescan",
        death_path,
        "death.mp4",
    );
    let initial_fixture = format!(
        r#"
        {{
          "key_wonderful_list_1001":[{{
            "matches_id":"timeline-rescan",
            "videos":[
              {{
                "video_id":"ordinary",
                "video_name":"普通高光",
                "highLightType":1,
                "video_src":"{}",
                "round_clips":[{{
                  "clip_id":"ordinary-segment",
                  "clip_sTime":1000,
                  "clip_duration":8000,
                  "clip_events":[{{
                    "event_id":"ordinary-kill",
                    "event_type":"kill",
                    "event_sTime":500,
                    "KilledIsMe":false,
                    "event_ext":{{"KillerIsMe":1}}
                  }}]
                }}]
              }},
              {{
                "video_id":"death",
                "video_name":"死亡集锦",
                "highLightType":3,
                "video_src":"{}",
                "round_clips":[{{
                  "clip_id":"death-segment",
                  "clip_sTime":3000,
                  "clip_duration":6000,
                  "clip_events":[{{
                    "event_id":"self-death",
                    "event_type":"death",
                    "event_sTime":1500,
                    "event_ext":{{"KilledIsMe":"1"}}
                  }}]
                }}]
              }}
            ]
          }}]
        }}
        "#,
        ordinary_path.replace('\\', "\\\\"),
        death_path.replace('\\', "\\\\"),
    );
    let initial = vec![WonderfulAccountRecord {
        openid: "1001".to_string(),
        matches: parse_wonderful_db_text("1001", &initial_fixture)
            .expect("real-shaped timeline fixture should parse"),
    }];

    let first_summary = ingest_wonderful_metadata(&connection, &initial)
        .expect("initial timeline ingest should succeed");
    assert_eq!(first_summary.warning_count, 0);
    let ordinary_event = db::list_clip_events_for_clip(&connection, ordinary_clip_id)
        .expect("ordinary event should load")
        .pop()
        .expect("ordinary event should exist");
    let death_event = db::list_clip_events_for_clip(&connection, death_clip_id)
        .expect("death event should load")
        .pop()
        .expect("death event should exist");
    assert_eq!(ordinary_event.video_time_ms, Some(1_500));
    assert_eq!(ordinary_event.killed_is_me, Some(false));
    assert_eq!(death_event.video_time_ms, Some(1_500));
    assert_eq!(death_event.killed_is_me, Some(true));

    let rescanned_fixture = initial_fixture
        .replace(r#""event_sTime":1500"#, r#""event_sTime":1600"#)
        .replace(r#""KilledIsMe":"1""#, r#""KilledIsMe":"0""#);
    let rescanned = vec![WonderfulAccountRecord {
        openid: "1001".to_string(),
        matches: parse_wonderful_db_text("1001", &rescanned_fixture)
            .expect("rescanned timeline fixture should parse"),
    }];
    ingest_wonderful_metadata(&connection, &rescanned)
        .expect("rescanned timeline should replace old state");

    let death_events = db::list_clip_events_for_clip(&connection, death_clip_id)
        .expect("rescanned death event should load");
    assert_eq!(death_events.len(), 1);
    assert_eq!(death_events[0].video_time_ms, Some(1_600));
    assert_eq!(death_events[0].killed_is_me, Some(false));
}

#[test]
fn reports_bounded_ingest_warnings_and_persists_unknown_flags_and_times() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\timeline-warnings\warnings.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "timeline-warnings",
        clip_path,
        "warnings.mp4",
    );
    let events = (0..25)
        .map(|index| {
            format!(
                r#"{{"event_id":"warning-{index}","event_type":"death","event_sTime":2000,"KilledIsMe":2}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let fixture = format!(
        r#"
        {{
          "key_wonderful_list_1001":[{{
            "matches_id":"timeline-warnings",
            "videos":[{{
              "video_id":"warnings",
              "video_name":"死亡集锦",
              "highLightType":3,
              "video_src":"{}",
              "round_clips":[{{
                "clip_id":"warning-segment",
                "clip_sTime":0,
                "clip_duration":1000,
                "clip_events":[{events}]
              }}]
            }}]
          }}]
        }}
        "#,
        clip_path.replace('\\', "\\\\"),
    );
    let accounts = vec![WonderfulAccountRecord {
        openid: "1001".to_string(),
        matches: parse_wonderful_db_text("1001", &fixture).expect("warning fixture should parse"),
    }];

    let summary = ingest_wonderful_metadata(&connection, &accounts)
        .expect("warning timeline should ingest without clipping");

    assert_eq!(summary.warning_count, 50);
    assert_eq!(summary.warnings.len(), 20);
    assert!(summary
        .warnings
        .iter()
        .any(|warning| warning.ends_with("invalid-top-level-killed-is-me")));
    assert!(summary
        .warnings
        .iter()
        .any(|warning| warning.ends_with("video-time-out-of-bounds")));
    let stored =
        db::list_clip_events_for_clip(&connection, clip_id).expect("warning events should load");
    assert_eq!(stored.len(), 25);
    assert!(stored
        .iter()
        .all(|event| event.video_time_ms.is_none() && event.killed_is_me == Some(false)));
}

#[test]
fn uses_indexed_duration_when_wonderful_segments_do_not_provide_one() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\timeline-indexed-duration\duration.mp4";
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "timeline-indexed-duration",
        clip_path,
        "duration.mp4",
    );
    connection
        .execute(
            "UPDATE clips SET duration_ms = 1000 WHERE id = ?1",
            params![clip_id],
        )
        .expect("indexed duration should seed");
    let fixture = format!(
        r#"
        {{
          "key_wonderful_list_1001":[{{
            "matches_id":"timeline-indexed-duration",
            "videos":[{{
              "video_id":"duration",
              "video_name":"击杀集锦",
              "highLightType":2,
              "video_src":"{}",
              "round_clips":[{{
                "clip_sTime":500,
                "clip_events":[{{
                  "event_id":"too-late",
                  "event_type":"kill",
                  "event_sTime":1500
                }}]
              }}]
            }}]
          }}]
        }}
        "#,
        clip_path.replace('\\', "\\\\"),
    );
    let accounts = vec![WonderfulAccountRecord {
        openid: "1001".to_string(),
        matches: parse_wonderful_db_text("1001", &fixture)
            .expect("duration fallback fixture should parse"),
    }];

    let summary = ingest_wonderful_metadata(&connection, &accounts)
        .expect("indexed duration should validate Wonderful events");

    assert_eq!(summary.warning_count, 1);
    assert!(summary.warnings[0].ends_with("video-time-out-of-bounds"));
    let event = db::list_clip_events_for_clip(&connection, clip_id)
        .expect("validated event should load")
        .pop()
        .expect("validated event should exist");
    assert_eq!(event.video_time_ms, None);
}

#[test]
fn fallback_event_hash_distinguishes_killed_is_me_state() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_path = r"D:\synthetic\wonderfulVideos1001\timeline-hash\hash.mp4";
    let clip_id = insert_indexed_clip(&connection, "1001", "timeline-hash", clip_path, "hash.mp4");
    let mut self_death = event_without_id("Player#1001", 1_500, 7);
    self_death.event_type = "death".to_string();
    self_death.killed_is_me = Some(true);
    let mut other_death = self_death.clone();
    other_death.killed_is_me = Some(false);
    let mut video = video_record("hash", "死亡集锦", "3", Some(clip_path), 0);
    video.segments[0].events = vec![self_death, other_death];
    let accounts = vec![account_with_videos("1001", "timeline-hash", vec![video])];

    ingest_wonderful_metadata(&connection, &accounts)
        .expect("events differing only by killed state should ingest");

    let events =
        db::list_clip_events_for_clip(&connection, clip_id).expect("hashed events should load");
    assert_eq!(events.len(), 2);
    assert_ne!(events[0].event_key, events[1].event_key);
}

#[test]
fn ingests_official_rounds_shape_with_six_kills_score_and_title() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let clip_id = insert_indexed_clip(
        &connection,
        "1001",
        "official-rounds-match",
        r"D:\synthetic\wonderfulVideos1001\official-rounds-match\official-six-kill.mp4",
        "official-six-kill.mp4",
    );
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
            {"round_id":1,"round_clips":[{"clip_id":"segment-1","clip_sTime":0,"clip_duration":1000,"clip_events":[
              {"event_id":"event-1","event_type":"kill","event_sTime":100,"event_ext":{"KillerIsMe":1}},
              {"event_id":"event-2","event_type":"kill","event_sTime":200,"event_ext":{"KillerIsMe":1}}
            ]}]},
            {"round_id":2,"round_clips":[{"clip_id":"segment-2","clip_sTime":1000,"clip_duration":1000,"clip_events":[
              {"event_id":"event-3","event_type":"kill","event_sTime":100,"event_ext":{"KillerIsMe":1}},
              {"event_id":"event-4","event_type":"kill","event_sTime":200,"event_ext":{"KillerIsMe":1}}
            ]}]},
            {"round_id":3,"round_clips":[{"clip_id":"segment-3","clip_sTime":2000,"clip_duration":1000,"clip_events":[
              {"event_id":"event-5","event_type":"kill","event_sTime":100,"event_ext":{"KillerIsMe":1}},
              {"event_id":"event-6","event_type":"kill","event_sTime":200,"event_ext":{"KillerIsMe":1}}
            ]}]}
          ]
        }]
      }]
    }
    "#;
    let accounts = vec![WonderfulAccountRecord {
        openid: "1001".to_string(),
        matches: parse_wonderful_db_text("1001", fixture)
            .expect("official rounds fixture should parse"),
    }];

    ingest_wonderful_metadata(&connection, &accounts).expect("official ingest should succeed");

    let metadata = load_official_metadata(&connection, clip_id);
    assert_eq!(metadata.kill_count, Some(6));
    assert_eq!(metadata.official_video_name.as_deref(), Some("六杀时刻"));
    assert_eq!(metadata.highlight_type, Some(10));
    let round_score: Option<i64> = connection
        .query_row(
            "SELECT CAST(round_score AS INTEGER) FROM clip_metadata WHERE clip_id = ?1",
            params![clip_id],
            |row| row.get(0),
        )
        .expect("round score should load");
    assert_eq!(round_score, Some(13));
    assert_eq!(
        db::list_clip_events_for_clip(&connection, clip_id)
            .expect("official events should load")
            .len(),
        6
    );
    assert!(assigned_tag_names(&connection, clip_id).is_empty());
}

#[test]
fn wonderful_name_overrides_lower_priority_names_account_wide_and_is_idempotent() {
    let connection = Connection::open_in_memory().expect("database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let openid = "90000000000000000006";
    let old_path = r"D:\synthetic\wonderfulVideos90000000000000000006\old-match\old.mp4";
    let new_path = r"D:\synthetic\wonderfulVideos90000000000000000006\new-match\new.mp4";
    let old_clip_id = insert_indexed_clip(&connection, openid, "old-match", old_path, "old.mp4");
    let new_clip_id = insert_indexed_clip(&connection, openid, "new-match", new_path, "new.mp4");
    let eventless_clip_id = insert_indexed_clip(
        &connection,
        openid,
        "eventless-match",
        r"D:\synthetic\wonderfulVideos90000000000000000006\eventless-match\eventless.mp4",
        "eventless.mp4",
    );
    for clip_id in [old_clip_id, new_clip_id, eventless_clip_id] {
        connection
            .execute(
                "UPDATE clip_metadata SET account_name = 'LogName#1111', player_name = 'LegacyName' WHERE clip_id = ?1",
                params![clip_id],
            )
            .expect("lower-priority names should seed");
    }
    db::update_clip_favorite(&connection, old_clip_id, true).expect("favorite should seed");
    db::update_clip_note(&connection, old_clip_id, Some("保留备注")).expect("note should seed");
    let custom_tag =
        db::create_tag(&connection, "复盘", Some("blue")).expect("custom tag should exist");
    db::assign_tag_to_clip(&connection, old_clip_id, custom_tag.id)
        .expect("custom tag should assign");
    connection
        .execute(
            "INSERT INTO matches (game_id, account_id, player_name) VALUES ('old-match', ?1, 'LegacyName'), ('new-match', ?1, 'LegacyName')",
            params![openid],
        )
        .expect("matches should seed");

    let mut old_video = video_record("old", "旧视频", "1", Some(old_path), 1);
    set_video_player_name(&mut old_video, Some("OldOfficial#1001"));
    let mut new_video = video_record("new", "新视频", "1", Some(new_path), 1);
    set_video_player_name(&mut new_video, Some("NewOfficial#2002"));
    let account = WonderfulAccountRecord {
        openid: openid.to_string(),
        matches: vec![
            match_record("old-match", "2026-07-01T12:00:00Z", old_video),
            match_record("new-match", "2026-07-04T12:00:00Z", new_video),
        ],
    };

    ingest_wonderful_metadata(&connection, std::slice::from_ref(&account))
        .expect("first ingest should succeed");
    ingest_wonderful_metadata(&connection, &[account]).expect("second ingest should succeed");

    for clip_id in [old_clip_id, new_clip_id, eventless_clip_id] {
        let names: (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT account_name, player_name FROM clip_metadata WHERE clip_id = ?1",
                params![clip_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("propagated names should load");
        assert_eq!(names.0.as_deref(), Some("NewOfficial#2002"));
        assert_eq!(names.1.as_deref(), Some("NewOfficial#2002"));
    }
    let updated_match_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM matches WHERE account_id = ?1 AND player_name = 'NewOfficial#2002'",
            params![openid],
            |row| row.get(0),
        )
        .expect("match names should load");
    assert_eq!(updated_match_count, 2);
    let old_clip = db::find_clip_by_id(&connection, old_clip_id).expect("clip should reload");
    assert_eq!(old_clip.id, old_clip_id);
    assert!(old_clip.favorite);
    assert_eq!(old_clip.note.as_deref(), Some("保留备注"));
    assert!(assigned_tag_names(&connection, old_clip_id).contains(&"复盘".to_string()));
    let indexed_clip_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM clips", [], |row| row.get(0))
        .expect("clip count should load");
    assert_eq!(indexed_clip_count, 3);
}

#[test]
fn invalid_or_missing_wonderful_names_preserve_existing_fallbacks() {
    for (openid, player_name) in [
        ("90000000000000000006", Some("Cards/card.png#1001")),
        ("undefined", Some("OfficialLooking#1001")),
        ("90000000000000000007", None),
    ] {
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let match_id = "fallback-match";
        let path = format!(r"D:\synthetic\wonderfulVideos{openid}\{match_id}\clip.mp4");
        let clip_id = insert_indexed_clip(&connection, openid, match_id, &path, "clip.mp4");
        connection
            .execute(
                "UPDATE clip_metadata SET account_name = 'Existing#1111', player_name = 'Existing#1111' WHERE clip_id = ?1",
                params![clip_id],
            )
            .expect("fallback should seed");
        let mut video = video_record("clip", "测试", "1", Some(&path), 1);
        set_video_player_name(&mut video, player_name);
        let accounts = vec![account_with_videos(openid, match_id, vec![video])];

        ingest_wonderful_metadata(&connection, &accounts).expect("ingest should succeed");

        let names: (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT account_name, player_name FROM clip_metadata WHERE clip_id = ?1",
                params![clip_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("fallback names should load");
        assert_eq!(names.0.as_deref(), Some("Existing#1111"));
        assert_eq!(names.1.as_deref(), Some("Existing#1111"));
    }
}

fn insert_indexed_clip(
    connection: &Connection,
    openid: &str,
    match_id: &str,
    video_path: &str,
    file_name: &str,
) -> i64 {
    let source_path = format!(r"D:\synthetic\wonderfulVideos{openid}");
    let source_name = format!("wonderfulVideos{openid}");
    let source = db::upsert_source_dir(
        connection,
        SourceDirInput {
            path: &source_path,
            name: &source_name,
        },
    )
    .expect("source should upsert");
    let group = db::upsert_clip_group(
        connection,
        ClipGroupInput {
            source_dir_id: source.id,
            group_key: match_id,
            display_name: match_id,
        },
    )
    .expect("group should upsert");
    db::upsert_clip(
        connection,
        ClipInput {
            source_dir_id: source.id,
            clip_group_id: Some(group.id),
            video_path,
            file_name,
            file_size: 42,
            modified_at: Some("1782634272"),
            duration_ms: None,
            recorded_at: None,
            cover_path: None,
            cover_source: "missing",
        },
    )
    .expect("clip should upsert")
    .id
}

fn account_with_videos(
    openid: &str,
    match_id: &str,
    videos: Vec<WonderfulVideoRecord>,
) -> WonderfulAccountRecord {
    WonderfulAccountRecord {
        openid: openid.to_string(),
        matches: vec![WonderfulMatchRecord {
            match_id: match_id.to_string(),
            match_time: Some("2026-07-04T12:00:00Z".to_string()),
            map_name: Some("隐世修所".to_string()),
            career: None,
            videos,
            ..WonderfulMatchRecord::default()
        }],
    }
}

fn set_video_player_name(video: &mut WonderfulVideoRecord, player_name: Option<&str>) {
    for segment in &mut video.segments {
        for event in &mut segment.events {
            event.player_name = player_name.map(str::to_string);
        }
    }
}

fn match_record(
    match_id: &str,
    match_time: &str,
    video: WonderfulVideoRecord,
) -> WonderfulMatchRecord {
    WonderfulMatchRecord {
        match_id: match_id.to_string(),
        match_time: Some(match_time.to_string()),
        map_name: Some("隐世修所".to_string()),
        career: None,
        videos: vec![video],
        ..WonderfulMatchRecord::default()
    }
}

fn video_record(
    video_id: &str,
    video_name: &str,
    video_type: &str,
    video_src: Option<&str>,
    kill_count: usize,
) -> WonderfulVideoRecord {
    WonderfulVideoRecord {
        video_id: video_id.to_string(),
        video_name: video_name.to_string(),
        video_type: video_type.to_string(),
        highlight_type: video_type.parse().ok(),
        video_src: video_src.map(str::to_string),
        round_score: None,
        segments: vec![WonderfulSegmentRecord {
            segment_id: format!("{video_id}-segment"),
            round_id: Some(7),
            clip_start_ms: Some(1_000),
            clip_end_ms: Some(9_000),
            events: (0..kill_count)
                .map(|index| WonderfulEventRecord {
                    event_id: format!("{video_id}-event-{index}"),
                    event_type: "kill".to_string(),
                    video_time_ms: Some(1_500 + index as i64 * 500),
                    event_time: Some(format!("2026-07-04T12:00:{index:02}Z")),
                    round_id: Some(7),
                    player_name: Some("Player#1001".to_string()),
                    agent_name: Some("Jett".to_string()),
                    weapon_name: Some("Vandal".to_string()),
                    killer_name: Some("Player#1001".to_string()),
                    killed_name: Some(format!("Opponent#{index:04}")),
                    killer_is_me: true,
                    killed_is_me: None,
                    normalization_warnings: Vec::new(),
                    raw_json: format!(r#"{{"video":"{video_id}","event":{index}}}"#),
                })
                .collect(),
        }],
    }
}

fn priority_log_record(clip_path: &str, kill_count: usize) -> HighlightLogRecord {
    HighlightLogRecord {
        line_kind: HighlightLogLineKind::TemplateParam,
        match_id: Some("match-priority".to_string()),
        battle_id: Some("battle-priority".to_string()),
        record_src: Some(clip_path.to_string()),
        player_name: Some("Player#1001".to_string()),
        map_id: None,
        map_name: Some("隐世修所".to_string()),
        game_mode: Some("竞技模式".to_string()),
        agent_name: Some("Jett".to_string()),
        kda: Some("1/1/1".to_string()),
        scoreline: Some("1/1".to_string()),
        has_won: Some(false),
        combat_score: Some(1),
        kill_events: (0..kill_count)
            .map(|index| HighlightLogKillEvent {
                event_time: Some(format!("2026-07-04T12:01:{index:02}Z")),
                round_id: Some(index as i64 + 1),
                weapon_name: Some("Classic".to_string()),
                killer_name: Some("Player#1001".to_string()),
                killed_name: Some(format!("Opponent#{index:04}")),
                raw_json: Some(format!(r#"{{"source":"log","event":{index}}}"#)),
            })
            .collect(),
        has_gzip_event: false,
        raw_json: r#"{"match_id":"match-priority"}"#.to_string(),
    }
}

fn event_without_id(killed_name: &str, video_time_ms: i64, round_id: i64) -> WonderfulEventRecord {
    WonderfulEventRecord {
        event_id: String::new(),
        event_type: "kill".to_string(),
        video_time_ms: Some(video_time_ms),
        event_time: Some(format!("2026-07-04T12:00:{round_id:02}Z")),
        round_id: Some(round_id),
        player_name: Some("Player#1001".to_string()),
        agent_name: Some("Jett".to_string()),
        weapon_name: Some("Vandal".to_string()),
        killer_name: Some("Player#1001".to_string()),
        killed_name: Some(killed_name.to_string()),
        killer_is_me: true,
        killed_is_me: None,
        normalization_warnings: Vec::new(),
        raw_json: format!(r#"{{"killed":"{killed_name}"}}"#),
    }
}

fn event_keys_by_victim(
    connection: &Connection,
    clip_id: i64,
) -> std::collections::HashMap<String, String> {
    db::list_clip_events_for_clip(connection, clip_id)
        .expect("events should load")
        .into_iter()
        .map(|event| {
            (
                event.killed_name.expect("synthetic victim should exist"),
                event.event_key,
            )
        })
        .collect()
}

fn load_official_metadata(connection: &Connection, clip_id: i64) -> OfficialMetadata {
    connection
        .query_row(
            "
            SELECT
                kill_count,
                official_video_name,
                official_video_id,
                official_video_type,
                CAST(highlight_type AS INTEGER),
                metadata_source
            FROM clip_metadata
            WHERE clip_id = ?1
            ",
            params![clip_id],
            |row| {
                Ok(OfficialMetadata {
                    kill_count: row.get(0)?,
                    official_video_name: row.get(1)?,
                    official_video_id: row.get(2)?,
                    official_video_type: row.get(3)?,
                    highlight_type: row.get(4)?,
                    metadata_source: row.get(5)?,
                })
            },
        )
        .expect("official metadata should load")
}

fn assigned_tag_names(connection: &Connection, clip_id: i64) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "
            SELECT tags.name
            FROM tags
            JOIN clip_tags ON clip_tags.tag_id = tags.id
            WHERE clip_tags.clip_id = ?1
            ORDER BY tags.name
            ",
        )
        .expect("tag query should prepare");
    statement
        .query_map(params![clip_id], |row| row.get::<_, String>(0))
        .expect("tag query should run")
        .collect::<Result<Vec<_>, _>>()
        .expect("tag names should load")
}
