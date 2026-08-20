//! NVIDIA pending-recording queue commands: list, ignore and manual classification import.

use tauri::State;

use crate::{db, AppState};

/// Stable contract for a successful manual import; the hydrated clip lets the caller surface
/// the new library card immediately.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualClipImportResult {
    pub clip: db::Clip,
}

#[tauri::command]
pub fn list_pending_manual_clips(
    state: State<'_, AppState>,
    include_ignored: bool,
) -> Result<Vec<db::PendingManualClip>, String> {
    let connection = db::open_database_read_only(&state.database_path)?;
    db::list_pending_manual_clips(&connection, include_ignored)
}

#[tauri::command]
pub fn import_pending_manual_clip(
    state: State<'_, AppState>,
    pending_id: i64,
    input: db::ManualClipImportInput,
) -> Result<ManualClipImportResult, String> {
    let connection = db::open_database(&state.database_path)?;
    let clip_id = db::import_pending_manual_clip(&connection, pending_id, &input)?;
    let clip = db::find_clip_by_id(&connection, clip_id)?;
    state.thumbnail_queue.reconcile_and_wake();
    Ok(ManualClipImportResult { clip })
}

#[tauri::command]
pub fn set_pending_manual_clip_ignored(
    state: State<'_, AppState>,
    pending_id: i64,
    ignored: bool,
) -> Result<(), String> {
    let connection = db::open_database(&state.database_path)?;
    db::set_pending_manual_clip_ignored(&connection, pending_id, ignored)
}
