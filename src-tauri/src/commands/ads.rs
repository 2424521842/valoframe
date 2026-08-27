//! Ad slot commands: creative listing, impression/click recording, and landing-page opening.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::{
    ads::{self, AD_IMAGE_CACHE_DIR_NAME},
    db, AppState,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdCreativeView {
    pub creative_id: String,
    pub title: String,
    pub body: Option<String>,
    pub advertiser_name: String,
    pub weight: i64,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    /// `clip-media` protocol path for the locally cached image. Never a vendor URL: the webview
    /// has no permission to load external origins.
    pub image_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordAdClickInput {
    pub creative_id: String,
    pub slot: String,
}

/// Returns cached creatives that have a usable local image.
///
/// This never performs network I/O, so the slot renders from cache and stays empty when the
/// vendor is unreachable.
#[tauri::command]
pub fn list_ad_creatives(state: State<'_, AppState>) -> Result<Vec<AdCreativeView>, String> {
    if ads::trusted_ad_config()?.is_none() {
        return Ok(Vec::new());
    }
    let connection = db::open_database_read_only(&state.database_path)?;
    let creatives = db::list_ad_creatives(&connection)?;

    Ok(creatives
        .into_iter()
        .filter_map(|creative| {
            let cache_file = creative.cached_image_file.as_deref()?;
            ads::validate_cache_basename(cache_file).ok()?;
            Some(AdCreativeView {
                creative_id: creative.creative_id.clone(),
                title: creative.title,
                body: creative.body,
                advertiser_name: creative.advertiser_name,
                weight: creative.weight,
                start_at: creative.start_at,
                end_at: creative.end_at,
                image_path: format!("ad/{}", creative.creative_id),
            })
        })
        .collect())
}

/// Fetches the vendor manifest and refreshes the local cache.
#[tauri::command]
pub async fn refresh_ad_creatives(app: AppHandle) -> Result<usize, String> {
    let config = match ads::trusted_ad_config() {
        Ok(Some(config)) => config,
        Ok(None) => return clear_cached_ad_creatives(&app),
        Err(error) => {
            let _ = clear_cached_ad_creatives(&app);
            return Err(error);
        }
    };

    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法定位缓存目录：{error}"))?
        .join(AD_IMAGE_CACHE_DIR_NAME);
    let creatives = match ads::fetch_ad_creatives(&config.manifest_endpoint, &cache_root).await {
        Ok(creatives) => creatives,
        Err(error) => {
            // A stale campaign must not survive an offline, malformed, or rejected refresh.
            let _ = clear_cached_ad_creatives(&app);
            return Err(error);
        }
    };

    let database_path = app.state::<AppState>().database_path.clone();
    let connection = db::open_database(&database_path)?;
    db::replace_ad_creatives(&connection, &creatives)
}

fn clear_cached_ad_creatives(app: &AppHandle) -> Result<usize, String> {
    let database_path = app.state::<AppState>().database_path.clone();
    let connection = db::open_database(&database_path)?;
    db::replace_ad_creatives(&connection, &[])
}

#[tauri::command]
pub fn record_ad_impression(
    state: State<'_, AppState>,
    creative_id: String,
    slot: String,
) -> Result<(), String> {
    if ads::trusted_ad_config()?.is_none() {
        return Err("广告活动未配置".to_string());
    }
    if !ads::is_known_ad_slot(&slot) {
        return Err("未知广告位".to_string());
    }
    let connection = db::open_database(&state.database_path)?;
    db::record_ad_impression(&connection, &creative_id, &slot)
}

/// Records the click, builds the tracked landing URL, and opens it in the system browser.
///
/// The URL is rebuilt from the stored template rather than accepted from the frontend, and the
/// host is checked against the vendor allowlist before the shell is involved.
#[tauri::command]
pub fn record_ad_click(
    state: State<'_, AppState>,
    input: RecordAdClickInput,
) -> Result<String, String> {
    let config = ads::trusted_ad_config()?.ok_or_else(|| "广告活动未配置".to_string())?;
    if !ads::is_known_ad_slot(&input.slot) {
        return Err("未知广告位".to_string());
    }

    let connection = db::open_database(&state.database_path)?;
    let creative = db::find_ad_creative(&connection, &input.creative_id)?
        .ok_or_else(|| "广告素材不存在".to_string())?;

    let click_id = ads::generate_click_id();
    let landing_url = ads::build_landing_url(
        &creative.landing_url_template,
        &input.slot,
        &click_id,
        &config.allowed_hosts,
    )?;

    // Persist before opening: an unrecorded click that reaches the vendor cannot be reconciled.
    db::record_ad_click(&connection, &click_id, &creative.creative_id, &input.slot)?;
    open_url_in_default_browser(&landing_url)?;
    Ok(click_id)
}

/// Opens an already-validated HTTPS URL with the system default browser.
///
/// Uses `ShellExecuteW` directly, matching `open_with_default_player`, so no shell plugin or
/// extra window capability is needed.
fn open_url_in_default_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

        // Defence in depth: callers already validated this, but ShellExecuteW would happily run a
        // local executable if a non-HTTP scheme ever reached it.
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("https://") || lower.starts_with("http://")) {
            return Err("落地页地址协议不受支持".to_string());
        }

        let operation = OsStr::new("open")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let target = OsStr::new(url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(format!("系统浏览器启动失败（ShellExecuteW={result:?}）"));
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = url;
        Err("打开落地页当前仅支持 Windows".to_string())
    }
}
