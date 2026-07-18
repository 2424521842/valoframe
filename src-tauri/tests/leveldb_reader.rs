// Privacy note: account IDs, player names, match IDs, and paths in this file are synthetic fixtures.
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use valorant_highlight_manager_lib::leveldb_reader::read_leveldb_battle_lists;

#[test]
fn reads_utf16le_battle_list_records_from_leveldb_files() {
    let fixture = TestFixture::new("leveldb-reader");
    let payload = r#"[
        {
            "battleId": "battle-a-001",
            "matchId": "match-a-001",
            "kills": 18,
            "deaths": 7,
            "assists": 3,
            "date": "2026-07-01T10:00:00Z",
            "GameName": "FixtureAlpha",
            "TagLine": "0001",
            "heroAvatarUrl": "https://assets.example/jett.png"
        },
        {
            "battle_id": "battle-b-002",
            "game_id": "match-b-002",
            "KDA": "12/8/9",
            "matchDate": "2026-07-02T11:00:00Z",
            "agent_avatar_url": "https://assets.example/sage.png"
        }
    ]"#;
    fs::write(
        fixture.path().join("000003.ldb"),
        leveldb_blob("1001", payload),
    )
    .expect("leveldb fixture should be written");

    let result = read_leveldb_battle_lists(fixture.path()).expect("leveldb should parse");

    assert!(!result.used_snapshot);
    assert_eq!(result.bad_record_count, 0);
    assert_eq!(result.warning_count, 0);
    assert_eq!(result.battles.len(), 2);

    assert_eq!(result.battles[0].account_id, "1001");
    assert_eq!(result.battles[0].battle_id.as_deref(), Some("battle-a-001"));
    assert_eq!(result.battles[0].match_id.as_deref(), Some("match-a-001"));
    assert_eq!(
        result.battles[0].player_name.as_deref(),
        Some("FixtureAlpha#0001")
    );
    assert_eq!(result.battles[0].kda.as_deref(), Some("18/7/3"));
    assert_eq!(
        result.battles[0].match_date.as_deref(),
        Some("2026-07-01T10:00:00Z")
    );
    assert_eq!(
        result.battles[0].agent_avatar_url.as_deref(),
        Some("https://assets.example/jett.png")
    );

    assert_eq!(result.battles[1].battle_id.as_deref(), Some("battle-b-002"));
    assert_eq!(result.battles[1].match_id.as_deref(), Some("match-b-002"));
    assert_eq!(result.battles[1].kda.as_deref(), Some("12/8/9"));
    assert_eq!(
        result.battles[1].agent_avatar_url.as_deref(),
        Some("https://assets.example/sage.png")
    );
}

#[test]
fn uses_snapshot_when_lock_file_exists_and_skips_bad_records() {
    let fixture = TestFixture::new("leveldb-lock");
    fs::write(fixture.path().join("LOCK"), b"locked").expect("lock file should be written");
    fs::write(
        fixture.path().join("000004.log"),
        leveldb_blob(
            "2002",
            r#"[{"battleId":"battle-ok","matchId":"match-ok","kills":4,"deaths":2,"assists":1},{"date":"missing ids"}]"#,
        ),
    )
    .expect("leveldb log fixture should be written");

    let result = read_leveldb_battle_lists(fixture.path()).expect("snapshot should parse");

    assert!(result.used_snapshot);
    assert_eq!(result.copied_file_count, 1);
    assert_eq!(result.bad_record_count, 1);
    assert_eq!(result.battles.len(), 1);
    assert_eq!(result.battles[0].account_id, "2002");
    assert_eq!(result.battles[0].match_id.as_deref(), Some("match-ok"));
    assert_eq!(result.battles[0].kda.as_deref(), Some("4/2/1"));
}

#[test]
fn reads_account_role_names_from_leveldb_files() {
    let fixture = TestFixture::new("leveldb-account-roles");
    let payload = r#"[{
        "openid": "9000000000000000002",
        "nick": "FixtureBravo",
        "tag": "0002"
    }]"#;
    fs::write(
        fixture.path().join("000005.ldb"),
        account_roles_blob(payload),
    )
    .expect("leveldb fixture should be written");

    let result = read_leveldb_battle_lists(fixture.path()).expect("leveldb should parse");

    assert_eq!(result.bad_record_count, 0);
    assert_eq!(result.account_roles.len(), 1);
    assert_eq!(result.account_roles[0].account_id, "9000000000000000002");
    assert_eq!(
        result.account_roles[0].player_name.as_deref(),
        Some("FixtureBravo#0002")
    );
}

#[test]
fn repairs_control_byte_account_role_entries_from_leveldb_blocks() {
    let fixture = TestFixture::new("leveldb-account-roles-repair");
    let mut blob =
        Vec::from(b"noise-ACLOS_USER_ROLES_INFO\x01L\x0d\x7fh[{\"online_status\":\"\",\"openi");
    blob.extend_from_slice(&[0x01, 0x90, 0x88]);
    blob.extend_from_slice(
        b"90000000000000000008\",\"nick\":\"FixtureBravo\r\x10\x00_\x050@00002\",\"avatar\":\"https://game.gtimg.cn/card.png\",\"level\":\"152\"}]tail",
    );
    fs::write(fixture.path().join("003983.ldb"), blob).expect("leveldb fixture should be written");

    let result = read_leveldb_battle_lists(fixture.path()).expect("leveldb should parse");

    assert_eq!(result.account_roles.len(), 1);
    assert_eq!(result.account_roles[0].account_id, "90000000000000000008");
    assert_eq!(
        result.account_roles[0].player_name.as_deref(),
        Some("FixtureBravo#00002")
    );
}

#[test]
fn repairs_noisy_battle_list_entries_from_leveldb_blocks() {
    let fixture = TestFixture::new("leveldb-battle-repair");
    let mut blob = Vec::from(b"noise-acloshighlight_battle_list_9000000000000000004");
    blob.extend_from_slice(b"\x01\x02{\"battle_id\":\"11111111-1111-4111-8111-111111111101\",");
    blob.extend_from_slice(b"\"matc\x07h_i\x10d\":\"11111111-1111-4111-8111L-111111111102\",");
    blob.extend_from_slice(b"\"k\x00d\x00a\":\"022/013/002\",");
    blob.extend_from_slice(
        b"\"heroAvatarUrl\":\"https://game.gtimg.cn/images/headico/02.png\"}tail",
    );
    fs::write(fixture.path().join("003978.ldb"), blob).expect("leveldb fixture should be written");

    let result = read_leveldb_battle_lists(fixture.path()).expect("leveldb should parse");

    assert_eq!(result.bad_record_count, 0);
    assert_eq!(result.battles.len(), 1);
    assert_eq!(result.battles[0].account_id, "9000000000000000004");
    assert_eq!(
        result.battles[0].match_id.as_deref(),
        Some("11111111-1111-4111-8111-111111111102")
    );
    assert_eq!(result.battles[0].kda.as_deref(), Some("22/13/2"));
    assert_eq!(
        result.battles[0].agent_avatar_url.as_deref(),
        Some("https://game.gtimg.cn/images/headico/02.png")
    );
}

#[test]
fn missing_leveldb_directory_returns_empty_result() {
    let missing_path = std::env::temp_dir().join("vhm-missing-leveldb-reader");

    let result = read_leveldb_battle_lists(missing_path).expect("missing dir should be non-fatal");

    assert!(result.battles.is_empty());
    assert_eq!(result.bad_record_count, 0);
    assert_eq!(result.warning_count, 0);
    assert_eq!(result.copied_file_count, 0);
    assert!(!result.used_snapshot);
}

fn leveldb_blob(account_id: &str, json_payload: &str) -> Vec<u8> {
    let mut blob = Vec::from("noise-acloshighlight_battle_list_".as_bytes());
    blob.extend_from_slice(account_id.as_bytes());
    blob.extend_from_slice(b"\x01\x02");
    for unit in json_payload.encode_utf16() {
        blob.extend_from_slice(&unit.to_le_bytes());
    }
    blob.extend_from_slice(b"tail-noise");
    blob
}

fn account_roles_blob(json_payload: &str) -> Vec<u8> {
    let mut blob = Vec::from("noise-ACLOS_USER_ROLES_INFO".as_bytes());
    blob.extend_from_slice(b"\x01\x02");
    blob.extend_from_slice(json_payload.as_bytes());
    blob.extend_from_slice(b"tail-noise");
    blob
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
