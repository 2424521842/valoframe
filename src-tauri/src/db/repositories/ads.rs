//! Ad slot persistence: vendor creative cache plus local click/impression counters.
//!
//! Nothing in here touches user, game, or file data. The click log exists so ad revenue can be
//! reconciled against the vendor's own reporting instead of being taken on faith.

use rusqlite::{params, Connection, Row};

use super::super::{
    normalize_optional, readable_error, require_non_empty, AdClickRecord, AdCreative, DbResult,
};

/// Replaces the cached creative list with a freshly fetched manifest.
///
/// The click and impression logs are deliberately left untouched: they are reconciliation
/// evidence and must outlive whatever the vendor currently happens to be serving.
pub fn replace_ad_creatives(connection: &Connection, creatives: &[AdCreative]) -> DbResult<usize> {
    connection
        .execute("DELETE FROM ad_creatives", [])
        .map_err(|error| readable_error("clearing cached ad creatives", error))?;

    let mut stored = 0usize;
    for creative in creatives {
        let creative_id = require_non_empty(&creative.creative_id, "ad creative id")?;
        let title = require_non_empty(&creative.title, "ad creative title")?;
        let image_url = require_non_empty(&creative.image_url, "ad creative image url")?;
        let landing_url_template =
            require_non_empty(&creative.landing_url_template, "ad landing url template")?;
        let advertiser_name = require_non_empty(&creative.advertiser_name, "ad advertiser name")?;

        connection
            .execute(
                "
                INSERT INTO ad_creatives (
                    creative_id, title, body, image_url, landing_url_template,
                    advertiser_name, weight, start_at, end_at, cached_image_file, fetched_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, CURRENT_TIMESTAMP)
                ON CONFLICT(creative_id) DO UPDATE SET
                    title = excluded.title,
                    body = excluded.body,
                    image_url = excluded.image_url,
                    landing_url_template = excluded.landing_url_template,
                    advertiser_name = excluded.advertiser_name,
                    weight = excluded.weight,
                    start_at = excluded.start_at,
                    end_at = excluded.end_at,
                    cached_image_file = excluded.cached_image_file,
                    fetched_at = CURRENT_TIMESTAMP
                ",
                params![
                    creative_id,
                    title,
                    normalize_optional(creative.body.as_deref()),
                    image_url,
                    landing_url_template,
                    advertiser_name,
                    creative.weight.max(1),
                    normalize_optional(creative.start_at.as_deref()),
                    normalize_optional(creative.end_at.as_deref()),
                    normalize_optional(creative.cached_image_file.as_deref()),
                ],
            )
            .map_err(|error| readable_error("caching ad creative", error))?;
        stored += 1;
    }

    Ok(stored)
}

pub fn list_ad_creatives(connection: &Connection) -> DbResult<Vec<AdCreative>> {
    let mut statement = connection
        .prepare(
            "
            SELECT creative_id, title, body, image_url, landing_url_template,
                   advertiser_name, weight, start_at, end_at, cached_image_file
            FROM ad_creatives
            ORDER BY creative_id
            ",
        )
        .map_err(|error| readable_error("preparing ad creative query", error))?;

    let rows = statement
        .query_map([], map_ad_creative)
        .map_err(|error| readable_error("listing ad creatives", error))?;

    let mut creatives = Vec::new();
    for row in rows {
        creatives.push(row.map_err(|error| readable_error("reading ad creative", error))?);
    }
    Ok(creatives)
}

pub fn find_ad_creative(
    connection: &Connection,
    creative_id: &str,
) -> DbResult<Option<AdCreative>> {
    let creative_id = require_non_empty(creative_id, "ad creative id")?;
    let mut statement = connection
        .prepare(
            "
            SELECT creative_id, title, body, image_url, landing_url_template,
                   advertiser_name, weight, start_at, end_at, cached_image_file
            FROM ad_creatives
            WHERE creative_id = ?1
            ",
        )
        .map_err(|error| readable_error("preparing ad creative lookup", error))?;

    let mut rows = statement
        .query_map(params![creative_id], map_ad_creative)
        .map_err(|error| readable_error("looking up ad creative", error))?;

    match rows.next() {
        Some(row) => {
            Ok(Some(row.map_err(|error| {
                readable_error("reading ad creative", error)
            })?))
        }
        None => Ok(None),
    }
}

pub fn set_ad_creative_cached_image(
    connection: &Connection,
    creative_id: &str,
    cached_image_file: Option<&str>,
) -> DbResult<()> {
    let creative_id = require_non_empty(creative_id, "ad creative id")?;
    connection
        .execute(
            "UPDATE ad_creatives SET cached_image_file = ?2 WHERE creative_id = ?1",
            params![creative_id, normalize_optional(cached_image_file)],
        )
        .map_err(|error| readable_error("recording cached ad image", error))?;
    Ok(())
}

/// Records a click. The `click_id` is the only value handed to the vendor, so it must be
/// queryable locally afterwards or self-service reconciliation is impossible.
pub fn record_ad_click(
    connection: &Connection,
    click_id: &str,
    creative_id: &str,
    slot: &str,
) -> DbResult<()> {
    let click_id = require_non_empty(click_id, "ad click id")?;
    let creative_id = require_non_empty(creative_id, "ad creative id")?;
    let slot = require_non_empty(slot, "ad slot")?;

    connection
        .execute(
            "
            INSERT INTO ad_click_log (click_id, creative_id, slot, clicked_at)
            VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)
            ",
            params![click_id, creative_id, slot],
        )
        .map_err(|error| readable_error("recording ad click", error))?;
    Ok(())
}

pub fn record_ad_impression(
    connection: &Connection,
    creative_id: &str,
    slot: &str,
) -> DbResult<()> {
    let creative_id = require_non_empty(creative_id, "ad creative id")?;
    let slot = require_non_empty(slot, "ad slot")?;

    connection
        .execute(
            "
            INSERT INTO ad_impression_log (creative_id, slot, impression_date, impression_count)
            VALUES (?1, ?2, DATE('now'), 1)
            ON CONFLICT(creative_id, slot, impression_date) DO UPDATE SET
                impression_count = impression_count + 1
            ",
            params![creative_id, slot],
        )
        .map_err(|error| readable_error("recording ad impression", error))?;
    Ok(())
}

pub fn list_recent_ad_clicks(connection: &Connection, limit: i64) -> DbResult<Vec<AdClickRecord>> {
    let limit = limit.clamp(1, 1000);
    let mut statement = connection
        .prepare(
            "
            SELECT click_id, creative_id, slot, clicked_at
            FROM ad_click_log
            ORDER BY clicked_at DESC, click_id DESC
            LIMIT ?1
            ",
        )
        .map_err(|error| readable_error("preparing ad click query", error))?;

    let rows = statement
        .query_map(params![limit], |row| {
            Ok(AdClickRecord {
                click_id: row.get(0)?,
                creative_id: row.get(1)?,
                slot: row.get(2)?,
                clicked_at: row.get(3)?,
            })
        })
        .map_err(|error| readable_error("listing ad clicks", error))?;

    let mut clicks = Vec::new();
    for row in rows {
        clicks.push(row.map_err(|error| readable_error("reading ad click", error))?);
    }
    Ok(clicks)
}

fn map_ad_creative(row: &Row<'_>) -> rusqlite::Result<AdCreative> {
    Ok(AdCreative {
        creative_id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        image_url: row.get(3)?,
        landing_url_template: row.get(4)?,
        advertiser_name: row.get(5)?,
        weight: row.get(6)?,
        start_at: row.get(7)?,
        end_at: row.get(8)?,
        cached_image_file: row.get(9)?,
    })
}
