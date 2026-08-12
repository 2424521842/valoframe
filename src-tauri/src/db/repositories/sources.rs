//! Source-directory and clip-group persistence.

use rusqlite::{params, Connection, Row};

use super::super::{
    normalize_path, readable_error, require_non_empty, stable_path_for_storage, ClipGroup,
    ClipGroupInput, DbResult, Source, SourceDir, SourceDirInput, SourceProfileInput,
};

pub fn upsert_source_dir(
    connection: &Connection,
    input: SourceDirInput<'_>,
) -> DbResult<SourceDir> {
    let profile = SourceProfileInput::aclos(input.path);
    upsert_source_dir_with_profile(connection, input, profile)
}

pub fn upsert_source_dir_with_profile(
    connection: &Connection,
    input: SourceDirInput<'_>,
    profile: SourceProfileInput<'_>,
) -> DbResult<SourceDir> {
    let path = stable_path_for_storage(require_non_empty(input.path, "source directory path")?);
    let name = require_non_empty(input.name, "source directory name")?;
    let scan_root_path = stable_path_for_storage(require_non_empty(
        profile.scan_root_path,
        "source scan root path",
    )?);

    if profile.scan_mode != profile.source_kind.default_scan_mode() {
        return Err(format!(
            "Unsupported scan mode '{}' for source kind '{}'",
            profile.scan_mode.as_str(),
            profile.source_kind.as_str()
        ));
    }

    if let Some(existing) = find_source_dir_by_normalized_path(connection, &path)? {
        connection
            .execute(
                "
                UPDATE source_dirs
                SET name = ?2,
                    source_kind = ?3,
                    scan_mode = ?4,
                    scan_root_path = ?5,
                    path = ?6,
                    enabled = 1,
                    status = 'available',
                    last_error = NULL,
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                ",
                params![
                    existing.id,
                    name,
                    profile.source_kind.as_str(),
                    profile.scan_mode.as_str(),
                    scan_root_path,
                    path,
                ],
            )
            .map_err(|error| readable_error("updating source directory", error))?;

        return find_source_dir_by_id(connection, existing.id);
    }

    connection
        .execute(
            "
            INSERT INTO source_dirs (
                path,
                name,
                source_kind,
                scan_mode,
                scan_root_path,
                enabled,
                status,
                last_error
            )
            VALUES (?1, ?2, ?3, ?4, ?5, 1, 'available', NULL)
            ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                source_kind = excluded.source_kind,
                scan_mode = excluded.scan_mode,
                scan_root_path = excluded.scan_root_path,
                enabled = 1,
                status = 'available',
                last_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                path,
                name,
                profile.source_kind.as_str(),
                profile.scan_mode.as_str(),
                scan_root_path,
            ],
        )
        .map_err(|error| readable_error("saving source directory", error))?;

    find_source_dir_by_path(connection, &path)
}

/// Registers a user-managed scan source without treating registration itself as a successful
/// scan. Existing rows keep their scan health while their explicit display/sync settings are
/// updated; newly registered rows remain pending until the first adapter run finishes.
pub fn register_source_dir(
    connection: &Connection,
    input: SourceDirInput<'_>,
    profile: SourceProfileInput<'_>,
    enabled: bool,
) -> DbResult<SourceDir> {
    let path = stable_path_for_storage(require_non_empty(input.path, "source directory path")?);
    let name = require_non_empty(input.name, "source directory name")?;
    let scan_root_path = stable_path_for_storage(require_non_empty(
        profile.scan_root_path,
        "source scan root path",
    )?);

    if profile.scan_mode != profile.source_kind.default_scan_mode() {
        return Err(format!(
            "Unsupported scan mode '{}' for source kind '{}'",
            profile.scan_mode.as_str(),
            profile.source_kind.as_str()
        ));
    }

    if let Some(existing) = find_source_dir_by_normalized_path(connection, &path)? {
        if existing.source_kind != profile.source_kind || existing.scan_mode != profile.scan_mode {
            return Err(format!(
                "Source '{}' is already registered as '{}'",
                existing.path,
                existing.source_kind.as_str()
            ));
        }

        connection
            .execute(
                "
                UPDATE source_dirs
                SET name = ?2,
                    scan_root_path = ?3,
                    enabled = ?4,
                    path = ?5,
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                ",
                params![existing.id, name, scan_root_path, i64::from(enabled), path],
            )
            .map_err(|error| readable_error("updating registered source", error))?;
        return find_source_dir_by_id(connection, existing.id);
    }

    connection
        .execute(
            "
            INSERT INTO source_dirs (
                path,
                name,
                source_kind,
                scan_mode,
                scan_root_path,
                enabled,
                status,
                last_error
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', NULL)
            ",
            params![
                path,
                name,
                profile.source_kind.as_str(),
                profile.scan_mode.as_str(),
                scan_root_path,
                i64::from(enabled),
            ],
        )
        .map_err(|error| readable_error("registering source directory", error))?;

    find_source_dir_by_path(connection, &path)
}

pub fn set_source_dir_enabled(
    connection: &Connection,
    source_dir_id: i64,
    enabled: bool,
) -> DbResult<SourceDir> {
    let changed = connection
        .execute(
            "
            UPDATE source_dirs
            SET enabled = ?2,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            ",
            params![source_dir_id, i64::from(enabled)],
        )
        .map_err(|error| readable_error("updating source synchronization setting", error))?;
    if changed == 0 {
        return Err(format!("Source id {source_dir_id} was not found"));
    }
    find_source_dir_by_id(connection, source_dir_id)
}

pub fn list_source_dirs(connection: &Connection) -> DbResult<Vec<SourceDir>> {
    let mut statement = connection
        .prepare(
            "
            SELECT
                id,
                path,
                name,
                source_kind,
                scan_mode,
                scan_root_path,
                enabled,
                status,
                last_error,
                strftime('%Y-%m-%dT%H:%M:%SZ', last_scanned_at)
            FROM source_dirs
            ORDER BY id
            ",
        )
        .map_err(|error| readable_error("preparing registered source query", error))?;
    let sources = statement
        .query_map([], map_source_dir)
        .map_err(|error| readable_error("querying registered sources", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading registered sources", error))?;
    Ok(sources)
}

pub fn list_enabled_source_dirs(connection: &Connection) -> DbResult<Vec<SourceDir>> {
    Ok(list_source_dirs(connection)?
        .into_iter()
        .filter(|source| source.enabled)
        .collect())
}

pub fn mark_source_dir_scanned(connection: &Connection, source_dir_id: i64) -> DbResult<()> {
    mark_source_dirs_scan_completed(connection, &[source_dir_id])
}

/// Records a successful source enumeration without changing its freshness timestamp. The
/// job-level completion gate commits `last_scanned_at` only after every source has settled.
pub fn mark_source_dir_scan_succeeded(connection: &Connection, source_dir_id: i64) -> DbResult<()> {
    let changed = connection
        .execute(
            "
            UPDATE source_dirs
            SET status = 'available',
                last_error = NULL,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            ",
            params![source_dir_id],
        )
        .map_err(|error| readable_error("marking source enumeration successful", error))?;
    if changed == 0 {
        return Err(format!("Source id {source_dir_id} was not found"));
    }
    Ok(())
}

/// Commits source freshness only after the caller has established that the entire scan job
/// completed and that each listed source was fully enumerated. Keeping this as one transaction
/// prevents a cancelled/partial multi-source job from making an early source look freshly
/// scanned.
pub fn mark_source_dirs_scan_completed(
    connection: &Connection,
    source_dir_ids: &[i64],
) -> DbResult<()> {
    let mut source_dir_ids = source_dir_ids
        .iter()
        .copied()
        .filter(|source_dir_id| *source_dir_id > 0)
        .collect::<Vec<_>>();
    source_dir_ids.sort_unstable();
    source_dir_ids.dedup();
    if source_dir_ids.is_empty() {
        return Ok(());
    }

    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| readable_error("starting source scan completion", error))?;
    for source_dir_id in source_dir_ids {
        let changed = transaction
            .execute(
                "
                UPDATE source_dirs
                SET status = 'available',
                    last_error = NULL,
                    last_scanned_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                ",
                params![source_dir_id],
            )
            .map_err(|error| readable_error("marking source directory scanned", error))?;
        if changed == 0 {
            return Err(format!("Source id {source_dir_id} was not found"));
        }
    }
    transaction
        .commit()
        .map_err(|error| readable_error("committing source scan completion", error))
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
                source_dirs.source_kind,
                source_dirs.scan_mode,
                source_dirs.scan_root_path,
                source_dirs.enabled,
                source_dirs.status,
                source_dirs.last_error,
                COUNT(clips.id) AS clip_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', source_dirs.last_scanned_at)
            FROM source_dirs
            LEFT JOIN clips
                ON clips.source_dir_id = source_dirs.id
            GROUP BY
                source_dirs.id,
                source_dirs.path,
                source_dirs.name,
                source_dirs.source_kind,
                source_dirs.scan_mode,
                source_dirs.scan_root_path,
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
            SELECT
                id,
                path,
                name,
                source_kind,
                scan_mode,
                scan_root_path,
                enabled,
                status,
                last_error,
                strftime('%Y-%m-%dT%H:%M:%SZ', last_scanned_at)
            FROM source_dirs
            WHERE path = ?1
            ",
            params![path],
            map_source_dir,
        )
        .map_err(|error| readable_error("reading source directory", error))
}

pub fn find_source_dir_by_id(connection: &Connection, source_dir_id: i64) -> DbResult<SourceDir> {
    connection
        .query_row(
            "
            SELECT
                id,
                path,
                name,
                source_kind,
                scan_mode,
                scan_root_path,
                enabled,
                status,
                last_error,
                strftime('%Y-%m-%dT%H:%M:%SZ', last_scanned_at)
            FROM source_dirs
            WHERE id = ?1
            ",
            params![source_dir_id],
            map_source_dir,
        )
        .map_err(|error| readable_error("reading source directory", error))
}

pub fn find_source_dir_by_normalized_path(
    connection: &Connection,
    path: &str,
) -> DbResult<Option<SourceDir>> {
    let normalized_path = normalize_path(path);
    let normalized_path = normalized_path.trim_end_matches('/');
    let matches = list_source_dirs(connection)?
        .into_iter()
        .filter(|source| normalize_path(&source.path).trim_end_matches('/') == normalized_path)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [source] => Ok(Some(source.clone())),
        _ => Err(format!(
            "Multiple source directories resolve to the same normalized path: {path}"
        )),
    }
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
    let enabled: i64 = row.get(6)?;

    Ok(SourceDir {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        source_kind: row.get(3)?,
        scan_mode: row.get(4)?,
        scan_root_path: row.get(5)?,
        enabled: enabled != 0,
        status: row.get(7)?,
        last_error: row.get(8)?,
        last_scanned_at: row.get(9)?,
    })
}

fn map_source(row: &Row<'_>) -> rusqlite::Result<Source> {
    let enabled = row.get::<_, i64>(6)? != 0;
    let status: String = row.get(7)?;

    Ok(Source {
        id: row.get(0)?,
        path: row.get(1)?,
        display_name: row.get(2)?,
        source_kind: row.get(3)?,
        scan_mode: row.get(4)?,
        scan_root_path: row.get(5)?,
        enabled,
        accessibility: enabled && status.eq_ignore_ascii_case("available"),
        status,
        last_error: row.get(8)?,
        clip_count: row.get(9)?,
        last_scan_at: row.get(10)?,
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
