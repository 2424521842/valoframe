//! High-level scan entry points and multi-root orchestration.

use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

use rusqlite::Connection;

use super::{
    default_aclos_app_data_dir, discover_library_sources, is_source_directory_name,
    normalize_unique_scan_paths, push_unique_scan_root, read_sorted_entries, run_scan_batch,
    scan_path_key, scan_roots_from_videocut_log, MetadataScanConfig, ScanBatchInput, ScanExecution,
    ScanProgress, ScanProgressReporter, ScanRuntime, ScanSummary,
};
use crate::db::DbResult;

const VIDEOCUT_LOG_NAME: &str = "videocut.txt";

pub fn default_aclos_dir() -> PathBuf {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Default"))
        .join("AppData")
        .join("ACLOS")
        .join("aclos-highlight")
}

pub fn scan_directory(connection: &Connection, root: impl AsRef<Path>) -> DbResult<ScanSummary> {
    scan_library_roots(
        connection,
        &[root.as_ref().to_path_buf()],
        false,
        None,
        None,
        ScanRuntime::default(),
    )
    .map(|execution| execution.summary)
}

pub fn scan_directory_with_progress<F>(
    connection: &Connection,
    root: impl AsRef<Path>,
    progress: F,
) -> DbResult<ScanSummary>
where
    F: Fn(ScanProgress),
{
    scan_library_roots(
        connection,
        &[root.as_ref().to_path_buf()],
        false,
        None,
        Some(&progress),
        ScanRuntime::default(),
    )
    .map(|execution| execution.summary)
}

pub fn scan_default_aclos_library(connection: &Connection) -> DbResult<ScanSummary> {
    let roots = default_aclos_library_roots();
    scan_library_roots(
        connection,
        &roots,
        false,
        None,
        None,
        ScanRuntime::default(),
    )
    .map(|execution| execution.summary)
}

pub fn scan_default_aclos_library_with_progress<F>(
    connection: &Connection,
    progress: F,
) -> DbResult<ScanSummary>
where
    F: Fn(ScanProgress),
{
    let roots = default_aclos_library_roots();
    scan_library_roots(
        connection,
        &roots,
        false,
        None,
        Some(&progress),
        ScanRuntime::default(),
    )
    .map(|execution| execution.summary)
}

pub fn scan_default_aclos_library_with_progress_and_cancel<F>(
    connection: &Connection,
    job_id: &str,
    cancellation: &AtomicBool,
    progress: F,
) -> DbResult<ScanExecution>
where
    F: Fn(ScanProgress),
{
    let roots = default_aclos_library_roots();
    scan_library_roots(
        connection,
        &roots,
        false,
        None,
        Some(&progress),
        ScanRuntime {
            job_id: Some(job_id),
            cancellation: Some(cancellation),
        },
    )
}

pub fn scan_discovered_aclos_roots_with_progress<F>(
    connection: &Connection,
    roots: &[PathBuf],
    validated_source_dirs: &[PathBuf],
    progress: F,
) -> DbResult<ScanSummary>
where
    F: Fn(ScanProgress),
{
    scan_discovered_aclos_roots_with_progress_inner(
        connection,
        roots,
        validated_source_dirs,
        Some(&progress),
        ScanRuntime::default(),
    )
    .map(|execution| execution.summary)
}

pub fn scan_discovered_aclos_roots_with_progress_and_cancel<F>(
    connection: &Connection,
    roots: &[PathBuf],
    validated_source_dirs: &[PathBuf],
    job_id: &str,
    cancellation: &AtomicBool,
    progress: F,
) -> DbResult<ScanExecution>
where
    F: Fn(ScanProgress),
{
    scan_discovered_aclos_roots_with_progress_inner(
        connection,
        roots,
        validated_source_dirs,
        Some(&progress),
        ScanRuntime {
            job_id: Some(job_id),
            cancellation: Some(cancellation),
        },
    )
}

fn scan_discovered_aclos_roots_with_progress_inner(
    connection: &Connection,
    roots: &[PathBuf],
    validated_source_dirs: &[PathBuf],
    progress: Option<ScanProgressReporter<'_>>,
    runtime: ScanRuntime<'_>,
) -> DbResult<ScanExecution> {
    if validated_source_dirs.is_empty() {
        let roots = normalize_unique_scan_paths(roots.iter().map(PathBuf::as_path));
        return run_scan_batch(
            connection,
            ScanBatchInput {
                requested_roots: roots,
                source_paths: Vec::new(),
                metadata_config: MetadataScanConfig {
                    anchor: None,
                    allow_external_fallback: true,
                    account_hint_scope: None,
                    use_local_account_hint_scope: false,
                },
                initial_errors: Vec::new(),
                empty_message: Some("未发现标准无畏时刻素材".to_string()),
            },
            progress,
            runtime,
        );
    }

    let source_path_filter = validated_source_dirs
        .iter()
        .map(|path| scan_path_key(path))
        .collect::<HashSet<_>>();
    scan_library_roots(
        connection,
        roots,
        true,
        Some(&source_path_filter),
        progress,
        runtime,
    )
}

pub fn scan_custom_directory(
    connection: &Connection,
    root: impl AsRef<Path>,
) -> DbResult<ScanSummary> {
    scan_custom_directory_inner(connection, root.as_ref(), None, ScanRuntime::default())
        .map(|execution| execution.summary)
}

pub fn scan_custom_directory_with_progress<F>(
    connection: &Connection,
    root: impl AsRef<Path>,
    progress: F,
) -> DbResult<ScanSummary>
where
    F: Fn(ScanProgress),
{
    scan_custom_directory_inner(
        connection,
        root.as_ref(),
        Some(&progress),
        ScanRuntime::default(),
    )
    .map(|execution| execution.summary)
}

pub fn scan_custom_directory_with_progress_and_cancel<F>(
    connection: &Connection,
    root: impl AsRef<Path>,
    job_id: &str,
    cancellation: &AtomicBool,
    progress: F,
) -> DbResult<ScanExecution>
where
    F: Fn(ScanProgress),
{
    scan_custom_directory_inner(
        connection,
        root.as_ref(),
        Some(&progress),
        ScanRuntime {
            job_id: Some(job_id),
            cancellation: Some(cancellation),
        },
    )
}

/// Scans concrete source directories (`source_dirs.path`) as one logical batch.
pub fn scan_roots(connection: &Connection, roots: &[PathBuf]) -> DbResult<ScanSummary> {
    scan_roots_inner(connection, roots, None, ScanRuntime::default())
        .map(|execution| execution.summary)
}

pub fn scan_roots_with_progress<F>(
    connection: &Connection,
    roots: &[PathBuf],
    progress: F,
) -> DbResult<ScanSummary>
where
    F: Fn(ScanProgress),
{
    scan_roots_inner(connection, roots, Some(&progress), ScanRuntime::default())
        .map(|execution| execution.summary)
}

pub fn scan_roots_with_progress_and_cancel<F>(
    connection: &Connection,
    roots: &[PathBuf],
    job_id: &str,
    cancellation: &AtomicBool,
    progress: F,
) -> DbResult<ScanExecution>
where
    F: Fn(ScanProgress),
{
    scan_roots_inner(
        connection,
        roots,
        Some(&progress),
        ScanRuntime {
            job_id: Some(job_id),
            cancellation: Some(cancellation),
        },
    )
}

fn scan_roots_inner(
    connection: &Connection,
    roots: &[PathBuf],
    progress: Option<ScanProgressReporter<'_>>,
    runtime: ScanRuntime<'_>,
) -> DbResult<ScanExecution> {
    let roots = normalize_unique_scan_paths(roots.iter().map(PathBuf::as_path));
    let source_paths = resolve_explicit_source_paths(&roots);
    run_scan_batch(
        connection,
        ScanBatchInput {
            requested_roots: roots.clone(),
            source_paths,
            metadata_config: MetadataScanConfig {
                anchor: None,
                allow_external_fallback: true,
                account_hint_scope: None,
                use_local_account_hint_scope: true,
            },
            initial_errors: Vec::new(),
            empty_message: Some("No source directories provided".to_string()),
        },
        progress,
        runtime,
    )
}

fn resolve_explicit_source_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut source_paths = Vec::new();
    for root in roots {
        if is_source_directory_name(root) {
            push_unique_scan_root(&mut source_paths, root.clone());
            continue;
        }

        let discovered_children = read_sorted_entries(root)
            .ok()
            .into_iter()
            .flatten()
            .filter(|path| path.is_dir() && is_source_directory_name(path))
            .collect::<Vec<_>>();
        if discovered_children.is_empty() {
            push_unique_scan_root(&mut source_paths, root.clone());
        } else {
            for source_path in discovered_children {
                push_unique_scan_root(&mut source_paths, source_path);
            }
        }
    }
    source_paths
}

fn scan_custom_directory_inner(
    connection: &Connection,
    root: &Path,
    progress: Option<ScanProgressReporter<'_>>,
    runtime: ScanRuntime<'_>,
) -> DbResult<ScanExecution> {
    let roots = normalize_unique_scan_paths(std::iter::once(root));
    let discovery = discover_library_sources(&roots, None);
    if !discovery.sources.is_empty() {
        return run_scan_batch(
            connection,
            ScanBatchInput {
                requested_roots: discovery.roots.clone(),
                source_paths: discovery.sources,
                metadata_config: MetadataScanConfig {
                    anchor: discovery.roots.first().cloned(),
                    allow_external_fallback: true,
                    account_hint_scope: None,
                    use_local_account_hint_scope: true,
                },
                initial_errors: discovery.errors,
                empty_message: discovery.empty_message,
            },
            progress,
            runtime,
        );
    }

    run_scan_batch(
        connection,
        ScanBatchInput {
            requested_roots: roots.clone(),
            source_paths: roots,
            metadata_config: MetadataScanConfig {
                anchor: None,
                allow_external_fallback: true,
                account_hint_scope: None,
                use_local_account_hint_scope: true,
            },
            initial_errors: Vec::new(),
            empty_message: None,
        },
        progress,
        runtime,
    )
}

pub(super) fn scan_library_roots(
    connection: &Connection,
    roots: &[PathBuf],
    all_roots_external: bool,
    source_path_filter: Option<&HashSet<String>>,
    progress: Option<ScanProgressReporter<'_>>,
    runtime: ScanRuntime<'_>,
) -> DbResult<ScanExecution> {
    let discovery = discover_library_sources(roots, source_path_filter);
    let metadata_anchor = discovery.roots.first().cloned();
    let account_hint_scope = if all_roots_external {
        None
    } else {
        metadata_anchor.clone()
    };

    run_scan_batch(
        connection,
        ScanBatchInput {
            requested_roots: discovery.roots,
            source_paths: discovery.sources,
            metadata_config: MetadataScanConfig {
                anchor: metadata_anchor,
                allow_external_fallback: all_roots_external,
                account_hint_scope,
                use_local_account_hint_scope: false,
            },
            initial_errors: discovery.errors,
            empty_message: discovery.empty_message,
        },
        progress,
        runtime,
    )
}

fn default_aclos_library_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    push_unique_scan_root(&mut roots, default_aclos_dir());

    let videocut_log_path = default_aclos_app_data_dir()
        .join("logs")
        .join(VIDEOCUT_LOG_NAME);
    if let Ok(discovered_roots) = scan_roots_from_videocut_log(&videocut_log_path) {
        for root in discovered_roots {
            if root.is_dir() {
                push_unique_scan_root(&mut roots, root);
            }
        }
    }

    roots
}
