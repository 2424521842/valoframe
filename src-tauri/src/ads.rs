//! Ad manifest fetching, creative image caching, and landing-URL construction.
//!
//! Security posture: the vendor manifest is untrusted input. Every field is validated before it
//! reaches the database or the webview, images are verified by magic bytes rather than by the
//! `Content-Type` header, and landing URLs are checked against an allowlist before any browser is
//! launched. The frontend never talks to a vendor host directly — see `docs/AD_INTEGRATION_SPEC.md`.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

use crate::db::AdCreative;

pub const AD_IMAGE_CACHE_DIR_NAME: &str = "ads";
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_CREATIVE_IMAGE_BYTES: u64 = 512 * 1024;
const MIN_CREATIVE_IMAGE_BYTES: u64 = 64;
const MAX_CREATIVES: usize = 32;
const MAX_ENDPOINT_CHARS: usize = 300;
const MAX_TITLE_CHARS: usize = 24;
const MAX_BODY_CHARS: usize = 60;
const MAX_ADVERTISER_CHARS: usize = 20;
const MAX_CREATIVE_ID_CHARS: usize = 64;
const FETCH_TIMEOUT_SECS: u64 = 20;

/// Slot identifiers double as the `sub_id` reported to the vendor.
pub const AD_SLOT_SIDEBAR: &str = "valoframe-sidebar";
pub const AD_SLOT_LIBRARY: &str = "valoframe-library";

pub fn is_known_ad_slot(slot: &str) -> bool {
    matches!(slot, AD_SLOT_SIDEBAR | AD_SLOT_LIBRARY)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdManifest {
    #[serde(default)]
    schema_version: i64,
    #[serde(default)]
    creatives: Vec<ManifestCreative>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestCreative {
    creative_id: String,
    title: String,
    #[serde(default)]
    body: Option<String>,
    image_url: String,
    landing_url_template: String,
    advertiser_name: String,
    #[serde(default)]
    weight: Option<i64>,
    #[serde(default)]
    start_at: Option<String>,
    #[serde(default)]
    end_at: Option<String>,
}

/// Validates a vendor endpoint. Mirrors the feedback endpoint rule: HTTPS only, with plain HTTP
/// allowed for loopback so the integration can be exercised against a local mock.
pub fn normalize_ad_endpoint(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.chars().count() > MAX_ENDPOINT_CHARS {
        return Err(format!("广告接口地址不能超过 {MAX_ENDPOINT_CHARS} 字"));
    }
    if !is_https_or_loopback(trimmed) {
        return Err("广告接口必须以 https:// 开头；本机测试可使用 http://localhost".to_string());
    }
    Ok(trimmed.to_string())
}

fn is_https_or_loopback(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://localhost")
        || lower.starts_with("http://127.0.0.1")
        || lower.starts_with("http://[::1]")
}

/// Parses and validates an untrusted manifest payload.
///
/// Invalid individual creatives are dropped rather than failing the whole fetch, so one bad row
/// from the vendor cannot blank the slot entirely.
pub fn parse_ad_manifest(payload: &[u8]) -> Result<Vec<AdCreative>, String> {
    if payload.len() > MAX_MANIFEST_BYTES {
        return Err("广告清单过大".to_string());
    }
    let manifest: AdManifest =
        serde_json::from_slice(payload).map_err(|error| format!("广告清单解析失败：{error}"))?;
    if manifest.schema_version != 0 && manifest.schema_version != 1 {
        return Err(format!("不支持的广告清单版本 {}", manifest.schema_version));
    }

    let mut creatives = Vec::new();
    let mut seen_ids = Vec::new();
    for raw in manifest.creatives {
        let Some(creative) = validate_manifest_creative(raw) else {
            continue;
        };
        if seen_ids.contains(&creative.creative_id) {
            continue;
        }
        seen_ids.push(creative.creative_id.clone());
        creatives.push(creative);
        if creatives.len() >= MAX_CREATIVES {
            break;
        }
    }
    Ok(creatives)
}

fn validate_manifest_creative(raw: ManifestCreative) -> Option<AdCreative> {
    let creative_id = raw.creative_id.trim().to_string();
    if creative_id.is_empty() || creative_id.chars().count() > MAX_CREATIVE_ID_CHARS {
        return None;
    }
    // The id becomes part of a cache filename, so restrict it to characters that cannot escape a
    // directory or collide with path separators.
    if !creative_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
    {
        return None;
    }

    let title = raw.title.trim().to_string();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return None;
    }

    let body = raw
        .body
        .map(|body| body.trim().to_string())
        .filter(|body| !body.is_empty());
    if body
        .as_ref()
        .is_some_and(|body| body.chars().count() > MAX_BODY_CHARS)
    {
        return None;
    }

    let advertiser_name = raw.advertiser_name.trim().to_string();
    if advertiser_name.is_empty() || advertiser_name.chars().count() > MAX_ADVERTISER_CHARS {
        return None;
    }

    let image_url = raw.image_url.trim().to_string();
    if !is_https_or_loopback(&image_url) || image_url.chars().count() > MAX_ENDPOINT_CHARS {
        return None;
    }

    let landing_url_template = raw.landing_url_template.trim().to_string();
    if !is_https_or_loopback(&landing_url_template)
        || landing_url_template.chars().count() > MAX_ENDPOINT_CHARS
        || !landing_url_template.contains("{click_id}")
    {
        return None;
    }

    Some(AdCreative {
        creative_id,
        title,
        body,
        image_url,
        landing_url_template,
        advertiser_name,
        weight: raw.weight.unwrap_or(100).clamp(1, 10_000),
        start_at: raw
            .start_at
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        end_at: raw
            .end_at
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        cached_image_file: None,
    })
}

/// Substitutes the tracking placeholders and verifies the result is safe to hand to the shell.
///
/// The host allowlist is the load-bearing check: without it a tampered manifest could smuggle a
/// non-HTTP scheme into `ShellExecuteW` and get arbitrary local execution.
pub fn build_landing_url(
    template: &str,
    slot: &str,
    click_id: &str,
    allowed_hosts: &[String],
) -> Result<String, String> {
    if !is_known_ad_slot(slot) {
        return Err("未知广告位".to_string());
    }
    if click_id.is_empty()
        || !click_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("点击标识无效".to_string());
    }

    let url = template
        .replace("{sub_id}", slot)
        .replace("{click_id}", click_id);

    if url.contains('{') || url.contains('}') {
        return Err("落地页地址仍含未替换占位符".to_string());
    }
    if !is_https_or_loopback(&url) {
        return Err("落地页地址必须为 https://".to_string());
    }

    let host = host_from_url(&url).ok_or_else(|| "落地页地址无法解析域名".to_string())?;
    if !host_is_allowed(&host, allowed_hosts) {
        return Err(format!("落地页域名 {host} 不在允许列表内"));
    }
    Ok(url)
}

fn host_from_url(url: &str) -> Option<String> {
    let without_scheme = url
        .split_once("://")
        .map(|(_, remainder)| remainder)
        .unwrap_or(url);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    // Reject embedded credentials outright rather than trying to interpret them.
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = authority
        .rsplit_once(':')
        .filter(|(head, _)| !head.ends_with(']'))
        .map(|(head, _)| head)
        .unwrap_or(authority);
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

fn host_is_allowed(host: &str, allowed_hosts: &[String]) -> bool {
    allowed_hosts.iter().any(|allowed| {
        let allowed = allowed.trim().to_ascii_lowercase();
        if allowed.is_empty() {
            return false;
        }
        // A bare entry authorizes that host plus its subdomains, never a suffix match that would
        // let "evil-example.com" pass for "example.com".
        host == allowed || host.ends_with(&format!(".{allowed}"))
    })
}

pub fn generate_click_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("vf-{}-{}", std::process::id(), millis)
}

/// Verifies downloaded bytes really are a PNG or JPEG and returns the matching extension.
pub fn creative_image_extension(bytes: &[u8]) -> Option<&'static str> {
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.starts_with(&PNG_MAGIC) {
        return Some("png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) && bytes.ends_with(&[0xFF, 0xD9]) {
        return Some("jpg");
    }
    None
}

pub fn cache_file_name(creative_id: &str, extension: &str) -> Option<String> {
    if creative_id.is_empty()
        || !creative_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return None;
    }
    if !matches!(extension, "png" | "jpg") {
        return None;
    }
    Some(format!("{creative_id}.{extension}"))
}

/// Resolves a cached image basename to a real file, refusing anything that escapes the cache
/// directory. Mirrors `thumbnail::resolve_ready_cache_file`.
pub fn resolve_cached_image(cache_root: &Path, cache_file: &str) -> Result<PathBuf, String> {
    validate_cache_basename(cache_file)?;
    let root = fs::canonicalize(cache_root).map_err(|_| "ad-cache-unavailable".to_string())?;
    let candidate = root.join(cache_file);
    let metadata =
        fs::symlink_metadata(&candidate).map_err(|_| "ad-cache-file-missing".to_string())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("ad-cache-file-invalid".to_string());
    }
    let canonical =
        fs::canonicalize(&candidate).map_err(|_| "ad-cache-file-invalid".to_string())?;
    if canonical.parent() != Some(root.as_path()) {
        return Err("ad-cache-path-escape".to_string());
    }
    Ok(canonical)
}

pub fn validate_cache_basename(cache_file: &str) -> Result<(), String> {
    let path = Path::new(cache_file);
    let mut components = path.components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("invalid ad cache basename".to_string());
    }
    let Some((stem, extension)) = cache_file.rsplit_once('.') else {
        return Err("invalid ad cache basename".to_string());
    };
    if !matches!(extension, "png" | "jpg") {
        return Err("invalid ad cache extension".to_string());
    }
    if stem.is_empty()
        || !stem.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err("invalid ad cache basename".to_string());
    }
    Ok(())
}

/// Fetches the manifest and caches each creative image. Returns creatives whose image is on disk.
pub async fn fetch_ad_creatives(
    endpoint: &str,
    cache_root: &Path,
) -> Result<Vec<AdCreative>, String> {
    let endpoint = normalize_ad_endpoint(endpoint)?;
    if endpoint.is_empty() {
        return Ok(Vec::new());
    }

    // reqwest ships with no default crypto provider here; building a Client without this panics.
    crate::http_client::ensure_crypto_provider();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("广告客户端初始化失败：{error}"))?;

    let response = client
        .get(&endpoint)
        .send()
        .await
        .map_err(|error| format!("广告清单请求失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("广告清单返回状态 {}", response.status()));
    }
    let payload = response
        .bytes()
        .await
        .map_err(|error| format!("广告清单读取失败：{error}"))?;

    let creatives = parse_ad_manifest(&payload)?;
    fs::create_dir_all(cache_root).map_err(|error| format!("广告素材缓存目录创建失败：{error}"))?;

    let mut cached = Vec::new();
    for mut creative in creatives {
        match cache_creative_image(&client, &creative, cache_root).await {
            Ok(cache_file) => {
                creative.cached_image_file = Some(cache_file);
                cached.push(creative);
            }
            // A single unusable image should not take the rest of the batch down with it.
            Err(error) => eprintln!("Skipping ad creative {}: {error}", creative.creative_id),
        }
    }
    Ok(cached)
}

async fn cache_creative_image(
    client: &reqwest::Client,
    creative: &AdCreative,
    cache_root: &Path,
) -> Result<String, String> {
    let response = client
        .get(&creative.image_url)
        .send()
        .await
        .map_err(|error| format!("素材图请求失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("素材图返回状态 {}", response.status()));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_CREATIVE_IMAGE_BYTES {
            return Err("素材图超过体积上限".to_string());
        }
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("素材图读取失败：{error}"))?;
    let size = bytes.len() as u64;
    if !(MIN_CREATIVE_IMAGE_BYTES..=MAX_CREATIVE_IMAGE_BYTES).contains(&size) {
        return Err("素材图体积不合规".to_string());
    }

    // Trust the bytes, not the declared Content-Type.
    let extension =
        creative_image_extension(&bytes).ok_or_else(|| "素材图不是 PNG/JPEG".to_string())?;
    let cache_file = cache_file_name(&creative.creative_id, extension)
        .ok_or_else(|| "素材缓存文件名无效".to_string())?;

    fs::write(cache_root.join(&cache_file), &bytes)
        .map_err(|error| format!("素材图写入失败：{error}"))?;
    Ok(cache_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed() -> Vec<String> {
        vec!["ad.example.com".to_string()]
    }

    #[test]
    fn endpoint_normalization_accepts_https_and_loopback_http_only() {
        assert_eq!(normalize_ad_endpoint("  "), Ok(String::new()));
        assert_eq!(
            normalize_ad_endpoint("https://ad.example.com/manifest").unwrap(),
            "https://ad.example.com/manifest"
        );
        assert!(normalize_ad_endpoint("http://localhost:8080/ads").is_ok());
        assert!(normalize_ad_endpoint("http://127.0.0.1/ads").is_ok());
        assert!(normalize_ad_endpoint("http://ad.example.com/ads").is_err());
        assert!(normalize_ad_endpoint("ftp://ad.example.com/ads").is_err());
        assert!(normalize_ad_endpoint(&format!("https://a.com/{}", "b".repeat(300))).is_err());
    }

    #[test]
    fn manifest_parsing_keeps_valid_creatives_and_drops_bad_ones() {
        let payload = r#"{
            "schemaVersion": 1,
            "creatives": [
                {
                    "creativeId": "cr-001",
                    "title": "正规素材",
                    "body": "描述",
                    "imageUrl": "https://cdn.example.com/a.jpg",
                    "landingUrlTemplate": "https://ad.example.com/lp?sub_id={sub_id}&click_id={click_id}",
                    "advertiserName": "某广告主",
                    "weight": 50
                },
                {
                    "creativeId": "cr-002",
                    "title": "无点击占位符",
                    "imageUrl": "https://cdn.example.com/b.jpg",
                    "landingUrlTemplate": "https://ad.example.com/lp",
                    "advertiserName": "某广告主"
                },
                {
                    "creativeId": "cr-003",
                    "title": "图片非 https",
                    "imageUrl": "http://cdn.example.com/c.jpg",
                    "landingUrlTemplate": "https://ad.example.com/lp?click_id={click_id}",
                    "advertiserName": "某广告主"
                },
                {
                    "creativeId": "../escape",
                    "title": "路径穿越 ID",
                    "imageUrl": "https://cdn.example.com/d.jpg",
                    "landingUrlTemplate": "https://ad.example.com/lp?click_id={click_id}",
                    "advertiserName": "某广告主"
                }
            ]
        }"#;

        let creatives = parse_ad_manifest(payload.as_bytes()).expect("manifest should parse");
        assert_eq!(creatives.len(), 1);
        assert_eq!(creatives[0].creative_id, "cr-001");
        assert_eq!(creatives[0].weight, 50);
    }

    #[test]
    fn manifest_parsing_rejects_oversized_and_unknown_versions() {
        let oversized = vec![b'x'; MAX_MANIFEST_BYTES + 1];
        assert!(parse_ad_manifest(&oversized).is_err());
        assert!(parse_ad_manifest(br#"{"schemaVersion": 99, "creatives": []}"#).is_err());
    }

    #[test]
    fn landing_url_substitutes_placeholders() {
        let url = build_landing_url(
            "https://ad.example.com/lp?sub_id={sub_id}&click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf-1-2",
            &allowed(),
        )
        .expect("landing url should build");
        assert_eq!(
            url,
            "https://ad.example.com/lp?sub_id=valoframe-sidebar&click_id=vf-1-2"
        );
    }

    #[test]
    fn landing_url_enforces_host_allowlist_without_suffix_confusion() {
        // A tampered manifest must not be able to redirect the user to a lookalike host.
        assert!(build_landing_url(
            "https://evil-ad.example.com.attacker.net/lp?click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf-1-2",
            &allowed(),
        )
        .is_err());
        // "evilad.example.com" must not satisfy an "ad.example.com" entry via bare suffix match.
        assert!(build_landing_url(
            "https://evilad.example.com/lp?click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf-1-2",
            &allowed(),
        )
        .is_err());
        // Genuine subdomains remain allowed.
        assert!(build_landing_url(
            "https://lp.ad.example.com/x?click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf-1-2",
            &allowed(),
        )
        .is_ok());
    }

    #[test]
    fn landing_url_rejects_non_https_credentials_and_unknown_slots() {
        assert!(build_landing_url(
            "file:///C:/Windows/System32/calc.exe?click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf-1-2",
            &allowed(),
        )
        .is_err());
        assert!(build_landing_url(
            "https://ad.example.com@attacker.net/lp?click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf-1-2",
            &allowed(),
        )
        .is_err());
        assert!(build_landing_url(
            "https://ad.example.com/lp?click_id={click_id}",
            "unknown-slot",
            "vf-1-2",
            &allowed(),
        )
        .is_err());
        assert!(build_landing_url(
            "https://ad.example.com/lp?click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf 1;2",
            &allowed(),
        )
        .is_err());
        // An empty allowlist blocks everything.
        assert!(build_landing_url(
            "https://ad.example.com/lp?click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf-1-2",
            &[],
        )
        .is_err());
    }

    #[test]
    fn landing_url_rejects_leftover_placeholders() {
        assert!(build_landing_url(
            "https://ad.example.com/lp?extra={campaign}&click_id={click_id}",
            AD_SLOT_SIDEBAR,
            "vf-1-2",
            &allowed(),
        )
        .is_err());
    }

    #[test]
    fn image_extension_trusts_magic_bytes_not_declared_type() {
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01];
        assert_eq!(creative_image_extension(&png), Some("png"));

        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(creative_image_extension(&jpeg), Some("jpg"));

        assert_eq!(creative_image_extension(b"<html>nope</html>"), None);
        assert_eq!(creative_image_extension(b"MZ\x90\x00"), None);
        // A truncated JPEG has no end-of-image marker and must be refused.
        assert_eq!(creative_image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), None);
    }

    #[test]
    fn cache_basenames_reject_traversal_and_foreign_extensions() {
        assert_eq!(
            cache_file_name("cr-001", "png").as_deref(),
            Some("cr-001.png")
        );
        assert!(cache_file_name("../escape", "png").is_none());
        assert!(cache_file_name("cr-001", "exe").is_none());

        assert!(validate_cache_basename("cr-001.jpg").is_ok());
        assert!(validate_cache_basename("../cr-001.jpg").is_err());
        assert!(validate_cache_basename("sub/cr-001.jpg").is_err());
        assert!(validate_cache_basename("cr-001.exe").is_err());
        assert!(validate_cache_basename("cr-001").is_err());
    }
}
