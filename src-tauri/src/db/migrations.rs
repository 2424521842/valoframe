//! Database schema creation and versioned migrations.

use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::{configure_connection, readable_error, DbResult};

pub(super) const SCHEMA_VERSION: i64 = 13;

/// Applies the idempotent schema migration to a caller-controlled connection.
pub fn initialize_schema(connection: &Connection) -> DbResult<()> {
    configure_connection(connection)?;
    let previous_schema_version = read_schema_user_version(connection)?;

    if previous_schema_version > SCHEMA_VERSION {
        return Err(format!(
            "database schema version {previous_schema_version} is newer than the supported version {SCHEMA_VERSION}; refusing to open it"
        ));
    }

    // journal_mode cannot be changed from inside the migration transaction. Keep the durable
    // schema changes and user_version update below in one IMMEDIATE transaction so a failed
    // migration can never leave a partially-upgraded database behind.
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| readable_error("enabling WAL journal mode", error))?;
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|error| readable_error("starting schema migration transaction", error))?;
    initialize_schema_versioned(&transaction, previous_schema_version)?;
    transaction
        .commit()
        .map_err(|error| readable_error("committing schema migration", error))
}

fn initialize_schema_versioned(
    connection: &Connection,
    previous_schema_version: i64,
) -> DbResult<()> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS source_dirs (
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

            CREATE TABLE IF NOT EXISTS clip_groups (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_dir_id INTEGER NOT NULL,
                group_key TEXT NOT NULL,
                display_name TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE (source_dir_id, group_key),
                FOREIGN KEY (source_dir_id) REFERENCES source_dirs(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS clips (
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
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (source_dir_id) REFERENCES source_dirs(id) ON DELETE CASCADE,
                FOREIGN KEY (clip_group_id) REFERENCES clip_groups(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS clip_thumbnails (
                clip_id INTEGER PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                cache_file TEXT,
                status TEXT NOT NULL CHECK (
                    status IN (
                        'pending',
                        'running',
                        'ready',
                        'failed',
                        'unavailable',
                        'suppressed',
                        'evicted'
                    )
                ),
                attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                next_attempt_at TEXT,
                error_code TEXT,
                last_error TEXT,
                byte_size INTEGER CHECK (byte_size IS NULL OR byte_size >= 0),
                revision TEXT,
                generated_at TEXT,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (clip_id) REFERENCES clips(id) ON DELETE CASCADE,
                CHECK (
                    cache_file IS NULL
                    OR (
                        LENGTH(TRIM(cache_file)) > 0
                        AND INSTR(cache_file, '/') = 0
                        AND INSTR(cache_file, CHAR(92)) = 0
                        AND cache_file NOT IN ('.', '..')
                    )
                ),
                CHECK (
                    status != 'ready'
                    OR (cache_file IS NOT NULL AND revision IS NOT NULL AND byte_size IS NOT NULL)
                )
            );

            CREATE TABLE IF NOT EXISTS clip_metadata (
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
                official_video_id TEXT,
                official_video_name TEXT,
                official_video_type TEXT,
                highlight_type TEXT,
                round_score TEXT,
                round_score_source TEXT,
                metadata_source TEXT,
                raw_title TEXT,
                extracted_text TEXT,
                extra_json TEXT,
                parse_error TEXT,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (clip_id) REFERENCES clips(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS matches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                game_id TEXT UNIQUE,
                battle_id TEXT,
                account_id TEXT,
                player_name TEXT,
                agent_name TEXT,
                agent_id TEXT,
                agent_avatar_url TEXT,
                map_id TEXT,
                map_name TEXT,
                game_mode TEXT,
                started_at TEXT,
                ended_at TEXT,
                source_leveldb INTEGER NOT NULL DEFAULT 0 CHECK (source_leveldb IN (0, 1)),
                source_log INTEGER NOT NULL DEFAULT 0 CHECK (source_log IN (0, 1)),
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS match_stats (
                match_id INTEGER PRIMARY KEY REFERENCES matches(id) ON DELETE CASCADE,
                kills INTEGER,
                deaths INTEGER,
                assists INTEGER,
                headshots INTEGER,
                combat_score INTEGER,
                rounds_won INTEGER,
                rounds_lost INTEGER,
                rounds_played INTEGER,
                has_won INTEGER CHECK (has_won IN (0, 1)),
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS match_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                match_id INTEGER REFERENCES matches(id) ON DELETE SET NULL,
                snapshot_id TEXT NOT NULL UNIQUE,
                account_id TEXT NOT NULL,
                captured_at TEXT,
                account_name TEXT,
                package_path TEXT,
                thumb_path TEXT,
                width INTEGER,
                height INTEGER,
                size_bytes INTEGER,
                raw_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS match_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
                event_type TEXT NOT NULL,
                event_time TEXT,
                round_id INTEGER,
                weapon_name TEXT,
                killer_name TEXT,
                killed_name TEXT,
                raw_json TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS clip_segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
                segment_key TEXT NOT NULL,
                round_id INTEGER,
                start_ms INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                game_start_ms INTEGER,
                game_end_ms INTEGER,
                UNIQUE (clip_id, segment_key)
            );

            CREATE TABLE IF NOT EXISTS clip_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
                segment_id INTEGER REFERENCES clip_segments(id) ON DELETE SET NULL,
                event_key TEXT NOT NULL,
                event_type TEXT NOT NULL,
                video_time_ms INTEGER,
                event_time TEXT,
                round_id INTEGER,
                player_name TEXT,
                agent_name TEXT,
                weapon_name TEXT,
                killer_name TEXT,
                killed_name TEXT,
                killer_is_me INTEGER NOT NULL DEFAULT 0 CHECK (killer_is_me IN (0, 1)),
                raw_json TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE (clip_id, event_key)
            );

            CREATE TRIGGER IF NOT EXISTS validate_clip_event_segment_on_insert
            BEFORE INSERT ON clip_events
            WHEN NEW.segment_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1
                    FROM clip_segments
                    WHERE id = NEW.segment_id
                      AND clip_id = NEW.clip_id
                )
            BEGIN
                SELECT RAISE(ABORT, 'clip event segment must belong to the same clip');
            END;

            CREATE TRIGGER IF NOT EXISTS validate_clip_event_segment_on_update
            BEFORE UPDATE OF clip_id, segment_id ON clip_events
            WHEN NEW.segment_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1
                    FROM clip_segments
                    WHERE id = NEW.segment_id
                      AND clip_id = NEW.clip_id
                )
            BEGIN
                SELECT RAISE(ABORT, 'clip event segment must belong to the same clip');
            END;

            CREATE TRIGGER IF NOT EXISTS validate_referenced_clip_segment_on_update
            BEFORE UPDATE OF clip_id ON clip_segments
            WHEN EXISTS (
                SELECT 1
                FROM clip_events
                WHERE segment_id = OLD.id
                  AND clip_id != NEW.clip_id
            )
            BEGIN
                SELECT RAISE(ABORT, 'referenced clip segment cannot move to another clip');
            END;

            CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                color TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS clip_tags (
                clip_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (clip_id, tag_id),
                FOREIGN KEY (clip_id) REFERENCES clips(id) ON DELETE CASCADE,
                FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS scan_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT,
                root_path TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'completed',
                source_dir_count INTEGER NOT NULL DEFAULT 0,
                clip_group_count INTEGER NOT NULL DEFAULT 0,
                new_clip_count INTEGER NOT NULL DEFAULT 0,
                updated_clip_count INTEGER NOT NULL DEFAULT 0,
                missing_clip_count INTEGER NOT NULL DEFAULT 0,
                cover_missing_count INTEGER NOT NULL DEFAULT 0,
                metadata_match_count INTEGER NOT NULL DEFAULT 0,
                metadata_enriched_clip_count INTEGER NOT NULL DEFAULT 0,
                metadata_event_count INTEGER NOT NULL DEFAULT 0,
                metadata_warning_count INTEGER NOT NULL DEFAULT 0,
                diagnostic_omitted_count INTEGER NOT NULL DEFAULT 0,
                errors_json TEXT NOT NULL DEFAULT '[]',
                message TEXT,
                started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                finished_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS clip_trash_snapshots (
                clip_id INTEGER PRIMARY KEY,
                video_path TEXT NOT NULL,
                canonical_video_path TEXT NOT NULL,
                source_dir_path TEXT NOT NULL,
                canonical_source_dir_path TEXT NOT NULL,
                extension TEXT NOT NULL,
                file_existed INTEGER NOT NULL CHECK (file_existed IN (0, 1)),
                file_size_bytes INTEGER,
                file_modified_ticks INTEGER,
                file_volume_serial INTEGER CHECK (
                    file_volume_serial IS NULL OR file_volume_serial BETWEEN 0 AND 4294967295
                ),
                file_index_high INTEGER CHECK (
                    file_index_high IS NULL OR file_index_high BETWEEN 0 AND 4294967295
                ),
                file_index_low INTEGER CHECK (
                    file_index_low IS NULL OR file_index_low BETWEEN 0 AND 4294967295
                ),
                source_volume_serial INTEGER CHECK (
                    source_volume_serial IS NULL OR source_volume_serial BETWEEN 0 AND 4294967295
                ),
                source_file_index_high INTEGER CHECK (
                    source_file_index_high IS NULL OR source_file_index_high BETWEEN 0 AND 4294967295
                ),
                source_file_index_low INTEGER CHECK (
                    source_file_index_low IS NULL OR source_file_index_low BETWEEN 0 AND 4294967295
                ),
                captured_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (clip_id) REFERENCES clips(id) ON DELETE CASCADE,
                CHECK (
                    (file_existed = 0
                        AND file_size_bytes IS NULL
                        AND file_modified_ticks IS NULL
                        AND file_volume_serial IS NULL
                        AND file_index_high IS NULL
                        AND file_index_low IS NULL)
                    OR
                    (file_existed = 1
                        AND file_size_bytes IS NOT NULL
                        AND file_modified_ticks IS NOT NULL)
                ),
                CHECK (
                    (file_volume_serial IS NULL
                        AND file_index_high IS NULL
                        AND file_index_low IS NULL)
                    OR
                    (file_volume_serial IS NOT NULL
                        AND file_index_high IS NOT NULL
                        AND file_index_low IS NOT NULL)
                ),
                CHECK (
                    (source_volume_serial IS NULL
                        AND source_file_index_high IS NULL
                        AND source_file_index_low IS NULL)
                    OR
                    (source_volume_serial IS NOT NULL
                        AND source_file_index_high IS NOT NULL
                        AND source_file_index_low IS NOT NULL)
                )
            );

            CREATE TRIGGER IF NOT EXISTS prevent_clip_trash_snapshot_update
            BEFORE UPDATE ON clip_trash_snapshots
            BEGIN
                SELECT RAISE(ABORT, 'clip trash snapshots are immutable');
            END;

            CREATE TRIGGER IF NOT EXISTS require_clip_trash_snapshot
            BEFORE UPDATE OF file_status ON clips
            WHEN NEW.file_status = 'trashed'
                AND OLD.file_status <> 'trashed'
                AND NOT EXISTS (
                    SELECT 1
                    FROM clip_trash_snapshots
                    WHERE clip_id = NEW.id
                )
            BEGIN
                SELECT RAISE(ABORT, 'moving a clip to trash requires an identity snapshot');
            END;

            CREATE TABLE IF NOT EXISTS clip_delete_intents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                clip_id INTEGER NOT NULL UNIQUE,
                state TEXT NOT NULL DEFAULT 'pending' CHECK (
                    state IN ('pending', 'processing', 'blocked')
                ),
                video_path TEXT NOT NULL,
                canonical_video_path TEXT,
                source_dir_path TEXT NOT NULL,
                canonical_source_dir_path TEXT,
                extension TEXT NOT NULL,
                file_existed INTEGER NOT NULL CHECK (file_existed IN (0, 1)),
                file_size_bytes INTEGER,
                file_modified_ticks INTEGER,
                file_volume_serial INTEGER CHECK (
                    file_volume_serial IS NULL OR file_volume_serial BETWEEN 0 AND 4294967295
                ),
                file_index_high INTEGER CHECK (
                    file_index_high IS NULL OR file_index_high BETWEEN 0 AND 4294967295
                ),
                file_index_low INTEGER CHECK (
                    file_index_low IS NULL OR file_index_low BETWEEN 0 AND 4294967295
                ),
                source_volume_serial INTEGER CHECK (
                    source_volume_serial IS NULL OR source_volume_serial BETWEEN 0 AND 4294967295
                ),
                source_file_index_high INTEGER CHECK (
                    source_file_index_high IS NULL OR source_file_index_high BETWEEN 0 AND 4294967295
                ),
                source_file_index_low INTEGER CHECK (
                    source_file_index_low IS NULL OR source_file_index_low BETWEEN 0 AND 4294967295
                ),
                attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                lease_owner TEXT,
                lease_expires_at TEXT,
                last_attempt_at TEXT,
                last_error_code TEXT,
                last_error_message TEXT,
                requested_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (clip_id) REFERENCES clips(id) ON DELETE RESTRICT
            );

            CREATE INDEX IF NOT EXISTS idx_source_dirs_enabled
                ON source_dirs(enabled);
            CREATE INDEX IF NOT EXISTS idx_source_dirs_status
                ON source_dirs(status);
            CREATE INDEX IF NOT EXISTS idx_clip_groups_source_dir_id
                ON clip_groups(source_dir_id);
            CREATE INDEX IF NOT EXISTS idx_clips_source_dir_id
                ON clips(source_dir_id);
            CREATE INDEX IF NOT EXISTS idx_clips_clip_group_id
                ON clips(clip_group_id);
            CREATE INDEX IF NOT EXISTS idx_clips_file_status
                ON clips(file_status);
            CREATE INDEX IF NOT EXISTS idx_clips_modified_at
                ON clips(modified_at DESC);
            CREATE INDEX IF NOT EXISTS idx_clips_cover_source
                ON clips(cover_source);
            CREATE INDEX IF NOT EXISTS idx_clips_recorded_at
                ON clips(recorded_at DESC);
            CREATE INDEX IF NOT EXISTS idx_clip_thumbnails_status_due
                ON clip_thumbnails(status, next_attempt_at, clip_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_clip_thumbnails_cache_file
                ON clip_thumbnails(cache_file)
                WHERE cache_file IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_clip_metadata_status
                ON clip_metadata(metadata_status);
            CREATE INDEX IF NOT EXISTS idx_clip_metadata_agent
                ON clip_metadata(agent_name);
            CREATE INDEX IF NOT EXISTS idx_clip_metadata_map
                ON clip_metadata(map_name);
            CREATE INDEX IF NOT EXISTS idx_matches_battle_id
                ON matches(battle_id);
            CREATE INDEX IF NOT EXISTS idx_matches_account_id
                ON matches(account_id);
            CREATE INDEX IF NOT EXISTS idx_matches_started_at
                ON matches(started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_match_events_match_id
                ON match_events(match_id);
            CREATE INDEX IF NOT EXISTS idx_match_events_event_time
                ON match_events(event_time);
            CREATE INDEX IF NOT EXISTS idx_match_snapshots_match_id
                ON match_snapshots(match_id);
            CREATE INDEX IF NOT EXISTS idx_match_snapshots_account_id
                ON match_snapshots(account_id, captured_at DESC);
            CREATE INDEX IF NOT EXISTS idx_clip_segments_clip_start
                ON clip_segments(clip_id, start_ms);
            CREATE INDEX IF NOT EXISTS idx_clip_events_clip_video_time
                ON clip_events(clip_id, video_time_ms, id);
            CREATE INDEX IF NOT EXISTS idx_tags_name
                ON tags(name);
            CREATE INDEX IF NOT EXISTS idx_clip_tags_tag_id
                ON clip_tags(tag_id);
            CREATE INDEX IF NOT EXISTS idx_scan_runs_finished_at
                ON scan_runs(finished_at DESC);
            CREATE INDEX IF NOT EXISTS idx_clip_delete_intents_state_lease
                ON clip_delete_intents(state, lease_expires_at, id);
            ",
        )
        .map_err(|error| readable_error("initializing schema", error))?;

    ensure_column(connection, "clips", "modified_at", "TEXT")?;
    ensure_column(connection, "clips", "cover_path", "TEXT")?;
    ensure_column(
        connection,
        "clips",
        "cover_source",
        "TEXT NOT NULL DEFAULT 'missing'",
    )?;
    ensure_column(connection, "clips", "note", "TEXT")?;
    ensure_column(
        connection,
        "clips",
        "is_favorite",
        "INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1))",
    )?;
    ensure_column(connection, "tags", "created_at", "TEXT")?;
    ensure_column(connection, "tags", "updated_at", "TEXT")?;
    connection
        .execute(
            "
            UPDATE tags
            SET created_at = COALESCE(created_at, CURRENT_TIMESTAMP),
                updated_at = COALESCE(updated_at, created_at, CURRENT_TIMESTAMP)
            WHERE created_at IS NULL
               OR updated_at IS NULL
            ",
            [],
        )
        .map_err(|error| readable_error("backfilling tag timestamps", error))?;
    ensure_column(connection, "clip_metadata", "extracted_text", "TEXT")?;
    ensure_column(
        connection,
        "clip_metadata",
        "metadata_status",
        "TEXT NOT NULL DEFAULT 'not_found'",
    )?;
    ensure_column(connection, "clip_metadata", "json_path", "TEXT")?;
    ensure_column(connection, "clip_metadata", "account_name", "TEXT")?;
    ensure_column(connection, "clip_metadata", "player_name", "TEXT")?;
    ensure_column(connection, "clip_metadata", "agent_name", "TEXT")?;
    ensure_column(connection, "clip_metadata", "map_name", "TEXT")?;
    ensure_column(connection, "clip_metadata", "game_mode", "TEXT")?;
    ensure_column(connection, "clip_metadata", "match_id", "TEXT")?;
    ensure_column(connection, "clip_metadata", "round_label", "TEXT")?;
    ensure_column(connection, "clip_metadata", "scoreline", "TEXT")?;
    ensure_column(connection, "clip_metadata", "kda", "TEXT")?;
    ensure_column(connection, "clip_metadata", "weapon_name", "TEXT")?;
    ensure_column(connection, "clip_metadata", "kill_count", "INTEGER")?;
    ensure_column(connection, "clip_metadata", "official_video_id", "TEXT")?;
    ensure_column(connection, "clip_metadata", "official_video_name", "TEXT")?;
    ensure_column(connection, "clip_metadata", "official_video_type", "TEXT")?;
    ensure_column(connection, "clip_metadata", "highlight_type", "TEXT")?;
    ensure_column(connection, "clip_metadata", "round_score", "TEXT")?;
    ensure_column(connection, "clip_metadata", "round_score_source", "TEXT")?;
    ensure_column(connection, "clip_metadata", "metadata_source", "TEXT")?;
    if previous_schema_version < 11 {
        connection
            .execute(
                "
                UPDATE clip_metadata
                SET round_score_source = 'wonderful_db'
                WHERE round_score_source IS NULL
                  AND NULLIF(TRIM(round_score), '') IS NOT NULL
                  AND metadata_source = 'wonderful_db'
                ",
                [],
            )
            .map_err(|error| {
                readable_error("backfilling official round score provenance", error)
            })?;
    }
    connection
        .execute(
            "
            UPDATE clip_metadata
            SET metadata_source = 'video_export'
            WHERE metadata_source IS NULL
              AND NULLIF(TRIM(json_path), '') IS NOT NULL
            ",
            [],
        )
        .map_err(|error| readable_error("backfilling video export metadata ownership", error))?;
    ensure_column(connection, "clip_metadata", "raw_title", "TEXT")?;
    ensure_column(connection, "clip_metadata", "extra_json", "TEXT")?;
    ensure_column(connection, "clip_metadata", "parse_error", "TEXT")?;
    remove_legacy_official_precedence_trigger(connection)?;
    ensure_column(connection, "scan_runs", "job_id", "TEXT")?;
    ensure_column(
        connection,
        "scan_runs",
        "metadata_match_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        connection,
        "scan_runs",
        "metadata_enriched_clip_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        connection,
        "scan_runs",
        "metadata_event_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        connection,
        "scan_runs",
        "metadata_warning_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        connection,
        "scan_runs",
        "diagnostic_omitted_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    migrate_legacy_favorite_column(connection)?;
    create_schema_indexes(connection)?;
    if previous_schema_version < 7 {
        migrate_clip_paging_indexes(connection)?;
    }
    if previous_schema_version < 8 {
        migrate_thumbnail_queue_schema(connection)?;
    }
    if previous_schema_version < 10 {
        migrate_video_types_out_of_tags(connection)?;
    }
    if previous_schema_version < 13 {
        // Version 12 captured filesystem identity only when permanent deletion was requested.
        // Those intents are not proof that the same file was authorized when the clip entered
        // the recycle bin, so fail closed: leave the clips trashed and discard the stale grants.
        // The user can restore and move them to trash again to create a v13 snapshot.
        connection
            .execute("DELETE FROM clip_delete_intents", [])
            .map_err(|error| readable_error("invalidating legacy delete intents", error))?;
    }
    if previous_schema_version < SCHEMA_VERSION {
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|error| readable_error("updating schema version", error))?;
    }

    Ok(())
}

fn ensure_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
    definition: &str,
) -> DbResult<()> {
    if column_exists(connection, table_name, column_name)? {
        return Ok(());
    }

    let sql = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {definition}");
    connection
        .execute(&sql, [])
        .map_err(|error| readable_error("adding schema column", error))?;

    Ok(())
}

fn column_exists(connection: &Connection, table_name: &str, column_name: &str) -> DbResult<bool> {
    let sql = format!("PRAGMA table_info({table_name})");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| readable_error("reading table info", error))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| readable_error("querying table info", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("collecting table info", error))?;

    Ok(columns.iter().any(|name| name == column_name))
}

fn migrate_legacy_favorite_column(connection: &Connection) -> DbResult<()> {
    if !column_exists(connection, "clips", "favorite")? {
        return Ok(());
    }

    connection
        .execute(
            "
            UPDATE clips
            SET is_favorite = favorite
            WHERE is_favorite = 0
              AND favorite != 0
            ",
            [],
        )
        .map_err(|error| readable_error("migrating favorite column", error))?;

    Ok(())
}

fn create_schema_indexes(connection: &Connection) -> DbResult<()> {
    connection
        .execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_clips_is_favorite
                ON clips(is_favorite);
            CREATE INDEX IF NOT EXISTS idx_clip_metadata_match_id
                ON clip_metadata(match_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_scan_runs_job_id
                ON scan_runs(job_id)
                WHERE job_id IS NOT NULL;
            ",
        )
        .map_err(|error| readable_error("creating schema indexes", error))
}

pub(super) fn read_schema_user_version(connection: &Connection) -> DbResult<i64> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| readable_error("reading schema version", error))
}

/// Version 7 adds only the composite indexes used by the bounded list protocol. This function is
/// intentionally reached from explicit schema migration, never from ordinary database opens.
fn migrate_clip_paging_indexes(connection: &Connection) -> DbResult<()> {
    connection
        .execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_clips_page_modified_id
                ON clips(
                    COALESCE(
                        CASE
                            WHEN NULLIF(TRIM(modified_at), '') IS NULL THEN NULL
                            WHEN TRIM(modified_at) NOT GLOB '*[^0-9]*'
                                THEN CAST(modified_at AS INTEGER)
                            ELSE unixepoch(modified_at)
                        END,
                        CASE
                            WHEN NULLIF(TRIM(recorded_at), '') IS NULL THEN NULL
                            WHEN TRIM(recorded_at) NOT GLOB '*[^0-9]*'
                                THEN CAST(recorded_at AS INTEGER)
                            ELSE unixepoch(recorded_at)
                        END,
                        0
                    ) DESC,
                    id DESC
                );
            CREATE INDEX IF NOT EXISTS idx_clips_page_size_id
                ON clips(size_bytes, id);
            CREATE INDEX IF NOT EXISTS idx_clips_page_name_id
                ON clips(file_name COLLATE VHM_CLIP_NAME, id);
            CREATE INDEX IF NOT EXISTS idx_clip_tags_tag_clip
                ON clip_tags(tag_id, clip_id);
            ",
        )
        .map_err(|error| readable_error("migrating clip paging indexes", error))
}

/// Version 8 persists thumbnail work separately from source-owned cover metadata. Generated cache
/// paths must never be written into `clips.cover_path`, because a later scan is authoritative for
/// that column and would overwrite them.
fn migrate_thumbnail_queue_schema(connection: &Connection) -> DbResult<()> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS clip_thumbnails (
                clip_id INTEGER PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                cache_file TEXT,
                status TEXT NOT NULL CHECK (
                    status IN (
                        'pending',
                        'running',
                        'ready',
                        'failed',
                        'unavailable',
                        'suppressed',
                        'evicted'
                    )
                ),
                attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                next_attempt_at TEXT,
                error_code TEXT,
                last_error TEXT,
                byte_size INTEGER CHECK (byte_size IS NULL OR byte_size >= 0),
                revision TEXT,
                generated_at TEXT,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (clip_id) REFERENCES clips(id) ON DELETE CASCADE,
                CHECK (
                    cache_file IS NULL
                    OR (
                        LENGTH(TRIM(cache_file)) > 0
                        AND INSTR(cache_file, '/') = 0
                        AND INSTR(cache_file, CHAR(92)) = 0
                        AND cache_file NOT IN ('.', '..')
                    )
                ),
                CHECK (
                    status != 'ready'
                    OR (cache_file IS NOT NULL AND revision IS NOT NULL AND byte_size IS NOT NULL)
                )
            );
            CREATE INDEX IF NOT EXISTS idx_clip_thumbnails_status_due
                ON clip_thumbnails(status, next_attempt_at, clip_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_clip_thumbnails_cache_file
                ON clip_thumbnails(cache_file)
                WHERE cache_file IS NOT NULL;
            ",
        )
        .map_err(|error| readable_error("migrating thumbnail queue schema", error))
}

fn remove_legacy_official_precedence_trigger(connection: &Connection) -> DbResult<()> {
    connection
        .execute_batch("DROP TRIGGER IF EXISTS preserve_wonderful_kill_count;")
        .map_err(|error| readable_error("removing legacy official metadata trigger", error))
}

fn migrate_video_types_out_of_tags(connection: &Connection) -> DbResult<()> {
    connection
        .execute_batch(
            "
            UPDATE clip_metadata
            SET highlight_type = '4',
                kill_count = 3,
                updated_at = CURRENT_TIMESTAMP
            WHERE COALESCE(metadata_source, '') <> 'wonderful_db'
              AND EXISTS (
                  SELECT 1
                  FROM clip_tags
                  JOIN tags ON tags.id = clip_tags.tag_id
                  WHERE clip_tags.clip_id = clip_metadata.clip_id
                    AND tags.name = '三杀'
              );

            UPDATE clip_metadata
            SET highlight_type = '6',
                kill_count = 4,
                updated_at = CURRENT_TIMESTAMP
            WHERE COALESCE(metadata_source, '') <> 'wonderful_db'
              AND EXISTS (
                  SELECT 1
                  FROM clip_tags
                  JOIN tags ON tags.id = clip_tags.tag_id
                  WHERE clip_tags.clip_id = clip_metadata.clip_id
                    AND tags.name = '四杀'
              );

            UPDATE clip_metadata
            SET highlight_type = '10',
                kill_count = 5,
                updated_at = CURRENT_TIMESTAMP
            WHERE COALESCE(metadata_source, '') <> 'wonderful_db'
              AND EXISTS (
                  SELECT 1
                  FROM clip_tags
                  JOIN tags ON tags.id = clip_tags.tag_id
                  WHERE clip_tags.clip_id = clip_metadata.clip_id
                    AND tags.name IN ('ACE', '五杀')
              );

            UPDATE clip_metadata
            SET highlight_type = '10',
                kill_count = 6,
                updated_at = CURRENT_TIMESTAMP
            WHERE COALESCE(metadata_source, '') <> 'wonderful_db'
              AND EXISTS (
                  SELECT 1
                  FROM clip_tags
                  JOIN tags ON tags.id = clip_tags.tag_id
                  WHERE clip_tags.clip_id = clip_metadata.clip_id
                    AND tags.name = '六杀'
              );

            UPDATE clip_metadata
            SET highlight_type = '2',
                kill_count = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE COALESCE(metadata_source, '') <> 'wonderful_db'
              AND EXISTS (
                  SELECT 1
                  FROM clip_tags
                  JOIN tags ON tags.id = clip_tags.tag_id
                  WHERE clip_tags.clip_id = clip_metadata.clip_id
                    AND tags.name IN ('击杀合集', '击杀集锦')
              );

            UPDATE clip_metadata
            SET highlight_type = '3',
                kill_count = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE COALESCE(metadata_source, '') <> 'wonderful_db'
              AND EXISTS (
                  SELECT 1
                  FROM clip_tags
                  JOIN tags ON tags.id = clip_tags.tag_id
                  WHERE clip_tags.clip_id = clip_metadata.clip_id
                    AND tags.name = '死亡集锦'
              );

            DELETE FROM clip_tags
            WHERE tag_id IN (
                SELECT id
                FROM tags
                WHERE name IN (
                    'ACE',
                    '三杀',
                    '四杀',
                    '击杀合集',
                    '击杀集锦',
                    '死亡集锦',
                    '五杀',
                    '六杀',
                    'MVP',
                    '搞笑',
                    '待剪',
                    '教学',
                    '废片'
                )
            );

            DELETE FROM tags
            WHERE name IN (
                'ACE',
                '三杀',
                '四杀',
                '击杀合集',
                '击杀集锦',
                '死亡集锦',
                '五杀',
                '六杀',
                'MVP',
                '搞笑',
                '待剪',
                '教学',
                '废片'
            );
            ",
        )
        .map_err(|error| readable_error("migrating video types out of tags", error))
}
