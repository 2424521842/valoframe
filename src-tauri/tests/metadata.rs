use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use valorant_highlight_manager_lib::metadata::{
    parse_video_export_configs, DetectedVideoType, ParseStatus,
};

#[test]
fn parses_video_export_configs_by_recursively_collecting_strings() {
    let fixture = TestFixture::new("metadata-strings");
    let source_dir = fixture.path();
    let export_tmp = source_dir.join("videoExportTmp");
    fs::create_dir_all(&export_tmp).expect("export tmp should be created");
    fs::write(
        export_tmp.join("config-ace.json"),
        r#"{
            "unused": 42,
            "timeline": {
                "title": "ACE MVP 三杀 四杀 18/7/3",
                "details": ["地图：源工重镇", "模式：竞技模式", {"player": "玩家昵称：TenZ"}]
            }
        }"#,
    )
    .expect("config should be written");
    fs::write(export_tmp.join("ignored.json"), r#"{"title":"ACE"}"#)
        .expect("ignored json should be written");

    let configs = parse_video_export_configs(source_dir).expect("configs should parse");

    assert_eq!(configs.len(), 1);
    let config = &configs[0];
    assert!(config.json_path.ends_with("videoExportTmp/config-ace.json"));
    assert!(config.extracted_text.contains("ACE MVP 三杀 四杀 18/7/3"));
    assert!(config.extracted_text.contains("地图：源工重镇"));
    assert_eq!(config.parse_status, ParseStatus::Parsed);
    assert!(config.is_mvp);
    assert!(config.is_triple_kill);
    assert!(config.is_quadra_kill);
    assert!(config.is_ace);
    assert_eq!(config.kda.as_deref(), Some("18/7/3"));
    assert_eq!(config.map_name.as_deref(), Some("源工重镇"));
    assert_eq!(config.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(config.agent_name, None);
    assert_eq!(config.player_name.as_deref(), Some("TenZ"));
}

#[test]
fn recognizes_explicit_five_and_six_kill_export_text_without_count_inference() {
    let fixture = TestFixture::new("metadata-five-six-kill-text");
    let export_tmp = fixture.path().join("videoExportTmp");
    fs::create_dir_all(&export_tmp).expect("export tmp should be created");

    for (name, title) in [
        ("five-cn", "五杀"),
        ("five-digit", "5杀"),
        ("five-streak", "五连杀"),
        ("six-cn", "六杀"),
        ("six-digit", "6杀"),
        ("six-streak", "六连杀"),
        ("six-punctuation", "六杀）"),
    ] {
        fs::write(
            export_tmp.join(format!("config-{name}.json")),
            format!(r#"{{"title":"{title}"}}"#),
        )
        .expect("config should be written");
    }
    fs::write(
        export_tmp.join("config-count-only.json"),
        r#"{"summary":"match event count: 6"}"#,
    )
    .expect("count-only config should be written");
    for (name, title) in [
        ("five-killer", "五杀手"),
        ("six-killer", "六杀手"),
        ("six-insecticide", "6杀虫剂"),
    ] {
        fs::write(
            export_tmp.join(format!("config-{name}.json")),
            format!(r#"{{"title":"{title}"}}"#),
        )
        .expect("negative config should be written");
    }

    let configs = parse_video_export_configs(fixture.path()).expect("configs should parse");

    for name in ["five-cn", "five-digit", "five-streak"] {
        let config = configs
            .iter()
            .find(|config| config.json_path.ends_with(&format!("config-{name}.json")))
            .expect("five-kill config should exist");
        assert_eq!(config.parse_status, ParseStatus::Parsed);
        assert_eq!(config.detected_video_type(), Some(DetectedVideoType::Five));
    }
    for name in ["six-cn", "six-digit", "six-streak", "six-punctuation"] {
        let config = configs
            .iter()
            .find(|config| config.json_path.ends_with(&format!("config-{name}.json")))
            .expect("six-kill config should exist");
        assert_eq!(config.parse_status, ParseStatus::Parsed);
        assert_eq!(config.detected_video_type(), Some(DetectedVideoType::Six));
    }
    let count_only = configs
        .iter()
        .find(|config| config.json_path.ends_with("config-count-only.json"))
        .expect("count-only config should exist");
    assert_eq!(count_only.detected_video_type(), None);
    for name in ["five-killer", "six-killer", "six-insecticide"] {
        let config = configs
            .iter()
            .find(|config| config.json_path.ends_with(&format!("config-{name}.json")))
            .expect("negative config should exist");
        assert_eq!(config.detected_video_type(), None);
    }
}

#[test]
fn recognizes_compilation_and_death_video_types_from_export_text() {
    let fixture = TestFixture::new("metadata-compilation-death");
    let export_tmp = fixture.path().join("videoExportTmp");
    fs::create_dir_all(&export_tmp).expect("export tmp should be created");
    fs::write(
        export_tmp.join("config-compilation.json"),
        r#"{"title":"击杀集锦"}"#,
    )
    .expect("compilation config should be written");
    fs::write(
        export_tmp.join("config-death.json"),
        r#"{"title":"死亡时刻"}"#,
    )
    .expect("death config should be written");

    let configs = parse_video_export_configs(fixture.path()).expect("configs should parse");
    let compilation = configs
        .iter()
        .find(|config| config.json_path.ends_with("config-compilation.json"))
        .expect("compilation config should exist");
    let death = configs
        .iter()
        .find(|config| config.json_path.ends_with("config-death.json"))
        .expect("death config should exist");

    assert_eq!(
        compilation.detected_video_type(),
        Some(DetectedVideoType::KillCompilation)
    );
    assert_eq!(death.detected_video_type(), Some(DetectedVideoType::Death));
}

#[test]
fn parses_labeled_object_values_and_preserves_player_tag_number() {
    let fixture = TestFixture::new("metadata-player-tag");
    let source_dir = fixture.path();
    let export_tmp = source_dir.join("videoExportTmp");
    fs::create_dir_all(&export_tmp).expect("export tmp should be created");
    fs::write(
        export_tmp.join("config-player.json"),
        r#"{
            "玩家昵称": "FixtureAlpha#0001",
            "地图": "天枢之阙",
            "游戏模式": "竞技模式",
            "KDA": "36/17/6"
        }"#,
    )
    .expect("config should be written");

    let configs = parse_video_export_configs(source_dir).expect("configs should parse");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].player_name.as_deref(), Some("FixtureAlpha#0001"));
    assert_eq!(configs[0].map_name.as_deref(), Some("天枢之阙"));
    assert_eq!(configs[0].game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(configs[0].kda.as_deref(), Some("36/17/6"));
}

#[test]
fn repairs_mojibake_values_from_video_export_templates() {
    let fixture = TestFixture::new("metadata-mojibake");
    let source_dir = fixture.path();
    let export_tmp = source_dir.join("videoExportTmp");
    fs::create_dir_all(&export_tmp).expect("export tmp should be created");
    fs::write(
        export_tmp.join("config-mojibake.json"),
        r#"{
            "template": {
                "texts": [
                    {"value": "32/20/14"},
                    {"value": "ç¤ºä¾‹çŽ©å®¶#0003"},
                    {"value": "æ·±æµ·æ˜Žç "},
                    {"value": "ç«žæŠ€æ¨¡å¼"}
                ]
            }
        }"#,
    )
    .expect("config should be written");

    let configs = parse_video_export_configs(source_dir).expect("configs should parse");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].player_name.as_deref(), Some("示例玩家#0003"));
    assert_eq!(configs[0].map_name.as_deref(), Some("深海明珠"));
    assert_eq!(configs[0].game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(configs[0].kda.as_deref(), Some("32/20/14"));
}

#[test]
fn ignores_player_card_asset_paths_when_finding_player_name() {
    let fixture = TestFixture::new("metadata-player-card");
    let source_dir = fixture.path();
    let export_tmp = source_dir.join("videoExportTmp");
    fs::create_dir_all(&export_tmp).expect("export tmp should be created");
    fs::write(
        export_tmp.join("config-card.json"),
        r#"{
            "player_card": "Cards/D3018FBE-45CD-786A-DD6C-BCAF429F7096.png",
            "地图": "隐世修所",
            "游戏模式": "竞技模式",
            "KDA": "27/22/6"
        }"#,
    )
    .expect("config should be written");

    let configs = parse_video_export_configs(source_dir).expect("configs should parse");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].player_name, None);
    assert_eq!(configs[0].map_name.as_deref(), Some("隐世修所"));
    assert_eq!(configs[0].game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(configs[0].kda.as_deref(), Some("27/22/6"));
}

#[test]
fn prefers_riot_id_text_over_player_card_urls() {
    let fixture = TestFixture::new("metadata-player-card-with-riot-id");
    let source_dir = fixture.path();
    let export_tmp = source_dir.join("videoExportTmp");
    fs::create_dir_all(&export_tmp).expect("export tmp should be created");
    fs::write(
        export_tmp.join("config-card-riot-id.json"),
        r#"{
            "assets": [
                {
                    "url": "https://game.gtimg.cn/images/val/agamezlk/PlayerCards/D3018FBE-45CD-786A-DD6C-BCAF429F7096.png",
                    "value": "Cards/D3018FBE-45CD-786A-DD6C-BCAF429F7096.png"
                },
                {"value": "FixtureBravo#0002"},
                {"url": "https://game.gtimg.cn/images/val/agamezlk/agentbackground/agent/11.png"},
                {"value": "隐世修所"},
                {"value": "竞技模式"},
                {"value": "27/22/6"}
            ]
        }"#,
    )
    .expect("config should be written");

    let configs = parse_video_export_configs(source_dir).expect("configs should parse");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].player_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(configs[0].agent_name.as_deref(), Some("Reyna"));
    assert_eq!(configs[0].map_name.as_deref(), Some("隐世修所"));
    assert_eq!(configs[0].game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(configs[0].kda.as_deref(), Some("27/22/6"));
}

#[test]
fn marks_unrecognized_valid_json_as_partial_and_invalid_json_as_failed() {
    let fixture = TestFixture::new("metadata-statuses");
    let source_dir = fixture.path();
    let export_tmp = source_dir.join("videoExportTmp");
    fs::create_dir_all(&export_tmp).expect("export tmp should be created");
    fs::write(
        export_tmp.join("config-empty.json"),
        r#"{"notes":["only a plain note"],"nested":{"value":"still readable"}}"#,
    )
    .expect("partial config should be written");
    fs::write(
        export_tmp.join("config-broken.json"),
        r#"{"notes":["oops"]"#,
    )
    .expect("broken config should be written");

    let configs = parse_video_export_configs(source_dir).expect("configs should parse");

    assert_eq!(configs.len(), 2);
    let partial = configs
        .iter()
        .find(|config| config.json_path.ends_with("config-empty.json"))
        .expect("partial config should exist");
    assert_eq!(partial.parse_status, ParseStatus::Partial);
    assert!(partial.extracted_text.contains("only a plain note"));
    assert!(partial.extracted_text.contains("still readable"));

    let failed = configs
        .iter()
        .find(|config| config.json_path.ends_with("config-broken.json"))
        .expect("failed config should exist");
    assert_eq!(failed.parse_status, ParseStatus::Failed);
    assert!(failed.extracted_text.is_empty());
}

#[test]
fn missing_video_export_tmp_returns_an_empty_result() {
    let fixture = TestFixture::new("metadata-missing-dir");

    let configs = parse_video_export_configs(fixture.path()).expect("missing dir should not fail");

    assert!(configs.is_empty());
}

struct TestFixture {
    root: PathBuf,
}

impl TestFixture {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("vhm-{label}-{unique}"));
        fs::create_dir_all(&root).expect("fixture root should be created");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
