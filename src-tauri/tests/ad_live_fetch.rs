//! Live end-to-end check of the ad fetch path against a local mock manifest server.
//!
//! Ignored by default: it needs `.tmp-mock-ad-server.mjs` listening on 127.0.0.1:8791. Run with
//! `cargo test --manifest-path src-tauri/Cargo.toml --test ad_live_fetch -- --ignored --nocapture`.

use std::path::PathBuf;

use valorant_highlight_manager_lib::ads;

const MOCK_ENDPOINT: &str = "http://127.0.0.1:8791/manifest";

fn temp_cache_root() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("vhm-ad-live-{}-{unique}", std::process::id()))
}

#[test]
#[ignore = "requires the local mock ad server on 127.0.0.1:8791"]
fn live_manifest_fetch_caches_a_validated_creative() {
    let cache_root = temp_cache_root();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime should build");

    let creatives = runtime
        .block_on(ads::fetch_ad_creatives(MOCK_ENDPOINT, &cache_root))
        .expect("mock manifest should fetch");

    assert_eq!(creatives.len(), 1, "mock manifest serves one creative");
    let creative = &creatives[0];
    assert_eq!(creative.creative_id, "cr-mock-001");
    assert_eq!(creative.advertiser_name, "联调广告主");

    // The image must have been validated by magic bytes and written into the cache directory.
    let cache_file = creative
        .cached_image_file
        .as_deref()
        .expect("creative image should be cached");
    assert_eq!(cache_file, "cr-mock-001.png");
    let resolved = ads::resolve_cached_image(&cache_root, cache_file)
        .expect("cached image should resolve inside the cache root");
    assert!(resolved.is_file());

    // The tracked landing URL is built from the stored template, not from the frontend.
    let landing = ads::build_landing_url(
        &creative.landing_url_template,
        ads::AD_SLOT_SIDEBAR,
        "vf-1-2",
        &["127.0.0.1".to_string()],
    )
    .expect("landing url should build for an allowlisted host");
    assert!(landing.contains("sub_id=valoframe-sidebar"));
    assert!(landing.contains("click_id=vf-1-2"));

    // The same creative must be refused when the vendor host is not allowlisted.
    assert!(ads::build_landing_url(
        &creative.landing_url_template,
        ads::AD_SLOT_SIDEBAR,
        "vf-1-2",
        &["ad.example.com".to_string()],
    )
    .is_err());

    let _ = std::fs::remove_dir_all(&cache_root);
}
