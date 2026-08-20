//! NVIDIA pending-recording queue and manual classification import.
//!
//! Files under an `nvidia` source are never auto-indexed. A scan only records them here; the
//! user then fills account/map/agent classification and `import_pending_manual_clip` creates the
//! real clip, match and metadata rows in one transaction.

use std::{
    collections::HashSet,
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::super::{
    ensure_row_changed, find_clip_source_id_by_normalized_path, normalize_optional, normalize_path,
    readable_error, require_non_empty, stable_path_for_storage,
    upsert_scanned_clip_with_file_identity, ClipInput, DbResult, ManualClipImportInput,
    PendingManualClip, SavedClip,
};

const MANUAL_ACCOUNT_ID_PREFIX: &str = "manual-";
const EXISTING_ACCOUNT_KEY_PREFIX: &str = "match-account-";

/// Input for recording one scanned NVIDIA candidate in the pending queue.
#[derive(Debug, Clone, Copy)]
pub struct PendingManualClipInput<'a> {
    pub source_dir_id: i64,
    pub video_path: &'a str,
    pub file_name: &'a str,
    pub file_size: i64,
    pub modified_at: Option<&'a str>,
    pub source_relative_dir: &'a str,
}

pub fn list_pending_manual_clips(
    connection: &Connection,
    include_ignored: bool,
) -> DbResult<Vec<PendingManualClip>> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                pending_manual_clips.id,
                pending_manual_clips.source_dir_id,
                source_dirs.name,
                pending_manual_clips.file_path,
                pending_manual_clips.file_name,
                pending_manual_clips.size_bytes,
                pending_manual_clips.modified_at,
                pending_manual_clips.source_relative_dir,
                pending_manual_clips.ignored,
                pending_manual_clips.first_discovered_at
            FROM pending_manual_clips
            JOIN source_dirs ON source_dirs.id = pending_manual_clips.source_dir_id
            WHERE ?1 = 1 OR pending_manual_clips.ignored = 0
            ORDER BY pending_manual_clips.id ASC
            ",
        )
        .map_err(|error| readable_error("preparing pending clip list", error))?;
    let rows = statement
        .query_map(params![i64::from(include_ignored)], map_pending_manual_clip)
        .map_err(|error| readable_error("querying pending clip list", error))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading pending clip list", error))
}

fn map_pending_manual_clip(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingManualClip> {
    Ok(PendingManualClip {
        id: row.get(0)?,
        source_dir_id: row.get(1)?,
        source_dir_name: row.get(2)?,
        file_path: row.get(3)?,
        file_name: row.get(4)?,
        file_size: row.get(5)?,
        modified_at: row.get(6)?,
        source_relative_dir: row.get(7)?,
        ignored: row.get::<_, i64>(8)? != 0,
        first_discovered_at: row.get(9)?,
    })
}

/// Records a scanned NVIDIA candidate idempotently. Returns `true` when the row was newly
/// inserted; re-seen rows only refresh their last-seen timestamp and never reset the user's
/// `ignored` decision.
pub fn upsert_pending_manual_clip(
    connection: &Connection,
    input: PendingManualClipInput<'_>,
) -> DbResult<bool> {
    let video_path =
        stable_path_for_storage(require_non_empty(input.video_path, "pending clip path")?);
    let file_name = require_non_empty(input.file_name, "pending clip file name")?;
    let normalized_path = normalize_path(&video_path);

    if find_clip_source_id_by_normalized_path(connection, &normalized_path)?.is_some() {
        // An indexed clip already owns this physical file. Pending and indexed rows are mutually
        // exclusive; manual import is the only transition between them.
        return Ok(false);
    }

    let existing = connection
        .query_row(
            "SELECT id, source_dir_id FROM pending_manual_clips WHERE normalized_path = ?1",
            params![normalized_path],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|error| readable_error("reading pending clip row", error))?;

    if let Some((pending_id, owner_source_id)) = existing {
        if owner_source_id != input.source_dir_id {
            // A single file can only be claimed by one source; the scan coordinator already
            // guards ownership conflicts, and a foreign row must never be stolen.
            return Ok(false);
        }
        connection
            .execute(
                "
                UPDATE pending_manual_clips
                SET file_path = ?2,
                    file_name = ?3,
                    size_bytes = ?4,
                    modified_at = ?5,
                    source_relative_dir = ?6,
                    last_seen_at = CURRENT_TIMESTAMP,
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                ",
                params![
                    pending_id,
                    video_path,
                    file_name,
                    input.file_size,
                    input.modified_at,
                    input.source_relative_dir,
                ],
            )
            .map_err(|error| readable_error("refreshing pending clip row", error))?;
        return Ok(false);
    }

    connection
        .execute(
            "
            INSERT INTO pending_manual_clips (
                source_dir_id,
                file_path,
                normalized_path,
                file_name,
                extension,
                size_bytes,
                modified_at,
                source_relative_dir
            )
            VALUES (?1, ?2, ?3, ?4, 'mp4', ?5, ?6, ?7)
            ",
            params![
                input.source_dir_id,
                video_path,
                normalized_path,
                file_name,
                input.file_size,
                input.modified_at,
                input.source_relative_dir,
            ],
        )
        .map_err(|error| readable_error("inserting pending clip row", error))?;

    Ok(true)
}

pub fn find_pending_manual_clip_source_id_by_normalized_path(
    connection: &Connection,
    normalized_path: &str,
) -> DbResult<Option<i64>> {
    connection
        .query_row(
            "SELECT source_dir_id FROM pending_manual_clips WHERE normalized_path = ?1",
            params![normalized_path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| readable_error("reading pending clip source ownership", error))
}

/// After a complete scan of one source, removes pending rows whose files were no longer seen.
/// Same safety policy as missing-clip reconciliation: only a fully enumerable scan can clean up.
pub fn delete_missing_pending_manual_clips(
    connection: &Connection,
    source_dir_id: i64,
    seen_paths: &HashSet<String>,
) -> DbResult<usize> {
    let paths = {
        let mut statement = connection
            .prepare(
                "SELECT id, normalized_path FROM pending_manual_clips WHERE source_dir_id = ?1",
            )
            .map_err(|error| readable_error("preparing pending cleanup query", error))?;
        let rows = statement
            .query_map(params![source_dir_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| readable_error("querying pending cleanup", error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading pending cleanup", error))?
    };

    let mut deleted = 0usize;
    for (pending_id, normalized_path) in paths {
        if seen_paths.contains(&normalized_path) {
            continue;
        }
        connection
            .execute(
                "DELETE FROM pending_manual_clips WHERE id = ?1",
                params![pending_id],
            )
            .map_err(|error| readable_error("deleting stale pending clip", error))?;
        deleted += 1;
    }
    Ok(deleted)
}

pub fn set_pending_manual_clip_ignored(
    connection: &Connection,
    pending_id: i64,
    ignored: bool,
) -> DbResult<()> {
    let changed = connection
        .execute(
            "
            UPDATE pending_manual_clips
            SET ignored = ?2,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            ",
            params![pending_id, i64::from(ignored)],
        )
        .map_err(|error| readable_error("updating pending clip ignored flag", error))?;
    ensure_row_changed(changed, "updating pending clip ignored flag", pending_id)
}

/// Creates the clip, its manual metadata and a synthetic match row, then removes the pending
/// entry. Returns the new clip id. The caller keeps the pending row when this fails.
pub fn import_pending_manual_clip(
    connection: &Connection,
    pending_id: i64,
    input: &ManualClipImportInput,
) -> DbResult<i64> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting pending clip import", error))?;
    let pending = transaction
        .query_row(
            "
            SELECT
                source_dir_id,
                file_path,
                file_name,
                size_bytes,
                modified_at
            FROM pending_manual_clips
            WHERE id = ?1
            ",
            params![pending_id],
            |row| {
                Ok(PendingImportRow {
                    source_dir_id: row.get(0)?,
                    file_path: row.get(1)?,
                    file_name: row.get(2)?,
                    file_size: row.get(3)?,
                    modified_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| readable_error("reading pending clip for import", error))?
        .ok_or_else(|| format!("待录入视频不存在：{pending_id}"))?;

    let file_metadata = fs::metadata(Path::new(&pending.file_path))
        .map_err(|error| format!("无法读取待录入视频 {}：{error}", pending.file_path))?;
    if !file_metadata.is_file() {
        return Err(format!("待录入视频不是文件：{}", pending.file_path));
    }

    let normalized_path = normalize_path(&pending.file_path);
    if let Some(owner_source_id) =
        find_clip_source_id_by_normalized_path(&transaction, &normalized_path)?
    {
        if owner_source_id != pending.source_dir_id {
            return Err("该视频已归属于其他素材来源，无法从待录入队列覆盖".to_string());
        }
    }

    let account_name = require_non_empty(&input.account_name, "账户名称")?.to_string();
    let player_name = normalize_optional(input.player_name.as_deref()).map(str::to_owned);
    let agent_name = require_non_empty(&input.agent_name, "英雄名称")?.to_string();
    let map_name = Some(
        require_non_empty(input.map_name.as_deref().unwrap_or_default(), "地图名称")?.to_string(),
    );
    let game_mode = normalize_optional(input.game_mode.as_deref()).map(str::to_owned);
    let display_identity_name = player_name.clone().unwrap_or_else(|| account_name.clone());

    let account_id =
        resolve_manual_account_id(&transaction, &input.account_key, &display_identity_name)?;
    let game_id = unique_identifier("game", pending_id);

    let deleted = transaction
        .execute(
            "DELETE FROM pending_manual_clips WHERE id = ?1",
            params![pending_id],
        )
        .map_err(|error| readable_error("claiming pending clip for import", error))?;
    ensure_row_changed(deleted, "claiming pending clip for import", pending_id)?;

    let saved: SavedClip = upsert_scanned_clip_with_file_identity(
        &transaction,
        ClipInput {
            source_dir_id: pending.source_dir_id,
            clip_group_id: None,
            video_path: &pending.file_path,
            file_name: &pending.file_name,
            file_size: pending.file_size,
            modified_at: pending.modified_at.as_deref(),
            duration_ms: None,
            recorded_at: None,
            cover_path: None,
            cover_source: "missing",
        },
        None,
    )?;
    let clip_id = saved.clip.id;

    transaction
        .execute(
            "
            INSERT INTO matches (
                game_id,
                account_id,
                player_name,
                agent_name,
                map_name,
                game_mode,
                started_at,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)
            ON CONFLICT(game_id) DO UPDATE SET
                account_id = excluded.account_id,
                player_name = excluded.player_name,
                agent_name = excluded.agent_name,
                map_name = excluded.map_name,
                game_mode = excluded.game_mode,
                started_at = excluded.started_at,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                game_id,
                account_id,
                display_identity_name,
                agent_name,
                map_name,
                game_mode,
                pending.modified_at,
            ],
        )
        .map_err(|error| readable_error("inserting manual import match", error))?;

    transaction
        .execute(
            "
            INSERT INTO clip_metadata (
                clip_id,
                metadata_status,
                account_name,
                player_name,
                agent_name,
                map_name,
                game_mode,
                match_id,
                metadata_source,
                updated_at
            )
            VALUES (?1, 'manual', ?2, ?3, ?4, ?5, ?6, ?7, 'manual', CURRENT_TIMESTAMP)
            ON CONFLICT(clip_id) DO UPDATE SET
                metadata_status = 'manual',
                account_name = excluded.account_name,
                player_name = excluded.player_name,
                agent_name = excluded.agent_name,
                map_name = excluded.map_name,
                game_mode = excluded.game_mode,
                match_id = excluded.match_id,
                metadata_source = 'manual',
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                clip_id,
                account_name,
                player_name,
                agent_name,
                map_name,
                game_mode,
                game_id,
            ],
        )
        .map_err(|error| readable_error("inserting manual clip metadata", error))?;

    if let Some(note) = normalize_optional(input.note.as_deref()) {
        transaction
            .execute(
                "UPDATE clips SET note = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![clip_id, note],
            )
            .map_err(|error| readable_error("saving manual import note", error))?;
    }

    transaction
        .commit()
        .map_err(|error| readable_error("committing pending clip import", error))?;

    Ok(clip_id)
}

struct PendingImportRow {
    source_dir_id: i64,
    file_path: String,
    file_name: String,
    file_size: i64,
    modified_at: Option<String>,
}

fn resolve_manual_account_id(
    connection: &Connection,
    account_key: &Option<String>,
    display_identity_name: &str,
) -> DbResult<String> {
    if let Some(key) = account_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        if let Some(suffix) = key.strip_prefix(EXISTING_ACCOUNT_KEY_PREFIX) {
            let suffix = suffix.trim();
            if !suffix.is_empty() {
                return Ok(suffix.to_string());
            }
        }
        // `source-<id>` fallback accounts carry no portable identity; they become a new
        // manual account so NVIDIA clips group together instead of pretending to be that source.
    }

    if let Some(existing) = connection
        .query_row(
            "
            SELECT account_id
            FROM matches
            WHERE account_id LIKE ?1
              AND TRIM(COALESCE(player_name, '')) = ?2
            ORDER BY id ASC
            LIMIT 1
            ",
            params![
                format!("{MANUAL_ACCOUNT_ID_PREFIX}%"),
                display_identity_name
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| readable_error("reusing manual account", error))?
    {
        return Ok(existing);
    }

    Ok(unique_identifier(
        MANUAL_ACCOUNT_ID_PREFIX.trim_end_matches('-'),
        pending_salt(),
    ))
}

fn pending_salt() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or(0)
}

fn unique_identifier(prefix: &str, salt: i64) -> String {
    format!("{prefix}-{salt:x}-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, ScanMode, SourceDirInput, SourceKind};
    use std::fs;

    fn temp_fixture(label: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let fixture = std::env::temp_dir().join(format!(
            "vhm-pending-{label}-{}-{unique}",
            std::process::id()
        ));
        let data = fixture.join("data");
        let root = fixture.join("recordings");
        fs::create_dir_all(&data).expect("data directory should be created");
        fs::create_dir_all(&root).expect("recording root should be created");
        (fixture, data, root)
    }

    fn register_nvidia_source(database_path: &Path, root: &Path) -> db::SourceDir {
        let connection = db::open_database(database_path).expect("database should open");
        let canonical_root = root
            .canonicalize()
            .expect("recording root should canonicalize")
            .display()
            .to_string();
        db::register_source_dir(
            &connection,
            SourceDirInput {
                path: &canonical_root,
                name: "NVIDIA recordings",
            },
            db::SourceProfileInput {
                source_kind: SourceKind::Nvidia,
                scan_mode: ScanMode::RecursiveMp4,
                scan_root_path: &canonical_root,
            },
            true,
        )
        .expect("source should register")
    }

    fn register_generic_source(database_path: &Path, root: &Path) -> db::SourceDir {
        let connection = db::open_database(database_path).expect("database should open");
        let canonical_root = root
            .canonicalize()
            .expect("recording root should canonicalize")
            .display()
            .to_string();
        db::register_source_dir(
            &connection,
            SourceDirInput {
                path: &canonical_root,
                name: "Generic recordings",
            },
            db::SourceProfileInput {
                source_kind: SourceKind::Generic,
                scan_mode: ScanMode::RecursiveMp4,
                scan_root_path: &canonical_root,
            },
            true,
        )
        .expect("source should register")
    }

    fn fixture_video_path(source_id: i64, relative_dir: &str, file_name: &str) -> String {
        let mut path = std::path::PathBuf::from("D:\\NvidiaFixture");
        path.push(source_id.to_string());
        if !relative_dir.is_empty() {
            path.push(relative_dir);
        }
        path.push(file_name);
        path.display().to_string()
    }

    fn upsert_fixture(
        connection: &Connection,
        source_id: i64,
        file_name: &str,
        relative_dir: &str,
    ) -> bool {
        let video_path = fixture_video_path(source_id, relative_dir, file_name);
        upsert_pending_manual_clip(
            connection,
            PendingManualClipInput {
                source_dir_id: source_id,
                video_path: &video_path,
                file_name,
                file_size: 42,
                modified_at: Some("1782634272"),
                source_relative_dir: relative_dir,
            },
        )
        .expect("pending fixture should upsert")
    }

    fn import_input(account_key: Option<&str>, account_name: &str) -> ManualClipImportInput {
        ManualClipImportInput {
            account_key: account_key.map(str::to_string),
            account_name: account_name.to_string(),
            player_name: None,
            agent_name: "捷风".to_string(),
            map_name: Some("霓虹町".to_string()),
            game_mode: Some("竞技模式".to_string()),
            note: None,
        }
    }

    #[test]
    fn pending_rows_upsert_ignore_and_reconcile_idempotently() {
        let (fixture, data, root) = temp_fixture("queue");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        assert!(upsert_fixture(&connection, source.id, "a.mp4", ""));
        assert!(upsert_fixture(&connection, source.id, "b.mp4", "Valorant"));
        assert!(!upsert_fixture(&connection, source.id, "a.mp4", ""));
        assert_eq!(
            list_pending_manual_clips(&connection, false).unwrap().len(),
            2
        );

        let pending_id = list_pending_manual_clips(&connection, false).unwrap()[0].id;
        set_pending_manual_clip_ignored(&connection, pending_id, true).unwrap();
        assert_eq!(
            list_pending_manual_clips(&connection, false).unwrap().len(),
            1
        );
        assert_eq!(
            list_pending_manual_clips(&connection, true).unwrap().len(),
            2
        );

        // The ignored decision survives re-discovery.
        assert!(!upsert_fixture(&connection, source.id, "a.mp4", ""));
        assert!(
            list_pending_manual_clips(&connection, true)
                .unwrap()
                .iter()
                .find(|clip| clip.id == pending_id)
                .unwrap()
                .ignored
        );

        // Cleanup only drops rows that a complete scan did not see.
        let seen = std::collections::HashSet::from([normalize_path(&fixture_video_path(
            source.id, "", "a.mp4",
        ))]);
        assert_eq!(
            delete_missing_pending_manual_clips(&connection, source.id, &seen).unwrap(),
            1
        );
        assert_eq!(
            list_pending_manual_clips(&connection, true).unwrap().len(),
            1
        );

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn import_creates_manual_account_and_reuses_it_by_display_name() {
        let (fixture, data, root) = temp_fixture("account");
        fs::write(root.join("first.mp4"), b"first").expect("first file should be written");
        fs::write(root.join("second.mp4"), b"second").expect("second file should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        let first_path = root.join("first.mp4").display().to_string();
        let second_path = root.join("second.mp4").display().to_string();
        upsert_pending_manual_clip(
            &connection,
            PendingManualClipInput {
                source_dir_id: source.id,
                video_path: &first_path,
                file_name: "first.mp4",
                file_size: 5,
                modified_at: Some("1782634272"),
                source_relative_dir: "",
            },
        )
        .unwrap();
        upsert_pending_manual_clip(
            &connection,
            PendingManualClipInput {
                source_dir_id: source.id,
                video_path: &second_path,
                file_name: "second.mp4",
                file_size: 6,
                modified_at: Some("1782634272"),
                source_relative_dir: "",
            },
        )
        .unwrap();
        let pending_ids = list_pending_manual_clips(&connection, false)
            .unwrap()
            .into_iter()
            .map(|clip| clip.id)
            .collect::<Vec<_>>();

        let first_clip_id = import_pending_manual_clip(
            &connection,
            pending_ids[0],
            &import_input(None, "共享小号"),
        )
        .unwrap();
        let second_clip_id = import_pending_manual_clip(
            &connection,
            pending_ids[1],
            &import_input(None, "共享小号"),
        )
        .unwrap();

        let first = db::find_clip_by_id(&connection, first_clip_id).unwrap();
        let second = db::find_clip_by_id(&connection, second_clip_id).unwrap();
        assert_eq!(first.account_identity_key, second.account_identity_key);
        assert!(first
            .account_identity_key
            .starts_with("match-account-manual-"));
        assert_eq!(first.account_display_name, "共享小号");
        assert!(list_pending_manual_clips(&connection, true)
            .unwrap()
            .is_empty());

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn import_reuses_existing_match_account_key() {
        let (fixture, data, root) = temp_fixture("existing-key");
        fs::write(root.join("clip.mp4"), b"recording").expect("file should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");
        connection
            .execute(
                "INSERT INTO matches (game_id, account_id, player_name) VALUES ('legacy-match', '1001', 'OldName#1001')",
                [],
            )
            .unwrap();

        let video_path = root.join("clip.mp4").display().to_string();
        upsert_pending_manual_clip(
            &connection,
            PendingManualClipInput {
                source_dir_id: source.id,
                video_path: &video_path,
                file_name: "clip.mp4",
                file_size: 9,
                modified_at: Some("1782634272"),
                source_relative_dir: "",
            },
        )
        .unwrap();
        let pending_id = list_pending_manual_clips(&connection, false).unwrap()[0].id;
        let clip_id = import_pending_manual_clip(
            &connection,
            pending_id,
            &import_input(Some("match-account-1001"), "OldName#1001"),
        )
        .unwrap();
        let clip = db::find_clip_by_id(&connection, clip_id).unwrap();
        assert_eq!(clip.account_identity_key, "match-account-1001");
        assert_eq!(clip.account_display_name, "OldName#1001");

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn import_keeps_pending_row_when_the_video_file_is_missing() {
        let (fixture, data, root) = temp_fixture("missing-file");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        let video_path = root.join("gone.mp4").display().to_string();
        upsert_pending_manual_clip(
            &connection,
            PendingManualClipInput {
                source_dir_id: source.id,
                video_path: &video_path,
                file_name: "gone.mp4",
                file_size: 3,
                modified_at: Some("1782634272"),
                source_relative_dir: "",
            },
        )
        .unwrap();
        let pending_id = list_pending_manual_clips(&connection, false).unwrap()[0].id;

        let error =
            import_pending_manual_clip(&connection, pending_id, &import_input(None, "Tester"))
                .expect_err("missing video must block the import");
        assert!(error.contains("无法读取"), "unexpected error: {error}");
        assert_eq!(
            list_pending_manual_clips(&connection, false).unwrap().len(),
            1
        );
        assert!(db::list_clips(&connection).unwrap().is_empty());

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn import_requires_a_map_and_keeps_the_recording_pending_on_validation_failure() {
        let (fixture, data, root) = temp_fixture("required-map");
        fs::write(root.join("clip.mp4"), b"recording").expect("file should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        let video_path = root.join("clip.mp4").display().to_string();
        upsert_pending_manual_clip(
            &connection,
            PendingManualClipInput {
                source_dir_id: source.id,
                video_path: &video_path,
                file_name: "clip.mp4",
                file_size: 9,
                modified_at: Some("1782634272"),
                source_relative_dir: "",
            },
        )
        .unwrap();
        let pending_id = list_pending_manual_clips(&connection, false).unwrap()[0].id;
        let mut input = import_input(None, "Tester");
        input.map_name = None;

        let error = import_pending_manual_clip(&connection, pending_id, &input)
            .expect_err("a map is required before importing");
        assert!(error.contains("地图名称"), "unexpected error: {error}");
        assert_eq!(
            list_pending_manual_clips(&connection, false).unwrap().len(),
            1
        );
        assert!(db::list_clips(&connection).unwrap().is_empty());

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn pending_and_indexed_rows_cannot_steal_a_foreign_sources_path() {
        let (fixture, data, root) = temp_fixture("cross-table-ownership");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("nested root should be created");
        let pending_first_path = nested.join("pending-first.mp4");
        let indexed_first_path = nested.join("indexed-first.mp4");
        fs::write(&pending_first_path, b"pending first").expect("fixture should be written");
        fs::write(&indexed_first_path, b"indexed first").expect("fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let nvidia_source = register_nvidia_source(&database_path, &root);
        let generic_source = register_generic_source(&database_path, &nested);
        let connection = db::open_database(&database_path).expect("database should open");

        let pending_first = pending_first_path.display().to_string();
        assert!(upsert_pending_manual_clip(
            &connection,
            PendingManualClipInput {
                source_dir_id: nvidia_source.id,
                video_path: &pending_first,
                file_name: "pending-first.mp4",
                file_size: 13,
                modified_at: Some("1782634272"),
                source_relative_dir: "nested",
            },
        )
        .unwrap());
        let pending_id = list_pending_manual_clips(&connection, false).unwrap()[0].id;
        let shared_write_error = db::upsert_clip(
            &connection,
            db::ClipInput {
                source_dir_id: generic_source.id,
                clip_group_id: None,
                video_path: &pending_first,
                file_name: "pending-first.mp4",
                file_size: 13,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect_err("the shared clip writer must reject a foreign pending owner");
        assert!(
            shared_write_error.contains("reserved for manual NVIDIA import"),
            "unexpected error: {shared_write_error}"
        );
        assert!(db::list_clips(&connection).unwrap().is_empty());

        // Simulate a legacy database that already contains the pre-guard conflict so the manual
        // import defense is exercised independently from the shared writer above.
        let pending_first_normalized = normalize_path(&pending_first);
        let legacy_conflict_insert = "
            INSERT INTO clips (
                source_dir_id, file_path, normalized_path, file_name, size_bytes, modified_at
            ) VALUES (?1, ?2, ?3, 'pending-first.mp4', 13, '1782634272')
        ";
        let trigger_error = connection
            .execute(
                legacy_conflict_insert,
                params![generic_source.id, &pending_first, &pending_first_normalized],
            )
            .expect_err("the database trigger must reject a pending/indexed path conflict");
        assert!(
            trigger_error
                .to_string()
                .contains("clip path is pending manual import"),
            "unexpected error: {trigger_error}"
        );
        connection
            .execute_batch(
                "
                DROP TRIGGER prevent_clip_pending_path_on_insert;
                DROP TRIGGER prevent_clip_pending_path_on_update;
                DROP TRIGGER prevent_pending_indexed_path_on_insert;
                DROP TRIGGER prevent_pending_indexed_path_on_update;
                ",
            )
            .expect("legacy fixture should temporarily remove v17 exclusivity triggers");
        connection
            .execute(
                legacy_conflict_insert,
                params![generic_source.id, pending_first, pending_first_normalized],
            )
            .expect("legacy foreign indexed fixture should insert");
        let foreign_clip_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO clip_metadata (clip_id) VALUES (?1)",
                [foreign_clip_id],
            )
            .expect("legacy foreign clip metadata should insert");

        let error =
            import_pending_manual_clip(&connection, pending_id, &import_input(None, "Tester"))
                .expect_err("manual import must not steal a foreign indexed clip");
        assert!(error.contains("其他素材来源"), "unexpected error: {error}");
        assert_eq!(
            list_pending_manual_clips(&connection, false).unwrap().len(),
            1
        );
        assert_eq!(
            db::list_clips(&connection).unwrap()[0].source_dir_id,
            generic_source.id
        );

        let indexed_first = indexed_first_path.display().to_string();
        db::upsert_clip(
            &connection,
            db::ClipInput {
                source_dir_id: generic_source.id,
                clip_group_id: None,
                video_path: &indexed_first,
                file_name: "indexed-first.mp4",
                file_size: 13,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("indexed-first fixture should insert");
        assert!(
            !upsert_pending_manual_clip(
                &connection,
                PendingManualClipInput {
                    source_dir_id: nvidia_source.id,
                    video_path: &indexed_first,
                    file_name: "indexed-first.mp4",
                    file_size: 13,
                    modified_at: Some("1782634272"),
                    source_relative_dir: "nested",
                },
            )
            .unwrap(),
            "a foreign indexed clip must not be duplicated into the pending queue"
        );
        assert_eq!(
            list_pending_manual_clips(&connection, false).unwrap().len(),
            1
        );
        assert_eq!(db::list_clips(&connection).unwrap().len(), 2);

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }
}
