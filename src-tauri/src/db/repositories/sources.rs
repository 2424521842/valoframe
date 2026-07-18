//! Source-directory and clip-group persistence.

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::super::{
    normalize_path, readable_error, require_non_empty, ClipGroup, ClipGroupInput, DbResult, Source,
    SourceDir, SourceDirInput,
};

pub fn upsert_source_dir(
    connection: &Connection,
    input: SourceDirInput<'_>,
) -> DbResult<SourceDir> {
    let path = require_non_empty(input.path, "source directory path")?;
    let name = require_non_empty(input.name, "source directory name")?;

    if let Some(existing) = find_source_dir_by_normalized_path(connection, path)? {
        connection
            .execute(
                "
                UPDATE source_dirs
                SET name = ?2,
                    enabled = 1,
                    status = 'available',
                    last_error = NULL,
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                ",
                params![existing.id, name],
            )
            .map_err(|error| readable_error("updating source directory", error))?;

        return find_source_dir_by_id(connection, existing.id);
    }

    connection
        .execute(
            "
            INSERT INTO source_dirs (path, name, enabled, status, last_error)
            VALUES (?1, ?2, 1, 'available', NULL)
            ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                enabled = 1,
                status = 'available',
                last_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![path, name],
        )
        .map_err(|error| readable_error("saving source directory", error))?;

    find_source_dir_by_path(connection, path)
}

pub fn mark_source_dir_scanned(connection: &Connection, source_dir_id: i64) -> DbResult<()> {
    connection
        .execute(
            "
            UPDATE source_dirs
            SET status = 'available',
                last_error = NULL,
                last_scanned_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            ",
            params![source_dir_id],
        )
        .map_err(|error| readable_error("marking source directory scanned", error))?;

    Ok(())
}

pub fn mark_source_dir_scan_error(
    connection: &Connection,
    source_dir_id: i64,
    status: &str,
    error: &str,
) -> DbResult<()> {
    let status = match status {
        "partial" | "unavailable" => status,
        _ => return Err(format!("Unsupported source scan status: {status}")),
    };
    let error = require_non_empty(error, "source scan error")?;

    connection
        .execute(
            "
            UPDATE source_dirs
            SET status = ?2,
                last_error = ?3,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            ",
            params![source_dir_id, status, error],
        )
        .map_err(|database_error| {
            readable_error("marking source directory scan error", database_error)
        })?;

    Ok(())
}

pub fn upsert_clip_group(
    connection: &Connection,
    input: ClipGroupInput<'_>,
) -> DbResult<ClipGroup> {
    let group_key = require_non_empty(input.group_key, "clip group key")?;
    let display_name = require_non_empty(input.display_name, "clip group name")?;

    connection
        .execute(
            "
            INSERT INTO clip_groups (source_dir_id, group_key, display_name)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(source_dir_id, group_key) DO UPDATE SET
                display_name = excluded.display_name,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![input.source_dir_id, group_key, display_name],
        )
        .map_err(|error| readable_error("saving clip group", error))?;

    find_clip_group(connection, input.source_dir_id, group_key)
}

pub fn list_sources(connection: &Connection) -> DbResult<Vec<Source>> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                source_dirs.id,
                source_dirs.path,
                source_dirs.name,
                source_dirs.enabled,
                source_dirs.status,
                source_dirs.last_error,
                COUNT(clips.id) AS clip_count,
                source_dirs.last_scanned_at
            FROM source_dirs
            LEFT JOIN clips
                ON clips.source_dir_id = source_dirs.id
            GROUP BY
                source_dirs.id,
                source_dirs.path,
                source_dirs.name,
                source_dirs.enabled,
                source_dirs.status,
                source_dirs.last_error,
                source_dirs.last_scanned_at
            ORDER BY source_dirs.name COLLATE NOCASE, source_dirs.id
            ",
        )
        .map_err(|error| readable_error("preparing source list query", error))?;

    let sources = statement
        .query_map([], map_source)
        .map_err(|error| readable_error("querying source list", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading source list", error))?;

    Ok(sources)
}

fn find_source_dir_by_path(connection: &Connection, path: &str) -> DbResult<SourceDir> {
    connection
        .query_row(
            "
            SELECT id, path, name, enabled, status, last_error, last_scanned_at
            FROM source_dirs
            WHERE path = ?1
            ",
            params![path],
            map_source_dir,
        )
        .map_err(|error| readable_error("reading source directory", error))
}

fn find_source_dir_by_id(connection: &Connection, source_dir_id: i64) -> DbResult<SourceDir> {
    connection
        .query_row(
            "
            SELECT id, path, name, enabled, status, last_error, last_scanned_at
            FROM source_dirs
            WHERE id = ?1
            ",
            params![source_dir_id],
            map_source_dir,
        )
        .map_err(|error| readable_error("reading source directory", error))
}

fn find_source_dir_by_normalized_path(
    connection: &Connection,
    path: &str,
) -> DbResult<Option<SourceDir>> {
    let normalized_path = normalize_path(path);
    let normalized_path = normalized_path.trim_end_matches('/');

    connection
        .query_row(
            "
            SELECT id, path, name, enabled, status, last_error, last_scanned_at
            FROM source_dirs
            WHERE RTRIM(LOWER(REPLACE(path, '\\', '/')), '/') = ?1
            LIMIT 1
            ",
            params![normalized_path],
            map_source_dir,
        )
        .optional()
        .map_err(|error| readable_error("reading normalized source directory", error))
}

fn find_clip_group(
    connection: &Connection,
    source_dir_id: i64,
    group_key: &str,
) -> DbResult<ClipGroup> {
    connection
        .query_row(
            "
            SELECT id, source_dir_id, group_key, display_name
            FROM clip_groups
            WHERE source_dir_id = ?1
              AND group_key = ?2
            ",
            params![source_dir_id, group_key],
            map_clip_group,
        )
        .map_err(|error| readable_error("reading clip group", error))
}

fn map_source_dir(row: &Row<'_>) -> rusqlite::Result<SourceDir> {
    let enabled: i64 = row.get(3)?;

    Ok(SourceDir {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        enabled: enabled != 0,
        status: row.get(4)?,
        last_error: row.get(5)?,
        last_scanned_at: row.get(6)?,
    })
}

fn map_source(row: &Row<'_>) -> rusqlite::Result<Source> {
    let enabled = row.get::<_, i64>(3)? != 0;
    let status: String = row.get(4)?;

    Ok(Source {
        id: row.get(0)?,
        path: row.get(1)?,
        display_name: row.get(2)?,
        enabled,
        accessibility: enabled && status.eq_ignore_ascii_case("available"),
        status,
        last_error: row.get(5)?,
        clip_count: row.get(6)?,
        last_scan_at: row.get(7)?,
    })
}

fn map_clip_group(row: &Row<'_>) -> rusqlite::Result<ClipGroup> {
    Ok(ClipGroup {
        id: row.get(0)?,
        source_dir_id: row.get(1)?,
        group_key: row.get(2)?,
        display_name: row.get(3)?,
    })
}
