//! Read-only recursive MP4 adapter used by NVIDIA, Tracker and generic folders.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use rusqlite::Connection;

use super::reconnect::{
    canonicalize_non_reparse_directory_chain, collect_scan_candidate,
    concrete_file_identity_changed, metadata_is_reparse_point, revalidate_staged_candidate,
    IdentityReadDiagnostics, ScanReconnectPlanGuard,
};
use super::{
    path_to_string, push_source_error, ScanProgressState, ScanRuntime, ScanSummary,
    SourceScanOutcome, SourceScanStep,
};
use crate::db::{self, ClipInput, ClipSaveOutcome, DbResult, SourceDir, SourceKind};

const DATABASE_BATCH_SIZE: usize = 128;
const MAX_RECURSIVE_DEPTH: usize = 64;
const MAX_RECURSIVE_MP4_FILES: usize = 1_000_000;
const MAX_SOURCE_ERROR_SAMPLES: usize = 16;
const MAX_SOURCE_ERROR_TEXT_BYTES: usize = 4 * 1024;

pub(super) fn scan_recursive_source(
    connection: &Connection,
    source: SourceDir,
    summary: &mut ScanSummary,
    progress: &mut ScanProgressState<'_>,
    runtime: ScanRuntime<'_>,
) -> DbResult<SourceScanStep> {
    let source_path = PathBuf::from(&source.path);
    let scan_root = PathBuf::from(&source.scan_root_path);
    progress.source_started(&source.name, &scan_root);
    summary.source_dir_count += 1;

    if runtime.is_cancelled() {
        return Ok(cancelled_step(source, HashSet::new(), progress));
    }

    let canonical_root = match canonicalize_non_reparse_directory_chain(&scan_root) {
        Ok(path) => path,
        Err(error) => {
            return unavailable_source(
                connection,
                source,
                source_path,
                format!(
                    "Recursive MP4 root failed its non-reparse path-chain check {}: {error}",
                    scan_root.display()
                ),
                summary,
                progress,
            )
        }
    };

    let _reconnect_plan_cleanup = ScanReconnectPlanGuard::begin(connection, source.id)?;
    let mut seen_paths = HashSet::new();
    let mut seen_pending_paths = HashSet::new();
    let mut seen_directories = HashSet::new();
    let mut visited_directories = HashSet::new();
    let mut stack = vec![(canonical_root.clone(), 0usize)];
    let mut source_errors = Vec::new();
    let mut identity_diagnostics = IdentityReadDiagnostics::default();
    let mut complete_for_missing = true;
    let mut candidate_count = 0usize;

    'directories: while let Some((directory, depth)) = stack.pop() {
        if runtime.is_cancelled() {
            return Ok(cancelled_step(source, seen_paths, progress));
        }
        if depth > MAX_RECURSIVE_DEPTH {
            record_source_warning(
                summary,
                &mut source_errors,
                &mut complete_for_missing,
                format!(
                    "Skipped directory deeper than {MAX_RECURSIVE_DEPTH} levels: {}",
                    directory.display()
                ),
            );
            continue;
        }
        let directory_key = db::normalize_path(&path_to_string(&directory));
        if !visited_directories.insert(directory_key) {
            continue;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                record_source_warning(
                    summary,
                    &mut source_errors,
                    &mut complete_for_missing,
                    format!("Failed to read directory {}: {error}", directory.display()),
                );
                continue;
            }
        };

        for entry in entries {
            if runtime.is_cancelled() {
                return Ok(cancelled_step(source, seen_paths, progress));
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    record_source_warning(
                        summary,
                        &mut source_errors,
                        &mut complete_for_missing,
                        format!(
                            "Failed to enumerate an entry in {}: {error}",
                            directory.display()
                        ),
                    );
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    record_source_warning(
                        summary,
                        &mut source_errors,
                        &mut complete_for_missing,
                        format!("Failed to inspect {}: {error}", path.display()),
                    );
                    continue;
                }
            };
            if metadata_is_reparse_point(&metadata) {
                record_source_warning(
                    summary,
                    &mut source_errors,
                    &mut complete_for_missing,
                    format!("Skipped symbolic link or reparse point: {}", path.display()),
                );
                continue;
            }

            if metadata.is_dir() {
                match canonical_path_within_root(&path, &canonical_root) {
                    Ok(canonical) => stack.push((canonical, depth.saturating_add(1))),
                    Err(error) => record_source_warning(
                        summary,
                        &mut source_errors,
                        &mut complete_for_missing,
                        error,
                    ),
                }
                continue;
            }
            if !metadata.is_file() || !has_mp4_extension(&path) {
                continue;
            }

            candidate_count = candidate_count.saturating_add(1);
            if candidate_count > MAX_RECURSIVE_MP4_FILES {
                record_source_warning(
                    summary,
                    &mut source_errors,
                    &mut complete_for_missing,
                    format!(
                        "Stopped after {MAX_RECURSIVE_MP4_FILES} MP4 candidates under {}",
                        canonical_root.display()
                    ),
                );
                break 'directories;
            }

            let canonical_path = match canonical_path_within_root(&path, &canonical_root) {
                Ok(path) => path,
                Err(error) => {
                    record_source_warning(
                        summary,
                        &mut source_errors,
                        &mut complete_for_missing,
                        error,
                    );
                    continue;
                }
            };
            if let Err(error) = ensure_non_reparse_chain(&canonical_path, &canonical_root) {
                record_source_warning(
                    summary,
                    &mut source_errors,
                    &mut complete_for_missing,
                    error,
                );
                continue;
            }
            let candidate = match collect_scan_candidate(canonical_path, &mut identity_diagnostics)
            {
                Ok(candidate) => candidate,
                Err(error) => {
                    record_source_warning(
                        summary,
                        &mut source_errors,
                        &mut complete_for_missing,
                        error,
                    );
                    continue;
                }
            };
            seen_paths.insert(candidate.normalized_path.clone());
            let db::StageScanReconnectCandidateOutcome::Staged(_) =
                db::stage_scan_reconnect_candidate(connection, candidate.stage_input(source.id))?;
        }
    }

    if runtime.is_cancelled() {
        db::clear_scan_reconnect_plan(connection)?;
        return Ok(cancelled_step(source, seen_paths, progress));
    }
    db::finalize_scan_reconnect_plan(connection, source.id)?;
    let completed = process_staged_candidates(
        connection,
        &source,
        &mut seen_directories,
        &mut seen_pending_paths,
        summary,
        progress,
        runtime,
        &mut source_errors,
        &mut complete_for_missing,
        &mut identity_diagnostics,
    )?;
    db::clear_scan_reconnect_plan(connection)?;
    if !completed {
        return Ok(cancelled_step(source, seen_paths, progress));
    }

    // Only a fully enumerable scan may drop pending rows whose files disappeared; the same
    // policy protects historical clip state from offline/partial enumerations.
    if complete_for_missing {
        db::delete_missing_pending_manual_clips(connection, source.id, &seen_pending_paths)?;
    }

    if source_errors.is_empty() {
        db::mark_source_dir_scan_succeeded(connection, source.id)?;
    } else {
        db::mark_source_dir_scan_error(
            connection,
            source.id,
            "partial",
            &bounded_source_error_text(&source_errors),
        )?;
    }
    progress.source_finished(&source.name);
    Ok(SourceScanStep::finished(SourceScanOutcome {
        source_path,
        source_id: Some(source.id),
        accessible: true,
        metadata_eligible: false,
        complete_for_missing,
        seen_paths,
    }))
}

#[allow(clippy::too_many_arguments)]
fn process_staged_candidates(
    connection: &Connection,
    source: &SourceDir,
    seen_directories: &mut HashSet<String>,
    seen_pending_paths: &mut HashSet<String>,
    summary: &mut ScanSummary,
    progress: &mut ScanProgressState<'_>,
    runtime: ScanRuntime<'_>,
    source_errors: &mut Vec<String>,
    complete_for_missing: &mut bool,
    identity_diagnostics: &mut IdentityReadDiagnostics,
) -> DbResult<bool> {
    let mut after_candidate_id = 0;
    loop {
        if runtime.is_cancelled() {
            return Ok(false);
        }
        let staged = db::list_staged_scan_reconnect_candidates(
            connection,
            source.id,
            after_candidate_id,
            DATABASE_BATCH_SIZE as i64,
        )?;
        if staged.is_empty() {
            return Ok(true);
        }
        after_candidate_id = staged
            .last()
            .map_or(after_candidate_id, |item| item.candidate_id);
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("Database starting recursive MP4 batch failed: {error}"))?;
        let mut saved_clips = Vec::with_capacity(staged.len());
        let mut pending_progress: Vec<(PathBuf, String, String)> = Vec::new();
        for staged_candidate in staged {
            if runtime.is_cancelled() {
                return Ok(false);
            }
            let current = match revalidate_staged_candidate(&staged_candidate, identity_diagnostics)
            {
                Ok(candidate) => candidate,
                Err(error) => {
                    record_source_warning(summary, source_errors, complete_for_missing, error);
                    continue;
                }
            };
            if concrete_file_identity_changed(staged_candidate.file_identity, current.file_identity)
            {
                record_source_warning(
                    summary,
                    source_errors,
                    complete_for_missing,
                    format!(
                        "Deferred MP4 whose stable identity changed after planning: {}",
                        current.path.display()
                    ),
                );
                continue;
            }
            let decision = match db::resolve_scan_reconnect_candidate(
                &transaction,
                source.id,
                staged_candidate.candidate_id,
            ) {
                Ok(decision) => decision,
                Err(error) => {
                    record_source_warning(
                        summary,
                        source_errors,
                        complete_for_missing,
                        format!(
                            "Failed to resolve recursive MP4 reconnect plan {}: {error}",
                            current.path.display()
                        ),
                    );
                    continue;
                }
            };

            let mut reconnect = None;
            let mut skip_candidate = false;
            let mut queue_pending = false;
            match decision {
                db::ScanReconnectDecision::ExistingPath { .. } => {}
                db::ScanReconnectDecision::New(_) => {
                    queue_pending = source.source_kind == SourceKind::Nvidia;
                }
                db::ScanReconnectDecision::NewWithWarning { warning, .. } => {
                    skip_candidate = matches!(
                        warning.kind,
                        db::ScanReconnectWarningKind::ForeignPathOwner
                            | db::ScanReconnectWarningKind::NormalizedPathConflict
                    );
                    record_reconnect_warning(summary, source_errors, complete_for_missing, warning);
                    queue_pending = !skip_candidate && source.source_kind == SourceKind::Nvidia;
                }
                db::ScanReconnectDecision::Reconnect(planned) => {
                    if current.file_identity != planned.candidate.file_identity {
                        // Identity availability is explicitly best-effort. Never use a reconnect
                        // plan built with a different amount of identity evidence, but continue
                        // ordinary indexing without turning the source partial.
                    } else {
                        reconnect = Some(planned);
                    }
                }
            }
            if skip_candidate {
                if source.source_kind == SourceKind::Nvidia
                    && db::find_pending_manual_clip_source_id_by_normalized_path(
                        &transaction,
                        &current.normalized_path,
                    )? == Some(source.id)
                {
                    // A pre-existing pending row remains visible even if a legacy indexed clip
                    // now conflicts with it. Complete scans must not silently clean it up.
                    seen_pending_paths.insert(current.normalized_path.clone());
                }
                continue;
            }

            // NVIDIA recordings have no reliable metadata: new discoveries are never auto-imported.
            // They enter the pending manual-classification queue instead, while clips that were
            // indexed before this policy still reconnect/update normally.
            if queue_pending {
                let relative_directory = relative_directory_for(source, &current.path);
                if db::upsert_pending_manual_clip(
                    &transaction,
                    db::PendingManualClipInput {
                        source_dir_id: source.id,
                        video_path: &current.file_path,
                        file_name: &current.file_name,
                        file_size: current.size_bytes,
                        modified_at: Some(&current.modified_at),
                        source_relative_dir: &relative_directory,
                    },
                )? {
                    summary.pending_clip_count += 1;
                }
                seen_pending_paths.insert(current.normalized_path.clone());
                pending_progress.push((current.path, relative_directory, current.file_name));
                continue;
            }

            let input = ClipInput {
                source_dir_id: source.id,
                clip_group_id: None,
                video_path: &current.file_path,
                file_name: &current.file_name,
                file_size: current.size_bytes,
                modified_at: Some(&current.modified_at),
                duration_ms: None,
                recorded_at: None,
                cover_path: None,
                cover_source: "missing",
            };
            let saved = if let Some(planned) = reconnect {
                match db::apply_planned_scan_reconnect(
                    &transaction,
                    &planned,
                    input,
                    current.file_identity,
                ) {
                    Ok(db::ApplyScanReconnectOutcome::Reconnected(saved)) => Some(*saved),
                    Ok(db::ApplyScanReconnectOutcome::OldPathPresent) => {
                        save_candidate_normally(&transaction, source, input, current.file_identity)?
                    }
                    Ok(db::ApplyScanReconnectOutcome::OldPathUnverifiable(error)) => {
                        record_source_warning(
                            summary,
                            source_errors,
                            complete_for_missing,
                            format!(
                                "Old clip path could not be verified before reconnecting {}: {error}",
                                current.path.display()
                            ),
                        );
                        save_candidate_normally(&transaction, source, input, current.file_identity)?
                    }
                    Ok(db::ApplyScanReconnectOutcome::StalePlan) => {
                        record_source_warning(
                            summary,
                            source_errors,
                            complete_for_missing,
                            format!(
                                "Reconnect plan became stale before indexing {}",
                                current.path.display()
                            ),
                        );
                        save_candidate_normally(&transaction, source, input, current.file_identity)?
                    }
                    Err(error) => {
                        record_source_warning(
                            summary,
                            source_errors,
                            complete_for_missing,
                            format!(
                                "Failed to reconnect recursive MP4 {}: {error}",
                                current.path.display()
                            ),
                        );
                        continue;
                    }
                }
            } else {
                save_candidate_normally(&transaction, source, input, current.file_identity)?
            };
            if let Some(saved) = saved {
                saved_clips.push((saved.outcome, current.path, current.file_name));
            }
        }
        if runtime.is_cancelled() {
            return Ok(false);
        }
        transaction
            .commit()
            .map_err(|error| format!("Database committing recursive MP4 batch failed: {error}"))?;

        for (outcome, path, file_name) in saved_clips {
            match outcome {
                ClipSaveOutcome::Inserted => summary.new_clip_count += 1,
                ClipSaveOutcome::Updated => summary.updated_clip_count += 1,
                ClipSaveOutcome::Unchanged => {}
            }
            summary.cover_missing_count += 1;
            let relative_directory = relative_directory_for(source, &path);
            if seen_directories.insert(relative_directory.clone()) {
                progress.group_scanned(if relative_directory.is_empty() {
                    "根目录"
                } else {
                    &relative_directory
                });
            }
            progress.clip_scanned(&file_name);
        }
        for (_, relative_directory, file_name) in pending_progress {
            if seen_directories.insert(relative_directory.clone()) {
                progress.group_scanned(if relative_directory.is_empty() {
                    "根目录"
                } else {
                    &relative_directory
                });
            }
            progress.clip_scanned(&file_name);
        }
    }
}

fn relative_directory_for(source: &SourceDir, path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.strip_prefix(Path::new(&source.scan_root_path)).ok())
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

fn save_candidate_normally(
    connection: &Connection,
    source: &SourceDir,
    input: ClipInput<'_>,
    file_identity: Option<db::StableFileIdentity>,
) -> DbResult<Option<db::SavedClip>> {
    let normalized_path = db::normalize_path(input.video_path);
    if db::find_pending_manual_clip_source_id_by_normalized_path(connection, &normalized_path)?
        .is_some()
    {
        return Ok(None);
    }
    if let Some(owner_id) =
        db::find_clip_source_id_by_normalized_path(connection, &normalized_path)?
    {
        if owner_id != source.id {
            return Ok(None);
        }
    }
    db::upsert_scanned_clip_with_file_identity(connection, input, file_identity).map(Some)
}

fn record_reconnect_warning(
    summary: &mut ScanSummary,
    source_errors: &mut Vec<String>,
    complete_for_missing: &mut bool,
    warning: db::ScanReconnectWarning,
) {
    if warning.kind.blocks_missing_reconciliation() {
        *complete_for_missing = false;
    }
    summary.push_error(warning.message.clone());
    if source_errors.len() < MAX_SOURCE_ERROR_SAMPLES && !source_errors.contains(&warning.message) {
        source_errors.push(warning.message);
    }
}

fn unavailable_source(
    connection: &Connection,
    source: SourceDir,
    source_path: PathBuf,
    error: String,
    summary: &mut ScanSummary,
    progress: &mut ScanProgressState<'_>,
) -> DbResult<SourceScanStep> {
    push_source_error(summary, &source_path, &error);
    db::mark_source_dir_scan_error(connection, source.id, "unavailable", &error)?;
    progress.source_finished(&source.name);
    Ok(SourceScanStep::finished(SourceScanOutcome {
        source_path,
        source_id: Some(source.id),
        accessible: false,
        metadata_eligible: false,
        complete_for_missing: false,
        seen_paths: HashSet::new(),
    }))
}

fn cancelled_step(
    source: SourceDir,
    seen_paths: HashSet<String>,
    progress: &mut ScanProgressState<'_>,
) -> SourceScanStep {
    progress.source_interrupted(&source.name);
    SourceScanStep::cancelled(SourceScanOutcome {
        source_path: PathBuf::from(source.path),
        source_id: Some(source.id),
        accessible: true,
        metadata_eligible: false,
        complete_for_missing: false,
        seen_paths,
    })
}

fn canonical_path_within_root(path: &Path, canonical_root: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Failed to canonicalize {}: {error}", path.display()))?;
    if canonical == canonical_root || !canonical.starts_with(canonical_root) {
        return Err(format!(
            "Skipped path outside the authorized source root: {}",
            path.display()
        ));
    }
    Ok(canonical)
}

fn ensure_non_reparse_chain(path: &Path, canonical_root: &Path) -> Result<(), String> {
    let mut cursor = Some(path);
    while let Some(current) = cursor {
        let metadata = fs::symlink_metadata(current).map_err(|error| {
            format!(
                "Failed to validate path chain {}: {error}",
                current.display()
            )
        })?;
        if metadata_is_reparse_point(&metadata) {
            return Err(format!(
                "Skipped path whose chain contains a symbolic link or reparse point: {}",
                current.display()
            ));
        }
        if current == canonical_root {
            return Ok(());
        }
        cursor = current.parent();
    }
    Err(format!(
        "Skipped path whose parent chain does not reach the authorized root: {}",
        path.display()
    ))
}

fn record_source_warning(
    summary: &mut ScanSummary,
    source_errors: &mut Vec<String>,
    complete_for_missing: &mut bool,
    error: String,
) {
    *complete_for_missing = false;
    summary.push_error(error.clone());
    if source_errors.len() < MAX_SOURCE_ERROR_SAMPLES && !source_errors.contains(&error) {
        source_errors.push(error);
    }
}

fn bounded_source_error_text(errors: &[String]) -> String {
    let mut text = errors.join(" | ");
    if text.len() <= MAX_SOURCE_ERROR_TEXT_BYTES {
        return text;
    }
    let mut end = MAX_SOURCE_ERROR_TEXT_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push('…');
    text
}

fn has_mp4_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::SystemTime;

    fn temp_fixture(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let fixture = std::env::temp_dir().join(format!(
            "vhm-recursive-mp4-{label}-{}-{unique}",
            std::process::id()
        ));
        let data = fixture.join("data");
        let root = fixture.join("recordings");
        fs::create_dir_all(&data).expect("data directory should be created");
        fs::create_dir_all(&root).expect("recording root should be created");
        (fixture, data, root)
    }

    fn register_generic_source(database_path: &Path, root: &Path) -> SourceDir {
        let connection = db::open_database(database_path).expect("database should open");
        let canonical_root = root
            .canonicalize()
            .expect("recording root should canonicalize")
            .display()
            .to_string();
        db::register_source_dir(
            &connection,
            db::SourceDirInput {
                path: &canonical_root,
                name: "Generic recordings",
            },
            db::SourceProfileInput {
                source_kind: db::SourceKind::Generic,
                scan_mode: db::ScanMode::RecursiveMp4,
                scan_root_path: &canonical_root,
            },
            true,
        )
        .expect("source should register")
    }

    fn register_nvidia_source(database_path: &Path, root: &Path) -> SourceDir {
        let connection = db::open_database(database_path).expect("database should open");
        let canonical_root = root
            .canonicalize()
            .expect("recording root should canonicalize")
            .display()
            .to_string();
        db::register_source_dir(
            &connection,
            db::SourceDirInput {
                path: &canonical_root,
                name: "NVIDIA recordings",
            },
            db::SourceProfileInput {
                source_kind: db::SourceKind::Nvidia,
                scan_mode: db::ScanMode::RecursiveMp4,
                scan_root_path: &canonical_root,
            },
            true,
        )
        .expect("source should register")
    }
    fn directory_inventory(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut inventory = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(directory) = stack.pop() {
            for entry in fs::read_dir(&directory)
                .expect("fixture directory should enumerate")
                .map(|entry| entry.expect("fixture entry should load"))
            {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let mut bytes = Vec::new();
                    fs::File::open(&path)
                        .expect("fixture file should open")
                        .read_to_end(&mut bytes)
                        .expect("fixture file should read");
                    inventory.push((
                        path.strip_prefix(root)
                            .expect("fixture path should be relative")
                            .to_string_lossy()
                            .replace('\\', "/"),
                        bytes,
                    ));
                }
            }
        }
        inventory.sort_by(|left, right| left.0.cmp(&right.0));
        inventory
    }

    #[test]
    fn extension_matching_is_mp4_only_and_case_insensitive() {
        assert!(has_mp4_extension(Path::new("clip.mp4")));
        assert!(has_mp4_extension(Path::new("clip.MP4")));
        assert!(!has_mp4_extension(Path::new("clip.mov")));
        assert!(!has_mp4_extension(Path::new("clip.mp4.part")));
    }

    #[test]
    fn canonical_boundary_rejects_a_regular_mp4_outside_the_authorized_root() {
        let (fixture, _data, root) = temp_fixture("outside-boundary");
        let outside = fixture.join("outside.mp4");
        fs::write(&outside, b"outside").expect("outside fixture should be written");
        let canonical_root = root.canonicalize().expect("root should canonicalize");

        let error = canonical_path_within_root(&outside, &canonical_root)
            .expect_err("outside path must be rejected");
        assert!(error.contains("outside the authorized source root"));
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn changed_file_is_deferred_before_the_database_batch_commits() {
        let (fixture, data, root) = temp_fixture("unstable");
        let video_path = root.join("recording.mp4");
        fs::write(&video_path, b"initial").expect("initial fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");
        let mut identity_diagnostics = IdentityReadDiagnostics::default();
        let candidate = collect_scan_candidate(video_path.clone(), &mut identity_diagnostics)
            .expect("candidate should collect");
        db::begin_scan_reconnect_plan(&connection, source.id).unwrap();
        db::stage_scan_reconnect_candidate(&connection, candidate.stage_input(source.id)).unwrap();
        db::finalize_scan_reconnect_plan(&connection, source.id).unwrap();
        fs::write(&video_path, b"recording grew while the scan was active")
            .expect("fixture should change");

        let mut seen_directories = HashSet::new();
        let mut seen_pending_paths = HashSet::new();
        let mut summary = ScanSummary::empty(path_to_string(&root));
        let mut progress = ScanProgressState::new(path_to_string(&root), None);
        let mut source_errors = Vec::new();
        let mut complete_for_missing = true;
        assert!(process_staged_candidates(
            &connection,
            &source,
            &mut seen_directories,
            &mut seen_pending_paths,
            &mut summary,
            &mut progress,
            ScanRuntime::default(),
            &mut source_errors,
            &mut complete_for_missing,
            &mut identity_diagnostics,
        )
        .expect("unstable batch should finish safely"));

        assert!(db::list_clips(&connection).unwrap().is_empty());
        assert!(!complete_for_missing);
        assert!(source_errors
            .iter()
            .any(|error| error.contains("changed after source-wide reconnect planning")));
        db::clear_scan_reconnect_plan(&connection).unwrap();
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn recursive_source_indexes_root_and_nested_mp4s_incrementally_without_writes() {
        let (fixture, data, root) = temp_fixture("incremental");
        let nested = root.join("Valorant").join("2026-08-08");
        fs::create_dir_all(&nested).expect("nested directory should be created");
        fs::write(root.join("root.MP4"), b"root video").expect("root MP4 should be written");
        fs::write(nested.join("nested.mp4"), b"nested video")
            .expect("nested MP4 should be written");
        fs::write(nested.join("ignored.mov"), b"ignored").expect("ignored file should be written");
        let inventory_before = directory_inventory(&root);
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);

        let connection = db::open_database(&database_path).expect("database should open");
        let first = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("first recursive synchronization should complete");
        assert_eq!(first.new_clip_count, 2);
        assert!(first.errors.is_empty());
        let clips = db::list_clips(&connection).expect("clips should list");
        assert_eq!(clips.len(), 2);
        assert!(clips.iter().any(|clip| clip.source_relative_dir.is_empty()));
        assert!(clips
            .iter()
            .any(|clip| clip.source_relative_dir == "Valorant/2026-08-08"));
        assert!(clips
            .iter()
            .all(|clip| clip.source_kind == db::SourceKind::Generic));
        assert_eq!(directory_inventory(&root), inventory_before);

        fs::write(nested.join("later.mp4"), b"later video")
            .expect("incremental MP4 should be written");
        let second = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("incremental synchronization should complete");
        assert_eq!(second.new_clip_count, 1);
        assert_eq!(db::list_clips(&connection).unwrap().len(), 3);
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn renamed_subdirectory_reconnects_in_place_and_invalidates_old_thumbnail_claims() {
        let (fixture, data, root) = temp_fixture("renamed-subdirectory");
        let old_dir = root.join("old-folder");
        let new_dir = root.join("new-folder");
        fs::create_dir_all(&old_dir).expect("old directory should be created");
        fs::write(old_dir.join("kept.mp4"), b"same-file").expect("fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        let first = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("initial synchronization should complete");
        assert_eq!(first.new_clip_count, 1);
        let initial = db::list_clips(&connection).unwrap().pop().unwrap();
        db::update_clip_note(&connection, initial.id, Some("keep-note")).unwrap();
        db::set_clip_review_decision(&connection, initial.id, db::ReviewDecision::Liked).unwrap();
        let tag = db::create_tag(&connection, "keep-tag", Some("blue")).unwrap();
        db::assign_tag_to_clip(&connection, initial.id, tag.id).unwrap();
        db::upsert_clip_metadata(
            &connection,
            db::ClipMetadataInput {
                clip_id: initial.id,
                metadata_status: "parsed",
                json_path: None,
                account_name: Some("kept-account"),
                player_name: Some("kept-player"),
                agent_name: Some("Jett"),
                map_name: Some("Ascent"),
                game_mode: Some("ranked"),
                scoreline: Some("13-9"),
                kda: Some("20/10/4"),
                extracted_text: Some("kept-structured-metadata"),
                parse_error: None,
            },
        )
        .unwrap();
        db::ensure_clip_thumbnails(&connection, &[initial.id]).unwrap();
        let old_job = db::claim_next_thumbnail_job(&connection, "2099-01-01T00:00:00Z")
            .unwrap()
            .expect("thumbnail should be claimed");

        fs::rename(&old_dir, &new_dir).expect("subdirectory should be renamed in place");
        let second = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("renamed source should synchronize");
        assert_eq!(second.new_clip_count, 0);
        assert_eq!(second.updated_clip_count, 1);
        let clips = db::list_clips(&connection).unwrap();
        assert_eq!(clips.len(), 1);
        let reconnected = &clips[0];
        assert_eq!(reconnected.id, initial.id);
        assert_eq!(reconnected.status, "available");
        assert_eq!(reconnected.note.as_deref(), Some("keep-note"));
        assert!(reconnected.favorite);
        assert_eq!(reconnected.review_decision, db::ReviewDecision::Liked);
        assert_eq!(reconnected.account_name.as_deref(), Some("kept-account"));
        assert_eq!(reconnected.player_name.as_deref(), Some("kept-player"));
        assert_eq!(reconnected.agent_name.as_deref(), Some("Jett"));
        assert_eq!(reconnected.source_relative_dir, "new-folder");
        let detail = db::find_clip_detail_by_id(&connection, initial.id)
            .unwrap()
            .expect("clip detail should remain");
        assert_eq!(
            detail.tags.iter().map(|tag| tag.id).collect::<Vec<_>>(),
            vec![tag.id]
        );

        let old_cache = format!("{}-{}.jpg", old_job.clip_id, old_job.fingerprint);
        assert!(!db::complete_thumbnail_job_if_current(
            &connection,
            &old_job,
            &old_cache,
            10,
            &old_job.fingerprint,
        )
        .unwrap());
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[cfg(windows)]
    #[test]
    fn renamed_file_moved_across_multiple_subdirectories_reconnects_in_place() {
        let (fixture, data, root) = temp_fixture("renamed-file-multilevel");
        let old_parent = root.join("old-a").join("old-b").join("old-c");
        let new_parent = root.join("new-a").join("new-b").join("new-c").join("new-d");
        fs::create_dir_all(&old_parent).expect("old nested directory should be created");
        let old_path = old_parent.join("before.mp4");
        let new_path = new_parent.join("after.mp4");
        fs::write(&old_path, b"same stable file identity").expect("fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        super::super::sync_scan_sources(&connection, &[source.id])
            .expect("initial synchronization should complete");
        let initial = db::list_clips(&connection).unwrap().pop().unwrap();
        db::update_clip_note(&connection, initial.id, Some("keep-multilevel-note")).unwrap();

        fs::create_dir_all(&new_parent).expect("new nested directory should be created");
        fs::rename(&old_path, &new_path).expect("the same file should move and be renamed");
        fs::remove_dir_all(root.join("old-a")).expect("empty old hierarchy should be removed");
        let summary = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("moved file should synchronize");

        assert_eq!(summary.new_clip_count, 0);
        assert_eq!(summary.updated_clip_count, 1);
        let clips = db::list_clips(&connection).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].id, initial.id);
        assert_eq!(clips[0].file_name, "after.mp4");
        assert_eq!(clips[0].source_relative_dir, "new-a/new-b/new-c/new-d");
        assert_eq!(clips[0].note.as_deref(), Some("keep-multilevel-note"));
        assert_eq!(clips[0].status, "available");

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[cfg(windows)]
    #[test]
    fn case_only_file_rename_updates_the_existing_path_without_a_duplicate() {
        let (fixture, data, root) = temp_fixture("case-only-file-rename");
        let original_path = root.join("Clip.MP4");
        let intermediate_path = root.join("case-rename.tmp");
        let final_path = root.join("clip.mp4");
        fs::write(&original_path, b"case-only rename").expect("fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        super::super::sync_scan_sources(&connection, &[source.id])
            .expect("initial synchronization should complete");
        let initial = db::list_clips(&connection).unwrap().pop().unwrap();
        fs::rename(&original_path, &intermediate_path)
            .expect("case-only rename should use an intermediate path");
        fs::rename(&intermediate_path, &final_path).expect("final casing should be applied");

        let summary = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("case-only rename should synchronize");
        assert_eq!(summary.new_clip_count, 0);
        let clips = db::list_clips(&connection).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].id, initial.id);
        assert_eq!(clips[0].file_name, "clip.mp4");
        assert!(clips[0].video_path.ends_with("clip.mp4"));

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn copied_file_is_indexed_separately_while_the_old_path_still_exists() {
        let (fixture, data, root) = temp_fixture("copy-keeps-old-path");
        let old_path = root.join("original.mp4");
        let copy_path = root.join("copied.mp4");
        fs::write(&old_path, b"copy must not inherit user state")
            .expect("fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        super::super::sync_scan_sources(&connection, &[source.id])
            .expect("initial synchronization should complete");
        let initial = db::list_clips(&connection).unwrap().pop().unwrap();
        db::update_clip_note(&connection, initial.id, Some("old-only-note")).unwrap();
        fs::copy(&old_path, &copy_path)
            .expect("copy should be created without removing the old path");

        let summary = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("copied file should synchronize");
        assert_eq!(summary.new_clip_count, 1);
        let clips = db::list_clips(&connection).unwrap();
        assert_eq!(clips.len(), 2);
        let original = clips.iter().find(|clip| clip.id == initial.id).unwrap();
        let copied = clips.iter().find(|clip| clip.id != initial.id).unwrap();
        assert_eq!(original.note.as_deref(), Some("old-only-note"));
        assert_eq!(copied.file_name, "copied.mp4");
        assert_eq!(copied.note, None);
        assert_eq!(original.status, "available");
        assert_eq!(copied.status, "available");

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn ambiguous_hardlink_candidates_are_inserted_separately_and_old_clip_becomes_missing() {
        let (fixture, data, root) = temp_fixture("hardlink-conflict");
        let old_dir = root.join("old");
        fs::create_dir_all(&old_dir).expect("old directory should exist");
        let old_path = old_dir.join("same.mp4");
        fs::write(&old_path, b"shared-identity").expect("old clip should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");
        super::super::sync_scan_sources(&connection, &[source.id])
            .expect("initial synchronization should complete");
        let old_id = db::list_clips(&connection).unwrap()[0].id;

        let first_dir = root.join("new-a");
        let second_dir = root.join("new-b");
        fs::create_dir_all(&first_dir).unwrap();
        fs::create_dir_all(&second_dir).unwrap();
        fs::hard_link(&old_path, first_dir.join("same.mp4"))
            .expect("first hard link should be created");
        fs::hard_link(&old_path, second_dir.join("same.mp4"))
            .expect("second hard link should be created");
        fs::remove_file(&old_path).expect("old name should disappear");

        let summary = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("ambiguous hard links should be handled safely");
        assert_eq!(summary.new_clip_count, 2);
        assert_eq!(summary.missing_clip_count, 1);
        assert!(summary.errors.iter().any(|error| {
            error.contains("not unique on both sides") || error.contains("stable file identity")
        }));
        let clips = db::list_clips(&connection).unwrap();
        assert_eq!(clips.len(), 3);
        assert_eq!(
            clips
                .iter()
                .find(|clip| clip.id == old_id)
                .expect("old clip should remain indexed")
                .status,
            "missing"
        );
        assert_eq!(
            clips
                .iter()
                .filter(|clip| clip.status == "available")
                .count(),
            2
        );
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn complete_sync_marks_missing_but_offline_source_preserves_history() {
        let (fixture, data, root) = temp_fixture("missing");
        let first_path = root.join("first.mp4");
        let second_path = root.join("second.mp4");
        fs::write(&first_path, b"first").unwrap();
        fs::write(&second_path, b"second").unwrap();
        let database_path = db::initialize_database_in(&data).unwrap();
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).unwrap();
        super::super::sync_scan_sources(&connection, &[source.id]).unwrap();

        fs::remove_file(&first_path).expect("one fixture clip should be removed");
        let missing = super::super::sync_scan_sources(&connection, &[source.id]).unwrap();
        assert_eq!(missing.missing_clip_count, 1);
        let clips = db::list_clips(&connection).unwrap();
        assert_eq!(
            clips.iter().filter(|clip| clip.status == "missing").count(),
            1
        );

        fs::remove_dir_all(&root).expect("source should be made offline");
        let offline = super::super::sync_scan_sources(&connection, &[source.id]).unwrap();
        assert_eq!(offline.missing_clip_count, 0);
        let clips_after_offline = db::list_clips(&connection).unwrap();
        assert_eq!(
            clips_after_offline
                .iter()
                .filter(|clip| clip.status == "available")
                .count(),
            1
        );
        let source_after_offline = db::find_source_dir_by_id(&connection, source.id).unwrap();
        assert_eq!(source_after_offline.status, "unavailable");
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn successful_source_in_partial_batch_becomes_available_without_becoming_fresh() {
        let (fixture, data, root) = temp_fixture("partial-batch-freshness");
        let offline_root = fixture.join("offline");
        fs::create_dir_all(&offline_root).expect("offline source should seed");
        fs::write(root.join("good.mp4"), b"good").expect("good clip should seed");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let good_source = register_generic_source(&database_path, &root);
        let offline_source = register_generic_source(&database_path, &offline_root);
        fs::remove_dir_all(&offline_root).expect("second source should become unavailable");
        let connection = db::open_database(&database_path).expect("database should open");

        let summary =
            super::super::sync_scan_sources(&connection, &[good_source.id, offline_source.id])
                .expect("partial batch should return safely");
        let good_after = db::find_source_dir_by_id(&connection, good_source.id).unwrap();
        let offline_after = db::find_source_dir_by_id(&connection, offline_source.id).unwrap();

        assert!(!summary.errors.is_empty());
        assert_eq!(good_after.status, "available");
        assert_eq!(good_after.last_scanned_at, None);
        assert_eq!(offline_after.status, "unavailable");
        assert_eq!(offline_after.last_scanned_at, None);
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn cancelled_recursive_sync_does_not_mark_unseen_history_missing() {
        let (fixture, data, root) = temp_fixture("cancelled");
        let first_path = root.join("first.mp4");
        let second_path = root.join("second.mp4");
        fs::write(&first_path, b"first").expect("first fixture should be written");
        fs::write(&second_path, b"second").expect("second fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");
        super::super::sync_scan_sources(&connection, &[source.id])
            .expect("initial synchronization should complete");
        fs::remove_file(&first_path).expect("one historical fixture should be removed");

        let cancellation = AtomicBool::new(false);
        let execution = super::super::sync_scan_source_with_progress_and_cancel(
            &connection,
            source.id,
            "cancelled-recursive-test",
            &cancellation,
            |progress| {
                if progress.clip_file_count > 0 {
                    cancellation.store(true, Ordering::Release);
                }
            },
        )
        .expect("cancelled synchronization should return a terminal result");
        assert_eq!(
            execution.status,
            super::super::ScanExecutionStatus::Cancelled
        );
        assert_eq!(execution.summary.missing_clip_count, 0);
        assert!(db::list_clips(&connection)
            .expect("clips should list")
            .iter()
            .all(|clip| clip.status == "available"));

        cancellation.store(false, Ordering::Release);
        let resumed = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("the same connection should immediately accept a fresh reconnect plan");
        assert_eq!(resumed.missing_clip_count, 1);
        assert_eq!(
            db::list_clips(&connection)
                .expect("clips should list after resumed scan")
                .iter()
                .filter(|clip| clip.status == "missing")
                .count(),
            1
        );
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn overlapping_sources_cannot_reassign_an_already_owned_clip() {
        let (fixture, data, root) = temp_fixture("ownership");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("nested root should be created");
        fs::write(nested.join("owned.mp4"), b"owned").expect("fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let parent_source = register_generic_source(&database_path, &root);
        let child_source = register_generic_source(&database_path, &nested);
        let connection = db::open_database(&database_path).expect("database should open");

        super::super::sync_scan_sources(&connection, &[parent_source.id])
            .expect("parent synchronization should complete");
        let second = super::super::sync_scan_sources(&connection, &[child_source.id])
            .expect("overlapping synchronization should remain bounded");
        let clips = db::list_clips(&connection).expect("clips should list");
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].source_dir_id, parent_source.id);
        assert_eq!(second.new_clip_count, 0);
        assert!(second
            .errors
            .iter()
            .any(|error| error.contains("already owned by source")));
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn overlapping_generic_source_cannot_import_a_nvidia_pending_file() {
        let (fixture, data, root) = temp_fixture("pending-ownership");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("nested root should be created");
        fs::write(nested.join("pending.mp4"), b"pending").expect("fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let nvidia_source = register_nvidia_source(&database_path, &root);
        let generic_source = register_generic_source(&database_path, &nested);
        let connection = db::open_database(&database_path).expect("database should open");

        let nvidia_summary = super::super::sync_scan_sources(&connection, &[nvidia_source.id])
            .expect("NVIDIA synchronization should queue the recording");
        assert_eq!(nvidia_summary.pending_clip_count, 1);
        assert!(db::list_clips(&connection).unwrap().is_empty());

        let generic_summary = super::super::sync_scan_sources(&connection, &[generic_source.id])
            .expect("overlapping generic synchronization should remain bounded");
        assert_eq!(generic_summary.new_clip_count, 0);
        assert!(db::list_clips(&connection).unwrap().is_empty());
        let pending = db::list_pending_manual_clips(&connection, false).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source_dir_id, nvidia_source.id);

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn reconnect_cannot_move_a_generic_clip_onto_a_nvidia_pending_path() {
        let (fixture, data, root) = temp_fixture("pending-reconnect-ownership");
        let old_directory = root.join("old");
        let nvidia_root = root.join("nvidia");
        fs::create_dir_all(&old_directory).expect("old directory should be created");
        fs::create_dir_all(&nvidia_root).expect("NVIDIA root should be created");
        let old_path = old_directory.join("moved.mp4");
        let new_path = nvidia_root.join("moved.mp4");
        fs::write(&old_path, b"same physical recording").expect("fixture should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let generic_source = register_generic_source(&database_path, &root);
        let nvidia_source = register_nvidia_source(&database_path, &nvidia_root);
        let connection = db::open_database(&database_path).expect("database should open");

        let first = super::super::sync_scan_sources(&connection, &[generic_source.id])
            .expect("generic synchronization should index the original path");
        assert_eq!(first.new_clip_count, 1);
        let original_clip = db::list_clips(&connection).unwrap()[0].clone();

        fs::rename(&old_path, &new_path).expect("fixture should move into the NVIDIA root");
        let nvidia_summary = super::super::sync_scan_sources(&connection, &[nvidia_source.id])
            .expect("NVIDIA synchronization should queue the moved recording");
        assert_eq!(nvidia_summary.pending_clip_count, 1);

        let reconnect = super::super::sync_scan_sources(&connection, &[generic_source.id])
            .expect("generic reconnect attempt should fail closed");
        let indexed = db::find_clip_by_id(&connection, original_clip.id)
            .expect("the original generic clip should remain indexed");
        assert_eq!(indexed.source_dir_id, generic_source.id);
        assert_eq!(indexed.normalized_path, original_clip.normalized_path);
        assert_ne!(
            indexed.normalized_path,
            db::normalize_path(&new_path.display().to_string())
        );
        let pending = db::list_pending_manual_clips(&connection, false).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source_dir_id, nvidia_source.id);
        assert!(reconnect
            .errors
            .iter()
            .any(|error| error.contains("pending manual import")));

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[cfg(windows)]
    #[test]
    fn recursive_source_skips_file_and_directory_reparse_points() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let (fixture, data, root) = temp_fixture("reparse");
        let outside_directory = fixture.join("outside");
        fs::create_dir_all(&outside_directory).expect("outside directory should be created");
        let outside_file = outside_directory.join("outside.mp4");
        fs::write(&outside_file, b"outside").expect("outside clip should be written");
        let linked_file = root.join("linked.mp4");
        let linked_directory = root.join("linked-directory");
        if let Err(error) = symlink_file(&outside_file, &linked_file) {
            eprintln!(
                "skipping reparse integration assertion because symlinks are unavailable: {error}"
            );
            fs::remove_dir_all(&fixture).expect("fixture should be removed");
            return;
        }
        if let Err(error) = symlink_dir(&outside_directory, &linked_directory) {
            eprintln!("directory symlink unavailable; file reparse assertion still runs: {error}");
        }
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        let summary = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("reparse scan should finish safely");
        assert!(db::list_clips(&connection).unwrap().is_empty());
        assert!(summary
            .errors
            .iter()
            .any(|error| error.contains("reparse point")));
        assert_eq!(
            db::find_source_dir_by_id(&connection, source.id)
                .expect("source should remain")
                .status,
            "partial"
        );
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn nvidia_sources_queue_new_recordings_instead_of_auto_importing() {
        let (fixture, data, root) = temp_fixture("nvidia-pending");
        let nested = root.join("Valorant");
        fs::create_dir_all(&nested).expect("nested directory should be created");
        fs::write(root.join("first.mp4"), b"first").expect("first recording should be written");
        fs::write(nested.join("second.mp4"), b"second")
            .expect("second recording should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        let summary = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("nvidia synchronization should complete");
        assert_eq!(
            summary.new_clip_count, 0,
            "NVIDIA files must not auto-import"
        );
        assert_eq!(summary.pending_clip_count, 2);
        assert!(db::list_clips(&connection).unwrap().is_empty());
        let pending = db::list_pending_manual_clips(&connection, false).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].source_dir_name, "NVIDIA recordings");
        assert!(!pending.iter().any(|clip| clip.ignored));

        // Re-scanning is idempotent and never resets user decisions.
        let second = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("second synchronization should complete");
        assert_eq!(second.pending_clip_count, 0);
        assert_eq!(
            db::list_pending_manual_clips(&connection, false)
                .unwrap()
                .len(),
            2
        );

        // A complete scan removes pending rows whose files disappeared.
        fs::remove_file(root.join("first.mp4")).expect("first recording should disappear");
        let third = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("cleanup synchronization should complete");
        assert_eq!(third.missing_clip_count, 0);
        assert_eq!(
            db::list_pending_manual_clips(&connection, false)
                .unwrap()
                .len(),
            1
        );

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn enabled_sync_discovers_nvidia_without_importing_it() {
        let (fixture, data, root) = temp_fixture("nvidia-enabled-sync");
        fs::write(root.join("clip.mp4"), b"recording").expect("recording should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");
        let cancellation = AtomicBool::new(false);

        let execution = super::super::sync_enabled_scan_sources_with_progress_and_cancel(
            &connection,
            "nvidia-enabled-sync-job",
            &cancellation,
            |_| {},
        )
        .expect("enabled NVIDIA synchronization should complete");

        assert_eq!(execution.summary.pending_clip_count, 1);
        assert_eq!(execution.summary.new_clip_count, 0);
        assert_eq!(
            db::list_pending_manual_clips(&connection, false)
                .unwrap()
                .len(),
            1
        );
        assert!(db::list_clips(&connection).unwrap().is_empty());

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn general_root_scan_routes_registered_nvidia_to_its_recursive_adapter() {
        let (fixture, data, root) = temp_fixture("nvidia-general-scan");
        fs::write(root.join("clip.mp4"), b"recording").expect("recording should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");

        let summary = super::super::scan_roots(&connection, std::slice::from_ref(&root))
            .expect("general root scan should preserve the registered adapter");

        assert_eq!(summary.pending_clip_count, 1);
        assert_eq!(summary.new_clip_count, 0);
        assert!(db::list_clips(&connection).unwrap().is_empty());
        let persisted = db::find_source_dir_by_id(&connection, source.id).unwrap();
        assert_eq!(persisted.source_kind, db::SourceKind::Nvidia);
        assert_eq!(persisted.scan_mode, db::ScanMode::RecursiveMp4);

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }

    #[test]
    fn imported_nvidia_pending_clip_is_indexed_and_not_requeued() {
        let (fixture, data, root) = temp_fixture("nvidia-import");
        fs::write(root.join("clip.mp4"), b"recording").expect("recording should be written");
        let database_path = db::initialize_database_in(&data).expect("database should initialize");
        let source = register_nvidia_source(&database_path, &root);
        let connection = db::open_database(&database_path).expect("database should open");
        super::super::sync_scan_sources(&connection, &[source.id])
            .expect("nvidia synchronization should complete");
        let pending_id = db::list_pending_manual_clips(&connection, false).unwrap()[0].id;

        let clip_id = db::import_pending_manual_clip(
            &connection,
            pending_id,
            &db::ManualClipImportInput {
                account_key: None,
                account_name: "Tester#123".to_string(),
                player_name: None,
                agent_name: "捷风".to_string(),
                map_name: Some("霓虹町".to_string()),
                game_mode: Some("竞技模式".to_string()),
                note: Some("手动录入".to_string()),
            },
        )
        .expect("pending clip should import");

        assert!(db::list_pending_manual_clips(&connection, false)
            .unwrap()
            .is_empty());
        let clip = db::find_clip_by_id(&connection, clip_id).expect("imported clip should load");
        assert_eq!(clip.account_display_name, "Tester#123");
        assert!(clip
            .account_identity_key
            .starts_with("match-account-manual-"));
        assert_eq!(clip.agent_name.as_deref(), Some("捷风"));
        assert_eq!(clip.map_name.as_deref(), Some("霓虹町"));
        assert_eq!(clip.metadata_status, "manual");
        assert_eq!(clip.note.as_deref(), Some("手动录入"));

        // The indexed clip is now an existing path; rescanning must not requeue it.
        let rescan = super::super::sync_scan_sources(&connection, &[source.id])
            .expect("rescan should complete");
        assert_eq!(rescan.pending_clip_count, 0);
        assert_eq!(rescan.new_clip_count, 0);
        assert_eq!(db::list_clips(&connection).unwrap().len(), 1);
        assert!(db::list_pending_manual_clips(&connection, false)
            .unwrap()
            .is_empty());

        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }
    #[test]
    #[ignore = "M2 10k-file performance gate; run explicitly before release candidates"]
    fn ten_thousand_file_gate_uses_bounded_batches() {
        let (fixture, data, root) = temp_fixture("ten-thousand");
        for index in 0..10_000 {
            fs::write(root.join(format!("clip-{index:05}.mp4")), b"x")
                .expect("performance fixture should be written");
        }
        let database_path = db::initialize_database_in(&data).unwrap();
        let source = register_generic_source(&database_path, &root);
        let connection = db::open_database(&database_path).unwrap();
        let started = std::time::Instant::now();
        let summary = super::super::sync_scan_sources(&connection, &[source.id]).unwrap();
        assert_eq!(summary.new_clip_count, 10_000);
        assert_eq!(db::list_clips(&connection).unwrap().len(), 10_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(120));
        drop(connection);
        fs::remove_dir_all(&fixture).expect("fixture should be removed");
    }
}
