mod migrations;
mod models;
mod repositories;

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{backup::Backup, params, Connection, OpenFlags};
use tauri::{AppHandle, Manager};

pub use migrations::initialize_schema;
#[cfg(test)]
use migrations::SCHEMA_VERSION;
pub use models::{
    AccountIdentitySource, AccountNameHint, BatchClipMutationResult, Clip, ClipAgentAssetHint,
    ClipDetail, ClipEvent, ClipEventInput, ClipGroup, ClipGroupInput, ClipInput, ClipListQuery,
    ClipMetadataInput, ClipPage, ClipSaveOutcome, ClipSegmentInput, ClipSort, ClipSummary,
    FavoriteFilter, HighlightFilter, LibraryAccountFacet, LibraryFacetValue, LibraryFacets,
    LibrarySourceFacet, LibraryTagFacet, SavedClip, Source, SourceDir, SourceDirInput, Tag,
    ThumbnailCacheRef, ThumbnailEnsureResult, ThumbnailJob, ThumbnailQueueStatus,
    ThumbnailReconcileResult, ThumbnailStatus,
};
pub(crate) use models::{ClipFileTarget, ClipMediaPaths};
#[cfg(test)]
use repositories::clips::empty_batch_clip_mutation_result;
pub use repositories::clips::{
    add_tag_to_clips, delete_clip_from_index, find_clip_by_id, find_clip_detail_by_id,
    list_active_clip_paths_for_source, mark_clip_missing_by_normalized_path, remove_tag_from_clips,
    set_clips_favorite, set_clips_trashed, update_clip_favorite, update_clip_note,
    update_clip_trashed,
};
use repositories::clips::{find_clip_by_normalized_path, find_optional_clip_by_normalized_path};
pub(crate) use repositories::clips::{find_clip_file_target_by_id, find_clip_media_paths_by_id};
pub(crate) use repositories::deletions::{
    delete_clip_from_index_guarded, delete_clip_permanently, recover_pending_clip_deletions,
    set_clips_trashed_guarded, ClipDeleteItemOutcome,
};
pub(in crate::db) use repositories::library::{attach_clip_events, map_clip, CLIP_SELECT_SQL};
pub use repositories::library::{
    get_library_facets, list_clip_events_for_clip, list_clip_page, list_clips,
};
pub use repositories::sources::{
    list_sources, mark_source_dir_scan_error, mark_source_dir_scanned, upsert_clip_group,
    upsert_source_dir,
};
use repositories::tags::list_tags_for_clip;
pub use repositories::tags::{
    assign_tag_to_clip, create_tag, delete_tag, list_tags, remove_tag_from_clip, update_tag,
};
pub use repositories::thumbnails::{
    claim_next_thumbnail_job, complete_thumbnail_job_if_current, delete_orphan_thumbnail_rows,
    ensure_clip_thumbnails, fail_thumbnail_job_if_current, get_thumbnail_queue_status,
    get_thumbnail_status, list_ready_thumbnail_cache_refs, list_thumbnail_statuses,
    mark_pending_thumbnails_unavailable, mark_thumbnail_cache_missing_if_current,
    mark_thumbnail_evicted_if_current, reconcile_clip_thumbnails, recover_running_thumbnail_jobs,
    recover_unavailable_thumbnail_jobs, retry_clip_thumbnails, thumbnail_fingerprint,
};

const DATABASE_FILE_NAME: &str = "highlight-index.sqlite3";
const CLIP_NAME_COLLATION: &str = "VHM_CLIP_NAME";
pub const DEFAULT_CLIP_PAGE_LIMIT: i64 = 50;
pub const MAX_CLIP_PAGE_LIMIT: i64 = 200;

pub type DbResult<T> = Result<T, String>;

pub fn initialize_database(app: &AppHandle) -> DbResult<PathBuf> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| readable_error("locating app data directory", error))?;

    initialize_database_in(&data_dir)
}

/// Creates or upgrades the application database below a caller-selected data directory.
///
/// Production startup normally uses [`initialize_database`]. The explicit-directory form keeps
/// release smoke tests out of the signed-in user's real Known Folder, whose Windows resolution is
/// not affected by overriding `APPDATA` in a child process.
pub(crate) fn initialize_database_in(data_dir: impl AsRef<Path>) -> DbResult<PathBuf> {
    let data_dir = data_dir.as_ref();
    fs::create_dir_all(data_dir)
        .map_err(|error| readable_error("creating app data directory", error))?;

    let database_path = data_dir.join(DATABASE_FILE_NAME);
    migrate_database(&database_path)?;

    Ok(database_path)
}

/// Opens an existing database for ordinary reads and writes without running migrations.
pub fn open_database(database_path: impl AsRef<Path>) -> DbResult<Connection> {
    open_configured_database(database_path.as_ref(), OpenFlags::SQLITE_OPEN_READ_WRITE)
}

/// Opens an existing database for a short-lived query without permitting writes.
pub fn open_database_read_only(database_path: impl AsRef<Path>) -> DbResult<Connection> {
    open_configured_database(database_path.as_ref(), OpenFlags::SQLITE_OPEN_READ_ONLY)
}

/// Creates or upgrades the database during controlled startup or fixture setup.
pub fn migrate_database(database_path: impl AsRef<Path>) -> DbResult<()> {
    let database_path = database_path.as_ref();
    let database_existed = database_path.is_file();
    let connection = open_configured_database(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    let previous_schema_version = migrations::read_schema_user_version(&connection)?;

    if previous_schema_version > migrations::SCHEMA_VERSION {
        return Err(format!(
            "Database schema version {previous_schema_version} is newer than this build supports ({}); refusing to modify it.",
            migrations::SCHEMA_VERSION
        ));
    }

    if database_existed {
        verify_database_health(&connection, "before migration")?;
        if previous_schema_version < migrations::SCHEMA_VERSION {
            create_migration_backup(
                &connection,
                database_path,
                previous_schema_version,
                migrations::SCHEMA_VERSION,
            )?;
        }
    }

    initialize_schema(&connection)?;
    verify_database_health(&connection, "after migration")
}

fn verify_database_health(connection: &Connection, phase: &str) -> DbResult<()> {
    let mut quick_check = connection
        .prepare("PRAGMA quick_check(1)")
        .map_err(|error| readable_error(&format!("preparing {phase} quick_check"), error))?;
    let results = quick_check
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| readable_error(&format!("running {phase} quick_check"), error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error(&format!("collecting {phase} quick_check"), error))?;
    if results.len() != 1 || !results[0].eq_ignore_ascii_case("ok") {
        return Err(format!(
            "Database health check failed {phase}: {}",
            results.join("; ")
        ));
    }

    let mut foreign_key_check = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|error| readable_error(&format!("preparing {phase} foreign_key_check"), error))?;
    let violation = foreign_key_check
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| readable_error(&format!("running {phase} foreign_key_check"), error))?
        .next()
        .transpose()
        .map_err(|error| readable_error(&format!("reading {phase} foreign_key_check"), error))?;
    if let Some((table, row_id, parent)) = violation {
        return Err(format!(
            "Database foreign-key check failed {phase}: table {table}, row {row_id:?}, parent {parent}."
        ));
    }

    Ok(())
}

fn create_migration_backup(
    source: &Connection,
    database_path: &Path,
    from_version: i64,
    to_version: i64,
) -> DbResult<PathBuf> {
    let database_parent = database_path.parent().ok_or_else(|| {
        "Database path has no parent directory for migration backups.".to_string()
    })?;
    let backup_dir = database_parent.join("backups");
    fs::create_dir_all(&backup_dir)
        .map_err(|error| readable_error("creating migration backup directory", error))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_name =
        format!("highlight-index.pre-v{from_version}-to-v{to_version}-{timestamp:020}.sqlite3");
    let backup_path = backup_dir.join(file_name);
    let partial_path = backup_dir.join(format!(
        ".highlight-index-backup-{timestamp:020}-{}.part",
        std::process::id()
    ));

    let backup_result = (|| -> DbResult<()> {
        let mut destination = Connection::open_with_flags(
            &partial_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|error| readable_error("opening migration backup destination", error))?;
        {
            let backup = Backup::new(source, &mut destination)
                .map_err(|error| readable_error("starting online migration backup", error))?;
            backup
                .run_to_completion(64, Duration::from_millis(10), None)
                .map_err(|error| readable_error("writing online migration backup", error))?;
        }
        // The source uses WAL, and SQLite copies that persistent journal-mode flag into the
        // destination header. Publish a self-contained backup in DELETE mode so opening it never
        // depends on or creates sibling -wal/-shm files.
        destination
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(|error| readable_error("making migration backup self-contained", error))?;
        drop(destination);

        let verification =
            open_configured_database(&partial_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        verify_database_health(&verification, "for migration backup")?;
        drop(verification);

        remove_sqlite_sidecar_if_present(&partial_path, "-wal")?;
        remove_sqlite_sidecar_if_present(&partial_path, "-shm")?;

        fs::rename(&partial_path, &backup_path)
            .map_err(|error| readable_error("publishing verified migration backup", error))?;
        Ok(())
    })();

    if let Err(error) = backup_result {
        let _ = fs::remove_file(&partial_path);
        let _ = fs::remove_file(sqlite_sidecar_path(&partial_path, "-wal"));
        let _ = fs::remove_file(sqlite_sidecar_path(&partial_path, "-shm"));
        return Err(error);
    }

    prune_migration_backups(&backup_dir, 3);
    Ok(backup_path)
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut value = database_path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn remove_sqlite_sidecar_if_present(database_path: &Path, suffix: &str) -> DbResult<()> {
    let path = sqlite_sidecar_path(database_path, suffix);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(readable_error(
            &format!("removing migration backup sidecar '{}'", path.display()),
            error,
        )),
    }
}

fn prune_migration_backups(backup_dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(backup_dir) else {
        return;
    };
    let mut backups = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            (file_type.is_file()
                && name.starts_with("highlight-index.pre-v")
                && name.ends_with(".sqlite3"))
            .then(|| {
                let modified = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(UNIX_EPOCH);
                (modified, name, entry.path())
            })
        })
        .collect::<Vec<_>>();
    backups.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    for (_, _, path) in backups.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
}

fn open_configured_database(database_path: &Path, flags: OpenFlags) -> DbResult<Connection> {
    let connection = Connection::open_with_flags(database_path, flags)
        .map_err(|error| readable_error("opening SQLite file", error))?;
    configure_connection(&connection)?;
    Ok(connection)
}

fn configure_connection(connection: &Connection) -> DbResult<()> {
    connection
        .create_collation(CLIP_NAME_COLLATION, compare_clip_names)
        .map_err(|error| readable_error("registering clip name collation", error))?;
    connection
        .execute_batch(
            "
            PRAGMA busy_timeout = 5000;
            PRAGMA foreign_keys = ON;
            -- Permanent deletion commits a durable intent before touching the file. FULL is
            -- required here: WAL + NORMAL may acknowledge that commit without syncing the WAL,
            -- allowing a power loss to erase the authorization after the file was deleted.
            PRAGMA synchronous = FULL;
            ",
        )
        .map_err(|error| readable_error("configuring SQLite connection", error))
}

fn compare_clip_names(left: &str, right: &str) -> Ordering {
    let mut left_offset = 0;
    let mut right_offset = 0;

    loop {
        let left_chunk = next_natural_name_chunk(left, &mut left_offset);
        let right_chunk = next_natural_name_chunk(right, &mut right_offset);
        let ordering = match (left_chunk, right_chunk) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((left_chunk, true)), Some((right_chunk, true))) => {
                compare_numeric_name_chunks(left_chunk, right_chunk)
            }
            (Some((left_chunk, _)), Some((right_chunk, _))) => {
                left_chunk.to_lowercase().cmp(&right_chunk.to_lowercase())
            }
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
}

fn next_natural_name_chunk<'a>(value: &'a str, offset: &mut usize) -> Option<(&'a str, bool)> {
    if *offset >= value.len() {
        return None;
    }
    let starts_with_digit = value.as_bytes()[*offset].is_ascii_digit();
    let start = *offset;
    while *offset < value.len() {
        let character = value[*offset..]
            .chars()
            .next()
            .expect("offset should remain on a character boundary");
        if character.is_ascii_digit() != starts_with_digit {
            break;
        }
        *offset += character.len_utf8();
    }
    Some((&value[start..*offset], starts_with_digit))
}

fn compare_numeric_name_chunks(left: &str, right: &str) -> Ordering {
    let left_significant = left.trim_start_matches('0');
    let right_significant = right.trim_start_matches('0');
    let left_significant = if left_significant.is_empty() {
        "0"
    } else {
        left_significant
    };
    let right_significant = if right_significant.is_empty() {
        "0"
    } else {
        right_significant
    };

    left_significant
        .len()
        .cmp(&right_significant.len())
        .then_with(|| left_significant.cmp(right_significant))
}

pub fn upsert_clip(connection: &Connection, input: ClipInput<'_>) -> DbResult<Clip> {
    Ok(upsert_scanned_clip(connection, input)?.clip)
}

pub fn upsert_scanned_clip(connection: &Connection, input: ClipInput<'_>) -> DbResult<SavedClip> {
    let video_path = require_non_empty(input.video_path, "clip video path")?;
    let file_name = require_non_empty(input.file_name, "clip file name")?;
    let cover_source = require_non_empty(input.cover_source, "cover source")?;
    let normalized_path = normalize_path(video_path);
    let extension = extension_from_file_name(file_name);
    let normalized_cover_path = normalize_optional(input.cover_path);
    let existing = find_optional_clip_by_normalized_path(connection, &normalized_path)?;
    let modified_at = input.modified_at.map(str::to_string);
    let cover_path = normalized_cover_path.map(str::to_string);

    let outcome = match &existing {
        None => ClipSaveOutcome::Inserted,
        Some(existing)
            if scanned_clip_changed(existing, &input, &extension, &modified_at, &cover_path) =>
        {
            ClipSaveOutcome::Updated
        }
        Some(_) => ClipSaveOutcome::Unchanged,
    };

    connection
        .execute(
            "
            INSERT INTO clips (
                source_dir_id,
                clip_group_id,
                file_path,
                normalized_path,
                file_name,
                extension,
                size_bytes,
                modified_at,
                duration_ms,
                recorded_at,
                cover_path,
                cover_source,
                file_status
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'available')
            ON CONFLICT(normalized_path) DO UPDATE SET
                source_dir_id = excluded.source_dir_id,
                clip_group_id = excluded.clip_group_id,
                file_path = excluded.file_path,
                file_name = excluded.file_name,
                extension = excluded.extension,
                size_bytes = excluded.size_bytes,
                modified_at = excluded.modified_at,
                duration_ms = CASE
                    WHEN excluded.duration_ms IS NULL
                        AND EXISTS (
                            SELECT 1
                            FROM clip_metadata
                            WHERE clip_metadata.clip_id = clips.id
                                AND clip_metadata.metadata_source = 'wonderful_db'
                        )
                    THEN clips.duration_ms
                    ELSE excluded.duration_ms
                END,
                recorded_at = CASE
                    WHEN excluded.recorded_at IS NULL
                        AND EXISTS (
                            SELECT 1
                            FROM clip_metadata
                            WHERE clip_metadata.clip_id = clips.id
                                AND clip_metadata.metadata_source = 'wonderful_db'
                        )
                    THEN clips.recorded_at
                    ELSE excluded.recorded_at
                END,
                cover_path = excluded.cover_path,
                cover_source = excluded.cover_source,
                file_status = CASE
                    WHEN clips.file_status = 'trashed' THEN 'trashed'
                    ELSE 'available'
                END,
                last_seen_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                input.source_dir_id,
                input.clip_group_id,
                video_path,
                normalized_path,
                file_name,
                extension,
                input.file_size,
                input.modified_at,
                input.duration_ms,
                input.recorded_at,
                normalized_cover_path,
                cover_source,
            ],
        )
        .map_err(|error| readable_error("saving clip", error))?;

    let clip = find_clip_by_normalized_path(connection, &normalized_path)?;
    connection
        .execute(
            "
            INSERT INTO clip_metadata (clip_id)
            VALUES (?1)
            ON CONFLICT(clip_id) DO NOTHING
            ",
            params![clip.id],
        )
        .map_err(|error| readable_error("initializing clip metadata", error))?;

    Ok(SavedClip { clip, outcome })
}

pub fn upsert_clip_metadata(connection: &Connection, input: ClipMetadataInput<'_>) -> DbResult<()> {
    connection
        .execute(
            "
            INSERT INTO clip_metadata (
                clip_id,
                metadata_status,
                json_path,
                account_name,
                player_name,
                agent_name,
                map_name,
                game_mode,
                scoreline,
                kda,
                extracted_text,
                parse_error,
                metadata_source
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                CASE
                    WHEN NULLIF(TRIM(?3), '') IS NOT NULL THEN 'video_export'
                    ELSE NULL
                END
            )
            ON CONFLICT(clip_id) DO UPDATE SET
                metadata_status = excluded.metadata_status,
                json_path = excluded.json_path,
                account_name = excluded.account_name,
                player_name = excluded.player_name,
                agent_name = excluded.agent_name,
                map_name = excluded.map_name,
                game_mode = excluded.game_mode,
                scoreline = excluded.scoreline,
                kda = excluded.kda,
                extracted_text = excluded.extracted_text,
                parse_error = excluded.parse_error,
                metadata_source = CASE
                    WHEN excluded.json_path IS NOT NULL THEN 'video_export'
                    ELSE clip_metadata.metadata_source
                END,
                updated_at = CURRENT_TIMESTAMP
            WHERE COALESCE(clip_metadata.metadata_source, '') <> 'wonderful_db'
            ",
            params![
                input.clip_id,
                input.metadata_status,
                normalize_optional(input.json_path),
                normalize_optional(input.account_name),
                normalize_optional(input.player_name),
                normalize_optional(input.agent_name),
                normalize_optional(input.map_name),
                normalize_optional(input.game_mode),
                normalize_optional(input.scoreline),
                normalize_optional(input.kda),
                normalize_optional(input.extracted_text),
                normalize_optional(input.parse_error),
            ],
        )
        .map_err(|error| readable_error("saving clip metadata", error))?;

    Ok(())
}

pub fn update_video_export_classification(
    connection: &Connection,
    clip_id: i64,
    highlight_type: Option<i64>,
    kill_count: Option<i64>,
) -> DbResult<()> {
    connection
        .execute(
            "
            UPDATE clip_metadata
            SET highlight_type = ?2,
                kill_count = ?3,
                updated_at = CURRENT_TIMESTAMP
            WHERE clip_id = ?1
              AND metadata_source = 'video_export'
            ",
            params![clip_id, highlight_type, kill_count],
        )
        .map_err(|error| readable_error("updating video export classification", error))?;

    Ok(())
}

pub fn replace_clip_timeline(
    connection: &Connection,
    clip_id: i64,
    segments: &[ClipSegmentInput<'_>],
    events: &[ClipEventInput<'_>],
) -> DbResult<()> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting clip timeline transaction", error))?;

    transaction
        .execute(
            "DELETE FROM clip_events WHERE clip_id = ?1",
            params![clip_id],
        )
        .map_err(|error| readable_error("deleting old clip events", error))?;
    transaction
        .execute(
            "DELETE FROM clip_segments WHERE clip_id = ?1",
            params![clip_id],
        )
        .map_err(|error| readable_error("deleting old clip segments", error))?;

    let mut segment_ids = HashMap::new();
    for segment in segments {
        transaction
            .execute(
                "
                INSERT INTO clip_segments (
                    clip_id,
                    segment_key,
                    round_id,
                    start_ms,
                    duration_ms,
                    game_start_ms,
                    game_end_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ",
                params![
                    clip_id,
                    segment.segment_key,
                    segment.round_id,
                    segment.start_ms,
                    segment.duration_ms,
                    segment.game_start_ms,
                    segment.game_end_ms,
                ],
            )
            .map_err(|error| readable_error("inserting clip segment", error))?;
        segment_ids.insert(segment.segment_key, transaction.last_insert_rowid());
    }

    for event in events {
        let segment_id = match event.segment_key {
            Some(segment_key) => Some(
                segment_ids
                    .get(segment_key)
                    .copied()
                    .ok_or_else(|| format!("clip segment key {segment_key} was not found"))?,
            ),
            None => None,
        };
        transaction
            .execute(
                "
                INSERT INTO clip_events (
                    clip_id,
                    segment_id,
                    event_key,
                    event_type,
                    video_time_ms,
                    event_time,
                    round_id,
                    player_name,
                    agent_name,
                    weapon_name,
                    killer_name,
                    killed_name,
                    killer_is_me,
                    raw_json
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                    ?8, ?9, ?10, ?11, ?12, ?13, ?14
                )
                ",
                params![
                    clip_id,
                    segment_id,
                    event.event_key,
                    event.event_type,
                    event.video_time_ms,
                    event.event_time,
                    event.round_id,
                    event.player_name,
                    event.agent_name,
                    event.weapon_name,
                    event.killer_name,
                    event.killed_name,
                    bool_to_integer(event.killer_is_me),
                    event.raw_json,
                ],
            )
            .map_err(|error| readable_error("inserting clip event", error))?;
    }

    transaction
        .commit()
        .map_err(|error| readable_error("committing clip timeline transaction", error))
}

pub fn clear_invalid_display_metadata(connection: &Connection) -> DbResult<usize> {
    let clip_changes = connection
        .execute(
            "
            UPDATE clip_metadata
            SET account_name = CASE
                    WHEN LOWER(REPLACE(COALESCE(account_name, ''), '\\', '/')) LIKE 'cards/%'
                        OR LOWER(REPLACE(COALESCE(account_name, ''), '\\', '/')) LIKE '%/cards/%'
                        OR LOWER(COALESCE(account_name, '')) LIKE '%playercards/%'
                        OR LOWER(COALESCE(account_name, '')) LIKE 'http%'
                        OR LOWER(COALESCE(account_name, '')) LIKE '%.png'
                        OR LOWER(COALESCE(account_name, '')) LIKE '%.jpg'
                        OR LOWER(COALESCE(account_name, '')) LIKE '%.jpeg'
                        OR LOWER(COALESCE(account_name, '')) LIKE '%.webp'
                    THEN NULL
                    ELSE account_name
                END,
                player_name = CASE
                    WHEN LOWER(REPLACE(COALESCE(player_name, ''), '\\', '/')) LIKE 'cards/%'
                        OR LOWER(REPLACE(COALESCE(player_name, ''), '\\', '/')) LIKE '%/cards/%'
                        OR LOWER(COALESCE(player_name, '')) LIKE '%playercards/%'
                        OR LOWER(COALESCE(player_name, '')) LIKE 'http%'
                        OR LOWER(COALESCE(player_name, '')) LIKE '%.png'
                        OR LOWER(COALESCE(player_name, '')) LIKE '%.jpg'
                        OR LOWER(COALESCE(player_name, '')) LIKE '%.jpeg'
                        OR LOWER(COALESCE(player_name, '')) LIKE '%.webp'
                    THEN NULL
                    ELSE player_name
                END,
                updated_at = CURRENT_TIMESTAMP
            WHERE COALESCE(metadata_source, '') <> 'wonderful_db'
                AND (
                    LOWER(REPLACE(COALESCE(account_name, ''), '\\', '/')) LIKE 'cards/%'
                    OR LOWER(REPLACE(COALESCE(account_name, ''), '\\', '/')) LIKE '%/cards/%'
                    OR LOWER(COALESCE(account_name, '')) LIKE '%playercards/%'
                    OR LOWER(COALESCE(account_name, '')) LIKE 'http%'
                    OR LOWER(COALESCE(account_name, '')) LIKE '%.png'
                    OR LOWER(COALESCE(account_name, '')) LIKE '%.jpg'
                    OR LOWER(COALESCE(account_name, '')) LIKE '%.jpeg'
                    OR LOWER(COALESCE(account_name, '')) LIKE '%.webp'
                    OR LOWER(REPLACE(COALESCE(player_name, ''), '\\', '/')) LIKE 'cards/%'
                    OR LOWER(REPLACE(COALESCE(player_name, ''), '\\', '/')) LIKE '%/cards/%'
                    OR LOWER(COALESCE(player_name, '')) LIKE '%playercards/%'
                    OR LOWER(COALESCE(player_name, '')) LIKE 'http%'
                    OR LOWER(COALESCE(player_name, '')) LIKE '%.png'
                    OR LOWER(COALESCE(player_name, '')) LIKE '%.jpg'
                    OR LOWER(COALESCE(player_name, '')) LIKE '%.jpeg'
                    OR LOWER(COALESCE(player_name, '')) LIKE '%.webp'
                )
            ",
            [],
        )
        .map_err(|error| readable_error("clearing asset-like account metadata", error))?;

    let untagged_clip_changes = connection
        .execute(
            "
            UPDATE clip_metadata
            SET account_name = CASE
                    WHEN account_name IS NOT NULL
                        AND TRIM(account_name) <> ''
                        AND account_name NOT LIKE '%#%'
                    THEN NULL
                    ELSE account_name
                END,
                player_name = CASE
                    WHEN player_name IS NOT NULL
                        AND TRIM(player_name) <> ''
                        AND player_name NOT LIKE '%#%'
                    THEN NULL
                    ELSE player_name
                END,
                updated_at = CURRENT_TIMESTAMP
            WHERE metadata_status = 'enriched'
                AND COALESCE(metadata_source, '') <> 'wonderful_db'
                AND (
                    (
                        account_name IS NOT NULL
                        AND TRIM(account_name) <> ''
                        AND account_name NOT LIKE '%#%'
                    )
                    OR (
                        player_name IS NOT NULL
                        AND TRIM(player_name) <> ''
                        AND player_name NOT LIKE '%#%'
                    )
                )
            ",
            [],
        )
        .map_err(|error| readable_error("clearing untagged enriched account metadata", error))?;

    let match_changes = connection
        .execute(
            "
            UPDATE matches
            SET game_mode = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE TRIM(COALESCE(game_mode, '')) GLOB '[0-9]*'
            ",
            [],
        )
        .map_err(|error| readable_error("clearing invalid match display metadata", error))?;

    let untagged_match_changes = connection
        .execute(
            "
            UPDATE matches
            SET player_name = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE source_log = 1
                AND source_leveldb = 0
                AND player_name IS NOT NULL
                AND TRIM(player_name) <> ''
                AND player_name NOT LIKE '%#%'
            ",
            [],
        )
        .map_err(|error| readable_error("clearing untagged log match player names", error))?;

    Ok(clip_changes + untagged_clip_changes + match_changes + untagged_match_changes)
}

pub fn clear_mismatched_match_metadata(connection: &Connection) -> DbResult<usize> {
    connection
        .execute(
            "
            UPDATE clip_metadata
            SET metadata_status = 'not_found',
                account_name = NULL,
                player_name = NULL,
                agent_name = NULL,
                map_name = NULL,
                game_mode = NULL,
                match_id = NULL,
                round_label = NULL,
                scoreline = NULL,
                kda = NULL,
                weapon_name = NULL,
                kill_count = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE metadata_status = 'enriched'
                AND COALESCE(metadata_source, '') <> 'wonderful_db'
                AND EXISTS (
                    SELECT 1
                    FROM clips
                    JOIN source_dirs
                        ON source_dirs.id = clips.source_dir_id
                    JOIN matches
                        ON matches.game_id = clip_metadata.match_id
                    WHERE clips.id = clip_metadata.clip_id
                        AND (
                            matches.account_id IS NULL
                            OR TRIM(matches.account_id) = ''
                            OR matches.account_id <> SUBSTR(
                                source_dirs.name,
                                LENGTH('wonderfulVideos') + 1
                            )
                        )
                )
            ",
            [],
        )
        .map_err(|error| readable_error("clearing mismatched match metadata", error))
}

pub fn clear_weak_account_name_hints_for_source_root(
    connection: &Connection,
    source_root: &Path,
) -> DbResult<usize> {
    let Some((root_exact, root_children)) = source_root_match_patterns(Some(source_root)) else {
        return Ok(0);
    };

    connection
        .execute(
            "
            UPDATE clip_metadata
            SET account_name = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE account_name IS NOT NULL
                AND TRIM(account_name) <> ''
                AND (player_name IS NULL OR TRIM(player_name) = '')
                AND match_id IS NULL
                AND metadata_status <> 'enriched'
                AND clip_id IN (
                    SELECT clips.id
                    FROM clips
                    JOIN source_dirs
                        ON source_dirs.id = clips.source_dir_id
                    WHERE LOWER(REPLACE(source_dirs.path, '\\', '/')) = ?1
                        OR LOWER(REPLACE(source_dirs.path, '\\', '/')) LIKE ?2 ESCAPE '!'
                )
            ",
            params![root_exact, root_children],
        )
        .map_err(|error| readable_error("clearing weak account name hints for source root", error))
}

pub fn backfill_agent_names_from_export_text(connection: &Connection) -> DbResult<usize> {
    connection
        .execute(
            "
            UPDATE clip_metadata
            SET agent_name = 'Reyna',
                updated_at = CURRENT_TIMESTAMP
            WHERE agent_name IS NULL
                AND (
                    LOWER(REPLACE(COALESCE(extracted_text, ''), '\\', '/'))
                        LIKE '%agentbackground/agent/11.png%'
                    OR LOWER(REPLACE(COALESCE(extracted_text, ''), '\\', '/'))
                        LIKE '%agentskill/11_%'
                )
            ",
            [],
        )
        .map_err(|error| readable_error("backfilling agent names from export text", error))
}

pub fn backfill_agent_names_from_asset_hints(
    connection: &Connection,
    hints: &[ClipAgentAssetHint],
    window_seconds: i64,
) -> DbResult<usize> {
    if hints.is_empty() {
        return Ok(0);
    }

    let candidate_groups = candidate_groups_missing_agent(connection)?;
    let mut changed = 0usize;

    for group in candidate_groups {
        let agents = hints
            .iter()
            .filter(|hint| hint.source_dir_name == group.source_dir_name)
            .filter(|hint| (hint.observed_at - group.modified_at).abs() <= window_seconds)
            .filter_map(|hint| {
                let trimmed = hint.agent_name.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .collect::<HashSet<_>>();

        if agents.len() != 1 {
            continue;
        }

        let agent_name = agents.into_iter().next().unwrap_or_default();
        let update_count = connection
            .execute(
                "
                UPDATE clip_metadata
                SET agent_name = ?2,
                    metadata_status = CASE
                        WHEN metadata_status IN ('not_found', 'failed') THEN 'partial'
                        ELSE metadata_status
                    END,
                    updated_at = CURRENT_TIMESTAMP
                WHERE clip_id IN (
                    SELECT id
                    FROM clips
                    WHERE clip_group_id = ?1
                )
                    AND (agent_name IS NULL OR TRIM(agent_name) = '')
                ",
                params![group.clip_group_id, agent_name],
            )
            .map_err(|error| readable_error("backfilling agent names from asset hints", error))?;
        changed += update_count;
    }

    Ok(changed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentBackfillGroup {
    source_dir_name: String,
    clip_group_id: i64,
    modified_at: i64,
}

fn candidate_groups_missing_agent(connection: &Connection) -> DbResult<Vec<AgentBackfillGroup>> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                source_dirs.name,
                clip_groups.id,
                MIN(CAST(clips.modified_at AS INTEGER)) AS group_modified_at
            FROM clips
            JOIN source_dirs
                ON source_dirs.id = clips.source_dir_id
            JOIN clip_groups
                ON clip_groups.id = clips.clip_group_id
            JOIN clip_metadata
                ON clip_metadata.clip_id = clips.id
            WHERE clips.clip_group_id IS NOT NULL
                AND (clip_metadata.agent_name IS NULL OR TRIM(clip_metadata.agent_name) = '')
            GROUP BY source_dirs.name, clip_groups.id
            ",
        )
        .map_err(|error| readable_error("preparing asset hint backfill query", error))?;

    let rows = statement
        .query_map([], |row| {
            Ok(AgentBackfillGroup {
                source_dir_name: row.get(0)?,
                clip_group_id: row.get(1)?,
                modified_at: row.get(2)?,
            })
        })
        .map_err(|error| readable_error("querying asset hint backfill groups", error))?;

    let mut groups = Vec::new();
    for row in rows {
        groups
            .push(row.map_err(|error| readable_error("reading asset hint backfill group", error))?);
    }

    Ok(groups)
}

pub fn propagate_known_account_names(
    connection: &Connection,
    source_root: Option<&Path>,
) -> DbResult<usize> {
    let root_patterns = source_root_match_patterns(source_root);
    let root_filter = if root_patterns.is_some() {
        "
                AND EXISTS (
                    SELECT 1
                    FROM clips
                    JOIN source_dirs
                        ON source_dirs.id = clips.source_dir_id
                    WHERE clips.id = clip_metadata.clip_id
                        AND (
                            LOWER(REPLACE(source_dirs.path, '\\', '/')) = ?1
                            OR LOWER(REPLACE(source_dirs.path, '\\', '/')) LIKE ?2 ESCAPE '!'
                        )
                )
        "
    } else {
        ""
    };
    let sql = format!(
        "
            UPDATE clip_metadata
            SET account_name = (
                    SELECT known_accounts.account_name
                    FROM clips
                    JOIN source_dirs
                        ON source_dirs.id = clips.source_dir_id
                    JOIN (
                        SELECT account_id, MIN(player_name) AS account_name
                        FROM matches
                        WHERE account_id IS NOT NULL
                            AND TRIM(account_id) <> ''
                            AND player_name LIKE '%#%'
                        GROUP BY account_id
                        HAVING COUNT(DISTINCT player_name) = 1
                    ) AS known_accounts
                        ON known_accounts.account_id = SUBSTR(
                            source_dirs.name,
                            LENGTH('wonderfulVideos') + 1
                        )
                    WHERE clips.id = clip_metadata.clip_id
                    LIMIT 1
                ),
                updated_at = CURRENT_TIMESTAMP
            WHERE EXISTS (
                    SELECT 1
                    FROM clips
                    JOIN source_dirs
                        ON source_dirs.id = clips.source_dir_id
                    JOIN (
                        SELECT account_id, MIN(player_name) AS account_name
                        FROM matches
                        WHERE account_id IS NOT NULL
                            AND TRIM(account_id) <> ''
                            AND player_name LIKE '%#%'
                        GROUP BY account_id
                        HAVING COUNT(DISTINCT player_name) = 1
                    ) AS known_accounts
                        ON known_accounts.account_id = SUBSTR(
                            source_dirs.name,
                            LENGTH('wonderfulVideos') + 1
                        )
                    WHERE clips.id = clip_metadata.clip_id
                )
                {root_filter}
                AND (
                    COALESCE(metadata_source, '') <> 'wonderful_db'
                    OR account_name IS NULL
                    OR TRIM(account_name) = ''
                )
                AND (
                    account_name IS NULL
                    OR account_name <> (
                        SELECT known_accounts.account_name
                        FROM clips
                        JOIN source_dirs
                            ON source_dirs.id = clips.source_dir_id
                        JOIN (
                            SELECT account_id, MIN(player_name) AS account_name
                            FROM matches
                            WHERE account_id IS NOT NULL
                                AND TRIM(account_id) <> ''
                                AND player_name LIKE '%#%'
                            GROUP BY account_id
                            HAVING COUNT(DISTINCT player_name) = 1
                        ) AS known_accounts
                            ON known_accounts.account_id = SUBSTR(
                                source_dirs.name,
                                LENGTH('wonderfulVideos') + 1
                            )
                        WHERE clips.id = clip_metadata.clip_id
                        LIMIT 1
                    )
                )
            "
    );

    match root_patterns {
        Some((root_exact, root_children)) => connection
            .execute(&sql, params![root_exact, root_children])
            .map_err(|error| readable_error("propagating known account names", error)),
        None => connection
            .execute(&sql, [])
            .map_err(|error| readable_error("propagating known account names", error)),
    }
}

pub fn propagate_account_name_hints(
    connection: &Connection,
    hints: &[AccountNameHint],
    source_root: Option<&Path>,
) -> DbResult<usize> {
    propagate_account_name_hints_with_authority(connection, hints, source_root, false)
}

pub fn propagate_authoritative_account_name_hints(
    connection: &Connection,
    hints: &[AccountNameHint],
    source_root: Option<&Path>,
) -> DbResult<usize> {
    propagate_account_name_hints_with_authority(connection, hints, source_root, true)
}

fn propagate_account_name_hints_with_authority(
    connection: &Connection,
    hints: &[AccountNameHint],
    source_root: Option<&Path>,
    authoritative: bool,
) -> DbResult<usize> {
    let mut names_by_account = HashMap::new();
    for hint in hints {
        let account_id = hint.account_id.trim();
        let account_name = hint.account_name.trim();
        if !looks_like_source_account_id(account_id) || !account_name.contains('#') {
            continue;
        }

        names_by_account
            .entry(account_id.to_string())
            .or_insert_with(|| account_name.to_string());
    }

    let mut changed = 0usize;
    let root_patterns = source_root_match_patterns(source_root);
    for (account_id, account_name) in names_by_account {
        let source_dir_name = format!("wonderfulVideos{account_id}");
        changed += connection
            .execute(
                "
                UPDATE matches
                SET player_name = ?2,
                    updated_at = CURRENT_TIMESTAMP
                WHERE account_id = ?1
                    AND player_name IS NOT ?2
                    AND (
                        ?3 = 1
                        OR player_name IS NULL
                        OR TRIM(player_name) = ''
                        OR player_name LIKE '%#%'
                        )
                ",
                params![account_id, account_name, i64::from(authoritative)],
            )
            .map_err(|error| readable_error("repairing match account names from hints", error))?;

        let update_count = if let Some((root_exact, root_children)) = root_patterns.as_ref() {
            connection
                .execute(
                    "
                    UPDATE clip_metadata
                    SET account_name = ?2,
                        updated_at = CURRENT_TIMESTAMP
                    WHERE (account_name IS NULL OR account_name <> ?2)
                        AND clip_id IN (
                            SELECT clips.id
                            FROM clips
                            JOIN source_dirs
                                ON source_dirs.id = clips.source_dir_id
                            WHERE source_dirs.name = ?1
                                AND (
                                    LOWER(REPLACE(source_dirs.path, '\\', '/')) = ?3
                                    OR LOWER(REPLACE(source_dirs.path, '\\', '/')) LIKE ?4 ESCAPE '!'
                                )
                        )
                    ",
                    params![source_dir_name, account_name, root_exact, root_children],
                )
                .map_err(|error| readable_error("propagating account name hints", error))?
        } else {
            connection
                .execute(
                    "
                UPDATE clip_metadata
                SET account_name = ?2,
                    updated_at = CURRENT_TIMESTAMP
                WHERE (account_name IS NULL OR account_name <> ?2)
                    AND clip_id IN (
                        SELECT clips.id
                        FROM clips
                        JOIN source_dirs
                            ON source_dirs.id = clips.source_dir_id
                        WHERE source_dirs.name = ?1
                    )
                ",
                    params![source_dir_name, account_name],
                )
                .map_err(|error| readable_error("propagating account name hints", error))?
        };
        changed += update_count;

        let player_update_count = if let Some((root_exact, root_children)) = root_patterns.as_ref()
        {
            connection
                .execute(
                    "
                    UPDATE clip_metadata
                    SET player_name = ?2,
                        updated_at = CURRENT_TIMESTAMP
                    WHERE player_name IS NOT ?2
                        AND (
                            ?5 = 1
                            OR player_name IS NULL
                            OR TRIM(player_name) = ''
                            OR player_name LIKE '%#%'
                            )
                        AND clip_id IN (
                            SELECT clips.id
                            FROM clips
                            JOIN source_dirs
                                ON source_dirs.id = clips.source_dir_id
                            WHERE source_dirs.name = ?1
                                AND (
                                    LOWER(REPLACE(source_dirs.path, '\\', '/')) = ?3
                                    OR LOWER(REPLACE(source_dirs.path, '\\', '/')) LIKE ?4 ESCAPE '!'
                                )
                        )
                    ",
                    params![
                        source_dir_name,
                        account_name,
                        root_exact,
                        root_children,
                        i64::from(authoritative)
                    ],
                )
                .map_err(|error| readable_error("repairing clip player names from hints", error))?
        } else {
            connection
                .execute(
                    "
                UPDATE clip_metadata
                SET player_name = ?2,
                    updated_at = CURRENT_TIMESTAMP
                WHERE player_name IS NOT ?2
                    AND (
                        ?3 = 1
                        OR player_name IS NULL
                        OR TRIM(player_name) = ''
                        OR player_name LIKE '%#%'
                        )
                    AND clip_id IN (
                            SELECT clips.id
                            FROM clips
                            JOIN source_dirs
                                ON source_dirs.id = clips.source_dir_id
                            WHERE source_dirs.name = ?1
                        )
                    ",
                    params![source_dir_name, account_name, i64::from(authoritative)],
                )
                .map_err(|error| readable_error("repairing clip player names from hints", error))?
        };
        changed += player_update_count;
    }

    Ok(changed)
}

fn scanned_clip_changed(
    existing: &Clip,
    input: &ClipInput<'_>,
    extension: &str,
    modified_at: &Option<String>,
    cover_path: &Option<String>,
) -> bool {
    existing.source_dir_id != input.source_dir_id
        || existing.clip_group_id != input.clip_group_id
        || existing.video_path != input.video_path
        || existing.file_name != input.file_name
        || existing.extension != extension
        || existing.file_size != input.file_size
        || &existing.modified_at != modified_at
        || existing.duration_ms != input.duration_ms
        || existing.recorded_at.as_deref() != input.recorded_at
        || &existing.cover_path != cover_path
        || existing.cover_source != input.cover_source
        || existing.status != "available"
}

fn ensure_row_changed(changed: usize, action: &str, clip_id: i64) -> DbResult<()> {
    if changed == 0 {
        Err(format!("{action} failed: clip id {clip_id} was not found"))
    } else {
        Ok(())
    }
}

fn require_non_empty<'a>(value: &'a str, label: &str) -> DbResult<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("{label} cannot be empty"))
    } else {
        Ok(trimmed)
    }
}

fn normalize_optional(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn looks_like_source_account_id(value: &str) -> bool {
    let trimmed = value.trim();
    (10..=24).contains(&trimmed.len())
        && trimmed.chars().all(|character| character.is_ascii_digit())
}

pub fn normalize_path(path: &str) -> String {
    path.trim().replace('\\', "/").to_lowercase()
}

fn source_root_match_patterns(source_root: Option<&Path>) -> Option<(String, String)> {
    let source_root = source_root?;
    let path_text = source_root.to_string_lossy();
    let normalized = normalize_path(path_text.as_ref());
    let root = normalized.trim_end_matches('/').to_string();
    if root.is_empty() {
        None
    } else {
        let escaped_root = escape_like_pattern(&root);
        Some((root, format!("{escaped_root}/%")))
    }
}

fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '!' => escaped.push_str("!!"),
            '%' => escaped.push_str("!%"),
            '_' => escaped.push_str("!_"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn extension_from_file_name(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.trim().to_lowercase())
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "mp4".to_string())
}

fn bool_to_integer(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn readable_error(action: &str, error: impl std::fmt::Display) -> String {
    format!("Database {action} failed: {error}")
}

#[cfg(test)]
mod tests {
    // Account IDs and player names below are synthetic fixtures, not captured user data.
    use std::{
        ffi::CStr,
        fs,
        os::raw::{c_char, c_void},
        path::PathBuf,
        ptr,
        sync::Mutex,
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn configured_connections_use_full_synchronous_durability() {
        let connection = Connection::open_in_memory().expect("in-memory database should open");
        configure_connection(&connection).expect("database connection should configure");

        let synchronous: i64 = connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .expect("synchronous mode should be readable");

        assert_eq!(synchronous, 2, "SQLite FULL synchronous mode is required");
    }

    #[test]
    fn migrate_database_initializes_a_new_file_and_is_idempotent() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let database_path = root.join("highlight-index.sqlite3");

        migrate_database(&database_path).expect("new database should migrate");
        migrate_database(&database_path).expect("database migration should be repeatable");

        let connection = open_database_read_only(&database_path)
            .expect("migrated database should open read-only");
        let tag_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM tags", [], |row| row.get(0))
            .expect("tag count should be readable");

        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        assert_eq!(tag_count, 0);

        drop(connection);
        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn migrate_database_upgrades_an_existing_file_without_losing_user_state() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let database_path = root.join("legacy.sqlite3");
        let legacy = Connection::open(&database_path).expect("legacy database should open");
        create_v4_database_with_user_state(&legacy);
        drop(legacy);

        migrate_database(&database_path).expect("legacy database should migrate");

        let connection = open_database_read_only(&database_path)
            .expect("migrated database should open read-only");
        let user_state: (i64, String) = connection
            .query_row(
                "SELECT is_favorite, note FROM clips WHERE id = 42",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("migrated user state should remain readable");
        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        assert_eq!(user_state, (1, "keep this note".to_string()));

        let backups = fs::read_dir(root.join("backups"))
            .expect("migration backup directory should exist")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        assert_eq!(
            backups.len(),
            1,
            "migration should retain one verified backup"
        );
        let backup =
            open_database_read_only(&backups[0]).expect("migration backup should remain readable");
        assert_eq!(schema_user_version(&backup), 4);
        let backup_note: String = backup
            .query_row("SELECT note FROM clips WHERE id = 42", [], |row| row.get(0))
            .expect("user state should be present in the pre-migration backup");
        assert_eq!(backup_note, "keep this note");

        drop(backup);
        drop(connection);
        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn migrate_database_upgrades_v11_with_delete_outbox_and_trash_snapshots() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let database_path = root.join("v11.sqlite3");
        let connection = Connection::open(&database_path).expect("v11 database should open");
        initialize_schema(&connection).expect("current schema fixture should initialize");
        create_tag(&connection, "保留标签", Some("teal")).expect("user tag should seed");
        connection
            .execute_batch(
                "
                DROP TRIGGER prevent_clip_trash_snapshot_update;
                DROP TRIGGER require_clip_trash_snapshot;
                DROP TABLE clip_trash_snapshots;
                DROP TABLE clip_delete_intents;
                PRAGMA user_version = 11;
                ",
            )
            .expect("fixture should emulate schema v11");
        drop(connection);

        migrate_database(&database_path).expect("v11 database should migrate");

        let connection = open_database_read_only(&database_path)
            .expect("migrated database should open read-only");
        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        assert_table_exists(&connection, "clip_delete_intents");
        assert_table_exists(&connection, "clip_trash_snapshots");
        let preserved_tags: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tags WHERE name = '保留标签'",
                [],
                |row| row.get(0),
            )
            .expect("preserved tag should remain readable");
        assert_eq!(preserved_tags, 1);
        let on_delete: String = connection
            .query_row(
                "SELECT on_delete FROM pragma_foreign_key_list('clip_delete_intents') WHERE \"table\" = 'clips'",
                [],
                |row| row.get(0),
            )
            .expect("delete intent foreign key should be inspectable");
        assert_eq!(on_delete, "RESTRICT");

        let backup_path = fs::read_dir(root.join("backups"))
            .expect("v11 backup directory should exist")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "sqlite3")
            })
            .expect("v11 migration should create a backup");
        let backup = open_database_read_only(&backup_path).expect("v11 backup should be readable");
        assert_eq!(schema_user_version(&backup), 11);
        let backup_has_outbox: i64 = backup
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'clip_delete_intents'",
                [],
                |row| row.get(0),
            )
            .expect("backup schema should be inspectable");
        assert_eq!(backup_has_outbox, 0);
        let backup_has_trash_snapshots: i64 = backup
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'clip_trash_snapshots'",
                [],
                |row| row.get(0),
            )
            .expect("backup schema should be inspectable");
        assert_eq!(backup_has_trash_snapshots, 0);

        drop(backup);
        drop(connection);
        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn migrate_database_v12_invalidates_intents_without_trash_time_authorization() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let clip_path = root.join("legacy-authorized-too-late.mp4");
        fs::write(&clip_path, b"must survive v12 migration")
            .expect("legacy fixture video should be created");
        let database_path = root.join("v12.sqlite3");
        let connection = Connection::open(&database_path).expect("v12 database should open");
        initialize_schema(&connection).expect("current schema fixture should initialize");
        let source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: root.to_string_lossy().as_ref(),
                name: "Legacy v12 source",
            },
        )
        .expect("legacy source should seed");
        let clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source.id,
                clip_group_id: None,
                video_path: clip_path.to_string_lossy().as_ref(),
                file_name: "legacy-authorized-too-late.mp4",
                file_size: 26,
                modified_at: None,
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("legacy clip should seed");
        set_clips_trashed(&connection, &[clip.id], true).expect("fixture should enter trash");
        connection
            .execute(
                "
                INSERT INTO clip_delete_intents (
                    clip_id,
                    video_path,
                    source_dir_path,
                    extension,
                    file_existed
                )
                VALUES (?1, ?2, ?3, 'mp4', 1)
                ",
                params![clip.id, clip_path.to_string_lossy(), root.to_string_lossy()],
            )
            .expect("legacy delete intent should seed");
        connection
            .execute_batch(
                "
                DROP TRIGGER prevent_clip_trash_snapshot_update;
                DROP TRIGGER require_clip_trash_snapshot;
                DROP TABLE clip_trash_snapshots;
                PRAGMA user_version = 12;
                ",
            )
            .expect("fixture should emulate schema v12");
        drop(connection);

        migrate_database(&database_path).expect("v12 database should migrate");

        let connection = open_database(&database_path).expect("migrated database should open");
        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        assert_table_exists(&connection, "clip_trash_snapshots");
        let state: (String, i64, i64) = connection
            .query_row(
                "
                SELECT
                    clips.file_status,
                    (SELECT COUNT(*) FROM clip_delete_intents WHERE clip_id = clips.id),
                    (SELECT COUNT(*) FROM clip_trash_snapshots WHERE clip_id = clips.id)
                FROM clips
                WHERE clips.id = ?1
                ",
                [clip.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("migrated legacy state should be readable");
        assert_eq!(state, ("trashed".to_string(), 0, 0));
        let outcome = delete_clip_permanently(&connection, clip.id)
            .expect("fail-closed delete decision should be readable");
        assert!(matches!(
            outcome,
            ClipDeleteItemOutcome::Rejected(ref issue) if issue.code == "trash-snapshot-missing"
        ));
        assert_eq!(
            fs::read(&clip_path).expect("legacy video must survive"),
            b"must survive v12 migration"
        );

        drop(connection);
        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn migrate_database_rejects_a_future_schema_without_modifying_it() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let database_path = root.join("future.sqlite3");
        let connection = Connection::open(&database_path).expect("future database should open");
        connection
            .execute_batch("CREATE TABLE future_data (value TEXT); PRAGMA user_version = 999;")
            .expect("future database should seed");
        drop(connection);

        let error = migrate_database(&database_path)
            .expect_err("a newer database must not be opened by an older build");
        assert!(error.contains("newer than this build supports"));

        let connection = Connection::open(&database_path).expect("future database should reopen");
        assert_eq!(schema_user_version(&connection), 999);
        assert_table_exists(&connection, "future_data");
        assert!(!root.join("backups").exists());

        drop(connection);
        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn migrate_database_refuses_a_foreign_key_damaged_database_before_backup() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let database_path = root.join("damaged.sqlite3");
        let connection = Connection::open(&database_path).expect("damaged database should open");
        connection
            .execute_batch(
                "
                PRAGMA foreign_keys = OFF;
                CREATE TABLE parents (id INTEGER PRIMARY KEY);
                CREATE TABLE children (
                    id INTEGER PRIMARY KEY,
                    parent_id INTEGER REFERENCES parents(id)
                );
                INSERT INTO children (id, parent_id) VALUES (1, 404);
                PRAGMA user_version = 1;
                ",
            )
            .expect("damaged database should seed");
        drop(connection);

        let error = migrate_database(&database_path)
            .expect_err("foreign-key damage must stop startup before migration");
        assert!(error.contains("foreign-key check failed before migration"));
        assert!(!root.join("backups").exists());

        let connection = Connection::open(&database_path).expect("damaged database should reopen");
        assert_eq!(schema_user_version(&connection), 1);
        let orphan_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM children", [], |row| row.get(0))
            .expect("orphan fixture should remain unchanged");
        assert_eq!(orphan_count, 1);

        drop(connection);
        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn migrate_database_refuses_a_non_sqlite_file_without_overwriting_it() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let database_path = root.join("not-sqlite.sqlite3");
        let original = b"this is not a SQLite database";
        fs::write(&database_path, original).expect("invalid database fixture should be written");

        migrate_database(&database_path).expect_err("invalid SQLite data must stop startup");
        assert_eq!(
            fs::read(&database_path).expect("invalid fixture should remain readable"),
            original
        );
        assert!(!root.join("backups").exists());

        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn initialize_schema_repairs_current_database_missing_tag_updated_at() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        connection
            .execute_batch(
                "
                CREATE TABLE tags (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    color TEXT,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                INSERT INTO tags (id, name, color)
                VALUES (197303, '待剪辑', 'teal');
                PRAGMA user_version = 10;
                ",
            )
            .expect("legacy current-version tag table should seed");

        initialize_schema(&connection).expect("schema repair should succeed");

        let columns = table_columns(&connection, "tags");
        assert!(columns.iter().any(|column| column == "updated_at"));
        let updated = update_tag(&connection, 197303, "待剪辑", Some("green"))
            .expect("color-only tag update should succeed after repair");
        assert_eq!(updated.name, "待剪辑");
        assert_eq!(updated.color.as_deref(), Some("green"));
        let updated_at: Option<String> = connection
            .query_row("SELECT updated_at FROM tags WHERE id = 197303", [], |row| {
                row.get(0)
            })
            .expect("updated timestamp should be readable");
        assert!(updated_at.is_some());
    }

    #[test]
    fn initialize_schema_rolls_back_all_schema_changes_when_a_step_fails() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        connection
            .execute_batch("CREATE VIEW tags AS SELECT 'blocked' AS name; PRAGMA user_version = 0;")
            .expect("conflicting legacy fixture should seed");

        let error = initialize_schema(&connection)
            .expect_err("a conflicting schema object should make migration fail");
        assert!(error.contains("initializing schema"));
        assert_eq!(schema_user_version(&connection), 0);
        let source_dir_tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'source_dirs'",
                [],
                |row| row.get(0),
            )
            .expect("sqlite schema should remain readable");
        assert_eq!(
            source_dir_tables, 0,
            "tables created before the failing statement must be rolled back"
        );
    }

    #[test]
    fn ordinary_connections_do_not_run_schema_initialization_side_effects() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let database_path = root.join("highlight-index.sqlite3");
        migrate_database(&database_path).expect("database should migrate once");

        let connection = open_database(&database_path).expect("database should open");
        connection
            .execute("DELETE FROM tags WHERE name = 'ACE'", [])
            .expect("fixture tag should be removable");
        connection
            .execute_batch("PRAGMA user_version = 73;")
            .expect("fixture schema marker should update");
        drop(connection);

        for _ in 0..3 {
            let connection = open_database(&database_path).expect("database should open lightly");
            let foreign_keys: i64 = connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .expect("connection pragma should be readable");
            assert_eq!(foreign_keys, 1);
        }

        let connection =
            open_database_read_only(&database_path).expect("database should remain readable");
        let ace_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM tags WHERE name = 'ACE'", [], |row| {
                row.get(0)
            })
            .expect("tag count should be readable");
        assert_eq!(ace_count, 0, "ordinary opens must not reseed tags");
        assert_eq!(
            schema_user_version(&connection),
            73,
            "ordinary opens must not rewrite the schema version"
        );

        drop(connection);
        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn ordinary_connection_does_not_create_a_missing_database() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("database fixture root should be created");
        let database_path = root.join("missing.sqlite3");

        let error = open_database(&database_path)
            .expect_err("ordinary connection should reject a missing database");

        assert!(error.contains("opening SQLite file"));
        assert!(!database_path.exists());

        fs::remove_dir_all(root).expect("database fixture should be removed");
    }

    #[test]
    fn initialize_schema_creates_required_tables() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");

        initialize_schema(&connection).expect("schema should initialize");

        let mut statement = connection
            .prepare(
                "
                SELECT name
                FROM sqlite_master
                WHERE type = 'table'
                  AND name IN (
                    'source_dirs',
                    'clip_groups',
                    'clip_delete_intents',
                    'clip_trash_snapshots',
                    'clips',
                    'clip_thumbnails',
                    'clip_metadata',
                    'matches',
                    'match_stats',
                    'match_snapshots',
                    'match_events',
                    'tags',
                    'clip_tags',
                    'scan_runs'
                  )
                ORDER BY name
                ",
            )
            .expect("table query should prepare");

        let tables = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("table query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("table names should load");

        assert_eq!(
            tables,
            vec![
                "clip_delete_intents",
                "clip_groups",
                "clip_metadata",
                "clip_tags",
                "clip_thumbnails",
                "clip_trash_snapshots",
                "clips",
                "match_events",
                "match_snapshots",
                "match_stats",
                "matches",
                "scan_runs",
                "source_dirs",
                "tags"
            ]
        );

        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
    }

    #[test]
    fn initialize_schema_migrates_v3_database_for_match_metadata() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        create_v3_clip_metadata_schema(&connection);

        initialize_schema(&connection).expect("schema should migrate");

        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        assert_table_exists(&connection, "matches");
        assert_table_exists(&connection, "match_stats");
        assert_table_exists(&connection, "match_snapshots");
        assert_table_exists(&connection, "match_events");

        let columns = table_columns(&connection, "clip_metadata");
        for column in [
            "match_id",
            "scoreline",
            "kill_count",
            "weapon_name",
            "round_label",
            "raw_title",
            "extra_json",
        ] {
            assert!(
                columns.iter().any(|existing| existing == column),
                "clip_metadata should include {column}"
            );
        }
    }

    #[test]
    fn migrates_v4_to_current_without_losing_user_state() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        create_v4_database_with_user_state(&connection);

        initialize_schema(&connection).expect("schema should migrate");

        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        let user_state: (i64, i64, String, String) = connection
            .query_row(
                "
                SELECT clips.id, clips.is_favorite, clips.note, tags.name
                FROM clips
                JOIN clip_tags ON clip_tags.clip_id = clips.id
                JOIN tags ON tags.id = clip_tags.tag_id
                WHERE clips.id = 42
                ",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("user state should remain readable");
        assert_eq!(
            user_state,
            (42, 1, "keep this note".to_string(), "复盘".to_string())
        );
        let source_dir: (i64, String, String) = connection
            .query_row(
                "SELECT id, path, name FROM source_dirs WHERE id = 7",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("source directory should remain readable");
        assert_eq!(
            source_dir,
            (7, "D:/Clips/Valorant".to_string(), "Valorant".to_string())
        );
        let clip_source_dir_id: i64 = connection
            .query_row("SELECT source_dir_id FROM clips WHERE id = 42", [], |row| {
                row.get(0)
            })
            .expect("clip source directory relationship should remain readable");
        assert_eq!(clip_source_dir_id, 7);

        let migrated_metadata_source: Option<String> = connection
            .query_row(
                "SELECT metadata_source FROM clip_metadata WHERE clip_id = 42",
                [],
                |row| row.get(0),
            )
            .expect("migrated metadata source should remain readable");
        assert_eq!(
            migrated_metadata_source.as_deref(),
            Some("video_export"),
            "legacy rows with a JSON path must retain export priority"
        );

        let columns = table_columns(&connection, "clip_metadata");
        for column in [
            "official_video_id",
            "official_video_name",
            "official_video_type",
            "highlight_type",
            "round_score",
            "round_score_source",
            "metadata_source",
        ] {
            assert!(
                columns.iter().any(|existing| existing == column),
                "clip_metadata should include {column}"
            );
        }
        assert_table_exists(&connection, "clip_segments");
        assert_table_exists(&connection, "clip_events");
    }

    #[test]
    fn clip_events_reject_cross_clip_segment_on_insert() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip_a = insert_test_clip_with_file_name(&connection, "clip-a.mp4");
        let clip_b = insert_test_clip_with_file_name(&connection, "clip-b.mp4");
        connection
            .execute(
                "
                INSERT INTO clip_segments (
                    clip_id, segment_key, start_ms, duration_ms
                )
                VALUES (?1, 'clip-b-segment', 1000, 5000)
                ",
                params![clip_b.id],
            )
            .expect("clip B segment should insert");
        let clip_b_segment_id = connection.last_insert_rowid();

        let result = connection.execute(
            "
            INSERT INTO clip_events (
                clip_id, segment_id, event_key, event_type
            )
            VALUES (?1, ?2, 'cross-clip-event', 'kill')
            ",
            params![clip_a.id, clip_b_segment_id],
        );

        assert!(result.is_err(), "cross-clip segment reference should fail");
    }

    #[test]
    fn clip_events_reject_cross_clip_segment_on_update() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip_a = insert_test_clip_with_file_name(&connection, "clip-a.mp4");
        let clip_b = insert_test_clip_with_file_name(&connection, "clip-b.mp4");
        connection
            .execute(
                "
                INSERT INTO clip_segments (
                    clip_id, segment_key, start_ms, duration_ms
                )
                VALUES (?1, 'clip-a-segment', 1000, 5000)
                ",
                params![clip_a.id],
            )
            .expect("clip A segment should insert");
        let clip_a_segment_id = connection.last_insert_rowid();
        connection
            .execute(
                "
                INSERT INTO clip_segments (
                    clip_id, segment_key, start_ms, duration_ms
                )
                VALUES (?1, 'clip-b-segment', 1000, 5000)
                ",
                params![clip_b.id],
            )
            .expect("clip B segment should insert");
        let clip_b_segment_id = connection.last_insert_rowid();
        connection
            .execute(
                "
                INSERT INTO clip_events (
                    clip_id, segment_id, event_key, event_type
                )
                VALUES (?1, ?2, 'clip-a-event', 'kill')
                ",
                params![clip_a.id, clip_a_segment_id],
            )
            .expect("clip A event should insert");
        let clip_a_event_id = connection.last_insert_rowid();

        let result = connection.execute(
            "UPDATE clip_events SET segment_id = ?1 WHERE id = ?2",
            params![clip_b_segment_id, clip_a_event_id],
        );

        assert!(result.is_err(), "cross-clip segment reference should fail");
    }

    #[test]
    fn clip_segments_reject_moving_a_referenced_segment_to_another_clip() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip_a = insert_test_clip_with_file_name(&connection, "clip-a.mp4");
        let clip_b = insert_test_clip_with_file_name(&connection, "clip-b.mp4");
        connection
            .execute(
                "
                INSERT INTO clip_segments (
                    clip_id, segment_key, start_ms, duration_ms
                )
                VALUES (?1, 'clip-a-segment', 1000, 5000)
                ",
                params![clip_a.id],
            )
            .expect("clip A segment should insert");
        let segment_id = connection.last_insert_rowid();
        connection
            .execute(
                "
                INSERT INTO clip_events (
                    clip_id, segment_id, event_key, event_type
                )
                VALUES (?1, ?2, 'clip-a-event', 'kill')
                ",
                params![clip_a.id, segment_id],
            )
            .expect("clip A event should insert");

        let result = connection.execute(
            "UPDATE clip_segments SET clip_id = ?1 WHERE id = ?2",
            params![clip_b.id, segment_id],
        );

        assert!(result.is_err(), "referenced segment move should fail");
        let stored_relation: (i64, i64, i64) = connection
            .query_row(
                "
                SELECT clip_segments.clip_id, clip_events.clip_id, clip_events.segment_id
                FROM clip_segments
                JOIN clip_events ON clip_events.segment_id = clip_segments.id
                WHERE clip_segments.id = ?1
                ",
                params![segment_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("original segment relationship should remain readable");
        assert_eq!(stored_relation, (clip_a.id, clip_a.id, segment_id));
    }

    #[test]
    fn replacing_clip_timeline_does_not_alter_another_clip() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip_a = insert_test_clip_with_file_name(&connection, "clip-a.mp4");
        let clip_b = insert_test_clip_with_file_name(&connection, "clip-b.mp4");

        replace_clip_timeline(
            &connection,
            clip_b.id,
            &[ClipSegmentInput {
                segment_key: "b-round-1",
                round_id: Some(1),
                start_ms: 1_000,
                duration_ms: 5_000,
                game_start_ms: Some(100),
                game_end_ms: Some(5_100),
            }],
            &[ClipEventInput {
                segment_key: Some("b-round-1"),
                event_key: "b-kill-1",
                event_type: "kill",
                video_time_ms: Some(2_000),
                event_time: Some("2026-06-28T08:01:00Z"),
                round_id: Some(1),
                player_name: Some("Player#1001"),
                agent_name: Some("Jett"),
                weapon_name: Some("Vandal"),
                killer_name: Some("Player#1001"),
                killed_name: Some("Opponent#2002"),
                killer_is_me: true,
                raw_json: Some("{\"source\":\"clip-b\"}"),
            }],
        )
        .expect("clip B timeline should seed");
        replace_clip_timeline(
            &connection,
            clip_a.id,
            &[ClipSegmentInput {
                segment_key: "a-old",
                round_id: Some(2),
                start_ms: 2_000,
                duration_ms: 4_000,
                game_start_ms: None,
                game_end_ms: None,
            }],
            &[ClipEventInput {
                segment_key: Some("a-old"),
                event_key: "a-old-kill",
                event_type: "kill",
                video_time_ms: Some(3_000),
                event_time: None,
                round_id: Some(2),
                player_name: None,
                agent_name: None,
                weapon_name: None,
                killer_name: None,
                killed_name: None,
                killer_is_me: false,
                raw_json: None,
            }],
        )
        .expect("clip A old timeline should seed");

        replace_clip_timeline(
            &connection,
            clip_a.id,
            &[ClipSegmentInput {
                segment_key: "a-new",
                round_id: Some(3),
                start_ms: 7_000,
                duration_ms: 2_000,
                game_start_ms: Some(6_500),
                game_end_ms: Some(8_500),
            }],
            &[ClipEventInput {
                segment_key: Some("a-new"),
                event_key: "a-new-kill",
                event_type: "kill",
                video_time_ms: Some(7_500),
                event_time: None,
                round_id: Some(3),
                player_name: Some("Player#1001"),
                agent_name: Some("Jett"),
                weapon_name: Some("Phantom"),
                killer_name: Some("Player#1001"),
                killed_name: Some("Opponent#3003"),
                killer_is_me: true,
                raw_json: None,
            }],
        )
        .expect("clip A timeline should replace");

        let clip_a_events =
            list_clip_events_for_clip(&connection, clip_a.id).expect("clip A events should list");
        let clip_b_events =
            list_clip_events_for_clip(&connection, clip_b.id).expect("clip B events should list");
        assert_eq!(clip_a_events.len(), 1);
        assert_eq!(clip_a_events[0].event_key, "a-new-kill");
        assert_eq!(clip_a_events[0].segment_key.as_deref(), Some("a-new"));
        assert_eq!(clip_b_events.len(), 1);
        assert_eq!(clip_b_events[0].event_key, "b-kill-1");
        assert_eq!(clip_b_events[0].segment_key.as_deref(), Some("b-round-1"));
        assert!(clip_b_events[0].killer_is_me);
        assert_eq!(
            clip_b_events[0].raw_json.as_deref(),
            Some("{\"source\":\"clip-b\"}")
        );
        let clip_a_old_segment_count: i64 = connection
            .query_row(
                "
                SELECT COUNT(*)
                FROM clip_segments
                WHERE clip_id = ?1 AND segment_key = 'a-old'
                ",
                params![clip_a.id],
                |row| row.get(0),
            )
            .expect("clip A old segment count should be readable");
        let clip_b_segment_count: i64 = connection
            .query_row(
                "
                SELECT COUNT(*)
                FROM clip_segments
                WHERE clip_id = ?1 AND segment_key = 'b-round-1'
                ",
                params![clip_b.id],
                |row| row.get(0),
            )
            .expect("clip B segment count should be readable");
        assert_eq!(clip_a_old_segment_count, 0);
        assert_eq!(clip_b_segment_count, 1);
    }

    #[test]
    fn replacing_clip_timeline_rolls_back_when_an_insert_fails() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip = insert_test_clip(&connection);
        replace_clip_timeline(
            &connection,
            clip.id,
            &[ClipSegmentInput {
                segment_key: "existing",
                round_id: None,
                start_ms: 1_000,
                duration_ms: 3_000,
                game_start_ms: None,
                game_end_ms: None,
            }],
            &[ClipEventInput {
                segment_key: Some("existing"),
                event_key: "existing-event",
                event_type: "kill",
                video_time_ms: Some(1_500),
                event_time: None,
                round_id: None,
                player_name: None,
                agent_name: None,
                weapon_name: None,
                killer_name: None,
                killed_name: None,
                killer_is_me: false,
                raw_json: None,
            }],
        )
        .expect("initial timeline should seed");

        let result = replace_clip_timeline(
            &connection,
            clip.id,
            &[
                ClipSegmentInput {
                    segment_key: "duplicate",
                    round_id: None,
                    start_ms: 2_000,
                    duration_ms: 3_000,
                    game_start_ms: None,
                    game_end_ms: None,
                },
                ClipSegmentInput {
                    segment_key: "duplicate",
                    round_id: None,
                    start_ms: 3_000,
                    duration_ms: 3_000,
                    game_start_ms: None,
                    game_end_ms: None,
                },
            ],
            &[],
        );

        assert!(result.is_err());
        let events =
            list_clip_events_for_clip(&connection, clip.id).expect("old events should remain");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_key, "existing-event");
    }

    #[test]
    fn replacing_clip_timeline_rolls_back_when_event_segment_key_is_unknown() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip = insert_test_clip(&connection);
        replace_clip_timeline(
            &connection,
            clip.id,
            &[ClipSegmentInput {
                segment_key: "existing",
                round_id: None,
                start_ms: 1_000,
                duration_ms: 3_000,
                game_start_ms: None,
                game_end_ms: None,
            }],
            &[ClipEventInput {
                segment_key: Some("existing"),
                event_key: "existing-event",
                event_type: "kill",
                video_time_ms: Some(1_500),
                event_time: None,
                round_id: None,
                player_name: None,
                agent_name: None,
                weapon_name: None,
                killer_name: None,
                killed_name: None,
                killer_is_me: false,
                raw_json: None,
            }],
        )
        .expect("initial timeline should seed");

        let result = replace_clip_timeline(
            &connection,
            clip.id,
            &[ClipSegmentInput {
                segment_key: "replacement",
                round_id: None,
                start_ms: 2_000,
                duration_ms: 3_000,
                game_start_ms: None,
                game_end_ms: None,
            }],
            &[ClipEventInput {
                segment_key: Some("missing"),
                event_key: "replacement-event",
                event_type: "kill",
                video_time_ms: Some(2_500),
                event_time: None,
                round_id: None,
                player_name: None,
                agent_name: None,
                weapon_name: None,
                killer_name: None,
                killed_name: None,
                killer_is_me: false,
                raw_json: None,
            }],
        );

        assert!(result.is_err());
        let events =
            list_clip_events_for_clip(&connection, clip.id).expect("old events should remain");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_key, "existing-event");
        assert_eq!(events[0].segment_key.as_deref(), Some("existing"));
        let replacement_segment_count: i64 = connection
            .query_row(
                "
                SELECT COUNT(*)
                FROM clip_segments
                WHERE clip_id = ?1 AND segment_key = 'replacement'
                ",
                params![clip.id],
                |row| row.get(0),
            )
            .expect("replacement segment count should be readable");
        assert_eq!(replacement_segment_count, 0);
    }

    #[test]
    fn initialize_schema_can_run_match_metadata_migration_repeatedly() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");

        initialize_schema(&connection).expect("schema should initialize");
        initialize_schema(&connection).expect("schema should initialize again");

        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        assert_table_exists(&connection, "matches");
        assert_table_exists(&connection, "match_stats");
        assert_table_exists(&connection, "match_snapshots");
        assert_table_exists(&connection, "match_events");
    }

    #[test]
    fn clear_invalid_display_metadata_removes_asset_accounts_and_numeric_match_modes() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let inserted = insert_test_clip(&connection);
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: inserted.id,
                metadata_status: "parsed",
                json_path: None,
                account_name: Some("Cards/D3018FBE-45CD-786A-DD6C-BCAF429F7096.png"),
                player_name: Some("https://game.gtimg.cn/images/val/agamezlk/PlayerCards/card.png"),
                agent_name: Some("Jett"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: None,
                kda: Some("27/22/6"),
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("metadata should seed");
        let official_clip = insert_test_clip_with_file_name(&connection, "official.mp4");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: official_clip.id,
                metadata_status: "enriched",
                json_path: None,
                account_name: Some("Cards/official-player-card.png"),
                player_name: Some("https://assets.example/official-player.png"),
                agent_name: Some("Jett"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: None,
                kda: None,
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("official metadata should seed");
        connection
            .execute(
                "UPDATE clip_metadata SET metadata_source = 'wonderful_db' WHERE clip_id = ?1",
                params![official_clip.id],
            )
            .expect("official source should seed");
        connection
            .execute(
                "
                INSERT INTO matches (game_id, player_name, map_name, game_mode)
                VALUES ('match-invalid-mode', 'FixtureBravo#0002', '隐世修所', '1')
                ",
                [],
            )
            .expect("match should seed");

        let changed = clear_invalid_display_metadata(&connection).expect("cleanup should complete");

        assert_eq!(changed, 2);
        let clip_metadata: (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = connection
            .query_row(
                "
                    SELECT account_name, player_name, map_name, game_mode
                    FROM clip_metadata
                    WHERE clip_id = ?1
                    ",
                params![inserted.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("clip metadata should load");
        assert_eq!(clip_metadata.0, None);
        assert_eq!(clip_metadata.1, None);
        assert_eq!(clip_metadata.2.as_deref(), Some("隐世修所"));
        assert_eq!(clip_metadata.3.as_deref(), Some("竞技模式"));

        let official_names: (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT account_name, player_name FROM clip_metadata WHERE clip_id = ?1",
                params![official_clip.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("official metadata should load");
        assert_eq!(
            official_names.0.as_deref(),
            Some("Cards/official-player-card.png")
        );
        assert_eq!(
            official_names.1.as_deref(),
            Some("https://assets.example/official-player.png")
        );

        let mode: Option<String> = connection
            .query_row(
                "SELECT game_mode FROM matches WHERE game_id = 'match-invalid-mode'",
                [],
                |row| row.get(0),
            )
            .expect("match should load");
        assert_eq!(mode, None);
    }

    #[test]
    fn backfill_agent_names_from_export_text_uses_known_aclos_asset_ids() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let inserted = insert_test_clip(&connection);
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: inserted.id,
                metadata_status: "parsed",
                json_path: None,
                account_name: Some("FixtureBravo#0002"),
                player_name: Some("FixtureBravo#0002"),
                agent_name: None,
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: None,
                kda: Some("27/22/6"),
                extracted_text: Some(
                    "url: https://game.gtimg.cn/images/val/agamezlk/agentbackground/agent/11.png",
                ),
                parse_error: None,
            },
        )
        .expect("metadata should seed");

        let changed = backfill_agent_names_from_export_text(&connection)
            .expect("agent names should backfill");

        assert_eq!(changed, 1);
        let agent_name: Option<String> = connection
            .query_row(
                "SELECT agent_name FROM clip_metadata WHERE clip_id = ?1",
                params![inserted.id],
                |row| row.get(0),
            )
            .expect("metadata should load");
        assert_eq!(agent_name.as_deref(), Some("Reyna"));
    }

    #[test]
    fn propagate_known_account_names_uses_source_account_id_without_overwriting_player_name() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source_dir = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\ACLOS\\wonderfulVideos1001",
                name: "wonderfulVideos1001",
            },
        )
        .expect("source dir should upsert");
        let clip_group = upsert_clip_group(
            &connection,
            ClipGroupInput {
                source_dir_id: source_dir.id,
                group_key: "match-a",
                display_name: "match-a",
            },
        )
        .expect("clip group should upsert");
        let clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\ACLOS\\wonderfulVideos1001\\match-a\\ace.mp4",
                file_name: "ace.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: clip.id,
                metadata_status: "enriched",
                json_path: None,
                account_name: None,
                player_name: Some("FixtureBravo"),
                agent_name: Some("Jett"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: None,
                kda: None,
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("metadata should seed");
        let official_existing = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\ACLOS\\wonderfulVideos1001\\match-a\\official-existing.mp4",
                file_name: "official-existing.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("official existing clip should upsert");
        let official_missing = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\ACLOS\\wonderfulVideos1001\\match-a\\official-missing.mp4",
                file_name: "official-missing.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("official missing clip should upsert");
        for (clip_id, account_name) in [
            (official_existing.id, Some("Official#1001")),
            (official_missing.id, None),
        ] {
            upsert_clip_metadata(
                &connection,
                ClipMetadataInput {
                    clip_id,
                    metadata_status: "enriched",
                    json_path: None,
                    account_name,
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
            .expect("official metadata should seed");
            connection
                .execute(
                    "UPDATE clip_metadata SET metadata_source = 'wonderful_db' WHERE clip_id = ?1",
                    params![clip_id],
                )
                .expect("official source should seed");
        }
        connection
            .execute(
                "
                INSERT INTO matches (game_id, account_id, player_name, map_name, game_mode)
                VALUES ('match-known-account', '1001', 'FixtureBravo#0002', '隐世修所', '竞技模式')
                ",
                [],
            )
            .expect("match should seed");

        let changed = propagate_known_account_names(&connection, None)
            .expect("known account names should propagate");

        assert_eq!(changed, 2);
        let metadata: (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT account_name, player_name FROM clip_metadata WHERE clip_id = ?1",
                params![clip.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("metadata should load");
        assert_eq!(metadata.0.as_deref(), Some("FixtureBravo#0002"));
        assert_eq!(metadata.1.as_deref(), Some("FixtureBravo"));
        let official_names = [official_existing.id, official_missing.id]
            .into_iter()
            .map(|clip_id| {
                connection
                    .query_row(
                        "SELECT account_name FROM clip_metadata WHERE clip_id = ?1",
                        params![clip_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .expect("official account name should load")
            })
            .collect::<Vec<_>>();
        assert_eq!(official_names[0].as_deref(), Some("Official#1001"));
        assert_eq!(official_names[1].as_deref(), Some("FixtureBravo#0002"));
    }

    #[test]
    fn propagate_known_account_names_corrects_wrong_tagged_account_label() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source_dir = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\ACLOS\\wonderfulVideos90000000000000001",
                name: "wonderfulVideos90000000000000001",
            },
        )
        .expect("source dir should upsert");
        let clip_group = upsert_clip_group(
            &connection,
            ClipGroupInput {
                source_dir_id: source_dir.id,
                group_key: "match-a",
                display_name: "match-a",
            },
        )
        .expect("clip group should upsert");
        let clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\ACLOS\\wonderfulVideos90000000000000001\\match-a\\ace.mp4",
                file_name: "ace.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: clip.id,
                metadata_status: "enriched",
                json_path: None,
                account_name: Some("FixtureBravo#0002"),
                player_name: Some("FixtureBravo#0002"),
                agent_name: Some("Sova"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: None,
                kda: Some("14/16/5"),
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("metadata should seed");
        connection
            .execute(
                "
                INSERT INTO matches (game_id, account_id, player_name, map_name, game_mode)
                VALUES ('match-known-account', '90000000000000001', 'FixtureAlpha#0001', '隐世修所', '竞技模式')
                ",
                [],
            )
            .expect("match should seed");

        let changed = propagate_known_account_names(&connection, None)
            .expect("known account names should propagate");

        assert_eq!(changed, 1);
        let metadata: (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT account_name, player_name FROM clip_metadata WHERE clip_id = ?1",
                params![clip.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("metadata should load");
        assert_eq!(metadata.0.as_deref(), Some("FixtureAlpha#0001"));
        assert_eq!(metadata.1.as_deref(), Some("FixtureBravo#0002"));
    }

    #[test]
    fn propagate_known_account_names_ignores_conflicting_match_player_names() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source_dir = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\ACLOS\\wonderfulVideos9000000000000000002",
                name: "wonderfulVideos9000000000000000002",
            },
        )
        .expect("source dir should upsert");
        let clip_group = upsert_clip_group(
            &connection,
            ClipGroupInput {
                source_dir_id: source_dir.id,
                group_key: "match-a",
                display_name: "match-a",
            },
        )
        .expect("clip group should upsert");
        let clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\ACLOS\\wonderfulVideos9000000000000000002\\match-a\\ace.mp4",
                file_name: "ace.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: clip.id,
                metadata_status: "parsed",
                json_path: None,
                account_name: None,
                player_name: None,
                agent_name: Some("Jett"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: None,
                kda: None,
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("metadata should seed");
        connection
            .execute(
                "
                INSERT INTO matches (game_id, account_id, player_name)
                VALUES
                    ('match-fixture-bravo', '9000000000000000002', 'FixtureBravo#0002'),
                    ('match-fixture-alpha', '9000000000000000002', 'FixtureAlpha#0001')
                ",
                [],
            )
            .expect("matches should seed");

        let changed = propagate_known_account_names(&connection, None)
            .expect("known account names should propagate");

        assert_eq!(changed, 0);
        let account_name: Option<String> = connection
            .query_row(
                "SELECT account_name FROM clip_metadata WHERE clip_id = ?1",
                params![clip.id],
                |row| row.get(0),
            )
            .expect("metadata should load");
        assert_eq!(account_name, None);
    }

    #[test]
    fn propagate_account_name_hints_corrects_wrong_tagged_account_label() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source_dir = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\ACLOS\\wonderfulVideos9000000000000000002",
                name: "wonderfulVideos9000000000000000002",
            },
        )
        .expect("source dir should upsert");
        let clip_group = upsert_clip_group(
            &connection,
            ClipGroupInput {
                source_dir_id: source_dir.id,
                group_key: "match-a",
                display_name: "match-a",
            },
        )
        .expect("clip group should upsert");
        let clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\ACLOS\\wonderfulVideos9000000000000000002\\match-a\\ace.mp4",
                file_name: "ace.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: clip.id,
                metadata_status: "enriched",
                json_path: None,
                account_name: Some("Other#0000"),
                player_name: Some("Other#0000"),
                agent_name: Some("Jett"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: None,
                kda: None,
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("metadata should seed");

        connection
            .execute(
                "
                INSERT INTO matches (game_id, account_id, player_name)
                VALUES ('match-a', '9000000000000000002', 'Other#0000')
                ",
                [],
            )
            .expect("match should seed");

        let changed = propagate_account_name_hints(
            &connection,
            &[AccountNameHint {
                account_id: "9000000000000000002".to_string(),
                account_name: "FixtureBravo#0002".to_string(),
            }],
            None,
        )
        .expect("account name hints should propagate");

        assert_eq!(changed, 3);
        let metadata: (Option<String>, Option<String>, Option<String>) = connection
            .query_row(
                "
                SELECT clip_metadata.account_name, clip_metadata.player_name, matches.player_name
                FROM clip_metadata
                JOIN matches ON matches.game_id = 'match-a'
                WHERE clip_metadata.clip_id = ?1
                ",
                params![clip.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("metadata should load");
        assert_eq!(metadata.0.as_deref(), Some("FixtureBravo#0002"));
        assert_eq!(metadata.1.as_deref(), Some("FixtureBravo#0002"));
        assert_eq!(metadata.2.as_deref(), Some("FixtureBravo#0002"));
    }

    #[test]
    fn clear_mismatched_match_metadata_removes_stale_cross_account_enrichment() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source_dir = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\ACLOS\\wonderfulVideos1001",
                name: "wonderfulVideos1001",
            },
        )
        .expect("source dir should upsert");
        let clip_group = upsert_clip_group(
            &connection,
            ClipGroupInput {
                source_dir_id: source_dir.id,
                group_key: "shared-match",
                display_name: "shared-match",
            },
        )
        .expect("clip group should upsert");
        let clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\ACLOS\\wonderfulVideos1001\\shared-match\\ace.mp4",
                file_name: "ace.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: clip.id,
                metadata_status: "enriched",
                json_path: None,
                account_name: Some("Other#0000"),
                player_name: Some("Other#0000"),
                agent_name: Some("Jett"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: Some("13/7"),
                kda: Some("21/14/1"),
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("metadata should seed");
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET match_id = 'shared-match',
                    round_label = 'R03',
                    weapon_name = 'Vandal',
                    kill_count = 3
                WHERE clip_id = ?1
                ",
                params![clip.id],
            )
            .expect("match metadata should seed");
        let official_clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\ACLOS\\wonderfulVideos1001\\shared-match\\official.mp4",
                file_name: "official.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("official clip should upsert");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: official_clip.id,
                metadata_status: "enriched",
                json_path: None,
                account_name: Some("Official#1001"),
                player_name: Some("Official#1001"),
                agent_name: Some("Jett"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: Some("13/7"),
                kda: Some("21/14/1"),
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("official metadata should seed");
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET match_id = 'shared-match',
                    kill_count = 6,
                    official_video_name = '六杀时刻',
                    metadata_source = 'wonderful_db'
                WHERE clip_id = ?1
                ",
                params![official_clip.id],
            )
            .expect("official match metadata should seed");
        connection
            .execute(
                "
                INSERT INTO matches (game_id, account_id, player_name)
                VALUES ('shared-match', '2002', 'Other#0000')
                ",
                [],
            )
            .expect("mismatched match should seed");

        let changed =
            clear_mismatched_match_metadata(&connection).expect("mismatched metadata should clear");

        assert_eq!(changed, 1);
        type ClearedMetadata = (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        );
        let metadata: ClearedMetadata = connection
            .query_row(
                "
                SELECT
                    metadata_status,
                    account_name,
                    player_name,
                    agent_name,
                    map_name,
                    game_mode,
                    match_id,
                    kda,
                    weapon_name,
                    kill_count
                FROM clip_metadata
                WHERE clip_id = ?1
                ",
                params![clip.id],
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
                        row.get(9)?,
                    ))
                },
            )
            .expect("metadata should reload");
        assert_eq!(metadata.0, "not_found");
        assert_eq!(metadata.1, None);
        assert_eq!(metadata.2, None);
        assert_eq!(metadata.3, None);
        assert_eq!(metadata.4, None);
        assert_eq!(metadata.5, None);
        assert_eq!(metadata.6, None);
        assert_eq!(metadata.7, None);
        assert_eq!(metadata.8, None);
        assert_eq!(metadata.9, None);

        let official_metadata: (String, Option<String>, Option<String>, Option<i64>) = connection
            .query_row(
                "
                SELECT metadata_status, match_id, official_video_name, kill_count
                FROM clip_metadata
                WHERE clip_id = ?1
                ",
                params![official_clip.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("official metadata should reload");
        assert_eq!(official_metadata.0, "enriched");
        assert_eq!(official_metadata.1.as_deref(), Some("shared-match"));
        assert_eq!(official_metadata.2.as_deref(), Some("六杀时刻"));
        assert_eq!(official_metadata.3, Some(6));
    }

    #[test]
    fn clear_invalid_display_metadata_removes_untagged_enriched_account_labels() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source_dir = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\ACLOS\\wonderfulVideos9000000000000000002",
                name: "wonderfulVideos9000000000000000002",
            },
        )
        .expect("source dir should upsert");
        let clip_group = upsert_clip_group(
            &connection,
            ClipGroupInput {
                source_dir_id: source_dir.id,
                group_key: "match-opponent-name",
                display_name: "match-opponent-name",
            },
        )
        .expect("clip group should upsert");
        let clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path:
                    "D:\\ACLOS\\wonderfulVideos9000000000000000002\\match-opponent-name\\ace.mp4",
                file_name: "ace.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: clip.id,
                metadata_status: "enriched",
                json_path: None,
                account_name: Some("测试玩家甲"),
                player_name: Some("测试玩家甲"),
                agent_name: Some("Sova"),
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: Some("12/14"),
                kda: Some("13/17/4"),
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("metadata should seed");
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET match_id = 'match-opponent-name'
                WHERE clip_id = ?1
                ",
                params![clip.id],
            )
            .expect("match metadata should seed");
        connection
            .execute(
                "
                INSERT INTO matches (game_id, account_id, player_name, source_log)
                VALUES ('match-opponent-name', '9000000000000000002', '测试玩家甲', 1)
                ",
                [],
            )
            .expect("match should seed");

        let changed = clear_invalid_display_metadata(&connection).expect("cleanup should complete");

        assert_eq!(changed, 2);
        let metadata: (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = connection
            .query_row(
                "
                    SELECT account_name, player_name, map_name, kda
                    FROM clip_metadata
                    WHERE clip_id = ?1
                    ",
                params![clip.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("metadata should reload");
        assert_eq!(metadata.0, None);
        assert_eq!(metadata.1, None);
        assert_eq!(metadata.2.as_deref(), Some("隐世修所"));
        assert_eq!(metadata.3.as_deref(), Some("13/17/4"));

        let match_player_name: Option<String> = connection
            .query_row(
                "SELECT player_name FROM matches WHERE game_id = 'match-opponent-name'",
                [],
                |row| row.get(0),
            )
            .expect("match should reload");
        assert_eq!(match_player_name, None);
    }

    #[test]
    fn source_root_filter_treats_like_metacharacters_literally() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let mut clip_ids = Vec::new();

        for (root_name, group_key) in [
            ("clips_2026", "match-underscore-root"),
            ("clipsX2026", "match-sibling-root"),
        ] {
            let source_path = format!(r"D:\{root_name}\wonderfulVideos1001");
            let source_dir = upsert_source_dir(
                &connection,
                SourceDirInput {
                    path: &source_path,
                    name: "wonderfulVideos1001",
                },
            )
            .expect("source dir should upsert");
            let clip_group = upsert_clip_group(
                &connection,
                ClipGroupInput {
                    source_dir_id: source_dir.id,
                    group_key,
                    display_name: group_key,
                },
            )
            .expect("clip group should upsert");
            let video_path = format!(r"{source_path}\{group_key}\clip.mp4");
            let clip = upsert_clip(
                &connection,
                ClipInput {
                    source_dir_id: source_dir.id,
                    clip_group_id: Some(clip_group.id),
                    video_path: &video_path,
                    file_name: "clip.mp4",
                    file_size: 42,
                    modified_at: Some("1782634272"),
                    duration_ms: None,
                    recorded_at: None,
                    cover_path: None,
                    cover_source: "missing",
                },
            )
            .expect("clip should upsert");
            upsert_clip_metadata(
                &connection,
                ClipMetadataInput {
                    clip_id: clip.id,
                    metadata_status: "parsed",
                    json_path: None,
                    account_name: Some("Hint#1001"),
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
            .expect("metadata should seed");
            clip_ids.push(clip.id);
        }

        let changed =
            clear_weak_account_name_hints_for_source_root(&connection, Path::new(r"D:\clips_2026"))
                .expect("scoped cleanup should run");
        let first = find_clip_by_id(&connection, clip_ids[0]).expect("first clip should load");
        let sibling = find_clip_by_id(&connection, clip_ids[1]).expect("sibling clip should load");

        assert_eq!(changed, 1);
        assert_eq!(first.account_name, None);
        assert_eq!(sibling.account_name.as_deref(), Some("Hint#1001"));
    }

    #[test]
    fn clip_page_applies_default_maximum_and_invalid_pagination_bounds() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        seed_large_clip_fixture(&connection, 250);

        let default_page = list_clip_page(&connection, &ClipListQuery::default())
            .expect("default page should list");
        assert_eq!(default_page.offset, 0);
        assert_eq!(default_page.limit, DEFAULT_CLIP_PAGE_LIMIT);
        assert_eq!(default_page.items.len(), DEFAULT_CLIP_PAGE_LIMIT as usize);
        assert_eq!(default_page.total_count, 250);

        let maximum_page = list_clip_page(
            &connection,
            &ClipListQuery {
                limit: Some(MAX_CLIP_PAGE_LIMIT),
                ..ClipListQuery::default()
            },
        )
        .expect("maximum page should list");
        assert_eq!(maximum_page.items.len(), MAX_CLIP_PAGE_LIMIT as usize);

        for invalid_limit in [-1, 0, MAX_CLIP_PAGE_LIMIT + 1] {
            let error = list_clip_page(
                &connection,
                &ClipListQuery {
                    limit: Some(invalid_limit),
                    ..ClipListQuery::default()
                },
            )
            .expect_err("invalid limit should fail");
            assert!(error.contains("limit"));
        }
        let error = list_clip_page(
            &connection,
            &ClipListQuery {
                offset: Some(-1),
                ..ClipListQuery::default()
            },
        )
        .expect_err("negative offset should fail");
        assert!(error.contains("offset"));
    }

    #[test]
    fn clip_list_sort_deserialization_is_a_closed_whitelist() {
        let query: ClipListQuery = serde_json::from_value(serde_json::json!({
            "sortBy": "name-asc",
            "limit": 25
        }))
        .expect("known sort should deserialize");
        assert_eq!(query.sort_by, Some(ClipSort::NameAsc));

        let arbitrary = serde_json::from_value::<ClipListQuery>(serde_json::json!({
            "sortBy": "modified-desc; DROP TABLE clips"
        }));
        assert!(
            arbitrary.is_err(),
            "arbitrary SQL must not enter the sort path"
        );
    }

    #[test]
    fn clip_page_is_deterministic_without_duplicates_or_gaps() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let inserted_ids = seed_large_clip_fixture(&connection, 333);
        let mut expected_ids = inserted_ids;
        expected_ids.reverse();

        let mut collected_ids = Vec::new();
        let mut offset = 0;
        loop {
            let page = list_clip_page(
                &connection,
                &ClipListQuery {
                    offset: Some(offset),
                    limit: Some(37),
                    ..ClipListQuery::default()
                },
            )
            .expect("page should list");
            assert_eq!(page.total_count, 333);
            assert!(page.items.len() <= 37);
            collected_ids.extend(page.items.iter().map(|item| item.id));
            match page.next_offset {
                Some(next_offset) => {
                    assert!(page.has_more);
                    assert_eq!(next_offset, offset + page.items.len() as i64);
                    offset = next_offset;
                }
                None => {
                    assert!(!page.has_more);
                    break;
                }
            }
        }

        assert_eq!(collected_ids, expected_ids);
        assert_eq!(
            collected_ids.iter().copied().collect::<HashSet<_>>().len(),
            333
        );

        let beyond_end = list_clip_page(
            &connection,
            &ClipListQuery {
                offset: Some(400),
                limit: Some(25),
                ..ClipListQuery::default()
            },
        )
        .expect("offset beyond total should be valid");
        assert!(beyond_end.items.is_empty());
        assert_eq!(beyond_end.total_count, 333);
        assert!(!beyond_end.has_more);
        assert_eq!(beyond_end.next_offset, None);
    }

    #[test]
    fn clip_page_search_escapes_like_wildcards_backslashes_and_supports_chinese() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let percent = insert_page_fixture_clip(&connection, "percent.mp4", 100, 10);
        let underscore = insert_page_fixture_clip(&connection, "underscore.mp4", 101, 11);
        let backslash = insert_page_fixture_clip(&connection, "backslash.mp4", 102, 12);
        let chinese = insert_page_fixture_clip(&connection, "chinese.mp4", 103, 13);
        for (clip_id, note) in [
            (percent, "literal%marker"),
            (underscore, "literal_marker"),
            (backslash, "folder\\special\\marker"),
            (chinese, "中文复盘路径"),
        ] {
            connection
                .execute(
                    "UPDATE clips SET note = ?2 WHERE id = ?1",
                    params![clip_id, note],
                )
                .expect("search note should update");
        }

        for (query, expected_id) in [
            ("%marker", percent),
            ("_marker", underscore),
            ("folder\\special", backslash),
            ("中文", chinese),
        ] {
            let page = list_clip_page(
                &connection,
                &ClipListQuery {
                    query: Some(query.to_string()),
                    ..ClipListQuery::default()
                },
            )
            .expect("literal search should list");
            assert_eq!(page.total_count, 1, "query {query}");
            assert_eq!(page.items[0].id, expected_id, "query {query}");
        }
    }

    #[test]
    fn clip_page_matches_current_filters_with_or_search_and_and_cross_filter_semantics() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source_a = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\Library\\Ranked",
                name: "Ranked Highlights",
            },
        )
        .expect("source A should upsert");
        let source_b = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "E:\\Library\\Archive",
                name: "Archive",
            },
        )
        .expect("source B should upsert");
        let triple = insert_page_fixture_clip_for_source(
            &connection,
            source_a.id,
            "alpha-triple.mp4",
            1_000,
            100,
        );
        let quad = insert_page_fixture_clip_for_source(
            &connection,
            source_b.id,
            "bravo-quad.mp4",
            2_000,
            200,
        );
        let ace = insert_page_fixture_clip_for_source(
            &connection,
            source_b.id,
            "charlie-ace.mp4",
            3_000,
            300,
        );
        let compilation = insert_page_fixture_clip_for_source(
            &connection,
            source_b.id,
            "delta-compilation.mp4",
            4_000,
            400,
        );
        let trashed = insert_page_fixture_clip_for_source(
            &connection,
            source_b.id,
            "echo-trashed.mp4",
            5_000,
            500,
        );

        for (clip_id, status, favorite, metadata_status, agent, map, mode, kills) in [
            (
                triple,
                "available",
                1,
                "enriched",
                "Jett",
                "源工重镇",
                "竞技模式",
                3,
            ),
            (
                quad,
                "missing",
                0,
                "parsed",
                "Omen",
                "隐世修所",
                "极速模式",
                4,
            ),
            (
                ace,
                "available",
                0,
                "parsed",
                "Reyna",
                "霓虹町",
                "竞技模式",
                5,
            ),
            (
                compilation,
                "available",
                0,
                "parsed",
                "Sova",
                "森寒冬港",
                "未评级",
                1,
            ),
            (
                trashed,
                "trashed",
                0,
                "not_found",
                "Viper",
                "亚海悬城",
                "竞技模式",
                0,
            ),
        ] {
            if status == "trashed" {
                seed_test_trash_snapshot(&connection, clip_id);
            }
            connection
                .execute(
                    "UPDATE clips SET file_status = ?2, is_favorite = ?3 WHERE id = ?1",
                    params![clip_id, status, favorite],
                )
                .expect("clip state should update");
            upsert_clip_metadata(
                &connection,
                ClipMetadataInput {
                    clip_id,
                    metadata_status,
                    json_path: None,
                    account_name: (clip_id == triple).then_some("FixtureAlpha#0001"),
                    player_name: (clip_id == triple).then_some("FixtureAlpha#0001"),
                    agent_name: Some(agent),
                    map_name: Some(map),
                    game_mode: Some(mode),
                    scoreline: Some("13/10"),
                    kda: Some("20/10/5"),
                    extracted_text: (clip_id == triple).then_some("OCR 任一字段命中"),
                    parse_error: None,
                },
            )
            .expect("metadata should upsert");
            connection
                .execute(
                    "UPDATE clip_metadata SET kill_count = ?2 WHERE clip_id = ?1",
                    params![clip_id, kills],
                )
                .expect("kill count should update");
        }
        connection
            .execute(
                "UPDATE clips SET note = 'alpha-note-only' WHERE id = ?1",
                params![triple],
            )
            .expect("note should update");
        connection
            .execute(
                "UPDATE clip_metadata SET highlight_type = '2' WHERE clip_id = ?1",
                params![compilation],
            )
            .expect("compilation metadata should update");
        connection
            .execute(
                "
                INSERT INTO matches (game_id, account_id, player_name, map_id, started_at)
                VALUES ('fixture-match', 'account-alpha', 'FixtureAlpha#0001', 'Bind', '2026-07-01T00:00:00Z')
                ",
                [],
            )
            .expect("match should insert");
        let match_row_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO match_stats (match_id, combat_score) VALUES (?1, 321)",
                params![match_row_id],
            )
            .expect("match stats should insert");
        connection
            .execute(
                "UPDATE clip_metadata SET match_id = 'fixture-match' WHERE clip_id = ?1",
                params![triple],
            )
            .expect("match should link");

        let review =
            create_tag(&connection, "分页复盘", Some("blue")).expect("review tag should create");
        let secondary = create_tag(&connection, "次要标签", Some("green"))
            .expect("secondary tag should create");
        assign_tag_to_clip(&connection, triple, review.id).expect("review tag should assign");
        assign_tag_to_clip(&connection, triple, secondary.id).expect("secondary tag should assign");

        assert_eq!(
            page_ids_for_query(&connection, ClipListQuery::default()),
            vec![compilation, ace, quad, triple],
            "the default/all scope must exclude only trashed clips"
        );
        assert_eq!(
            page_ids_for_query(
                &connection,
                ClipListQuery {
                    source_dir_id: Some(source_a.id),
                    ..ClipListQuery::default()
                }
            ),
            vec![triple]
        );
        assert_eq!(
            page_ids_for_query(
                &connection,
                ClipListQuery {
                    account_id: Some("match-account-account-alpha".to_string()),
                    ..ClipListQuery::default()
                }
            ),
            vec![triple]
        );

        for (field, query, expected) in [
            (
                "agent",
                ClipListQuery {
                    agent_name: Some("Jett".to_string()),
                    ..ClipListQuery::default()
                },
                vec![triple],
            ),
            (
                "map",
                ClipListQuery {
                    map_name: Some("源工重镇".to_string()),
                    ..ClipListQuery::default()
                },
                vec![triple],
            ),
            (
                "game mode",
                ClipListQuery {
                    game_mode: Some("极速模式".to_string()),
                    ..ClipListQuery::default()
                },
                vec![quad],
            ),
            (
                "tag",
                ClipListQuery {
                    tag_id: Some(review.id),
                    ..ClipListQuery::default()
                },
                vec![triple],
            ),
            (
                "favorite",
                ClipListQuery {
                    favorite_filter: Some(FavoriteFilter::Favorite),
                    ..ClipListQuery::default()
                },
                vec![triple],
            ),
            (
                "not favorite",
                ClipListQuery {
                    favorite_filter: Some(FavoriteFilter::NotFavorite),
                    ..ClipListQuery::default()
                },
                vec![compilation, ace, quad],
            ),
            (
                "missing",
                ClipListQuery {
                    file_status: Some("missing".to_string()),
                    ..ClipListQuery::default()
                },
                vec![quad],
            ),
            (
                "trash",
                ClipListQuery {
                    file_status: Some("trashed".to_string()),
                    ..ClipListQuery::default()
                },
                vec![trashed],
            ),
            (
                "metadata",
                ClipListQuery {
                    metadata_status: Some("enriched".to_string()),
                    ..ClipListQuery::default()
                },
                vec![triple],
            ),
            (
                "modified range",
                ClipListQuery {
                    modified_from: Some(1_000),
                    modified_to: Some(1_000),
                    ..ClipListQuery::default()
                },
                vec![triple],
            ),
            (
                "size range",
                ClipListQuery {
                    size_min_bytes: Some(90),
                    size_max_bytes: Some(110),
                    ..ClipListQuery::default()
                },
                vec![triple],
            ),
        ] {
            assert_eq!(page_ids_for_query(&connection, query), expected, "{field}");
        }

        for (filter, expected) in [
            (HighlightFilter::Triple, vec![triple]),
            (HighlightFilter::Quad, vec![quad]),
            (HighlightFilter::Five, vec![ace]),
            (HighlightFilter::KillCompilation, vec![compilation]),
        ] {
            assert_eq!(
                page_ids_for_query(
                    &connection,
                    ClipListQuery {
                        highlight_filter: Some(filter),
                        ..ClipListQuery::default()
                    }
                ),
                expected,
                "highlight {filter:?}"
            );
        }

        let combined = list_clip_page(
            &connection,
            &ClipListQuery {
                query: Some("alpha-note-only".to_string()),
                source_dir_id: Some(source_a.id),
                agent_name: Some("Jett".to_string()),
                map_name: Some("源工重镇".to_string()),
                game_mode: Some("竞技模式".to_string()),
                tag_id: Some(review.id),
                favorite_filter: Some(FavoriteFilter::Favorite),
                file_status: Some("available".to_string()),
                metadata_status: Some("enriched".to_string()),
                ..ClipListQuery::default()
            },
        )
        .expect("combined filters should list");
        assert_eq!(page_ids(&combined), vec![triple]);
        assert_eq!(
            combined.total_count, 1,
            "tag joins must not duplicate totals"
        );
        assert_eq!(combined.items[0].tag_ids, vec![review.id, secondary.id]);
        assert_eq!(
            combined.items[0].account_identity_key,
            "match-account-account-alpha"
        );
        assert_eq!(combined.items[0].source_dir_path, "D:\\Library\\Ranked");
        assert_eq!(combined.items[0].combat_score, Some(321));

        let search_any_field = list_clip_page(
            &connection,
            &ClipListQuery {
                query: Some("OCR 任一字段".to_string()),
                ..ClipListQuery::default()
            },
        )
        .expect("search should OR across searchable fields");
        assert_eq!(page_ids(&search_any_field), vec![triple]);
    }

    #[test]
    fn video_type_filters_are_separate_from_user_tags_and_keep_product_order() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\Library\\VideoTypes",
                name: "Video Types",
            },
        )
        .expect("source should upsert");

        let triple =
            insert_page_fixture_clip_for_source(&connection, source.id, "triple.mp4", 1_000, 100);
        let quad =
            insert_page_fixture_clip_for_source(&connection, source.id, "quad.mp4", 2_000, 100);
        let five =
            insert_page_fixture_clip_for_source(&connection, source.id, "five.mp4", 3_000, 100);
        let six =
            insert_page_fixture_clip_for_source(&connection, source.id, "six.mp4", 4_000, 100);
        let compilation = insert_page_fixture_clip_for_source(
            &connection,
            source.id,
            "compilation.mp4",
            5_000,
            100,
        );
        let death =
            insert_page_fixture_clip_for_source(&connection, source.id, "death.mp4", 6_000, 100);
        let custom_tag_only = insert_page_fixture_clip_for_source(
            &connection,
            source.id,
            "custom-tag-only.mp4",
            7_000,
            100,
        );

        for (clip_id, highlight_type, kill_count) in [
            (triple, 4, Some(3)),
            (quad, 6, Some(4)),
            (five, 10, Some(5)),
            (six, 10, Some(6)),
            (compilation, 2, None),
            (death, 3, None),
        ] {
            connection
                .execute(
                    "
                    UPDATE clip_metadata
                    SET metadata_status = 'enriched',
                        highlight_type = ?2,
                        kill_count = ?3
                    WHERE clip_id = ?1
                    ",
                    params![clip_id, highlight_type, kill_count],
                )
                .expect("video type metadata should update");
        }
        let misleading_tag =
            create_tag(&connection, "六杀时刻", Some("red")).expect("custom tag should create");
        assign_tag_to_clip(&connection, custom_tag_only, misleading_tag.id)
            .expect("custom tag should assign");

        for (filter, expected) in [
            (HighlightFilter::Triple, triple),
            (HighlightFilter::Quad, quad),
            (HighlightFilter::Five, five),
            (HighlightFilter::Six, six),
            (HighlightFilter::KillCompilation, compilation),
            (HighlightFilter::Death, death),
        ] {
            assert_eq!(
                page_ids_for_query(
                    &connection,
                    ClipListQuery {
                        highlight_filter: Some(filter),
                        ..ClipListQuery::default()
                    }
                ),
                vec![expected],
                "video type {filter:?} must ignore custom tag text"
            );
        }

        let facets = get_library_facets(&connection).expect("facets should load");
        assert_eq!(
            facets
                .kill_types
                .iter()
                .map(|facet| facet.value.as_str())
                .collect::<Vec<_>>(),
            vec!["triple", "quad", "five", "six", "kill-compilation", "death"]
        );
        for value in ["triple", "quad", "five", "six", "kill-compilation", "death"] {
            assert_facet_count(&facets.kill_types, value, 1, 1);
        }
    }

    #[test]
    fn clip_page_preserves_match_openid_and_source_account_identities() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let openid = "1234567890123456";
        let openid_source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\Accounts\\wonderfulVideos1234567890123456",
                name: "wonderfulVideos1234567890123456",
            },
        )
        .expect("openid source should upsert");
        let plain_source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "E:\\Manual",
                name: "Manual",
            },
        )
        .expect("plain source should upsert");
        let openid_clip = insert_page_fixture_clip_for_source(
            &connection,
            openid_source.id,
            "openid.mp4",
            100,
            10,
        );
        let source_clip = insert_page_fixture_clip_for_source(
            &connection,
            plain_source.id,
            "source.mp4",
            200,
            20,
        );
        let match_clip =
            insert_page_fixture_clip_for_source(&connection, plain_source.id, "match.mp4", 300, 30);
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: match_clip,
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
        .expect("match metadata should upsert");
        connection
            .execute(
                "INSERT INTO matches (game_id, account_id) VALUES ('identity-match', 'match-identity')",
                [],
            )
            .expect("identity match should insert");
        connection
            .execute(
                "UPDATE clip_metadata SET match_id = 'identity-match' WHERE clip_id = ?1",
                params![match_clip],
            )
            .expect("identity match should link");

        for (account_id, expected) in [
            (format!("match-account-{openid}"), vec![openid_clip]),
            (format!("source-{}", plain_source.id), vec![source_clip]),
            ("match-account-match-identity".to_string(), vec![match_clip]),
        ] {
            let page = list_clip_page(
                &connection,
                &ClipListQuery {
                    account_id: Some(account_id.clone()),
                    ..ClipListQuery::default()
                },
            )
            .expect("identity page should list");
            assert_eq!(page_ids(&page), expected, "account {account_id}");
        }

        let openid_page = list_clip_page(
            &connection,
            &ClipListQuery {
                query: Some(format!("账号 {openid}")),
                ..ClipListQuery::default()
            },
        )
        .expect("openid display search should list");
        assert_eq!(page_ids(&openid_page), vec![openid_clip]);
        assert_eq!(
            openid_page.items[0].account_identity_source,
            AccountIdentitySource::Openid
        );
        assert_eq!(
            openid_page.items[0].account_display_name,
            format!("账号 {openid}")
        );
    }

    #[test]
    fn clip_page_supports_every_production_sort_with_stable_id_ties() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let first = insert_page_fixture_clip(&connection, "z.mp4", 100, 10);
        let second = insert_page_fixture_clip(&connection, "clip-10.mp4", 200, 10);
        let third = insert_page_fixture_clip(&connection, "clip-2.mp4", 200, 5);

        for (sort_by, expected) in [
            (ClipSort::ModifiedDesc, vec![third, second, first]),
            (ClipSort::ModifiedAsc, vec![first, second, third]),
            (ClipSort::SizeDesc, vec![second, first, third]),
            (ClipSort::SizeAsc, vec![third, first, second]),
            (ClipSort::NameAsc, vec![third, second, first]),
        ] {
            let page = list_clip_page(
                &connection,
                &ClipListQuery {
                    sort_by: Some(sort_by),
                    ..ClipListQuery::default()
                },
            )
            .expect("sorted page should list");
            assert_eq!(page_ids(&page), expected, "sort {sort_by:?}");
        }
    }

    #[test]
    fn clip_page_summary_omits_detail_payloads_and_batches_current_page_tags() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip_ids = seed_large_clip_fixture(&connection, 205);
        let first_id = clip_ids[0];
        connection
            .execute(
                "UPDATE clips SET note = 'detail-only-note' WHERE id = ?1",
                params![first_id],
            )
            .expect("note should update");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: first_id,
                metadata_status: "parsed",
                json_path: Some("D:/detail-only.json"),
                account_name: None,
                player_name: None,
                agent_name: None,
                map_name: None,
                game_mode: None,
                scoreline: None,
                kda: None,
                extracted_text: Some("detail-only-extracted-text"),
                parse_error: None,
            },
        )
        .expect("detail metadata should upsert");
        replace_clip_timeline(
            &connection,
            first_id,
            &[],
            &[ClipEventInput {
                segment_key: None,
                event_key: "summary-must-not-load-this-event",
                event_type: "kill",
                video_time_ms: Some(500),
                event_time: None,
                round_id: Some(1),
                player_name: Some("Tester"),
                agent_name: Some("Jett"),
                weapon_name: Some("Vandal"),
                killer_name: Some("Tester"),
                killed_name: Some("Enemy"),
                killer_is_me: true,
                raw_json: Some("{\"detail\":true}"),
            }],
        )
        .expect("event should seed");
        let tag =
            create_tag(&connection, "页内批量标签", Some("gold")).expect("page tag should create");
        for clip_id in clip_ids.iter().take(30) {
            assign_tag_to_clip(&connection, *clip_id, tag.id).expect("tag should assign");
        }

        let (page, statements) = trace_sql_queries(&connection, || {
            list_clip_page(
                &connection,
                &ClipListQuery {
                    limit: Some(200),
                    ..ClipListQuery::default()
                },
            )
            .expect("summary page should list")
        });
        assert_eq!(page.items.len(), 200);
        assert!(
            statements
                .iter()
                .all(|sql| !sql.contains("FROM clip_events")),
            "summary page must not query events: {statements:#?}"
        );
        let page_tag_queries = statements
            .iter()
            .filter(|sql| sql.contains("SELECT clip_id, tag_id") && sql.contains("FROM clip_tags"))
            .collect::<Vec<_>>();
        assert_eq!(
            page_tag_queries.len(),
            1,
            "current-page tag ids must use one batch query: {page_tag_queries:#?}"
        );
        let serialized = serde_json::to_value(&page.items[0]).expect("summary should serialize");
        for forbidden in [
            "clipEvents",
            "note",
            "extractedText",
            "rawJson",
            "normalizedPath",
        ] {
            assert!(
                serialized.get(forbidden).is_none(),
                "summary must omit {forbidden}: {serialized}"
            );
        }
    }

    #[test]
    fn clip_detail_loads_only_the_target_full_events_and_tags_and_reports_absence() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let target = insert_page_fixture_clip(&connection, "target-detail.mp4", 100, 10);
        let other = insert_page_fixture_clip(&connection, "other-detail.mp4", 200, 20);
        connection
            .execute(
                "UPDATE clips SET note = 'full detail note' WHERE id = ?1",
                params![target],
            )
            .expect("detail note should update");
        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: target,
                metadata_status: "parsed",
                json_path: Some("D:/target-detail.json"),
                account_name: Some("Target#1000"),
                player_name: Some("Target#1000"),
                agent_name: Some("Jett"),
                map_name: Some("源工重镇"),
                game_mode: Some("竞技模式"),
                scoreline: Some("13/9"),
                kda: Some("20/9/3"),
                extracted_text: Some("full detail extracted text"),
                parse_error: None,
            },
        )
        .expect("detail metadata should upsert");
        for (clip_id, event_key, raw_json) in [
            (target, "target-event", "{\"target\":true}"),
            (other, "other-event", "{\"other\":true}"),
        ] {
            replace_clip_timeline(
                &connection,
                clip_id,
                &[],
                &[ClipEventInput {
                    segment_key: None,
                    event_key,
                    event_type: "kill",
                    video_time_ms: Some(750),
                    event_time: None,
                    round_id: Some(2),
                    player_name: Some("Tester"),
                    agent_name: Some("Jett"),
                    weapon_name: Some("Phantom"),
                    killer_name: Some("Tester"),
                    killed_name: Some("Enemy"),
                    killer_is_me: true,
                    raw_json: Some(raw_json),
                }],
            )
            .expect("detail event should seed");
        }
        let tag =
            create_tag(&connection, "详情完整标签", Some("red")).expect("detail tag should create");
        assign_tag_to_clip(&connection, target, tag.id).expect("detail tag should assign");

        let (detail, statements) = trace_sql_queries(&connection, || {
            find_clip_detail_by_id(&connection, target)
                .expect("detail query should succeed")
                .expect("target detail should exist")
        });
        let event_queries = statements
            .iter()
            .filter(|sql| sql.contains("FROM clip_events"))
            .collect::<Vec<_>>();
        assert_eq!(event_queries.len(), 1);
        assert!(event_queries[0].contains(&format!("IN ({target})")));
        assert_eq!(detail.clip.id, target);
        assert_eq!(detail.clip.event_count, 1);
        assert_eq!(detail.clip.clip_events[0].event_key, "target-event");
        assert_eq!(
            detail.clip.clip_events[0].raw_json.as_deref(),
            Some("{\"target\":true}")
        );
        assert_eq!(detail.tags, vec![tag]);
        assert_eq!(detail.clip.tag_ids, vec![detail.tags[0].id]);
        assert!(
            find_clip_detail_by_id(&connection, other + 10_000)
                .expect("missing detail query should succeed")
                .is_none(),
            "missing clips must not produce empty placeholder objects"
        );
    }

    #[test]
    fn clip_page_large_fixtures_keep_materialized_results_bounded() {
        for fixture_size in [1_000usize, 10_000usize] {
            let connection = Connection::open_in_memory().expect("in-memory db should open");
            initialize_schema(&connection).expect("schema should initialize");
            seed_large_clip_fixture(&connection, fixture_size);
            let started_at = Instant::now();
            let (page, statements) = trace_sql_queries(&connection, || {
                list_clip_page(
                    &connection,
                    &ClipListQuery {
                        offset: Some((fixture_size / 2) as i64),
                        limit: Some(64),
                        ..ClipListQuery::default()
                    },
                )
                .expect("large fixture page should list")
            });
            let elapsed = started_at.elapsed();
            eprintln!(
                "bounded clip page fixture={fixture_size} items={} elapsed={elapsed:?}",
                page.items.len()
            );

            assert_eq!(page.total_count, fixture_size as i64);
            assert_eq!(page.items.len(), 64);
            assert!(page.items.len() <= page.limit as usize);
            assert!(statements
                .iter()
                .all(|sql| !sql.contains("FROM clip_events")));
            assert_eq!(
                statements
                    .iter()
                    .filter(|sql| sql.contains("SELECT clip_id, tag_id"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn library_facets_return_complete_zero_values_for_an_empty_clip_index() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");

        let facets = get_library_facets(&connection).expect("empty facets should load");

        assert_eq!(facets.total_count, 0);
        assert_eq!(facets.active_count, 0);
        assert_eq!(facets.favorite_count, 0);
        assert_eq!(facets.active_favorite_count, 0);
        assert_eq!(facets.trashed_count, 0);
        assert_eq!(facets.tagged_count, 0);
        assert_eq!(facets.active_tagged_count, 0);
        assert_eq!(facets.total_size_bytes, 0);
        assert_eq!(facets.active_size_bytes, 0);
        assert_eq!(facets.size_bytes_min, None);
        assert_eq!(facets.size_bytes_max, None);
        assert_eq!(facets.recent_count, 0);
        assert_eq!(facets.recorded_at_min, None);
        assert_eq!(facets.recorded_at_max, None);
        assert_eq!(facets.modified_at_min, None);
        assert_eq!(facets.modified_at_max, None);
        assert!(facets.file_statuses.is_empty());
        assert!(facets.metadata_statuses.is_empty());
        assert!(facets.accounts.is_empty());
        assert!(facets.source_dirs.is_empty());
        assert!(facets.agents.is_empty());
        assert!(facets.maps.is_empty());
        assert!(facets.game_modes.is_empty());
        assert!(facets.kill_types.is_empty());
        assert!(facets.tags.is_empty());
    }

    #[test]
    fn library_facets_count_every_dimension_and_unify_renamed_accounts() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let account_source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: r"D:\FacetLibrary\wonderfulVideos1001",
                name: "wonderfulVideos1001",
            },
        )
        .expect("account source should upsert");
        let archive_source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: r"E:\FacetArchive",
                name: "Facet Archive",
            },
        )
        .expect("archive source should upsert");

        let available = insert_library_facet_clip(
            &connection,
            LibraryFacetClipInput {
                source_dir_id: account_source.id,
                file_name: "available.mp4",
                modified_at: 100,
                recorded_at: 1_000,
                file_size: 100,
                file_status: "available",
                favorite: true,
                metadata_status: Some("enriched"),
                account_name: Some("OldName#1001"),
                agent_name: Some("Jett"),
                map_name: Some("源工重镇"),
                game_mode: Some("竞技模式"),
                match_id: Some("facet-match-a"),
                match_account_id: Some("1001"),
                kill_count: Some(3),
                highlight_type: None,
            },
        );
        let missing = insert_library_facet_clip(
            &connection,
            LibraryFacetClipInput {
                source_dir_id: account_source.id,
                file_name: "missing.mp4",
                modified_at: 200,
                recorded_at: 2_000,
                file_size: 200,
                file_status: "missing",
                favorite: false,
                metadata_status: Some("enriched"),
                account_name: Some("MiddleName#1001"),
                agent_name: Some("捷风"),
                map_name: Some("源工重镇"),
                game_mode: Some("竞技模式"),
                match_id: Some("facet-match-b"),
                match_account_id: Some("1001"),
                kill_count: Some(4),
                highlight_type: None,
            },
        );
        let trashed = insert_library_facet_clip(
            &connection,
            LibraryFacetClipInput {
                source_dir_id: archive_source.id,
                file_name: "trashed.mp4",
                modified_at: 300,
                recorded_at: 3_000,
                file_size: 300,
                file_status: "trashed",
                favorite: true,
                metadata_status: Some("parsed"),
                account_name: Some("Archive#2002"),
                agent_name: Some("KAY/O"),
                map_name: Some("隐世修所"),
                game_mode: Some("极速模式"),
                match_id: None,
                match_account_id: None,
                kill_count: Some(5),
                highlight_type: Some(10),
            },
        );
        let compilation = insert_library_facet_clip(
            &connection,
            LibraryFacetClipInput {
                source_dir_id: account_source.id,
                file_name: "compilation.mp4",
                modified_at: 400,
                recorded_at: 4_000,
                file_size: 400,
                file_status: "available",
                favorite: false,
                metadata_status: Some("enriched"),
                account_name: Some("NewestName#1001"),
                agent_name: Some("幽影"),
                map_name: Some("霓虹町"),
                game_mode: Some("竞技模式"),
                match_id: None,
                match_account_id: None,
                kill_count: Some(1),
                highlight_type: Some(2),
            },
        );
        let _other_status = insert_library_facet_clip(
            &connection,
            LibraryFacetClipInput {
                source_dir_id: archive_source.id,
                file_name: "offline.mp4",
                modified_at: 500,
                recorded_at: 5_000,
                file_size: 500,
                file_status: "offline",
                favorite: false,
                metadata_status: None,
                account_name: None,
                agent_name: None,
                map_name: None,
                game_mode: None,
                match_id: None,
                match_account_id: None,
                kill_count: None,
                highlight_type: None,
            },
        );
        let alpha =
            create_tag(&connection, "Alpha Label", Some("red")).expect("alpha tag should create");
        let beta =
            create_tag(&connection, "Beta Label", Some("green")).expect("beta tag should create");
        for clip_id in [available, missing, trashed] {
            assign_tag_to_clip(&connection, clip_id, alpha.id).expect("alpha should assign");
        }
        for clip_id in [available, compilation] {
            assign_tag_to_clip(&connection, clip_id, beta.id).expect("beta should assign");
        }

        let facets = get_library_facets(&connection).expect("facets should aggregate");

        assert_eq!(facets.total_count, 5);
        assert_eq!(facets.active_count, 4);
        assert_eq!(facets.favorite_count, 2);
        assert_eq!(facets.active_favorite_count, 1);
        assert_eq!(facets.trashed_count, 1);
        assert_eq!(
            facets.tagged_count, 4,
            "tag joins must not duplicate clip totals"
        );
        assert_eq!(facets.active_tagged_count, 3);
        assert_eq!(facets.total_size_bytes, 1_500);
        assert_eq!(facets.active_size_bytes, 1_200);
        assert_eq!(facets.size_bytes_min, Some(100));
        assert_eq!(facets.size_bytes_max, Some(500));
        assert_eq!(facets.recorded_at_min, Some(1_000));
        assert_eq!(facets.recorded_at_max, Some(5_000));
        assert_eq!(facets.modified_at_min, Some(100));
        assert_eq!(facets.modified_at_max, Some(500));

        assert_facet_count(&facets.file_statuses, "available", 2, 2);
        assert_facet_count(&facets.file_statuses, "missing", 1, 1);
        assert_facet_count(&facets.file_statuses, "trashed", 1, 0);
        assert_facet_count(&facets.file_statuses, "offline", 1, 1);
        assert_facet_count(&facets.metadata_statuses, "enriched", 3, 3);
        assert_facet_count(&facets.metadata_statuses, "parsed", 1, 0);
        assert_facet_count(&facets.metadata_statuses, "not_found", 1, 1);

        assert_eq!(facets.accounts.len(), 2);
        let renamed_account = facets
            .accounts
            .iter()
            .find(|facet| facet.account_identity_key == "match-account-1001")
            .expect("stable account should exist once");
        assert_eq!(renamed_account.account_display_name, "NewestName#1001");
        assert_eq!(renamed_account.count, 3);
        assert_eq!(renamed_account.active_count, 3);
        let archive_account = facets
            .accounts
            .iter()
            .find(|facet| facet.account_identity_key == format!("source-{}", archive_source.id))
            .expect("source fallback account should exist");
        assert_eq!(archive_account.account_display_name, "Archive#2002");
        assert_eq!(archive_account.count, 2);
        assert_eq!(archive_account.active_count, 1);

        assert_eq!(facets.source_dirs.len(), 2);
        assert_eq!(facets.source_dirs[0].source_dir_id, account_source.id);
        assert_eq!(
            (
                facets.source_dirs[0].count,
                facets.source_dirs[0].active_count
            ),
            (3, 3)
        );
        assert_facet_count(&facets.agents, "捷风", 2, 2);
        assert_facet_count(&facets.agents, "K/O", 1, 0);
        assert_facet_count(&facets.agents, "幽影", 1, 1);
        assert!(
            facets.agents.iter().all(|facet| facet.value != "Jett"),
            "source-facing and localized names must not create duplicate hero options"
        );

        let localized_agent_page = list_clip_page(
            &connection,
            &ClipListQuery {
                agent_name: Some("捷风".to_string()),
                ..ClipListQuery::default()
            },
        )
        .expect("localized agent facet should filter source-facing and localized rows");
        assert_eq!(localized_agent_page.total_count, 2);
        assert_eq!(
            localized_agent_page
                .items
                .iter()
                .map(|clip| clip.id)
                .collect::<HashSet<_>>(),
            HashSet::from([available, missing])
        );

        let source_agent_page = list_clip_page(
            &connection,
            &ClipListQuery {
                agent_name: Some("Jett".to_string()),
                ..ClipListQuery::default()
            },
        )
        .expect("source-facing agent filters should remain backward compatible");
        assert_eq!(source_agent_page.total_count, 2);

        let kayo_page = list_clip_page(
            &connection,
            &ClipListQuery {
                agent_name: Some("K/O".to_string()),
                file_status: Some("trashed".to_string()),
                ..ClipListQuery::default()
            },
        )
        .expect("K/O slash aliases should survive facet selection and filtering");
        assert_eq!(kayo_page.total_count, 1);
        assert_eq!(kayo_page.items[0].id, trashed);
        assert_facet_count(&facets.maps, "源工重镇", 2, 2);
        assert_facet_count(&facets.maps, "隐世修所", 1, 0);
        assert_facet_count(&facets.maps, "霓虹町", 1, 1);
        assert_facet_count(&facets.game_modes, "竞技模式", 3, 3);
        assert_facet_count(&facets.game_modes, "极速模式", 1, 0);
        assert_facet_count(&facets.kill_types, "triple", 1, 1);
        assert_facet_count(&facets.kill_types, "quad", 1, 1);
        assert_facet_count(&facets.kill_types, "five", 1, 0);
        assert_facet_count(&facets.kill_types, "kill-compilation", 1, 1);

        let alpha_facet = facets
            .tags
            .iter()
            .find(|facet| facet.id == alpha.id)
            .expect("alpha facet should exist");
        assert_eq!((alpha_facet.count, alpha_facet.active_count), (3, 2));
        assert_eq!(alpha_facet.name, "Alpha Label");
        assert_eq!(alpha_facet.color.as_deref(), Some("red"));
        let beta_facet = facets
            .tags
            .iter()
            .find(|facet| facet.id == beta.id)
            .expect("beta facet should exist");
        assert_eq!((beta_facet.count, beta_facet.active_count), (2, 2));
    }

    #[test]
    fn library_facets_large_fixtures_are_exact_fixed_query_and_uniquely_bounded() {
        let mut returned_item_count = None;
        for fixture_size in [1_000usize, 10_000usize] {
            let connection = Connection::open_in_memory().expect("in-memory db should open");
            initialize_schema(&connection).expect("schema should initialize");
            seed_large_clip_fixture(&connection, fixture_size);
            let started_at = Instant::now();
            let (facets, statements) = trace_sql_queries(&connection, || {
                get_library_facets(&connection).expect("large facets should aggregate")
            });
            let elapsed = started_at.elapsed();
            eprintln!(
                "library facets fixture={fixture_size} returned={} elapsed={elapsed:?}",
                library_facet_item_count(&facets)
            );

            assert_eq!(facets.total_count, fixture_size as i64);
            assert_eq!(facets.active_count, fixture_size as i64);
            assert_eq!(
                facets.total_size_bytes,
                (fixture_size * (fixture_size + 1) / 2) as i64
            );
            assert_eq!(facets.size_bytes_min, Some(1));
            assert_eq!(facets.size_bytes_max, Some(fixture_size as i64));
            assert_eq!(facets.file_statuses.len(), 1);
            assert_eq!(facets.metadata_statuses.len(), 1);
            assert_eq!(facets.accounts.len(), 1);
            assert_eq!(facets.source_dirs.len(), 1);
            assert!(facets.agents.is_empty());
            assert!(facets.maps.is_empty());
            assert!(facets.game_modes.is_empty());
            assert!(facets.kill_types.is_empty());
            assert!(facets.tags.is_empty());

            let item_count = library_facet_item_count(&facets);
            assert_eq!(*returned_item_count.get_or_insert(item_count), item_count);
            let aggregate_statements = statements
                .iter()
                .filter(|sql| {
                    let sql = sql.trim_start().to_ascii_uppercase();
                    sql.starts_with("SELECT") || sql.starts_with("WITH")
                })
                .collect::<Vec<_>>();
            assert_eq!(
                aggregate_statements.len(),
                7,
                "facet query count must not depend on clip count: {aggregate_statements:#?}"
            );
            assert!(statements.iter().all(|sql| {
                !sql.contains("FROM clip_events")
                    && !sql.contains("raw_json")
                    && !sql.contains("extra_json")
            }));
        }
    }

    #[test]
    fn version_ten_moves_legacy_video_type_tags_into_metadata_and_keeps_custom_tags() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip = insert_test_clip(&connection);
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET metadata_source = 'video_export',
                    json_path = 'D:/Clips/videoExportTmp/config-six.json'
                WHERE clip_id = ?1
                ",
                params![clip.id],
            )
            .expect("legacy metadata should seed");
        let legacy_type =
            create_tag(&connection, "六杀", Some("gold")).expect("legacy tag should create");
        let custom =
            create_tag(&connection, "复盘", Some("blue")).expect("custom tag should create");
        assign_tag_to_clip(&connection, clip.id, legacy_type.id).expect("legacy tag should assign");
        assign_tag_to_clip(&connection, clip.id, custom.id).expect("custom tag should assign");
        connection
            .pragma_update(None, "user_version", 9)
            .expect("legacy schema version should seed");

        initialize_schema(&connection).expect("version ten migration should run");

        let classification: (Option<i64>, Option<i64>) = connection
            .query_row(
                "
                SELECT CAST(highlight_type AS INTEGER), kill_count
                FROM clip_metadata
                WHERE clip_id = ?1
                ",
                params![clip.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("classification should load");
        assert_eq!(classification, (Some(10), Some(6)));
        let tags = list_tags(&connection).expect("tags should list");
        assert_eq!(
            tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>(),
            vec!["复盘"]
        );
        assert_eq!(
            find_clip_by_id(&connection, clip.id)
                .expect("clip should reload")
                .tag_ids,
            vec![custom.id]
        );

        let post_migration =
            create_tag(&connection, "六杀", Some("green")).expect("new custom tag should create");
        initialize_schema(&connection).expect("current migration should be idempotent");
        assert!(list_tags(&connection)
            .expect("tags should list")
            .iter()
            .any(|tag| tag.id == post_migration.id));
    }

    #[test]
    fn version_eleven_backfills_official_score_provenance_only_during_upgrade() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip = insert_test_clip(&connection);
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET round_score = '921',
                    round_score_source = NULL,
                    metadata_source = 'wonderful_db'
                WHERE clip_id = ?1
                ",
                params![clip.id],
            )
            .expect("legacy official score should seed");
        connection
            .pragma_update(None, "user_version", 10)
            .expect("version ten marker should seed");

        initialize_schema(&connection).expect("version eleven migration should run");
        let migrated_source: Option<String> = connection
            .query_row(
                "SELECT round_score_source FROM clip_metadata WHERE clip_id = ?1",
                params![clip.id],
                |row| row.get(0),
            )
            .expect("migrated score source should load");
        assert_eq!(migrated_source.as_deref(), Some("wonderful_db"));

        connection
            .execute(
                "UPDATE clip_metadata SET round_score_source = NULL WHERE clip_id = ?1",
                params![clip.id],
            )
            .expect("explicit provenance removal should seed");
        initialize_schema(&connection).expect("current schema repair should be idempotent");
        let current_source: Option<String> = connection
            .query_row(
                "SELECT round_score_source FROM clip_metadata WHERE clip_id = ?1",
                params![clip.id],
                |row| row.get(0),
            )
            .expect("current score source should load");
        assert_eq!(current_source, None);
    }

    #[test]
    fn version_seven_migration_adds_paging_indexes_to_old_databases_idempotently() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        create_v4_database_with_user_state(&connection);

        initialize_schema(&connection).expect("old database should migrate");
        initialize_schema(&connection).expect("migration should be repeatable");

        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        let mut statement = connection
            .prepare(
                "
                SELECT name
                FROM sqlite_master
                WHERE type = 'index'
                  AND name LIKE 'idx_%page_%'
                ORDER BY name
                ",
            )
            .expect("paging index query should prepare");
        let indexes = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("paging indexes should query")
            .collect::<Result<Vec<_>, _>>()
            .expect("paging indexes should read");
        assert_eq!(
            indexes,
            vec![
                "idx_clips_page_modified_id",
                "idx_clips_page_name_id",
                "idx_clips_page_size_id"
            ]
        );
        assert_index_exists(&connection, "idx_clip_tags_tag_clip");
    }

    #[test]
    fn version_eight_migration_adds_thumbnail_queue_without_touching_source_cover_state() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        create_v4_database_with_user_state(&connection);
        initialize_schema(&connection).expect("fixture should first reach current schema");
        connection
            .execute_batch(
                "
                DROP TABLE clip_thumbnails;
                PRAGMA user_version = 7;
                ",
            )
            .expect("fixture should emulate schema version seven");

        let source_cover_before: (Option<String>, String) = connection
            .query_row(
                "SELECT cover_path, cover_source FROM clips WHERE id = 42",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("source cover state should be readable");
        initialize_schema(&connection).expect("v7 database should migrate");
        initialize_schema(&connection).expect("v8 migration should be repeatable");

        assert_eq!(schema_user_version(&connection), SCHEMA_VERSION);
        assert_table_exists(&connection, "clip_thumbnails");
        assert_index_exists(&connection, "idx_clip_thumbnails_status_due");
        assert_index_exists(&connection, "idx_clip_thumbnails_cache_file");
        let source_cover_after: (Option<String>, String) = connection
            .query_row(
                "SELECT cover_path, cover_source FROM clips WHERE id = 42",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("source cover state should remain readable");
        assert_eq!(source_cover_after, source_cover_before);
    }

    #[test]
    fn upsert_clip_then_list_clips_returns_inserted_clip() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");

        let source_dir = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\Clips\\Valorant",
                name: "Valorant",
            },
        )
        .expect("source dir should upsert");
        let clip_group = upsert_clip_group(
            &connection,
            ClipGroupInput {
                source_dir_id: source_dir.id,
                group_key: "2026-06-28",
                display_name: "2026-06-28",
            },
        )
        .expect("clip group should upsert");

        let inserted = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\Clips\\Valorant\\ace.mp4",
                file_name: "ace.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: Some(12_000),
                recorded_at: Some("2026-06-28T10:11:12Z"),
                cover_path: Some("D:\\Clips\\Valorant\\cover-ace.jpeg"),
                cover_source: "file",
            },
        )
        .expect("clip should upsert");
        upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: Some(clip_group.id),
                video_path: "D:\\Clips\\Valorant\\ace.mp4",
                file_name: "ace.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: Some("D:\\Clips\\Valorant\\cover-ace.jpeg"),
                cover_source: "file",
            },
        )
        .expect("non-official rescan should upsert");

        let clips = list_clips(&connection).expect("clips should list");

        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].id, inserted.id);
        assert_eq!(clips[0].file_name, "ace.mp4");
        assert_eq!(clips[0].source_dir_id, source_dir.id);
        assert_eq!(clips[0].clip_group_id, Some(clip_group.id));
        assert_eq!(clips[0].file_size, 42);
        assert_eq!(clips[0].modified_at.as_deref(), Some("1782634272"));
        assert_eq!(clips[0].duration_ms, None);
        assert_eq!(clips[0].recorded_at, None);
        assert_eq!(clips[0].cover_source, "file");
        assert_eq!(
            clips[0].cover_path.as_deref(),
            Some("D:\\Clips\\Valorant\\cover-ace.jpeg")
        );
        assert_eq!(clips[0].status, "available");
        assert!(!clips[0].favorite);
        assert_eq!(clips[0].note, None);
    }

    #[test]
    fn list_sources_returns_root_clips_and_zero_clip_sources_from_schema_state() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");

        let direct_source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\Direct Clips",
                name: "Direct Clips",
            },
        )
        .expect("direct source should upsert");
        let empty_source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "E:\\Empty Clips",
                name: "Empty Clips",
            },
        )
        .expect("empty source should upsert");
        upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: direct_source.id,
                clip_group_id: None,
                video_path: "D:\\Direct Clips\\root-clip.mp4",
                file_name: "root-clip.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("root clip should upsert");
        mark_source_dir_scanned(&connection, direct_source.id)
            .expect("direct source scan time should update");
        connection
            .execute(
                "
                UPDATE source_dirs
                SET status = 'unavailable',
                    last_error = 'directory is offline',
                    last_scanned_at = '2026-07-02 22:35:00'
                WHERE id = ?1
                ",
                params![empty_source.id],
            )
            .expect("empty source status should update");

        let sources = list_sources(&connection).expect("sources should list");

        assert_eq!(sources.len(), 2);
        let direct = sources
            .iter()
            .find(|source| source.id == direct_source.id)
            .expect("direct source should be returned");
        assert_eq!(direct.path, "D:\\Direct Clips");
        assert_eq!(direct.display_name, "Direct Clips");
        assert_eq!(direct.clip_count, 1);
        assert!(direct.accessibility);
        assert!(direct.last_scan_at.is_some());
        let json = serde_json::to_value(direct).expect("source DTO should serialize");
        assert_eq!(json["displayName"], "Direct Clips");
        assert_eq!(json["accessibility"], true);
        assert_eq!(json["clipCount"], 1);
        assert!(json.get("accessible").is_none());

        let empty = sources
            .iter()
            .find(|source| source.id == empty_source.id)
            .expect("empty source should be returned");
        assert_eq!(empty.clip_count, 0);
        assert!(!empty.accessibility);
        assert_eq!(empty.status, "unavailable");
        assert_eq!(empty.last_error.as_deref(), Some("directory is offline"));
        assert_eq!(empty.last_scan_at.as_deref(), Some("2026-07-02 22:35:00"));
    }

    #[test]
    fn clip_dto_uses_stable_account_identity_without_player_names_in_the_key() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let official_source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "D:\\ACLOS\\wonderfulVideos2002",
                name: "wonderfulVideos2002",
            },
        )
        .expect("official source should upsert");
        let manual_source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: "E:\\Manual Clips",
                name: "Manual Clips",
            },
        )
        .expect("manual source should upsert");
        let insert_clip = |source_dir_id, path: &str, file_name: &str| {
            upsert_clip(
                &connection,
                ClipInput {
                    source_dir_id,
                    clip_group_id: None,
                    video_path: path,
                    file_name,
                    file_size: 42,
                    modified_at: None,
                    duration_ms: None,
                    recorded_at: None,
                    cover_path: None,
                    cover_source: "missing",
                },
            )
            .expect("clip should upsert")
        };
        let match_clip = insert_clip(
            official_source.id,
            "D:\\ACLOS\\wonderfulVideos2002\\matched.mp4",
            "matched.mp4",
        );
        let openid_clip = insert_clip(
            official_source.id,
            "D:\\ACLOS\\wonderfulVideos2002\\openid.mp4",
            "openid.mp4",
        );
        let fallback_clip = insert_clip(
            manual_source.id,
            "E:\\Manual Clips\\fallback.mp4",
            "fallback.mp4",
        );
        connection
            .execute(
                "INSERT INTO matches (game_id, account_id) VALUES ('match-a', '2002')",
                [],
            )
            .expect("match should seed");
        for (clip_id, account_name, match_id) in [
            (match_clip.id, "旧名字#1001", Some("match-a")),
            (openid_clip.id, "新名字#2002", None),
            (fallback_clip.id, "任意玩家名#9999", None),
        ] {
            connection
                .execute(
                    "
                    INSERT INTO clip_metadata (
                        clip_id,
                        metadata_status,
                        account_name,
                        player_name,
                        match_id
                    )
                    VALUES (?1, 'enriched', ?2, ?2, ?3)
                    ON CONFLICT(clip_id) DO UPDATE SET
                        metadata_status = excluded.metadata_status,
                        account_name = excluded.account_name,
                        player_name = excluded.player_name,
                        match_id = excluded.match_id
                    ",
                    params![clip_id, account_name, match_id],
                )
                .expect("clip metadata should seed");
        }

        let clips = list_clips(&connection).expect("clips should list");
        let by_id = clips
            .iter()
            .map(|clip| (clip.id, clip))
            .collect::<HashMap<_, _>>();
        let matched = by_id[&match_clip.id];
        let from_openid = by_id[&openid_clip.id];
        let from_source = by_id[&fallback_clip.id];

        assert_eq!(matched.account_identity_key, "match-account-2002");
        assert_eq!(
            matched.account_identity_source,
            AccountIdentitySource::MatchAccountId
        );
        assert_eq!(matched.openid.as_deref(), Some("2002"));
        assert_eq!(matched.account_display_name, "旧名字#1001");
        assert_eq!(
            from_openid.account_identity_key,
            matched.account_identity_key
        );
        assert_eq!(
            from_openid.account_identity_source,
            AccountIdentitySource::Openid
        );
        assert_eq!(from_openid.account_display_name, "新名字#2002");
        assert_eq!(
            from_source.account_identity_key,
            format!("source-{}", manual_source.id)
        );
        assert_eq!(
            from_source.account_identity_source,
            AccountIdentitySource::SourceDir
        );
        assert_eq!(from_source.openid, None);
        assert_eq!(from_source.account_display_name, "任意玩家名#9999");
        assert!(!from_source
            .account_identity_key
            .contains(&from_source.account_display_name));

        let json = serde_json::to_value(matched).expect("clip DTO should serialize");
        assert_eq!(json["accountIdentityKey"], "match-account-2002");
        assert_eq!(json["accountIdentitySource"], "match-account-id");
        assert_eq!(json["accountDisplayName"], "旧名字#1001");
        assert_eq!(json["openid"], "2002");
    }

    #[test]
    fn clip_query_attaches_only_its_own_events() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip_a = insert_test_clip_with_file_name(&connection, "clip-a.mp4");
        let clip_b = insert_test_clip_with_file_name(&connection, "clip-b.mp4");

        for clip in [&clip_a, &clip_b] {
            upsert_clip_metadata(
                &connection,
                ClipMetadataInput {
                    clip_id: clip.id,
                    metadata_status: "enriched",
                    json_path: None,
                    account_name: Some("Tester#1001"),
                    player_name: Some("Tester#1001"),
                    agent_name: Some("Jett"),
                    map_name: Some("Ascent"),
                    game_mode: Some("Competitive"),
                    scoreline: Some("13/7"),
                    kda: Some("21/14/1"),
                    extracted_text: None,
                    parse_error: None,
                },
            )
            .expect("clip metadata should seed");
            connection
                .execute(
                    "UPDATE clip_metadata SET match_id = 'shared-match' WHERE clip_id = ?1",
                    params![clip.id],
                )
                .expect("shared match id should seed");
        }
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET official_video_name = '六杀时刻',
                    official_video_type = '五杀时刻',
                    highlight_type = '10',
                    round_score = '1670',
                    metadata_source = 'wonderful_db'
                WHERE clip_id = ?1
                ",
                params![clip_a.id],
            )
            .expect("official metadata should seed");

        replace_clip_timeline(
            &connection,
            clip_a.id,
            &[],
            &[
                ClipEventInput {
                    segment_key: None,
                    event_key: "a-kill-early",
                    event_type: "kill",
                    video_time_ms: Some(6_000),
                    event_time: Some("2026-07-03 17:20:55.495"),
                    round_id: Some(2),
                    player_name: Some("Tester #1001"),
                    agent_name: Some("Jett"),
                    weapon_name: Some("Vandal"),
                    killer_name: Some("Tester"),
                    killed_name: Some("Enemy A"),
                    killer_is_me: true,
                    raw_json: None,
                },
                ClipEventInput {
                    segment_key: None,
                    event_key: "a-kill-unknown-time",
                    event_type: "kill",
                    video_time_ms: None,
                    event_time: None,
                    round_id: Some(2),
                    player_name: Some("Tester #1001"),
                    agent_name: Some("Jett"),
                    weapon_name: Some("Vandal"),
                    killer_name: Some("Tester"),
                    killed_name: Some("Enemy C"),
                    killer_is_me: true,
                    raw_json: None,
                },
                ClipEventInput {
                    segment_key: None,
                    event_key: "a-kill-same-time",
                    event_type: "kill",
                    video_time_ms: Some(6_000),
                    event_time: Some("2026-07-03 17:20:56.495"),
                    round_id: Some(2),
                    player_name: Some("Tester #1001"),
                    agent_name: Some("Jett"),
                    weapon_name: Some("Vandal"),
                    killer_name: Some("Tester"),
                    killed_name: Some("Enemy B"),
                    killer_is_me: true,
                    raw_json: None,
                },
            ],
        )
        .expect("clip A timeline should seed");
        replace_clip_timeline(
            &connection,
            clip_b.id,
            &[],
            &[ClipEventInput {
                segment_key: None,
                event_key: "b-kill",
                event_type: "kill",
                video_time_ms: Some(1_000),
                event_time: Some("2026-07-03 17:21:00.000"),
                round_id: Some(3),
                player_name: Some("Tester #1001"),
                agent_name: Some("Jett"),
                weapon_name: Some("Phantom"),
                killer_name: Some("Tester"),
                killed_name: Some("Enemy D"),
                killer_is_me: true,
                raw_json: None,
            }],
        )
        .expect("clip B timeline should seed");

        let (clips, list_event_queries) = trace_clip_event_queries(&connection, || {
            let mut no_clips = Vec::new();
            attach_clip_events(&connection, &mut no_clips)
                .expect("empty clip attachment should be a no-op");
            list_clips(&connection).expect("clips should list")
        });
        assert_eq!(
            list_event_queries.len(),
            1,
            "list should attach all requested clip events with one query: {list_event_queries:#?}"
        );
        let listed_a = clips
            .iter()
            .find(|clip| clip.id == clip_a.id)
            .expect("clip A should list");
        let listed_b = clips
            .iter()
            .find(|clip| clip.id == clip_b.id)
            .expect("clip B should list");

        assert_eq!(listed_a.match_id.as_deref(), Some("shared-match"));
        assert_eq!(listed_b.match_id.as_deref(), Some("shared-match"));
        assert_eq!(listed_a.event_count, 3);
        assert_eq!(listed_b.event_count, 1);
        assert_eq!(
            listed_a
                .clip_events
                .iter()
                .map(|event| event.event_key.as_str())
                .collect::<Vec<_>>(),
            vec!["a-kill-early", "a-kill-same-time", "a-kill-unknown-time"]
        );
        assert_eq!(listed_b.clip_events[0].event_key, "b-kill");
        assert_eq!(listed_a.official_video_name.as_deref(), Some("六杀时刻"));
        assert_eq!(listed_a.official_video_type.as_deref(), Some("五杀时刻"));
        assert_eq!(listed_a.highlight_type, Some(10));
        assert_eq!(listed_a.round_score, Some(1670));
        assert_eq!(listed_a.metadata_source.as_deref(), Some("wonderful_db"));

        let (found_b, find_event_queries) = trace_clip_event_queries(&connection, || {
            find_clip_by_id(&connection, clip_b.id).expect("clip B should reload")
        });
        assert_eq!(
            find_event_queries.len(),
            1,
            "find should read only its requested clip events once: {find_event_queries:#?}"
        );
        assert_eq!(found_b.event_count, 1);
        assert_eq!(found_b.clip_events[0].event_key, "b-kill");
    }

    #[test]
    fn list_clips_returns_account_and_match_metadata() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let inserted = insert_test_clip(&connection);

        upsert_clip_metadata(
            &connection,
            ClipMetadataInput {
                clip_id: inserted.id,
                metadata_status: "parsed",
                json_path: Some("D:/Clips/videoExportTmp/config-player.json"),
                account_name: Some("FixtureAlpha#0001"),
                player_name: Some("FixtureAlpha#0001"),
                agent_name: Some("芮娜"),
                map_name: Some("迷邃幽境"),
                game_mode: Some("竞技模式"),
                scoreline: Some("11/13"),
                kda: Some("36/17/6"),
                extracted_text: Some("玩家昵称 FixtureAlpha#0001 地图 迷邃幽境"),
                parse_error: None,
            },
        )
        .expect("metadata should upsert");
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET metadata_status = 'enriched',
                    match_id = 'match-a-001',
                    round_label = 'R03',
                    weapon_name = 'Vandal',
                    kill_count = 3
                WHERE clip_id = ?1
                ",
                params![inserted.id],
            )
            .expect("enriched metadata should update");
        connection
            .execute(
                "
                INSERT INTO matches (
                    game_id,
                    battle_id,
                    account_id,
                    player_name,
                    agent_name,
                    agent_avatar_url,
                    map_name,
                    map_id,
                    game_mode,
                    started_at,
                    source_log
                )
                VALUES (
                    'match-a-001',
                    'battle-a-001',
                    '1001',
                    'FixtureAlpha#0001',
                    '芮娜',
                    'https://assets.example/reyna.png',
                    '迷邃幽境',
                    '/Game/Maps/Plummet/Plummet',
                    '竞技模式',
                    '2026-06-28T08:00:00Z',
                    1
                )
                ",
                [],
            )
            .expect("match should insert");
        let match_row_id: i64 = connection
            .query_row(
                "SELECT id FROM matches WHERE game_id = 'match-a-001'",
                [],
                |row| row.get(0),
            )
            .expect("match id should be readable");
        connection
            .execute(
                "
                INSERT INTO match_stats (
                    match_id,
                    kills,
                    deaths,
                    assists,
                    combat_score,
                    rounds_won,
                    rounds_lost,
                    rounds_played,
                    has_won
                )
                VALUES (?1, 36, 17, 6, 287, 11, 13, 24, 0)
                ",
                params![match_row_id],
            )
            .expect("match stats should insert");
        for event_time in ["2026-06-28T08:01:00Z", "2026-06-28T08:02:00Z"] {
            connection
                .execute(
                    "
                    INSERT INTO match_events (
                        match_id,
                        event_type,
                        event_time,
                        round_id,
                        weapon_name,
                        killer_name,
                        killed_name
                    )
                    VALUES (?1, 'kill', ?2, 3, 'Vandal', 'FixtureAlpha#0001', 'Opponent#0001')
                    ",
                    params![match_row_id, event_time],
                )
                .expect("match event should insert");
        }

        let clips = list_clips(&connection).expect("clips should list");

        assert_eq!(clips[0].metadata_status, "enriched");
        assert_eq!(clips[0].match_id.as_deref(), Some("match-a-001"));
        assert_eq!(clips[0].match_account_id.as_deref(), Some("1001"));
        assert_eq!(clips[0].account_name.as_deref(), Some("FixtureAlpha#0001"));
        assert_eq!(clips[0].player_name.as_deref(), Some("FixtureAlpha#0001"));
        assert_eq!(clips[0].agent_name.as_deref(), Some("芮娜"));
        assert_eq!(
            clips[0].agent_avatar_url.as_deref(),
            Some("https://assets.example/reyna.png")
        );
        assert_eq!(clips[0].map_name.as_deref(), Some("天枢云阙"));
        assert_eq!(clips[0].game_mode.as_deref(), Some("竞技模式"));
        assert_eq!(clips[0].scoreline.as_deref(), Some("11/13"));
        assert_eq!(clips[0].kda.as_deref(), Some("36/17/6"));
        assert_eq!(clips[0].round_label.as_deref(), Some("R03"));
        assert_eq!(clips[0].weapon_name.as_deref(), Some("Vandal"));
        assert_eq!(clips[0].kill_count, Some(3));
        assert_eq!(
            clips[0].match_started_at.as_deref(),
            Some("2026-06-28T08:00:00Z")
        );
        assert_eq!(clips[0].combat_score, Some(287));
        assert_eq!(clips[0].has_won, Some(false));
        assert_eq!(clips[0].event_count, 0);
        assert!(clips[0].clip_events.is_empty());
        let internal_match_event_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM match_events", [], |row| row.get(0))
            .expect("internal match event count should be readable");
        assert_eq!(internal_match_event_count, 2);
        assert_eq!(
            clips[0].extracted_text,
            "玩家昵称 FixtureAlpha#0001 地图 迷邃幽境"
        );
    }

    #[test]
    fn initialize_schema_starts_with_an_empty_custom_tag_catalog() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");

        initialize_schema(&connection).expect("schema should initialize");

        let tag_names = list_tags(&connection)
            .expect("tags should list")
            .into_iter()
            .map(|tag| tag.name)
            .collect::<Vec<_>>();

        assert!(tag_names.is_empty());
    }

    #[test]
    fn custom_tags_can_be_updated_and_deleted_with_clip_links_cascading() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip = insert_test_clip(&connection);
        let custom =
            create_tag(&connection, "  复盘  ", Some("green")).expect("custom tag should create");
        assign_tag_to_clip(&connection, clip.id, custom.id).expect("tag should assign");

        let updated = update_tag(&connection, custom.id, "精选复盘", Some("gold"))
            .expect("custom tag should update");
        assert_eq!(updated.name, "精选复盘");
        assert_eq!(updated.color.as_deref(), Some("gold"));

        delete_tag(&connection, custom.id).expect("custom tag should delete");
        let reloaded = find_clip_by_id(&connection, clip.id).expect("clip should reload");
        assert!(reloaded.tag_ids.is_empty());
        assert!(!list_tags(&connection)
            .expect("tags should list")
            .iter()
            .any(|tag| tag.id == custom.id));
    }

    #[test]
    fn video_type_like_names_are_ordinary_user_tags_after_migration() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let ace = create_tag(&connection, "ACE", Some("red")).expect("tag should create");

        let recolored = update_tag(&connection, ace.id, "王牌复盘", Some("teal"))
            .expect("user tag should update");
        assert_eq!(recolored.name, "王牌复盘");
        assert_eq!(recolored.color.as_deref(), Some("teal"));
        assert!(update_tag(&connection, ace.id, "王牌复盘", Some("purple")).is_err());
        delete_tag(&connection, ace.id).expect("user tag should delete");
    }

    #[test]
    fn update_clip_favorite_and_note_persist_to_clips_columns() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip = insert_test_clip(&connection);

        update_clip_favorite(&connection, clip.id, true).expect("favorite should update");
        update_clip_note(&connection, clip.id, Some("  1v3 clutch  ")).expect("note should update");

        let reloaded = find_clip_by_id(&connection, clip.id).expect("clip should reload");
        let stored_favorite: i64 = connection
            .query_row(
                "SELECT is_favorite FROM clips WHERE id = ?1",
                params![clip.id],
                |row| row.get(0),
            )
            .expect("is_favorite should be stored on clips");

        assert!(reloaded.favorite);
        assert_eq!(stored_favorite, 1);
        assert_eq!(reloaded.note.as_deref(), Some("1v3 clutch"));
    }

    #[test]
    fn favorite_batch_deduplicates_ids_and_reports_matches_and_missing_ids() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let first = insert_test_clip_with_file_name(&connection, "favorite-first.mp4");
        let second = insert_test_clip_with_file_name(&connection, "favorite-second.mp4");
        let missing_id = second.id + 10_000;

        let result = set_clips_favorite(
            &connection,
            &[first.id, first.id, missing_id, second.id],
            true,
        )
        .expect("favorite batch should commit");

        assert_eq!(result.requested, 3);
        assert_eq!(result.matched, 2);
        assert_eq!(result.updated, 2);
        assert_eq!(result.missing_ids, vec![missing_id]);
        assert_eq!(
            result.clips.iter().map(|clip| clip.id).collect::<Vec<_>>(),
            vec![first.id, second.id]
        );
        assert!(result.clips.iter().all(|clip| clip.favorite));

        let idempotent = set_clips_favorite(&connection, &[second.id, first.id], true)
            .expect("repeated favorite batch should be idempotent");
        assert_eq!(idempotent.updated, 0);
    }

    #[test]
    fn empty_batches_are_noops_without_opening_a_schema_transaction() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");

        for result in [
            set_clips_favorite(&connection, &[], true),
            set_clips_trashed(&connection, &[], true),
            add_tag_to_clips(&connection, &[], -1),
            remove_tag_from_clips(&connection, &[], -1),
        ] {
            assert_eq!(
                result.expect("empty batch should not touch the database"),
                empty_batch_clip_mutation_result()
            );
        }
    }

    #[test]
    fn tag_batches_add_and_remove_idempotently_and_reject_unknown_tags() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let first = insert_test_clip_with_file_name(&connection, "tag-first.mp4");
        let second = insert_test_clip_with_file_name(&connection, "tag-second.mp4");
        let review =
            create_tag(&connection, "批量复盘", Some("blue")).expect("batch tag should create");
        let missing_id = second.id + 10_000;

        let added = add_tag_to_clips(
            &connection,
            &[first.id, second.id, first.id, missing_id],
            review.id,
        )
        .expect("tag batch should add");
        assert_eq!(added.requested, 3);
        assert_eq!(added.matched, 2);
        assert_eq!(added.updated, 2);
        assert_eq!(added.missing_ids, vec![missing_id]);
        assert!(added
            .clips
            .iter()
            .all(|clip| clip.tag_ids.contains(&review.id)));

        let duplicate = add_tag_to_clips(&connection, &[first.id, second.id], review.id)
            .expect("duplicate bindings should be idempotent");
        assert_eq!(duplicate.updated, 0);

        let removed =
            remove_tag_from_clips(&connection, &[second.id, first.id, missing_id], review.id)
                .expect("tag batch should remove");
        assert_eq!(removed.updated, 2);
        assert_eq!(removed.missing_ids, vec![missing_id]);
        assert!(removed
            .clips
            .iter()
            .all(|clip| !clip.tag_ids.contains(&review.id)));

        let already_removed = remove_tag_from_clips(&connection, &[first.id], review.id)
            .expect("removing an absent binding should be idempotent");
        assert_eq!(already_removed.updated, 0);

        let invalid = add_tag_to_clips(&connection, &[first.id], review.id + 10_000)
            .expect_err("unknown tag ids must be rejected");
        assert!(invalid.contains("tag id"));
        assert!(invalid.contains("was not found"));
    }

    #[test]
    fn recycle_batch_changes_only_database_state_and_restores_from_file_presence() {
        let root = unique_database_temp_dir();
        fs::create_dir_all(&root).expect("recycle fixture root should be created");
        let clip_path = root.join("keep-original.mp4");
        fs::write(&clip_path, b"original video").expect("fixture video should be created");
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let source = upsert_source_dir(
            &connection,
            SourceDirInput {
                path: root.to_string_lossy().as_ref(),
                name: "Recycle fixture",
            },
        )
        .expect("source should upsert");
        let clip = upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source.id,
                clip_group_id: None,
                video_path: clip_path.to_string_lossy().as_ref(),
                file_name: "keep-original.mp4",
                file_size: 14,
                modified_at: None,
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert");

        let trashed = set_clips_trashed(&connection, &[clip.id], true)
            .expect("clip should enter recycle bin");
        assert_eq!(trashed.updated, 1);
        assert_eq!(trashed.clips[0].status, "trashed");
        assert_eq!(
            fs::read(&clip_path).expect("video should remain"),
            b"original video"
        );

        let restored =
            set_clips_trashed(&connection, &[clip.id], false).expect("clip should restore");
        assert_eq!(restored.updated, 1);
        assert_eq!(restored.clips[0].status, "available");
        assert_eq!(
            fs::read(&clip_path).expect("video should remain"),
            b"original video"
        );

        drop(connection);
        fs::remove_dir_all(root).expect("recycle fixture should be removed");
    }

    #[test]
    fn favorite_batch_rolls_back_all_rows_when_a_later_update_fails() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let first = insert_test_clip_with_file_name(&connection, "rollback-first.mp4");
        let second = insert_test_clip_with_file_name(&connection, "rollback-second.mp4");
        connection
            .execute_batch(&format!(
                "
                CREATE TRIGGER fail_favorite_batch
                BEFORE UPDATE OF is_favorite ON clips
                WHEN NEW.id = {}
                BEGIN
                    SELECT RAISE(ABORT, 'forced favorite batch failure');
                END;
                ",
                second.id
            ))
            .expect("rollback trigger should install");

        let error = set_clips_favorite(&connection, &[first.id, second.id], true)
            .expect_err("second update should abort the batch");
        assert!(error.contains("forced favorite batch failure"));

        let favorites = [first.id, second.id]
            .into_iter()
            .map(|clip_id| {
                connection
                    .query_row(
                        "SELECT is_favorite FROM clips WHERE id = ?1",
                        params![clip_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("favorite state should be readable")
            })
            .collect::<Vec<_>>();
        assert_eq!(favorites, vec![0, 0]);
    }

    #[test]
    fn assigning_and_removing_clip_tags_is_reflected_in_clip_list() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        initialize_schema(&connection).expect("schema should initialize");
        let clip = insert_test_clip(&connection);
        let ace = create_tag(&connection, "ACE", Some("red")).expect("tag should upsert");
        let review = create_tag(&connection, "复盘", Some("blue")).expect("tag should create");

        assign_tag_to_clip(&connection, clip.id, ace.id).expect("ACE should assign");
        assign_tag_to_clip(&connection, clip.id, review.id).expect("review should assign");

        let tagged_clip = list_clips(&connection)
            .expect("clips should list")
            .into_iter()
            .find(|candidate| candidate.id == clip.id)
            .expect("clip should exist");
        assert_eq!(tagged_clip.tag_ids, vec![ace.id, review.id]);

        remove_tag_from_clip(&connection, clip.id, ace.id).expect("ACE should remove");

        let reloaded_clip = find_clip_by_id(&connection, clip.id).expect("clip should reload");
        assert_eq!(reloaded_clip.tag_ids, vec![review.id]);
    }

    fn create_v3_clip_metadata_schema(connection: &Connection) {
        connection
            .execute_batch(
                "
                CREATE TABLE clip_metadata (
                    clip_id INTEGER PRIMARY KEY,
                    metadata_status TEXT NOT NULL DEFAULT 'not_found',
                    json_path TEXT,
                    account_name TEXT,
                    player_name TEXT,
                    agent_name TEXT,
                    map_name TEXT,
                    game_mode TEXT,
                    kda TEXT,
                    extracted_text TEXT,
                    parse_error TEXT,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                PRAGMA user_version = 3;
                ",
            )
            .expect("legacy clip_metadata schema should be created");
    }

    fn create_v4_database_with_user_state(connection: &Connection) {
        connection
            .execute_batch(
                "
                CREATE TABLE source_dirs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    path TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
                    status TEXT NOT NULL DEFAULT 'available',
                    last_error TEXT,
                    last_scanned_at TEXT,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE TABLE clips (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source_dir_id INTEGER NOT NULL,
                    clip_group_id INTEGER,
                    file_path TEXT NOT NULL UNIQUE,
                    normalized_path TEXT NOT NULL UNIQUE,
                    file_name TEXT NOT NULL,
                    extension TEXT NOT NULL DEFAULT 'mp4',
                    size_bytes INTEGER NOT NULL DEFAULT 0,
                    modified_at TEXT,
                    duration_ms INTEGER,
                    recorded_at TEXT,
                    cover_path TEXT,
                    cover_source TEXT NOT NULL DEFAULT 'missing',
                    file_status TEXT NOT NULL DEFAULT 'available',
                    is_favorite INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
                    note TEXT,
                    first_indexed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE TABLE clip_metadata (
                    clip_id INTEGER PRIMARY KEY,
                    metadata_status TEXT NOT NULL DEFAULT 'not_found',
                    json_path TEXT,
                    account_name TEXT,
                    player_name TEXT,
                    agent_name TEXT,
                    map_name TEXT,
                    game_mode TEXT,
                    match_id TEXT,
                    round_label TEXT,
                    scoreline TEXT,
                    kda TEXT,
                    weapon_name TEXT,
                    kill_count INTEGER,
                    raw_title TEXT,
                    extracted_text TEXT,
                    extra_json TEXT,
                    parse_error TEXT,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE TABLE tags (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    color TEXT,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE TABLE clip_tags (
                    clip_id INTEGER NOT NULL,
                    tag_id INTEGER NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (clip_id, tag_id)
                );

                INSERT INTO source_dirs (id, path, name)
                VALUES (7, 'D:/Clips/Valorant', 'Valorant');
                INSERT INTO clips (
                    id,
                    source_dir_id,
                    file_path,
                    normalized_path,
                    file_name,
                    size_bytes,
                    is_favorite,
                    note
                )
                VALUES (
                    42,
                    7,
                    'D:/Clips/Valorant/ace.mp4',
                    'd:/clips/valorant/ace.mp4',
                    'ace.mp4',
                    1024,
                    1,
                    'keep this note'
                );
                INSERT INTO clip_metadata (clip_id, metadata_status, json_path)
                VALUES (42, 'parsed', 'D:/Clips/Valorant/videoExportTmp/config-ace.json');
                INSERT INTO tags (id, name, color)
                VALUES (77, '复盘', 'blue');
                INSERT INTO clip_tags (clip_id, tag_id)
                VALUES (42, 77);

                PRAGMA user_version = 4;
                ",
            )
            .expect("version 4 database with user state should be created");
    }

    fn schema_user_version(connection: &Connection) -> i64 {
        connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version should be readable")
    }

    fn assert_table_exists(connection: &Connection, table_name: &str) {
        let exists: i64 = connection
            .query_row(
                "
                SELECT COUNT(*)
                FROM sqlite_master
                WHERE type = 'table'
                  AND name = ?1
                ",
                params![table_name],
                |row| row.get(0),
            )
            .expect("table count should be readable");

        assert_eq!(exists, 1, "table {table_name} should exist");
    }

    fn assert_index_exists(connection: &Connection, index_name: &str) {
        let exists: i64 = connection
            .query_row(
                "
                SELECT COUNT(*)
                FROM sqlite_master
                WHERE type = 'index'
                  AND name = ?1
                ",
                params![index_name],
                |row| row.get(0),
            )
            .expect("index count should be readable");
        assert_eq!(exists, 1, "index {index_name} should exist");
    }

    fn table_columns(connection: &Connection, table_name: &str) -> Vec<String> {
        let sql = format!("PRAGMA table_info({table_name})");
        let mut statement = connection.prepare(&sql).expect("table info should prepare");

        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("table info should query")
            .collect::<Result<Vec<_>, _>>()
            .expect("table columns should collect")
    }

    fn trace_clip_event_queries<T>(
        connection: &Connection,
        action: impl FnOnce() -> T,
    ) -> (T, Vec<String>) {
        let (result, statements) = trace_sql_queries(connection, action);
        let clip_event_queries = statements
            .into_iter()
            .filter(|sql| sql.contains("FROM clip_events"))
            .collect();

        (result, clip_event_queries)
    }

    fn trace_sql_queries<T>(
        connection: &Connection,
        action: impl FnOnce() -> T,
    ) -> (T, Vec<String>) {
        unsafe extern "C" fn trace_callback(context: *mut c_void, sql: *const c_char) {
            let statements = unsafe { &*context.cast::<Mutex<Vec<String>>>() };
            let sql = unsafe { CStr::from_ptr(sql) }
                .to_string_lossy()
                .into_owned();
            statements
                .lock()
                .expect("SQL trace lock should not be poisoned")
                .push(sql);
        }

        let statements = Box::new(Mutex::new(Vec::<String>::new()));
        let trace_context = Box::into_raw(statements);
        unsafe {
            rusqlite::ffi::sqlite3_trace(
                connection.handle(),
                Some(trace_callback),
                trace_context.cast(),
            );
        }

        let result = action();

        unsafe {
            rusqlite::ffi::sqlite3_trace(connection.handle(), None, ptr::null_mut());
        }
        let statements = unsafe { Box::from_raw(trace_context) }
            .into_inner()
            .expect("SQL trace lock should not be poisoned");

        (result, statements)
    }

    fn page_ids(page: &ClipPage) -> Vec<i64> {
        page.items.iter().map(|item| item.id).collect()
    }

    fn page_ids_for_query(connection: &Connection, query: ClipListQuery) -> Vec<i64> {
        page_ids(&list_clip_page(connection, &query).expect("clip page should list"))
    }

    struct LibraryFacetClipInput<'a> {
        source_dir_id: i64,
        file_name: &'a str,
        modified_at: i64,
        recorded_at: i64,
        file_size: i64,
        file_status: &'a str,
        favorite: bool,
        metadata_status: Option<&'a str>,
        account_name: Option<&'a str>,
        agent_name: Option<&'a str>,
        map_name: Option<&'a str>,
        game_mode: Option<&'a str>,
        match_id: Option<&'a str>,
        match_account_id: Option<&'a str>,
        kill_count: Option<i64>,
        highlight_type: Option<i64>,
    }

    fn insert_library_facet_clip(connection: &Connection, input: LibraryFacetClipInput<'_>) -> i64 {
        let video_path = format!(
            "D:\\LibraryFacetFixture\\{}\\{}",
            input.source_dir_id, input.file_name
        );
        let modified_at = input.modified_at.to_string();
        let recorded_at = input.recorded_at.to_string();
        let clip = upsert_clip(
            connection,
            ClipInput {
                source_dir_id: input.source_dir_id,
                clip_group_id: None,
                video_path: &video_path,
                file_name: input.file_name,
                file_size: input.file_size,
                modified_at: Some(&modified_at),
                duration_ms: Some(1_000),
                recorded_at: Some(&recorded_at),
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("facet fixture clip should upsert");
        if input.file_status == "trashed" {
            seed_test_trash_snapshot(connection, clip.id);
        }
        connection
            .execute(
                "UPDATE clips SET file_status = ?2, is_favorite = ?3 WHERE id = ?1",
                params![clip.id, input.file_status, i64::from(input.favorite)],
            )
            .expect("facet clip state should update");

        if let Some(metadata_status) = input.metadata_status {
            upsert_clip_metadata(
                connection,
                ClipMetadataInput {
                    clip_id: clip.id,
                    metadata_status,
                    json_path: None,
                    account_name: input.account_name,
                    player_name: input.account_name,
                    agent_name: input.agent_name,
                    map_name: input.map_name,
                    game_mode: input.game_mode,
                    scoreline: None,
                    kda: None,
                    extracted_text: None,
                    parse_error: None,
                },
            )
            .expect("facet metadata should upsert");
            connection
                .execute(
                    "
                    UPDATE clip_metadata
                    SET match_id = ?2,
                        kill_count = ?3,
                        highlight_type = ?4,
                        metadata_source = 'wonderful_db'
                    WHERE clip_id = ?1
                    ",
                    params![
                        clip.id,
                        input.match_id,
                        input.kill_count,
                        input.highlight_type
                    ],
                )
                .expect("facet metadata dimensions should update");
        }
        if let Some(match_id) = input.match_id {
            connection
                .execute(
                    "
                    INSERT INTO matches (game_id, account_id)
                    VALUES (?1, ?2)
                    ON CONFLICT(game_id) DO UPDATE SET account_id = excluded.account_id
                    ",
                    params![match_id, input.match_account_id],
                )
                .expect("facet match identity should upsert");
        }

        clip.id
    }

    fn seed_test_trash_snapshot(connection: &Connection, clip_id: i64) {
        connection
            .execute(
                "
                INSERT INTO clip_trash_snapshots (
                    clip_id,
                    video_path,
                    canonical_video_path,
                    source_dir_path,
                    canonical_source_dir_path,
                    extension,
                    file_existed
                )
                SELECT
                    clips.id,
                    clips.file_path,
                    clips.file_path,
                    source_dirs.path,
                    source_dirs.path,
                    clips.extension,
                    0
                FROM clips
                JOIN source_dirs ON source_dirs.id = clips.source_dir_id
                WHERE clips.id = ?1
                ",
                [clip_id],
            )
            .expect("trashed test fixture should have an authorization snapshot");
    }

    fn assert_facet_count(
        facets: &[LibraryFacetValue],
        value: &str,
        count: i64,
        active_count: i64,
    ) {
        let facet = facets
            .iter()
            .find(|facet| facet.value == value)
            .unwrap_or_else(|| panic!("facet {value:?} should exist in {facets:#?}"));
        assert_eq!(
            (facet.count, facet.active_count),
            (count, active_count),
            "unexpected counts for facet {value:?}"
        );
    }

    fn library_facet_item_count(facets: &LibraryFacets) -> usize {
        facets.file_statuses.len()
            + facets.metadata_statuses.len()
            + facets.accounts.len()
            + facets.source_dirs.len()
            + facets.agents.len()
            + facets.maps.len()
            + facets.game_modes.len()
            + facets.kill_types.len()
            + facets.tags.len()
    }

    fn insert_page_fixture_clip(
        connection: &Connection,
        file_name: &str,
        modified_at: i64,
        file_size: i64,
    ) -> i64 {
        let source = upsert_source_dir(
            connection,
            SourceDirInput {
                path: "D:\\PagedFixture",
                name: "Paged Fixture",
            },
        )
        .expect("page fixture source should upsert");
        insert_page_fixture_clip_for_source(
            connection,
            source.id,
            file_name,
            modified_at,
            file_size,
        )
    }

    fn insert_page_fixture_clip_for_source(
        connection: &Connection,
        source_dir_id: i64,
        file_name: &str,
        modified_at: i64,
        file_size: i64,
    ) -> i64 {
        let video_path = format!("D:\\PagedFixture\\{source_dir_id}\\{file_name}");
        let modified_at = modified_at.to_string();
        upsert_clip(
            connection,
            ClipInput {
                source_dir_id,
                clip_group_id: None,
                video_path: &video_path,
                file_name,
                file_size,
                modified_at: Some(&modified_at),
                duration_ms: Some(1_000),
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("page fixture clip should upsert")
        .id
    }

    fn seed_large_clip_fixture(connection: &Connection, clip_count: usize) -> Vec<i64> {
        let source = upsert_source_dir(
            connection,
            SourceDirInput {
                path: "D:\\BoundedPageFixture",
                name: "Bounded Page Fixture",
            },
        )
        .expect("large fixture source should upsert");
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .expect("large fixture transaction should begin");
        let mut clip_ids = Vec::with_capacity(clip_count);
        {
            let mut statement = connection
                .prepare(
                    "
                    INSERT INTO clips (
                        source_dir_id,
                        file_path,
                        normalized_path,
                        file_name,
                        extension,
                        size_bytes,
                        modified_at,
                        duration_ms,
                        cover_source,
                        file_status
                    )
                    VALUES (?1, ?2, ?3, ?4, 'mp4', ?5, '1800000000', 1000, 'missing', 'available')
                    ",
                )
                .expect("large fixture insert should prepare");
            for index in 0..clip_count {
                let file_name = format!("clip-{index:05}.mp4");
                let video_path = format!("D:\\BoundedPageFixture\\{file_name}");
                statement
                    .execute(params![
                        source.id,
                        video_path,
                        normalize_path(&video_path),
                        file_name,
                        index as i64 + 1
                    ])
                    .expect("large fixture clip should insert");
                clip_ids.push(connection.last_insert_rowid());
            }
        }
        connection
            .execute_batch("COMMIT")
            .expect("large fixture transaction should commit");
        clip_ids
    }

    fn insert_test_clip(connection: &Connection) -> Clip {
        let source_dir = upsert_source_dir(
            connection,
            SourceDirInput {
                path: "D:\\Clips\\Valorant",
                name: "Valorant",
            },
        )
        .expect("source dir should upsert");

        upsert_clip(
            connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: None,
                video_path: "D:\\Clips\\Valorant\\test.mp4",
                file_name: "test.mp4",
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: Some(12_000),
                recorded_at: Some("2026-06-28T10:11:12Z"),
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert")
    }

    fn insert_test_clip_with_file_name(connection: &Connection, file_name: &str) -> Clip {
        let source_dir = upsert_source_dir(
            connection,
            SourceDirInput {
                path: "D:\\Clips\\Valorant",
                name: "Valorant",
            },
        )
        .expect("source dir should upsert");
        let video_path = format!("D:\\Clips\\Valorant\\{file_name}");

        upsert_clip(
            connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: None,
                video_path: &video_path,
                file_name,
                file_size: 42,
                modified_at: Some("1782634272"),
                duration_ms: Some(12_000),
                recorded_at: Some("2026-06-28T10:11:12Z"),
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should upsert")
    }

    fn unique_database_temp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("vhm-database-test-{unique}"))
    }
}
