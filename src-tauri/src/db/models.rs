//! Database input and output contracts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDir {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub last_scanned_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: i64,
    pub path: String,
    pub display_name: String,
    pub enabled: bool,
    pub status: String,
    pub accessibility: bool,
    pub last_error: Option<String>,
    pub clip_count: i64,
    pub last_scan_at: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct SourceDirInput<'a> {
    pub path: &'a str,
    pub name: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipGroup {
    pub id: i64,
    pub source_dir_id: i64,
    pub group_key: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ClipGroupInput<'a> {
    pub source_dir_id: i64,
    pub group_key: &'a str,
    pub display_name: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountIdentitySource {
    MatchAccountId,
    Openid,
    SourceDir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: i64,
    pub source_dir_id: i64,
    pub clip_group_id: Option<i64>,
    pub clip_group_name: Option<String>,
    pub video_path: String,
    pub normalized_path: String,
    pub file_name: String,
    pub extension: String,
    pub file_size: i64,
    pub modified_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub recorded_at: Option<String>,
    pub cover_path: Option<String>,
    pub cover_source: String,
    /// Generated-thumbnail queue state. Source-owned cover fields above remain authoritative and
    /// are never replaced with an application cache path.
    pub thumbnail_status: Option<String>,
    /// Cache-busting revision for a ready generated thumbnail.
    pub thumbnail_revision: Option<String>,
    pub status: String,
    pub favorite: bool,
    pub note: Option<String>,
    pub extracted_text: String,
    pub account_identity_key: String,
    pub account_identity_source: AccountIdentitySource,
    pub account_display_name: String,
    pub openid: Option<String>,
    pub account_name: Option<String>,
    pub player_name: Option<String>,
    pub agent_name: Option<String>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub metadata_status: String,
    pub match_id: Option<String>,
    pub match_account_id: Option<String>,
    pub scoreline: Option<String>,
    pub kda: Option<String>,
    pub agent_avatar_url: Option<String>,
    pub round_label: Option<String>,
    pub weapon_name: Option<String>,
    pub kill_count: Option<i64>,
    pub match_started_at: Option<String>,
    pub combat_score: Option<i64>,
    pub has_won: Option<bool>,
    pub official_video_name: Option<String>,
    pub official_video_type: Option<String>,
    pub highlight_type: Option<i64>,
    pub round_score: Option<i64>,
    pub metadata_source: Option<String>,
    pub event_count: i64,
    pub clip_events: Vec<ClipEvent>,
    pub tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClipSort {
    #[default]
    ModifiedDesc,
    ModifiedAsc,
    SizeDesc,
    SizeAsc,
    NameAsc,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FavoriteFilter {
    #[default]
    All,
    Favorite,
    NotFavorite,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HighlightFilter {
    #[default]
    All,
    Triple,
    Quad,
    #[serde(alias = "ace")]
    Five,
    Six,
    KillCompilation,
    Death,
}

/// Server-side equivalent of the production library filters.
///
/// Empty strings and the sentinel `"all"` are treated as an unset exact-value filter. Date
/// bounds are inclusive Unix timestamps in seconds, matching the numeric `clips.modified_at`
/// values written by the scanner. `account_id` is the stable `accountIdentityKey` returned by a
/// clip summary (for example `match-account-123` or `source-7`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipListQuery {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub query: Option<String>,
    pub account_id: Option<String>,
    pub source_dir_id: Option<i64>,
    pub agent_name: Option<String>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub tag_id: Option<i64>,
    pub highlight_filter: Option<HighlightFilter>,
    pub favorite_filter: Option<FavoriteFilter>,
    pub file_status: Option<String>,
    pub metadata_status: Option<String>,
    pub modified_from: Option<i64>,
    pub modified_to: Option<i64>,
    pub size_min_bytes: Option<i64>,
    pub size_max_bytes: Option<i64>,
    pub sort_by: Option<ClipSort>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipSummary {
    pub id: i64,
    pub source_dir_id: i64,
    pub source_dir_path: String,
    pub source_dir_name: String,
    pub clip_group_id: Option<i64>,
    pub clip_group_name: Option<String>,
    pub video_path: String,
    pub file_name: String,
    pub file_size: i64,
    pub modified_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub recorded_at: Option<String>,
    pub cover_path: Option<String>,
    pub cover_source: String,
    pub thumbnail_status: Option<String>,
    pub thumbnail_revision: Option<String>,
    pub status: String,
    pub favorite: bool,
    pub account_identity_key: String,
    pub account_identity_source: AccountIdentitySource,
    pub account_display_name: String,
    pub openid: Option<String>,
    pub account_name: Option<String>,
    pub player_name: Option<String>,
    pub agent_name: Option<String>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub metadata_status: String,
    pub match_id: Option<String>,
    pub match_account_id: Option<String>,
    pub scoreline: Option<String>,
    pub kda: Option<String>,
    pub agent_avatar_url: Option<String>,
    pub kill_count: Option<i64>,
    pub match_started_at: Option<String>,
    pub combat_score: Option<i64>,
    pub has_won: Option<bool>,
    pub official_video_name: Option<String>,
    pub official_video_type: Option<String>,
    pub highlight_type: Option<i64>,
    pub round_score: Option<i64>,
    pub metadata_source: Option<String>,
    pub tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipPage {
    pub items: Vec<ClipSummary>,
    pub offset: i64,
    pub limit: i64,
    pub total_count: i64,
    pub has_more: bool,
    pub next_offset: Option<i64>,
}

/// Exact, whole-index facet data. `count` values include every indexed clip, including missing
/// and trashed records; `active_count` excludes only `file_status = 'trashed'` so the production
/// library can preserve its current default scope without deriving counts from loaded pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFacetValue {
    pub value: String,
    pub count: i64,
    pub active_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAccountFacet {
    pub account_identity_key: String,
    pub account_display_name: String,
    pub count: i64,
    pub active_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySourceFacet {
    pub source_dir_id: i64,
    pub count: i64,
    pub active_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTagFacet {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub count: i64,
    pub active_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFacets {
    /// Every indexed clip, including missing and trashed records.
    pub total_count: i64,
    /// Every indexed clip except records in the recycle bin.
    pub active_count: i64,
    pub favorite_count: i64,
    pub active_favorite_count: i64,
    pub trashed_count: i64,
    pub tagged_count: i64,
    pub active_tagged_count: i64,
    pub total_size_bytes: i64,
    pub active_size_bytes: i64,
    pub size_bytes_min: Option<i64>,
    pub size_bytes_max: Option<i64>,
    /// Non-trashed clips whose effective modified timestamp falls on the current local date.
    pub recent_count: i64,
    pub recorded_at_min: Option<i64>,
    pub recorded_at_max: Option<i64>,
    pub modified_at_min: Option<i64>,
    pub modified_at_max: Option<i64>,
    pub file_statuses: Vec<LibraryFacetValue>,
    pub metadata_statuses: Vec<LibraryFacetValue>,
    pub accounts: Vec<LibraryAccountFacet>,
    pub source_dirs: Vec<LibrarySourceFacet>,
    pub agents: Vec<LibraryFacetValue>,
    pub maps: Vec<LibraryFacetValue>,
    pub game_modes: Vec<LibraryFacetValue>,
    /// Values use the production `HighlightFilter` wire names.
    pub kill_types: Vec<LibraryFacetValue>,
    pub tags: Vec<LibraryTagFacet>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipDetail {
    #[serde(flatten)]
    pub clip: Clip,
    /// Full tag records assigned to this clip. `clip.tag_ids` remains for legacy mapping.
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipEvent {
    pub id: i64,
    pub clip_id: i64,
    pub segment_id: Option<i64>,
    pub segment_key: Option<String>,
    pub event_key: String,
    pub event_type: String,
    pub video_time_ms: Option<i64>,
    pub event_time: Option<String>,
    pub round_id: Option<i64>,
    pub player_name: Option<String>,
    pub agent_name: Option<String>,
    pub weapon_name: Option<String>,
    pub killer_name: Option<String>,
    pub killed_name: Option<String>,
    pub killer_is_me: bool,
    pub raw_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ClipSegmentInput<'a> {
    pub segment_key: &'a str,
    pub round_id: Option<i64>,
    pub start_ms: i64,
    pub duration_ms: i64,
    pub game_start_ms: Option<i64>,
    pub game_end_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct ClipEventInput<'a> {
    pub segment_key: Option<&'a str>,
    pub event_key: &'a str,
    pub event_type: &'a str,
    pub video_time_ms: Option<i64>,
    pub event_time: Option<&'a str>,
    pub round_id: Option<i64>,
    pub player_name: Option<&'a str>,
    pub agent_name: Option<&'a str>,
    pub weapon_name: Option<&'a str>,
    pub killer_name: Option<&'a str>,
    pub killed_name: Option<&'a str>,
    pub killer_is_me: bool,
    pub raw_json: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ClipInput<'a> {
    pub source_dir_id: i64,
    pub clip_group_id: Option<i64>,
    pub video_path: &'a str,
    pub file_name: &'a str,
    pub file_size: i64,
    pub modified_at: Option<&'a str>,
    pub duration_ms: Option<i64>,
    pub recorded_at: Option<&'a str>,
    pub cover_path: Option<&'a str>,
    pub cover_source: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct ClipMetadataInput<'a> {
    pub clip_id: i64,
    pub metadata_status: &'a str,
    pub json_path: Option<&'a str>,
    pub account_name: Option<&'a str>,
    pub player_name: Option<&'a str>,
    pub agent_name: Option<&'a str>,
    pub map_name: Option<&'a str>,
    pub game_mode: Option<&'a str>,
    pub scoreline: Option<&'a str>,
    pub kda: Option<&'a str>,
    pub extracted_text: Option<&'a str>,
    pub parse_error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipAgentAssetHint {
    pub source_dir_name: String,
    pub observed_at: i64,
    pub agent_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountNameHint {
    pub account_id: String,
    pub account_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipSaveOutcome {
    Inserted,
    Updated,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedClip {
    pub clip: Clip,
    pub outcome: ClipSaveOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchClipMutationResult {
    /// Number of unique clip ids requested after preserving first-seen order.
    pub requested: usize,
    pub matched: usize,
    /// Number of rows or clip/tag bindings whose stored value actually changed.
    pub updated: usize,
    pub missing_ids: Vec<i64>,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailJob {
    /// Stable job identifier. The queue is one-to-one with clips, so this equals `clip_id`.
    pub id: i64,
    pub clip_id: i64,
    pub video_path: String,
    pub normalized_path: String,
    pub size_bytes: i64,
    pub modified_at: Option<String>,
    pub fingerprint: String,
    pub attempt_count: i64,
    pub revision: Option<String>,
    pub cache_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailStatus {
    pub clip_id: i64,
    pub status: String,
    pub revision: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailEnsureResult {
    pub requested: usize,
    pub queued: usize,
    pub already_queued: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThumbnailReconcileResult {
    pub counts: ThumbnailEnsureResult,
    pub changed: Vec<ThumbnailStatus>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailQueueStatus {
    pub pending: i64,
    pub running: i64,
    pub ready: i64,
    pub failed: i64,
    pub unavailable: i64,
    pub evicted: i64,
    pub cache_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailCacheRef {
    pub clip_id: i64,
    pub cache_file: String,
    pub revision: String,
    pub byte_size: i64,
    pub generated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipMediaPaths {
    pub id: i64,
    pub video_path: String,
    pub extension: String,
    pub cover_path: Option<String>,
    pub cover_source: String,
    pub generated_cover_file: Option<String>,
    pub thumbnail_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipFileTarget {
    pub video_path: String,
    pub file_status: String,
    pub extension: String,
    pub source_dir_path: String,
}
