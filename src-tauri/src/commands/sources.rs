use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::{run_scan_job, scan_execution_work, ScanCommandError, ScanJobResult, ScanJobStatus};
use crate::{
    critical_tasks::CriticalTaskGate,
    db::{self, SourceKind},
    scan_coordinator::ScanCoordinator,
    scanner,
    thumbnail::ThumbnailQueue,
    AppState,
};

const MAX_SOURCE_DISPLAY_NAME_CHARS: usize = 80;
const ENABLED_SOURCE_SYNC_ROOT_HINT: &str = "已启用视频来源";
const STARTUP_SYNC_RESTART_COOLDOWN_MINUTES: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupSourceSyncDecision {
    Run,
    SkipNoEnabledSources,
    SkipRestartCooldown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterScanSourceInput {
    pub source_kind: SourceKind,
    pub scan_root_path: String,
    pub display_name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub allow_overlap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSourceOverlap {
    pub id: i64,
    pub display_name: String,
    pub source_kind: SourceKind,
    pub scan_root_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterScanSourceResult {
    pub sources: Vec<db::Source>,
    pub created_count: usize,
    pub duplicate_count: usize,
    pub normalized_root_path: String,
    pub requires_overlap_confirmation: bool,
    pub overlaps: Vec<ScanSourceOverlap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocateScanSourceResult {
    pub preview: db::ScanSourceRelocationPreview,
    pub relocated_clip_count: usize,
    pub sync_job_id: Option<String>,
    pub sync_started: bool,
    pub sync_status: Option<ScanJobStatus>,
    pub sync_message: Option<String>,
}

fn default_enabled() -> bool {
    true
}

#[tauri::command]
pub fn register_scan_source(
    state: State<'_, AppState>,
    input: RegisterScanSourceInput,
) -> Result<RegisterScanSourceResult, String> {
    register_scan_source_for_database(&state.database_path, input)
}

pub(crate) fn register_scan_source_for_database(
    database_path: impl AsRef<Path>,
    input: RegisterScanSourceInput,
) -> Result<RegisterScanSourceResult, String> {
    let display_name = validate_display_name(&input.display_name)?;
    let scan_root = validate_scan_root(Path::new(input.scan_root_path.trim()))?;
    let normalized_root_path = persisted_source_path(&scan_root);
    let connection = db::open_database(database_path)?;
    let existing_sources = db::list_sources(&connection)?;
    let root_key = path_key(&scan_root);
    let overlaps = existing_sources
        .iter()
        .filter(|source| {
            let existing_key = normalized_path_key(&source.scan_root_path);
            existing_key != root_key && paths_overlap(&existing_key, &root_key)
        })
        .map(|source| ScanSourceOverlap {
            id: source.id,
            display_name: source.display_name.clone(),
            source_kind: source.source_kind,
            scan_root_path: source.scan_root_path.clone(),
        })
        .collect::<Vec<_>>();
    if !overlaps.is_empty() && !input.allow_overlap {
        return Ok(RegisterScanSourceResult {
            sources: Vec::new(),
            created_count: 0,
            duplicate_count: 0,
            normalized_root_path,
            requires_overlap_confirmation: true,
            overlaps,
        });
    }

    let source_paths = registration_source_paths(input.source_kind, &scan_root)?;
    let multiple_sources = source_paths.len() > 1;
    let mut source_ids = Vec::with_capacity(source_paths.len());
    let mut created_count = 0usize;
    let mut duplicate_count = 0usize;
    for source_path in source_paths {
        let source_path_text = persisted_source_path(&source_path);
        let existing = db::find_source_dir_by_normalized_path(&connection, &source_path_text)?;
        if existing.is_some() {
            duplicate_count += 1;
        } else {
            created_count += 1;
        }
        let source_name = if multiple_sources {
            format!(
                "{} · {}",
                display_name,
                source_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "ACLOS".to_string())
            )
        } else {
            display_name.clone()
        };
        let source = db::register_source_dir(
            &connection,
            db::SourceDirInput {
                path: &source_path_text,
                name: &source_name,
            },
            db::SourceProfileInput {
                source_kind: input.source_kind,
                scan_mode: input.source_kind.default_scan_mode(),
                scan_root_path: &normalized_root_path,
            },
            input.enabled,
        )?;
        source_ids.push(source.id);
    }

    let source_id_set = source_ids.into_iter().collect::<HashSet<_>>();
    let sources = db::list_sources(&connection)?
        .into_iter()
        .filter(|source| source_id_set.contains(&source.id))
        .collect();
    Ok(RegisterScanSourceResult {
        sources,
        created_count,
        duplicate_count,
        normalized_root_path,
        requires_overlap_confirmation: false,
        overlaps,
    })
}

#[tauri::command]
pub fn set_scan_source_enabled(
    state: State<'_, AppState>,
    source_id: i64,
    enabled: bool,
) -> Result<db::Source, String> {
    let connection = db::open_database(&state.database_path)?;
    db::set_source_dir_enabled(&connection, source_id, enabled)?;
    db::list_sources(&connection)?
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| format!("Source id {source_id} was not found"))
}

#[tauri::command]
pub fn preview_scan_source_relocation(
    state: State<'_, AppState>,
    source_id: i64,
    new_root_path: String,
) -> Result<db::ScanSourceRelocationPreview, String> {
    let connection = db::open_database_read_only(&state.database_path)?;
    db::preview_scan_source_relocation(&connection, source_id, Path::new(new_root_path.trim()))
}

#[tauri::command]
pub async fn relocate_scan_source(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: i64,
    new_root_path: String,
) -> Result<RelocateScanSourceResult, String> {
    let relocation_lease = state
        .critical_tasks
        .begin_source_relocation()
        .map_err(|snapshot| snapshot.busy_message())?;
    let database_path = state.database_path.clone();
    let commit_root = new_root_path.trim().to_string();
    let committed = tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open_database(&database_path)?;
        db::commit_scan_source_relocation(&connection, source_id, Path::new(&commit_root))
    })
    .await
    .map_err(|error| format!("来源重新定位任务失败：{error}"))??;

    // Scans are forbidden while the exclusive relocation lease is held. Drop it before the
    // single coordinated follow-up job; an updater/scan race after this point is recoverable and
    // reported as sync pending instead of rolling back the committed relocation.
    drop(relocation_lease);
    state.thumbnail_queue.reconcile_and_wake();

    let sync_result = sync_selected_sources_with_parts(
        app,
        state.database_path.clone(),
        state.scan_coordinator.clone(),
        state.critical_tasks.clone(),
        state.thumbnail_queue.clone(),
        committed.affected_source_ids.clone(),
        committed.preview.new_root_path.clone(),
    )
    .await;
    let (sync_started, sync_job_id, sync_status, sync_message) = match sync_result {
        Ok(result) => (
            true,
            Some(result.job_id),
            Some(result.status),
            Some(result.message),
        ),
        Err(error) => {
            let failed_job_id = error.job_id.clone();
            let failed_status = failed_job_id.as_ref().map(|_| ScanJobStatus::Failed);
            let sync_started = failed_job_id.is_some();
            eprintln!(
                "Source relocation committed but follow-up synchronization is pending: {}",
                error.message
            );
            (
                sync_started,
                failed_job_id,
                failed_status,
                Some(error.message),
            )
        }
    };

    Ok(RelocateScanSourceResult {
        preview: committed.preview,
        relocated_clip_count: committed.relocated_clip_count,
        sync_job_id,
        sync_started,
        sync_status,
        sync_message,
    })
}

#[tauri::command]
pub async fn sync_scan_source(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: i64,
) -> Result<ScanJobResult<scanner::ScanSummary>, ScanCommandError> {
    let root_hint = source_root_hint(&state.database_path, source_id)?;
    sync_one_source_with_parts(
        app,
        state.database_path.clone(),
        state.scan_coordinator.clone(),
        state.critical_tasks.clone(),
        state.thumbnail_queue.clone(),
        source_id,
        root_hint,
    )
    .await
}

#[tauri::command]
pub async fn sync_enabled_sources(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ScanJobResult<scanner::ScanSummary>, ScanCommandError> {
    sync_enabled_sources_with_parts(
        app,
        state.database_path.clone(),
        state.scan_coordinator.clone(),
        state.critical_tasks.clone(),
        state.thumbnail_queue.clone(),
    )
    .await
}

#[tauri::command]
pub fn request_startup_source_sync(app: AppHandle, state: State<'_, AppState>) {
    start_enabled_source_sync(app, state.inner().clone());
}

pub(crate) fn start_enabled_source_sync(app: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        let decision = db::open_database_read_only(&state.database_path)
            .and_then(|connection| startup_source_sync_decision(&connection));
        match decision {
            Ok(StartupSourceSyncDecision::SkipNoEnabledSources) => return,
            Ok(StartupSourceSyncDecision::SkipRestartCooldown) => {
                eprintln!(
                    "Startup source synchronization skipped: an enabled-source scan finished within the {STARTUP_SYNC_RESTART_COOLDOWN_MINUTES}-minute restart cooldown; manual synchronization remains available"
                );
                return;
            }
            Err(error) => {
                eprintln!("Startup source synchronization preflight failed: {error}");
                return;
            }
            Ok(StartupSourceSyncDecision::Run) => {}
        }

        if let Err(error) = sync_enabled_sources_with_parts(
            app,
            state.database_path,
            state.scan_coordinator,
            state.critical_tasks,
            state.thumbnail_queue,
        )
        .await
        {
            if error.code != "already-running" {
                eprintln!("Startup source synchronization failed: {}", error.message);
            }
        }
    });
}

fn startup_source_sync_decision(
    connection: &Connection,
) -> db::DbResult<StartupSourceSyncDecision> {
    if db::list_enabled_source_dirs(connection)?.is_empty() {
        return Ok(StartupSourceSyncDecision::SkipNoEnabledSources);
    }

    let cooldown = format!("-{STARTUP_SYNC_RESTART_COOLDOWN_MINUTES} minutes");
    let has_recent_enabled_source_scan = connection
        .query_row(
            "
            SELECT EXISTS (
                SELECT 1
                FROM scan_runs
                WHERE root_path = ?1
                  AND status IN ('completed', 'partial', 'cancelled', 'failed')
                  AND julianday(finished_at) >= julianday('now', ?2)
            )
            ",
            params![ENABLED_SOURCE_SYNC_ROOT_HINT, cooldown],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Database checking startup scan cooldown failed: {error}"))?;

    Ok(if has_recent_enabled_source_scan {
        StartupSourceSyncDecision::SkipRestartCooldown
    } else {
        StartupSourceSyncDecision::Run
    })
}

#[allow(clippy::too_many_arguments)]
async fn sync_one_source_with_parts(
    app: AppHandle,
    database_path: String,
    scan_coordinator: Arc<ScanCoordinator>,
    critical_tasks: Arc<CriticalTaskGate>,
    thumbnail_queue: Arc<ThumbnailQueue>,
    source_id: i64,
    root_hint: String,
) -> Result<ScanJobResult<scanner::ScanSummary>, ScanCommandError> {
    run_scan_job(
        app,
        database_path,
        scan_coordinator,
        critical_tasks,
        thumbnail_queue,
        root_hint,
        move |connection, job_id, cancellation, events| {
            scanner::sync_scan_source_with_progress_and_cancel(
                connection,
                source_id,
                job_id,
                cancellation,
                |progress| events.scanner_progress(progress),
            )
            .map(scan_execution_work)
        },
    )
    .await
}

async fn sync_enabled_sources_with_parts(
    app: AppHandle,
    database_path: String,
    scan_coordinator: Arc<ScanCoordinator>,
    critical_tasks: Arc<CriticalTaskGate>,
    thumbnail_queue: Arc<ThumbnailQueue>,
) -> Result<ScanJobResult<scanner::ScanSummary>, ScanCommandError> {
    run_scan_job(
        app,
        database_path,
        scan_coordinator,
        critical_tasks,
        thumbnail_queue,
        ENABLED_SOURCE_SYNC_ROOT_HINT.to_string(),
        |connection, job_id, cancellation, events| {
            scanner::sync_enabled_scan_sources_with_progress_and_cancel(
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

#[allow(clippy::too_many_arguments)]
async fn sync_selected_sources_with_parts(
    app: AppHandle,
    database_path: String,
    scan_coordinator: Arc<ScanCoordinator>,
    critical_tasks: Arc<CriticalTaskGate>,
    thumbnail_queue: Arc<ThumbnailQueue>,
    source_ids: Vec<i64>,
    root_hint: String,
) -> Result<ScanJobResult<scanner::ScanSummary>, ScanCommandError> {
    run_scan_job(
        app,
        database_path,
        scan_coordinator,
        critical_tasks,
        thumbnail_queue,
        root_hint,
        move |connection, job_id, cancellation, events| {
            scanner::sync_scan_sources_with_progress_and_cancel(
                connection,
                &source_ids,
                job_id,
                cancellation,
                |progress| events.scanner_progress(progress),
            )
            .map(scan_execution_work)
        },
    )
    .await
}

fn source_root_hint(database_path: &str, source_id: i64) -> Result<String, ScanCommandError> {
    db::open_database_read_only(database_path)
        .and_then(|connection| db::find_source_dir_by_id(&connection, source_id))
        .map(|source| source.scan_root_path)
        .map_err(|message| ScanCommandError {
            code: "source-not-found",
            message,
            job_id: None,
            active_job_id: None,
        })
}

fn validate_display_name(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("来源名称不能为空".to_string());
    }
    if value.chars().count() > MAX_SOURCE_DISPLAY_NAME_CHARS {
        return Err(format!(
            "来源名称最多 {MAX_SOURCE_DISPLAY_NAME_CHARS} 个字符"
        ));
    }
    Ok(value.to_string())
}

fn validate_scan_root(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("请选择视频来源目录".to_string());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法读取来源目录 {}：{error}", path.display()))?;
    if !metadata.is_dir() || metadata_is_reparse_point(&metadata) {
        return Err("视频来源必须是普通目录，不能是符号链接或 reparse point".to_string());
    }
    path.canonicalize()
        .map_err(|error| format!("无法规范化来源目录 {}：{error}", path.display()))
}

fn registration_source_paths(kind: SourceKind, root: &Path) -> Result<Vec<PathBuf>, String> {
    if kind != SourceKind::Aclos || is_aclos_source_directory(root) {
        return Ok(vec![root.to_path_buf()]);
    }
    let mut paths = fs::read_dir(root)
        .map_err(|error| format!("无法读取 ACLOS 根目录 {}：{error}", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_aclos_source_directory(path))
        .filter_map(|path| {
            let metadata = fs::symlink_metadata(&path).ok()?;
            if metadata.is_dir() && !metadata_is_reparse_point(&metadata) {
                path.canonicalize().ok()
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| normalized_path_key(&path.display().to_string()));
    if paths.is_empty() {
        paths.push(root.to_path_buf());
    }
    Ok(paths)
}

fn is_aclos_source_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("wonderfulvideos"))
}

fn path_key(path: &Path) -> String {
    normalized_path_key(&path.display().to_string())
}

fn persisted_source_path(path: &Path) -> String {
    db::stable_path_for_storage(&path.display().to_string())
}

fn normalized_path_key(path: &str) -> String {
    db::normalize_path(path).trim_end_matches('/').to_string()
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_directory(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "vhm-source-command-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("fixture directory should be created");
        path
    }

    #[test]
    fn persisted_source_paths_hide_windows_verbatim_drive_and_unc_prefixes() {
        assert_eq!(
            persisted_source_path(Path::new(r"\\?\D:\Captures\VALORANT")),
            r"D:\Captures\VALORANT"
        );
        assert_eq!(
            persisted_source_path(Path::new(r"\\?\UNC\server\share\VALORANT")),
            r"\\server\share\VALORANT"
        );
    }

    #[test]
    fn duplicate_registration_reuses_source_and_overlap_requires_confirmation() {
        let fixture = temp_directory("overlap");
        let data = fixture.join("data");
        let root = fixture.join("recordings");
        let child = root.join("nested");
        fs::create_dir_all(&data).expect("data directory should be created");
        fs::create_dir_all(&child).expect("source directories should be created");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");

        let first = register_scan_source_for_database(
            &database_path,
            RegisterScanSourceInput {
                source_kind: SourceKind::Nvidia,
                scan_root_path: root.display().to_string(),
                display_name: "NVIDIA".to_string(),
                enabled: true,
                allow_overlap: false,
            },
        )
        .expect("first source should register");
        assert_eq!(first.created_count, 1);
        assert!(!db::has_windows_verbatim_prefix(
            &first.normalized_root_path
        ));
        assert!(!db::has_windows_verbatim_prefix(
            &first.sources[0].scan_root_path
        ));

        let duplicate = register_scan_source_for_database(
            &database_path,
            RegisterScanSourceInput {
                source_kind: SourceKind::Nvidia,
                scan_root_path: root.display().to_string(),
                display_name: "NVIDIA Clips".to_string(),
                enabled: false,
                allow_overlap: false,
            },
        )
        .expect("duplicate should reuse the source");
        assert_eq!(duplicate.created_count, 0);
        assert_eq!(duplicate.duplicate_count, 1);
        assert!(!duplicate.sources[0].enabled);

        let overlap = register_scan_source_for_database(
            &database_path,
            RegisterScanSourceInput {
                source_kind: SourceKind::Generic,
                scan_root_path: child.display().to_string(),
                display_name: "Nested".to_string(),
                enabled: true,
                allow_overlap: false,
            },
        )
        .expect("overlap should return a confirmation result");
        assert!(overlap.requires_overlap_confirmation);
        assert_eq!(overlap.overlaps.len(), 1);
        assert_eq!(
            db::list_sources(&db::open_database(&database_path).unwrap())
                .unwrap()
                .len(),
            1
        );

        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn path_overlap_is_component_aware() {
        assert!(paths_overlap("c:/clips", "c:/clips/nested"));
        assert!(!paths_overlap("c:/clips", "c:/clips-old"));
    }

    #[test]
    fn startup_sync_skips_only_recent_enabled_source_scan() {
        let fixture = temp_directory("startup-cooldown");
        let data = fixture.join("data");
        let root = fixture.join("recordings");
        fs::create_dir_all(&data).expect("data directory should be created");
        fs::create_dir_all(&root).expect("source directory should be created");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");

        let connection = db::open_database(&database_path).expect("database should open");
        assert_eq!(
            startup_source_sync_decision(&connection).unwrap(),
            StartupSourceSyncDecision::SkipNoEnabledSources
        );
        drop(connection);

        register_scan_source_for_database(
            &database_path,
            RegisterScanSourceInput {
                source_kind: SourceKind::Generic,
                scan_root_path: root.display().to_string(),
                display_name: "Recordings".to_string(),
                enabled: true,
                allow_overlap: false,
            },
        )
        .expect("source should register");

        let connection = db::open_database(&database_path).expect("database should reopen");
        assert_eq!(
            startup_source_sync_decision(&connection).unwrap(),
            StartupSourceSyncDecision::Run
        );
        connection
            .execute(
                "INSERT INTO scan_runs (job_id, root_path, status, finished_at) VALUES (?1, ?2, 'completed', CURRENT_TIMESTAMP)",
                params!["recent-enabled-source-sync", ENABLED_SOURCE_SYNC_ROOT_HINT],
            )
            .expect("recent scan should insert");
        assert_eq!(
            startup_source_sync_decision(&connection).unwrap(),
            StartupSourceSyncDecision::SkipRestartCooldown
        );

        connection
            .execute(
                "UPDATE scan_runs SET finished_at = datetime('now', '-11 minutes') WHERE job_id = ?1",
                params!["recent-enabled-source-sync"],
            )
            .expect("scan timestamp should age");
        assert_eq!(
            startup_source_sync_decision(&connection).unwrap(),
            StartupSourceSyncDecision::Run
        );

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }
}
