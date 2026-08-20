mod export;
mod feedback;
mod library;
mod manual_import;
mod media_protocol;
mod sources;

use std::{
    cell::Cell,
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[cfg(test)]
use std::{cell::RefCell, collections::HashMap};

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

pub use export::*;
pub use feedback::*;
pub use library::*;
pub use manual_import::*;
pub use media_protocol::clip_media_protocol_response;
pub use sources::*;

#[cfg(test)]
use media_protocol::{
    clip_id_from_media_request, get_clip_media_for_database, parse_media_range, ByteRange,
    MAX_MEDIA_CHUNK_BYTES,
};

use crate::{
    critical_tasks::{CriticalTaskGate, CriticalTaskKind},
    db,
    drive_discovery::{self, DiscoveryProgress},
    scan_coordinator::{ScanCoordinator, ScanJobStatus, ScanProgressEvent, ScanStatus},
    scanner,
    thumbnail::{ThumbnailQueue, ThumbnailServiceStatus},
    AppState,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResponse {
    ok: bool,
    product_name: &'static str,
    backend: &'static str,
    database: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FullDriveScanResult {
    pub fixed_drive_count: u64,
    pub visited_directory_count: u64,
    pub validated_source_dir_count: u64,
    pub scan_root_count: u64,
    pub skipped_directory_count: u64,
    pub discovery_warnings: Vec<String>,
    pub scanned_clip_count: i64,
    pub scan_summary: scanner::ScanSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanJobResult<T> {
    pub job_id: String,
    pub status: ScanJobStatus,
    pub result: Option<T>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanCommandError {
    pub code: &'static str,
    pub message: String,
    pub job_id: Option<String>,
    pub active_job_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelScanResult {
    pub accepted: bool,
    pub reason: String,
    pub job_id: String,
    pub active_job_id: Option<String>,
    pub status: ScanJobStatus,
    pub message: String,
}

struct ScanWorkResult<T> {
    status: ScanJobStatus,
    result: Option<T>,
    message: String,
    /// Whether the work callback already committed this terminal state to `scan_runs`.
    /// Scanner executions only return success after that commit; pre-scan cancellation is the
    /// exceptional success path that still needs the outer fallback write.
    scan_run_terminal_persisted: bool,
}

#[derive(Default)]
struct ScanEventCursor {
    last_processed: u64,
    terminal_sent: bool,
}

impl ScanEventCursor {
    fn advance(
        &mut self,
        raw_processed: u64,
        raw_total: Option<u64>,
        terminal: bool,
    ) -> Option<(u64, Option<u64>)> {
        if terminal && self.terminal_sent {
            return None;
        }
        self.last_processed = self.last_processed.max(raw_processed);
        if terminal {
            self.terminal_sent = true;
        }
        let total = raw_total.filter(|total| *total >= self.last_processed);
        Some((self.last_processed, total))
    }
}

#[derive(Clone)]
struct ScanEventDispatcher {
    app: AppHandle,
    coordinator: Arc<ScanCoordinator>,
    cancellation: Arc<AtomicBool>,
    job_id: String,
    cursor: Arc<Mutex<ScanEventCursor>>,
}

impl ScanEventDispatcher {
    fn new(
        app: AppHandle,
        coordinator: Arc<ScanCoordinator>,
        cancellation: Arc<AtomicBool>,
        job_id: String,
    ) -> Self {
        Self {
            app,
            coordinator,
            cancellation,
            job_id,
            cursor: Arc::new(Mutex::new(ScanEventCursor::default())),
        }
    }

    fn starting(&self, root_hint: &str) {
        self.emit(
            "starting",
            Some(root_hint.to_string()),
            None,
            0,
            None,
            "正在启动扫描任务".to_string(),
            ScanJobStatus::Running,
            false,
            0,
            0,
            0,
        );
    }

    fn scanner_progress(&self, progress: scanner::ScanProgress) {
        if matches!(
            progress.phase.as_str(),
            "completed" | "partial" | "cancelled" | "failed"
        ) {
            return;
        }
        let status = if self.cancellation.load(Ordering::Acquire) {
            ScanJobStatus::Cancelling
        } else {
            ScanJobStatus::Running
        };
        self.emit(
            &progress.phase,
            (!progress.root_path.is_empty()).then_some(progress.root_path),
            progress.source,
            non_negative_i64(progress.current),
            (progress.total > 0).then_some(non_negative_i64(progress.total)),
            progress.message,
            status,
            false,
            progress.source_dir_count,
            progress.clip_group_count,
            progress.clip_file_count,
        );
    }

    fn discovery_progress(&self, progress: DiscoveryProgress) {
        let status = if self.cancellation.load(Ordering::Acquire) {
            ScanJobStatus::Cancelling
        } else {
            ScanJobStatus::Running
        };
        self.emit(
            "drive-discovery",
            Some(progress.current_drive),
            None,
            progress.visited_directory_count,
            None,
            progress.message,
            status,
            false,
            u64_to_i64(progress.validated_source_dir_count),
            0,
            0,
        );
    }

    fn terminal(&self, status: ScanJobStatus, message: &str) {
        debug_assert!(status.is_terminal());
        self.emit(
            status.as_str(),
            None,
            None,
            0,
            None,
            message.to_string(),
            status,
            true,
            0,
            0,
            0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        phase: &str,
        current_root: Option<String>,
        source: Option<String>,
        raw_processed: u64,
        raw_total: Option<u64>,
        message: String,
        status: ScanJobStatus,
        terminal: bool,
        source_dir_count: i64,
        clip_group_count: i64,
        clip_file_count: i64,
    ) {
        let (processed, total) = {
            let mut cursor = self
                .cursor
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(progress) = cursor.advance(raw_processed, raw_total, terminal) else {
                return;
            };
            progress
        };
        let event = ScanProgressEvent {
            job_id: self.job_id.clone(),
            phase: phase.to_string(),
            current_root,
            source,
            processed,
            total,
            message,
            terminal,
            status,
            source_dir_count,
            clip_group_count,
            clip_file_count,
        };
        if !self.coordinator.record_event(&event) {
            return;
        }
        emit_scan_progress_ignoring_failure(&event, |event| {
            self.app
                .emit("scan-progress", event)
                .map_err(|error| error.to_string())
        });
    }
}

fn emit_scan_progress_ignoring_failure<F>(event: &ScanProgressEvent, emit: F)
where
    F: FnOnce(&ScanProgressEvent) -> Result<(), String>,
{
    if let Err(error) = emit(event) {
        eprintln!(
            "scan-progress emit failed for job {} during {}: {error}",
            event.job_id, event.phase
        );
    }
}

fn non_negative_i64(value: i64) -> u64 {
    value.max(0) as u64
}

#[tauri::command]
pub fn ping_backend(state: State<'_, AppState>) -> PingResponse {
    PingResponse {
        ok: true,
        product_name: "瓦刻",
        backend: "tauri-rust",
        database: state.database_path.clone(),
    }
}

#[tauri::command]
pub async fn scan_default_aclos_dir(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ScanJobResult<scanner::ScanSummary>, ScanCommandError> {
    let root_hint = scanner::default_aclos_dir().display().to_string();
    run_scan_job(
        app,
        state.database_path.clone(),
        state.scan_coordinator.clone(),
        state.critical_tasks.clone(),
        state.thumbnail_queue.clone(),
        root_hint,
        |connection, job_id, cancellation, events| {
            scanner::scan_default_aclos_library_with_progress_and_cancel(
                connection,
                job_id,
                cancellation,
                |progress| events.scanner_progress(progress),
            )
            .map(scan_execution_work)
        },
    )
    .await
}

#[tauri::command]
pub async fn scan_custom_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ScanJobResult<scanner::ScanSummary>, ScanCommandError> {
    let root_hint = path.clone();
    run_scan_job(
        app,
        state.database_path.clone(),
        state.scan_coordinator.clone(),
        state.critical_tasks.clone(),
        state.thumbnail_queue.clone(),
        root_hint,
        move |connection, job_id, cancellation, events| {
            scanner::scan_custom_directory_with_progress_and_cancel(
                connection,
                path,
                job_id,
                cancellation,
                |progress| events.scanner_progress(progress),
            )
            .map(scan_execution_work)
        },
    )
    .await
}

#[tauri::command]
pub async fn scan_roots(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<ScanJobResult<scanner::ScanSummary>, ScanCommandError> {
    let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    let root_hint = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    run_scan_job(
        app,
        state.database_path.clone(),
        state.scan_coordinator.clone(),
        state.critical_tasks.clone(),
        state.thumbnail_queue.clone(),
        root_hint,
        move |connection, job_id, cancellation, events| {
            scanner::scan_roots_with_progress_and_cancel(
                connection,
                &paths,
                job_id,
                cancellation,
                |progress| events.scanner_progress(progress),
            )
            .map(scan_execution_work)
        },
    )
    .await
}

async fn run_scan_job<T, F>(
    app: AppHandle,
    database_path: String,
    coordinator: Arc<ScanCoordinator>,
    critical_tasks: Arc<CriticalTaskGate>,
    thumbnail_queue: Arc<ThumbnailQueue>,
    root_hint: String,
    work: F,
) -> Result<ScanJobResult<T>, ScanCommandError>
where
    T: Send + 'static,
    F: FnOnce(
            &Connection,
            &str,
            &AtomicBool,
            &ScanEventDispatcher,
        ) -> Result<ScanWorkResult<T>, String>
        + Send
        + 'static,
{
    let _critical_task = critical_tasks
        .enter(CriticalTaskKind::Scan)
        .map_err(|message| ScanCommandError {
            code: "update-installing",
            message: message.to_string(),
            job_id: None,
            active_job_id: None,
        })?;
    let lease = coordinator.begin().map_err(|conflict| ScanCommandError {
        code: "already-running",
        message: format!("已有扫描任务正在运行：{}", conflict.job_id),
        job_id: None,
        active_job_id: Some(conflict.job_id),
    })?;
    let job_id = lease.job_id().to_string();
    let cancellation = lease.cancellation();
    let events = ScanEventDispatcher::new(app, coordinator, cancellation.clone(), job_id.clone());
    events.starting(&root_hint);

    let worker_job_id = job_id.clone();
    let worker_root_hint = root_hint.clone();
    let worker_events = events.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open_database(&database_path)?;
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            scanner::ensure_scan_run_started(&connection, &worker_job_id, &worker_root_hint)?;
            work(
                &connection,
                &worker_job_id,
                cancellation.as_ref(),
                &worker_events,
            )
        }));

        match result {
            Ok(Ok(result)) => {
                ensure_scan_work_terminal(&connection, &worker_job_id, &worker_root_hint, &result)?;
                Ok(result)
            }
            Ok(Err(error)) => {
                let _ = scanner::ensure_scan_run_terminal(
                    &connection,
                    &worker_job_id,
                    &worker_root_hint,
                    "failed",
                    &error,
                );
                Err(error)
            }
            Err(payload) => {
                let _ = scanner::ensure_scan_run_terminal(
                    &connection,
                    &worker_job_id,
                    &worker_root_hint,
                    "failed",
                    "扫描任务发生 panic",
                );
                panic::resume_unwind(payload);
            }
        }
    })
    .await;

    let response = match worker {
        Ok(Ok(result)) => {
            events.terminal(result.status, &result.message);
            let response = ScanJobResult {
                job_id,
                status: result.status,
                result: result.result,
                message: result.message.clone(),
            };
            lease.finish(result.status, &result.message);
            Ok(response)
        }
        Ok(Err(error)) => {
            events.terminal(ScanJobStatus::Failed, &error);
            lease.finish(ScanJobStatus::Failed, &error);
            Err(ScanCommandError {
                code: "scan-failed",
                message: error,
                job_id: Some(job_id),
                active_job_id: None,
            })
        }
        Err(error) => {
            let message = format!("Scan task failed: {error}");
            events.terminal(ScanJobStatus::Failed, &message);
            lease.finish(ScanJobStatus::Failed, &message);
            Err(ScanCommandError {
                code: "scan-failed",
                message,
                job_id: Some(job_id),
                active_job_id: None,
            })
        }
    };
    // Scans may have persisted useful clips even when they end partial, cancelled, failed or
    // panicked. Reconciliation is asynchronous and cannot change the scan result. A coordinator
    // conflict returned above never reaches this point and therefore never wakes the queue.
    thumbnail_queue.reconcile_and_wake();
    response
}

fn ensure_scan_work_terminal<T>(
    connection: &Connection,
    job_id: &str,
    root_hint: &str,
    result: &ScanWorkResult<T>,
) -> Result<(), String> {
    if result.scan_run_terminal_persisted {
        return Ok(());
    }

    scanner::ensure_scan_run_terminal(
        connection,
        job_id,
        root_hint,
        result.status.as_str(),
        &result.message,
    )
}

fn scan_execution_work(execution: scanner::ScanExecution) -> ScanWorkResult<scanner::ScanSummary> {
    let status = match execution.status {
        scanner::ScanExecutionStatus::Completed => ScanJobStatus::Completed,
        scanner::ScanExecutionStatus::Partial => ScanJobStatus::Partial,
        scanner::ScanExecutionStatus::Cancelled => ScanJobStatus::Cancelled,
    };
    let message = execution
        .summary
        .message
        .clone()
        .unwrap_or_else(|| status.as_str().to_string());
    ScanWorkResult {
        status,
        result: Some(execution.summary),
        message,
        scan_run_terminal_persisted: true,
    }
}

#[cfg(test)]
fn discover_and_scan_roots<F, G>(
    connection: &Connection,
    roots: &[PathBuf],
    excluded_roots: &[PathBuf],
    discovery_progress: F,
    scan_progress: G,
) -> Result<FullDriveScanResult, String>
where
    F: Fn(DiscoveryProgress),
    G: Fn(scanner::ScanProgress),
{
    let discovery = drive_discovery::discover_scan_roots_with_exclusions(
        roots,
        excluded_roots,
        discovery_progress,
    );
    if discovery.opened_drive_count == 0 {
        return Err("无法读取任何本机固定磁盘".to_string());
    }

    let scanned_clips_by_root = RefCell::new(HashMap::<String, i64>::new());
    let mut scan_summary = scanner::scan_discovered_aclos_roots_with_progress(
        connection,
        &discovery.scan_roots,
        &discovery.validated_source_dirs,
        |progress| {
            {
                let mut counts = scanned_clips_by_root.borrow_mut();
                let count = counts.entry(progress.root_path.clone()).or_default();
                *count = (*count).max(progress.clip_file_count);
            }
            scan_progress(progress);
        },
    )?;

    scan_summary.merge_errors(discovery.warnings.clone());
    let scanned_clip_count = scanned_clips_by_root
        .borrow()
        .values()
        .copied()
        .fold(0i64, i64::saturating_add);

    Ok(FullDriveScanResult {
        fixed_drive_count: discovery.fixed_drive_count,
        visited_directory_count: discovery.visited_directory_count,
        validated_source_dir_count: discovery.validated_source_dir_count,
        scan_root_count: discovery.scan_roots.len().min(u64::MAX as usize) as u64,
        skipped_directory_count: discovery.skipped_directory_count,
        discovery_warnings: discovery.warnings,
        scanned_clip_count,
        scan_summary,
    })
}

fn discover_and_scan_roots_controlled(
    connection: &Connection,
    roots: &[PathBuf],
    excluded_roots: &[PathBuf],
    job_id: &str,
    cancellation: &AtomicBool,
    events: &ScanEventDispatcher,
) -> Result<ScanWorkResult<FullDriveScanResult>, String> {
    let discovery = drive_discovery::discover_scan_roots_with_exclusions_and_cancel(
        roots,
        excluded_roots,
        |progress| events.discovery_progress(progress),
        || cancellation.load(Ordering::Acquire),
    );
    if discovery.cancelled {
        return Ok(ScanWorkResult {
            status: ScanJobStatus::Cancelled,
            result: None,
            message: "已取消全电脑发现".to_string(),
            scan_run_terminal_persisted: false,
        });
    }
    if discovery.opened_drive_count == 0 {
        return Err("无法读取任何本机固定磁盘".to_string());
    }

    let scanned_clip_count = Cell::new(0i64);
    let execution = scanner::scan_discovered_aclos_roots_with_progress_and_cancel(
        connection,
        &discovery.scan_roots,
        &discovery.validated_source_dirs,
        job_id,
        cancellation,
        |progress| {
            scanned_clip_count.set(scanned_clip_count.get().max(progress.clip_file_count));
            events.scanner_progress(progress);
        },
    )?;
    let mut scan_summary = execution.summary;
    scan_summary.merge_errors(discovery.warnings.clone());

    let status = match execution.status {
        scanner::ScanExecutionStatus::Cancelled => ScanJobStatus::Cancelled,
        _ if scan_summary.errors.is_empty() => ScanJobStatus::Completed,
        _ => ScanJobStatus::Partial,
    };
    let persistence_status = match status {
        ScanJobStatus::Completed => scanner::ScanExecutionStatus::Completed,
        ScanJobStatus::Partial => scanner::ScanExecutionStatus::Partial,
        ScanJobStatus::Cancelled => scanner::ScanExecutionStatus::Cancelled,
        _ => unreachable!("controlled discovery only returns terminal scan states"),
    };
    scanner::finalize_scan_run_for_job(connection, job_id, persistence_status, &scan_summary)?;

    let message = scan_summary
        .message
        .clone()
        .unwrap_or_else(|| status.as_str().to_string());
    Ok(ScanWorkResult {
        status,
        result: Some(FullDriveScanResult {
            fixed_drive_count: discovery.fixed_drive_count,
            visited_directory_count: discovery.visited_directory_count,
            validated_source_dir_count: discovery.validated_source_dir_count,
            scan_root_count: discovery.scan_roots.len().min(u64::MAX as usize) as u64,
            skipped_directory_count: discovery.skipped_directory_count,
            discovery_warnings: discovery.warnings,
            scanned_clip_count: scanned_clip_count.get(),
            scan_summary,
        }),
        message,
        scan_run_terminal_persisted: true,
    })
}

#[tauri::command]
pub async fn discover_and_scan_fixed_drives(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ScanJobResult<FullDriveScanResult>, ScanCommandError> {
    run_scan_job(
        app,
        state.database_path.clone(),
        state.scan_coordinator.clone(),
        state.critical_tasks.clone(),
        state.thumbnail_queue.clone(),
        "本机固定磁盘".to_string(),
        |connection, job_id, cancellation, events| {
            let roots = drive_discovery::fixed_drive_roots()?;
            if roots.is_empty() {
                return Err("未发现本机固定磁盘".to_string());
            }

            let excluded_roots = drive_discovery::fixed_drive_exclusion_roots();
            discover_and_scan_roots_controlled(
                connection,
                &roots,
                &excluded_roots,
                job_id,
                cancellation,
                events,
            )
        },
    )
    .await
}

fn u64_to_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

#[tauri::command]
pub fn get_scan_status(state: State<'_, AppState>) -> ScanStatus {
    state.scan_coordinator.status()
}

#[tauri::command]
pub fn cancel_scan(app: AppHandle, state: State<'_, AppState>, job_id: String) -> CancelScanResult {
    let outcome = state.scan_coordinator.request_cancel(&job_id);
    if outcome.accepted {
        match db::open_database(&state.database_path).and_then(|connection| {
            scanner::mark_scan_run_cancelling(&connection, &job_id).map(|_| ())
        }) {
            Ok(()) => {}
            Err(error) => eprintln!("Failed to persist cancelling state for {job_id}: {error}"),
        }

        let event = ScanProgressEvent {
            job_id: job_id.clone(),
            phase: "cancelling".to_string(),
            current_root: outcome.status.current_root.clone(),
            source: outcome.status.source.clone(),
            processed: outcome.status.processed,
            total: outcome.status.total,
            message: outcome.status.message.clone(),
            terminal: false,
            status: ScanJobStatus::Cancelling,
            source_dir_count: 0,
            clip_group_count: 0,
            clip_file_count: 0,
        };
        emit_scan_progress_ignoring_failure(&event, |event| {
            app.emit("scan-progress", event)
                .map_err(|error| error.to_string())
        });
    }

    CancelScanResult {
        accepted: outcome.accepted,
        reason: outcome.reason.to_string(),
        job_id,
        active_job_id: outcome.active_job_id,
        status: outcome.status.status,
        message: outcome.status.message,
    }
}

#[tauri::command]
pub fn get_scan_summary(
    state: State<'_, AppState>,
    job_id: Option<String>,
) -> Result<Option<scanner::ScanSummary>, String> {
    let connection = db::open_database_read_only(&state.database_path)?;
    match job_id {
        Some(job_id) => scanner::scan_summary_for_job(&connection, &job_id),
        None => scanner::latest_scan_summary(&connection),
    }
}

#[tauri::command]
pub fn ensure_clip_thumbnails(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
) -> Result<db::ThumbnailEnsureResult, String> {
    state.thumbnail_queue.ensure_clip_thumbnails(&clip_ids)
}

#[tauri::command]
pub fn retry_clip_thumbnails(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
) -> Result<db::ThumbnailEnsureResult, String> {
    state.thumbnail_queue.retry_clip_thumbnails(&clip_ids)
}

#[tauri::command]
pub fn get_thumbnail_status(state: State<'_, AppState>) -> Result<ThumbnailServiceStatus, String> {
    state.thumbnail_queue.status()
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        collections::BTreeSet,
        fs,
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use sha2::{Digest, Sha256};

    use super::{
        add_tag_to_clip_for_database, add_tag_to_clips_for_database, clip_id_from_media_request,
        clip_media_protocol_response, copy_clip_path_for_database, create_tag_for_database,
        delete_clips_permanently_for_database, delete_tag_for_database, discover_and_scan_roots,
        emit_scan_progress_ignoring_failure, ensure_scan_work_terminal,
        get_clip_detail_for_database, get_clip_media_for_database, list_tags_for_database,
        open_clip_externally_for_database, open_clip_location_for_database, parse_media_range,
        remove_clip_from_index_for_database, remove_clips_from_index_for_database,
        remove_tag_from_clip_for_database, remove_tag_from_clips_for_database,
        reset_clip_review_decision_for_database, restore_clip_review_state_for_database,
        set_clip_favorite_for_database, set_clip_review_decision_for_database,
        set_clip_trashed_for_database, set_clips_favorite_for_database,
        set_clips_trashed_for_database, update_clip_note_for_database, update_tag_for_database,
        ByteRange, PingResponse, ScanCommandError, ScanEventCursor, ScanWorkResult,
        MAX_MEDIA_CHUNK_BYTES,
    };
    use crate::db::{self, ClipInput, SourceDirInput};
    use crate::scan_coordinator::{ScanJobStatus, ScanProgressEvent};
    use crate::scanner::{self, ScanExecutionStatus, ScanSummary};
    use crate::thumbnail::MAX_THUMBNAIL_BYTES;
    use tauri::http::{
        header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, RANGE},
        Method, Response, StatusCode,
    };

    #[test]
    fn ping_response_uses_frontend_contract_names() {
        let json = serde_json::to_value(PingResponse {
            ok: true,
            product_name: "瓦刻",
            backend: "tauri-rust",
            database: "highlight-index.sqlite3".to_string(),
        })
        .expect("response should serialize");

        assert_eq!(json["ok"], true);
        assert_eq!(json["productName"], "瓦刻");
        assert_eq!(json["backend"], "tauri-rust");
        assert_eq!(json["database"], "highlight-index.sqlite3");
    }

    #[test]
    fn already_running_error_has_a_stable_machine_readable_code() {
        let json = serde_json::to_value(ScanCommandError {
            code: "already-running",
            message: "已有扫描任务正在运行".to_string(),
            job_id: None,
            active_job_id: Some("scan-active".to_string()),
        })
        .expect("error should serialize");

        assert_eq!(json["code"], "already-running");
        assert_eq!(json["activeJobId"], "scan-active");
    }

    #[test]
    fn progress_cursor_is_monotonic_and_all_terminal_states_emit_once() {
        let mut cursor = ScanEventCursor::default();
        let values = [(0, None), (4, Some(10)), (2, Some(3)), (8, Some(8))]
            .into_iter()
            .map(|(processed, total)| {
                cursor
                    .advance(processed, total, false)
                    .expect("non-terminal progress should emit")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![(0, None), (4, Some(10)), (4, None), (8, Some(8))]
        );

        for status in [
            ScanJobStatus::Completed,
            ScanJobStatus::Partial,
            ScanJobStatus::Failed,
            ScanJobStatus::Cancelled,
        ] {
            let mut terminal_cursor = ScanEventCursor::default();
            assert!(status.is_terminal());
            assert!(terminal_cursor.advance(5, Some(5), true).is_some());
            assert!(terminal_cursor.advance(6, Some(6), true).is_none());
        }
    }

    #[test]
    fn progress_emit_failure_does_not_fail_the_scan() {
        let event = ScanProgressEvent {
            job_id: "scan-emit-failure".to_string(),
            phase: "scanning".to_string(),
            current_root: Some("D:\\Clips".to_string()),
            source: None,
            processed: 1,
            total: Some(2),
            message: "scanning".to_string(),
            terminal: false,
            status: ScanJobStatus::Running,
            source_dir_count: 1,
            clip_group_count: 0,
            clip_file_count: 0,
        };
        let attempted = Cell::new(false);

        emit_scan_progress_ignoring_failure(&event, |_| {
            attempted.set(true);
            Err("listener closed".to_string())
        });

        assert!(attempted.get());
    }

    #[test]
    fn persisted_scan_work_skips_the_redundant_terminal_write() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("fixture root should be created");
        let database_path = root.join("highlight-index.sqlite3");
        db::migrate_database(&database_path).expect("database should migrate");
        let connection = db::open_database(&database_path).expect("database should open");
        connection
            .busy_timeout(Duration::from_millis(1))
            .expect("short busy timeout should configure");

        let job_id = "scan-persisted-before-command-fallback";
        let root_hint = root.display().to_string();
        scanner::ensure_scan_run_started(&connection, job_id, &root_hint)
            .expect("scan run should start");
        let message = "Scan completed before the command fallback";
        let mut summary = ScanSummary::empty(root_hint.clone());
        summary.message = Some(message.to_string());
        scanner::finalize_scan_run_for_job(
            &connection,
            job_id,
            ScanExecutionStatus::Completed,
            &summary,
        )
        .expect("scanner should persist its terminal result");

        let blocker = db::open_database(&database_path).expect("blocking connection should open");
        blocker
            .execute_batch("BEGIN IMMEDIATE")
            .expect("blocking writer should acquire the database");
        let redundant_write_error = scanner::ensure_scan_run_terminal(
            &connection,
            job_id,
            &root_hint,
            "completed",
            message,
        )
        .expect_err("a redundant terminal write should contend with the blocking writer");
        assert!(
            redundant_write_error.contains("locked") || redundant_write_error.contains("busy"),
            "unexpected SQLite contention error: {redundant_write_error}"
        );

        let result = ScanWorkResult {
            status: ScanJobStatus::Completed,
            result: Some(()),
            message: message.to_string(),
            scan_run_terminal_persisted: true,
        };
        ensure_scan_work_terminal(&connection, job_id, &root_hint, &result)
            .expect("a persisted successful scan must not perform the redundant write");

        blocker
            .execute_batch("ROLLBACK")
            .expect("blocking writer should roll back");
        let (status, persisted_message): (String, String) = connection
            .query_row(
                "SELECT status, message FROM scan_runs WHERE job_id = ?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("persisted scan run should load");
        assert_eq!(status, "completed");
        assert_eq!(persisted_message, message);

        drop(blocker);
        drop(connection);
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn unpersisted_scan_work_uses_the_terminal_fallback() {
        let connection = rusqlite::Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let job_id = "scan-cancelled-before-scanner-start";
        let root_hint = "fixed drives";
        scanner::ensure_scan_run_started(&connection, job_id, root_hint)
            .expect("scan run should start");
        let result = ScanWorkResult::<()> {
            status: ScanJobStatus::Cancelled,
            result: None,
            message: "cancelled during discovery".to_string(),
            scan_run_terminal_persisted: false,
        };

        ensure_scan_work_terminal(&connection, job_id, root_hint, &result)
            .expect("pre-scan cancellation should use the fallback terminal write");

        let (status, message): (String, String) = connection
            .query_row(
                "SELECT status, message FROM scan_runs WHERE job_id = ?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("cancelled scan run should load");
        assert_eq!(status, "cancelled");
        assert_eq!(message, "cancelled during discovery");
    }

    #[test]
    fn discover_and_scan_roots_imports_valid_fixture_sources() {
        let root = unique_temp_dir();
        let group = root.join("wonderfulVideos1001").join("match-a");
        fs::create_dir_all(root.join("Local Storage").join("leveldb"))
            .expect("fixture metadata root should be created");
        fs::create_dir_all(&group).expect("fixture group should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("fixture clip should be written");
        let connection = rusqlite::Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let result = discover_and_scan_roots(
            &connection,
            std::slice::from_ref(&root),
            &[],
            |_| {},
            |_| {},
        )
        .expect("fixture discovery should run");

        assert_eq!(result.validated_source_dir_count, 1);
        assert_eq!(result.scan_root_count, 1);
        assert_eq!(result.scanned_clip_count, 1);
        assert_eq!(result.scan_summary.new_clip_count, 1);
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn discover_and_scan_roots_imports_only_validated_sources_case_insensitively() {
        let root = unique_temp_dir();
        let valid_source_name = "WonderfulVideos1001";
        let valid_group = root.join(valid_source_name).join("match-a");
        let invalid_group = root.join("wonderfulVideos-empty").join("match-b");
        fs::create_dir_all(root.join("Local Storage").join("leveldb"))
            .expect("fixture metadata root should be created");
        fs::create_dir_all(&valid_group).expect("valid fixture group should be created");
        fs::write(valid_group.join("clip.mp4"), b"video")
            .expect("valid fixture clip should be written");
        fs::create_dir_all(&invalid_group).expect("invalid fixture group should be created");
        fs::write(invalid_group.join("readme.txt"), b"not a video")
            .expect("invalid fixture file should be written");
        let connection = rusqlite::Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let result = discover_and_scan_roots(
            &connection,
            std::slice::from_ref(&root),
            &[],
            |_| {},
            |_| {},
        )
        .expect("filtered discovery should run");
        let indexed_source_name: String = connection
            .query_row("SELECT name FROM source_dirs", [], |row| row.get(0))
            .expect("one source should be indexed");

        assert_eq!(result.validated_source_dir_count, 1);
        assert_eq!(result.scan_summary.source_dir_count, 1);
        assert_eq!(result.scanned_clip_count, 1);
        assert_eq!(indexed_source_name, valid_source_name);
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn discover_and_scan_roots_counts_clips_across_scan_roots() {
        let root = unique_temp_dir();
        for archive_name in ["ArchiveA", "ArchiveB"] {
            let archive = root.join(archive_name);
            let group = archive.join("wonderfulVideos1001").join("match-a");
            fs::create_dir_all(archive.join("Local Storage").join("leveldb"))
                .expect("fixture metadata root should be created");
            fs::create_dir_all(&group).expect("fixture group should be created");
            fs::write(group.join(format!("{archive_name}.mp4")), b"video")
                .expect("fixture clip should be written");
        }
        let connection = rusqlite::Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let result = discover_and_scan_roots(
            &connection,
            std::slice::from_ref(&root),
            &[],
            |_| {},
            |_| {},
        )
        .expect("multi-root discovery should run");

        assert_eq!(result.scan_root_count, 2);
        assert_eq!(result.scanned_clip_count, 2);
        assert_eq!(db::list_clips(&connection).unwrap().len(), 2);
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn discover_and_scan_roots_returns_normal_empty_result() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("fixture root should be created");
        let connection = rusqlite::Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let source_dir = db::upsert_source_dir(
            &connection,
            SourceDirInput {
                path: root.to_string_lossy().as_ref(),
                name: "Existing source",
            },
        )
        .expect("existing source should be seeded");
        db::upsert_clip(
            &connection,
            ClipInput {
                source_dir_id: source_dir.id,
                clip_group_id: None,
                video_path: root.join("existing.mp4").to_string_lossy().as_ref(),
                file_name: "existing.mp4",
                file_size: 5,
                modified_at: None,
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("existing clip should be seeded");

        let result = discover_and_scan_roots(
            &connection,
            std::slice::from_ref(&root),
            &[],
            |_| {},
            |_| {},
        )
        .expect("empty discovery should run");

        assert_eq!(result.validated_source_dir_count, 0);
        assert_eq!(result.scanned_clip_count, 0);
        assert_eq!(db::list_clips(&connection).unwrap().len(), 1);
        assert_eq!(
            result.scan_summary.message.as_deref(),
            Some("未发现标准无畏时刻素材")
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn clip_media_uses_clip_id_and_returns_only_a_media_token_for_existing_file() {
        let fixture = ClipCommandFixture::with_file("ace.mp4");

        let media = get_clip_media_for_database(&fixture.database_path, fixture.clip_id).unwrap();

        assert!(media.playable);
        assert_eq!(
            media.media_path.as_deref(),
            Some(format!("clip/{}", fixture.clip_id).as_str())
        );
        assert_eq!(media.message, None);
    }

    #[test]
    fn clip_media_blocks_playback_when_file_is_missing() {
        let fixture = ClipCommandFixture::without_file("missing.mp4");

        let media = get_clip_media_for_database(&fixture.database_path, fixture.clip_id).unwrap();
        let open_error =
            open_clip_location_for_database(&fixture.database_path, fixture.clip_id, |_| {
                panic!("missing clips must not launch Explorer")
            })
            .unwrap_err();

        assert!(!media.playable);
        assert_eq!(media.media_path, None);
        assert_eq!(media.message.as_deref(), Some("文件不存在"));
        assert_eq!(open_error, "文件不存在");
    }

    #[test]
    fn open_clip_location_reveals_the_database_clip_path_by_id() {
        let fixture = ClipCommandFixture::with_file("open-me.mp4");
        let mut revealed_path = None;

        open_clip_location_for_database(&fixture.database_path, fixture.clip_id, |path| {
            revealed_path = Some(path.to_path_buf());
            Ok(())
        })
        .unwrap();

        let canonical_path = fixture
            .clip_path
            .canonicalize()
            .expect("fixture path should canonicalize");
        let expected_shell_path = PathBuf::from(db::stable_path_for_storage(
            canonical_path.to_string_lossy().as_ref(),
        ));
        assert_eq!(revealed_path, Some(expected_shell_path));
    }

    #[test]
    fn external_player_revalidates_index_status_extension_and_source_boundary() {
        let fixture = ClipCommandFixture::with_file("external-open.mp4");
        let mut opened_path = None;

        open_clip_externally_for_database(&fixture.database_path, fixture.clip_id, |path| {
            opened_path = Some(path.to_path_buf());
            Ok(())
        })
        .expect("an available in-root MP4 should be opened");

        let canonical_path = fixture
            .clip_path
            .canonicalize()
            .expect("fixture path should canonicalize");
        let expected_shell_path = PathBuf::from(db::stable_path_for_storage(
            canonical_path.to_string_lossy().as_ref(),
        ));
        assert_eq!(opened_path, Some(expected_shell_path));
        let connection = db::open_database(&fixture.database_path).unwrap();
        db::update_clip_trashed(&connection, fixture.clip_id, true).unwrap();
        let error =
            open_clip_externally_for_database(&fixture.database_path, fixture.clip_id, |_| {
                panic!("trashed clips must not launch the player")
            })
            .expect_err("trashed clips should be rejected");
        assert!(error.contains("可用素材"));
    }

    #[test]
    fn safe_open_commands_preserve_spaces_and_unicode_in_the_shell_path() {
        let fixture = ClipCommandFixture::with_file("录像 空格.mp4");
        let canonical_path = fixture
            .clip_path
            .canonicalize()
            .expect("unicode fixture path should canonicalize");
        let expected_shell_path = PathBuf::from(db::stable_path_for_storage(
            canonical_path.to_string_lossy().as_ref(),
        ));
        let mut revealed_path = None;
        let mut opened_path = None;

        open_clip_location_for_database(&fixture.database_path, fixture.clip_id, |path| {
            revealed_path = Some(path.to_path_buf());
            Ok(())
        })
        .expect("a safe unicode MP4 should be revealed");
        open_clip_externally_for_database(&fixture.database_path, fixture.clip_id, |path| {
            opened_path = Some(path.to_path_buf());
            Ok(())
        })
        .expect("a safe unicode MP4 should be opened");

        assert_eq!(revealed_path, Some(expected_shell_path.clone()));
        assert_eq!(opened_path, Some(expected_shell_path));
    }

    #[test]
    fn safe_open_commands_reject_an_existing_file_outside_the_registered_source() {
        let fixture = ClipCommandFixture::with_file("inside.mp4");
        let outside_root = unique_temp_dir();
        fs::create_dir_all(&outside_root).expect("outside root should be created");
        let outside_path = outside_root.join("越界 空格.mp4");
        fs::write(&outside_path, b"outside mp4").expect("outside file should be written");
        let connection = db::open_database(&fixture.database_path).unwrap();
        connection
            .execute(
                "UPDATE clips SET file_path = ?2, normalized_path = ?3 WHERE id = ?1",
                rusqlite::params![
                    fixture.clip_id,
                    outside_path.to_string_lossy().as_ref(),
                    db::normalize_path(outside_path.to_string_lossy().as_ref()),
                ],
            )
            .expect("fixture path should move outside its registered source");
        drop(connection);

        let reveal_error =
            open_clip_location_for_database(&fixture.database_path, fixture.clip_id, |_| {
                panic!("an out-of-source file must not be revealed")
            })
            .expect_err("location should reject an out-of-source file");
        let open_error =
            open_clip_externally_for_database(&fixture.database_path, fixture.clip_id, |_| {
                panic!("an out-of-source file must not be opened")
            })
            .expect_err("external open should reject an out-of-source file");

        assert!(reveal_error.contains("越出已授权来源目录"));
        assert!(open_error.contains("越出已授权来源目录"));
        let _ = fs::remove_dir_all(outside_root);
    }

    #[test]
    fn safe_open_commands_reject_an_ancestor_symlink_or_reparse_point_when_supported() {
        let fixture = ClipCommandFixture::without_file("original.mp4");
        let real_directory = fixture._root.join("真实目录");
        let linked_directory = fixture._root.join("链接目录");
        fs::create_dir_all(&real_directory).expect("real directory should be created");
        let real_path = real_directory.join("录像 空格.mp4");
        fs::write(&real_path, b"linked mp4").expect("linked target should be written");

        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_dir(&real_directory, &linked_directory);
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(&real_directory, &linked_directory);
        #[cfg(not(any(windows, unix)))]
        let link_result: std::io::Result<()> = Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "directory symlinks are unavailable on this platform",
        ));
        if link_result.is_err() {
            return;
        }

        let path_through_link = linked_directory.join("录像 空格.mp4");
        let connection = db::open_database(&fixture.database_path).unwrap();
        connection
            .execute(
                "UPDATE clips SET file_path = ?2, normalized_path = ?3 WHERE id = ?1",
                rusqlite::params![
                    fixture.clip_id,
                    path_through_link.to_string_lossy().as_ref(),
                    db::normalize_path(path_through_link.to_string_lossy().as_ref()),
                ],
            )
            .expect("fixture path should use the linked ancestor");
        drop(connection);

        let reveal_error =
            open_clip_location_for_database(&fixture.database_path, fixture.clip_id, |_| {
                panic!("a reparse path must not be revealed")
            })
            .expect_err("location should reject an ancestor reparse point");
        let open_error =
            open_clip_externally_for_database(&fixture.database_path, fixture.clip_id, |_| {
                panic!("a reparse path must not be opened")
            })
            .expect_err("external open should reject an ancestor reparse point");

        assert!(reveal_error.contains("reparse point"));
        assert!(open_error.contains("reparse point"));
    }

    #[test]
    fn copy_clip_path_returns_the_database_clip_path_by_id() {
        let fixture = ClipCommandFixture::with_file("copy-me.mp4");

        let copied_path =
            copy_clip_path_for_database(&fixture.database_path, fixture.clip_id).unwrap();

        assert_eq!(copied_path, fixture.clip_path.display().to_string());
    }

    #[test]
    fn clip_user_state_commands_update_and_return_the_reloaded_clip() {
        let fixture = ClipCommandFixture::with_file("user-state.mp4");

        let favorite_clip =
            set_clip_favorite_for_database(&fixture.database_path, fixture.clip_id, true)
                .expect("favorite should update");
        let noted_clip =
            update_clip_note_for_database(&fixture.database_path, fixture.clip_id, "  keep this  ")
                .expect("note should update");

        assert!(favorite_clip.favorite);
        assert_eq!(noted_clip.note.as_deref(), Some("keep this"));
    }

    #[test]
    fn review_command_core_supports_decision_reset_and_compare_and_swap_restore() {
        let fixture = ClipCommandFixture::with_file("review-command.mp4");
        let liked = set_clip_review_decision_for_database(
            &fixture.database_path,
            fixture.clip_id,
            db::ReviewDecision::Liked,
        )
        .expect("liked command should persist");
        assert!(liked.after.favorite);
        assert_eq!(liked.after.review_decision, db::ReviewDecision::Liked);

        let reset =
            reset_clip_review_decision_for_database(&fixture.database_path, fixture.clip_id)
                .expect("reset command should persist");
        assert!(reset.after.favorite);
        assert_eq!(reset.after.review_decision, db::ReviewDecision::Unreviewed);

        let restored = restore_clip_review_state_for_database(
            &fixture.database_path,
            fixture.clip_id,
            reset.after.clone(),
            reset.before.clone(),
        )
        .expect("restore command should compare and restore exact state");
        assert_eq!(restored.after, reset.before);

        let stale = restore_clip_review_state_for_database(
            &fixture.database_path,
            fixture.clip_id,
            reset.after,
            liked.before,
        )
        .expect_err("stale command restore should fail closed");
        assert!(stale.contains("stale undo"));
    }

    #[test]
    fn recycle_bin_commands_preserve_the_video_and_can_remove_only_the_index_row() {
        let fixture = ClipCommandFixture::with_file("recycle-me.mp4");
        let before_bytes = fs::read(&fixture.clip_path).unwrap();
        let before_hash = Sha256::digest(&before_bytes).to_vec();
        let before_metadata = fs::metadata(&fixture.clip_path).unwrap();
        let before_size = before_metadata.len();
        let before_modified = before_metadata.modified().unwrap();
        let before_listing = directory_file_names(&fixture._root);

        let trashed = set_clip_trashed_for_database(&fixture.database_path, fixture.clip_id, true)
            .expect("clip should enter recycle bin");
        assert_eq!(trashed.status, "trashed");
        assert!(fixture.clip_path.is_file());

        let restored =
            set_clip_trashed_for_database(&fixture.database_path, fixture.clip_id, false)
                .expect("clip should restore");
        assert_eq!(restored.status, "available");

        let available_error =
            remove_clip_from_index_for_database(&fixture.database_path, fixture.clip_id)
                .expect_err("available clip must not be removable from the index");
        assert!(available_error.contains("index-removal-not-eligible"));

        set_clip_trashed_for_database(&fixture.database_path, fixture.clip_id, true)
            .expect("clip should re-enter recycle bin");

        remove_clip_from_index_for_database(&fixture.database_path, fixture.clip_id)
            .expect("clip index row should be removed");
        let connection = db::open_database(&fixture.database_path).expect("database should open");
        assert!(db::find_clip_by_id(&connection, fixture.clip_id).is_err());
        let intent_count = connection
            .query_row("SELECT COUNT(*) FROM clip_delete_intents", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(intent_count, 0);
        drop(connection);

        assert!(fixture.clip_path.is_file());
        assert_eq!(fs::metadata(&fixture.clip_path).unwrap().len(), before_size);
        assert_eq!(
            fs::metadata(&fixture.clip_path)
                .unwrap()
                .modified()
                .unwrap(),
            before_modified
        );
        assert_eq!(
            Sha256::digest(fs::read(&fixture.clip_path).unwrap()).to_vec(),
            before_hash
        );
        assert_eq!(directory_file_names(&fixture._root), before_listing);
    }

    #[test]
    fn index_only_batch_is_partial_deduplicated_bounded_and_never_mutates_video_files() {
        let fixture = ClipCommandFixture::with_file("available.mp4");
        let videos_dir = fixture._root.join("offline-videos");
        fs::create_dir_all(&videos_dir).unwrap();
        let offline_path = videos_dir.join("offline.mp4");
        fs::write(
            &offline_path,
            b"offline video bytes that must remain unchanged",
        )
        .unwrap();

        let connection = db::open_database(&fixture.database_path).unwrap();
        let source_dir_id = connection
            .query_row(
                "SELECT source_dir_id FROM clips WHERE id = ?1",
                [fixture.clip_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        let offline_clip = db::upsert_clip(
            &connection,
            ClipInput {
                source_dir_id,
                clip_group_id: None,
                video_path: offline_path.to_string_lossy().as_ref(),
                file_name: "offline.mp4",
                file_size: i64::try_from(fs::metadata(&offline_path).unwrap().len()).unwrap(),
                modified_at: Some("1782634273"),
                duration_ms: Some(10_000),
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .unwrap();
        connection
            .execute(
                "
                UPDATE clips
                SET file_status = 'missing',
                    is_favorite = 1,
                    review_decision = 'disliked',
                    reviewed_at = CURRENT_TIMESTAMP,
                    note = 'must be cascaded'
                WHERE id = ?1
                ",
                [offline_clip.id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE clip_metadata SET metadata_status = 'enriched', extra_json = '{}' WHERE clip_id = ?1",
                [offline_clip.id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO clip_events (clip_id, event_key, event_type) VALUES (?1, 'event-a', 'kill')",
                [offline_clip.id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO clip_thumbnails (clip_id, fingerprint, status) VALUES (?1, 'fixture-fingerprint', 'pending')",
                [offline_clip.id],
            )
            .unwrap();
        connection
            .execute("INSERT INTO tags (name) VALUES ('offline-tag')", [])
            .unwrap();
        let tag_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO clip_tags (clip_id, tag_id) VALUES (?1, ?2)",
                [offline_clip.id, tag_id],
            )
            .unwrap();
        drop(connection);

        let before_bytes = fs::read(&offline_path).unwrap();
        let before_hash = Sha256::digest(&before_bytes).to_vec();
        let before_metadata = fs::metadata(&offline_path).unwrap();
        let before_size = before_metadata.len();
        let before_modified = before_metadata.modified().unwrap();
        let before_listing = directory_file_names(&videos_dir);
        let missing_id = offline_clip.id + 100_000;

        let result = remove_clips_from_index_for_database(
            &fixture.database_path,
            &[
                offline_clip.id,
                fixture.clip_id,
                offline_clip.id,
                missing_id,
            ],
        )
        .expect("mixed index cleanup should return a partial result");

        assert_eq!(result.requested, 3);
        assert_eq!(result.removed_ids, vec![offline_clip.id]);
        assert_eq!(result.missing_ids, vec![missing_id]);
        assert_eq!(result.blocked.len(), 1);
        assert_eq!(result.blocked[0].clip_id, fixture.clip_id);
        assert_eq!(result.blocked[0].code, "index-removal-not-eligible");
        assert!(result.failures.is_empty());
        assert_eq!(fs::metadata(&offline_path).unwrap().len(), before_size);
        assert_eq!(
            fs::metadata(&offline_path).unwrap().modified().unwrap(),
            before_modified
        );
        assert_eq!(
            Sha256::digest(fs::read(&offline_path).unwrap()).to_vec(),
            before_hash
        );
        assert_eq!(directory_file_names(&videos_dir), before_listing);

        let connection = db::open_database(&fixture.database_path).unwrap();
        for table in [
            "clips",
            "clip_metadata",
            "clip_events",
            "clip_thumbnails",
            "clip_tags",
        ] {
            let count = connection
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE {} = ?1",
                        if table == "clips" { "id" } else { "clip_id" }
                    ),
                    [offline_clip.id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} rows should cascade");
        }
        let intent_count = connection
            .query_row("SELECT COUNT(*) FROM clip_delete_intents", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(
            intent_count, 0,
            "index cleanup must not authorize permanent deletion"
        );
        drop(connection);

        let empty = remove_clips_from_index_for_database(&fixture.database_path, &[]).unwrap();
        assert_eq!(empty.requested, 0);
        assert!(empty.removed_ids.is_empty());
        assert!(empty.blocked.is_empty());

        let too_many = (1_i64..=201).collect::<Vec<_>>();
        let limit_error = remove_clips_from_index_for_database(&fixture.database_path, &too_many)
            .expect_err("oversized cleanup batch should be rejected before item processing");
        assert!(limit_error.contains("200"));
    }

    #[test]
    fn index_only_batch_removes_trashed_rows_without_mutating_their_videos() {
        let fixture = ClipCommandFixture::with_file("trashed-one.mp4");
        let second_path = fixture._root.join("trashed-two.mp4");
        fs::write(&second_path, b"second trashed video must survive").unwrap();
        let connection = db::open_database(&fixture.database_path).unwrap();
        let source_dir_id = connection
            .query_row(
                "SELECT source_dir_id FROM clips WHERE id = ?1",
                [fixture.clip_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        let second = db::upsert_clip(
            &connection,
            ClipInput {
                source_dir_id,
                clip_group_id: None,
                video_path: second_path.to_string_lossy().as_ref(),
                file_name: "trashed-two.mp4",
                file_size: i64::try_from(fs::metadata(&second_path).unwrap().len()).unwrap(),
                modified_at: Some("1782634274"),
                duration_ms: Some(10_000),
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .unwrap();
        drop(connection);
        let first_before = fs::read(&fixture.clip_path).unwrap();
        let second_before = fs::read(&second_path).unwrap();

        let trashed = set_clips_trashed_for_database(
            &fixture.database_path,
            &[fixture.clip_id, second.id],
            true,
        )
        .expect("both clips should enter the recycle bin");
        assert_eq!(trashed.updated, 2);

        let removed = remove_clips_from_index_for_database(
            &fixture.database_path,
            &[fixture.clip_id, second.id],
        )
        .expect("trashed rows should be eligible for batch index removal");
        assert_eq!(removed.removed_ids, vec![fixture.clip_id, second.id]);
        assert!(removed.blocked.is_empty());
        assert!(removed.failures.is_empty());

        let connection = db::open_database(&fixture.database_path).unwrap();
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM clips WHERE id IN (?1, ?2)",
                [fixture.clip_id, second.id],
                |row| row.get(0),
            )
            .unwrap();
        let snapshots: i64 = connection
            .query_row("SELECT COUNT(*) FROM clip_trash_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0);
        assert_eq!(
            snapshots, 0,
            "clip cascade should remove stale trash snapshots"
        );
        assert_eq!(fs::read(&fixture.clip_path).unwrap(), first_before);
        assert_eq!(fs::read(&second_path).unwrap(), second_before);
    }

    #[test]
    fn permanent_delete_requires_recycle_bin_and_removes_video_and_index() {
        let fixture = ClipCommandFixture::with_file("delete-forever.mp4");
        let missing_id = fixture.clip_id + 10_000;

        let rejected =
            delete_clips_permanently_for_database(&fixture.database_path, &[fixture.clip_id])
                .expect("active clip deletion should return an item failure");
        assert_eq!(rejected.requested, 1);
        assert!(rejected.deleted_ids.is_empty());
        assert_eq!(rejected.failures.len(), 1);
        assert!(rejected.failures[0].message.contains("不在回收站"));
        assert!(fixture.clip_path.is_file());

        set_clip_trashed_for_database(&fixture.database_path, fixture.clip_id, true)
            .expect("clip should enter recycle bin");
        let deleted = delete_clips_permanently_for_database(
            &fixture.database_path,
            &[fixture.clip_id, fixture.clip_id, missing_id],
        )
        .expect("trashed clip should delete permanently");

        assert_eq!(deleted.requested, 2);
        assert_eq!(deleted.deleted_ids, vec![fixture.clip_id]);
        assert_eq!(deleted.missing_ids, vec![missing_id]);
        assert!(deleted.failures.is_empty());
        assert!(!fixture.clip_path.exists());
        let connection = db::open_database(&fixture.database_path).expect("database should open");
        assert!(db::find_clip_by_id(&connection, fixture.clip_id).is_err());
    }

    #[test]
    fn permanent_delete_rejects_non_video_index_rows() {
        let fixture = ClipCommandFixture::with_file("do-not-delete.txt");
        set_clip_trashed_for_database(&fixture.database_path, fixture.clip_id, true)
            .expect("fixture should enter recycle bin");

        let rejected =
            delete_clips_permanently_for_database(&fixture.database_path, &[fixture.clip_id])
                .expect("unsafe target should return an item failure");

        assert!(rejected.deleted_ids.is_empty());
        assert_eq!(rejected.failures.len(), 1);
        assert!(rejected.failures[0].message.contains("MP4"));
        assert!(fixture.clip_path.is_file());
        let connection = db::open_database(&fixture.database_path).expect("database should open");
        assert!(db::find_clip_by_id(&connection, fixture.clip_id).is_ok());
    }

    #[test]
    fn permanent_delete_cleans_the_index_when_the_video_is_already_missing() {
        let fixture = ClipCommandFixture::without_file("already-missing.mp4");
        set_clip_trashed_for_database(&fixture.database_path, fixture.clip_id, true)
            .expect("fixture should enter recycle bin");

        let deleted =
            delete_clips_permanently_for_database(&fixture.database_path, &[fixture.clip_id])
                .expect("missing video should still allow recycle-bin cleanup");

        assert_eq!(deleted.deleted_ids, vec![fixture.clip_id]);
        assert!(deleted.failures.is_empty());
        let connection = db::open_database(&fixture.database_path).expect("database should open");
        assert!(db::find_clip_by_id(&connection, fixture.clip_id).is_err());
    }

    #[test]
    fn tag_commands_create_list_assign_and_remove_tags_without_touching_files() {
        let fixture = ClipCommandFixture::with_file("tagged.mp4");

        assert!(list_tags_for_database(&fixture.database_path)
            .expect("fresh tags should list")
            .is_empty());
        let tactical = create_tag_for_database(&fixture.database_path, "战术", Some("blue"))
            .expect("first custom tag should create");
        let custom = create_tag_for_database(&fixture.database_path, "复盘", Some("green"))
            .expect("custom tag should create");
        let custom =
            update_tag_for_database(&fixture.database_path, custom.id, "精选复盘", Some("gold"))
                .expect("custom tag should update");
        assert_eq!(custom.name, "精选复盘");
        assert_eq!(custom.color.as_deref(), Some("gold"));

        add_tag_to_clip_for_database(&fixture.database_path, fixture.clip_id, tactical.id)
            .expect("first custom tag should assign");
        let tagged_clip =
            add_tag_to_clip_for_database(&fixture.database_path, fixture.clip_id, custom.id)
                .expect("custom tag should assign");

        assert_eq!(tagged_clip.tag_ids, vec![tactical.id, custom.id]);
        assert!(fixture.clip_path.is_file());

        let reloaded_clip =
            remove_tag_from_clip_for_database(&fixture.database_path, fixture.clip_id, tactical.id)
                .expect("first custom tag should remove");

        assert_eq!(reloaded_clip.tag_ids, vec![custom.id]);
        assert!(fixture.clip_path.is_file());

        delete_tag_for_database(&fixture.database_path, custom.id)
            .expect("custom tag should delete");
        let reloaded_clip = db::open_database(&fixture.database_path)
            .and_then(|connection| db::find_clip_by_id(&connection, fixture.clip_id))
            .expect("clip should reload after tag deletion");
        assert!(reloaded_clip.tag_ids.is_empty());
        delete_tag_for_database(&fixture.database_path, tactical.id)
            .expect("first custom tag should delete");
        assert!(fixture.clip_path.is_file());
    }

    #[test]
    fn single_clip_commands_share_the_batch_mutation_semantics() {
        let fixture = ClipCommandFixture::with_file("single-batch-core.mp4");
        let tactical = create_tag_for_database(&fixture.database_path, "战术", Some("red"))
            .expect("custom tag should create");

        let favorite_batch =
            set_clips_favorite_for_database(&fixture.database_path, &[fixture.clip_id], true)
                .expect("favorite batch should update");
        assert_eq!(favorite_batch.updated, 1);
        assert!(favorite_batch.clips[0].favorite);
        let single_favorite =
            set_clip_favorite_for_database(&fixture.database_path, fixture.clip_id, false)
                .expect("single favorite should use the same core");
        assert!(!single_favorite.favorite);

        let tag_batch =
            add_tag_to_clips_for_database(&fixture.database_path, &[fixture.clip_id], tactical.id)
                .expect("tag batch should update");
        assert_eq!(tag_batch.updated, 1);
        assert!(tag_batch.clips[0].tag_ids.contains(&tactical.id));
        let single_untagged =
            remove_tag_from_clip_for_database(&fixture.database_path, fixture.clip_id, tactical.id)
                .expect("single remove should use the same core");
        assert!(!single_untagged.tag_ids.contains(&tactical.id));
        let idempotent_remove = remove_tag_from_clips_for_database(
            &fixture.database_path,
            &[fixture.clip_id],
            tactical.id,
        )
        .expect("batch remove should be idempotent");
        assert_eq!(idempotent_remove.updated, 0);

        let trashed_batch =
            set_clips_trashed_for_database(&fixture.database_path, &[fixture.clip_id], true)
                .expect("trash batch should update");
        assert_eq!(trashed_batch.clips[0].status, "trashed");
        let single_restored =
            set_clip_trashed_for_database(&fixture.database_path, fixture.clip_id, false)
                .expect("single restore should use the same core");
        assert_eq!(single_restored.status, "available");
        assert!(fixture.clip_path.is_file());

        let single_added =
            add_tag_to_clip_for_database(&fixture.database_path, fixture.clip_id, tactical.id)
                .expect("single add should remain available");
        assert!(single_added.tag_ids.contains(&tactical.id));
    }

    #[test]
    fn media_range_parser_accepts_standard_single_byte_ranges() {
        assert_eq!(
            parse_media_range(Some("bytes=10-19"), 100).unwrap(),
            Some(ByteRange { start: 10, end: 19 })
        );
        assert_eq!(
            parse_media_range(Some("bytes=90-"), 100).unwrap(),
            Some(ByteRange { start: 90, end: 99 })
        );
        assert_eq!(
            parse_media_range(Some("bytes=-10"), 100).unwrap(),
            Some(ByteRange { start: 90, end: 99 })
        );
        assert!(parse_media_range(Some("bytes=100-120"), 100).is_err());
    }

    #[test]
    fn large_media_without_range_returns_a_bounded_initial_chunk() {
        let fixture = ClipCommandFixture::with_file("large.mp4");
        let file_len = MAX_MEDIA_CHUNK_BYTES * 2 + 17;
        fs::write(&fixture.clip_path, vec![0x5a; file_len as usize])
            .expect("large fixture clip should be writable");

        let response = request_clip_media(&fixture, Method::GET, None);

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body().len(), MAX_MEDIA_CHUNK_BYTES as usize);
        assert_eq!(
            header(&response, CONTENT_LENGTH),
            MAX_MEDIA_CHUNK_BYTES.to_string()
        );
        assert_eq!(
            header(&response, CONTENT_RANGE),
            format!("bytes 0-{}/{file_len}", MAX_MEDIA_CHUNK_BYTES - 1)
        );
    }

    #[test]
    fn media_protocol_handles_ranges_head_empty_files_and_length_changes() {
        let fixture = ClipCommandFixture::with_file("ranges.mp4");
        let bytes = (0u8..20).collect::<Vec<_>>();
        fs::write(&fixture.clip_path, &bytes).expect("range fixture should be writable");

        let first = request_clip_media(&fixture, Method::GET, Some("bytes=0-3"));
        assert_eq!(first.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(first.body(), &bytes[0..4]);
        assert_eq!(header(&first, CONTENT_RANGE), "bytes 0-3/20");
        assert_eq!(header(&first, CONTENT_LENGTH), "4");

        let tail = request_clip_media(&fixture, Method::GET, Some("bytes=-4"));
        assert_eq!(tail.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(tail.body(), &bytes[16..20]);
        assert_eq!(header(&tail, CONTENT_RANGE), "bytes 16-19/20");
        assert_eq!(header(&tail, CONTENT_LENGTH), "4");

        let ranged_head = request_clip_media(&fixture, Method::HEAD, Some("bytes=5-9"));
        assert_eq!(ranged_head.status(), StatusCode::PARTIAL_CONTENT);
        assert!(ranged_head.body().is_empty());
        assert_eq!(header(&ranged_head, CONTENT_RANGE), "bytes 5-9/20");
        assert_eq!(header(&ranged_head, CONTENT_LENGTH), "5");

        let full_head = request_clip_media(&fixture, Method::HEAD, None);
        assert_eq!(full_head.status(), StatusCode::OK);
        assert!(full_head.body().is_empty());
        assert_eq!(header(&full_head, CONTENT_LENGTH), "20");

        for invalid_range in ["bytes=20-", "bytes=0-1,4-5", "items=0-1"] {
            let response = request_clip_media(&fixture, Method::GET, Some(invalid_range));
            assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
            assert!(response.body().is_empty());
            assert_eq!(header(&response, CONTENT_RANGE), "bytes */20");
            assert_eq!(header(&response, CONTENT_LENGTH), "0");
        }

        fs::write(&fixture.clip_path, []).expect("fixture should become empty");
        let empty = request_clip_media(&fixture, Method::GET, None);
        assert_eq!(empty.status(), StatusCode::OK);
        assert!(empty.body().is_empty());
        assert_eq!(header(&empty, CONTENT_LENGTH), "0");

        let empty_range = request_clip_media(&fixture, Method::GET, Some("bytes=0-0"));
        assert_eq!(empty_range.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(header(&empty_range, CONTENT_RANGE), "bytes */0");
        assert_eq!(header(&empty_range, CONTENT_LENGTH), "0");

        fs::write(&fixture.clip_path, b"changed").expect("fixture should change length");
        let changed = request_clip_media(&fixture, Method::GET, None);
        assert_eq!(changed.status(), StatusCode::OK);
        assert_eq!(changed.body(), b"changed");
        assert_eq!(header(&changed, CONTENT_LENGTH), "7");
        assert!(changed.headers().get(CONTENT_RANGE).is_none());
    }

    #[test]
    fn generated_cover_protocol_supports_get_head_and_security_headers() {
        let fixture = ClipCommandFixture::with_file("generated-cover.mp4");
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 0, 0xff, 0xd9];
        seed_generated_cover(&fixture, &jpeg);

        let get = request_cover_media(&fixture, Method::GET);
        assert_eq!(get.status(), StatusCode::OK);
        assert_eq!(get.body(), &jpeg);
        assert_eq!(header(&get, CONTENT_LENGTH), jpeg.len().to_string());
        assert_eq!(header(&get, CACHE_CONTROL), "no-cache");
        assert_eq!(
            get.headers()
                .get("x-content-type-options")
                .unwrap()
                .to_str()
                .unwrap(),
            "nosniff"
        );

        let head = request_cover_media(&fixture, Method::HEAD);
        assert_eq!(head.status(), StatusCode::OK);
        assert!(head.body().is_empty());
        assert_eq!(header(&head, CONTENT_LENGTH), jpeg.len().to_string());
    }

    #[test]
    fn source_cover_is_read_only_and_wins_over_generated_cache() {
        let fixture = ClipCommandFixture::with_file("source-cover.mp4");
        let generated = [0xff, 0xd8, 0xff, 0xe0, 1, 1, 0xff, 0xd9];
        seed_generated_cover(&fixture, &generated);
        let source_cover = fixture._root.join("cover-source-cover.jpeg");
        let source_bytes = b"source-owned-cover";
        fs::write(&source_cover, source_bytes).unwrap();
        let connection = db::open_database(&fixture.database_path).unwrap();
        connection
            .execute(
                "UPDATE clips SET cover_path = ?1, cover_source = 'file' WHERE id = ?2",
                rusqlite::params![source_cover.to_string_lossy().as_ref(), fixture.clip_id],
            )
            .unwrap();
        drop(connection);

        let response = request_cover_media(&fixture, Method::GET);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), source_bytes);
        assert_eq!(fs::read(&source_cover).unwrap(), source_bytes);
    }

    #[test]
    fn generated_cover_rejects_invalid_jpeg_and_source_cover_is_size_bounded() {
        let generated_fixture = ClipCommandFixture::with_file("invalid-cover.mp4");
        seed_generated_cover(&generated_fixture, b"not a jpeg");
        let invalid = request_cover_media(&generated_fixture, Method::GET);
        assert_eq!(invalid.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let source_fixture = ClipCommandFixture::with_file("oversized-cover.mp4");
        let source_cover = source_fixture._root.join("cover-oversized.jpeg");
        let file = fs::File::create(&source_cover).unwrap();
        file.set_len(MAX_THUMBNAIL_BYTES + 1).unwrap();
        let connection = db::open_database(&source_fixture.database_path).unwrap();
        connection
            .execute(
                "UPDATE clips SET cover_path = ?1, cover_source = 'file' WHERE id = ?2",
                rusqlite::params![
                    source_cover.to_string_lossy().as_ref(),
                    source_fixture.clip_id
                ],
            )
            .unwrap();
        drop(connection);
        let oversized = request_cover_media(&source_fixture, Method::GET);
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            fs::metadata(source_cover).unwrap().len(),
            MAX_THUMBNAIL_BYTES + 1
        );
    }

    #[test]
    fn consecutive_media_requests_do_not_run_database_migrations() {
        let fixture = ClipCommandFixture::with_file("no-migration.mp4");
        let connection = db::open_database(&fixture.database_path).expect("database should open");
        connection
            .execute("DELETE FROM tags WHERE name = 'ACE'", [])
            .expect("fixture tag should be removable");
        drop(connection);

        for range in ["bytes=0-2", "bytes=3-5"] {
            let response = request_clip_media(&fixture, Method::GET, Some(range));
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        }

        let connection = db::open_database_read_only(&fixture.database_path)
            .expect("database should open read-only");
        let ace_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM tags WHERE name = 'ACE'", [], |row| {
                row.get(0)
            })
            .expect("tag count should be readable");
        assert_eq!(ace_count, 0, "media requests must not reseed schema data");
    }

    #[test]
    fn media_protocol_reloads_the_current_indexed_path_for_each_request() {
        let fixture = ClipCommandFixture::with_file("old.mp4");
        fs::remove_file(&fixture.clip_path).expect("old clip should be removable");

        let missing = request_clip_media(&fixture, Method::GET, None);
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let replacement_path = fixture._root.join("replacement.mp4");
        fs::write(&replacement_path, b"replacement").expect("replacement should be writable");
        let replacement_text = replacement_path.to_string_lossy().to_string();
        let connection = db::open_database(&fixture.database_path).expect("database should open");
        connection
            .execute(
                "
                UPDATE clips
                SET file_path = ?1,
                    normalized_path = ?2,
                    file_name = 'replacement.mp4',
                    size_bytes = 11
                WHERE id = ?3
                ",
                rusqlite::params![
                    replacement_text,
                    db::normalize_path(&replacement_text),
                    fixture.clip_id
                ],
            )
            .expect("indexed media path should update");
        drop(connection);

        let replacement = request_clip_media(&fixture, Method::GET, None);
        assert_eq!(replacement.status(), StatusCode::OK);
        assert_eq!(replacement.body(), b"replacement");
        assert_eq!(header(&replacement, CONTENT_LENGTH), "11");
    }

    #[test]
    fn media_request_parser_accepts_convert_file_src_encoded_paths() {
        let request = tauri::http::Request::builder()
            .uri("https://clip-media.localhost/clip%2F42")
            .body(Vec::new())
            .expect("request should build");

        assert_eq!(clip_id_from_media_request(&request).unwrap(), 42);

        let nested = tauri::http::Request::builder()
            .uri("https://clip-media.localhost/clip/42/unindexed.mp4")
            .body(Vec::new())
            .expect("request should build");
        assert!(clip_id_from_media_request(&nested).is_err());
    }

    #[test]
    fn clip_detail_command_uses_stable_not_found_error_contract() {
        let fixture = ClipCommandFixture::without_file("detail-contract.mp4");
        let missing_id = fixture.clip_id + 10_000;

        let error = get_clip_detail_for_database(&fixture.database_path, missing_id)
            .expect_err("missing detail should return a structured error");

        assert_eq!(error.code, "clip-not-found");
        assert_eq!(error.clip_id, missing_id);
        assert!(error.message.contains(&missing_id.to_string()));
        let json = serde_json::to_value(error).expect("detail error should serialize");
        assert_eq!(json["code"], "clip-not-found");
        assert_eq!(json["clipId"], missing_id);
    }

    struct ClipCommandFixture {
        database_path: PathBuf,
        clip_id: i64,
        clip_path: PathBuf,
        thumbnail_cache_root: PathBuf,
        _root: PathBuf,
    }

    impl ClipCommandFixture {
        fn with_file(file_name: &str) -> Self {
            let fixture = Self::create(file_name);
            fs::write(&fixture.clip_path, b"mp4 bytes").expect("test clip should be writable");
            fixture
        }

        fn without_file(file_name: &str) -> Self {
            Self::create(file_name)
        }

        fn create(file_name: &str) -> Self {
            let root = unique_temp_dir();
            fs::create_dir_all(&root).expect("fixture root should be created");
            let clip_path = root.join(file_name);
            let thumbnail_cache_root = root.join("thumbnail-cache");
            let database_path = root.join("highlight-index.sqlite3");
            db::migrate_database(&database_path).expect("database should migrate");
            let connection = db::open_database(&database_path).expect("database should open");
            let source_dir = db::upsert_source_dir(
                &connection,
                SourceDirInput {
                    path: root.to_string_lossy().as_ref(),
                    name: "Fixture",
                },
            )
            .expect("source dir should upsert");
            let clip = db::upsert_clip(
                &connection,
                ClipInput {
                    source_dir_id: source_dir.id,
                    clip_group_id: None,
                    video_path: clip_path.to_string_lossy().as_ref(),
                    file_name,
                    file_size: 9,
                    modified_at: Some("1782634272"),
                    duration_ms: None,
                    recorded_at: None,
                    cover_path: None,
                    cover_source: "missing",
                },
            )
            .expect("clip should upsert");

            Self {
                database_path,
                clip_id: clip.id,
                clip_path,
                thumbnail_cache_root,
                _root: root,
            }
        }
    }

    impl Drop for ClipCommandFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self._root);
        }
    }

    fn request_clip_media(
        fixture: &ClipCommandFixture,
        method: Method,
        range: Option<&str>,
    ) -> Response<Vec<u8>> {
        let mut request = tauri::http::Request::builder().method(method).uri(format!(
            "https://clip-media.localhost/clip/{}",
            fixture.clip_id
        ));
        if let Some(range) = range {
            request = request.header(RANGE, range);
        }
        let request = request.body(Vec::new()).expect("request should build");

        clip_media_protocol_response(
            fixture.database_path.to_string_lossy().as_ref(),
            &fixture.thumbnail_cache_root,
            request,
        )
    }

    fn request_cover_media(fixture: &ClipCommandFixture, method: Method) -> Response<Vec<u8>> {
        let request = tauri::http::Request::builder()
            .method(method)
            .uri(format!(
                "https://clip-media.localhost/cover/{}",
                fixture.clip_id
            ))
            .body(Vec::new())
            .expect("cover request should build");
        clip_media_protocol_response(
            fixture.database_path.to_string_lossy().as_ref(),
            &fixture.thumbnail_cache_root,
            request,
        )
    }

    fn seed_generated_cover(fixture: &ClipCommandFixture, bytes: &[u8]) {
        fs::create_dir_all(&fixture.thumbnail_cache_root).unwrap();
        let connection = db::open_database(&fixture.database_path).unwrap();
        db::ensure_clip_thumbnails(&connection, &[fixture.clip_id]).unwrap();
        let job = db::claim_next_thumbnail_job(&connection, "2026-07-16 00:00:00")
            .unwrap()
            .unwrap();
        let cache_file = format!("{}-{}.jpg", job.clip_id, job.fingerprint);
        fs::write(fixture.thumbnail_cache_root.join(&cache_file), bytes).unwrap();
        assert!(db::complete_thumbnail_job_if_current(
            &connection,
            &job,
            &cache_file,
            bytes.len() as i64,
            &job.fingerprint,
        )
        .unwrap());
    }

    fn header(response: &Response<Vec<u8>>, name: tauri::http::HeaderName) -> &str {
        response
            .headers()
            .get(name)
            .expect("response header should be present")
            .to_str()
            .expect("response header should be text")
    }

    fn directory_file_names(path: &std::path::Path) -> BTreeSet<String> {
        fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn unique_temp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("vhm-clip-command-test-{unique}"))
    }
}
