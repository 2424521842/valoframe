// Privacy note: account IDs, player names, match IDs, and paths in this file are synthetic fixtures.
use std::path::PathBuf;

use base64::{engine::general_purpose, Engine as _};
use flate2::{write::GzEncoder, Compression};
use std::io::Write;
use valorant_highlight_manager_lib::highlight_log_parser::{
    parse_highlight_log, parse_highlight_log_content, parse_highlight_logs, HighlightLogLineKind,
    HighlightLogRoundScore,
};

#[test]
fn parses_first_request_data_and_template_param_lines() {
    let result = parse_highlight_log(fixture_path()).expect("fixture should parse");

    assert_eq!(result.records.len(), 3);
    assert_eq!(result.bad_line_count, 1);
    assert_eq!(result.gzip_event_count, 1);

    let first_request = &result.records[0];
    assert_eq!(
        first_request.line_kind,
        HighlightLogLineKind::FirstRequestData
    );
    assert_eq!(first_request.match_id.as_deref(), Some("match-a-001"));
    assert_eq!(first_request.battle_id.as_deref(), Some("battle-a-001"));
    assert_eq!(
        first_request.record_src.as_deref(),
        Some("D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-a-001")
    );
    assert_eq!(first_request.player_name.as_deref(), Some("PlayerOne#0000"));
    assert_eq!(first_request.map_id.as_deref(), Some("maps/ascent"));
    assert_eq!(first_request.map_name.as_deref(), Some("亚海悬城"));
    assert_eq!(first_request.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(first_request.agent_name.as_deref(), Some("Jett"));
    assert_eq!(first_request.kda.as_deref(), Some("18/7/3"));
    assert_eq!(first_request.scoreline.as_deref(), Some("13/11"));
    assert_eq!(first_request.has_won, Some(true));
    assert_eq!(first_request.combat_score, Some(287));
    assert_eq!(first_request.kill_events.len(), 1);
    assert_eq!(
        first_request.kill_events[0].event_time.as_deref(),
        Some("2026-07-01T10:00:31Z")
    );
    assert_eq!(first_request.kill_events[0].round_id, Some(3));
    assert_eq!(
        first_request.kill_events[0].weapon_name.as_deref(),
        Some("Vandal")
    );

    let template = &result.records[1];
    assert_eq!(template.line_kind, HighlightLogLineKind::TemplateParam);
    assert_eq!(template.match_id.as_deref(), Some("match-b-002"));
    assert_eq!(template.player_name.as_deref(), Some("PlayerTwo#0000"));
    assert_eq!(template.map_id.as_deref(), Some("maps/haven"));
    assert_eq!(template.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(template.game_mode.as_deref(), Some("未评级"));
    assert_eq!(template.agent_name.as_deref(), Some("Sage"));
    assert_eq!(template.kda.as_deref(), Some("12/8/9"));
    assert_eq!(template.scoreline.as_deref(), Some("13/9"));
    assert_eq!(template.has_won, Some(true));
    assert_eq!(template.combat_score, Some(221));
    assert_eq!(template.kill_events.len(), 2);
    assert_eq!(
        template.kill_events[1].event_time.as_deref(),
        Some("2026-07-02T11:05:12Z")
    );

    let gzip_record = &result.records[2];
    assert_eq!(gzip_record.match_id.as_deref(), Some("match-c-003"));
    assert!(gzip_record.has_gzip_event);
    assert!(gzip_record.kill_events.is_empty());
}

#[test]
fn skips_unrelated_and_bad_lines_without_failing() {
    let content = r#"
        harmless line
        first request data is [{"matchId":"match-ok","kills":4,"deaths":2,"assists":1}]
        first request data is [{"matchId":
        template param == {"match_id":"match-template","event":"H4sIA-synthetic"}
    "#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 2);
    assert_eq!(result.bad_line_count, 1);
    assert_eq!(result.gzip_event_count, 1);
    assert_eq!(result.records[0].kda.as_deref(), Some("4/2/1"));
    assert!(result.records[1].has_gzip_event);
}

#[test]
fn parses_rotated_highlight_logs_from_logs_directory() {
    let fixture = TestFixture::new("highlight-logs-dir");
    std::fs::write(
        fixture.path().join("highlight.log"),
        r#"first request data is {"matchId":"current-match","kills":4,"deaths":2,"assists":1}"#,
    )
    .expect("current log should be written");
    std::fs::write(
        fixture.path().join("highlight.old.log"),
        r#"first request data is {"matchId":"old-match","kills":24,"deaths":13,"assists":7}"#,
    )
    .expect("old log should be written");

    let result = parse_highlight_logs(fixture.path()).expect("logs should parse");

    let match_ids = result
        .records
        .iter()
        .filter_map(|record| record.match_id.as_deref())
        .collect::<Vec<_>>();
    assert!(match_ids.contains(&"current-match"));
    assert!(match_ids.contains(&"old-match"));
}

#[test]
fn combines_game_name_and_tag_line_into_riot_id() {
    let content = r##"
        first request data is {"matchId":"match-riot","GameName":"FixtureAlpha","TagLine":"0001"}
        template param == {"matchId":"match-lower","gameName":"FixtureBravo","tagLine":"0002"}
    "##;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 2);
    assert_eq!(
        result.records[0].player_name.as_deref(),
        Some("FixtureAlpha#0001")
    );
    assert_eq!(
        result.records[1].player_name.as_deref(),
        Some("FixtureBravo#0002")
    );
}

#[test]
fn normalizes_internal_map_mode_and_agent_paths_for_display() {
    let content = r#"first request data is {"matchId":"match-paths","mapName":"/Game/Maps/Ascent/Ascent","gameMode":"/Game/GameModes/Bomb/BombGameMode.BombGameMode_C","agentName":"/Game/Characters/Jett/Jett_PrimaryAsset.Jett_PrimaryAsset_C"}"#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].map_name.as_deref(), Some("亚海悬城"));
    assert_eq!(result.records[0].game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(result.records[0].agent_name.as_deref(), Some("Jett"));
}

#[test]
fn maps_valorant_internal_map_and_mode_names_to_display_names() {
    let content = r#"
        first request data is {"matchId":"match-haven","mapName":"Triad","gameMode":"BombGameMode"}
        first request data is {"matchId":"match-lotus","map_id":"/Game/Maps/Jam/Jam","modeName":"Swiftplay"}
        first request data is {"matchId":"match-sunset","mapName":"/Game/Maps/Juliett/Juliett","gameMode":"/Game/GameModes/Bomb/competitive"}
        first request data is {"matchId":"match-tdm","mapName":"Bonsai","gameMode":"SkirmishGameMode"}
    "#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 4);
    assert_eq!(result.records[0].map_name.as_deref(), Some("隐世修所"));
    assert_eq!(result.records[0].game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(result.records[1].map_name.as_deref(), Some("莲华古城"));
    assert_eq!(result.records[1].game_mode.as_deref(), Some("极速模式"));
    assert_eq!(result.records[2].map_name.as_deref(), Some("日落之城"));
    assert_eq!(result.records[2].game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(result.records[3].map_name.as_deref(), Some("霓虹町"));
    assert_eq!(result.records[3].game_mode.as_deref(), Some("团队乱斗"));
}

#[test]
fn skips_player_like_values_when_finding_agent_name() {
    let content = r##"
        first request data is {"matchId":"match-agent","career":{"heroName":"FixtureBravo #0002"},"event_ext":{"AgentName":"Clove"}}
    "##;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].agent_name.as_deref(), Some("Clove"));
}

#[test]
fn parses_current_miks_agent_name() {
    let content = r#"first request data is {"matchId":"match-miks-agent","agentName":"miks"}"#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].agent_name.as_deref(), Some("Miks"));
}

#[test]
fn skips_unknown_agent_names_when_finding_agent_name() {
    let content =
        r#"first request data is {"matchId":"match-unknown-agent","agentName":"future-agent"}"#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].agent_name, None);
}

#[test]
fn parses_gzip_event_parser_payload_for_match_context() {
    let event_payload = "H4sIAMPaR2oC/41SbWuDMBD+L9lXXaKFOfw23Avd1k5q2cA5SqYHyqKRJApj9L/vkom0X4YhJHf3PPeSu7yTn4KkXPFWFyRG+W6Ezmx5C6gWZCeHrsoMV6YgHhIF/wY1o3/qSwcXDJdjPCC2vnVoy01Z+6XP2MpBG95PCLUsirqmN7rEfNPlaPeDEBtZwbo64drDGjVNZNuDaUwzwqnsXHdSwFxcVktpQDn3RHbGpklqfGmJVk0fwRiaotRoCz3DCOLSGg9pckhcONeKfTPFC1l45bPIZ8E+YDGzOy/I8Ui8/1v41AiRGQX8a0nQVZA7mvXaDi2SQtTegPeym0O+8q7iYkly7NAnN1kpFSzKHubTqHB0Z75xeB150384B6IQy/j4BS5wndNIAgAA";
    let content = format!(
        r#"now event parser params is: {{"games":[{{"event":"{event_payload}","videoList":[{{"path":"D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-c-003/ace.mp4"}}]}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    assert_eq!(result.gzip_event_count, 1);
    let record = &result.records[0];
    assert_eq!(record.line_kind, HighlightLogLineKind::EventParser);
    assert_eq!(record.match_id.as_deref(), Some("match-c-003"));
    assert_eq!(
        record.record_src.as_deref(),
        Some("D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-c-003/ace.mp4")
    );
    assert_eq!(record.player_name.as_deref(), Some("PlayerOne#0000"));
    assert_eq!(record.map_id.as_deref(), Some("/Game/Maps/Ascent/Ascent"));
    assert_eq!(record.map_name.as_deref(), Some("亚海悬城"));
    assert_eq!(record.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(record.agent_name.as_deref(), Some("Jett"));
    assert_eq!(record.combat_score, Some(287));
    assert_eq!(record.kill_events.len(), 1);
    assert_eq!(
        record.kill_events[0].event_time.as_deref(),
        Some("2026-07-01T10:00:31Z")
    );
    assert_eq!(record.kill_events[0].weapon_name.as_deref(), Some("Vandal"));
}

#[test]
fn reconstructs_round_scores_from_current_players_cumulative_combat_score() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000001","PlayerName":"Current#NA1","GameID":"match-round-scores"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Enemy","MatchCombatScore":100,"RoundCombatScore":999,"GameID":"match-round-scores"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"current","MatchCombatScore":250,"RoundCombatScore":0,"GameID":"match-round-scores"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000001","RoundID":0}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"CURRENT","MatchCombatScore":500,"RoundCombatScore":0,"GameID":"match-round-scores"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":670,"RoundCombatScore":0,"GameID":"match-round-scores"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000001","RoundID":1}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000001","RoundID":2}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":945,"RoundCombatScore":0,"GameID":"match-round-scores"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000001","RoundID":3}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000001","PlayerName":"Current","GameID":"match-round-scores","TotalScore":945}}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"match-round-scores","user_name":"Current","user_nick_id":"NA1","event":"{event_payload}"}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(
        result.round_scores,
        vec![
            HighlightLogRoundScore {
                account_id: "10000001".to_string(),
                match_id: "match-round-scores".to_string(),
                round_id: 0,
                score: 250,
            },
            HighlightLogRoundScore {
                account_id: "10000001".to_string(),
                match_id: "match-round-scores".to_string(),
                round_id: 1,
                score: 420,
            },
            HighlightLogRoundScore {
                account_id: "10000001".to_string(),
                match_id: "match-round-scores".to_string(),
                round_id: 2,
                score: 0,
            },
            HighlightLogRoundScore {
                account_id: "10000001".to_string(),
                match_id: "match-round-scores".to_string(),
                round_id: 3,
                score: 275,
            },
        ]
    );
}

#[test]
fn reconstructs_only_observed_contiguous_rounds_when_log_tail_has_no_round_end() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000002","GameID":"match-missing-tail-round-end"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"match-missing-tail-round-end"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000002","RoundID":0}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":500,"GameID":"match-missing-tail-round-end"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000002","RoundID":1}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":900,"GameID":"match-missing-tail-round-end"}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000002","GameID":"match-missing-tail-round-end","TotalScore":900}}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"match-missing-tail-round-end","user_name":"Current","event":"{event_payload}"}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(
        result.round_scores,
        vec![
            HighlightLogRoundScore {
                account_id: "10000002".to_string(),
                match_id: "match-missing-tail-round-end".to_string(),
                round_id: 0,
                score: 250,
            },
            HighlightLogRoundScore {
                account_id: "10000002".to_string(),
                match_id: "match-missing-tail-round-end".to_string(),
                round_id: 1,
                score: 250,
            },
        ]
    );
}

#[test]
fn skips_round_scores_when_current_player_cannot_be_identified_uniquely() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000003","GameID":"match-ambiguous-score"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"PlayerA","MatchCombatScore":250,"GameID":"match-ambiguous-score"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"PlayerB","MatchCombatScore":250,"GameID":"match-ambiguous-score"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000003","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000003","GameID":"match-ambiguous-score","TotalScore":250}}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"match-ambiguous-score","user_name":"PlayerA","event":"{event_payload}"}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert!(result.round_scores.is_empty());
}

#[test]
fn reconstructs_round_scores_without_player_name_context_when_total_is_unique() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000004","GameID":"match-no-player-identity"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"match-no-player-identity"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000004","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000004","GameID":"match-no-player-identity","TotalScore":250}}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"match-no-player-identity","event":"{event_payload}"}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(
        result.round_scores,
        vec![HighlightLogRoundScore {
            account_id: "10000004".to_string(),
            match_id: "match-no-player-identity".to_string(),
            round_id: 0,
            score: 250,
        }]
    );
}

#[test]
fn combat_score_uses_exact_double_underscore_player_name_before_account_name() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000015","GameID":"exact-player-key"}}"#,
        r#"{"Params":{"EventName":"CombatScore","PlayerName":"Account#TAG","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"exact-player-key"}}"#,
        r#"{"Params":{"EventName":"CombatScore","PlayerName":"Account#TAG","__PlayerName__":"Enemy","MatchCombatScore":100,"GameID":"exact-player-key"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000015","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000015","GameID":"exact-player-key","TotalScore":250}}"#,
    ]);
    let content =
        format!(r#"now event parser params is: {{"games":[{{"event":"{event_payload}"}}]}}"#);

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(
        result.round_scores,
        vec![HighlightLogRoundScore {
            account_id: "10000015".to_string(),
            match_id: "exact-player-key".to_string(),
            round_id: 0,
            score: 250,
        }]
    );
}

#[test]
fn skips_round_scores_when_cumulative_score_moves_backwards() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000005","GameID":"match-backwards-score"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"match-backwards-score"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000005","RoundID":0}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":200,"GameID":"match-backwards-score"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000005","RoundID":1}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000005","GameID":"match-backwards-score","TotalScore":200}}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"match-backwards-score","user_name":"Current","event":"{event_payload}"}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert!(result.round_scores.is_empty());
}

#[test]
fn conflicting_round_scores_across_log_files_are_discarded() {
    let fixture = TestFixture::new("conflicting-round-scores");
    let first_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000006","GameID":"match-conflicting-score"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"match-conflicting-score"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000006","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000006","GameID":"match-conflicting-score","TotalScore":250}}"#,
    ]);
    let second_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":" 10000006 ","GameID":" MATCH-CONFLICTING-SCORE "}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":300,"GameID":"match-conflicting-score"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000006","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000006","GameID":"match-conflicting-score","TotalScore":300}}"#,
    ]);
    std::fs::write(
        fixture.path().join("highlight.log"),
        format!(
            r#"now event parser params is: {{"games":[{{"matches_id":"match-conflicting-score","user_name":"Current","event":"{first_payload}"}}]}}"#
        ),
    )
    .expect("first log should write");
    std::fs::write(
        fixture.path().join("highlight.old.log"),
        format!(
            r#"now event parser params is: {{"games":[{{"matches_id":" MATCH-CONFLICTING-SCORE ","user_name":"Current","event":"{second_payload}"}}]}}"#
        ),
    )
    .expect("second log should write");

    let result = parse_highlight_logs(fixture.path()).expect("logs should parse");

    assert!(result.round_scores.is_empty());
}

#[test]
fn segments_multiple_games_by_game_start_and_settle_ids() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000007","PlayerName":"Alice#TAG","GameID":"match-segment-a"}}"#,
        r#"{"Params":{"EventName":"RoundStart","OpenID":"10000007","PlayerName":"Alice","RoundID":0}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Alice","MatchCombatScore":240,"GameID":"match-segment-a"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000007","PlayerName":"Alice","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000007","PlayerName":"Alice","GameID":"match-segment-a","TotalScore":240}}"#,
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000008","PlayerName":"Bob","GameID":"match-segment-b"}}"#,
        r#"{"Params":{"EventName":"RoundStart","OpenID":"10000008","RoundID":0}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Bob","MatchCombatScore":310,"GameID":"match-segment-b"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000008","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000008","GameID":"match-segment-b","TotalScore":310}}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"wrong-outer-match","user_name":"WrongOuterPlayer","event":"{event_payload}"}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(
        result.round_scores,
        vec![
            HighlightLogRoundScore {
                account_id: "10000007".to_string(),
                match_id: "match-segment-a".to_string(),
                round_id: 0,
                score: 240,
            },
            HighlightLogRoundScore {
                account_id: "10000008".to_string(),
                match_id: "match-segment-b".to_string(),
                round_id: 0,
                score: 310,
            },
        ]
    );
}

#[test]
fn keeps_same_match_round_scores_isolated_by_account() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000009","GameID":"shared-match"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"First","MatchCombatScore":220,"GameID":"shared-match"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000009","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000009","GameID":"shared-match","TotalScore":220}}"#,
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000010","GameID":"shared-match"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Second","MatchCombatScore":330,"GameID":"shared-match"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000010","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000010","GameID":"shared-match","TotalScore":330}}"#,
    ]);
    let content =
        format!(r#"now event parser params is: {{"games":[{{"event":"{event_payload}"}}]}}"#);

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(result.round_scores.len(), 2);
    assert_eq!(result.round_scores[0].account_id, "10000009");
    assert_eq!(result.round_scores[0].score, 220);
    assert_eq!(result.round_scores[1].account_id, "10000010");
    assert_eq!(result.round_scores[1].score, 330);
}

#[test]
fn rejects_segment_when_combat_score_game_id_is_wrong() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000011","GameID":"expected-match"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"other-match"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000011","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000011","GameID":"expected-match","TotalScore":250}}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"expected-match","event":"{event_payload}"}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert!(result.round_scores.is_empty());
}

#[test]
fn rejects_segment_without_settle_account_identity() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000012","GameID":"missing-account"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"missing-account"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000012","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","GameID":"missing-account","TotalScore":250}}"#,
    ]);
    let content =
        format!(r#"now event parser params is: {{"games":[{{"event":"{event_payload}"}}]}}"#);

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert!(result.round_scores.is_empty());
}

#[test]
fn rejects_segment_when_same_account_context_names_disagree() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000013","PlayerName":"Other","GameID":"identity-mismatch"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"identity-mismatch"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000013","PlayerName":"Current","RoundID":0}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000013","PlayerName":"Current","GameID":"identity-mismatch","TotalScore":250}}"#,
    ]);
    let content =
        format!(r#"now event parser params is: {{"games":[{{"event":"{event_payload}"}}]}}"#);

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert!(result.round_scores.is_empty());
}

#[test]
fn rejects_segment_with_a_middle_round_gap() {
    let event_payload = gzip_json_array(&[
        r#"{"Params":{"EventName":"GameStart","OpenID":"10000014","GameID":"round-gap"}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":250,"GameID":"round-gap"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000014","RoundID":0}}"#,
        r#"{"Params":{"EventName":"CombatScore","__PlayerName__":"Current","MatchCombatScore":700,"GameID":"round-gap"}}"#,
        r#"{"Params":{"EventName":"RoundEnd","OpenID":"10000014","RoundID":2}}"#,
        r#"{"Params":{"EventName":"GameSettle","OpenID":"10000014","GameID":"round-gap","TotalScore":700}}"#,
    ]);
    let content =
        format!(r#"now event parser params is: {{"games":[{{"event":"{event_payload}"}}]}}"#);

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert!(result.round_scores.is_empty());
}

#[test]
fn parses_real_event_parser_game_context_without_role_path_player_name() {
    let event_payload = gzip_json_array(&[
        r#"{"EventName":"Shot","OpenID":"90000000000000001","RoleName":"/Game/Maps/Juliett/Juliett.Juliett:PersistentLevel.Nox_PC_C_2147277106","PlayerName":"","AgentName":"Vyse","MapName":"Triad","RoundID":4,"ModeID":"/Game/GameModes/Bomb/competitive","WeaponSkinName":"AK_Champions_PrimaryAsset_C /Game/Equippables/Guns/Rifles/AK/Champions/AK_Champions_PrimaryAsset.Default__AK_Champions_PrimaryAsset_C","KillerPlayerName":"FixtureAlpha","KilledPlayerName":"测试玩家乙","KillerIsMe":1,"KilledIsMe":0}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"22222222-2222-4222-8222-222222222201","user_name":"FixtureAlpha","user_nick_id":"0001","hero_id":"Vyse","map_id":"/Game/Maps/Juliett/Juliett","game_mode":"1","stats":{{"kills":16,"deaths":16,"assists":1,"score":208,"mode_name":"/Game/GameModes/Bomb/competitive","has_won":false,"rounds_lost":13,"rounds_won":9}},"agent":{{"agent_name":"Vyse"}},"career":{{"battle_id":"22222222-2222-4222-8222-222222222202","match_id":"22222222-2222-4222-8222-222222222201","hero_name":"维斯","kda":"16/16/1","game_mode":"竞技模式","map_name":"日落之城","won_match":0,"score":"208"}},"ext":"{{\"game_info\":{{\"career\":{{\"kda\":\"16/16/1\",\"map_name\":\"日落之城\",\"game_mode\":\"竞技模式\"}}}}}}","event":"{event_payload}","videoList":[{{"path":"C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos90000000000000001/22222222-2222-4222-8222-222222222201/clip.mp4"}}]}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert_eq!(
        record.match_id.as_deref(),
        Some("22222222-2222-4222-8222-222222222201")
    );
    assert_eq!(
        record.battle_id.as_deref(),
        Some("22222222-2222-4222-8222-222222222202")
    );
    assert_eq!(record.player_name.as_deref(), Some("FixtureAlpha#0001"));
    assert_eq!(record.agent_name.as_deref(), Some("Vyse"));
    assert_eq!(record.map_id.as_deref(), Some("/Game/Maps/Juliett/Juliett"));
    assert_eq!(record.map_name.as_deref(), Some("日落之城"));
    assert_eq!(record.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(record.kda.as_deref(), Some("16/16/1"));
    assert_eq!(record.scoreline.as_deref(), Some("9/13"));
    assert_eq!(record.has_won, Some(false));
    assert_eq!(record.combat_score, Some(208));
    assert_eq!(record.kill_events.len(), 1);
    assert_eq!(
        record.kill_events[0].killer_name.as_deref(),
        Some("FixtureAlpha")
    );
    assert_eq!(
        record.kill_events[0].killed_name.as_deref(),
        Some("测试玩家乙")
    );
}

#[test]
fn event_parser_ignores_round_zero_shot_events_as_kill_timeline_noise() {
    let event_payload = gzip_json_array(&[
        r#"{"EventName":"Shot","RoundID":0,"WeaponID":"BasePistol","KillerIsMe":0,"KilledIsMe":0}"#,
        r#"{"EventName":"Shot","RoundID":0,"WeaponID":"LugerPistol","KillerPlayerName":"FixtureBravo","KilledPlayerName":"贤者","KillerIsMe":1,"KilledIsMe":0}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"match-noisy-shot","event":"{event_payload}","videoList":[{{"path":"C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/match-noisy-shot/clip.mp4"}}]}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    assert!(result.records[0].kill_events.is_empty());
}

#[test]
fn event_parser_does_not_use_nested_event_player_names_as_account_name() {
    let event_payload = gzip_json_array(&[
        r#"{"EventName":"Shot","OpenID":"9000000000000000002","PlayerName":"测试玩家甲","AgentName":"Sova","MapName":"Triad","ModeID":"/Game/GameModes/Bomb/competitive","KillerPlayerName":"测试玩家甲","KilledPlayerName":"FixtureBravo","KillerIsMe":0,"KilledIsMe":1}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"22222222-2222-4222-8222-222222222203","event":"{event_payload}","videoList":[{{"path":"C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/22222222-2222-4222-8222-222222222203/clip.mp4"}}]}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert_eq!(record.player_name, None);
    assert!(record.kill_events.is_empty());
}

#[test]
fn event_parser_uses_only_marked_current_player_for_tagged_name_fallback() {
    let event_payload = gzip_json_array(&[
        r#"{"EventName":"Kill","gameName":"Enemy","tagLine":"9999","KillerPlayerName":"Enemy#9999","KilledPlayerName":"Current#1111"}"#,
        r#"{"EventName":"Kill","gameName":"Current","tagLine":"1111","KillerPlayerName":"Current#1111","KilledPlayerName":"Other#2222","KillerIsMe":1,"KilledIsMe":0}"#,
    ]);
    let content = format!(
        r#"now event parser params is: {{"games":[{{"matches_id":"match-current-player","event":"{event_payload}","videoList":[{{"path":"C:/ACLOS/aclos-highlight/wonderfulVideos1001/match-current-player/clip.mp4"}}]}}]}}"#
    );

    let result = parse_highlight_log_content(&content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    assert_eq!(
        result.records[0].player_name.as_deref(),
        Some("Current#1111")
    );
}

#[test]
fn parses_wonderfulsdk_ui_show_tlog_request_for_map_and_mode() {
    let content = r#"
        [07-04 16:10:50.613] [info] | REQUEST： {"method":"post","url":"/go/aclos-common/report/tlog","data":{"eventId":"wonderfulsdk_ui_show","openid":"9000***********0002","value":"{\"versionCode\":\"2.15.3\",\"hero\":\"800164b1-74b3-541f-be82-25c791fec72e\",\"mapId\":\"/Game/Maps/Triad/Triad\",\"modelId\":\"competitive/competitive\",\"GameId\":\"22222222-2222-4222-8222-222222222204\"}","extra":"{}"}}
    "#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert_eq!(
        record.match_id.as_deref(),
        Some("22222222-2222-4222-8222-222222222204")
    );
    assert_eq!(record.map_id.as_deref(), Some("/Game/Maps/Triad/Triad"));
    assert_eq!(record.map_name.as_deref(), Some("隐世修所"));
    assert_eq!(record.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(record.kda, None);
}

#[test]
fn parses_battle_list_response_records_for_match_summary() {
    let content = r#"
        [06-27 22:39:04.071] [info] | RESPONSE： {"result":0,"data":{"battle_list":[{"battle_id":"33333333-3333-4333-8333-333333333301","match_id":"33333333-3333-4333-8333-333333333302","hero_name":"霓虹","hero_image":"https://game.gtimg.cn/images/val/agamezlk/headicon/18.png","kda":"20/18/9","score":"6501","acs":"309","time":"06/27","game_mode":"竞技模式","match_result":"失败","map_name":"天枢云阙","rounds_result":"8-13"}]}}
    "#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert_eq!(record.line_kind, HighlightLogLineKind::BattleListResponse);
    assert_eq!(
        record.match_id.as_deref(),
        Some("33333333-3333-4333-8333-333333333302")
    );
    assert_eq!(
        record.battle_id.as_deref(),
        Some("33333333-3333-4333-8333-333333333301")
    );
    assert_eq!(record.agent_name.as_deref(), Some("霓虹"));
    assert_eq!(record.map_name.as_deref(), Some("天枢云阙"));
    assert_eq!(record.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(record.kda.as_deref(), Some("20/18/9"));
    assert_eq!(record.scoreline.as_deref(), Some("8/13"));
    assert_eq!(record.has_won, Some(false));
    assert_eq!(record.combat_score, Some(309));
}

#[test]
fn parses_video_list_data_lines_for_match_summary() {
    let content = r#"
        | msg: 获取视频列表。data:{"state":0,"msg":"callHighlightIPCFunc : 执行成功","data":[{"map":{"map_id":"/Game/Maps/Plummet/Plummet"},"agent":{"agent_id":"471782c4-4e75-56d6-9d8f-065abea781da","agent_name":"Neon"},"matches_id":"33333333-3333-4333-8333-333333333302","stats":{"deaths":18,"score":309,"mode_name":"/Game/GameModes/Bomb/competitive","has_won":false,"rounds_lost":13,"kills":20,"assists":9,"rounds_won":8},"openID":"9000000000000000003","videos":[{"video_src":"C:\\Users\\FixtureUser\\AppData\\ACLOS\\aclos-highlight\\wonderfulVideos9000000000000000003\\33333333-3333-4333-8333-333333333302\\clip.mp4"}],"career":{"battle_id":"33333333-3333-4333-8333-333333333301","match_id":"33333333-3333-4333-8333-333333333302","hero_name":"霓虹","game_mode":"竞技模式","rounds_score":"8/13","won_match":0,"score":"309","kda":"20/18/9","map_name":"天枢云阙"}}]}
    "#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert_eq!(
        record.match_id.as_deref(),
        Some("33333333-3333-4333-8333-333333333302")
    );
    assert_eq!(
        record.battle_id.as_deref(),
        Some("33333333-3333-4333-8333-333333333301")
    );
    assert_eq!(
        record.record_src.as_deref(),
        Some("C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000003/33333333-3333-4333-8333-333333333302/clip.mp4")
    );
    assert_eq!(record.agent_name.as_deref(), Some("Neon"));
    assert_eq!(record.map_name.as_deref(), Some("天枢云阙"));
    assert_eq!(record.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(record.kda.as_deref(), Some("20/18/9"));
    assert_eq!(record.scoreline.as_deref(), Some("8/13"));
    assert_eq!(record.has_won, Some(false));
    assert_eq!(record.combat_score, Some(309));
}

#[test]
fn parses_role_account_hints_from_response_payloads() {
    let content = r#"
        [04-23 16:54:05] | RESPONSE： {"result":0,"data":{"list":[{"role_name":"测试玩家丙","nick_id":"0003","g_open_id":"9000000000000000004","role_account_id":"55555555-5555-4555-8555-555555555501"},{"role_name":"FixtureCharlie","nick_id":"0004","g_open_id":"90000000000000000005"}]}}
    "#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.account_name_hints.len(), 2);
    assert_eq!(
        result.account_name_hints[0].account_id,
        "9000000000000000004"
    );
    assert_eq!(
        result.account_name_hints[0].account_name.as_deref(),
        Some("测试玩家丙#0003")
    );
    assert_eq!(
        result.account_name_hints[1].account_id,
        "90000000000000000005"
    );
    assert_eq!(
        result.account_name_hints[1].account_name.as_deref(),
        Some("FixtureCharlie#0004")
    );
}

#[test]
fn parses_nested_video_src_as_record_src() {
    let content = r#"
        template param == {"match_id":"match-video-src","gameInfo":{"ext":{"video_info":{"video_src":"C:\\Users\\FixtureUser\\AppData\\ACLOS\\aclos-highlight\\wonderfulVideos9000000000000000002\\match-video-src\\clip.mp4"}}},"GameName":"FixtureBravo","TagLine":"0002","kills":31,"deaths":15,"assists":6}
    "#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert_eq!(record.match_id.as_deref(), Some("match-video-src"));
    assert_eq!(
        record.record_src.as_deref(),
        Some("C:/Users/FixtureUser/AppData/ACLOS/aclos-highlight/wonderfulVideos9000000000000000002/match-video-src/clip.mp4")
    );
    assert_eq!(record.player_name.as_deref(), Some("FixtureBravo#0002"));
    assert_eq!(record.kda.as_deref(), Some("31/15/6"));
}

#[test]
fn parses_post_snapshot_career_records_for_match_summary() {
    let content = r#"
        [07-01 18:49:53] ==wonderful-log==> postSnapshot start data [[{"type":"match","killInfos":[{"EventTime":"2026-07-01 18:28:26.130","RoundID":9,"WeaponSkinName":"Carbine_SpecOps_PrimaryAsset_C /Game/Equippables/Guns/Rifles/Carbine/SpecOps/Carbine_SpecOps_PrimaryAsset.Default__Carbine_SpecOps_PrimaryAsset_C","KillerPlayerName":"FixtureBravo","KilledPlayerName":"FixtureDelta"}],"career":{"battle_id":"44444444-4444-4444-8444-444444444401","match_id":"44444444-4444-4444-8444-444444444402","hero_name":"芮娜","hero_image":"https://game.gtimg.cn/images/val/agamezlk/headicon/11.png","time":"2026-07-01 18:49:54","game_mode":"竞技模式","won_match":1,"score":"385","kda":"31/15/6","map_name":"霓虹町","rounds_result":"13-10"}}]]
    "#;

    let result = parse_highlight_log_content(content).expect("content should parse");

    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert_eq!(
        record.match_id.as_deref(),
        Some("44444444-4444-4444-8444-444444444402")
    );
    assert_eq!(
        record.battle_id.as_deref(),
        Some("44444444-4444-4444-8444-444444444401")
    );
    assert_eq!(record.agent_name.as_deref(), Some("芮娜"));
    assert_eq!(record.map_name.as_deref(), Some("霓虹町"));
    assert_eq!(record.game_mode.as_deref(), Some("竞技模式"));
    assert_eq!(record.kda.as_deref(), Some("31/15/6"));
    assert_eq!(record.scoreline.as_deref(), Some("13/10"));
    assert_eq!(record.has_won, Some(true));
    assert_eq!(record.combat_score, Some(385));
    assert_eq!(record.kill_events.len(), 1);
    assert_eq!(
        record.kill_events[0].event_time.as_deref(),
        Some("2026-07-01 18:28:26.130")
    );
}

#[test]
fn missing_highlight_log_returns_empty_result() {
    let missing_path = std::env::temp_dir().join("vhm-missing-highlight.log");

    let result = parse_highlight_log(missing_path).expect("missing log should be non-fatal");

    assert!(result.records.is_empty());
    assert_eq!(result.bad_line_count, 0);
    assert_eq!(result.gzip_event_count, 0);
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("highlight-log-sanitized.txt")
}

fn gzip_json_array(items: &[&str]) -> String {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(format!("[{}]", items.join(",")).as_bytes())
        .expect("gzip input should write");
    let compressed = encoder.finish().expect("gzip should finish");
    general_purpose::STANDARD.encode(compressed)
}

struct TestFixture {
    root: PathBuf,
}

impl TestFixture {
    fn new(label: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("vhm-{label}-{unique}"));
        std::fs::create_dir_all(&root).expect("fixture root should be created");
        Self { root }
    }

    fn path(&self) -> &std::path::Path {
        &self.root
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
