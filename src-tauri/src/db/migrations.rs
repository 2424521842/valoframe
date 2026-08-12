//! Database schema creation and versioned migrations.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;

use super::{configure_connection, readable_error, DbResult};
use crate::metadata::{classify_timeline_event_time, TimelineEventTimeSemantics};

pub(super) const SCHEMA_VERSION: i64 = 16;

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
                source_kind TEXT NOT NULL DEFAULT 'aclos' CHECK (
                    source_kind IN ('aclos', 'nvidia', 'tracker', 'generic')
                ),
                scan_mode TEXT NOT NULL DEFAULT 'aclos-structured' CHECK (
                    scan_mode IN ('aclos-structured', 'recursive-mp4')
                ),
                scan_root_path TEXT NOT NULL,
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
                file_volume_serial INTEGER CHECK (
                    file_volume_serial IS NULL OR file_volume_serial BETWEEN 0 AND 4294967295
                ),
                file_index_high INTEGER CHECK (
                    file_index_high IS NULL OR file_index_high BETWEEN 0 AND 4294967295
                ),
                file_index_low INTEGER CHECK (
                    file_index_low IS NULL OR file_index_low BETWEEN 0 AND 4294967295
                ),
                duration_ms INTEGER,
                recorded_at TEXT,
                source_relative_dir TEXT NOT NULL DEFAULT '',
                cover_path TEXT,
                cover_source TEXT NOT NULL DEFAULT 'missing',
                file_status TEXT NOT NULL DEFAULT 'available',
                is_favorite INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
                review_decision TEXT NOT NULL DEFAULT 'unreviewed' CHECK (
                    review_decision IN ('unreviewed', 'liked', 'disliked')
                ),
                reviewed_at TEXT,
                note TEXT,
                first_indexed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (source_dir_id) REFERENCES source_dirs(id) ON DELETE CASCADE,
                FOREIGN KEY (clip_group_id) REFERENCES clip_groups(id) ON DELETE SET NULL,
                CHECK (
                    (file_volume_serial IS NULL
                        AND file_index_high IS NULL
                        AND file_index_low IS NULL)
                    OR
                    (file_volume_serial IS NOT NULL
                        AND file_index_high IS NOT NULL
                        AND file_index_low IS NOT NULL)
                )
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
                killed_is_me INTEGER NOT NULL DEFAULT 0 CHECK (killed_is_me IN (0, 1)),
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
                summary_available INTEGER NOT NULL DEFAULT 0 CHECK (summary_available IN (0, 1)),
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
    ensure_column(
        connection,
        "clips",
        "file_volume_serial",
        "INTEGER CHECK (file_volume_serial IS NULL OR file_volume_serial BETWEEN 0 AND 4294967295)",
    )?;
    ensure_column(
        connection,
        "clips",
        "file_index_high",
        "INTEGER CHECK (file_index_high IS NULL OR file_index_high BETWEEN 0 AND 4294967295)",
    )?;
    ensure_column(
        connection,
        "clips",
        "file_index_low",
        "INTEGER CHECK (file_index_low IS NULL OR file_index_low BETWEEN 0 AND 4294967295)",
    )?;
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
    ensure_column(
        connection,
        "source_dirs",
        "source_kind",
        "TEXT NOT NULL DEFAULT 'aclos' CHECK (source_kind IN ('aclos', 'nvidia', 'tracker', 'generic'))",
    )?;
    ensure_column(
        connection,
        "source_dirs",
        "scan_mode",
        "TEXT NOT NULL DEFAULT 'aclos-structured' CHECK (scan_mode IN ('aclos-structured', 'recursive-mp4'))",
    )?;
    ensure_column(
        connection,
        "source_dirs",
        "scan_root_path",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        connection,
        "clips",
        "source_relative_dir",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        connection,
        "clips",
        "review_decision",
        "TEXT NOT NULL DEFAULT 'unreviewed' CHECK (review_decision IN ('unreviewed', 'liked', 'disliked'))",
    )?;
    ensure_column(connection, "clips", "reviewed_at", "TEXT")?;
    ensure_column(
        connection,
        "clip_events",
        "killed_is_me",
        "INTEGER NOT NULL DEFAULT 0 CHECK (killed_is_me IN (0, 1))",
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
    ensure_column(
        connection,
        "scan_runs",
        "summary_available",
        "INTEGER NOT NULL DEFAULT 0 CHECK (summary_available IN (0, 1))",
    )?;
    migrate_legacy_favorite_column(connection)?;
    if previous_schema_version < 14 {
        migrate_source_and_review_model_v14(connection)?;
    }
    if previous_schema_version < 15 {
        migrate_scan_summary_availability_v15(connection)?;
        migrate_deterministic_clip_event_fields_v15(connection)?;
    }
    if previous_schema_version < 16 {
        migrate_windows_verbatim_clip_duplicates_v16(connection)?;
        migrate_windows_verbatim_source_paths_v16(connection)?;
    }
    create_clip_identity_validation_triggers(connection)?;
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

fn migrate_source_and_review_model_v14(connection: &Connection) -> DbResult<()> {
    connection
        .execute(
            "
            UPDATE source_dirs
            SET source_kind = 'aclos',
                scan_mode = 'aclos-structured',
                scan_root_path = path
            ",
            [],
        )
        .map_err(|error| readable_error("backfilling source profiles", error))?;

    let clip_locations = {
        let mut statement = connection
            .prepare(
                "
                SELECT
                    clips.id,
                    clips.file_path,
                    source_dirs.scan_root_path,
                    source_dirs.path,
                    clip_groups.group_key
                FROM clips
                JOIN source_dirs ON source_dirs.id = clips.source_dir_id
                LEFT JOIN clip_groups ON clip_groups.id = clips.clip_group_id
                ORDER BY clips.id
                ",
            )
            .map_err(|error| readable_error("preparing source-relative migration", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|error| readable_error("querying source-relative migration", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading source-relative migration", error))?;
        rows
    };

    for (clip_id, video_path, scan_root_path, source_path, group_key) in clip_locations {
        let relative_dir = super::source_relative_directory(
            &video_path,
            &scan_root_path,
            &source_path,
            group_key.as_deref(),
        );
        connection
            .execute(
                "UPDATE clips SET source_relative_dir = ?2 WHERE id = ?1",
                params![clip_id, relative_dir],
            )
            .map_err(|error| readable_error("backfilling clip source-relative directory", error))?;
    }

    connection
        .execute(
            "
            UPDATE clips
            SET review_decision = CASE
                    WHEN is_favorite != 0 THEN 'liked'
                    ELSE 'unreviewed'
                END,
                reviewed_at = CASE
                    WHEN is_favorite != 0 THEN updated_at
                    ELSE NULL
                END
            ",
            [],
        )
        .map_err(|error| readable_error("backfilling clip review decisions", error))?;

    Ok(())
}

fn migrate_scan_summary_availability_v15(connection: &Connection) -> DbResult<()> {
    // Before v15, completed and partial rows were written only by the full summary persistence
    // path. Cancelled/failed recovery rows may contain default counters, so they deliberately stay
    // unavailable rather than presenting an invented zero.
    connection
        .execute(
            "
            UPDATE scan_runs
            SET summary_available = CASE
                    WHEN status IN ('completed', 'partial') THEN 1
                    ELSE 0
                END
            ",
            [],
        )
        .map_err(|error| readable_error("backfilling scan summary availability", error))?;

    Ok(())
}

fn migrate_deterministic_clip_event_fields_v15(connection: &Connection) -> DbResult<()> {
    #[derive(Debug)]
    struct HistoricalEvent {
        id: i64,
        raw_json: Option<String>,
        duration_ms: Option<i64>,
        highlight_type: Option<String>,
        official_video_name: Option<String>,
        official_video_type: Option<String>,
    }

    let events = {
        let mut statement = connection
            .prepare(
                "
                SELECT
                    clip_events.id,
                    clip_events.raw_json,
                    clips.duration_ms,
                    clip_metadata.highlight_type,
                    clip_metadata.official_video_name,
                    clip_metadata.official_video_type
                FROM clip_events
                JOIN clips ON clips.id = clip_events.clip_id
                LEFT JOIN clip_metadata ON clip_metadata.clip_id = clips.id
                ORDER BY clip_events.id
                ",
            )
            .map_err(|error| {
                readable_error("preparing deterministic clip event migration", error)
            })?;
        let collected = statement
            .query_map([], |row| {
                Ok(HistoricalEvent {
                    id: row.get(0)?,
                    raw_json: row.get(1)?,
                    duration_ms: row.get(2)?,
                    highlight_type: row.get(3)?,
                    official_video_name: row.get(4)?,
                    official_video_type: row.get(5)?,
                })
            })
            .map_err(|error| readable_error("reading historical clip events", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("collecting historical clip events", error))?;
        collected
    };

    for event in events {
        let raw_event = event
            .raw_json
            .as_deref()
            .and_then(|raw_json| serde_json::from_str::<Value>(raw_json).ok());

        if let Some(killed_is_me) = raw_event.as_ref().and_then(historical_killed_is_me) {
            connection
                .execute(
                    "UPDATE clip_events SET killed_is_me = ?2 WHERE id = ?1",
                    params![event.id, i64::from(killed_is_me)],
                )
                .map_err(|error| {
                    readable_error("backfilling historical killed-is-me flag", error)
                })?;
        }

        let highlight_type = event
            .highlight_type
            .as_deref()
            .and_then(historical_integral_i64);
        let time_semantics = classify_timeline_event_time(
            highlight_type,
            event.official_video_name.as_deref().unwrap_or_default(),
            event.official_video_type.as_deref().unwrap_or_default(),
        );
        if time_semantics != Some(TimelineEventTimeSemantics::VideoAbsolute) {
            // Segment-relative and unknown records retain their v14 value. Without a confirmed
            // compilation classification, clearing the time could destroy a correct ordinary
            // highlight marker.
            continue;
        }

        let repaired_video_time_ms = event
            .duration_ms
            .filter(|duration_ms| *duration_ms >= 0)
            .zip(raw_event.as_ref().and_then(historical_event_start_ms))
            .and_then(|(duration_ms, event_start_ms)| {
                (event_start_ms >= 0 && event_start_ms <= duration_ms).then_some(event_start_ms)
            });
        connection
            .execute(
                "UPDATE clip_events SET video_time_ms = ?2 WHERE id = ?1",
                params![event.id, repaired_video_time_ms],
            )
            .map_err(|error| {
                readable_error("repairing historical compilation event time", error)
            })?;
    }

    Ok(())
}

#[derive(Debug)]
struct VerbatimClipRowV16 {
    id: i64,
    source_dir_id: i64,
    file_path: String,
    file_status: String,
    is_favorite: bool,
    note: Option<String>,
    review_decision: String,
    reviewed_at: Option<String>,
    has_trash_snapshot: bool,
    has_delete_intent: bool,
}

/// Repairs the Windows canonical-path regression that allowed `D:\x` and `\\?\D:\x` to be
/// indexed as separate clips. Stable file identity alone is deliberately insufficient because
/// hard links share it. A pair is merged only when all of the following are true:
///
/// - exactly two rows share a complete identity inside one source;
/// - exactly one path uses the Win32 verbatim namespace;
/// - removing only that namespace produces the same normalized path; and
/// - the verbatim duplicate owns no trash snapshot, trash state, or permanent-delete intent.
///
/// The ordinary-path row remains the keeper, preserving its clip id and all authorization state.
fn migrate_windows_verbatim_clip_duplicates_v16(connection: &Connection) -> DbResult<()> {
    let rows = {
        let mut statement = connection
            .prepare(
                "
                SELECT
                    clips.id,
                    clips.source_dir_id,
                    clips.file_path,
                    clips.file_status,
                    clips.is_favorite,
                    clips.note,
                    clips.review_decision,
                    clips.reviewed_at,
                    EXISTS (
                        SELECT 1 FROM clip_trash_snapshots
                        WHERE clip_trash_snapshots.clip_id = clips.id
                    ),
                    EXISTS (
                        SELECT 1 FROM clip_delete_intents
                        WHERE clip_delete_intents.clip_id = clips.id
                    ),
                    clips.file_volume_serial,
                    clips.file_index_high,
                    clips.file_index_low
                FROM clips
                WHERE clips.file_volume_serial IS NOT NULL
                  AND clips.file_index_high IS NOT NULL
                  AND clips.file_index_low IS NOT NULL
                ORDER BY
                    clips.source_dir_id,
                    clips.file_volume_serial,
                    clips.file_index_high,
                    clips.file_index_low,
                    clips.id
                ",
            )
            .map_err(|error| readable_error("preparing v16 verbatim clip repair", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    (
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, i64>(11)?,
                        row.get::<_, i64>(12)?,
                    ),
                    VerbatimClipRowV16 {
                        id: row.get(0)?,
                        source_dir_id: row.get(1)?,
                        file_path: row.get(2)?,
                        file_status: row.get(3)?,
                        is_favorite: row.get::<_, i64>(4)? != 0,
                        note: row.get(5)?,
                        review_decision: row.get(6)?,
                        reviewed_at: row.get(7)?,
                        has_trash_snapshot: row.get::<_, i64>(8)? != 0,
                        has_delete_intent: row.get::<_, i64>(9)? != 0,
                    },
                ))
            })
            .map_err(|error| readable_error("querying v16 verbatim clip repair", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading v16 verbatim clip repair", error))?;
        rows
    };

    let mut by_identity = BTreeMap::<(i64, i64, i64, i64), Vec<VerbatimClipRowV16>>::new();
    for (identity, row) in rows {
        by_identity.entry(identity).or_default().push(row);
    }

    let mut repaired = 0usize;
    let mut skipped_protected = 0usize;
    let mut skipped_review_conflict = 0usize;
    for identity_rows in by_identity.into_values() {
        if identity_rows.len() != 2 {
            continue;
        }
        let ordinary = identity_rows
            .iter()
            .find(|row| !super::has_windows_verbatim_prefix(&row.file_path));
        let verbatim = identity_rows
            .iter()
            .find(|row| super::has_windows_verbatim_prefix(&row.file_path));
        let (Some(keeper), Some(duplicate)) = (ordinary, verbatim) else {
            continue;
        };
        if stable_path_exact_alias_v16(&keeper.file_path)
            != stable_path_exact_alias_v16(&duplicate.file_path)
        {
            continue;
        }
        if duplicate.file_status == "trashed"
            || duplicate.has_trash_snapshot
            || duplicate.has_delete_intent
        {
            skipped_protected = skipped_protected.saturating_add(1);
            continue;
        }
        if keeper.review_decision != "unreviewed"
            && duplicate.review_decision != "unreviewed"
            && keeper.review_decision != duplicate.review_decision
        {
            skipped_review_conflict = skipped_review_conflict.saturating_add(1);
            continue;
        }

        merge_verbatim_clip_pair_v16(connection, keeper, duplicate)?;
        repaired = repaired.saturating_add(1);
    }

    normalize_remaining_verbatim_clip_paths_v16(connection)?;
    if skipped_protected > 0 || skipped_review_conflict > 0 {
        eprintln!(
            "schema v16 left ambiguous verbatim clip pairs unchanged: protected={skipped_protected}, review_conflict={skipped_review_conflict}"
        );
    }
    if repaired > 0 {
        eprintln!("schema v16 merged {repaired} deterministic verbatim clip duplicates");
    }
    Ok(())
}

fn stable_path_exact_alias_v16(path: &str) -> String {
    super::stable_path_for_storage(path).replace('\\', "/")
}

/// Normalizes a legacy source only when it is the sole row for that stable Windows path. Source
/// aliases can own different clips and user state, so ambiguous rows are retained and reported
/// instead of attempting an implicit cross-source merge.
fn migrate_windows_verbatim_source_paths_v16(connection: &Connection) -> DbResult<()> {
    let rows = {
        let mut statement = connection
            .prepare("SELECT id, path, scan_root_path FROM source_dirs ORDER BY id")
            .map_err(|error| readable_error("preparing v16 source path repair", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| readable_error("querying v16 source path repair", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading v16 source path repair", error))?;
        rows
    };
    let mut key_counts = BTreeMap::<String, usize>::new();
    for (_, path, _) in &rows {
        let key = super::normalize_path(path)
            .trim_end_matches('/')
            .to_string();
        *key_counts.entry(key).or_default() += 1;
    }

    let mut repaired = 0usize;
    let mut ambiguous = 0usize;
    for (id, path, scan_root_path) in rows {
        let stable_path = super::stable_path_for_storage(&path);
        let stable_scan_root = super::stable_path_for_storage(&scan_root_path);
        if stable_path == path && stable_scan_root == scan_root_path {
            continue;
        }
        let key = super::normalize_path(&path)
            .trim_end_matches('/')
            .to_string();
        if key_counts.get(&key).copied().unwrap_or_default() != 1 {
            ambiguous = ambiguous.saturating_add(1);
            continue;
        }
        connection
            .execute(
                "UPDATE source_dirs
                 SET path = ?2, scan_root_path = ?3, updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![id, stable_path, stable_scan_root],
            )
            .map_err(|error| readable_error("repairing v16 source paths", error))?;
        repaired = repaired.saturating_add(1);
    }
    if ambiguous > 0 {
        eprintln!(
            "schema v16 left {ambiguous} ambiguous verbatim source rows unchanged; manual source reconciliation is required"
        );
    }
    if repaired > 0 {
        eprintln!("schema v16 normalized {repaired} unambiguous source paths");
    }
    Ok(())
}

fn merge_verbatim_clip_pair_v16(
    connection: &Connection,
    keeper: &VerbatimClipRowV16,
    duplicate: &VerbatimClipRowV16,
) -> DbResult<()> {
    debug_assert_eq!(keeper.source_dir_id, duplicate.source_dir_id);
    let merged_note = merge_clip_notes_v16(keeper.note.as_deref(), duplicate.note.as_deref());
    let (review_decision, reviewed_at) = merge_clip_review_v16(keeper, duplicate);
    let file_status = if keeper.file_status == "trashed" {
        "trashed"
    } else if keeper.file_status == "available" || duplicate.file_status == "available" {
        "available"
    } else {
        "missing"
    };

    connection
        .execute(
            "
            UPDATE clips
            SET clip_group_id = COALESCE(
                    clip_group_id,
                    (SELECT clip_group_id FROM clips AS duplicate WHERE duplicate.id = ?2)
                ),
                modified_at = COALESCE(
                    modified_at,
                    (SELECT modified_at FROM clips AS duplicate WHERE duplicate.id = ?2)
                ),
                duration_ms = COALESCE(
                    duration_ms,
                    (SELECT duration_ms FROM clips AS duplicate WHERE duplicate.id = ?2)
                ),
                recorded_at = COALESCE(
                    recorded_at,
                    (SELECT recorded_at FROM clips AS duplicate WHERE duplicate.id = ?2)
                ),
                source_relative_dir = CASE
                    WHEN NULLIF(TRIM(source_relative_dir), '') IS NULL THEN COALESCE(
                        (SELECT source_relative_dir FROM clips AS duplicate WHERE duplicate.id = ?2),
                        ''
                    )
                    ELSE source_relative_dir
                END,
                cover_path = COALESCE(
                    cover_path,
                    (SELECT cover_path FROM clips AS duplicate WHERE duplicate.id = ?2)
                ),
                cover_source = CASE
                    WHEN cover_source = 'missing' THEN COALESCE(
                        (SELECT cover_source FROM clips AS duplicate WHERE duplicate.id = ?2),
                        cover_source
                    )
                    ELSE cover_source
                END,
                file_status = ?3,
                is_favorite = ?4,
                review_decision = ?5,
                reviewed_at = ?6,
                note = ?7,
                first_indexed_at = MIN(
                    first_indexed_at,
                    (SELECT first_indexed_at FROM clips AS duplicate WHERE duplicate.id = ?2)
                ),
                last_seen_at = MAX(
                    last_seen_at,
                    (SELECT last_seen_at FROM clips AS duplicate WHERE duplicate.id = ?2)
                ),
                updated_at = MAX(
                    updated_at,
                    (SELECT updated_at FROM clips AS duplicate WHERE duplicate.id = ?2)
                )
            WHERE id = ?1
            ",
            params![
                keeper.id,
                duplicate.id,
                file_status,
                i64::from(keeper.is_favorite || duplicate.is_favorite),
                review_decision,
                reviewed_at,
                merged_note,
            ],
        )
        .map_err(|error| readable_error("merging v16 clip state", error))?;

    connection
        .execute(
            "
            INSERT OR IGNORE INTO clip_tags (clip_id, tag_id, created_at)
            SELECT ?1, tag_id, created_at
            FROM clip_tags
            WHERE clip_id = ?2
            ",
            params![keeper.id, duplicate.id],
        )
        .map_err(|error| readable_error("merging v16 clip tags", error))?;

    merge_clip_metadata_v16(connection, keeper.id, duplicate.id)?;
    merge_clip_timeline_v16(connection, keeper.id, duplicate.id)?;
    merge_clip_thumbnail_v16(connection, keeper.id, duplicate.id)?;

    connection
        .execute("DELETE FROM clips WHERE id = ?1", params![duplicate.id])
        .map_err(|error| readable_error("deleting v16 verbatim duplicate", error))?;

    let stable_path = super::stable_path_for_storage(&keeper.file_path);
    let normalized_path = super::normalize_path(&stable_path);
    connection
        .execute(
            "UPDATE clips SET file_path = ?2, normalized_path = ?3 WHERE id = ?1",
            params![keeper.id, stable_path, normalized_path],
        )
        .map_err(|error| readable_error("normalizing v16 keeper path", error))?;
    Ok(())
}

fn merge_clip_notes_v16(keeper: Option<&str>, duplicate: Option<&str>) -> Option<String> {
    let keeper = keeper.map(str::trim).filter(|value| !value.is_empty());
    let duplicate = duplicate.map(str::trim).filter(|value| !value.is_empty());
    match (keeper, duplicate) {
        (None, None) => None,
        (Some(value), None) | (None, Some(value)) => Some(value.to_string()),
        (Some(left), Some(right)) if left == right => Some(left.to_string()),
        (Some(left), Some(right)) => Some(format!("{left}\n\n{right}")),
    }
}

fn merge_clip_review_v16(
    keeper: &VerbatimClipRowV16,
    duplicate: &VerbatimClipRowV16,
) -> (String, Option<String>) {
    if keeper.review_decision == "unreviewed" && duplicate.review_decision != "unreviewed" {
        return (
            duplicate.review_decision.clone(),
            duplicate.reviewed_at.clone(),
        );
    }
    let reviewed_at = match (&keeper.reviewed_at, &duplicate.reviewed_at) {
        (Some(left), Some(right)) if keeper.review_decision == duplicate.review_decision => {
            Some(left.max(right).clone())
        }
        _ => keeper.reviewed_at.clone(),
    };
    (keeper.review_decision.clone(), reviewed_at)
}

fn merge_clip_metadata_v16(
    connection: &Connection,
    keeper_id: i64,
    duplicate_id: i64,
) -> DbResult<()> {
    connection
        .execute(
            "
            INSERT OR IGNORE INTO clip_metadata (
                clip_id, metadata_status, json_path, account_name, player_name, agent_name,
                map_name, game_mode, match_id, round_label, scoreline, kda, weapon_name,
                kill_count, official_video_id, official_video_name, official_video_type,
                highlight_type, round_score, round_score_source, metadata_source, raw_title,
                extracted_text, extra_json, parse_error, updated_at
            )
            SELECT
                ?1, metadata_status, json_path, account_name, player_name, agent_name,
                map_name, game_mode, match_id, round_label, scoreline, kda, weapon_name,
                kill_count, official_video_id, official_video_name, official_video_type,
                highlight_type, round_score, round_score_source, metadata_source, raw_title,
                extracted_text, extra_json, parse_error, updated_at
            FROM clip_metadata
            WHERE clip_id = ?2
            ",
            params![keeper_id, duplicate_id],
        )
        .map_err(|error| readable_error("copying v16 clip metadata", error))?;

    connection
        .execute(
            "
            UPDATE clip_metadata
            SET metadata_status = CASE
                    WHEN metadata_status = 'enriched' THEN metadata_status
                    WHEN (SELECT metadata_status FROM clip_metadata AS donor WHERE donor.clip_id = ?2) = 'enriched'
                        THEN 'enriched'
                    WHEN metadata_status = 'not_found' THEN COALESCE(
                        (SELECT metadata_status FROM clip_metadata AS donor WHERE donor.clip_id = ?2),
                        metadata_status
                    )
                    ELSE metadata_status
                END,
                json_path = COALESCE(NULLIF(TRIM(json_path), ''), (SELECT json_path FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                account_name = COALESCE(NULLIF(TRIM(account_name), ''), (SELECT account_name FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                player_name = COALESCE(NULLIF(TRIM(player_name), ''), (SELECT player_name FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                agent_name = COALESCE(NULLIF(TRIM(agent_name), ''), (SELECT agent_name FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                map_name = COALESCE(NULLIF(TRIM(map_name), ''), (SELECT map_name FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                game_mode = COALESCE(NULLIF(TRIM(game_mode), ''), (SELECT game_mode FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                match_id = COALESCE(NULLIF(TRIM(match_id), ''), (SELECT match_id FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                round_label = COALESCE(NULLIF(TRIM(round_label), ''), (SELECT round_label FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                scoreline = COALESCE(NULLIF(TRIM(scoreline), ''), (SELECT scoreline FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                kda = COALESCE(NULLIF(TRIM(kda), ''), (SELECT kda FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                weapon_name = COALESCE(NULLIF(TRIM(weapon_name), ''), (SELECT weapon_name FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                kill_count = COALESCE(kill_count, (SELECT kill_count FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                official_video_id = COALESCE(NULLIF(TRIM(official_video_id), ''), (SELECT official_video_id FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                official_video_name = COALESCE(NULLIF(TRIM(official_video_name), ''), (SELECT official_video_name FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                official_video_type = COALESCE(NULLIF(TRIM(official_video_type), ''), (SELECT official_video_type FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                highlight_type = COALESCE(NULLIF(TRIM(highlight_type), ''), (SELECT highlight_type FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                round_score = COALESCE(NULLIF(TRIM(round_score), ''), (SELECT round_score FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                round_score_source = COALESCE(NULLIF(TRIM(round_score_source), ''), (SELECT round_score_source FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                metadata_source = COALESCE(NULLIF(TRIM(metadata_source), ''), (SELECT metadata_source FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                raw_title = COALESCE(NULLIF(TRIM(raw_title), ''), (SELECT raw_title FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                extracted_text = COALESCE(NULLIF(TRIM(extracted_text), ''), (SELECT extracted_text FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                extra_json = COALESCE(NULLIF(TRIM(extra_json), ''), (SELECT extra_json FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                parse_error = COALESCE(NULLIF(TRIM(parse_error), ''), (SELECT parse_error FROM clip_metadata AS donor WHERE donor.clip_id = ?2)),
                updated_at = MAX(updated_at, COALESCE(
                    (SELECT updated_at FROM clip_metadata AS donor WHERE donor.clip_id = ?2),
                    updated_at
                ))
            WHERE clip_id = ?1
              AND EXISTS (SELECT 1 FROM clip_metadata AS donor WHERE donor.clip_id = ?2)
            ",
            params![keeper_id, duplicate_id],
        )
        .map_err(|error| readable_error("merging v16 clip metadata", error))?;
    Ok(())
}

fn merge_clip_timeline_v16(
    connection: &Connection,
    keeper_id: i64,
    duplicate_id: i64,
) -> DbResult<()> {
    connection
        .execute(
            "
            INSERT OR IGNORE INTO clip_segments (
                clip_id, segment_key, round_id, start_ms, duration_ms, game_start_ms, game_end_ms
            )
            SELECT ?1, segment_key, round_id, start_ms, duration_ms, game_start_ms, game_end_ms
            FROM clip_segments
            WHERE clip_id = ?2
            ",
            params![keeper_id, duplicate_id],
        )
        .map_err(|error| readable_error("merging v16 clip segments", error))?;

    connection
        .execute(
            "
            UPDATE clip_events
            SET killer_is_me = CASE
                    WHEN killer_is_me != 0 OR COALESCE((
                        SELECT donor.killer_is_me
                        FROM clip_events AS donor
                        WHERE donor.clip_id = ?2 AND donor.event_key = clip_events.event_key
                    ), 0) != 0 THEN 1 ELSE 0
                END,
                killed_is_me = CASE
                    WHEN killed_is_me != 0 OR COALESCE((
                        SELECT donor.killed_is_me
                        FROM clip_events AS donor
                        WHERE donor.clip_id = ?2 AND donor.event_key = clip_events.event_key
                    ), 0) != 0 THEN 1 ELSE 0
                END
            WHERE clip_id = ?1
              AND EXISTS (
                  SELECT 1 FROM clip_events AS donor
                  WHERE donor.clip_id = ?2 AND donor.event_key = clip_events.event_key
              )
            ",
            params![keeper_id, duplicate_id],
        )
        .map_err(|error| readable_error("merging v16 clip event flags", error))?;

    connection
        .execute(
            "
            INSERT OR IGNORE INTO clip_events (
                clip_id, segment_id, event_key, event_type, video_time_ms, event_time, round_id,
                player_name, agent_name, weapon_name, killer_name, killed_name, killer_is_me,
                killed_is_me, raw_json, created_at
            )
            SELECT
                ?1,
                (
                    SELECT target_segment.id
                    FROM clip_segments AS donor_segment
                    JOIN clip_segments AS target_segment
                      ON target_segment.clip_id = ?1
                     AND target_segment.segment_key = donor_segment.segment_key
                    WHERE donor_segment.id = donor_event.segment_id
                ),
                donor_event.event_key,
                donor_event.event_type,
                donor_event.video_time_ms,
                donor_event.event_time,
                donor_event.round_id,
                donor_event.player_name,
                donor_event.agent_name,
                donor_event.weapon_name,
                donor_event.killer_name,
                donor_event.killed_name,
                donor_event.killer_is_me,
                donor_event.killed_is_me,
                donor_event.raw_json,
                donor_event.created_at
            FROM clip_events AS donor_event
            WHERE donor_event.clip_id = ?2
            ",
            params![keeper_id, duplicate_id],
        )
        .map_err(|error| readable_error("merging v16 clip events", error))?;
    Ok(())
}

fn merge_clip_thumbnail_v16(
    connection: &Connection,
    keeper_id: i64,
    duplicate_id: i64,
) -> DbResult<()> {
    let thumbnail = |clip_id| -> DbResult<Option<(String, String)>> {
        connection
            .query_row(
                "SELECT status, updated_at FROM clip_thumbnails WHERE clip_id = ?1",
                params![clip_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| readable_error("reading v16 clip thumbnail", error))
    };
    let keeper_thumbnail = thumbnail(keeper_id)?;
    let duplicate_thumbnail = thumbnail(duplicate_id)?;
    let duplicate_is_better = match (&keeper_thumbnail, &duplicate_thumbnail) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(keeper), Some(duplicate)) => {
            thumbnail_sort_key_v16(duplicate) > thumbnail_sort_key_v16(keeper)
        }
    };
    if duplicate_is_better {
        connection
            .execute(
                "DELETE FROM clip_thumbnails WHERE clip_id = ?1",
                params![keeper_id],
            )
            .map_err(|error| readable_error("replacing v16 clip thumbnail", error))?;
        connection
            .execute(
                "UPDATE clip_thumbnails SET clip_id = ?1 WHERE clip_id = ?2",
                params![keeper_id, duplicate_id],
            )
            .map_err(|error| readable_error("moving v16 clip thumbnail", error))?;
    }
    Ok(())
}

fn thumbnail_sort_key_v16(thumbnail: &(String, String)) -> (u8, &str) {
    let rank = match thumbnail.0.as_str() {
        "ready" => 6,
        "pending" => 5,
        "running" => 4,
        "suppressed" => 3,
        "evicted" => 2,
        "failed" => 1,
        _ => 0,
    };
    (rank, thumbnail.1.as_str())
}

fn normalize_remaining_verbatim_clip_paths_v16(connection: &Connection) -> DbResult<()> {
    let paths = {
        let mut statement = connection
            .prepare("SELECT id, file_path, cover_path FROM clips ORDER BY id")
            .map_err(|error| readable_error("preparing v16 remaining path normalization", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| readable_error("querying v16 remaining path normalization", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| readable_error("reading v16 remaining path normalization", error))?;
        rows
    };

    let mut skipped_conflicts = 0usize;
    for (clip_id, file_path, cover_path) in paths {
        let stable_file_path = super::stable_path_for_storage(&file_path);
        let normalized_path = super::normalize_path(&stable_file_path);
        if stable_file_path != file_path {
            let conflicts = connection
                .query_row(
                    "
                    SELECT COUNT(*)
                    FROM clips
                    WHERE id != ?1
                      AND (normalized_path = ?2 OR file_path = ?3)
                    ",
                    params![clip_id, normalized_path, stable_file_path],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| readable_error("checking v16 path conflicts", error))?;
            if conflicts == 0 {
                connection
                    .execute(
                        "UPDATE clips SET file_path = ?2, normalized_path = ?3 WHERE id = ?1",
                        params![clip_id, stable_file_path, normalized_path],
                    )
                    .map_err(|error| readable_error("normalizing v16 clip path", error))?;
            } else {
                skipped_conflicts = skipped_conflicts.saturating_add(1);
            }
        }
        if let Some(cover_path) = cover_path {
            let stable_cover_path = super::stable_path_for_storage(&cover_path);
            if stable_cover_path != cover_path {
                connection
                    .execute(
                        "UPDATE clips SET cover_path = ?2 WHERE id = ?1",
                        params![clip_id, stable_cover_path],
                    )
                    .map_err(|error| readable_error("normalizing v16 clip cover path", error))?;
            }
        }
    }
    if skipped_conflicts > 0 {
        eprintln!(
            "schema v16 left {skipped_conflicts} verbatim clip paths unchanged because a database path conflict remains"
        );
    }
    Ok(())
}

fn historical_killed_is_me(event: &Value) -> Option<bool> {
    let event = event.as_object()?;
    let top_level = historical_present_field(event, &["KilledIsMe", "killedIsMe", "killed_is_me"]);
    let extended = historical_present_field(event, &["event_ext", "eventExt"])
        .and_then(Value::as_object)
        .and_then(|event_ext| {
            historical_present_field(event_ext, &["KilledIsMe", "killedIsMe", "killed_is_me"])
        });

    // An explicitly present extension value is authoritative even when malformed. Falling back
    // to the top-level value in that case would convert an uncertain historical event into a
    // false certainty.
    match extended {
        Some(value) => historical_strict_bool(value),
        None => top_level.and_then(historical_strict_bool),
    }
}

fn historical_event_start_ms(event: &Value) -> Option<i64> {
    let event = event.as_object()?;
    historical_present_field(
        event,
        &[
            "event_sTime",
            "eventStart",
            "event_start_ms",
            "eventStartMs",
        ],
    )
    .and_then(historical_json_i64)
}

fn historical_present_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

fn historical_strict_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => match historical_json_number_i64(value) {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        },
        Value::String(value) => match value.trim() {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        },
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn historical_json_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(value) => historical_json_number_i64(value),
        Value::String(value) => historical_integral_i64(value),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

fn historical_json_number_i64(value: &serde_json::Number) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().and_then(historical_integral_f64_i64))
}

fn historical_integral_i64(value: &str) -> Option<i64> {
    value.trim().parse().ok().or_else(|| {
        value
            .trim()
            .parse::<f64>()
            .ok()
            .and_then(historical_integral_f64_i64)
    })
}

fn historical_integral_f64_i64(value: f64) -> Option<i64> {
    (value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        // i64::MAX rounds up to 2^63 as f64. A strict upper bound avoids accepting an
        // unrepresentable historical value and silently saturating it during the cast.
        && value < i64::MAX as f64)
        .then_some(value as i64)
}

fn create_clip_identity_validation_triggers(connection: &Connection) -> DbResult<()> {
    // SQLite cannot add a table-level CHECK with ALTER TABLE. Fresh v15 databases have the CHECK
    // in CREATE TABLE; these equivalent triggers enforce all-null/all-set for upgraded databases.
    connection
        .execute_batch(
            "
            CREATE TRIGGER IF NOT EXISTS validate_clip_file_identity_on_insert
            BEFORE INSERT ON clips
            WHEN NOT (
                (NEW.file_volume_serial IS NULL
                    AND NEW.file_index_high IS NULL
                    AND NEW.file_index_low IS NULL)
                OR
                (NEW.file_volume_serial IS NOT NULL
                    AND NEW.file_index_high IS NOT NULL
                    AND NEW.file_index_low IS NOT NULL)
            )
            BEGIN
                SELECT RAISE(ABORT, 'clip file identity must be entirely null or entirely set');
            END;

            CREATE TRIGGER IF NOT EXISTS validate_clip_file_identity_on_update
            BEFORE UPDATE OF file_volume_serial, file_index_high, file_index_low ON clips
            WHEN NOT (
                (NEW.file_volume_serial IS NULL
                    AND NEW.file_index_high IS NULL
                    AND NEW.file_index_low IS NULL)
                OR
                (NEW.file_volume_serial IS NOT NULL
                    AND NEW.file_index_high IS NOT NULL
                    AND NEW.file_index_low IS NOT NULL)
            )
            BEGIN
                SELECT RAISE(ABORT, 'clip file identity must be entirely null or entirely set');
            END;
            ",
        )
        .map_err(|error| readable_error("creating clip identity validation triggers", error))
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
            CREATE INDEX IF NOT EXISTS idx_source_dirs_sync_profile
                ON source_dirs(enabled, scan_mode, scan_root_path);
            CREATE INDEX IF NOT EXISTS idx_clips_source_relative_dir
                ON clips(source_dir_id, source_relative_dir);
            CREATE INDEX IF NOT EXISTS idx_clips_source_file_identity
                ON clips(source_dir_id, file_volume_serial, file_index_high, file_index_low)
                WHERE file_volume_serial IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_clips_source_legacy_fingerprint
                ON clips(source_dir_id, file_name COLLATE NOCASE, size_bytes, modified_at);
            CREATE INDEX IF NOT EXISTS idx_clips_review_queue
                ON clips(
                    review_decision,
                    file_status,
                    source_dir_id,
                    recorded_at DESC,
                    modified_at DESC,
                    id DESC
                );
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
