//! User-created tag persistence.

use rusqlite::{params, Connection, Row};

use super::super::{normalize_optional, readable_error, require_non_empty, DbResult, Tag};

const TAG_COLORS: [&str; 5] = ["red", "teal", "gold", "blue", "green"];
const MAX_TAG_NAME_CHARS: usize = 24;

pub fn create_tag(connection: &Connection, name: &str, color: Option<&str>) -> DbResult<Tag> {
    let name = validate_tag_name(name)?;
    let color = validate_tag_color(color)?;

    connection
        .execute(
            "
            INSERT INTO tags (name, color, created_at, updated_at)
            VALUES (?1, ?2, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT(name) DO NOTHING
            ",
            params![name, color],
        )
        .map_err(|error| readable_error("creating tag", error))?;

    find_tag_by_name(connection, name)
}

pub fn update_tag(
    connection: &Connection,
    tag_id: i64,
    name: &str,
    color: Option<&str>,
) -> DbResult<Tag> {
    let name = validate_tag_name(name)?;
    let color = validate_tag_color(color)?;
    find_tag_by_id(connection, tag_id)?;

    connection
        .execute(
            "
            UPDATE tags
            SET name = ?1,
                color = ?2,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?3
            ",
            params![name, color, tag_id],
        )
        .map_err(|error| readable_error("updating tag", error))?;

    find_tag_by_id(connection, tag_id)
}

pub fn delete_tag(connection: &Connection, tag_id: i64) -> DbResult<()> {
    find_tag_by_id(connection, tag_id)?;

    let changed = connection
        .execute("DELETE FROM tags WHERE id = ?1", params![tag_id])
        .map_err(|error| readable_error("deleting tag", error))?;

    if changed == 0 {
        Err(format!(
            "deleting tag failed: tag id {tag_id} was not found"
        ))
    } else {
        Ok(())
    }
}

pub fn list_tags(connection: &Connection) -> DbResult<Vec<Tag>> {
    let mut statement = connection
        .prepare(
            "
            SELECT id, name, color
            FROM tags
            ORDER BY id
            ",
        )
        .map_err(|error| readable_error("preparing tag list query", error))?;

    let tags = statement
        .query_map([], map_tag)
        .map_err(|error| readable_error("querying tag list", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading tag list", error))?;

    Ok(tags)
}

pub fn assign_tag_to_clip(connection: &Connection, clip_id: i64, tag_id: i64) -> DbResult<()> {
    connection
        .execute(
            "
            INSERT INTO clip_tags (clip_id, tag_id)
            VALUES (?1, ?2)
            ON CONFLICT(clip_id, tag_id) DO NOTHING
            ",
            params![clip_id, tag_id],
        )
        .map_err(|error| readable_error("assigning tag to clip", error))?;

    Ok(())
}

pub fn remove_tag_from_clip(connection: &Connection, clip_id: i64, tag_id: i64) -> DbResult<()> {
    connection
        .execute(
            "
            DELETE FROM clip_tags
            WHERE clip_id = ?1
              AND tag_id = ?2
            ",
            params![clip_id, tag_id],
        )
        .map_err(|error| readable_error("removing clip tag", error))?;

    Ok(())
}

fn find_tag_by_name(connection: &Connection, name: &str) -> DbResult<Tag> {
    connection
        .query_row(
            "
            SELECT id, name, color
            FROM tags
            WHERE name = ?1
            ",
            params![name],
            map_tag,
        )
        .map_err(|error| readable_error("reading tag", error))
}

fn find_tag_by_id(connection: &Connection, tag_id: i64) -> DbResult<Tag> {
    connection
        .query_row(
            "
            SELECT id, name, color
            FROM tags
            WHERE id = ?1
            ",
            params![tag_id],
            map_tag,
        )
        .map_err(|error| readable_error("reading tag", error))
}

pub(in crate::db) fn list_tags_for_clip(
    connection: &Connection,
    clip_id: i64,
) -> DbResult<Vec<Tag>> {
    let mut statement = connection
        .prepare(
            "
            SELECT tags.id, tags.name, tags.color
            FROM tags
            JOIN clip_tags ON clip_tags.tag_id = tags.id
            WHERE clip_tags.clip_id = ?1
            ORDER BY tags.id
            ",
        )
        .map_err(|error| readable_error("preparing clip detail tag query", error))?;
    let tags = statement
        .query_map(params![clip_id], map_tag)
        .map_err(|error| readable_error("querying clip detail tags", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| readable_error("reading clip detail tags", error))?;
    Ok(tags)
}

fn map_tag(row: &Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
    })
}

fn validate_tag_name(value: &str) -> DbResult<&str> {
    let name = require_non_empty(value, "tag name")?;
    if name.chars().count() > MAX_TAG_NAME_CHARS {
        Err(format!(
            "tag name cannot exceed {MAX_TAG_NAME_CHARS} characters"
        ))
    } else {
        Ok(name)
    }
}

fn validate_tag_color(value: Option<&str>) -> DbResult<Option<&str>> {
    let color = normalize_optional(value);
    match color {
        Some(color) if !TAG_COLORS.contains(&color) => Err(format!(
            "tag color must be one of: {}",
            TAG_COLORS.join(", ")
        )),
        _ => Ok(color),
    }
}
