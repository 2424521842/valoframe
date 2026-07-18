// Privacy note: repository fixtures use synthetic IDs and generic minimums only.
// Site-specific regression inputs are supplied locally through VHM_REAL_SCAN_* variables.
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use valorant_highlight_manager_lib::{db, scanner, wonderful_db};

const MINIMUM_WONDERFUL_ACCOUNT_FILES: usize = 1;
const MINIMUM_WONDERFUL_SNAPSHOT_FILES: usize = 1;
const MINIMUM_WONDERFUL_MATCHES: usize = 1;
const MINIMUM_WONDERFUL_VIDEOS: usize = 1;
const MINIMUM_WONDERFUL_SNAPSHOTS: usize = 1;

struct KnownSixKillExpectation<'a> {
    match_id: &'a str,
    score: i64,
    min_duration_ms: i64,
    max_duration_ms: i64,
}

#[derive(Debug, PartialEq, Eq)]
struct PostScanVerification {
    matched_official_video_count: usize,
    missing_official_video_count: usize,
    official_event_count: i64,
    matched_type_four_count: i64,
    matched_type_six_count: i64,
    matched_type_ten_count: i64,
    invalid_type_four_count: i64,
    invalid_type_six_count: i64,
    invalid_type_ten_count: i64,
}

struct WorkingDatabaseGuard {
    path: PathBuf,
}

impl WorkingDatabaseGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn prepare(path: PathBuf) -> io::Result<Self> {
        for candidate in sqlite_database_files(&path) {
            match fs::remove_file(candidate) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(Self::new(path))
    }
}

impl Drop for WorkingDatabaseGuard {
    fn drop(&mut self) {
        for candidate in sqlite_database_files(&self.path) {
            let _ = fs::remove_file(candidate);
        }
    }
}

fn sqlite_database_files(path: &Path) -> [PathBuf; 3] {
    let mut wal_path = path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let mut shm_path = path.as_os_str().to_os_string();
    shm_path.push("-shm");
    [path.to_path_buf(), wal_path.into(), shm_path.into()]
}

fn live_production_database_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|app_data| {
        PathBuf::from(app_data)
            .join("com.valorant.highlight.manager")
            .join("highlight-index.sqlite3")
    })
}

fn validate_source_database_path(
    workspace_root: &Path,
    source_database_path: &Path,
    live_database_path: Option<&Path>,
) -> Result<PathBuf, String> {
    let canonical_source = fs::canonicalize(source_database_path)
        .map_err(|_| "real scan database copy could not be resolved".to_string())?;
    if let Some(live_path) = live_database_path {
        let is_live_database = source_database_path == live_path
            || fs::canonicalize(live_path)
                .map(|canonical_live| canonical_live == canonical_source)
                .unwrap_or(false);
        if is_live_database {
            return Err("real scan refuses the live production database".to_string());
        }
    }

    let canonical_workspace = fs::canonicalize(workspace_root)
        .map_err(|_| "workspace root could not be resolved".to_string())?;
    if !canonical_source.starts_with(&canonical_workspace) {
        return Err("real scan database source must be a workspace copy".to_string());
    }
    Ok(canonical_source)
}

fn validate_missing_real_scan_input(
    input_was_explicit: bool,
    explicit_error: &'static str,
) -> Result<(), &'static str> {
    if input_was_explicit {
        Err(explicit_error)
    } else {
        Ok(())
    }
}

fn copy_database_for_real_scan(source: &Path, working: &Path) -> io::Result<u64> {
    fs::copy(source, working)
}

fn wonderful_db_dir_for_real_scan(scan_root: &Path) -> PathBuf {
    let adjacent_aclos_root = if scan_root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("aclos-highlight"))
    {
        scan_root.parent().map(Path::to_path_buf)
    } else {
        Some(scan_root.to_path_buf())
    };
    if let Some(adjacent_wonderful_dir) = adjacent_aclos_root
        .map(|aclos_root| aclos_root.join("WonderfulDb"))
        .filter(|path| path.is_dir())
    {
        return adjacent_wonderful_dir;
    }

    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Users\Default"))
                .join("AppData")
                .join("Roaming")
        })
        .join("ACLOS")
        .join("WonderfulDb")
}

fn invalid_clip_scoped_kill_count(
    connection: &Connection,
    highlight_type: i64,
    expected_kill_counts: &[i64],
) -> i64 {
    let allowed_counts = expected_kill_counts
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    connection
        .query_row(
            &format!(
                "
                SELECT COUNT(*)
                FROM (
                    SELECT
                        clip_metadata.clip_id,
                        clip_metadata.kill_count,
                        SUM(CASE
                            WHEN clip_events.event_type = 'kill'
                             AND clip_events.killer_is_me = 1
                            THEN 1 ELSE 0
                        END) AS own_kill_events
                    FROM clip_metadata
                    JOIN clips ON clips.id = clip_metadata.clip_id
                    LEFT JOIN clip_events ON clip_events.clip_id = clip_metadata.clip_id
                    WHERE clip_metadata.metadata_source = 'wonderful_db'
                      AND clips.file_status = 'available'
                      AND CAST(clip_metadata.highlight_type AS INTEGER) = ?1
                    GROUP BY clip_metadata.clip_id, clip_metadata.kill_count
                    HAVING COALESCE(clip_metadata.kill_count, -1) NOT IN ({allowed_counts})
                        OR clip_metadata.kill_count <> own_kill_events
                )
                "
            ),
            [highlight_type],
            |row| row.get(0),
        )
        .expect("clip-scoped kill count verifier should load")
}

fn matched_highlight_type_count(connection: &Connection, highlight_type: i64) -> i64 {
    connection
        .query_row(
            "
            SELECT COUNT(*)
            FROM clip_metadata
            JOIN clips ON clips.id = clip_metadata.clip_id
            WHERE metadata_source = 'wonderful_db'
              AND clips.file_status = 'available'
              AND CAST(highlight_type AS INTEGER) = ?1
            ",
            [highlight_type],
            |row| row.get(0),
        )
        .expect("matched highlight type count should load")
}

fn verify_post_scan_metadata(
    connection: &Connection,
    wonderful_video_count: usize,
    known_six_kill: Option<&KnownSixKillExpectation<'_>>,
) -> PostScanVerification {
    let official_clip_count: i64 = connection
        .query_row(
            "
            SELECT COUNT(*)
            FROM clip_metadata
            JOIN clips ON clips.id = clip_metadata.clip_id
            WHERE clip_metadata.metadata_source = 'wonderful_db'
              AND clips.file_status = 'available'
            ",
            [],
            |row| row.get(0),
        )
        .expect("official clip count should load");
    let official_event_count: i64 = connection
        .query_row(
            "
            SELECT COUNT(*)
            FROM clip_events
            JOIN clip_metadata ON clip_metadata.clip_id = clip_events.clip_id
            JOIN clips ON clips.id = clip_metadata.clip_id
            WHERE clip_metadata.metadata_source = 'wonderful_db'
              AND clips.file_status = 'available'
            ",
            [],
            |row| row.get(0),
        )
        .expect("official clip event count should load");
    let invalid_type_four_count = invalid_clip_scoped_kill_count(connection, 4, &[3]);
    let invalid_type_six_count = invalid_clip_scoped_kill_count(connection, 6, &[4]);
    let invalid_type_ten_count = invalid_clip_scoped_kill_count(connection, 10, &[5, 6]);
    let matched_type_four_count = matched_highlight_type_count(connection, 4);
    let matched_type_six_count = matched_highlight_type_count(connection, 6);
    let matched_type_ten_count = matched_highlight_type_count(connection, 10);
    let matched_official_video_count =
        usize::try_from(official_clip_count).expect("official clip count should fit usize");
    assert!(
        matched_official_video_count <= wonderful_video_count,
        "matched local videos cannot exceed WonderfulDb official video records"
    );
    let missing_official_video_count = wonderful_video_count - matched_official_video_count;
    assert!(
        matched_type_four_count > 0,
        "type-4 verification must not be vacuous"
    );
    assert!(
        matched_type_six_count > 0,
        "type-6 verification must not be vacuous"
    );
    assert!(
        matched_type_ten_count > 0,
        "type-10 verification must not be vacuous"
    );
    assert_eq!(invalid_type_four_count, 0);
    assert_eq!(invalid_type_six_count, 0);
    assert_eq!(
        invalid_type_ten_count, 0,
        "type-10 clips must use their own five or six WonderfulDb kill events"
    );
    if let Some(expected) = known_six_kill {
        let known_six_kill_video: (i64, Option<String>, Option<i64>, Option<i64>, Option<i64>) =
            connection
                .query_row(
                    "
                    SELECT
                        COUNT(*),
                        MIN(official_video_name),
                        MIN(kill_count),
                        MIN(CAST(round_score AS INTEGER)),
                        MIN(clips.duration_ms)
                    FROM clip_metadata
                    JOIN clips ON clips.id = clip_metadata.clip_id
                    WHERE metadata_source = 'wonderful_db'
                      AND clips.file_status = 'available'
                      AND match_id = ?1
                    ",
                    [expected.match_id],
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
                .expect("known six-kill video should load");
        let old_incorrect_kill_count: i64 = connection
            .query_row(
                "
                SELECT COUNT(*)
                FROM clip_metadata
                WHERE metadata_source = 'wonderful_db'
                  AND match_id = ?1
                  AND kill_count = 26
                ",
                [expected.match_id],
                |row| row.get(0),
            )
            .expect("old incorrect kill count verifier should load");

        assert_eq!(known_six_kill_video.0, 1);
        assert_eq!(known_six_kill_video.1.as_deref(), Some("六杀时刻"));
        assert_eq!(known_six_kill_video.2, Some(6));
        assert_eq!(known_six_kill_video.3, Some(expected.score));
        assert!(
            matches!(
                known_six_kill_video.4,
                Some(duration_ms)
                    if (expected.min_duration_ms..=expected.max_duration_ms)
                        .contains(&duration_ms)
            ),
            "known six-kill duration should remain inside the fixture range"
        );
        assert_eq!(old_incorrect_kill_count, 0);
    }

    PostScanVerification {
        matched_official_video_count,
        missing_official_video_count,
        official_event_count,
        matched_type_four_count,
        matched_type_six_count,
        matched_type_ten_count,
        invalid_type_four_count,
        invalid_type_six_count,
        invalid_type_ten_count,
    }
}

#[test]
fn real_scan_uses_an_independent_working_database_copy() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow Unix epoch")
        .as_nanos();
    let temp_root = std::env::temp_dir().join(format!("vhm-real-scan-copy-{nonce}"));
    fs::create_dir_all(&temp_root).expect("temporary root should be created");
    let source_path = temp_root.join("source.sqlite3");
    let working_path = temp_root.join("working.sqlite3");
    let source = Connection::open(&source_path).expect("source database should open");
    source
        .execute_batch(
            "CREATE TABLE marker (value TEXT NOT NULL); INSERT INTO marker VALUES ('source');",
        )
        .expect("source marker should seed");
    drop(source);

    copy_database_for_real_scan(&source_path, &working_path)
        .expect("working database copy should be created");
    let working = Connection::open(&working_path).expect("working database should open");
    working
        .execute("UPDATE marker SET value = 'working'", [])
        .expect("working database should mutate");
    drop(working);

    let source = Connection::open(&source_path).expect("source database should reopen");
    let source_value: String = source
        .query_row("SELECT value FROM marker", [], |row| row.get(0))
        .expect("source marker should load");
    assert_eq!(source_value, "source");
    assert!(source_path.is_file());
    drop(source);
    fs::remove_dir_all(temp_root).expect("synthetic fixture should be removed");
}

#[test]
fn real_scan_rejects_the_live_production_database() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow Unix epoch")
        .as_nanos();
    let temp_root = std::env::temp_dir().join(format!("vhm-real-scan-live-reject-{nonce}"));
    let workspace_root = temp_root.join("workspace");
    fs::create_dir_all(&workspace_root).expect("synthetic workspace should be created");
    let live_database_path = workspace_root.join("highlight-index.sqlite3");
    fs::write(&live_database_path, b"live").expect("synthetic live database should be created");

    let error = validate_source_database_path(
        &workspace_root,
        &live_database_path,
        Some(&live_database_path),
    )
    .expect_err("live production database must be rejected");
    assert!(error.contains("live production database"));

    fs::remove_dir_all(temp_root).expect("synthetic fixture should be removed");
}

#[test]
fn real_scan_working_copy_guard_removes_database_and_sidecars() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow Unix epoch")
        .as_nanos();
    let temp_root = std::env::temp_dir().join(format!("vhm-real-scan-cleanup-{nonce}"));
    fs::create_dir_all(&temp_root).expect("temporary root should be created");
    let working_path = temp_root.join("working.sqlite3");
    let [_, wal_path, shm_path] = sqlite_database_files(&working_path);

    let unwind_result = std::panic::catch_unwind(|| {
        let _guard = WorkingDatabaseGuard::prepare(working_path.clone())
            .expect("working database guard should prepare");
        fs::write(&working_path, b"database").expect("working database should be created");
        fs::write(&wal_path, b"wal").expect("WAL sidecar should be created");
        fs::write(&shm_path, b"shm").expect("SHM sidecar should be created");
        panic!("exercise panic cleanup");
    });

    assert!(unwind_result.is_err());
    assert!(!working_path.exists());
    assert!(!wal_path.exists());
    assert!(!shm_path.exists());
    fs::remove_dir_all(temp_root).expect("synthetic fixture should be removed");
}

#[test]
fn missing_explicit_real_scan_inputs_are_errors_not_skips() {
    assert!(validate_missing_real_scan_input(true, "explicit database copy is missing").is_err());
    assert!(validate_missing_real_scan_input(true, "explicit scan root is missing").is_err());
    assert!(validate_missing_real_scan_input(false, "unused").is_ok());
}

#[test]
fn post_scan_verification_runs_for_a_default_workspace_copy() {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .execute_batch(
            "
            CREATE TABLE clips (
                id TEXT PRIMARY KEY,
                file_status TEXT NOT NULL,
                duration_ms INTEGER
            );
            CREATE TABLE clip_metadata (
                clip_id TEXT PRIMARY KEY,
                metadata_source TEXT NOT NULL,
                highlight_type INTEGER,
                kill_count INTEGER,
                official_video_name TEXT,
                match_id TEXT,
                round_score TEXT
            );
            CREATE TABLE clip_events (
                clip_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                killer_is_me INTEGER NOT NULL
            );

            INSERT INTO clips VALUES
                ('type-4', 'available', 9000),
                ('type-6', 'available', 12000),
                ('type-10-five', 'available', 15000),
                ('known-six', 'available', 48000);
            INSERT INTO clip_metadata VALUES
                ('type-4', 'wonderful_db', 4, 3, '三杀时刻', 'match-4', '300'),
                ('type-6', 'wonderful_db', 6, 4, '四杀时刻', 'match-6', '700'),
                ('type-10-five', 'wonderful_db', 10, 5, '五杀时刻', 'match-10', '1200'),
                ('known-six', 'wonderful_db', 10, 6, '六杀时刻',
                    '66666666-6666-4666-8666-666666666601', '1600');

            WITH RECURSIVE counts(clip_id, remaining) AS (
                VALUES
                    ('type-4', 3),
                    ('type-6', 4),
                    ('type-10-five', 5),
                    ('known-six', 6)
                UNION ALL
                SELECT clip_id, remaining - 1 FROM counts WHERE remaining > 1
            )
            INSERT INTO clip_events (clip_id, event_type, killer_is_me)
            SELECT clip_id, 'kill', 1 FROM counts;
            ",
        )
        .expect("post-scan verification fixture should seed");

    let known_six_kill = KnownSixKillExpectation {
        match_id: "66666666-6666-4666-8666-666666666601",
        score: 1600,
        min_duration_ms: 45_000,
        max_duration_ms: 50_000,
    };
    let verification = verify_post_scan_metadata(&connection, 4, Some(&known_six_kill));
    assert_eq!(verification.matched_official_video_count, 4);
    assert_eq!(verification.official_event_count, 18);
    assert_eq!(verification.missing_official_video_count, 0);
}

#[test]
#[ignore = "manual real ACLOS regression; scans the workspace temp database copy"]
fn scan_real_aclos_into_workspace_temp_copy() {
    let workspace_root = fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
        .expect("workspace root should resolve");
    let supplied_database_path = std::env::var_os("VHM_REAL_SCAN_DB").map(PathBuf::from);
    let source_database_path = supplied_database_path
        .clone()
        .unwrap_or_else(|| workspace_root.join(".tmp-highlight-index-copy.sqlite3"));
    let custom_scan_root = std::env::var_os("VHM_REAL_SCAN_ROOT").map(PathBuf::from);
    let scan_root = custom_scan_root
        .clone()
        .unwrap_or_else(scanner::default_aclos_dir);

    if !source_database_path.is_file() {
        validate_missing_real_scan_input(
            supplied_database_path.is_some(),
            "explicit real scan database copy is missing",
        )
        .unwrap_or_else(|message| panic!("{message}"));
    }
    if !scan_root.is_dir() {
        validate_missing_real_scan_input(
            custom_scan_root.is_some(),
            "explicit real scan source root is missing",
        )
        .unwrap_or_else(|message| panic!("{message}"));
    }
    if !source_database_path.is_file() {
        eprintln!("skipping real scan: workspace database copy is missing");
        return;
    }
    if !scan_root.is_dir() {
        eprintln!("skipping real scan: ACLOS source root is missing");
        return;
    }
    let source_database_path = validate_source_database_path(
        &workspace_root,
        &source_database_path,
        live_production_database_path().as_deref(),
    )
    .unwrap_or_else(|message| panic!("{message}"));

    let wonderful_db_dir = wonderful_db_dir_for_real_scan(&scan_root);
    assert!(
        wonderful_db_dir.is_dir(),
        "WonderfulDb baseline directory should exist for the manual regression"
    );
    let wonderful_result = wonderful_db::read_wonderful_db_dir(&wonderful_db_dir);
    let wonderful_account_file_count =
        wonderful_result.accounts.len() + wonderful_result.warnings.len();
    let wonderful_match_count = wonderful_result
        .accounts
        .iter()
        .map(|account| account.matches.len())
        .sum::<usize>();
    let wonderful_snapshot_count = wonderful_result
        .snapshot_accounts
        .iter()
        .map(|account| account.snapshots.len())
        .sum::<usize>();
    let wonderful_video_count = wonderful_result
        .accounts
        .iter()
        .flat_map(|account| &account.matches)
        .map(|match_record| match_record.videos.len())
        .sum::<usize>();
    let wonderful_videos_without_map = wonderful_result
        .accounts
        .iter()
        .flat_map(|account| &account.matches)
        .filter(|match_record| match_record.map_name.is_none() && match_record.map_id.is_none())
        .map(|match_record| match_record.videos.len())
        .sum::<usize>();
    println!(
        "WonderfulDb baseline: account_files={} readable={} snapshot_files={} warnings={} matches={} official_videos={} snapshots={} videos_without_map={}",
        wonderful_account_file_count,
        wonderful_result.accounts.len(),
        wonderful_result.snapshot_accounts.len(),
        wonderful_result.warnings.len(),
        wonderful_match_count,
        wonderful_video_count,
        wonderful_snapshot_count,
        wonderful_videos_without_map
    );
    assert!(
        wonderful_account_file_count >= MINIMUM_WONDERFUL_ACCOUNT_FILES,
        "WonderfulDb regression input must include an account file or warning"
    );
    assert!(
        wonderful_result.snapshot_accounts.len() >= MINIMUM_WONDERFUL_SNAPSHOT_FILES,
        "WonderfulDb regression input must include a readable snapshot file"
    );
    assert!(
        wonderful_match_count >= MINIMUM_WONDERFUL_MATCHES,
        "WonderfulDb match count must not fall below the verified baseline"
    );
    assert!(
        wonderful_video_count >= MINIMUM_WONDERFUL_VIDEOS,
        "WonderfulDb video count must not fall below the verified baseline"
    );
    assert!(
        wonderful_snapshot_count >= MINIMUM_WONDERFUL_SNAPSHOTS,
        "WonderfulDb snapshot count must not fall below the verified baseline"
    );
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow Unix epoch")
        .as_nanos();
    let working_database_path = workspace_root.join(format!(
        ".tmp-real-scan-working-{}-{nonce}.sqlite3",
        std::process::id()
    ));
    assert_ne!(
        source_database_path, working_database_path,
        "source database and working database must be different files"
    );
    let _working_database_guard = WorkingDatabaseGuard::prepare(working_database_path.clone())
        .expect("stale working database files should be removable");
    copy_database_for_real_scan(&source_database_path, &working_database_path)
        .expect("real scan working database should be copied");
    println!("real scan database: workspace source copy and disposable working copy prepared");

    let connection =
        Connection::open(&working_database_path).expect("working database should open");
    db::initialize_schema(&connection).expect("schema should initialize");
    let summary = if custom_scan_root.is_some() {
        scanner::scan_custom_directory(&connection, scan_root).expect("real scan should run")
    } else {
        scanner::scan_default_aclos_library(&connection).expect("real scan should run")
    };

    println!(
        "real scan: sources={} groups={} metadata_matches={} enriched={} events={} warnings={}",
        summary.source_dir_count,
        summary.clip_group_count,
        summary.metadata_match_count,
        summary.metadata_enriched_clip_count,
        summary.metadata_event_count,
        summary.metadata_warning_count
    );
    println!("real scan warnings: count={}", summary.errors.len());

    let verification = verify_post_scan_metadata(&connection, wonderful_video_count, None);
    println!(
        "official metadata: records={wonderful_video_count} matched_local={} missing_local={} clip_events={} type_4={} type_6={} type_10={} invalid_type_4={} invalid_type_6={} invalid_type_10={}",
        verification.matched_official_video_count,
        verification.missing_official_video_count,
        verification.official_event_count,
        verification.matched_type_four_count,
        verification.matched_type_six_count,
        verification.matched_type_ten_count,
        verification.invalid_type_four_count,
        verification.invalid_type_six_count,
        verification.invalid_type_ten_count
    );
    let (
        official_clips,
        missing_kda,
        missing_scoreline,
        missing_map,
        missing_match_rows,
        invalid_recorded_times,
    ): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = connection
            .query_row(
                "
                SELECT
                    COUNT(*),
                    SUM(CASE WHEN NULLIF(TRIM(clip_metadata.kda), '') IS NULL THEN 1 ELSE 0 END),
                    SUM(CASE WHEN NULLIF(TRIM(clip_metadata.scoreline), '') IS NULL THEN 1 ELSE 0 END),
                    SUM(CASE
                        WHEN NULLIF(TRIM(clip_metadata.map_name), '') IS NULL
                         AND NULLIF(TRIM(matches.map_id), '') IS NULL
                        THEN 1 ELSE 0
                    END),
                    SUM(CASE
                        WHEN NULLIF(TRIM(clip_metadata.match_id), '') IS NOT NULL
                         AND matches.id IS NULL
                        THEN 1 ELSE 0
                    END),
                    SUM(CASE
                        WHEN LENGTH(TRIM(clips.recorded_at)) >= 12
                         AND TRIM(clips.recorded_at) NOT GLOB '*[^0-9]*'
                        THEN 1 ELSE 0
                    END)
                FROM clip_metadata
                JOIN clips ON clips.id = clip_metadata.clip_id
                LEFT JOIN matches ON matches.game_id = clip_metadata.match_id
                WHERE clip_metadata.metadata_source = 'wonderful_db'
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
                    ))
                },
            )
            .expect("recoverable official metadata should be measurable");
    println!(
        "recoverable official metadata: clips={official_clips} missing_kda={missing_kda} missing_scoreline={missing_scoreline} missing_map={missing_map} missing_match_rows={missing_match_rows} invalid_recorded_times={invalid_recorded_times}"
    );
    assert_eq!(
        missing_kda, 0,
        "all recoverable WonderfulDb KDA should import"
    );
    assert_eq!(
        missing_scoreline, 0,
        "all recoverable WonderfulDb scorelines should import"
    );
    assert_eq!(
        missing_map, wonderful_videos_without_map as i64,
        "only WonderfulDb videos whose source records lack map data may remain without maps"
    );
    assert_eq!(
        missing_match_rows, 0,
        "every matched WonderfulDb clip should link to its official match row"
    );
    assert_eq!(
        invalid_recorded_times, 0,
        "WonderfulDb millisecond timestamps should normalize to Unix seconds"
    );
    let internal_weapon_paths: i64 = connection
        .query_row(
            "
            SELECT COUNT(*)
            FROM clip_events
            JOIN clip_metadata ON clip_metadata.clip_id = clip_events.clip_id
            WHERE clip_metadata.metadata_source = 'wonderful_db'
              AND REPLACE(clip_events.weapon_name, '\\', '/') LIKE '/Game/%'
            ",
            [],
            |row| row.get(0),
        )
        .expect("WonderfulDb weapon names should be measurable");
    println!("WonderfulDb internal weapon paths: {internal_weapon_paths}");
    assert_eq!(
        internal_weapon_paths, 0,
        "WonderfulDb weapon names should not expose internal asset paths"
    );
    let (stored_snapshots, snapshot_matches): (i64, i64) = connection
        .query_row(
            "SELECT COUNT(*), COUNT(DISTINCT match_id) FROM match_snapshots",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("stored WonderfulDb snapshots should be measurable");
    println!("WonderfulDb stored snapshots: rows={stored_snapshots} matches={snapshot_matches}");
    assert_eq!(stored_snapshots, wonderful_snapshot_count as i64);
    assert!(snapshot_matches > 0, "snapshot matches must not be empty");

    let (wonderful_scores, recovered_scores, untrusted_scores): (i64, i64, i64) = connection
        .query_row(
            "
            SELECT
                SUM(CASE WHEN round_score_source = 'wonderful_db' THEN 1 ELSE 0 END),
                SUM(CASE WHEN round_score_source = 'highlight_log_delta' THEN 1 ELSE 0 END),
                SUM(CASE
                    WHEN NULLIF(TRIM(round_score), '') IS NOT NULL
                     AND COALESCE(round_score_source, '') NOT IN (
                         'wonderful_db',
                         'highlight_log_delta'
                     )
                    THEN 1 ELSE 0
                END)
            FROM clip_metadata
            ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("round score provenance should be measurable");
    println!(
        "round score provenance: wonderful_db={wonderful_scores} highlight_log_delta={recovered_scores} untrusted={untrusted_scores}"
    );
    assert!(
        recovered_scores > 0,
        "verified local logs should recover at least one missing official score"
    );
    assert_eq!(
        untrusted_scores, 0,
        "every stored score must retain trusted provenance"
    );
}
