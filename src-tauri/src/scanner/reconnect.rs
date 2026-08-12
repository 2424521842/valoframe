//! Shared scan-candidate collection for source-local path reconnection.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    db::{self, DbResult, ScanReconnectCandidate, ScanReconnectCandidateInput},
    file_identity::{read_stable_file_snapshot, StableFileIdentity},
};

const MAX_IDENTITY_DIAGNOSTIC_SAMPLES: usize = 4;

pub(super) struct ScanReconnectPlanGuard<'a> {
    connection: &'a rusqlite::Connection,
}

impl<'a> ScanReconnectPlanGuard<'a> {
    pub(super) fn begin(
        connection: &'a rusqlite::Connection,
        source_dir_id: i64,
    ) -> DbResult<Self> {
        db::begin_scan_reconnect_plan(connection, source_dir_id)?;
        Ok(Self { connection })
    }
}

impl Drop for ScanReconnectPlanGuard<'_> {
    fn drop(&mut self) {
        let _ = db::clear_scan_reconnect_plan(self.connection);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CollectedScanCandidate {
    pub(super) path: PathBuf,
    pub(super) file_path: String,
    pub(super) normalized_path: String,
    pub(super) file_name: String,
    pub(super) size_bytes: i64,
    pub(super) modified_at: String,
    pub(super) file_identity: Option<StableFileIdentity>,
    pub(super) validation_token: String,
}

impl CollectedScanCandidate {
    pub(super) fn stage_input(&self, source_dir_id: i64) -> ScanReconnectCandidateInput<'_> {
        ScanReconnectCandidateInput {
            source_dir_id,
            file_path: &self.file_path,
            normalized_path: &self.normalized_path,
            file_name: &self.file_name,
            size_bytes: self.size_bytes,
            modified_at: Some(&self.modified_at),
            file_identity: self.file_identity,
            validation_token: &self.validation_token,
        }
    }

    pub(super) fn matches_staged(&self, staged: &ScanReconnectCandidate) -> bool {
        self.normalized_path == staged.normalized_path
            && self.file_name == staged.file_name
            && self.size_bytes == staged.size_bytes
            && Some(self.modified_at.as_str()) == staged.modified_at.as_deref()
            && self.validation_token == staged.validation_token
    }
}

#[derive(Debug, Default)]
pub(super) struct IdentityReadDiagnostics {
    failure_count: usize,
}

impl IdentityReadDiagnostics {
    fn record(&mut self, path: &Path, error: &std::io::Error) {
        self.failure_count = self.failure_count.saturating_add(1);
        if self.failure_count <= MAX_IDENTITY_DIAGNOSTIC_SAMPLES {
            eprintln!(
                "scan identity unavailable for {} (indexing continues): {error}",
                path.display()
            );
        } else if self.failure_count == MAX_IDENTITY_DIAGNOSTIC_SAMPLES + 1 {
            eprintln!("additional scan identity diagnostics omitted (indexing continues normally)");
        }
    }

    #[cfg(test)]
    pub(super) const fn failure_count(&self) -> usize {
        self.failure_count
    }
}

/// Collects the path/fingerprint used for matching. Stable identity is deliberately best-effort:
/// failure is a bounded debug diagnostic and never changes scan terminal status or freshness.
pub(super) fn collect_scan_candidate(
    path: PathBuf,
    identity_diagnostics: &mut IdentityReadDiagnostics,
) -> Result<CollectedScanCandidate, String> {
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        format!(
            "Failed to inspect MP4 candidate {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata_is_reparse_point(&metadata) {
        return Err(format!(
            "MP4 candidate is not a regular non-reparse file: {}",
            path.display()
        ));
    }
    let modified = metadata.modified().map_err(|error| {
        format!(
            "Failed to read a stable modified time for {}: {error}",
            path.display()
        )
    })?;
    let size_bytes = i64::try_from(metadata.len()).map_err(|_| {
        format!(
            "MP4 candidate size exceeds the supported range: {}",
            path.display()
        )
    })?;
    let file_identity = match read_stable_file_snapshot(&path) {
        Ok(snapshot) if snapshot.size_bytes == size_bytes => snapshot.identity,
        Ok(_) => {
            return Err(format!(
                "MP4 candidate changed while reading file identity: {}",
                path.display()
            ))
        }
        Err(error) => {
            identity_diagnostics.record(&path, &error);
            None
        }
    };
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    // Keep the canonical `PathBuf` for all filesystem authorization and revalidation, but never
    // let Windows' implementation-only `\\?\` spelling become a persisted candidate identity.
    let path_string = db::stable_path_for_storage(&path.to_string_lossy());
    let normalized_path = db::normalize_path(&path_string);
    Ok(CollectedScanCandidate {
        path,
        file_path: path_string,
        normalized_path,
        file_name,
        size_bytes,
        modified_at: super::format_system_time(modified),
        file_identity,
        validation_token: system_time_validation_token(modified),
    })
}

pub(super) fn revalidate_staged_candidate(
    staged: &ScanReconnectCandidate,
    identity_diagnostics: &mut IdentityReadDiagnostics,
) -> DbResult<CollectedScanCandidate> {
    let current = collect_scan_candidate(PathBuf::from(&staged.file_path), identity_diagnostics)?;
    if !current.matches_staged(staged) {
        return Err(format!(
            "MP4 candidate changed after source-wide reconnect planning: {}",
            staged.file_path
        ));
    }
    Ok(current)
}

pub(super) fn concrete_file_identity_changed(
    staged: Option<StableFileIdentity>,
    current: Option<StableFileIdentity>,
) -> bool {
    matches!((staged, current), (Some(staged), Some(current)) if staged != current)
}

pub(super) fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
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

/// Canonicalizes an authorized directory only after every existing component from the selected
/// path to its filesystem root has been proven to be a normal directory. Checking only the leaf
/// is insufficient on Windows because an ancestor junction can redirect an otherwise ordinary
/// child directory outside the user's registered root.
pub(super) fn canonicalize_non_reparse_directory_chain(path: &Path) -> Result<PathBuf, String> {
    for ancestor in path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
    {
        let metadata = fs::symlink_metadata(ancestor).map_err(|error| {
            format!(
                "Failed to inspect authorized directory chain {}: {error}",
                ancestor.display()
            )
        })?;
        if !metadata.is_dir() || metadata_is_reparse_point(&metadata) {
            return Err(format!(
                "Authorized directory chain contains a non-directory, symbolic link, or reparse point: {}",
                ancestor.display()
            ));
        }
    }

    let canonical = path.canonicalize().map_err(|error| {
        format!(
            "Failed to canonicalize authorized directory {}: {error}",
            path.display()
        )
    })?;
    let canonical_metadata = fs::symlink_metadata(&canonical).map_err(|error| {
        format!(
            "Failed to revalidate authorized directory {}: {error}",
            canonical.display()
        )
    })?;
    if !canonical_metadata.is_dir() || metadata_is_reparse_point(&canonical_metadata) {
        return Err(format!(
            "Canonical authorized path is not a regular non-reparse directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

pub(super) fn canonicalize_regular_path_within_root(
    path: &Path,
    canonical_root: &Path,
) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
    if metadata_is_reparse_point(&metadata) {
        return Err(format!(
            "Skipped symbolic link or reparse point: {}",
            path.display()
        ));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Failed to canonicalize {}: {error}", path.display()))?;
    if canonical == canonical_root || !canonical.starts_with(canonical_root) {
        return Err(format!(
            "Skipped path outside the authorized source root: {}",
            path.display()
        ));
    }
    let mut cursor = Some(canonical.as_path());
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
                path.display()
            ));
        }
        if current == canonical_root {
            return Ok(canonical);
        }
        cursor = current.parent();
    }
    Err(format!(
        "Skipped path whose parent chain does not reach the authorized root: {}",
        path.display()
    ))
}

fn system_time_validation_token(time: SystemTime) -> String {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => format!("after:{}:{}", duration.as_secs(), duration.subsec_nanos()),
        Err(error) => {
            let duration = error.duration();
            format!("before:{}:{}", duration.as_secs(), duration.subsec_nanos())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collecting_a_regular_file_produces_a_stable_validation_token() {
        let root = std::env::temp_dir().join(format!(
            "vhm-scan-candidate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("clip.mp4");
        fs::write(&path, b"candidate").unwrap();
        let mut diagnostics = IdentityReadDiagnostics::default();
        let first = collect_scan_candidate(path.clone(), &mut diagnostics).unwrap();
        let second = collect_scan_candidate(path, &mut diagnostics).unwrap();
        assert_eq!(first.validation_token, second.validation_token);
        assert_eq!(first.size_bytes, 9);
        assert_eq!(diagnostics.failure_count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_two_different_concrete_identities_are_a_candidate_change() {
        let first = StableFileIdentity {
            volume_serial: 1,
            file_index_high: 2,
            file_index_low: 3,
        };
        let second = StableFileIdentity {
            volume_serial: 1,
            file_index_high: 2,
            file_index_low: 4,
        };
        assert!(concrete_file_identity_changed(Some(first), Some(second)));
        assert!(!concrete_file_identity_changed(Some(first), Some(first)));
        assert!(!concrete_file_identity_changed(Some(first), None));
        assert!(!concrete_file_identity_changed(None, Some(first)));
    }

    #[test]
    fn authorized_directory_chain_rejects_a_linked_ancestor_when_supported() {
        let root = std::env::temp_dir().join(format!(
            "vhm-scan-root-chain-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let target = root.join("target");
        let child = target.join("child");
        let linked = root.join("linked");
        fs::create_dir_all(&child).unwrap();

        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_dir(&target, &linked) {
            eprintln!(
                "skipping linked-ancestor assertion because symlinks are unavailable: {error}"
            );
            fs::remove_dir_all(root).unwrap();
            return;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        #[cfg(not(any(windows, unix)))]
        {
            fs::remove_dir_all(root).unwrap();
            return;
        }

        let selected = linked.join("child");
        let error = canonicalize_non_reparse_directory_chain(&selected)
            .expect_err("an ordinary leaf below a linked ancestor must be rejected");
        assert!(
            error.contains("symbolic link") || error.contains("reparse point"),
            "{error}"
        );
        fs::remove_dir(&linked).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_not_candidates() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "vhm-scan-candidate-link-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target.mp4");
        let link = root.join("link.mp4");
        fs::write(&target, b"candidate").unwrap();
        symlink(&target, &link).unwrap();
        let mut diagnostics = IdentityReadDiagnostics::default();
        assert!(collect_scan_candidate(link, &mut diagnostics).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
