use serde::Serialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use tauri::State;

use super::media_protocol::{get_clip_media_for_database, FILE_NOT_FOUND_MESSAGE};
use crate::{critical_tasks::CriticalTaskKind, db, AppState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMediaResponse {
    pub clip_id: i64,
    pub playable: bool,
    pub media_path: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipDetailCommandError {
    pub code: &'static str,
    pub message: String,
    pub clip_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermanentDeleteFailure {
    pub clip_id: i64,
    pub code: String,
    pub retryable: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermanentDeleteResult {
    pub requested: usize,
    pub deleted_ids: Vec<i64>,
    pub missing_ids: Vec<i64>,
    pub pending_ids: Vec<i64>,
    pub blocked: Vec<PermanentDeleteFailure>,
    pub failures: Vec<PermanentDeleteFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRemovalProblem {
    pub clip_id: i64,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveClipsFromIndexResult {
    pub requested: usize,
    pub removed_ids: Vec<i64>,
    pub missing_ids: Vec<i64>,
    pub blocked: Vec<IndexRemovalProblem>,
    pub failures: Vec<IndexRemovalProblem>,
}

const MAX_INDEX_REMOVAL_CLIP_IDS: usize = 200;

#[tauri::command]
/// Legacy compatibility command. The paginated production replacement is `list_clip_page`.
pub fn list_clips(state: State<'_, AppState>) -> Result<Vec<db::Clip>, String> {
    let connection = db::open_database_read_only(&state.database_path)?;
    db::list_clips(&connection)
}

#[tauri::command]
pub fn list_clip_page(
    state: State<'_, AppState>,
    query: db::ClipListQuery,
) -> Result<db::ClipPage, String> {
    let connection = db::open_database_read_only(&state.database_path)?;
    db::list_clip_page(&connection, &query)
}

#[tauri::command]
pub fn list_review_clip_page(
    state: State<'_, AppState>,
    query: db::ReviewQueueQuery,
) -> Result<db::ReviewClipPage, String> {
    let connection = db::open_database_read_only(&state.database_path)?;
    db::list_review_clip_page(&connection, &query)
}

#[tauri::command]
pub fn set_clip_review_decision(
    state: State<'_, AppState>,
    clip_id: i64,
    decision: db::ReviewDecision,
) -> Result<db::ClipReviewMutationResult, String> {
    set_clip_review_decision_for_database(&state.database_path, clip_id, decision)
}

#[tauri::command]
pub fn reset_clip_review_decision(
    state: State<'_, AppState>,
    clip_id: i64,
) -> Result<db::ClipReviewMutationResult, String> {
    reset_clip_review_decision_for_database(&state.database_path, clip_id)
}

#[tauri::command]
pub fn restore_clip_review_state(
    state: State<'_, AppState>,
    clip_id: i64,
    expected_current: db::ClipReviewState,
    restore: db::ClipReviewState,
) -> Result<db::ClipReviewMutationResult, String> {
    restore_clip_review_state_for_database(&state.database_path, clip_id, expected_current, restore)
}

#[tauri::command]
pub fn get_library_facets(state: State<'_, AppState>) -> Result<db::LibraryFacets, String> {
    let connection = db::open_database_read_only(&state.database_path)?;
    db::get_library_facets(&connection)
}

#[tauri::command]
pub fn get_clip_detail(
    state: State<'_, AppState>,
    clip_id: i64,
) -> Result<db::ClipDetail, ClipDetailCommandError> {
    get_clip_detail_for_database(&state.database_path, clip_id)
}

pub(crate) fn get_clip_detail_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
) -> Result<db::ClipDetail, ClipDetailCommandError> {
    let connection =
        db::open_database_read_only(database_path).map_err(|message| ClipDetailCommandError {
            code: "database-error",
            message,
            clip_id,
        })?;
    db::find_clip_detail_by_id(&connection, clip_id)
        .map_err(|message| ClipDetailCommandError {
            code: "database-error",
            message,
            clip_id,
        })?
        .ok_or_else(|| ClipDetailCommandError {
            code: "clip-not-found",
            message: format!("Clip id {clip_id} was not found"),
            clip_id,
        })
}

#[tauri::command]
pub fn list_sources(state: State<'_, AppState>) -> Result<Vec<db::Source>, String> {
    let connection = db::open_database_read_only(&state.database_path)?;
    db::list_sources(&connection)
}

#[tauri::command]
pub fn list_tags(state: State<'_, AppState>) -> Result<Vec<db::Tag>, String> {
    list_tags_for_database(&state.database_path)
}

#[tauri::command]
pub fn create_tag(
    state: State<'_, AppState>,
    name: String,
    color: Option<String>,
) -> Result<db::Tag, String> {
    create_tag_for_database(&state.database_path, &name, color.as_deref())
}

#[tauri::command]
pub fn update_tag(
    state: State<'_, AppState>,
    tag_id: i64,
    name: String,
    color: Option<String>,
) -> Result<db::Tag, String> {
    update_tag_for_database(&state.database_path, tag_id, &name, color.as_deref())
}

#[tauri::command]
pub fn delete_tag(state: State<'_, AppState>, tag_id: i64) -> Result<(), String> {
    delete_tag_for_database(&state.database_path, tag_id)
}

#[tauri::command]
pub fn set_clip_favorite(
    state: State<'_, AppState>,
    clip_id: i64,
    is_favorite: bool,
) -> Result<db::Clip, String> {
    set_clip_favorite_for_database(&state.database_path, clip_id, is_favorite)
}

#[tauri::command]
pub fn set_clips_favorite(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
    is_favorite: bool,
) -> Result<db::BatchClipMutationResult, String> {
    set_clips_favorite_for_database(&state.database_path, &clip_ids, is_favorite)
}

#[tauri::command]
pub fn set_clip_trashed(
    state: State<'_, AppState>,
    clip_id: i64,
    is_trashed: bool,
) -> Result<db::Clip, String> {
    set_clip_trashed_for_database(&state.database_path, clip_id, is_trashed)
}

#[tauri::command]
pub fn set_clips_trashed(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
    is_trashed: bool,
) -> Result<db::BatchClipMutationResult, String> {
    set_clips_trashed_for_database(&state.database_path, &clip_ids, is_trashed)
}

#[tauri::command]
pub fn remove_clip_from_index(state: State<'_, AppState>, clip_id: i64) -> Result<(), String> {
    remove_clip_from_index_for_database(&state.database_path, clip_id)?;
    // The clip row cascades its thumbnail index row. Wake the existing reconciler so its bounded
    // cache maintainer can discard the now-unreferenced generated JPEG; it never touches videos.
    state.thumbnail_queue.reconcile_and_wake();
    Ok(())
}

#[tauri::command]
pub fn remove_clips_from_index(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
) -> Result<RemoveClipsFromIndexResult, String> {
    let result = remove_clips_from_index_for_database(&state.database_path, &clip_ids)?;
    if !result.removed_ids.is_empty() {
        state.thumbnail_queue.reconcile_and_wake();
    }
    Ok(result)
}

#[tauri::command]
pub fn delete_clips_permanently(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
) -> Result<PermanentDeleteResult, String> {
    let _critical_task = state
        .critical_tasks
        .enter(CriticalTaskKind::PermanentDelete)
        .map_err(str::to_string)?;
    let result = delete_clips_permanently_for_database(&state.database_path, &clip_ids)?;
    if !result.deleted_ids.is_empty() {
        state.thumbnail_queue.reconcile_and_wake();
    }
    Ok(result)
}

#[tauri::command]
pub fn update_clip_note(
    state: State<'_, AppState>,
    clip_id: i64,
    note: String,
) -> Result<db::Clip, String> {
    update_clip_note_for_database(&state.database_path, clip_id, &note)
}

#[tauri::command]
pub fn add_tag_to_clip(
    state: State<'_, AppState>,
    clip_id: i64,
    tag_id: i64,
) -> Result<db::Clip, String> {
    add_tag_to_clip_for_database(&state.database_path, clip_id, tag_id)
}

#[tauri::command]
pub fn add_tag_to_clips(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
    tag_id: i64,
) -> Result<db::BatchClipMutationResult, String> {
    add_tag_to_clips_for_database(&state.database_path, &clip_ids, tag_id)
}

#[tauri::command]
pub fn remove_tag_from_clip(
    state: State<'_, AppState>,
    clip_id: i64,
    tag_id: i64,
) -> Result<db::Clip, String> {
    remove_tag_from_clip_for_database(&state.database_path, clip_id, tag_id)
}

#[tauri::command]
pub fn remove_tag_from_clips(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
    tag_id: i64,
) -> Result<db::BatchClipMutationResult, String> {
    remove_tag_from_clips_for_database(&state.database_path, &clip_ids, tag_id)
}

#[tauri::command]
pub fn get_clip_media(
    state: State<'_, AppState>,
    clip_id: i64,
) -> Result<ClipMediaResponse, String> {
    get_clip_media_for_database(&state.database_path, clip_id)
}

#[tauri::command]
pub fn open_clip_location(state: State<'_, AppState>, clip_id: i64) -> Result<(), String> {
    open_clip_location_for_database(&state.database_path, clip_id, reveal_clip_in_explorer)
}

#[tauri::command]
pub fn open_clip_externally(state: State<'_, AppState>, clip_id: i64) -> Result<(), String> {
    open_clip_externally_for_database(&state.database_path, clip_id, open_with_default_player)
}

#[tauri::command]
pub fn copy_clip_path(state: State<'_, AppState>, clip_id: i64) -> Result<String, String> {
    copy_clip_path_for_database(&state.database_path, clip_id)
}

pub(crate) fn open_clip_location_for_database<F>(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    reveal: F,
) -> Result<(), String>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    let target = load_clip_file_target(database_path, clip_id)?;
    ensure_indexed_mp4(&target)?;
    let shell_path = resolve_safe_clip_shell_path(&target)?;
    reveal(&shell_path)
}

pub(crate) fn copy_clip_path_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
) -> Result<String, String> {
    let connection = db::open_database_read_only(database_path)?;
    let clip = db::find_clip_media_paths_by_id(&connection, clip_id)?;

    Ok(clip.video_path)
}

pub(crate) fn open_clip_externally_for_database<F>(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    open: F,
) -> Result<(), String>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    let target = load_clip_file_target(database_path, clip_id)?;
    if target.file_status != "available" {
        return Err("只有可用素材才能交给系统播放器".to_string());
    }
    ensure_indexed_mp4(&target)?;
    let shell_path = resolve_safe_clip_shell_path(&target)?;
    open(&shell_path)
}

fn load_clip_file_target(
    database_path: impl AsRef<Path>,
    clip_id: i64,
) -> Result<db::ClipFileTarget, String> {
    let connection = db::open_database_read_only(database_path)?;
    db::find_clip_file_target_by_id(&connection, clip_id)?
        .ok_or_else(|| format!("素材不存在：{clip_id}"))
}

fn ensure_indexed_mp4(target: &db::ClipFileTarget) -> Result<(), String> {
    if target.extension.eq_ignore_ascii_case("mp4")
        && Path::new(&target.video_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        return Ok(());
    }

    Err("文件操作仅允许已索引的 MP4 文件".to_string())
}

fn resolve_safe_clip_shell_path(target: &db::ClipFileTarget) -> Result<PathBuf, String> {
    let source_root = Path::new(&target.source_dir_path);
    let clip_path = Path::new(&target.video_path);

    if source_root
        .components()
        .chain(clip_path.components())
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("视频路径包含不允许的父目录跳转".to_string());
    }

    let source_metadata = std::fs::symlink_metadata(source_root)
        .map_err(|error| format!("来源目录不可用：{error}"))?;
    if !source_metadata.is_dir() || metadata_is_reparse_point(&source_metadata) {
        return Err("来源目录不可用或已变为 reparse point".to_string());
    }
    let clip_metadata =
        std::fs::symlink_metadata(clip_path).map_err(|_| FILE_NOT_FOUND_MESSAGE.to_string())?;
    if !clip_metadata.is_file() || metadata_is_reparse_point(&clip_metadata) {
        return Err("视频路径不是普通文件".to_string());
    }
    let canonical_root = source_root
        .canonicalize()
        .map_err(|error| format!("无法验证来源目录：{error}"))?;
    let canonical_clip = clip_path
        .canonicalize()
        .map_err(|_| FILE_NOT_FOUND_MESSAGE.to_string())?;
    if canonical_clip == canonical_root || !canonical_clip.starts_with(&canonical_root) {
        return Err("视频路径已越出已授权来源目录".to_string());
    }
    // Inspect the stored spelling rather than only the canonical result: canonicalization resolves
    // ancestor junctions/symlinks and would otherwise hide the reparse point from this walk.
    ensure_non_reparse_path_chain(clip_path, source_root)?;

    Ok(shell_compatible_path(&canonical_clip))
}

pub(crate) fn list_tags_for_database(
    database_path: impl AsRef<Path>,
) -> Result<Vec<db::Tag>, String> {
    let connection = db::open_database_read_only(database_path)?;
    db::list_tags(&connection)
}

pub(crate) fn create_tag_for_database(
    database_path: impl AsRef<Path>,
    name: &str,
    color: Option<&str>,
) -> Result<db::Tag, String> {
    let connection = db::open_database(database_path)?;
    db::create_tag(&connection, name, color)
}

pub(crate) fn update_tag_for_database(
    database_path: impl AsRef<Path>,
    tag_id: i64,
    name: &str,
    color: Option<&str>,
) -> Result<db::Tag, String> {
    let connection = db::open_database(database_path)?;
    db::update_tag(&connection, tag_id, name, color)
}

pub(crate) fn delete_tag_for_database(
    database_path: impl AsRef<Path>,
    tag_id: i64,
) -> Result<(), String> {
    let connection = db::open_database(database_path)?;
    db::delete_tag(&connection, tag_id)
}

pub(crate) fn set_clip_favorite_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    is_favorite: bool,
) -> Result<db::Clip, String> {
    let result = set_clips_favorite_for_database(database_path, &[clip_id], is_favorite)?;
    single_clip_from_batch(result, "updating favorite", clip_id)
}

pub(crate) fn set_clip_review_decision_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    decision: db::ReviewDecision,
) -> Result<db::ClipReviewMutationResult, String> {
    let connection = db::open_database(database_path)?;
    db::set_clip_review_decision(&connection, clip_id, decision)
}

pub(crate) fn reset_clip_review_decision_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
) -> Result<db::ClipReviewMutationResult, String> {
    let connection = db::open_database(database_path)?;
    db::reset_clip_review_decision(&connection, clip_id)
}

pub(crate) fn restore_clip_review_state_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    expected_current: db::ClipReviewState,
    restore: db::ClipReviewState,
) -> Result<db::ClipReviewMutationResult, String> {
    if expected_current.clip_id != clip_id || restore.clip_id != clip_id {
        return Err("clip review restore state belongs to a different clip".to_string());
    }
    let connection = db::open_database(database_path)?;
    db::restore_clip_review_state(&connection, &expected_current, &restore)
}

pub(crate) fn set_clips_favorite_for_database(
    database_path: impl AsRef<Path>,
    clip_ids: &[i64],
    is_favorite: bool,
) -> Result<db::BatchClipMutationResult, String> {
    let connection = db::open_database(database_path)?;
    db::set_clips_favorite(&connection, clip_ids, is_favorite)
}

pub(crate) fn set_clip_trashed_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    is_trashed: bool,
) -> Result<db::Clip, String> {
    let result = set_clips_trashed_for_database(database_path, &[clip_id], is_trashed)?;
    single_clip_from_batch(result, "updating recycle-bin state", clip_id)
}

pub(crate) fn set_clips_trashed_for_database(
    database_path: impl AsRef<Path>,
    clip_ids: &[i64],
    is_trashed: bool,
) -> Result<db::BatchClipMutationResult, String> {
    let connection = db::open_database(database_path)?;
    db::set_clips_trashed_guarded(&connection, clip_ids, is_trashed)
}

pub(crate) fn remove_clip_from_index_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
) -> Result<(), String> {
    let connection = db::open_database(database_path)?;
    match db::delete_clip_from_index_guarded(&connection, clip_id)? {
        db::ClipIndexRemovalOutcome::Removed => Ok(()),
        db::ClipIndexRemovalOutcome::Missing => {
            Err(format!("clip-not-found: 素材 {clip_id} 不存在"))
        }
        db::ClipIndexRemovalOutcome::Blocked(problem) => {
            Err(format!("{}: {}", problem.code, problem.message))
        }
    }
}

pub(crate) fn remove_clips_from_index_for_database(
    database_path: impl AsRef<Path>,
    clip_ids: &[i64],
) -> Result<RemoveClipsFromIndexResult, String> {
    let connection = db::open_database(database_path)?;
    let mut seen = HashSet::with_capacity(clip_ids.len());
    let unique_clip_ids = clip_ids
        .iter()
        .copied()
        .filter(|clip_id| seen.insert(*clip_id))
        .collect::<Vec<_>>();
    if unique_clip_ids.len() > MAX_INDEX_REMOVAL_CLIP_IDS {
        return Err(format!(
            "仅移除索引命令最多接受 {MAX_INDEX_REMOVAL_CLIP_IDS} 个不同的素材 ID"
        ));
    }

    let mut result = RemoveClipsFromIndexResult {
        requested: unique_clip_ids.len(),
        ..RemoveClipsFromIndexResult::default()
    };
    for clip_id in unique_clip_ids {
        match db::delete_clip_from_index_guarded(&connection, clip_id) {
            Ok(db::ClipIndexRemovalOutcome::Removed) => result.removed_ids.push(clip_id),
            Ok(db::ClipIndexRemovalOutcome::Missing) => result.missing_ids.push(clip_id),
            Ok(db::ClipIndexRemovalOutcome::Blocked(problem)) => {
                result.blocked.push(IndexRemovalProblem {
                    clip_id,
                    code: problem.code,
                    message: problem.message,
                });
            }
            Err(message) => result.failures.push(IndexRemovalProblem {
                clip_id,
                code: "database-error".to_string(),
                message,
            }),
        }
    }
    Ok(result)
}

pub(crate) fn delete_clips_permanently_for_database(
    database_path: impl AsRef<Path>,
    clip_ids: &[i64],
) -> Result<PermanentDeleteResult, String> {
    let connection = db::open_database(database_path)?;
    let mut seen = HashSet::with_capacity(clip_ids.len());
    let unique_clip_ids = clip_ids
        .iter()
        .copied()
        .filter(|clip_id| seen.insert(*clip_id))
        .collect::<Vec<_>>();
    let mut result = PermanentDeleteResult {
        requested: unique_clip_ids.len(),
        deleted_ids: Vec::new(),
        missing_ids: Vec::new(),
        pending_ids: Vec::new(),
        blocked: Vec::new(),
        failures: Vec::new(),
    };

    for clip_id in unique_clip_ids {
        match db::delete_clip_permanently(&connection, clip_id) {
            Ok(db::ClipDeleteItemOutcome::Deleted) => result.deleted_ids.push(clip_id),
            Ok(db::ClipDeleteItemOutcome::Missing) => result.missing_ids.push(clip_id),
            Ok(db::ClipDeleteItemOutcome::Pending(_)) => result.pending_ids.push(clip_id),
            Ok(db::ClipDeleteItemOutcome::Blocked(problem)) => {
                result.blocked.push(PermanentDeleteFailure {
                    clip_id,
                    code: problem.code,
                    retryable: problem.retryable,
                    message: problem.message,
                });
            }
            Ok(db::ClipDeleteItemOutcome::Rejected(problem)) => {
                result.failures.push(PermanentDeleteFailure {
                    clip_id,
                    code: problem.code,
                    retryable: problem.retryable,
                    message: problem.message,
                });
            }
            Err(message) => result.failures.push(PermanentDeleteFailure {
                clip_id,
                code: "database-error".to_string(),
                retryable: true,
                message,
            }),
        }
    }

    Ok(result)
}

pub(crate) fn update_clip_note_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    note: &str,
) -> Result<db::Clip, String> {
    let connection = db::open_database(database_path)?;
    db::update_clip_note(&connection, clip_id, Some(note))?;
    db::find_clip_by_id(&connection, clip_id)
}

pub(crate) fn add_tag_to_clip_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    tag_id: i64,
) -> Result<db::Clip, String> {
    let result = add_tag_to_clips_for_database(database_path, &[clip_id], tag_id)?;
    single_clip_from_batch(result, "assigning tag to clip", clip_id)
}

pub(crate) fn add_tag_to_clips_for_database(
    database_path: impl AsRef<Path>,
    clip_ids: &[i64],
    tag_id: i64,
) -> Result<db::BatchClipMutationResult, String> {
    let connection = db::open_database(database_path)?;
    db::add_tag_to_clips(&connection, clip_ids, tag_id)
}

pub(crate) fn remove_tag_from_clip_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
    tag_id: i64,
) -> Result<db::Clip, String> {
    let result = remove_tag_from_clips_for_database(database_path, &[clip_id], tag_id)?;
    single_clip_from_batch(result, "removing clip tag", clip_id)
}

pub(crate) fn remove_tag_from_clips_for_database(
    database_path: impl AsRef<Path>,
    clip_ids: &[i64],
    tag_id: i64,
) -> Result<db::BatchClipMutationResult, String> {
    let connection = db::open_database(database_path)?;
    db::remove_tag_from_clips(&connection, clip_ids, tag_id)
}

fn single_clip_from_batch(
    result: db::BatchClipMutationResult,
    action: &str,
    clip_id: i64,
) -> Result<db::Clip, String> {
    result
        .clips
        .into_iter()
        .next()
        .ok_or_else(|| format!("{action} failed: clip id {clip_id} was not found"))
}

fn reveal_clip_in_explorer(clip_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::ptr;
        use windows_sys::Win32::UI::Shell::{
            Common::ITEMIDLIST, SHOpenFolderAndSelectItems, SHParseDisplayName,
        };

        let _com = ComInitialization::initialize()?;
        let display_name = wide_null_terminated(clip_path.as_os_str());
        let mut raw_pidl: *mut ITEMIDLIST = ptr::null_mut();
        let parse_result = unsafe {
            SHParseDisplayName(
                display_name.as_ptr(),
                ptr::null_mut(),
                &mut raw_pidl,
                0,
                ptr::null_mut(),
            )
        };
        let pidl = OwnedPidl(raw_pidl);
        if parse_result < 0 {
            return Err(hresult_error("无法解析视频文件位置", parse_result));
        }
        if pidl.0.is_null() {
            return Err("无法解析视频文件位置：Shell 未返回项目标识".to_string());
        }

        // With cidl == 0, the fully-qualified PIDL names the single item to reveal. The Shell
        // opens its parent folder and selects that item, returning an HRESULT we can report.
        let reveal_result = unsafe { SHOpenFolderAndSelectItems(pidl.0, 0, ptr::null(), 0) };
        if reveal_result < 0 {
            return Err(hresult_error("文件资源管理器定位失败", reveal_result));
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = clip_path;
        Err("打开文件位置仅支持 Windows 文件资源管理器".to_string())
    }
}

/// Verifies `video_path` is a real MP4 file still contained in `source_root`, rejecting parent
/// traversal, non-files and any reparse point along the chain. Shared by indexed-clip shell
/// resolution and pending-recording preview so both enforce one boundary contract.
pub(super) fn ensure_mp4_inside_source_root(
    video_path: &str,
    source_dir_path: &str,
    extension: &str,
) -> Result<PathBuf, String> {
    if !extension.eq_ignore_ascii_case("mp4") {
        return Err("仅支持预览 MP4 文件".to_string());
    }

    let source_root = Path::new(source_dir_path);
    let clip_path = Path::new(video_path);

    if !clip_path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("mp4"))
    {
        return Err("仅支持预览 MP4 文件".to_string());
    }

    if source_root
        .components()
        .chain(clip_path.components())
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("视频路径包含不允许的父目录跳转".to_string());
    }

    let source_metadata = std::fs::symlink_metadata(source_root)
        .map_err(|error| format!("来源目录不可用：{error}"))?;
    if !source_metadata.is_dir() || metadata_is_reparse_point(&source_metadata) {
        return Err("来源目录不可用或已变为 reparse point".to_string());
    }
    let clip_metadata =
        std::fs::symlink_metadata(clip_path).map_err(|_| FILE_NOT_FOUND_MESSAGE.to_string())?;
    if !clip_metadata.is_file() || metadata_is_reparse_point(&clip_metadata) {
        return Err("视频路径不是普通文件".to_string());
    }

    let canonical_root = source_root
        .canonicalize()
        .map_err(|error| format!("无法验证来源目录：{error}"))?;
    let canonical_clip = clip_path
        .canonicalize()
        .map_err(|_| FILE_NOT_FOUND_MESSAGE.to_string())?;
    if canonical_clip == canonical_root || !canonical_clip.starts_with(&canonical_root) {
        return Err("视频路径已越出已授权来源目录".to_string());
    }
    ensure_non_reparse_path_chain(clip_path, source_root)?;

    Ok(canonical_clip)
}

pub(super) fn ensure_non_reparse_path_chain(path: &Path, root: &Path) -> Result<(), String> {
    let mut cursor = Some(path);
    while let Some(current) = cursor {
        let metadata = std::fs::symlink_metadata(current)
            .map_err(|error| format!("无法验证视频路径：{error}"))?;
        if metadata_is_reparse_point(&metadata) {
            return Err("视频路径链包含符号链接或 reparse point".to_string());
        }
        if paths_refer_to_same_spelling(current, root) {
            return Ok(());
        }
        cursor = current.parent();
    }
    Err("视频路径不属于已授权来源目录".to_string())
}

fn paths_refer_to_same_spelling(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    #[cfg(windows)]
    {
        db::normalize_path(left.to_string_lossy().as_ref())
            == db::normalize_path(right.to_string_lossy().as_ref())
    }

    #[cfg(not(windows))]
    {
        left == right
    }
}

fn shell_compatible_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::{
            ffi::OsString,
            os::windows::ffi::{OsStrExt, OsStringExt},
        };

        let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let is_verbatim =
            wide.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]);
        if !is_verbatim {
            return path.to_path_buf();
        }

        let is_verbatim_unc = wide.len() >= 8
            && matches!(wide[4], value if value == b'U' as u16 || value == b'u' as u16)
            && matches!(wide[5], value if value == b'N' as u16 || value == b'n' as u16)
            && matches!(wide[6], value if value == b'C' as u16 || value == b'c' as u16)
            && matches!(wide[7], value if value == b'\\' as u16 || value == b'/' as u16);
        let ordinary = if is_verbatim_unc {
            let mut ordinary = Vec::with_capacity(wide.len().saturating_sub(6));
            ordinary.extend([b'\\' as u16, b'\\' as u16]);
            ordinary.extend_from_slice(&wide[8..]);
            ordinary
        } else {
            let is_verbatim_drive = wide.len() >= 7
                && wide[4] <= u16::from(u8::MAX)
                && (wide[4] as u8).is_ascii_alphabetic()
                && wide[5] == b':' as u16
                && matches!(wide[6], value if value == b'\\' as u16 || value == b'/' as u16);
            if !is_verbatim_drive {
                return path.to_path_buf();
            }
            wide[4..].to_vec()
        };

        PathBuf::from(OsString::from_wide(&ordinary))
    }

    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

#[cfg(windows)]
fn wide_null_terminated(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn hresult_error(action: &str, result: windows_sys::core::HRESULT) -> String {
    format!("{action}（HRESULT=0x{:08X}）", result as u32)
}

#[cfg(windows)]
struct OwnedPidl(*mut windows_sys::Win32::UI::Shell::Common::ITEMIDLIST);

#[cfg(windows)]
impl Drop for OwnedPidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::System::Com::CoTaskMemFree(self.0.cast());
            }
        }
    }
}

#[cfg(windows)]
struct ComInitialization {
    should_uninitialize: bool,
}

#[cfg(windows)]
impl ComInitialization {
    fn initialize() -> Result<Self, String> {
        use std::ptr;
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

        const RPC_E_CHANGED_MODE: windows_sys::core::HRESULT = 0x80010106_u32 as i32;
        let result = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if result >= 0 {
            return Ok(Self {
                should_uninitialize: true,
            });
        }
        if result == RPC_E_CHANGED_MODE {
            // COM is already initialized on this thread with another concurrency model. It remains
            // valid for Shell calls, but this function must not balance an initialization it did
            // not perform.
            return Ok(Self {
                should_uninitialize: false,
            });
        }
        Err(hresult_error("初始化 Windows Shell 失败", result))
    }
}

#[cfg(windows)]
impl Drop for ComInitialization {
    fn drop(&mut self) {
        if self.should_uninitialize {
            unsafe {
                windows_sys::Win32::System::Com::CoUninitialize();
            }
        }
    }
}

pub(super) fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }

    #[cfg(not(windows))]
    false
}

fn open_with_default_player(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

        let operation = OsStr::new("open")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let file = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(format!(
                "系统默认播放器启动失败（ShellExecuteW={result:?}）"
            ));
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("系统默认播放器回退当前仅支持 Windows".to_string())
    }
}

#[cfg(all(test, windows))]
mod windows_shell_tests {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::Path};

    use super::{shell_compatible_path, wide_null_terminated};

    #[test]
    fn shell_path_conversion_preserves_unicode_and_removes_only_supported_verbatim_prefixes() {
        assert_eq!(
            shell_compatible_path(Path::new(r"\\?\D:\录像 空格.mp4")),
            Path::new(r"D:\录像 空格.mp4")
        );
        assert_eq!(
            shell_compatible_path(Path::new(r"\\?\UNC\服务器\共享\录像 空格.mp4")),
            Path::new(r"\\服务器\共享\录像 空格.mp4")
        );
        assert_eq!(
            shell_compatible_path(Path::new(
                r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\录像 空格.mp4"
            )),
            Path::new(r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\录像 空格.mp4")
        );
    }

    #[test]
    fn shell_display_name_is_null_terminated_without_losing_unicode_or_spaces() {
        let path = Path::new(r"D:\素材 库\录像 空格.mp4");
        let encoded = wide_null_terminated(path.as_os_str());

        assert_eq!(encoded.last(), Some(&0));
        assert_eq!(
            OsString::from_wide(&encoded[..encoded.len() - 1]),
            path.as_os_str()
        );
    }
}
