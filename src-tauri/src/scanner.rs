mod reconnect;
mod recursive_mp4;
mod scan_runs;
mod scan_service;

use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose, Engine as _};
use rusqlite::Connection;
use serde::Serialize;

use crate::db::{self, ClipInput, ClipSaveOutcome, DbResult};
use crate::metadata_ingest::{ingest_match_metadata, MetadataIngestInput};

use reconnect::{
    canonicalize_non_reparse_directory_chain, canonicalize_regular_path_within_root,
    collect_scan_candidate, concrete_file_identity_changed, metadata_is_reparse_point,
    revalidate_staged_candidate, CollectedScanCandidate, IdentityReadDiagnostics,
    ScanReconnectPlanGuard,
};

use scan_runs::ScanRunGuard;
pub use scan_runs::{
    ensure_scan_run_started, ensure_scan_run_terminal, finalize_scan_run_for_job,
    latest_scan_summary, mark_scan_run_cancelling, recover_interrupted_scan_runs,
    scan_summary_for_job,
};
#[cfg(test)]
use scan_service::scan_library_roots;
pub use scan_service::{
    default_aclos_dir, scan_custom_directory, scan_custom_directory_with_progress,
    scan_custom_directory_with_progress_and_cancel, scan_default_aclos_library,
    scan_default_aclos_library_with_progress, scan_default_aclos_library_with_progress_and_cancel,
    scan_directory, scan_directory_with_progress, scan_discovered_aclos_roots_with_progress,
    scan_discovered_aclos_roots_with_progress_and_cancel, scan_roots, scan_roots_with_progress,
    scan_roots_with_progress_and_cancel, sync_enabled_scan_sources_with_progress_and_cancel,
    sync_scan_source_with_progress_and_cancel, sync_scan_sources,
    sync_scan_sources_with_progress_and_cancel,
};

const EDIT_AGENT_ASSET_WINDOW_SECONDS: i64 = 60;
const VIDEO_EXPORT_CONFIG_WINDOW_SECONDS: i64 = 120;
pub(crate) const MAX_SCAN_ERROR_SAMPLES: usize = 200;
pub(crate) const MAX_SCAN_ERROR_MESSAGE_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub root_path: String,
    pub source_dir_count: i64,
    pub clip_group_count: i64,
    pub new_clip_count: i64,
    pub updated_clip_count: i64,
    pub missing_clip_count: i64,
    /// NVIDIA videos discovered but held back for manual classification; never auto-imported.
    pub pending_clip_count: i64,
    pub cover_missing_count: i64,
    pub metadata_match_count: i64,
    pub metadata_enriched_clip_count: i64,
    pub metadata_event_count: i64,
    pub metadata_warning_count: i64,
    #[serde(skip)]
    pub(crate) omitted_error_count: i64,
    pub errors: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub phase: String,
    pub root_path: String,
    pub source: Option<String>,
    pub current: i64,
    pub total: i64,
    pub source_dir_count: i64,
    pub clip_group_count: i64,
    pub clip_file_count: i64,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanProgressPhase {
    Discovering,
    Scanning,
    Importing,
    Metadata,
    Finalizing,
    Completed,
    Partial,
    Cancelled,
}

impl ScanProgressPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Discovering => "discovering",
            Self::Scanning => "scanning",
            Self::Importing => "importing",
            Self::Metadata => "metadata",
            Self::Finalizing => "finalizing",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanExecutionStatus {
    Completed,
    Partial,
    Cancelled,
}

impl ScanExecutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanExecution {
    pub status: ScanExecutionStatus,
    pub summary: ScanSummary,
}

#[derive(Clone, Copy, Default)]
struct ScanRuntime<'a> {
    job_id: Option<&'a str>,
    cancellation: Option<&'a AtomicBool>,
}

impl ScanRuntime<'_> {
    fn is_cancelled(self) -> bool {
        self.cancellation
            .is_some_and(|cancellation| cancellation.load(Ordering::Acquire))
    }
}

type ScanProgressReporter<'a> = &'a dyn Fn(ScanProgress);

struct ScanProgressState<'a> {
    reporter: Option<ScanProgressReporter<'a>>,
    root_path: String,
    source: Option<String>,
    current: i64,
    total: i64,
    source_dir_count: i64,
    clip_group_count: i64,
    clip_file_count: i64,
}

impl<'a> ScanProgressState<'a> {
    fn new(root_path: String, reporter: Option<ScanProgressReporter<'a>>) -> Self {
        Self {
            reporter,
            root_path,
            source: None,
            current: 0,
            total: 0,
            source_dir_count: 0,
            clip_group_count: 0,
            clip_file_count: 0,
        }
    }

    fn set_total_sources(&mut self, total: usize) {
        self.total = total.min(i64::MAX as usize) as i64;
    }

    fn emit(&self, phase: ScanProgressPhase, message: impl Into<String>) {
        let Some(reporter) = self.reporter else {
            return;
        };

        reporter(ScanProgress {
            phase: phase.as_str().to_string(),
            root_path: self.root_path.clone(),
            source: self.source.clone(),
            current: self.current,
            total: self.total,
            source_dir_count: self.source_dir_count,
            clip_group_count: self.clip_group_count,
            clip_file_count: self.clip_file_count,
            message: message.into(),
        });
    }

    fn source_started(&mut self, source_name: &str, source_path: &Path) {
        self.source = Some(path_to_string(source_path));
        self.emit(
            ScanProgressPhase::Scanning,
            format!("正在扫描 {source_name}"),
        );
    }

    fn clip_scanned(&mut self, file_name: &str) {
        self.clip_file_count += 1;
        self.emit(
            ScanProgressPhase::Scanning,
            format!("已扫描文件 {file_name}"),
        );
    }

    fn group_scanned(&mut self, group_name: &str) {
        self.clip_group_count += 1;
        self.emit(
            ScanProgressPhase::Scanning,
            format!("已扫描分组 {group_name}"),
        );
    }

    fn source_finished(&mut self, source_name: &str) {
        self.current += 1;
        self.source_dir_count += 1;
        self.emit(
            ScanProgressPhase::Scanning,
            format!("已完成来源 {source_name}"),
        );
        self.source = None;
    }

    fn source_interrupted(&mut self, source_name: &str) {
        self.emit(
            ScanProgressPhase::Scanning,
            format!("已停止扫描来源 {source_name}"),
        );
        self.source = None;
    }
}

impl ScanSummary {
    pub fn empty(root_path: String) -> Self {
        Self {
            root_path,
            source_dir_count: 0,
            clip_group_count: 0,
            new_clip_count: 0,
            updated_clip_count: 0,
            missing_clip_count: 0,
            pending_clip_count: 0,
            cover_missing_count: 0,
            metadata_match_count: 0,
            metadata_enriched_clip_count: 0,
            metadata_event_count: 0,
            metadata_warning_count: 0,
            omitted_error_count: 0,
            errors: Vec::new(),
            message: None,
        }
    }

    pub(crate) fn push_error(&mut self, error: String) {
        let error = truncate_utf8_bytes(error, MAX_SCAN_ERROR_MESSAGE_BYTES);
        if self.errors.contains(&error) {
            return;
        }
        if self.errors.len() >= MAX_SCAN_ERROR_SAMPLES {
            self.omitted_error_count = self.omitted_error_count.saturating_add(1);
            return;
        }
        self.errors.push(error);
    }

    pub(crate) fn merge_errors(&mut self, errors: impl IntoIterator<Item = String>) {
        for error in errors {
            self.push_error(error);
        }
    }
}

struct MetadataScanConfig {
    anchors: Vec<PathBuf>,
    allow_external_fallback: bool,
    account_hint_scopes: Vec<PathBuf>,
    use_local_account_hint_scopes: bool,
}

struct ScanBatchInput {
    requested_roots: Vec<PathBuf>,
    source_paths: Vec<PathBuf>,
    persistent_sources: Vec<db::SourceDir>,
    metadata_config: MetadataScanConfig,
    initial_errors: Vec<String>,
    empty_message: Option<String>,
}

struct LibrarySourceDiscovery {
    roots: Vec<PathBuf>,
    sources: Vec<PathBuf>,
    errors: Vec<String>,
    empty_message: Option<String>,
}

struct AclosScanGroup {
    path: PathBuf,
    name: String,
    clips: Vec<(PathBuf, i64)>,
    /// Clips sharing `path` for cover discovery. Equal to `clips.len()` for a per-match
    /// directory, but larger when several single-clip groups come out of one shared
    /// directory, so the lone-cover fallback cannot reuse one cover across unrelated clips.
    cover_scope_clip_count: usize,
}

struct CoverJpegDiscovery {
    paths: Vec<PathBuf>,
    warnings: Vec<String>,
}

struct SourceScanOutcome {
    source_path: PathBuf,
    source_id: Option<i64>,
    accessible: bool,
    metadata_eligible: bool,
    complete_for_missing: bool,
    seen_paths: HashSet<String>,
}

struct SourceScanStep {
    outcome: SourceScanOutcome,
    cancelled: bool,
}

impl SourceScanStep {
    fn finished(outcome: SourceScanOutcome) -> Self {
        Self {
            outcome,
            cancelled: false,
        }
    }

    fn cancelled(outcome: SourceScanOutcome) -> Self {
        Self {
            outcome,
            cancelled: true,
        }
    }
}

fn push_unique_scan_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    let normalized = scan_path_key(&root);
    if roots
        .iter()
        .any(|existing| scan_path_key(existing) == normalized)
    {
        return;
    }
    roots.push(root);
}

struct CachedMetadataIngest {
    leveldb_result: crate::leveldb_reader::LevelDbBattleListResult,
    log_result: crate::highlight_log_parser::HighlightLogParseResult,
    wonderful_result: crate::wonderful_db::WonderfulDbReadResult,
    local_account_hint_scope: Option<PathBuf>,
    errors: Vec<String>,
}

fn discover_library_sources(
    roots: &[PathBuf],
    source_path_filter: Option<&HashSet<String>>,
) -> LibrarySourceDiscovery {
    let roots = normalize_unique_scan_paths(roots.iter().map(PathBuf::as_path));
    let mut sources = Vec::new();
    let mut errors = Vec::new();
    let mut empty_message = None;

    for root in &roots {
        let root_path = path_to_string(root);
        let metadata = match fs::metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if roots.len() == 1 {
                    empty_message = Some(format!("Scan root not found: {root_path}"));
                }
                continue;
            }
            Err(error) => {
                push_unique_error(
                    &mut errors,
                    format!("Failed to inspect scan root {root_path}: {error}"),
                );
                continue;
            }
        };
        if !metadata.is_dir() {
            if roots.len() == 1 {
                empty_message = Some(format!("Scan root is not a directory: {root_path}"));
            }
            continue;
        }

        let entries = match read_sorted_entries(root) {
            Ok(entries) => entries,
            Err(error) => {
                push_unique_error(&mut errors, error);
                continue;
            }
        };
        for source_path in entries {
            if !source_path.is_dir() || !is_source_directory_name(&source_path) {
                continue;
            }
            if source_path_filter
                .is_some_and(|filter| !filter.contains(&scan_path_key(&source_path)))
            {
                continue;
            }
            push_unique_scan_root(&mut sources, source_path);
        }
    }

    LibrarySourceDiscovery {
        roots,
        sources,
        errors,
        empty_message,
    }
}

fn run_scan_batch(
    connection: &Connection,
    input: ScanBatchInput,
    progress_reporter: Option<ScanProgressReporter<'_>>,
    runtime: ScanRuntime<'_>,
) -> DbResult<ScanExecution> {
    let ScanBatchInput {
        requested_roots,
        source_paths,
        persistent_sources,
        metadata_config,
        initial_errors,
        empty_message,
    } = input;
    let requested_roots = normalize_unique_scan_paths(requested_roots.iter().map(PathBuf::as_path));
    let source_paths = normalize_unique_scan_paths(source_paths.iter().map(PathBuf::as_path));
    let root_path = requested_roots
        .iter()
        .map(|root| path_to_string(root))
        .collect::<Vec<_>>()
        .join("; ");
    let mut summary = ScanSummary::empty(root_path.clone());
    summary.merge_errors(initial_errors);
    let scan_run = ScanRunGuard::start(connection, runtime.job_id, &root_path)?;
    let mut progress = ScanProgressState::new(root_path.clone(), progress_reporter);
    progress.emit(
        ScanProgressPhase::Discovering,
        format!("正在准备扫描 {root_path}"),
    );

    if runtime.is_cancelled() {
        return finish_cancelled_scan(scan_run, &progress, summary);
    }

    if source_paths.is_empty() && persistent_sources.is_empty() {
        summary.message = empty_message.or_else(|| {
            Some(format!(
                "Scan completed: {} sources, {} groups",
                summary.source_dir_count, summary.clip_group_count
            ))
        });
        let status = completed_status(&summary);
        return finish_scan_execution(scan_run, &progress, status, summary);
    }

    let total_sources = source_paths.len().saturating_add(persistent_sources.len());
    progress.set_total_sources(total_sources);
    progress.emit(
        ScanProgressPhase::Scanning,
        format!("发现 {total_sources} 个来源目录"),
    );
    let mut outcomes = Vec::with_capacity(total_sources);
    for source_path in source_paths {
        if runtime.is_cancelled() {
            return finish_cancelled_scan(scan_run, &progress, summary);
        }
        let scan_root_path = scan_root_for_source(&requested_roots, &source_path);
        let registered_source = registered_recursive_source_for_path(connection, &source_path)?;
        let step = match registered_source {
            Some(source) if source.scan_mode == db::ScanMode::RecursiveMp4 => {
                recursive_mp4::scan_recursive_source(
                    connection,
                    source,
                    &mut summary,
                    &mut progress,
                    runtime,
                )?
            }
            _ => scan_one_source(
                connection,
                source_path,
                scan_root_path,
                None,
                &mut summary,
                &mut progress,
                runtime,
            )?,
        };
        outcomes.push(step.outcome);
        if step.cancelled {
            return finish_cancelled_scan(scan_run, &progress, summary);
        }
    }
    for source in persistent_sources {
        if runtime.is_cancelled() {
            return finish_cancelled_scan(scan_run, &progress, summary);
        }
        let step = match source.scan_mode {
            db::ScanMode::AclosStructured => scan_one_source(
                connection,
                PathBuf::from(&source.path),
                PathBuf::from(&source.scan_root_path),
                Some(&source),
                &mut summary,
                &mut progress,
                runtime,
            )?,
            db::ScanMode::RecursiveMp4 => recursive_mp4::scan_recursive_source(
                connection,
                source,
                &mut summary,
                &mut progress,
                runtime,
            )?,
        };
        outcomes.push(step.outcome);
        if step.cancelled {
            return finish_cancelled_scan(scan_run, &progress, summary);
        }
    }

    progress.emit(ScanProgressPhase::Importing, "扫描完成，正在导入数据");
    if !reconcile_missing_clips(connection, &outcomes, &mut summary, runtime)? {
        return finish_cancelled_scan(scan_run, &progress, summary);
    }

    let accessible_sources = outcomes
        .iter()
        .filter(|outcome| outcome.accessible && outcome.metadata_eligible)
        .map(|outcome| outcome.source_path.clone())
        .collect::<Vec<_>>();
    if !accessible_sources.is_empty() {
        if runtime.is_cancelled() {
            return finish_cancelled_scan(scan_run, &progress, summary);
        }
        progress.emit(ScanProgressPhase::Metadata, "正在导入对局元数据");
        let mut effective_account_hint_scopes = normalize_unique_scan_paths(
            metadata_config
                .account_hint_scopes
                .iter()
                .map(PathBuf::as_path),
        );
        let metadata_anchors =
            normalize_metadata_scan_anchors(if metadata_config.anchors.is_empty() {
                normalize_unique_scan_paths(
                    accessible_sources
                        .iter()
                        .map(|source| metadata_scan_root_for_source(source))
                        .collect::<Vec<_>>()
                        .iter()
                        .map(PathBuf::as_path),
                )
            } else {
                normalize_unique_scan_paths(metadata_config.anchors.iter().map(PathBuf::as_path))
            });
        // The fallback decision is per anchor, not per batch: `metadata_source_paths` only reaches
        // the AppData root when the anchor itself carries no `WonderfulDb`/`logs`/`Local Storage`.
        // Gating it on a single anchor stranded every multi-account library, because the default
        // layout keeps recordings under `AppData\ACLOS` while metadata lives in `AppData\Roaming\ACLOS`.
        let allow_metadata_fallback = metadata_config.allow_external_fallback;
        let mut wonderful_accounts = Vec::new();
        let mut wonderful_snapshot_accounts = Vec::new();
        for metadata_anchor in metadata_anchors {
            if !metadata_anchor_has_sources(&metadata_anchor, allow_metadata_fallback) {
                continue;
            }
            let snapshot = collect_metadata_snapshot(&metadata_anchor, allow_metadata_fallback);
            if runtime.is_cancelled() {
                return finish_cancelled_scan(scan_run, &progress, summary);
            }
            let local_account_hint_scope = metadata_config
                .use_local_account_hint_scopes
                .then(|| snapshot.local_account_hint_scope.clone())
                .flatten();
            if let Some(scope) = local_account_hint_scope.as_ref() {
                push_unique_scan_root(&mut effective_account_hint_scopes, scope.clone());
            }
            let account_hint_scope = local_account_hint_scope.or_else(|| {
                effective_account_hint_scopes
                    .iter()
                    .filter(|scope| {
                        path_is_within(&metadata_anchor, scope)
                            || path_is_within(scope, &metadata_anchor)
                    })
                    .max_by_key(|scope| scan_path_key(scope).len())
                    .cloned()
            });
            run_metadata_ingest(
                connection,
                &mut summary,
                &snapshot,
                account_hint_scope.as_deref(),
            );
            wonderful_accounts.extend(snapshot.wonderful_result.accounts.iter().cloned());
            wonderful_snapshot_accounts
                .extend(snapshot.wonderful_result.snapshot_accounts.iter().cloned());
        }
        if let Err(error) = crate::wonderful_ingest::propagate_latest_wonderful_account_names(
            connection,
            &wonderful_accounts,
            &wonderful_snapshot_accounts,
        ) {
            summary.metadata_warning_count += 1;
            summary.push_error(error);
        }

        if runtime.is_cancelled() {
            return finish_cancelled_scan(scan_run, &progress, summary);
        }
        finalize_scanned_metadata(
            connection,
            &accessible_sources,
            &effective_account_hint_scopes,
            &mut summary,
        )?;
        if runtime.is_cancelled() {
            return finish_cancelled_scan(scan_run, &progress, summary);
        }
    }

    progress.emit(ScanProgressPhase::Finalizing, "正在完成扫描收尾");
    if runtime.is_cancelled() {
        return finish_cancelled_scan(scan_run, &progress, summary);
    }

    summary.message = Some(if summary.errors.is_empty() {
        format!(
            "Scan completed: {} roots, {} sources, {} groups",
            requested_roots.len(),
            summary.source_dir_count,
            summary.clip_group_count
        )
    } else {
        format!(
            "Scan completed with warnings: {} roots, {} sources, {} groups",
            requested_roots.len(),
            summary.source_dir_count,
            summary.clip_group_count
        )
    });
    let status = completed_status(&summary);
    if status == ScanExecutionStatus::Completed {
        let completed_source_ids = outcomes
            .iter()
            .filter(|outcome| outcome.accessible && outcome.complete_for_missing)
            .filter_map(|outcome| outcome.source_id)
            .collect::<Vec<_>>();
        db::mark_source_dirs_scan_completed(connection, &completed_source_ids)?;
    }
    finish_scan_execution(scan_run, &progress, status, summary)
}

fn registered_recursive_source_for_path(
    connection: &Connection,
    source_path: &Path,
) -> DbResult<Option<db::SourceDir>> {
    let source_key = scan_path_key(source_path);
    let matches = db::list_source_dirs(connection)?
        .into_iter()
        .filter(|source| source.scan_mode == db::ScanMode::RecursiveMp4)
        .filter(|source| {
            scan_path_key(Path::new(&source.path)) == source_key
                || scan_path_key(Path::new(&source.scan_root_path)) == source_key
        })
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [] => Ok(None),
        [source] => Ok(Some(source.clone())),
        _ => Err(format!(
            "Multiple recursive sources resolve to the same scan root: {}",
            source_path.display()
        )),
    }
}

fn completed_status(summary: &ScanSummary) -> ScanExecutionStatus {
    if summary.errors.is_empty() {
        ScanExecutionStatus::Completed
    } else {
        ScanExecutionStatus::Partial
    }
}

fn finish_cancelled_scan(
    scan_run: ScanRunGuard<'_>,
    progress: &ScanProgressState<'_>,
    mut summary: ScanSummary,
) -> DbResult<ScanExecution> {
    summary.message = Some(format!(
        "Scan cancelled after {} sources and {} groups",
        summary.source_dir_count, summary.clip_group_count
    ));
    finish_scan_execution(scan_run, progress, ScanExecutionStatus::Cancelled, summary)
}

fn finish_scan_execution(
    scan_run: ScanRunGuard<'_>,
    progress: &ScanProgressState<'_>,
    status: ScanExecutionStatus,
    summary: ScanSummary,
) -> DbResult<ScanExecution> {
    scan_run.finish(status.as_str(), &summary)?;
    let phase = match status {
        ScanExecutionStatus::Completed => ScanProgressPhase::Completed,
        ScanExecutionStatus::Partial => ScanProgressPhase::Partial,
        ScanExecutionStatus::Cancelled => ScanProgressPhase::Cancelled,
    };
    progress.emit(
        phase,
        summary
            .message
            .clone()
            .unwrap_or_else(|| status.as_str().to_string()),
    );
    Ok(ScanExecution { status, summary })
}

fn scan_one_source(
    connection: &Connection,
    source_path: PathBuf,
    scan_root_path: PathBuf,
    registered_source: Option<&db::SourceDir>,
    summary: &mut ScanSummary,
    progress: &mut ScanProgressState<'_>,
    runtime: ScanRuntime<'_>,
) -> DbResult<SourceScanStep> {
    let source_name = registered_source
        .map(|source| source.name.clone())
        .unwrap_or_else(|| path_file_name(&source_path));
    let source_path_string = path_to_string(&source_path);
    let scan_root_path_string = path_to_string(&scan_root_path);
    progress.source_started(&source_name, &source_path);
    summary.source_dir_count += 1;

    if runtime.is_cancelled() {
        return Ok(cancelled_source_step(
            progress,
            &source_name,
            source_path,
            None,
            HashSet::new(),
        ));
    }

    let source_dir_result = registered_source.cloned().map_or_else(
        || {
            db::upsert_source_dir_with_profile(
                connection,
                db::SourceDirInput {
                    path: &source_path_string,
                    name: &source_name,
                },
                db::SourceProfileInput::aclos(&scan_root_path_string),
            )
        },
        Ok,
    );
    let source_dir = match source_dir_result {
        Ok(source_dir) => source_dir,
        Err(error) => {
            push_source_error(summary, &source_path, &error);
            progress.source_finished(&source_name);
            return Ok(SourceScanStep::finished(SourceScanOutcome {
                source_path,
                source_id: None,
                accessible: false,
                metadata_eligible: true,
                complete_for_missing: false,
                seen_paths: HashSet::new(),
            }));
        }
    };

    if runtime.is_cancelled() {
        return Ok(cancelled_source_step(
            progress,
            &source_name,
            source_path,
            Some(source_dir.id),
            HashSet::new(),
        ));
    }

    let mut source_errors = Vec::new();
    let canonical_scan_root = match canonicalize_non_reparse_directory_chain(&scan_root_path) {
        Ok(path) => path,
        Err(error) => {
            let error = format!(
                "Failed to validate ACLOS scan root path chain {scan_root_path_string}: {error}"
            );
            push_source_error(summary, &source_path, &error);
            db::mark_source_dir_scan_error(connection, source_dir.id, "unavailable", &error)?;
            progress.source_finished(&source_name);
            return Ok(SourceScanStep::finished(SourceScanOutcome {
                source_path,
                source_id: Some(source_dir.id),
                accessible: false,
                metadata_eligible: true,
                complete_for_missing: false,
                seen_paths: HashSet::new(),
            }));
        }
    };
    let canonical_source_path = match canonicalize_non_reparse_directory_chain(&source_path) {
        Ok(path) => path,
        Err(error) => {
            let error =
                format!("Failed to validate ACLOS source path chain {source_path_string}: {error}");
            push_source_error(summary, &source_path, &error);
            db::mark_source_dir_scan_error(connection, source_dir.id, "unavailable", &error)?;
            progress.source_finished(&source_name);
            return Ok(SourceScanStep::finished(SourceScanOutcome {
                source_path,
                source_id: Some(source_dir.id),
                accessible: false,
                metadata_eligible: true,
                complete_for_missing: false,
                seen_paths: HashSet::new(),
            }));
        }
    };
    if canonical_source_path != canonical_scan_root
        && !canonical_source_path.starts_with(&canonical_scan_root)
    {
        let error = format!(
            "ACLOS source canonical path is outside its authorized scan root: {}",
            canonical_source_path.display()
        );
        push_source_error(summary, &source_path, &error);
        db::mark_source_dir_scan_error(connection, source_dir.id, "unavailable", &error)?;
        progress.source_finished(&source_name);
        return Ok(SourceScanStep::finished(SourceScanOutcome {
            source_path,
            source_id: Some(source_dir.id),
            accessible: false,
            metadata_eligible: true,
            complete_for_missing: false,
            seen_paths: HashSet::new(),
        }));
    }

    let entries = match read_sorted_entries(&source_path) {
        Ok(entries) => entries,
        Err(error) => {
            push_source_error(summary, &source_path, &error);
            db::mark_source_dir_scan_error(connection, source_dir.id, "unavailable", &error)?;
            progress.source_finished(&source_name);
            return Ok(SourceScanStep::finished(SourceScanOutcome {
                source_path,
                source_id: Some(source_dir.id),
                accessible: false,
                metadata_eligible: true,
                complete_for_missing: false,
                seen_paths: HashSet::new(),
            }));
        }
    };

    if runtime.is_cancelled() {
        return Ok(cancelled_source_step(
            progress,
            &source_name,
            source_path,
            Some(source_dir.id),
            HashSet::new(),
        ));
    }

    let parsed_configs = match crate::metadata::parse_video_export_configs(&source_path) {
        Ok(configs) => configs,
        Err(error) => {
            push_source_error(summary, &source_path, &error);
            push_unique_error(&mut source_errors, error);
            Vec::new()
        }
    };
    let mut complete_for_missing = true;
    let mut seen_paths = HashSet::new();
    let mut raw_groups = Vec::new();
    for path in entries {
        let entry_metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                complete_for_missing = false;
                let error = format!("Failed to inspect source entry {}: {error}", path.display());
                push_source_error(summary, &source_path, &error);
                push_unique_error(&mut source_errors, error);
                continue;
            }
        };
        if metadata_is_reparse_point(&entry_metadata) {
            complete_for_missing = false;
            let error = format!("Skipped symbolic link or reparse point: {}", path.display());
            push_source_error(summary, &source_path, &error);
            push_unique_error(&mut source_errors, error);
            continue;
        }
        if entry_metadata.is_dir() {
            match canonicalize_regular_path_within_root(&path, &canonical_source_path) {
                Ok(_) => {
                    let dir_name = path_file_name(&path);
                    if is_aclos_per_clip_group_dir(&dir_name) {
                        // Files here belong to unrelated matches, so each becomes its own group.
                        match mp4_files_in_dir(&path) {
                            Ok(files) => {
                                let shared_clip_count = files.len();
                                for file in files {
                                    let group_name = clip_group_name_for_file(&file);
                                    raw_groups.push((
                                        path.clone(),
                                        group_name,
                                        Some(file),
                                        shared_clip_count,
                                    ));
                                }
                            }
                            Err(error) => {
                                complete_for_missing = false;
                                push_source_error(summary, &source_path, &error);
                                push_unique_error(&mut source_errors, error);
                            }
                        }
                    } else {
                        raw_groups.push((path.clone(), dir_name, None, 0));
                    }
                }
                Err(error) => {
                    complete_for_missing = false;
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                }
            }
        } else if entry_metadata.is_file() && has_extension(&path, "mp4") {
            let group_name = clip_group_name_for_file(&path);
            match canonicalize_regular_path_within_root(&path, &canonical_source_path) {
                Ok(_) => raw_groups.push((source_path.clone(), group_name, Some(path), 0)),
                Err(error) => {
                    complete_for_missing = false;
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                }
            }
        }
    }

    let _reconnect_plan = ScanReconnectPlanGuard::begin(connection, source_dir.id)?;
    let mut identity_diagnostics = IdentityReadDiagnostics::default();
    let mut scan_groups = Vec::with_capacity(raw_groups.len());
    for (group_path, group_name, root_clip_path, shared_dir_clip_count) in raw_groups {
        if runtime.is_cancelled() {
            return Ok(cancelled_source_step(
                progress,
                &source_name,
                source_path,
                Some(source_dir.id),
                seen_paths,
            ));
        }
        let discovered_files = if let Some(root_clip_path) = root_clip_path {
            vec![root_clip_path]
        } else {
            match mp4_files_in_dir(&group_path) {
                Ok(files) => files,
                Err(error) => {
                    complete_for_missing = false;
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                    continue;
                }
            }
        };
        let mut clips = Vec::with_capacity(discovered_files.len());
        for file in discovered_files {
            if let Err(error) = canonicalize_regular_path_within_root(&file, &canonical_source_path)
            {
                complete_for_missing = false;
                push_source_error(summary, &source_path, &error);
                push_unique_error(&mut source_errors, error);
                continue;
            }
            let candidate = match collect_scan_candidate(file, &mut identity_diagnostics) {
                Ok(candidate) => candidate,
                Err(error) => {
                    complete_for_missing = false;
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                    continue;
                }
            };
            seen_paths.insert(candidate.normalized_path.clone());
            let db::StageScanReconnectCandidateOutcome::Staged(candidate_id) =
                db::stage_scan_reconnect_candidate(
                    connection,
                    candidate.stage_input(source_dir.id),
                )?;
            clips.push((candidate.path, candidate_id));
        }
        if !clips.is_empty() {
            let cover_scope_clip_count = shared_dir_clip_count.max(clips.len());
            scan_groups.push(AclosScanGroup {
                path: group_path,
                name: group_name,
                clips,
                cover_scope_clip_count,
            });
        }
    }
    if runtime.is_cancelled() {
        return Ok(cancelled_source_step(
            progress,
            &source_name,
            source_path,
            Some(source_dir.id),
            seen_paths,
        ));
    }
    db::finalize_scan_reconnect_plan(connection, source_dir.id)?;

    for group in scan_groups {
        if runtime.is_cancelled() {
            return Ok(cancelled_source_step(
                progress,
                &source_name,
                source_path,
                Some(source_dir.id),
                seen_paths,
            ));
        }
        let group_path = group.path;
        let group_name = group.name;
        let staged_clips = group.clips;
        let cover_scope_clip_count = group.cover_scope_clip_count;

        let clip_group = match db::upsert_clip_group(
            connection,
            db::ClipGroupInput {
                source_dir_id: source_dir.id,
                group_key: &group_name,
                display_name: &group_name,
            },
        ) {
            Ok(group) => group,
            Err(error) => {
                complete_for_missing = false;
                push_source_error(summary, &source_path, &error);
                push_unique_error(&mut source_errors, error);
                continue;
            }
        };
        summary.clip_group_count += 1;

        let cover_paths = match find_cover_jpegs(&group_path, &canonical_source_path) {
            Ok(discovery) => {
                for error in discovery.warnings {
                    complete_for_missing = false;
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                }
                discovery.paths
            }
            Err(error) => {
                complete_for_missing = false;
                push_source_error(summary, &source_path, &error);
                push_unique_error(&mut source_errors, error);
                Vec::new()
            }
        };
        let clip_count = cover_scope_clip_count.max(staged_clips.len());

        for (clip_path, candidate_id) in &staged_clips {
            if runtime.is_cancelled() {
                return Ok(cancelled_source_step(
                    progress,
                    &source_name,
                    source_path,
                    Some(source_dir.id),
                    seen_paths,
                ));
            }
            let decision = match db::resolve_scan_reconnect_candidate(
                connection,
                source_dir.id,
                *candidate_id,
            ) {
                Ok(decision) => decision,
                Err(error) => {
                    complete_for_missing = false;
                    let error = format!(
                        "Failed to resolve clip reconnect plan {}: {error}",
                        path_to_string(clip_path)
                    );
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                    continue;
                }
            };
            let current = match revalidate_staged_candidate(
                decision.candidate(),
                &mut identity_diagnostics,
            ) {
                Ok(candidate) => candidate,
                Err(error) => {
                    complete_for_missing = false;
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                    continue;
                }
            };
            let modified_at_unix = current.modified_at.parse::<i64>().ok();
            let file_name = current.file_name.clone();
            let selected_metadata = select_video_export_metadata(&parsed_configs, modified_at_unix);
            let cover_path = select_cover_for_clip(clip_path, &cover_paths, clip_count);
            let cover_path_string = cover_path.map(path_to_string);
            let cover_source = if cover_path_string.is_some() {
                "file"
            } else {
                "missing"
            };

            let saved = match persist_aclos_scan_candidate(
                connection,
                &source_dir,
                decision,
                &current,
                ClipInput {
                    source_dir_id: source_dir.id,
                    clip_group_id: Some(clip_group.id),
                    video_path: &current.file_path,
                    file_name: &file_name,
                    file_size: current.size_bytes,
                    modified_at: Some(&current.modified_at),
                    duration_ms: None,
                    recorded_at: None,
                    cover_path: cover_path_string.as_deref(),
                    cover_source,
                },
                summary,
                &source_path,
                &mut source_errors,
                &mut complete_for_missing,
            ) {
                Ok(Some(saved)) => saved,
                Ok(None) => continue,
                Err(error) => {
                    complete_for_missing = false;
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                    continue;
                }
            };
            match saved.outcome {
                ClipSaveOutcome::Inserted => summary.new_clip_count += 1,
                ClipSaveOutcome::Updated => summary.updated_clip_count += 1,
                ClipSaveOutcome::Unchanged => {}
            }
            if cover_source == "missing" {
                summary.cover_missing_count += 1;
            }
            progress.clip_scanned(&file_name);

            if runtime.is_cancelled() {
                return Ok(cancelled_source_step(
                    progress,
                    &source_name,
                    source_path,
                    Some(source_dir.id),
                    seen_paths,
                ));
            }
            if let Some(metadata) = selected_metadata {
                let metadata_result = db::upsert_clip_metadata(
                    connection,
                    db::ClipMetadataInput {
                        clip_id: saved.clip.id,
                        metadata_status: match metadata.parse_status {
                            crate::metadata::ParseStatus::Parsed => "parsed",
                            crate::metadata::ParseStatus::Partial => "partial",
                            crate::metadata::ParseStatus::Failed => "failed",
                        },
                        json_path: Some(metadata.json_path.as_str()),
                        account_name: metadata.player_name.as_deref(),
                        player_name: metadata.player_name.as_deref(),
                        agent_name: metadata.agent_name.as_deref(),
                        map_name: metadata.map_name.as_deref(),
                        game_mode: metadata.game_mode.as_deref(),
                        scoreline: None,
                        kda: metadata.kda.as_deref(),
                        extracted_text: Some(metadata.extracted_text.as_str()),
                        parse_error: metadata.parse_error.as_deref(),
                    },
                )
                .and_then(|_| {
                    let video_type = metadata.detected_video_type();
                    db::update_video_export_classification(
                        connection,
                        saved.clip.id,
                        video_type.map(|value| value.highlight_type()),
                        video_type.and_then(|value| value.kill_count()),
                    )
                });
                if let Err(error) = metadata_result {
                    complete_for_missing = false;
                    push_source_error(summary, &source_path, &error);
                    push_unique_error(&mut source_errors, error);
                    continue;
                }
            }
        }
        if runtime.is_cancelled() {
            return Ok(cancelled_source_step(
                progress,
                &source_name,
                source_path,
                Some(source_dir.id),
                seen_paths,
            ));
        }
        progress.group_scanned(&group_name);
    }

    if runtime.is_cancelled() {
        return Ok(cancelled_source_step(
            progress,
            &source_name,
            source_path,
            Some(source_dir.id),
            seen_paths,
        ));
    }
    if source_errors.is_empty() {
        db::mark_source_dir_scan_succeeded(connection, source_dir.id)?;
    } else {
        db::mark_source_dir_scan_error(
            connection,
            source_dir.id,
            "partial",
            &source_errors.join(" | "),
        )?;
    }
    progress.source_finished(&source_name);
    Ok(SourceScanStep::finished(SourceScanOutcome {
        source_path,
        source_id: Some(source_dir.id),
        accessible: true,
        metadata_eligible: true,
        complete_for_missing,
        seen_paths,
    }))
}

#[allow(clippy::too_many_arguments)]
fn persist_aclos_scan_candidate(
    connection: &Connection,
    source: &db::SourceDir,
    decision: db::ScanReconnectDecision,
    current: &CollectedScanCandidate,
    input: ClipInput<'_>,
    summary: &mut ScanSummary,
    source_path: &Path,
    source_errors: &mut Vec<String>,
    complete_for_missing: &mut bool,
) -> DbResult<Option<db::SavedClip>> {
    if concrete_file_identity_changed(decision.candidate().file_identity, current.file_identity) {
        let error = format!(
            "Deferred MP4 whose stable identity changed after planning: {}",
            current.path.display()
        );
        *complete_for_missing = false;
        push_source_error(summary, source_path, &error);
        push_unique_error(source_errors, error);
        return Ok(None);
    }
    let reconnect = match decision {
        db::ScanReconnectDecision::ExistingPath { .. } | db::ScanReconnectDecision::New(_) => None,
        db::ScanReconnectDecision::NewWithWarning { warning, .. } => {
            let skip_candidate = matches!(
                warning.kind,
                db::ScanReconnectWarningKind::ForeignPathOwner
                    | db::ScanReconnectWarningKind::NormalizedPathConflict
            );
            record_aclos_reconnect_warning(
                summary,
                source_path,
                source_errors,
                complete_for_missing,
                warning,
            );
            if skip_candidate {
                return Ok(None);
            }
            None
        }
        db::ScanReconnectDecision::Reconnect(planned) => {
            (current.file_identity == planned.candidate.file_identity).then_some(planned)
        }
    };

    if let Some(planned) = reconnect {
        match db::apply_planned_scan_reconnect(connection, &planned, input, current.file_identity)?
        {
            db::ApplyScanReconnectOutcome::Reconnected(saved) => return Ok(Some(*saved)),
            db::ApplyScanReconnectOutcome::OldPathPresent => {}
            db::ApplyScanReconnectOutcome::OldPathUnverifiable(error) => {
                let error = format!(
                    "Old clip path could not be verified before reconnecting {}: {error}",
                    current.path.display()
                );
                *complete_for_missing = false;
                push_source_error(summary, source_path, &error);
                push_unique_error(source_errors, error);
            }
            db::ApplyScanReconnectOutcome::StalePlan => {
                let error = format!(
                    "Reconnect plan became stale before indexing {}",
                    current.path.display()
                );
                *complete_for_missing = false;
                push_source_error(summary, source_path, &error);
                push_unique_error(source_errors, error);
            }
        }
    }

    let normalized_path = db::normalize_path(input.video_path);
    if let Some(owner_id) =
        db::find_clip_source_id_by_normalized_path(connection, &normalized_path)?
    {
        if owner_id != source.id {
            return Err(format!(
                "Skipped MP4 already owned by source {owner_id}: {}",
                input.video_path
            ));
        }
    }
    db::upsert_scanned_clip_with_file_identity(connection, input, current.file_identity).map(Some)
}

fn record_aclos_reconnect_warning(
    summary: &mut ScanSummary,
    source_path: &Path,
    source_errors: &mut Vec<String>,
    complete_for_missing: &mut bool,
    warning: db::ScanReconnectWarning,
) {
    if warning.kind.blocks_missing_reconciliation() {
        *complete_for_missing = false;
    }
    push_source_error(summary, source_path, &warning.message);
    push_unique_error(source_errors, warning.message);
}

fn cancelled_source_step(
    progress: &mut ScanProgressState<'_>,
    source_name: &str,
    source_path: PathBuf,
    source_id: Option<i64>,
    seen_paths: HashSet<String>,
) -> SourceScanStep {
    progress.source_interrupted(source_name);
    SourceScanStep::cancelled(SourceScanOutcome {
        source_path,
        source_id,
        accessible: true,
        metadata_eligible: true,
        complete_for_missing: false,
        seen_paths,
    })
}

fn reconcile_missing_clips(
    connection: &Connection,
    outcomes: &[SourceScanOutcome],
    summary: &mut ScanSummary,
    runtime: ScanRuntime<'_>,
) -> DbResult<bool> {
    for outcome in outcomes {
        if runtime.is_cancelled() {
            return Ok(false);
        }
        if !outcome.complete_for_missing {
            continue;
        }
        let Some(source_id) = outcome.source_id else {
            continue;
        };
        for normalized_path in db::list_active_clip_paths_for_source(connection, source_id)? {
            if runtime.is_cancelled() {
                return Ok(false);
            }
            if outcome.seen_paths.contains(&normalized_path) {
                continue;
            }
            if db::mark_clip_missing_by_normalized_path(connection, &normalized_path)? {
                summary.missing_clip_count += 1;
            }
        }
    }
    Ok(true)
}

fn collect_metadata_snapshot(
    scan_root: &Path,
    allow_external_fallback: bool,
) -> CachedMetadataIngest {
    let metadata_paths = metadata_source_paths(scan_root, allow_external_fallback);
    let mut errors = Vec::new();
    let leveldb_result =
        match crate::leveldb_reader::read_leveldb_battle_lists(&metadata_paths.leveldb_dir) {
            Ok(result) => result,
            Err(error) => {
                errors.push(error);
                Default::default()
            }
        };
    let log_result =
        match crate::highlight_log_parser::parse_highlight_logs(&metadata_paths.logs_dir) {
            Ok(result) => result,
            Err(error) => {
                errors.push(error);
                Default::default()
            }
        };
    let wonderful_result = if metadata_paths.wonderful_dir.is_dir() {
        crate::wonderful_db::read_wonderful_db_dir(&metadata_paths.wonderful_dir)
    } else {
        Default::default()
    };

    CachedMetadataIngest {
        leveldb_result,
        log_result,
        wonderful_result,
        local_account_hint_scope: metadata_paths.account_hint_scope,
        errors,
    }
}

fn finalize_scanned_metadata(
    connection: &Connection,
    source_paths: &[PathBuf],
    account_hint_scopes: &[PathBuf],
    summary: &mut ScanSummary,
) -> DbResult<()> {
    db::clear_invalid_display_metadata(connection)?;
    db::clear_mismatched_match_metadata(connection)?;
    for source_path in source_paths {
        if !account_hint_scopes
            .iter()
            .any(|scope| path_is_within(source_path, scope))
        {
            db::clear_weak_account_name_hints_for_source_root(connection, source_path)?;
        }
    }
    db::backfill_agent_names_from_export_text(connection)?;

    let mut all_hints = Vec::new();
    for source_path in source_paths {
        match collect_edit_agent_asset_hints(source_path) {
            Ok(mut hints) => all_hints.append(&mut hints),
            Err(error) => {
                summary.metadata_warning_count += 1;
                summary.push_error(error);
            }
        }
    }
    db::backfill_agent_names_from_asset_hints(
        connection,
        &all_hints,
        EDIT_AGENT_ASSET_WINDOW_SECONDS,
    )?;
    for scope in account_hint_scopes {
        db::propagate_known_account_names(connection, Some(scope))?;
    }
    Ok(())
}

fn push_source_error(summary: &mut ScanSummary, source_path: &Path, error: &str) {
    summary.push_error(format!("Source {}: {error}", path_to_string(source_path)));
}

fn push_unique_error(errors: &mut Vec<String>, error: String) {
    let error = truncate_utf8_bytes(error, MAX_SCAN_ERROR_MESSAGE_BYTES);
    if !errors.contains(&error) && errors.len() < MAX_SCAN_ERROR_SAMPLES {
        errors.push(error);
    }
}

fn truncate_utf8_bytes(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

fn is_source_directory_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("wonderfulvideos"))
}

fn normalize_unique_scan_paths<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Vec<PathBuf> {
    let mut normalized_paths = Vec::new();
    let mut keys = HashSet::new();
    for path in paths {
        let Some(path) = normalize_scan_path(path) else {
            continue;
        };
        if keys.insert(scan_path_key(&path)) {
            normalized_paths.push(path);
        }
    }
    normalized_paths
}

fn normalize_scan_path(path: &Path) -> Option<PathBuf> {
    let value = path.to_string_lossy();
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let normalized = Path::new(value).components().collect::<PathBuf>();
    if normalized.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(normalized)
    }
}

fn scan_path_key(path: &Path) -> String {
    let comparable = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let normalized = db::normalize_path(&path_to_string(&comparable));
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.is_empty() {
        normalized
    } else {
        trimmed.to_string()
    }
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path_key = scan_path_key(path);
    let root_key = scan_path_key(root);
    path_key == root_key
        || path_key
            .strip_prefix(&root_key)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

fn scan_root_for_source(requested_roots: &[PathBuf], source_path: &Path) -> PathBuf {
    requested_roots
        .iter()
        .filter(|root| path_is_within(source_path, root))
        .max_by_key(|root| scan_path_key(root).len())
        .cloned()
        .unwrap_or_else(|| source_path.to_path_buf())
}

fn metadata_scan_root_for_source(source_path: &Path) -> PathBuf {
    source_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(source_path)
        .to_path_buf()
}

fn normalize_metadata_scan_anchors(anchors: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut normalized = Vec::new();
    for anchor in anchors {
        let (aclos_root, is_structural_anchor) = metadata_aclos_root_for_scan_anchor(&anchor);
        push_unique_scan_root(
            &mut normalized,
            if is_structural_anchor {
                aclos_root
            } else {
                anchor
            },
        );
    }
    normalized
}

fn metadata_anchor_has_sources(scan_root: &Path, allow_external_fallback: bool) -> bool {
    let paths = metadata_source_paths(scan_root, allow_external_fallback);
    paths.leveldb_dir.is_dir() || paths.logs_dir.is_dir() || paths.wonderful_dir.is_dir()
}

fn run_metadata_ingest(
    connection: &Connection,
    summary: &mut ScanSummary,
    metadata: &CachedMetadataIngest,
    account_hint_scope: Option<&Path>,
) {
    summary.metadata_warning_count += metadata.errors.len().min(i64::MAX as usize) as i64;
    for error in &metadata.errors {
        summary.push_error(error.clone());
    }
    let leveldb_result = &metadata.leveldb_result;
    let log_result = &metadata.log_result;

    summary.metadata_warning_count += (leveldb_result.warning_count
        + leveldb_result.bad_record_count
        + log_result.bad_line_count) as i64;
    push_metadata_warnings(summary, leveldb_result, log_result);

    match ingest_match_metadata(
        connection,
        MetadataIngestInput {
            leveldb_battles: &leveldb_result.battles,
            log_records: &log_result.records,
        },
    ) {
        Ok(ingest_summary) => {
            summary.metadata_match_count += ingest_summary.matches_upserted as i64;
            summary.metadata_enriched_clip_count += ingest_summary.enriched_clip_count as i64;
            summary.metadata_event_count += ingest_summary.events_inserted as i64;
        }
        Err(error) => {
            summary.metadata_warning_count += 1;
            summary.push_error(error);
        }
    }

    let mut account_name_hints = leveldb_result
        .account_roles
        .iter()
        .filter_map(|role| {
            role.player_name
                .as_ref()
                .map(|account_name| db::AccountNameHint {
                    account_id: role.account_id.clone(),
                    account_name: account_name.clone(),
                })
        })
        .collect::<Vec<_>>();
    account_name_hints.extend(log_result.account_name_hints.iter().filter_map(|hint| {
        hint.account_name
            .as_ref()
            .map(|account_name| db::AccountNameHint {
                account_id: hint.account_id.clone(),
                account_name: account_name.clone(),
            })
    }));
    if let Some(scope) = account_hint_scope {
        if let Err(error) =
            db::propagate_account_name_hints(connection, &account_name_hints, Some(scope))
        {
            summary.metadata_warning_count += 1;
            summary.push_error(error);
        }
    }

    let wonderful_result = &metadata.wonderful_result;
    summary.metadata_warning_count += wonderful_result.warnings.len().min(i64::MAX as usize) as i64;
    for warning in &wonderful_result.warnings {
        let warning = format!(
            "WonderfulDb 账号文件 {} 读取警告：{}",
            warning.account_filename, warning.message
        );
        summary.push_error(warning);
    }
    // Snapshot nickname fields cover exports whose video events omit PlayerName. Both stores are
    // ingested before the account-wide resolver below selects the newest timestamped Riot ID.
    if let Err(error) = crate::wonderful_ingest::ingest_wonderful_snapshots(
        connection,
        &wonderful_result.snapshot_accounts,
    ) {
        summary.metadata_warning_count += 1;
        summary.push_error(error);
    }
    match crate::wonderful_ingest::ingest_wonderful_metadata_with_round_scores(
        connection,
        &wonderful_result.accounts,
        &log_result.round_scores,
    ) {
        Ok(ingest_summary) => {
            summary.metadata_enriched_clip_count +=
                ingest_summary.matched_video_count.min(i64::MAX as usize) as i64;
            summary.metadata_event_count +=
                ingest_summary.event_count.min(i64::MAX as usize) as i64;
            summary.metadata_warning_count +=
                ingest_summary.warning_count.min(i64::MAX as usize) as i64;
            for warning in ingest_summary.warnings {
                summary.push_error(warning);
            }
        }
        Err(error) => {
            summary.metadata_warning_count += 1;
            summary.push_error(error);
        }
    }
}

fn push_metadata_warnings(
    summary: &mut ScanSummary,
    leveldb_result: &crate::leveldb_reader::LevelDbBattleListResult,
    log_result: &crate::highlight_log_parser::HighlightLogParseResult,
) {
    if leveldb_result.warning_count > 0 {
        summary.push_error(format!(
            "LevelDB 读取警告 {} 条，已跳过不可读文件并继续扫描",
            leveldb_result.warning_count
        ));
    }

    if leveldb_result.bad_record_count > 0 {
        summary.push_error(format!(
            "LevelDB 对局记录解析失败 {} 条，其他记录已继续导入",
            leveldb_result.bad_record_count
        ));
    }

    if log_result.bad_line_count > 0 {
        summary.push_error(format!(
            "highlight.log 解析坏行 {} 条，其他日志行已继续导入",
            log_result.bad_line_count
        ));
    }
}

fn collect_edit_agent_asset_hints(
    source_path: &Path,
) -> Result<Vec<db::ClipAgentAssetHint>, String> {
    let mut hints = Vec::new();
    let edit_dir = source_path.join("edit");
    if !edit_dir.is_dir() {
        return Ok(hints);
    }

    let source_dir_name = path_file_name(source_path);
    for asset_path in read_sorted_entries(&edit_dir)? {
        if !asset_path.is_file() {
            continue;
        }

        let Some(decoded_url) = decode_base64_file_stem(&asset_path) else {
            continue;
        };
        let Some(asset_id) = agent_asset_id_from_url(&decoded_url) else {
            continue;
        };
        let Some(agent_name) = crate::display_names::agent_name_from_asset_id(&asset_id) else {
            continue;
        };
        let Some(observed_at) = fs::metadata(&asset_path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| system_time_unix_seconds(time).ok())
        else {
            continue;
        };

        hints.push(db::ClipAgentAssetHint {
            source_dir_name: source_dir_name.clone(),
            observed_at,
            agent_name,
        });
    }

    Ok(hints)
}

fn decode_base64_file_stem(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let mut padded = stem.to_string();
    let remainder = padded.len() % 4;
    if remainder != 0 {
        padded.extend(std::iter::repeat_n('=', 4 - remainder));
    }

    let bytes = general_purpose::STANDARD.decode(padded).ok()?;
    String::from_utf8(bytes).ok()
}

fn agent_asset_id_from_url(value: &str) -> Option<String> {
    let lower = value.replace('\\', "/").to_ascii_lowercase();
    agent_asset_id_after_marker(&lower, "agentbackground/agent/").or_else(|| {
        let id = agent_asset_id_after_marker(&lower, "agentskill/")?;
        let marker = format!("agentskill/{id}_");
        if lower.contains(&marker) {
            Some(id)
        } else {
            None
        }
    })
}

fn agent_asset_id_after_marker(value: &str, marker: &str) -> Option<String> {
    let start = value.find(marker)? + marker.len();
    let id = value[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

struct MetadataSourcePaths {
    leveldb_dir: PathBuf,
    logs_dir: PathBuf,
    wonderful_dir: PathBuf,
    account_hint_scope: Option<PathBuf>,
}

fn metadata_source_paths(
    scan_root: &Path,
    allow_external_metadata_fallback: bool,
) -> MetadataSourcePaths {
    let (candidate_aclos_root, is_structural_aclos_anchor) =
        metadata_aclos_root_for_scan_anchor(scan_root);

    let (fallback_aclos_root, wonderful_aclos_root, account_hint_scope) =
        if is_structural_aclos_anchor || allow_external_metadata_fallback {
            let default_aclos_root = default_aclos_app_data_dir();
            let fallback_aclos_root = select_metadata_aclos_root(
                Some(candidate_aclos_root.clone()),
                default_aclos_root.clone(),
            );
            let account_hint_scope = (scan_path_key(&candidate_aclos_root)
                == scan_path_key(&fallback_aclos_root))
            .then(|| scan_root.to_path_buf());
            let wonderful_aclos_root =
                select_wonderful_aclos_root(Some(candidate_aclos_root), default_aclos_root);
            (
                fallback_aclos_root,
                wonderful_aclos_root,
                account_hint_scope,
            )
        } else {
            (
                scan_root.to_path_buf(),
                scan_root.to_path_buf(),
                Some(scan_root.to_path_buf()),
            )
        };

    MetadataSourcePaths {
        leveldb_dir: fallback_aclos_root.join("Local Storage").join("leveldb"),
        logs_dir: fallback_aclos_root.join("logs"),
        wonderful_dir: wonderful_aclos_root.join("WonderfulDb"),
        account_hint_scope,
    }
}

/// Resolves any supported scan entry point back to the ACLOS application-data root that owns
/// `WonderfulDb`, `logs`, and `Local Storage`. In particular, the source wizard permits selecting
/// a `wonderfulVideos<openid>` directory directly; treating that directory as the application
/// root silently falls back to the current machine's database instead of the imported one.
fn metadata_aclos_root_for_scan_anchor(scan_root: &Path) -> (PathBuf, bool) {
    let leaf = scan_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    if leaf.eq_ignore_ascii_case("aclos-highlight") {
        return (scan_root.parent().unwrap_or(scan_root).to_path_buf(), true);
    }
    if leaf.to_ascii_lowercase().starts_with("wonderfulvideos") {
        let Some(parent) = scan_root.parent() else {
            return (scan_root.to_path_buf(), true);
        };
        return if parent
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("aclos-highlight"))
        {
            (parent.parent().unwrap_or(parent).to_path_buf(), true)
        } else {
            (parent.to_path_buf(), true)
        };
    }

    (scan_root.to_path_buf(), false)
}

fn select_metadata_aclos_root(
    scan_parent_aclos_root: Option<PathBuf>,
    default_appdata_aclos_root: PathBuf,
) -> PathBuf {
    match scan_parent_aclos_root {
        Some(scan_parent_aclos_root)
            if metadata_root_has_sources(&scan_parent_aclos_root)
                || !metadata_root_has_sources(&default_appdata_aclos_root) =>
        {
            scan_parent_aclos_root
        }
        Some(_) | None => default_appdata_aclos_root,
    }
}

fn metadata_root_has_sources(aclos_root: &Path) -> bool {
    aclos_root.join("Local Storage").join("leveldb").is_dir()
        || aclos_root.join("logs").join("highlight.log").is_file()
}

fn select_wonderful_aclos_root(
    candidate_aclos_root: Option<PathBuf>,
    default_appdata_aclos_root: PathBuf,
) -> PathBuf {
    match candidate_aclos_root {
        Some(candidate_aclos_root) if candidate_aclos_root.join("WonderfulDb").is_dir() => {
            candidate_aclos_root
        }
        Some(_) | None => default_appdata_aclos_root,
    }
}

fn default_aclos_app_data_dir() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Users\Default"))
                .join("AppData")
                .join("Roaming")
        })
        .join("ACLOS")
}

fn scan_roots_from_videocut_log(path: &Path) -> Result<Vec<PathBuf>, String> {
    if !path.is_file() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read videocut log {}: {error}",
            path_to_string(path)
        )
    })?;
    let mut roots = Vec::new();

    for line in content.lines() {
        for marker in ["clip file:", "file path:"] {
            let Some(path_text) = path_after_log_marker(line, marker) else {
                continue;
            };
            if let Some(root) = scan_root_from_aclos_path(path_text) {
                push_unique_scan_root(&mut roots, root);
            }
        }
    }

    Ok(roots)
}

fn path_after_log_marker<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let marker_index = line.find(marker)? + marker.len();
    let tail = line[marker_index..].trim();
    tail.split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn scan_root_from_aclos_path(value: &str) -> Option<PathBuf> {
    let normalized = value.trim().replace('/', "\\");
    let lower = normalized.to_ascii_lowercase();
    let marker_index = lower.find("\\wonderfulvideos")?;
    let root = normalized[..marker_index].trim();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

/// ACLOS keeps per-match clips in `<match-id>/` directories, but raw full-session
/// recordings land in a shared `record/` directory. Folding `record/` into one clip
/// group would present unrelated matches under a single match header, so its files
/// are grouped per file instead. `metadata_ingest::record_src_group_key` already
/// refuses to resolve this directory name for the same reason.
fn is_aclos_per_clip_group_dir(dir_name: &str) -> bool {
    dir_name.eq_ignore_ascii_case("record")
}

fn clip_group_name_for_file(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path_file_name(path))
}

fn mp4_files_in_dir(path: &Path) -> Result<Vec<PathBuf>, String> {
    let files = read_sorted_entries(path)?
        .into_iter()
        .filter(|entry| entry.is_file() && has_extension(entry, "mp4"))
        .collect::<Vec<_>>();

    Ok(files)
}

fn find_cover_jpegs(
    path: &Path,
    canonical_source_root: &Path,
) -> Result<CoverJpegDiscovery, String> {
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    for entry in read_sorted_entries(path)? {
        let is_cover_name = entry
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("cover-"))
            && has_extension(&entry, "jpeg");
        if !is_cover_name {
            continue;
        }

        let metadata = match fs::symlink_metadata(&entry) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!(
                    "Failed to inspect cover candidate {}: {error}",
                    entry.display()
                ));
                continue;
            }
        };
        if metadata_is_reparse_point(&metadata) {
            warnings.push(format!(
                "Skipped cover symbolic link or reparse point: {}",
                entry.display()
            ));
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        match canonicalize_regular_path_within_root(&entry, canonical_source_root) {
            Ok(_) => paths.push(entry),
            Err(error) => warnings.push(format!("Skipped unsafe cover candidate: {error}")),
        }
    }

    Ok(CoverJpegDiscovery { paths, warnings })
}

fn select_cover_for_clip<'a>(
    clip_path: &Path,
    cover_paths: &'a [PathBuf],
    clip_count: usize,
) -> Option<&'a Path> {
    let clip_stem = path_file_stem_lower(clip_path)?;

    for cover_path in cover_paths {
        let Some(cover_stem) = path_file_stem_lower(cover_path) else {
            continue;
        };
        let cover_key = cover_stem.strip_prefix("cover-").unwrap_or(&cover_stem);

        if cover_key == clip_stem {
            return Some(cover_path.as_path());
        }
    }

    if cover_paths.len() == 1 && clip_count == 1 {
        return cover_paths.first().map(PathBuf::as_path);
    }

    None
}

fn select_video_export_metadata(
    configs: &[crate::metadata::VideoExportConfigMetadata],
    clip_modified_at: Option<i64>,
) -> Option<&crate::metadata::VideoExportConfigMetadata> {
    let clip_modified_at = clip_modified_at?;

    configs
        .iter()
        .filter_map(|config| {
            let observed_at = video_export_config_observed_at(config)?;
            let difference = (observed_at - clip_modified_at).abs();
            if difference <= VIDEO_EXPORT_CONFIG_WINDOW_SECONDS {
                Some((difference, config))
            } else {
                None
            }
        })
        .min_by_key(|(difference, _config)| *difference)
        .map(|(_difference, config)| config)
}

fn video_export_config_observed_at(
    config: &crate::metadata::VideoExportConfigMetadata,
) -> Option<i64> {
    fs::metadata(&config.json_path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| system_time_unix_seconds(time).ok())
}

fn read_sorted_entries(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("Failed to read directory {}: {error}", path_to_string(path)))?
        .map(|entry| {
            entry.map(|entry| entry.path()).map_err(|error| {
                format!(
                    "Failed to read directory entry {}: {error}",
                    path_to_string(path)
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    entries.sort_by_key(|path| path_to_string(path).to_lowercase());
    Ok(entries)
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn path_file_stem_lower(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .map(|name| name.to_lowercase())
}

fn path_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path_to_string(path))
}

fn path_to_string(path: &Path) -> String {
    path.display().to_string()
}

fn format_system_time(time: SystemTime) -> String {
    system_time_unix_seconds(time)
        .map(|seconds| seconds.to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn system_time_unix_seconds(time: SystemTime) -> Result<i64, std::time::SystemTimeError> {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
}

#[cfg(test)]
mod tests {
    // Account IDs, player names, match IDs, and paths below are synthetic fixtures.
    use aes::Aes256;
    use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    use sha2::{Digest, Sha256};
    use std::{
        fs::{self, File, FileTimes},
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicBool, Ordering},
            Mutex,
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use rusqlite::Connection;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    use crate::db;

    type Aes256CbcEnc = cbc::Encryptor<Aes256>;

    #[test]
    fn scan_directory_indexes_sources_groups_clips_and_cover_status() {
        let fixture = TestFixture::new("indexes");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let group_with_cover = source.join("11111111-1111-1111-1111-111111111111");
        let group_without_cover = source.join("22222222-2222-2222-2222-222222222222");
        let ignored_empty_group = source.join("33333333-3333-3333-3333-333333333333");
        fs::create_dir_all(&group_with_cover).expect("group should be created");
        fs::create_dir_all(&group_without_cover).expect("group should be created");
        fs::create_dir_all(&ignored_empty_group).expect("group should be created");
        fs::write(group_with_cover.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(group_with_cover.join("cover-ace.jpeg"), b"cover")
            .expect("cover should be written");
        fs::write(group_without_cover.join("clutch.MP4"), b"video-two")
            .expect("clip should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");
        let sources = db::list_sources(&connection).expect("sources should list");

        assert_eq!(summary.source_dir_count, 1);
        assert_eq!(summary.clip_group_count, 2);
        assert_eq!(summary.new_clip_count, 2);
        assert_eq!(summary.updated_clip_count, 0);
        assert_eq!(summary.missing_clip_count, 0);
        assert_eq!(summary.cover_missing_count, 1);
        assert!(summary.errors.is_empty());
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_kind, db::SourceKind::Aclos);
        assert_eq!(sources[0].scan_mode, db::ScanMode::AclosStructured);
        assert_eq!(sources[0].scan_root_path, super::path_to_string(root));
        assert_eq!(clips.len(), 2);
        assert!(clips.iter().any(|clip| {
            clip.file_name == "ace.mp4"
                && clip.source_relative_dir
                    == "wonderfulVideos-main/11111111-1111-1111-1111-111111111111"
                && clip.cover_source == "file"
                && clip
                    .cover_path
                    .as_deref()
                    .unwrap_or_default()
                    .ends_with("cover-ace.jpeg")
                && clip.status == "available"
        }));
        assert!(clips.iter().any(|clip| {
            clip.file_name == "clutch.MP4"
                && clip.cover_source == "missing"
                && clip.cover_path.is_none()
                && clip.status == "available"
        }));
    }

    #[test]
    fn scan_directory_does_not_guess_cover_order_for_multi_video_groups() {
        let fixture = TestFixture::new("ambiguous-covers");
        let root = fixture.path();
        let group = root
            .join("wonderfulVideos1001")
            .join("11111111-1111-1111-1111-111111111111");
        fs::create_dir_all(&group).expect("group should be created");
        for file_name in [
            "8f2b9e4c63a747fda66c48df2a61d001.mp4",
            "b12d72e0fba24ee4ad42c80ac37d1002.mp4",
            "cover-4-0001.jpeg",
            "cover-4-0002.jpeg",
        ] {
            fs::write(group.join(file_name), file_name.as_bytes())
                .expect("fixture media should be written");
        }
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(summary.new_clip_count, 2);
        assert_eq!(summary.cover_missing_count, 2);
        assert_eq!(clips.len(), 2);
        assert!(clips
            .iter()
            .all(|clip| clip.cover_source == "missing" && clip.cover_path.is_none()));
    }

    #[test]
    fn scan_directory_preserves_unicode_casefolded_exact_covers_in_multi_video_groups() {
        let fixture = TestFixture::new("unicode-exact-covers");
        let root = fixture.path();
        let group = root
            .join("wonderfulVideos1001")
            .join("22222222-2222-2222-2222-222222222222");
        fs::create_dir_all(&group).expect("group should be created");
        for file_name in [
            "ÄCE.mp4",
            "普通击杀.mp4",
            "cover-äce.jpeg",
            "cover-普通击杀.jpeg",
        ] {
            fs::write(group.join(file_name), file_name.as_bytes())
                .expect("fixture media should be written");
        }
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(summary.new_clip_count, 2);
        assert_eq!(summary.cover_missing_count, 0);
        assert_eq!(clips.len(), 2);
        assert!(clips.iter().all(|clip| clip.cover_source == "file"));
        assert!(clips.iter().any(|clip| {
            clip.file_name == "ÄCE.mp4"
                && clip
                    .cover_path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("cover-äce.jpeg"))
        }));
        assert!(clips.iter().any(|clip| {
            clip.file_name == "普通击杀.mp4"
                && clip
                    .cover_path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("cover-普通击杀.jpeg"))
        }));
    }

    #[test]
    fn aclos_group_rename_reconnects_the_clip_without_losing_user_state() {
        let fixture = TestFixture::new("aclos-group-rename");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let old_group = source.join("11111111-1111-1111-1111-111111111111");
        let new_group = source.join("22222222-2222-2222-2222-222222222222");
        fs::create_dir_all(&old_group).expect("old group should be created");
        fs::write(old_group.join("ace.mp4"), b"same-aclos-file").expect("clip should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let first = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        assert_eq!(first.new_clip_count, 1);
        let initial = db::list_clips(&connection).unwrap().pop().unwrap();
        db::update_clip_note(&connection, initial.id, Some("keep-aclos-note")).unwrap();
        db::set_clip_review_decision(&connection, initial.id, db::ReviewDecision::Liked).unwrap();
        let tag = db::create_tag(&connection, "keep-aclos-tag", None).unwrap();
        db::assign_tag_to_clip(&connection, initial.id, tag.id).unwrap();
        db::upsert_clip_metadata(
            &connection,
            db::ClipMetadataInput {
                clip_id: initial.id,
                metadata_status: "parsed",
                json_path: None,
                account_name: Some("keep-aclos-account"),
                player_name: Some("keep-aclos-player"),
                agent_name: Some("Jett"),
                map_name: Some("Ascent"),
                game_mode: Some("ranked"),
                scoreline: Some("13-9"),
                kda: Some("20/10/4"),
                extracted_text: Some("keep-aclos-metadata"),
                parse_error: None,
            },
        )
        .unwrap();
        db::ensure_clip_thumbnails(&connection, &[initial.id]).unwrap();
        let old_thumbnail_job = db::claim_next_thumbnail_job(&connection, "2099-01-01T00:00:00Z")
            .unwrap()
            .expect("thumbnail should be claimed before reconnecting");

        fs::rename(&old_group, &new_group).expect("group directory should be renamed");
        let second = crate::scanner::scan_directory(&connection, root)
            .expect("renamed group should scan safely");
        assert_eq!(second.new_clip_count, 0);
        assert_eq!(second.updated_clip_count, 1);
        assert!(second.errors.is_empty());
        let clips = db::list_clips(&connection).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].id, initial.id);
        assert_eq!(clips[0].note.as_deref(), Some("keep-aclos-note"));
        assert!(clips[0].favorite);
        assert_eq!(clips[0].review_decision, db::ReviewDecision::Liked);
        assert_eq!(clips[0].account_name.as_deref(), Some("keep-aclos-account"));
        assert_eq!(clips[0].player_name.as_deref(), Some("keep-aclos-player"));
        assert_eq!(clips[0].agent_name.as_deref(), Some("Jett"));
        assert!(clips[0]
            .source_relative_dir
            .ends_with("22222222-2222-2222-2222-222222222222"));
        let detail = db::find_clip_detail_by_id(&connection, initial.id)
            .unwrap()
            .expect("clip should remain addressable");
        assert_eq!(
            detail.tags.iter().map(|tag| tag.id).collect::<Vec<_>>(),
            vec![tag.id]
        );
        let stale_cache = format!(
            "{}-{}.jpg",
            old_thumbnail_job.clip_id, old_thumbnail_job.fingerprint
        );
        assert!(!db::complete_thumbnail_job_if_current(
            &connection,
            &old_thumbnail_job,
            &stale_cache,
            10,
            &old_thumbnail_job.fingerprint,
        )
        .unwrap());
    }

    #[test]
    fn aclos_source_wide_staging_does_not_merge_ambiguous_hardlinks() {
        let fixture = TestFixture::new("aclos-hardlink-ambiguity");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let old_group = source.join("old");
        let old_path = old_group.join("same.mp4");
        fs::create_dir_all(&old_group).unwrap();
        fs::write(&old_path, b"shared-aclos-identity").unwrap();

        let connection = Connection::open_in_memory().unwrap();
        db::initialize_schema(&connection).unwrap();
        crate::scanner::scan_directory(&connection, root).unwrap();
        let old_clip_id = db::list_clips(&connection).unwrap()[0].id;

        let first_group = source.join("new-a");
        let second_group = source.join("new-b");
        fs::create_dir_all(&first_group).unwrap();
        fs::create_dir_all(&second_group).unwrap();
        fs::hard_link(&old_path, first_group.join("same.mp4")).unwrap();
        fs::hard_link(&old_path, second_group.join("same.mp4")).unwrap();
        fs::remove_file(&old_path).unwrap();

        let summary = crate::scanner::scan_directory(&connection, root).unwrap();
        let clips = db::list_clips(&connection).unwrap();
        assert_eq!(summary.new_clip_count, 2);
        assert_eq!(summary.missing_clip_count, 1);
        assert_eq!(clips.len(), 3);
        assert!(clips
            .iter()
            .any(|clip| clip.id == old_clip_id && clip.status == "missing"));
        assert_eq!(
            clips
                .iter()
                .filter(|clip| clip.status == "available")
                .count(),
            2
        );
        assert!(summary.errors.iter().any(|error| {
            error.contains("not unique on both sides")
                || error.contains("indexed without reconnecting")
        }));
    }

    #[cfg(windows)]
    #[test]
    fn aclos_scan_rejects_file_and_directory_reparse_points() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let fixture = TestFixture::new("aclos-reparse");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let outside = root.join("outside");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let outside_clip = outside.join("outside.mp4");
        fs::write(&outside_clip, b"outside").unwrap();
        if let Err(error) = symlink_file(&outside_clip, source.join("linked.mp4")) {
            eprintln!("skipping ACLOS reparse assertion because symlinks are unavailable: {error}");
            return;
        }
        if let Err(error) = symlink_dir(&outside, source.join("linked-group")) {
            eprintln!("directory symlink unavailable; file assertion still runs: {error}");
        }

        let connection = Connection::open_in_memory().unwrap();
        db::initialize_schema(&connection).unwrap();
        let summary = crate::scanner::scan_directory(&connection, root).unwrap();
        assert!(db::list_clips(&connection).unwrap().is_empty());
        assert!(summary
            .errors
            .iter()
            .any(|error| error.contains("reparse point")));
    }

    #[test]
    fn aclos_sync_rejects_a_source_outside_its_registered_scan_root() {
        let fixture = TestFixture::new("aclos-outside-root");
        let authorized_root = fixture.path().join("authorized");
        let source = fixture.path().join("outside/wonderfulVideos-main");
        let group = source.join("match-a");
        fs::create_dir_all(&authorized_root).unwrap();
        fs::create_dir_all(&group).unwrap();
        fs::write(group.join("outside.mp4"), b"outside").unwrap();

        let connection = Connection::open_in_memory().unwrap();
        db::initialize_schema(&connection).unwrap();
        let source_path_string = super::path_to_string(&source);
        let authorized_root_string = super::path_to_string(&authorized_root);
        let registered = db::register_source_dir(
            &connection,
            db::SourceDirInput {
                path: &source_path_string,
                name: "wonderfulVideos-main",
            },
            db::SourceProfileInput::aclos(&authorized_root_string),
            true,
        )
        .unwrap();

        let summary = crate::scanner::sync_scan_sources(&connection, &[registered.id]).unwrap();
        assert!(db::list_clips(&connection).unwrap().is_empty());
        assert!(summary
            .errors
            .iter()
            .any(|error| { error.contains("outside its authorized scan root") }));
        let source_after = db::find_source_dir_by_id(&connection, registered.id).unwrap();
        assert_eq!(source_after.status, "unavailable");
        assert_eq!(source_after.last_scanned_at, None);
    }

    #[cfg(windows)]
    #[test]
    fn aclos_cover_discovery_does_not_follow_file_or_directory_reparse_points() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let fixture = TestFixture::new("aclos-cover-reparse");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let group = source.join("match-a");
        let outside = root.join("outside");
        fs::create_dir_all(&group).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(group.join("clip.mp4"), b"inside-clip").unwrap();
        let outside_cover = outside.join("outside.jpeg");
        fs::write(&outside_cover, b"outside-cover").unwrap();
        if let Err(error) = symlink_file(&outside_cover, group.join("cover-clip.jpeg")) {
            eprintln!("skipping cover reparse assertion because symlinks are unavailable: {error}");
            return;
        }
        let directory_link_created =
            symlink_dir(&outside, group.join("cover-linked-directory.jpeg")).is_ok();

        let connection = Connection::open_in_memory().unwrap();
        db::initialize_schema(&connection).unwrap();
        let summary = crate::scanner::scan_directory(&connection, root).unwrap();
        let clips = db::list_clips(&connection).unwrap();

        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].file_name, "clip.mp4");
        assert_eq!(clips[0].cover_source, "missing");
        assert!(clips[0].cover_path.is_none());
        assert_eq!(fs::read(&outside_cover).unwrap(), b"outside-cover");
        assert!(summary
            .errors
            .iter()
            .any(|error| error.contains("cover-clip.jpeg") && error.contains("reparse point")));
        if directory_link_created {
            assert!(summary.errors.iter().any(|error| {
                error.contains("cover-linked-directory.jpeg") && error.contains("reparse point")
            }));
        }
    }

    #[test]
    fn scan_directory_gives_each_shared_record_recording_its_own_clip_group() {
        let fixture = TestFixture::new("shared-record-dir");
        let root = fixture.path();
        let source = root.join("wonderfulVideos1001");
        let match_group = source.join("11111111-1111-1111-1111-111111111111");
        let record = source.join("record");
        fs::create_dir_all(&match_group).expect("match group should be created");
        fs::create_dir_all(&record).expect("record directory should be created");
        fs::write(match_group.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(record.join("20260710-161959.mp4"), b"session-one")
            .expect("recording should be written");
        fs::write(record.join("20260710-172047.mp4"), b"session-two")
            .expect("recording should be written");
        fs::write(record.join("cover-20260710-161959.jpeg"), b"cover-one")
            .expect("recording cover should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert!(summary.errors.is_empty());
        assert_eq!(summary.new_clip_count, 3);
        // One group for the real match plus one per raw recording, never a shared `record` group.
        assert_eq!(summary.clip_group_count, 3);

        let mut group_names = clips
            .iter()
            .map(|clip| clip.clip_group_name.clone().unwrap_or_default())
            .collect::<Vec<_>>();
        group_names.sort();
        assert_eq!(
            group_names,
            vec![
                "11111111-1111-1111-1111-111111111111",
                "20260710-161959",
                "20260710-172047",
            ]
        );

        // The lone cover in `record/` must stay with the clip whose stem it matches, even though
        // every group carved out of that directory holds a single clip.
        let matched = clips
            .iter()
            .find(|clip| clip.file_name == "20260710-161959.mp4")
            .expect("covered recording should be indexed");
        assert!(matched
            .cover_path
            .as_deref()
            .is_some_and(|path| path.ends_with("cover-20260710-161959.jpeg")));
        let unmatched = clips
            .iter()
            .find(|clip| clip.file_name == "20260710-172047.mp4")
            .expect("uncovered recording should be indexed");
        assert_eq!(unmatched.cover_path, None);
    }

    #[test]
    fn scan_directory_indexes_legacy_mp4_files_in_source_root() {
        let fixture = TestFixture::new("root-level-legacy-video");
        let root = fixture.path();
        let source = root.join("wonderfulVideos1001");
        fs::create_dir_all(&source).expect("source should be created");
        fs::write(source.join("legacy-video.mp4"), b"legacy-video")
            .expect("root-level clip should be written");
        fs::write(source.join("cover-legacy-video.jpeg"), b"legacy-cover")
            .expect("root-level cover should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(summary.source_dir_count, 1);
        assert_eq!(summary.clip_group_count, 1);
        assert_eq!(summary.new_clip_count, 1);
        assert!(summary.errors.is_empty());
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].file_name, "legacy-video.mp4");
        assert_eq!(clips[0].clip_group_name.as_deref(), Some("legacy-video"));
        assert!(clips[0]
            .cover_path
            .as_deref()
            .is_some_and(|path| path.ends_with("cover-legacy-video.jpeg")));
    }

    #[test]
    fn scan_roots_indexes_two_sources_in_one_scan_run() {
        let fixture = ScanBatchFixture::new("two-sources");
        let library = fixture.path().join("Library");
        prepare_empty_metadata_root(&library);
        let sources = [
            library.join("wonderfulVideos1001"),
            library.join("wonderfulVideos2002"),
        ];
        for (index, source) in sources.iter().enumerate() {
            let group = source.join(format!("match-{index}"));
            fs::create_dir_all(&group).expect("source group should be created");
            fs::write(group.join(format!("clip-{index}.mp4")), b"video")
                .expect("source clip should be written");
        }
        let connection = fixture.open();

        let summary = crate::scanner::scan_roots(&connection, &sources)
            .expect("multi-source scan should run");
        let listed_sources = db::list_sources(&connection).expect("sources should list");

        assert_eq!(summary.source_dir_count, 2);
        assert_eq!(summary.clip_group_count, 2);
        assert_eq!(summary.new_clip_count, 2);
        assert!(summary.errors.is_empty());
        assert_eq!(scan_run_count(&connection), 1);
        assert_eq!(listed_sources.len(), 2);
        assert!(listed_sources
            .iter()
            .all(|source| source.status == "available" && source.clip_count == 1));
        assert!(listed_sources.iter().all(|source| {
            source
                .last_scan_at
                .as_deref()
                .is_some_and(|timestamp| timestamp.ends_with('Z') && timestamp.contains('T'))
        }));
    }

    #[test]
    fn scan_roots_deduplicates_windows_case_insensitive_paths() {
        let fixture = ScanBatchFixture::new("deduplicated-roots");
        let library = fixture.path().join("Library");
        prepare_empty_metadata_root(&library);
        let source = library.join("wonderfulVideos1001");
        let group = source.join("match-a");
        fs::create_dir_all(&group).expect("source group should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("source clip should be written");
        let case_variant = PathBuf::from(source.to_string_lossy().to_uppercase());
        let connection = fixture.open();

        let summary = crate::scanner::scan_roots(
            &connection,
            &[source.clone(), case_variant, source.join(".")],
        )
        .expect("deduplicated scan should run");

        assert_eq!(summary.source_dir_count, 1);
        assert_eq!(summary.new_clip_count, 1);
        assert_eq!(db::list_sources(&connection).unwrap().len(), 1);
        assert_eq!(db::list_clips(&connection).unwrap().len(), 1);
        assert_eq!(scan_run_count(&connection), 1);
    }

    #[test]
    fn scan_roots_empty_input_creates_one_noop_run() {
        let fixture = ScanBatchFixture::new("empty-roots");
        let connection = fixture.open();

        let summary = crate::scanner::scan_roots(&connection, &[])
            .expect("empty source list should be a normal no-op");

        assert_eq!(summary.root_path, "");
        assert_eq!(summary.source_dir_count, 0);
        assert_eq!(summary.new_clip_count, 0);
        assert_eq!(
            summary.message.as_deref(),
            Some("No source directories provided")
        );
        assert!(summary.errors.is_empty());
        assert_eq!(scan_run_count(&connection), 1);
    }

    #[test]
    fn scan_roots_indexes_mp4_at_direct_source_root() {
        let fixture = ScanBatchFixture::new("direct-root-mp4");
        let library = fixture.path().join("Library");
        prepare_empty_metadata_root(&library);
        let source = library.join("wonderfulVideos1001");
        fs::create_dir_all(&source).expect("source should be created");
        fs::write(source.join("legacy.mp4"), b"video").expect("root clip should be written");
        fs::write(source.join("cover-legacy.jpeg"), b"cover")
            .expect("root cover should be written");
        let connection = fixture.open();

        let summary = crate::scanner::scan_roots(&connection, std::slice::from_ref(&source))
            .expect("direct source scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(summary.source_dir_count, 1);
        assert_eq!(summary.clip_group_count, 1);
        assert_eq!(summary.new_clip_count, 1);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].file_name, "legacy.mp4");
        assert_eq!(clips[0].clip_group_name.as_deref(), Some("legacy"));
        assert_eq!(clips[0].cover_source, "file");
        assert_eq!(scan_run_count(&connection), 1);
    }

    #[test]
    fn scan_roots_uses_adjacent_account_hints_for_direct_source() {
        let fixture = ScanBatchFixture::new("direct-source-account-hints");
        let aclos_root = fixture.path().join("ACLOS");
        prepare_empty_metadata_root(&aclos_root);
        let account_id = "9000000000000000002";
        let source = aclos_root
            .join("aclos-highlight")
            .join(format!("wonderfulVideos{account_id}"));
        let group = source.join("match-without-full-metadata");
        fs::create_dir_all(&group).expect("source group should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("source clip should be written");
        fs::write(
            aclos_root
                .join("Local Storage")
                .join("leveldb")
                .join("000005.ldb"),
            account_roles_blob(
                r#"[{"openid":"9000000000000000002","nick":"FixtureBravo","tag":"0002"}]"#,
            ),
        )
        .expect("adjacent account hints should be written");
        let connection = fixture.open();

        crate::scanner::scan_roots(&connection, std::slice::from_ref(&source))
            .expect("direct source scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].account_name.as_deref(), Some("FixtureBravo#0002"));
        assert_eq!(clips[0].player_name.as_deref(), Some("FixtureBravo#0002"));
    }

    #[test]
    fn scan_roots_continues_after_unavailable_source_without_marking_history_missing() {
        let fixture = ScanBatchFixture::new("partial-source-failure");
        let library = fixture.path().join("Library");
        prepare_empty_metadata_root(&library);
        let unavailable_source = library.join("wonderfulVideos1001");
        let unavailable_group = unavailable_source.join("old-match");
        let unavailable_clip = unavailable_group.join("old.mp4");
        fs::create_dir_all(&unavailable_group).expect("historical group should be created");
        fs::write(&unavailable_clip, b"old-video").expect("historical clip should be written");
        let connection = fixture.open();
        crate::scanner::scan_roots(&connection, std::slice::from_ref(&unavailable_source))
            .expect("initial source scan should run");
        let initial_unavailable_last_scan = db::list_sources(&connection)
            .expect("initial source should list")
            .into_iter()
            .find(|source| {
                super::scan_path_key(Path::new(&source.path))
                    == super::scan_path_key(&unavailable_source)
            })
            .and_then(|source| source.last_scan_at)
            .expect("initial complete scan should set freshness");
        connection
            .execute("DELETE FROM scan_runs", [])
            .expect("initial scan run should be cleared for batch assertion");
        fs::remove_dir_all(&unavailable_source).expect("source should become unavailable");

        let available_source = library.join("wonderfulVideos2002");
        let available_group = available_source.join("new-match");
        fs::create_dir_all(&available_group).expect("available group should be created");
        fs::write(available_group.join("new.mp4"), b"new-video")
            .expect("available clip should be written");

        let summary = crate::scanner::scan_roots(
            &connection,
            &[unavailable_source.clone(), available_source.clone()],
        )
        .expect("partial source failure should not abort the batch");
        let clips = db::list_clips(&connection).expect("clips should list");
        let sources = db::list_sources(&connection).expect("sources should list");
        let unavailable = sources
            .iter()
            .find(|source| {
                super::scan_path_key(Path::new(&source.path))
                    == super::scan_path_key(&unavailable_source)
            })
            .expect("unavailable source should remain indexed");
        let available = sources
            .iter()
            .find(|source| {
                super::scan_path_key(Path::new(&source.path))
                    == super::scan_path_key(&available_source)
            })
            .expect("available source should be indexed");

        assert_eq!(summary.source_dir_count, 2);
        assert_eq!(summary.new_clip_count, 1);
        assert_eq!(summary.missing_clip_count, 0);
        assert!(!summary.errors.is_empty());
        assert_eq!(scan_run_count(&connection), 1);
        let scan_status: String = connection
            .query_row("SELECT status FROM scan_runs", [], |row| row.get(0))
            .expect("partial scan status should load");
        assert_eq!(scan_status, "partial");
        assert_eq!(unavailable.status, "unavailable");
        assert!(unavailable.last_error.is_some());
        assert_eq!(
            unavailable.last_scan_at.as_deref(),
            Some(initial_unavailable_last_scan.as_str()),
            "partial jobs must preserve the prior freshness timestamp",
        );
        assert_eq!(unavailable.clip_count, 1);
        assert_eq!(available.status, "available");
        assert_eq!(
            available.last_scan_at, None,
            "a successful source inside an overall partial job must not look fresh",
        );
        assert_eq!(available.clip_count, 1);
        assert_eq!(clips.len(), 2);
        assert!(clips
            .iter()
            .any(|clip| { clip.file_name == "old.mp4" && clip.status == "available" }));
        assert!(clips
            .iter()
            .any(|clip| { clip.file_name == "new.mp4" && clip.status == "available" }));
    }

    #[test]
    fn scan_roots_imports_one_shared_metadata_snapshot() {
        let fixture = ScanBatchFixture::new("shared-metadata");
        let archive_a = fixture.path().join("ArchiveA");
        let archive_b = fixture.path().join("ArchiveB");
        prepare_empty_metadata_root(&archive_a);
        let sources = [
            archive_a.join("wonderfulVideos1001"),
            archive_b.join("wonderfulVideos1001"),
        ];
        for (index, source) in sources.iter().enumerate() {
            let group = source.join("match-shared");
            fs::create_dir_all(&group).expect("shared match group should be created");
            fs::write(group.join(format!("clip-{index}.mp4")), b"video")
                .expect("shared match clip should be written");
        }
        let leveldb_payload = r#"[{
            "battleId":"battle-shared",
            "matchId":"match-shared",
            "kills":18,
            "deaths":7,
            "assists":3,
            "date":"2026-07-01T10:00:00Z"
        }]"#;
        fs::write(
            archive_a
                .join("Local Storage")
                .join("leveldb")
                .join("000003.ldb"),
            leveldb_blob("1001", leveldb_payload),
        )
        .expect("shared LevelDB fixture should be written");
        let log_content = concat!(
            "2026-07-01 first request data is [{",
            r#""matchId":"match-shared","#,
            r#""battleId":"battle-shared","#,
            r#""recordSrc":"D:/ArchiveA/wonderfulVideos1001/match-shared","#,
            r#""player":{"name":"PlayerOne#0000"},"#,
            r#""map":{"id":"maps/ascent","name":"Ascent"},"#,
            r#""mode":"Competitive","#,
            r#""agent":{"name":"Jett"},"#,
            r#""killEvents":[{"#,
            r#""eventTime":"2026-07-01T10:00:31Z","#,
            r#""roundId":3,"#,
            r#""weaponName":"Vandal","#,
            r#""killerName":"PlayerOne#0000","#,
            r#""killedName":"Opponent#0000""#,
            "}]}]"
        );
        fs::write(archive_a.join("logs").join("highlight.log"), log_content)
            .expect("shared log fixture should be written");
        let connection = fixture.open();
        connection
            .execute_batch(
                "
                CREATE TABLE match_write_audit (operation TEXT NOT NULL);
                CREATE TRIGGER audit_match_insert
                AFTER INSERT ON matches
                BEGIN
                    INSERT INTO match_write_audit (operation) VALUES ('insert');
                END;
                CREATE TRIGGER audit_match_update
                AFTER UPDATE ON matches
                BEGIN
                    INSERT INTO match_write_audit (operation) VALUES ('update');
                END;
                ",
            )
            .expect("metadata write audit should be installed");

        let summary = crate::scanner::scan_roots(&connection, &sources)
            .expect("shared metadata scan should run");
        let match_write_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM match_write_audit", [], |row| {
                row.get(0)
            })
            .expect("metadata write count should load");
        let clips = db::list_clips(&connection).expect("enriched clips should list");

        assert_eq!(summary.metadata_match_count, 1);
        assert_eq!(summary.metadata_enriched_clip_count, 2);
        assert_eq!(summary.metadata_event_count, 1);
        assert_eq!(summary.metadata_warning_count, 0);
        assert_eq!(match_write_count, 1);
        assert_eq!(scan_run_count(&connection), 1);
        assert_eq!(clips.len(), 2);
        assert!(clips
            .iter()
            .all(|clip| clip.player_name.as_deref() == Some("PlayerOne#0000")));
    }

    #[test]
    fn scan_roots_import_metadata_from_each_external_aclos_root() {
        let fixture = ScanBatchFixture::new("multiple-external-metadata");
        let archive_a = fixture.path().join("FriendA");
        let archive_b = fixture.path().join("FriendB");
        prepare_empty_metadata_root(&archive_a);
        prepare_empty_metadata_root(&archive_b);
        let fixtures = [
            (&archive_a, "1001", "match-friend-a", "friend-a.mp4"),
            (&archive_b, "2002", "match-friend-b", "friend-b.mp4"),
        ];
        let mut sources = Vec::new();
        for (archive, openid, match_id, file_name) in fixtures {
            let source = archive.join(format!("wonderfulVideos{openid}"));
            let group = source.join(match_id);
            fs::create_dir_all(&group).expect("external match group should be created");
            fs::write(group.join(file_name), b"video").expect("external clip should be written");
            let payload = format!(
                r#"[{{"battleId":"battle-{openid}","matchId":"{match_id}","kills":12,"deaths":8,"assists":4,"date":"2026-07-01T10:00:00Z"}}]"#
            );
            fs::write(
                archive
                    .join("Local Storage")
                    .join("leveldb")
                    .join("000003.ldb"),
                leveldb_blob(openid, &payload),
            )
            .expect("external LevelDB fixture should be written");
            sources.push(source);
        }
        let connection = fixture.open();

        let summary = crate::scanner::scan_roots(&connection, &sources)
            .expect("multi-root metadata scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(summary.metadata_match_count, 2);
        assert_eq!(summary.metadata_enriched_clip_count, 2);
        assert_eq!(clips.len(), 2);
        assert!(clips.iter().all(|clip| clip.match_id.is_some()));
    }

    #[test]
    fn scan_roots_resolves_same_openid_globally_by_newest_wonderful_timestamp() {
        let fixture = ScanBatchFixture::new("multiple-roots-same-openid");
        let openid = "90000000000000000006";
        let cases = [
            (
                "A-New",
                "match-new",
                "new-video",
                "2026-07-04T12:00:00Z",
                "NewestGlobalName",
                "2002",
            ),
            (
                "Z-Old",
                "match-old",
                "old-video",
                "2026-07-01T12:00:00Z",
                "OlderRootName",
                "1001",
            ),
        ];
        let mut sources = Vec::new();
        for (archive_name, match_id, video_id, match_time, name, tag) in cases {
            let archive = fixture.path().join(archive_name);
            prepare_empty_metadata_root(&archive);
            let source = archive.join(format!("wonderfulVideos{openid}"));
            let group = source.join(match_id);
            fs::create_dir_all(&group).expect("external match group should be created");
            fs::write(group.join(format!("{video_id}.mp4")), b"video")
                .expect("external clip should be written");
            let plaintext = format!(
                r#"{{"key_wonderful_list_{openid}":[{{"matches_id":"{match_id}","match_startTime":"{match_time}","user_name":"{name}","user_nick_id":"{tag}","videos":[{{"video_id":"{video_id}","video_name":"击杀集锦","video_type":"2","round_clips":[]}}]}}]}}"#,
            );
            fs::write(
                archive.join("WonderfulDb").join(openid),
                encrypt_wonderful_db_text(openid, &plaintext),
            )
            .expect("WonderfulDb account fixture should be written");
            sources.push(source);
        }
        let connection = fixture.open();

        crate::scanner::scan_roots(&connection, &sources)
            .expect("multi-root WonderfulDb scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips.len(), 2);
        assert!(clips.iter().all(|clip| {
            clip.account_name.as_deref() == Some("NewestGlobalName#2002")
                && clip.player_name.as_deref() == Some("NewestGlobalName#2002")
        }));
    }

    /// Reproduces the shipped default layout: recordings live under several account directories
    /// that carry no metadata of their own, while `WonderfulDb` sits in the APPDATA root. Gating
    /// the AppData fallback on a single metadata anchor stranded every multi-account library,
    /// because each `wonderfulVideos<openid>` parent contributes its own anchor.
    #[test]
    fn scan_roots_falls_back_to_appdata_metadata_for_multiple_anchors() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = ScanBatchFixture::new("multi-anchor-appdata-fallback");
        let original_appdata = std::env::var_os("APPDATA");
        let appdata_root = fixture.path().join("Roaming");
        let appdata_aclos_root = appdata_root.join("ACLOS");
        prepare_empty_metadata_root(&appdata_aclos_root);

        // Two accounts stored under two unrelated parents, mirroring "素材搬到别的盘" setups.
        let cases = [
            ("ArchiveA", "90000000000000000006", "match-a", "video-a"),
            ("ArchiveB", "90000000000000000007", "match-b", "video-b"),
        ];
        let mut sources = Vec::new();
        for (archive_name, openid, match_id, video_id) in cases {
            let archive = fixture.path().join(archive_name);
            let source = archive.join(format!("wonderfulVideos{openid}"));
            let group = source.join(match_id);
            fs::create_dir_all(&group).expect("match group should be created");
            fs::write(group.join(format!("{video_id}.mp4")), b"video")
                .expect("clip should be written");
            let plaintext = format!(
                r#"{{"key_wonderful_list_{openid}":[{{"matches_id":"{match_id}","match_startTime":"2026-07-04T12:00:00Z","user_name":"Player{openid}","user_nick_id":"2002","match_map":"隐世修所","videos":[{{"video_id":"{video_id}","video_name":"击杀集锦","video_type":"2","round_clips":[]}}]}}]}}"#,
            );
            // Metadata exists ONLY in the APPDATA root, never beside the recordings.
            fs::write(
                appdata_aclos_root.join("WonderfulDb").join(openid),
                encrypt_wonderful_db_text(openid, &plaintext),
            )
            .expect("appdata WonderfulDb fixture should be written");
            sources.push(source);
        }

        std::env::set_var("APPDATA", &appdata_root);
        let connection = fixture.open();
        let summary = crate::scanner::scan_roots(&connection, &sources);
        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        let summary = summary.expect("multi-anchor metadata scan should run");

        let clips = db::list_clips(&connection).expect("clips should list");
        assert_eq!(clips.len(), 2);
        // Each anchor reads the shared AppData WonderfulDb, so the per-anchor counter observes
        // every video once per anchor. Ingest upserts are idempotent, so assert on the resulting
        // rows rather than on that inflated tally.
        assert!(summary.metadata_enriched_clip_count >= 2);
        for clip in &clips {
            assert_eq!(clip.map_name.as_deref(), Some("隐世修所"));
            assert!(
                clip.account_name
                    .as_deref()
                    .is_some_and(|name| name.ends_with("#2002")),
                "each account should resolve its Riot ID from the AppData WonderfulDb"
            );
        }
    }

    #[test]
    fn scan_custom_directory_matches_scan_roots_summary_for_direct_source() {
        let fixture = ScanBatchFixture::new("compat-summary");
        let library = fixture.path().join("Library");
        prepare_empty_metadata_root(&library);
        let source = library.join("wonderfulVideos1001");
        let group = source.join("match-a");
        fs::create_dir_all(&group).expect("source group should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("source clip should be written");
        let new_connection = fixture.open();
        let compatibility_database = fixture.path().join("compatibility.sqlite3");
        db::migrate_database(&compatibility_database)
            .expect("compatibility database should migrate explicitly");
        let compatibility_connection =
            db::open_database(&compatibility_database).expect("compatibility database should open");

        let new_summary =
            crate::scanner::scan_roots(&new_connection, std::slice::from_ref(&source))
                .expect("new scan core should run");
        let compatibility_summary =
            crate::scanner::scan_custom_directory(&compatibility_connection, &source)
                .expect("compatibility scan should run");

        assert_eq!(compatibility_summary, new_summary);
        assert_eq!(scan_run_count(&new_connection), 1);
        assert_eq!(scan_run_count(&compatibility_connection), 1);
    }

    #[test]
    fn scan_imports_wonderful_db_after_indexing_files() {
        let fixture = TestFixture::new("wonderful-db-ingest");
        let aclos_root = fixture.path().join("ACLOS");
        let root = aclos_root.join("aclos-highlight");
        let group = root.join("wonderfulVideos1001").join("match-a");
        let wonderful_dir = aclos_root.join("WonderfulDb");
        fs::create_dir_all(&group).expect("clip group should be created");
        fs::create_dir_all(&wonderful_dir).expect("WonderfulDb should be created");
        fs::create_dir_all(aclos_root.join("Local Storage").join("leveldb"))
            .expect("local metadata root should be created");
        let clip_path = group.join("video-six.mp4");
        fs::write(&clip_path, b"video-six").expect("clip should be written");
        let events = (0..6)
            .map(|index| {
                format!(
                    r#"{{"event_id":"event-{index}","event_type":"kill","event_sTime":{},"killer_is_me":true}}"#,
                    index * 500
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let plaintext = format!(
            r#"{{"key_wonderful_list_1001":[{{"matches_id":"match-a","match_startTime":"2026-07-04T12:00:00Z","match_map":"隐世修所","videos":[{{"video_id":"video-six","video_name":"六杀时刻","video_type":"10","round_clips":[{{"segment_id":"segment-a","round_id":7,"clip_sTime":1000,"clip_eTime":5000,"clip_events":[{events}]}}]}}]}}]}}"#
        );
        fs::write(
            wonderful_dir.join("1001"),
            encrypt_wonderful_db_text("1001", &plaintext),
        )
        .expect("WonderfulDb account fixture should be written");
        fs::write(wonderful_dir.join("1002"), "not-hex")
            .expect("corrupt WonderfulDb account fixture should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, &root).expect("scan should run");
        let normalized_path = db::normalize_path(&super::path_to_string(&clip_path));
        let (clip_id, official_video_name, kill_count): (i64, Option<String>, Option<i64>) =
            connection
                .query_row(
                    "
                    SELECT clips.id, clip_metadata.official_video_name, clip_metadata.kill_count
                    FROM clips
                    JOIN clip_metadata ON clip_metadata.clip_id = clips.id
                    WHERE clips.normalized_path = ?1
                    ",
                    rusqlite::params![normalized_path],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("indexed clip metadata should load");

        assert_eq!(official_video_name.as_deref(), Some("六杀时刻"));
        assert_eq!(kill_count, Some(6));
        assert_eq!(summary.metadata_warning_count, 1);
        assert_eq!(
            db::list_clip_events_for_clip(&connection, clip_id)
                .expect("clip events should load")
                .len(),
            6
        );

        let metadata_dir = root.join("wonderfulVideos1001").join("videoExportTmp");
        fs::create_dir_all(&metadata_dir).expect("fallback metadata dir should be created");
        fs::write(
            metadata_dir.join("config-fallback.json"),
            r#"{"玩家昵称":"Fallback#0001","地图":"天枢之阙","游戏模式":"竞技模式","KDA":"1/1/1"}"#,
        )
        .expect("fallback metadata should be written");
        fs::remove_file(wonderful_dir.join("1001"))
            .expect("valid WonderfulDb fixture should be removed");

        crate::scanner::scan_directory(&connection, &root).expect("rescan should run");
        let preserved: (String, Option<String>, Option<String>, Option<i64>, Option<String>) =
            connection
                .query_row(
                    "
                    SELECT metadata_status, map_name, official_video_name, kill_count, metadata_source
                    FROM clip_metadata
                    WHERE clip_id = ?1
                    ",
                    rusqlite::params![clip_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .expect("rescanned official metadata should load");
        assert_eq!(preserved.0, "enriched");
        assert_eq!(preserved.1.as_deref(), Some("隐世修所"));
        assert_eq!(preserved.2.as_deref(), Some("六杀时刻"));
        assert_eq!(preserved.3, Some(6));
        assert_eq!(preserved.4.as_deref(), Some("wonderful_db"));
        let clip_times: (Option<i64>, Option<String>) = connection
            .query_row(
                "SELECT duration_ms, recorded_at FROM clips WHERE id = ?1",
                rusqlite::params![clip_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("rescanned official clip times should load");
        assert_eq!(clip_times.0, Some(5_000));
        assert_eq!(clip_times.1.as_deref(), Some("2026-07-04T12:00:00Z"));
    }

    #[test]
    fn official_ingest_replaces_fallback_video_type_without_creating_tags() {
        let fixture = TestFixture::new("official-video-type-wins");
        let aclos_root = fixture.path().join("ACLOS");
        let root = aclos_root.join("aclos-highlight");
        let source = root.join("wonderfulVideos1001");
        let group = source.join("match-death");
        let metadata_dir = source.join("videoExportTmp");
        let wonderful_dir = aclos_root.join("WonderfulDb");
        fs::create_dir_all(&group).expect("clip group should be created");
        fs::create_dir_all(&metadata_dir).expect("fallback metadata dir should be created");
        fs::create_dir_all(&wonderful_dir).expect("WonderfulDb should be created");
        fs::write(group.join("death.mp4"), b"death-video").expect("clip should be written");
        fs::write(
            metadata_dir.join("config-death.json"),
            r#"{"title":"ACE 击杀合集","玩家昵称":"Fallback#0001","地图":"天枢之阙"}"#,
        )
        .expect("fallback metadata should be written");
        let plaintext = r#"{"key_wonderful_list_1001":[{"matches_id":"match-death","videos":[{"video_id":"death","video_name":"死亡集锦","video_type":"3","round_clips":[]}]}]}"#;
        fs::write(
            wonderful_dir.join("1001"),
            encrypt_wonderful_db_text("1001", plaintext),
        )
        .expect("WonderfulDb account fixture should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        crate::scanner::scan_directory(&connection, &root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips[0].highlight_type, Some(3));
        assert_eq!(clips[0].official_video_name.as_deref(), Some("死亡集锦"));
        assert!(clips[0].tag_ids.is_empty());
    }

    #[test]
    fn scan_directory_assigns_matching_cover_to_each_clip() {
        let fixture = TestFixture::new("matching-covers");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let group = source.join("11111111-1111-1111-1111-111111111111");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(group.join("clutch.mp4"), b"video-two").expect("clip should be written");
        fs::write(group.join("cover-ace.jpeg"), b"cover-one").expect("cover should be written");
        fs::write(group.join("cover-clutch.jpeg"), b"cover-two").expect("cover should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");
        let ace = clips
            .iter()
            .find(|clip| clip.file_name == "ace.mp4")
            .expect("ace clip should exist");
        let clutch = clips
            .iter()
            .find(|clip| clip.file_name == "clutch.mp4")
            .expect("clutch clip should exist");

        assert_eq!(summary.cover_missing_count, 0);
        assert!(ace
            .cover_path
            .as_deref()
            .unwrap_or_default()
            .ends_with("cover-ace.jpeg"));
        assert!(clutch
            .cover_path
            .as_deref()
            .unwrap_or_default()
            .ends_with("cover-clutch.jpeg"));
    }

    #[test]
    fn scan_directory_ignores_package_match_covers_for_clip_thumbnails() {
        let fixture = TestFixture::new("package-summary-cover");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let group_with_cover = source.join("match-a");
        let group_without_cover = source.join("match-b");
        let snapshot_dir = source.join("snapshot");
        fs::create_dir_all(&group_with_cover).expect("group should be created");
        fs::create_dir_all(&group_without_cover).expect("group should be created");
        fs::create_dir_all(&snapshot_dir).expect("snapshot dir should be created");
        let clip_with_cover_path = group_with_cover.join("ace.mp4");
        let clip_without_cover_path = group_without_cover.join("clutch.mp4");
        let ordinary_cover = group_with_cover.join("cover-ace.jpeg");
        let package_cover = snapshot_dir.join("package_match_a.jpeg");
        fs::write(&clip_with_cover_path, b"video-one").expect("clip should be written");
        fs::write(&clip_without_cover_path, b"video-two").expect("clip should be written");
        fs::write(&ordinary_cover, b"ordinary-cover").expect("cover should be written");
        fs::write(&package_cover, b"summary-cover").expect("package cover should be written");
        let clip_time = UNIX_EPOCH + Duration::from_secs(1_782_000_000);
        set_file_modified_time(&clip_with_cover_path, clip_time);
        set_file_modified_time(&clip_without_cover_path, clip_time);
        set_file_modified_time(&ordinary_cover, clip_time);
        set_file_modified_time(&package_cover, clip_time + Duration::from_secs(4));

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");
        let clip_with_cover = clips
            .iter()
            .find(|clip| clip.file_name == "ace.mp4")
            .expect("clip with cover should exist");
        let clip_without_cover = clips
            .iter()
            .find(|clip| clip.file_name == "clutch.mp4")
            .expect("clip without cover should exist");

        assert_eq!(summary.cover_missing_count, 1);
        assert_eq!(clips.len(), 2);
        assert_eq!(clip_with_cover.cover_source, "file");
        assert_eq!(
            clip_with_cover.cover_path.as_deref(),
            Some(super::path_to_string(&ordinary_cover).as_str())
        );
        assert_eq!(clip_without_cover.cover_source, "missing");
        assert_eq!(clip_without_cover.cover_path, None);
    }

    #[test]
    fn scan_directory_stores_source_metadata_on_clips() {
        let fixture = TestFixture::new("metadata");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let metadata_dir = source.join("videoExportTmp");
        let group = source.join("11111111-1111-1111-1111-111111111111");
        fs::create_dir_all(&metadata_dir).expect("metadata dir should be created");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(
            metadata_dir.join("config-player.json"),
            r#"{
                "玩家昵称": "FixtureAlpha#0001",
                "地图": "天枢之阙",
                "游戏模式": "竞技模式",
                "KDA": "36/17/6"
            }"#,
        )
        .expect("metadata should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].account_name.as_deref(), Some("FixtureAlpha#0001"));
        assert_eq!(clips[0].player_name.as_deref(), Some("FixtureAlpha#0001"));
        assert_eq!(clips[0].map_name.as_deref(), Some("天枢之阙"));
        assert_eq!(clips[0].game_mode.as_deref(), Some("竞技模式"));
        assert_eq!(clips[0].kda.as_deref(), Some("36/17/6"));
        assert!(clips[0].extracted_text.contains("FixtureAlpha#0001"));
    }

    #[test]
    fn scan_directory_persists_exported_video_type_without_creating_tags() {
        let fixture = TestFixture::new("metadata-video-type");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let metadata_dir = source.join("videoExportTmp");
        let group = source.join("11111111-1111-1111-1111-111111111111");
        fs::create_dir_all(&metadata_dir).expect("metadata dir should be created");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(
            metadata_dir.join("config-video-type.json"),
            r#"{
                "title": "六杀时刻",
                "玩家昵称": "FixtureAlpha#0001",
                "地图": "天枢之阙"
            }"#,
        )
        .expect("metadata should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].highlight_type, Some(10));
        assert_eq!(clips[0].kill_count, Some(6));
        assert!(clips[0].tag_ids.is_empty());
    }

    #[test]
    fn scan_directory_uses_nearest_export_config_for_each_clip_group() {
        let fixture = TestFixture::new("metadata-nearest-config");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let metadata_dir = source.join("videoExportTmp");
        let old_group = source.join("old-match");
        let new_group = source.join("new-match");
        fs::create_dir_all(&metadata_dir).expect("metadata dir should be created");
        fs::create_dir_all(&old_group).expect("old group should be created");
        fs::create_dir_all(&new_group).expect("new group should be created");

        let old_config = metadata_dir.join("config-100-old.json");
        let new_config = metadata_dir.join("config-200-new.json");
        let old_clip = old_group.join("old.mp4");
        let new_clip = new_group.join("new.mp4");
        fs::write(
            &old_config,
            r#"{"玩家昵称":"Old#0001","地图":"源工重镇","游戏模式":"竞技模式","KDA":"10/1/2"}"#,
        )
        .expect("old metadata should be written");
        fs::write(
            &new_config,
            r#"{"玩家昵称":"New#0002","地图":"隐世修所","游戏模式":"竞技模式","KDA":"20/2/4"}"#,
        )
        .expect("new metadata should be written");
        fs::write(&old_clip, b"old-video").expect("old clip should be written");
        fs::write(&new_clip, b"new-video").expect("new clip should be written");

        let old_time = UNIX_EPOCH + Duration::from_secs(1_782_000_000);
        let new_time = old_time + Duration::from_secs(600);
        set_file_modified_time(&old_config, old_time);
        set_file_modified_time(&old_clip, old_time + Duration::from_secs(3));
        set_file_modified_time(&new_config, new_time);
        set_file_modified_time(&new_clip, new_time + Duration::from_secs(3));

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");
        let old = clips
            .iter()
            .find(|clip| clip.clip_group_name.as_deref() == Some("old-match"))
            .expect("old clip should exist");
        let new = clips
            .iter()
            .find(|clip| clip.clip_group_name.as_deref() == Some("new-match"))
            .expect("new clip should exist");

        assert_eq!(old.account_name.as_deref(), Some("Old#0001"));
        assert_eq!(old.map_name.as_deref(), Some("源工重镇"));
        assert_eq!(old.kda.as_deref(), Some("10/1/2"));
        assert_eq!(new.account_name.as_deref(), Some("New#0002"));
        assert_eq!(new.map_name.as_deref(), Some("隐世修所"));
        assert_eq!(new.kda.as_deref(), Some("20/2/4"));
    }

    #[test]
    fn scan_directory_clears_stale_asset_account_metadata_without_source_config() {
        let fixture = TestFixture::new("stale-asset-account");
        let root = fixture.path();
        let source = root.join("wonderfulVideos1001");
        let group = source.join("match-a-001");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        crate::scanner::scan_directory(&connection, root).expect("initial scan should run");
        let clip_id = db::list_clips(&connection).expect("clips should list")[0].id;
        db::upsert_clip_metadata(
            &connection,
            db::ClipMetadataInput {
                clip_id,
                metadata_status: "parsed",
                json_path: None,
                account_name: Some("Cards/D3018FBE-45CD-786A-DD6C-BCAF429F7096.png"),
                player_name: Some("Cards/D3018FBE-45CD-786A-DD6C-BCAF429F7096.png"),
                agent_name: None,
                map_name: Some("隐世修所"),
                game_mode: Some("竞技模式"),
                scoreline: None,
                kda: Some("27/22/6"),
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("stale metadata should seed");

        crate::scanner::scan_directory(&connection, root).expect("rescan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips[0].account_name, None);
        assert_eq!(clips[0].player_name, None);
        assert_eq!(clips[0].map_name.as_deref(), Some("隐世修所"));
        assert_eq!(clips[0].game_mode.as_deref(), Some("竞技模式"));
    }

    #[test]
    fn scan_directory_backfills_agent_from_nearby_edit_asset() {
        let fixture = TestFixture::new("edit-agent-asset");
        let root = fixture.path();
        let source = root.join("wonderfulVideos1001");
        let group = source.join("match-a-001");
        let edit_dir = source.join("edit");
        fs::create_dir_all(&group).expect("group should be created");
        fs::create_dir_all(&edit_dir).expect("edit dir should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(
            edit_dir.join(
                "aHR0cHM6Ly9nYW1lLmd0aW1nLmNuL2ltYWdlcy92YWwvYWdhbWV6bGsvYWdlbnRiYWNrZ3JvdW5kL2FnZW50LzE3LnBuZw==.png",
            ),
            b"agent-background",
        )
        .expect("agent asset should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].metadata_status, "partial");
        assert_eq!(clips[0].agent_name.as_deref(), Some("Chamber"));
    }

    #[test]
    fn scan_directory_runs_metadata_ingest_for_matching_match_group() {
        let fixture = TestFixture::new("metadata-ingest");
        let aclos_root = fixture.path().join("ACLOS");
        prepare_empty_metadata_root(&aclos_root);
        let root = aclos_root.join("aclos-highlight");
        let source = root.join("wonderfulVideos1001");
        let group = source.join("match-a-001");
        let leveldb_dir = aclos_root.join("Local Storage").join("leveldb");
        let logs_dir = aclos_root.join("logs");
        fs::create_dir_all(&group).expect("group should be created");
        fs::create_dir_all(&leveldb_dir).expect("leveldb dir should be created");
        fs::create_dir_all(&logs_dir).expect("logs dir should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");
        let leveldb_payload = r#"[{
            "battleId":"battle-a-001",
            "matchId":"match-a-001",
            "kills":18,
            "deaths":7,
            "assists":3,
            "date":"2026-07-01T10:00:00Z",
            "heroAvatarUrl":"https://assets.example/jett.png"
        }]"#;
        let log_content = concat!(
            "2026-07-01 first request data is [{",
            r#""matchId":"match-a-001","#,
            r#""battleId":"battle-a-001","#,
            r#""recordSrc":"D:/ACLOS/aclos-highlight/wonderfulVideos1001/match-a-001","#,
            r#""player":{"name":"PlayerOne#0000"},"#,
            r#""map":{"id":"maps/ascent","name":"Ascent"},"#,
            r#""mode":"Competitive","#,
            r#""agent":{"name":"Jett"},"#,
            r#""score":{"roundsWon":13,"roundsLost":11,"hasWon":true},"#,
            r#""combatScore":287,"#,
            r#""killEvents":[{"#,
            r#""eventTime":"2026-07-01T10:00:31Z","#,
            r#""roundId":3,"#,
            r#""weaponName":"Vandal","#,
            r#""killerName":"PlayerOne#0000","#,
            r#""killedName":"Opponent#0000""#,
            "}]}]"
        );
        fs::write(
            leveldb_dir.join("000003.ldb"),
            leveldb_blob("1001", leveldb_payload),
        )
        .expect("leveldb fixture should be written");
        fs::write(logs_dir.join("highlight.log"), log_content)
            .expect("log fixture should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, &root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(summary.metadata_match_count, 1);
        assert_eq!(summary.metadata_enriched_clip_count, 1);
        assert_eq!(summary.metadata_event_count, 1);
        assert_eq!(summary.metadata_warning_count, 0);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].player_name.as_deref(), Some("PlayerOne#0000"));
        assert_eq!(clips[0].agent_name.as_deref(), Some("Jett"));
        assert_eq!(clips[0].map_name.as_deref(), Some("亚海悬城"));
        assert_eq!(clips[0].game_mode.as_deref(), Some("竞技模式"));
        assert_eq!(clips[0].scoreline.as_deref(), Some("13/11"));
        assert_eq!(clips[0].kda.as_deref(), Some("18/7/3"));
        assert_eq!(clips[0].recorded_at.as_deref(), None);

        let latest = crate::scanner::latest_scan_summary(&connection)
            .expect("summary query should run")
            .expect("summary should exist");
        assert_eq!(latest.metadata_match_count, 1);
        assert_eq!(latest.metadata_enriched_clip_count, 1);
        assert_eq!(latest.metadata_event_count, 1);
        assert_eq!(latest.metadata_warning_count, 0);
    }

    #[test]
    fn scan_directory_backfills_account_name_from_leveldb_account_roles() {
        let fixture = TestFixture::new("account-role-ingest");
        let aclos_root = fixture.path().join("ACLOS");
        prepare_empty_metadata_root(&aclos_root);
        let root = aclos_root.join("aclos-highlight");
        let source = root.join("wonderfulVideos9000000000000000002");
        let group = source.join("match-without-full-metadata");
        let leveldb_dir = aclos_root.join("Local Storage").join("leveldb");
        fs::create_dir_all(&group).expect("group should be created");
        fs::create_dir_all(&leveldb_dir).expect("leveldb dir should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(
            leveldb_dir.join("000005.ldb"),
            account_roles_blob(
                r#"[{"openid":"9000000000000000002","nick":"FixtureBravo","tag":"0002"}]"#,
            ),
        )
        .expect("leveldb account role fixture should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        crate::scanner::scan_directory(&connection, &root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].account_name.as_deref(), Some("FixtureBravo#0002"));
        assert_eq!(clips[0].player_name.as_deref(), Some("FixtureBravo#0002"));
    }

    #[test]
    fn scan_directory_backfills_account_name_from_highlight_log_account_hints() {
        let fixture = TestFixture::new("log-account-role-ingest");
        let aclos_root = fixture.path().join("ACLOS");
        prepare_empty_metadata_root(&aclos_root);
        let root = aclos_root.join("aclos-highlight");
        let source = root.join("wonderfulVideos90000000000000000005");
        let group = source.join("match-without-full-metadata");
        let logs_dir = aclos_root.join("logs");
        fs::create_dir_all(&group).expect("group should be created");
        fs::create_dir_all(&logs_dir).expect("logs dir should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(
            logs_dir.join("highlight.log"),
            r#"RESPONSE: {"result":0,"data":{"list":[{"role_name":"FixtureCharlie","nick_id":"0004","g_open_id":"90000000000000000005"}]}}"#,
        )
        .expect("highlight log fixture should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        crate::scanner::scan_directory(&connection, &root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(clips.len(), 1);
        assert_eq!(
            clips[0].account_name.as_deref(),
            Some("FixtureCharlie#0004")
        );
        assert_eq!(clips[0].player_name.as_deref(), Some("FixtureCharlie#0004"));
    }

    #[test]
    fn metadata_source_paths_fall_back_to_appdata_when_scan_parent_has_no_metadata() {
        let fixture = TestFixture::new("metadata-paths");
        let scan_aclos_root = fixture.path().join("AppData").join("ACLOS");
        let appdata_aclos_root = fixture.path().join("Roaming").join("ACLOS");
        let scan_root = scan_aclos_root.join("aclos-highlight");
        fs::create_dir_all(&scan_root).expect("scan root should be created");
        fs::create_dir_all(appdata_aclos_root.join("Local Storage").join("leveldb"))
            .expect("appdata leveldb should be created");
        fs::create_dir_all(appdata_aclos_root.join("logs"))
            .expect("appdata logs should be created");
        fs::write(appdata_aclos_root.join("logs").join("highlight.log"), b"")
            .expect("appdata log should be written");

        let selected_root =
            super::select_metadata_aclos_root(Some(scan_aclos_root), appdata_aclos_root.clone());

        assert_eq!(selected_root, appdata_aclos_root);
    }

    #[test]
    fn metadata_source_paths_select_wonderful_db_independently_from_logs() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = TestFixture::new("split-metadata-paths");
        let original_appdata = std::env::var_os("APPDATA");
        let candidate_aclos_root = fixture.path().join("Local").join("ACLOS");
        let scan_root = candidate_aclos_root.join("aclos-highlight");
        let appdata = fixture.path().join("Roaming");
        let appdata_aclos_root = appdata.join("ACLOS");
        fs::create_dir_all(candidate_aclos_root.join("logs"))
            .expect("candidate logs should be created");
        fs::write(candidate_aclos_root.join("logs").join("highlight.log"), b"")
            .expect("candidate log should be written");
        fs::create_dir_all(appdata_aclos_root.join("WonderfulDb"))
            .expect("appdata WonderfulDb should be created");
        std::env::set_var("APPDATA", &appdata);

        let paths = super::metadata_source_paths(&scan_root, false);

        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        assert_eq!(paths.logs_dir, candidate_aclos_root.join("logs"));
        assert_eq!(
            paths.leveldb_dir,
            candidate_aclos_root.join("Local Storage").join("leveldb")
        );
        assert_eq!(paths.wonderful_dir, appdata_aclos_root.join("WonderfulDb"));
    }

    #[test]
    fn metadata_source_paths_prefer_adjacent_wonderful_db() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = TestFixture::new("adjacent-wonderful-path");
        let original_appdata = std::env::var_os("APPDATA");
        let candidate_aclos_root = fixture.path().join("Local").join("ACLOS");
        let scan_root = candidate_aclos_root.join("aclos-highlight");
        let appdata = fixture.path().join("Roaming");
        fs::create_dir_all(candidate_aclos_root.join("WonderfulDb"))
            .expect("candidate WonderfulDb should be created");
        fs::create_dir_all(appdata.join("ACLOS").join("WonderfulDb"))
            .expect("appdata WonderfulDb should be created");
        std::env::set_var("APPDATA", &appdata);

        let paths = super::metadata_source_paths(&scan_root, false);

        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        assert_eq!(
            paths.wonderful_dir,
            candidate_aclos_root.join("WonderfulDb")
        );
    }

    #[test]
    fn metadata_source_paths_resolve_a_direct_wonderful_videos_selection() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = TestFixture::new("direct-wonderful-videos-path");
        let original_appdata = std::env::var_os("APPDATA");
        let imported_aclos_root = fixture.path().join("Friend Export").join("ACLOS");
        let direct_source = imported_aclos_root
            .join("aclos-highlight")
            .join("wonderfulVideos90000000000000000006");
        let local_appdata = fixture.path().join("Local AppData");
        fs::create_dir_all(&direct_source).expect("direct source should be created");
        fs::create_dir_all(imported_aclos_root.join("WonderfulDb"))
            .expect("imported WonderfulDb should be created");
        fs::create_dir_all(local_appdata.join("ACLOS").join("WonderfulDb"))
            .expect("local WonderfulDb should be created");
        std::env::set_var("APPDATA", &local_appdata);

        let paths = super::metadata_source_paths(&direct_source, true);

        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        assert_eq!(paths.wonderful_dir, imported_aclos_root.join("WonderfulDb"));
    }

    #[test]
    fn metadata_source_paths_fall_back_to_appdata_for_custom_roots_without_metadata() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = TestFixture::new("metadata-paths-custom-root");
        let original_appdata = std::env::var_os("APPDATA");
        let custom_root = fixture.path().join("Imported");
        let appdata_aclos_root = fixture.path().join("Roaming").join("ACLOS");
        fs::create_dir_all(&custom_root).expect("custom root should be created");
        fs::create_dir_all(appdata_aclos_root.join("logs")).expect("appdata logs should exist");
        fs::write(appdata_aclos_root.join("logs").join("highlight.log"), b"")
            .expect("appdata log should be written");
        std::env::set_var("APPDATA", fixture.path().join("Roaming"));

        let paths = super::metadata_source_paths(&custom_root, true);

        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        assert_eq!(paths.logs_dir, appdata_aclos_root.join("logs"));
        assert_eq!(
            paths.leveldb_dir,
            appdata_aclos_root.join("Local Storage").join("leveldb")
        );
        assert_eq!(paths.wonderful_dir, appdata_aclos_root.join("WonderfulDb"));
        assert_eq!(paths.account_hint_scope, None);
    }

    #[test]
    fn metadata_source_paths_keep_custom_root_without_external_fallback() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = TestFixture::new("metadata-paths-custom-no-fallback");
        let original_appdata = std::env::var_os("APPDATA");
        let custom_root = fixture.path().join("Imported");
        let appdata_aclos_root = fixture.path().join("Roaming").join("ACLOS");
        fs::create_dir_all(&custom_root).expect("custom root should be created");
        fs::create_dir_all(appdata_aclos_root.join("logs")).expect("appdata logs should exist");
        fs::write(appdata_aclos_root.join("logs").join("highlight.log"), b"")
            .expect("appdata log should be written");
        std::env::set_var("APPDATA", fixture.path().join("Roaming"));

        let paths = super::metadata_source_paths(&custom_root, false);

        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        assert_eq!(paths.logs_dir, custom_root.join("logs"));
        assert_eq!(
            paths.leveldb_dir,
            custom_root.join("Local Storage").join("leveldb")
        );
        assert_eq!(paths.wonderful_dir, custom_root.join("WonderfulDb"));
        assert_eq!(
            paths.account_hint_scope.as_deref(),
            Some(custom_root.as_path())
        );
    }

    #[test]
    fn discovers_scan_roots_from_videocut_log() {
        let fixture = TestFixture::new("videocut-roots");
        let archive_root = fixture.path().join("Archive");
        let default_root = fixture
            .path()
            .join("AppData")
            .join("ACLOS")
            .join("aclos-highlight");
        fs::create_dir_all(&archive_root).expect("archive root should be created");
        fs::create_dir_all(&default_root).expect("default root should be created");
        let log_path = fixture.path().join("videocut.txt");
        fs::write(
            &log_path,
            format!(
                "[cut] clip file:{}\\wonderfulVideos1001\\snapshot\\snapshot_match_a.jpeg\n\
                 [cut] export video successfully, file path:{}\\wonderfulVideos1001\\videoExportTmp\\template.mp4\n\
                 [cut] clip file:{}\\wonderfulVideos2002\\snapshot\\snapshot_match_b.jpeg\n",
                archive_root.display(),
                archive_root.display(),
                default_root.display()
            ),
        )
        .expect("videocut fixture should be written");

        let roots =
            super::scan_roots_from_videocut_log(&log_path).expect("videocut roots should parse");

        assert_eq!(roots, vec![archive_root, default_root]);
    }

    #[test]
    fn scan_default_aclos_library_includes_videocut_external_roots() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = TestFixture::new("default-library-with-archive");
        let original_appdata = std::env::var_os("APPDATA");
        let original_userprofile = std::env::var_os("USERPROFILE");
        let user_profile = fixture.path().join("User");
        let appdata = fixture.path().join("Roaming");
        let default_root = user_profile
            .join("AppData")
            .join("ACLOS")
            .join("aclos-highlight");
        let archive_root = fixture.path().join("Archive");
        let default_group = default_root
            .join("wonderfulVideos1001")
            .join("default-match");
        let archive_group = archive_root
            .join("wonderfulVideos2002")
            .join("archive-match");
        fs::create_dir_all(&default_group).expect("default group should be created");
        fs::create_dir_all(&archive_group).expect("archive group should be created");
        fs::create_dir_all(appdata.join("ACLOS").join("logs"))
            .expect("fake logs dir should be created");
        fs::write(default_group.join("default.mp4"), b"default-video")
            .expect("default clip should be written");
        fs::write(archive_group.join("archive.mp4"), b"archive-video")
            .expect("archive clip should be written");
        fs::write(
            appdata.join("ACLOS").join("logs").join("videocut.txt"),
            format!(
                "[cut] clip file:{}\\wonderfulVideos2002\\snapshot\\snapshot_match_b.jpeg\n",
                archive_root.display()
            ),
        )
        .expect("videocut fixture should be written");
        std::env::set_var("APPDATA", &appdata);
        std::env::set_var("USERPROFILE", &user_profile);

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let summary = crate::scanner::scan_default_aclos_library(&connection)
            .expect("default library scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        assert_eq!(summary.source_dir_count, 2);
        assert_eq!(summary.clip_group_count, 2);
        assert_eq!(clips.len(), 2);
        assert!(clips
            .iter()
            .any(|clip| clip.video_path.ends_with("default.mp4")));
        assert!(clips
            .iter()
            .any(|clip| clip.video_path.ends_with("archive.mp4")));
    }

    #[test]
    fn scan_default_aclos_library_does_not_apply_default_log_account_hints_to_external_roots() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = TestFixture::new("default-library-hints-scoped-to-root");
        let original_appdata = std::env::var_os("APPDATA");
        let original_userprofile = std::env::var_os("USERPROFILE");
        let user_profile = fixture.path().join("User");
        let appdata = fixture.path().join("Roaming");
        let default_aclos_root = user_profile.join("AppData").join("ACLOS");
        let default_root = default_aclos_root.join("aclos-highlight");
        let archive_root = fixture.path().join("Archive");
        let account_id = "90000000000000000006";
        let default_group = default_root
            .join(format!("wonderfulVideos{account_id}"))
            .join("default-match");
        let archive_source = archive_root.join(format!("wonderfulVideos{account_id}"));
        let archive_group = archive_root
            .join(format!("wonderfulVideos{account_id}"))
            .join("archive-match");
        fs::create_dir_all(&default_group).expect("default group should be created");
        fs::create_dir_all(&archive_group).expect("archive group should be created");
        fs::create_dir_all(default_aclos_root.join("logs"))
            .expect("default logs dir should be created");
        fs::create_dir_all(appdata.join("ACLOS").join("logs"))
            .expect("appdata logs dir should be created");
        fs::write(default_group.join("default.mp4"), b"default-video")
            .expect("default clip should be written");
        fs::write(archive_group.join("archive.mp4"), b"archive-video")
            .expect("archive clip should be written");
        fs::write(
            default_aclos_root.join("logs").join("highlight.log"),
            format!(
                r#"RESPONSE: {{"result":0,"data":{{"list":[{{"role_name":"FixtureDelta","nick_id":"0004","g_open_id":"{account_id}"}}]}}}}"#
            ),
        )
        .expect("highlight log fixture should be written");
        fs::write(
            appdata.join("ACLOS").join("logs").join("videocut.txt"),
            format!(
                "[cut] clip file:{}\\wonderfulVideos{}\\snapshot\\snapshot_match_b.jpeg\n",
                archive_root.display(),
                account_id
            ),
        )
        .expect("videocut fixture should be written");
        std::env::set_var("APPDATA", &appdata);
        std::env::set_var("USERPROFILE", &user_profile);

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let seeded_source = db::upsert_source_dir(
            &connection,
            db::SourceDirInput {
                path: &super::path_to_string(&archive_source),
                name: &format!("wonderfulVideos{account_id}"),
            },
        )
        .expect("stale archive source should seed");
        let seeded_group = db::upsert_clip_group(
            &connection,
            db::ClipGroupInput {
                source_dir_id: seeded_source.id,
                group_key: "archive-match",
                display_name: "archive-match",
            },
        )
        .expect("stale archive group should seed");
        let seeded_clip = db::upsert_clip(
            &connection,
            db::ClipInput {
                source_dir_id: seeded_source.id,
                clip_group_id: Some(seeded_group.id),
                video_path: &super::path_to_string(&archive_group.join("archive.mp4")),
                file_name: "archive.mp4",
                file_size: 13,
                modified_at: None,
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("stale archive clip should seed");
        db::upsert_clip_metadata(
            &connection,
            db::ClipMetadataInput {
                clip_id: seeded_clip.id,
                metadata_status: "partial",
                json_path: None,
                account_name: Some("FixtureDelta#0004"),
                player_name: None,
                agent_name: Some("Jett"),
                map_name: None,
                game_mode: None,
                scoreline: None,
                kda: None,
                extracted_text: None,
                parse_error: None,
            },
        )
        .expect("stale archive metadata should seed");
        crate::scanner::scan_default_aclos_library(&connection)
            .expect("default library scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        let default_clip = clips
            .iter()
            .find(|clip| clip.video_path.ends_with("default.mp4"))
            .expect("default clip should be indexed");
        let archive_clip = clips
            .iter()
            .find(|clip| clip.video_path.ends_with("archive.mp4"))
            .expect("archive clip should be indexed");
        assert_eq!(
            default_clip.account_name.as_deref(),
            Some("FixtureDelta#0004")
        );
        assert_eq!(archive_clip.account_name, None);
    }

    #[test]
    fn scan_directory_reports_metadata_parse_warnings() {
        let fixture = TestFixture::new("metadata-warnings");
        let aclos_root = fixture.path().join("ACLOS");
        prepare_empty_metadata_root(&aclos_root);
        let root = aclos_root.join("aclos-highlight");
        let source = root.join("wonderfulVideos1001");
        let group = source.join("match-a-001");
        let logs_dir = aclos_root.join("logs");
        fs::create_dir_all(&group).expect("group should be created");
        fs::create_dir_all(&logs_dir).expect("logs dir should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");
        fs::write(
            logs_dir.join("highlight.log"),
            "first request data is {\"matchId\":",
        )
        .expect("bad log fixture should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = crate::scanner::scan_directory(&connection, &root).expect("scan should run");

        assert_eq!(summary.new_clip_count, 1);
        assert_eq!(summary.metadata_warning_count, 1);
        assert!(summary
            .errors
            .iter()
            .any(|error| error.contains("highlight.log") && error.contains("1")));
    }

    #[test]
    fn wonderful_timeline_warnings_make_the_scan_partial_and_block_freshness() {
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let openid = "90000000000000000009";
        let match_id = "warning-match";
        let video_path = "D:\\wonderfulVideos90000000000000000009\\warning-match\\clip.mp4";
        let source = db::upsert_source_dir(
            &connection,
            db::SourceDirInput {
                path: "D:\\wonderfulVideos90000000000000000009",
                name: "wonderfulVideos90000000000000000009",
            },
        )
        .expect("source should seed");
        let group = db::upsert_clip_group(
            &connection,
            db::ClipGroupInput {
                source_dir_id: source.id,
                group_key: match_id,
                display_name: match_id,
            },
        )
        .expect("group should seed");
        db::upsert_clip(
            &connection,
            db::ClipInput {
                source_dir_id: source.id,
                clip_group_id: Some(group.id),
                video_path,
                file_name: "clip.mp4",
                file_size: 1,
                modified_at: None,
                duration_ms: Some(10_000),
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            },
        )
        .expect("clip should seed");
        let metadata = super::CachedMetadataIngest {
            leveldb_result: Default::default(),
            log_result: Default::default(),
            wonderful_result: crate::wonderful_db::WonderfulDbReadResult {
                accounts: vec![crate::wonderful_db::WonderfulAccountRecord {
                    openid: openid.to_string(),
                    matches: vec![crate::wonderful_db::WonderfulMatchRecord {
                        match_id: match_id.to_string(),
                        videos: vec![crate::wonderful_db::WonderfulVideoRecord {
                            video_id: "clip".to_string(),
                            video_name: "击杀集锦".to_string(),
                            video_type: "2".to_string(),
                            highlight_type: Some(2),
                            video_src: Some(video_path.to_string()),
                            round_score: None,
                            segments: vec![crate::wonderful_db::WonderfulSegmentRecord {
                                segment_id: "segment".to_string(),
                                round_id: Some(1),
                                clip_start_ms: Some(0),
                                clip_end_ms: Some(10_000),
                                events: vec![crate::wonderful_db::WonderfulEventRecord {
                                    event_id: "outside".to_string(),
                                    event_type: "kill".to_string(),
                                    video_time_ms: Some(20_000),
                                    event_time: None,
                                    round_id: Some(1),
                                    player_name: None,
                                    agent_name: None,
                                    weapon_name: None,
                                    killer_name: None,
                                    killed_name: None,
                                    killer_is_me: true,
                                    killed_is_me: Some(false),
                                    normalization_warnings: Vec::new(),
                                    raw_json: "{}".to_string(),
                                }],
                            }],
                        }],
                        ..Default::default()
                    }],
                }],
                ..Default::default()
            },
            local_account_hint_scope: None,
            errors: Vec::new(),
        };
        let mut summary = super::ScanSummary::empty("D:/".to_string());

        super::run_metadata_ingest(&connection, &mut summary, &metadata, None);

        assert_eq!(summary.metadata_warning_count, 1);
        assert!(summary
            .errors
            .iter()
            .any(|warning| warning.contains("video-time-out-of-bounds")));
        assert_eq!(
            super::completed_status(&summary),
            super::ScanExecutionStatus::Partial,
            "metadata warnings must prevent lastScanAt from being refreshed",
        );
    }

    #[test]
    fn scan_directory_returns_empty_summary_when_root_is_missing() {
        let fixture = TestFixture::new("missing-root");
        let missing_root = fixture.path().join("does-not-exist");
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary =
            crate::scanner::scan_directory(&connection, &missing_root).expect("scan should run");

        assert_eq!(summary.source_dir_count, 0);
        assert_eq!(summary.clip_group_count, 0);
        assert_eq!(summary.new_clip_count, 0);
        assert_eq!(summary.updated_clip_count, 0);
        assert_eq!(summary.missing_clip_count, 0);
        assert_eq!(summary.cover_missing_count, 0);
        assert!(summary
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("not found"));
        assert!(summary.errors.is_empty());
    }

    #[test]
    fn scan_directory_marks_previously_indexed_missing_clip() {
        let fixture = TestFixture::new("missing-clip");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let group = source.join("11111111-1111-1111-1111-111111111111");
        let clip_path = group.join("ace.mp4");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(&clip_path, b"video-one").expect("clip should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        crate::scanner::scan_directory(&connection, root).expect("first scan should run");
        fs::remove_file(&clip_path).expect("clip should be removed from fixture");

        let summary = crate::scanner::scan_directory(&connection, root).expect("scan should run");
        let clips = db::list_clips(&connection).expect("clips should list");

        assert_eq!(summary.new_clip_count, 0);
        assert_eq!(summary.updated_clip_count, 0);
        assert_eq!(summary.missing_clip_count, 1);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].status, "missing");
    }

    #[test]
    fn latest_scan_summary_returns_most_recent_scan() {
        let fixture = TestFixture::new("summary");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let group = source.join("11111111-1111-1111-1111-111111111111");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("clip should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        crate::scanner::scan_directory(&connection, root).expect("scan should run");

        let summary = crate::scanner::latest_scan_summary(&connection)
            .expect("summary query should run")
            .expect("summary should exist");

        assert_eq!(summary.source_dir_count, 1);
        assert_eq!(summary.clip_group_count, 1);
        assert_eq!(summary.new_clip_count, 1);
        assert_eq!(summary.cover_missing_count, 1);
        assert!(summary.errors.is_empty());

        crate::scanner::ensure_scan_run_started(
            &connection,
            "job-running",
            root.to_string_lossy().as_ref(),
        )
        .expect("running scan should be recorded");
        let while_running = crate::scanner::latest_scan_summary(&connection)
            .expect("summary query should run")
            .expect("completed summary should remain available");
        assert_eq!(while_running, summary);

        crate::scanner::ensure_scan_run_terminal(
            &connection,
            "job-running",
            root.to_string_lossy().as_ref(),
            "cancelled",
            "cancelled for test",
        )
        .expect("cancelled scan should finalize");
        let after_cancel = crate::scanner::latest_scan_summary(&connection)
            .expect("summary query should run")
            .expect("completed summary should survive cancellation");
        assert_eq!(after_cancel, summary);
    }

    #[test]
    fn scan_directory_reports_progress_for_sources_groups_and_clips() {
        let fixture = TestFixture::new("progress");
        let root = fixture.path();
        let source = root.join("wonderfulVideos-main");
        let group = source.join("11111111-1111-1111-1111-111111111111");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("ace.mp4"), b"video-one").expect("first clip should be written");
        fs::write(group.join("retake.mp4"), b"video-two").expect("second clip should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let progress_events = std::cell::RefCell::new(Vec::new());

        let summary = crate::scanner::scan_directory_with_progress(&connection, root, |event| {
            progress_events.borrow_mut().push(event);
        })
        .expect("scan should run");
        let progress_events = progress_events.into_inner();

        assert_eq!(summary.new_clip_count, 2);
        assert!(progress_events
            .iter()
            .any(|event| { event.phase == "scanning" && event.total == 1 && event.current == 0 }));
        assert!(progress_events.iter().any(|event| {
            event.phase == "scanning"
                && event.source_dir_count == 1
                && event.clip_group_count == 1
                && event.clip_file_count == 2
        }));
        let phase_position = |phase: &str| {
            progress_events
                .iter()
                .position(|event| event.phase == phase)
                .unwrap_or_else(|| panic!("missing {phase} progress phase"))
        };
        assert!(phase_position("scanning") < phase_position("importing"));
        assert!(phase_position("importing") < phase_position("metadata"));
        assert!(phase_position("metadata") < phase_position("finalizing"));
        assert!(phase_position("finalizing") < phase_position("completed"));
        assert_eq!(
            progress_events.last().map(|event| event.phase.as_str()),
            Some("completed")
        );
    }

    #[test]
    fn file_level_cancellation_preserves_missing_state_and_allows_immediate_rescan() {
        let fixture = TestFixture::new("cancel-file");
        let source = fixture.path().join("wonderfulVideos-main");
        let group = source.join("match-a");
        let removed_clip = group.join("removed.mp4");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(&removed_clip, b"old-video").expect("old clip should be written");
        fs::write(group.join("survivor.mp4"), b"survivor")
            .expect("surviving clip should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        crate::scanner::scan_roots(&connection, std::slice::from_ref(&source))
            .expect("initial scan should run");
        connection
            .execute("DELETE FROM scan_runs", [])
            .expect("initial run should be cleared");
        fs::remove_file(&removed_clip).expect("old clip should be removed");
        fs::write(group.join("added.mp4"), b"added").expect("new clip should be written");

        let cancellation = AtomicBool::new(false);
        let execution = crate::scanner::scan_roots_with_progress_and_cancel(
            &connection,
            std::slice::from_ref(&source),
            "job-cancel-file",
            &cancellation,
            |event| {
                if event.clip_file_count >= 1 {
                    cancellation.store(true, Ordering::Release);
                }
            },
        )
        .expect("cancelled scan should return a result");

        assert_eq!(execution.status, super::ScanExecutionStatus::Cancelled);
        assert_eq!(execution.summary.missing_clip_count, 0);
        assert_eq!(scan_run_status(&connection, "job-cancel-file"), "cancelled");
        assert_eq!(running_scan_run_count(&connection), 0);
        assert!(db::list_clips(&connection)
            .expect("clips should list")
            .iter()
            .any(|clip| clip.file_name == "removed.mp4" && clip.status == "available"));

        let next_cancellation = AtomicBool::new(false);
        let next = crate::scanner::scan_roots_with_progress_and_cancel(
            &connection,
            std::slice::from_ref(&source),
            "job-after-cancel",
            &next_cancellation,
            |_| {},
        )
        .expect("a new scan should start immediately after cancellation");
        assert_ne!(next.status, super::ScanExecutionStatus::Cancelled);
        assert_eq!(next.summary.missing_clip_count, 1);
        assert_eq!(running_scan_run_count(&connection), 0);
        assert!(db::list_clips(&connection)
            .expect("clips should list")
            .iter()
            .any(|clip| clip.file_name == "removed.mp4" && clip.status == "missing"));
    }

    #[test]
    fn phase_boundary_cancellation_finishes_the_scan_run_as_cancelled() {
        let fixture = TestFixture::new("cancel-phase");
        let source = fixture.path().join("wonderfulVideos-main");
        let group = source.join("match-a");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("clip should be written");
        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let cancellation = AtomicBool::new(false);

        let execution = crate::scanner::scan_roots_with_progress_and_cancel(
            &connection,
            std::slice::from_ref(&source),
            "job-cancel-phase",
            &cancellation,
            |event| {
                if event.phase == "finalizing" {
                    cancellation.store(true, Ordering::Release);
                }
            },
        )
        .expect("phase cancellation should return a result");

        assert_eq!(execution.status, super::ScanExecutionStatus::Cancelled);
        assert_eq!(
            scan_run_status(&connection, "job-cancel-phase"),
            "cancelled"
        );
        assert_eq!(running_scan_run_count(&connection), 0);
        let cancelled_source = db::list_sources(&connection)
            .expect("cancelled source should list")
            .into_iter()
            .find(|candidate| {
                super::scan_path_key(Path::new(&candidate.path)) == super::scan_path_key(&source)
            })
            .expect("cancelled source should remain");
        assert_eq!(
            cancelled_source.last_scan_at, None,
            "cancellation at finalization must not set freshness",
        );
    }

    #[test]
    fn scan_discovered_roots_indexes_each_root_as_external() {
        let _env_guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let fixture = TestFixture::new("discovered-roots");
        let appdata = fixture.path().join("AppData");
        fs::create_dir_all(&appdata).expect("appdata should be created");
        let original_appdata = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", &appdata);

        let roots = [
            fixture.path().join("ArchiveA"),
            fixture.path().join("ArchiveB"),
        ];
        for (index, root) in roots.iter().enumerate() {
            let group = root
                .join(format!("wonderfulVideos{index}"))
                .join(format!("match-{index}"));
            fs::create_dir_all(&group).expect("clip group should be created");
            fs::write(group.join(format!("clip-{index}.mp4")), b"video")
                .expect("clip should be written");
        }

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");
        let summary = crate::scanner::scan_discovered_aclos_roots_with_progress(
            &connection,
            &roots,
            &roots
                .iter()
                .enumerate()
                .map(|(index, root)| root.join(format!("wonderfulVideos{index}")))
                .collect::<Vec<_>>(),
            |_| {},
        )
        .expect("discovered roots should scan");

        match original_appdata {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
        assert_eq!(summary.source_dir_count, 2);
        assert_eq!(summary.clip_group_count, 2);
        assert_eq!(db::list_clips(&connection).unwrap().len(), 2);
    }

    #[test]
    fn scan_library_roots_reports_cached_metadata_read_errors() {
        let fixture = TestFixture::new("cached-metadata-read-error");
        let aclos_root = fixture.path();
        prepare_empty_metadata_root(aclos_root);
        let scan_root = aclos_root.join("aclos-highlight");
        let group = scan_root
            .join("wonderfulVideos1001")
            .join("match-cached-error");
        let logs_dir = aclos_root.join("logs");
        fs::create_dir_all(&group).expect("clip group should be created");
        fs::create_dir_all(&logs_dir).expect("logs directory should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("clip should be written");
        fs::write(logs_dir.join("highlight.log"), [0xff, 0xfe])
            .expect("invalid log should be written");

        let connection = Connection::open_in_memory().expect("database should open");
        db::initialize_schema(&connection).expect("schema should initialize");

        let summary = super::scan_library_roots(
            &connection,
            &[scan_root],
            false,
            None,
            None,
            super::ScanRuntime::default(),
        )
        .expect("library scan should continue")
        .summary;

        assert_eq!(summary.new_clip_count, 1);
        assert_eq!(summary.metadata_warning_count, 1);
        assert!(summary
            .errors
            .iter()
            .any(|error| error.contains("Failed to read highlight log")));
    }

    fn encrypt_wonderful_db_text(openid: &str, plaintext: &str) -> String {
        let digest = format!("{:x}", Sha256::digest(openid.as_bytes()));
        let key = &digest.as_bytes()[..32];
        let iv = &digest.as_bytes()[..16];
        let ciphertext = Aes256CbcEnc::new_from_slices(key, iv)
            .expect("synthetic key material should be valid")
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext.as_bytes());
        hex::encode(ciphertext)
    }

    struct ScanBatchFixture {
        root: PathBuf,
        database_path: PathBuf,
    }

    impl ScanBatchFixture {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("vhm-scan-batch-{label}-{unique}"));
            fs::create_dir_all(&root).expect("batch fixture root should be created");
            let database_path = root.join("highlight-index.sqlite3");
            db::migrate_database(&database_path)
                .expect("batch fixture database should migrate explicitly");
            Self {
                root,
                database_path,
            }
        }

        fn path(&self) -> &Path {
            &self.root
        }

        fn open(&self) -> Connection {
            db::open_database(&self.database_path).expect("batch fixture database should open")
        }
    }

    impl Drop for ScanBatchFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn prepare_empty_metadata_root(root: &Path) {
        fs::create_dir_all(root.join("Local Storage").join("leveldb"))
            .expect("fixture LevelDB directory should be created");
        fs::create_dir_all(root.join("logs")).expect("fixture logs directory should be created");
        fs::create_dir_all(root.join("WonderfulDb"))
            .expect("fixture WonderfulDb directory should be created");
    }

    fn scan_run_count(connection: &Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
            .expect("scan run count should load")
    }

    fn scan_run_status(connection: &Connection, job_id: &str) -> String {
        connection
            .query_row(
                "SELECT status FROM scan_runs WHERE job_id = ?1",
                [job_id],
                |row| row.get(0),
            )
            .expect("scan run status should load")
    }

    fn running_scan_run_count(connection: &Connection) -> i64 {
        connection
            .query_row(
                "SELECT COUNT(*) FROM scan_runs WHERE status IN ('running', 'cancelling')",
                [],
                |row| row.get(0),
            )
            .expect("running scan run count should load")
    }

    struct TestFixture {
        root: PathBuf,
    }

    impl TestFixture {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("vhm-scanner-{label}-{unique}"));
            fs::create_dir_all(&root).expect("fixture root should be created");
            Self { root }
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn leveldb_blob(account_id: &str, json_payload: &str) -> Vec<u8> {
        let mut blob = Vec::from("noise-acloshighlight_battle_list_".as_bytes());
        blob.extend_from_slice(account_id.as_bytes());
        blob.extend_from_slice(b"\x01\x02");
        for unit in json_payload.encode_utf16() {
            blob.extend_from_slice(&unit.to_le_bytes());
        }
        blob
    }

    fn account_roles_blob(json_payload: &str) -> Vec<u8> {
        let mut blob = Vec::from("noise-ACLOS_USER_ROLES_INFO".as_bytes());
        blob.extend_from_slice(b"\x01\x02");
        blob.extend_from_slice(json_payload.as_bytes());
        blob
    }

    fn set_file_modified_time(path: &Path, modified_at: SystemTime) {
        let file = File::options()
            .write(true)
            .open(path)
            .expect("file should open for timestamp update");
        file.set_times(FileTimes::new().set_modified(modified_at))
            .expect("modified time should update");
    }
}
