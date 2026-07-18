use std::{
    collections::{HashMap, VecDeque},
    env, fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

pub const MAX_WARNING_SAMPLES: usize = 8;
const PROGRESS_INTERVAL_DIRECTORIES: u64 = 250;
const SYSTEM_JUNK_DIRECTORY_NAMES: [&str; 2] = ["$recycle.bin", "system volume information"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryProgress {
    pub current_drive: String,
    pub visited_directory_count: u64,
    pub validated_source_dir_count: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryResult {
    pub fixed_drive_count: u64,
    pub opened_drive_count: u64,
    pub visited_directory_count: u64,
    pub validated_source_dir_count: u64,
    pub validated_source_dirs: Vec<PathBuf>,
    pub scan_roots: Vec<PathBuf>,
    pub skipped_directory_count: u64,
    pub warnings: Vec<String>,
    pub cancelled: bool,
}

impl DiscoveryResult {
    fn new(fixed_drive_count: usize) -> Self {
        Self {
            fixed_drive_count: fixed_drive_count.min(u64::MAX as usize) as u64,
            opened_drive_count: 0,
            visited_directory_count: 0,
            validated_source_dir_count: 0,
            validated_source_dirs: Vec::new(),
            scan_roots: Vec::new(),
            skipped_directory_count: 0,
            warnings: Vec::new(),
            cancelled: false,
        }
    }

    fn record_warning(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.skipped_directory_count = self.skipped_directory_count.saturating_add(1);
        if self.warnings.len() < MAX_WARNING_SAMPLES {
            self.warnings
                .push(format!("无法读取目录 {}：{error}", path.display()));
        }
    }
}

pub fn discover_scan_roots<F>(roots: &[PathBuf], progress: F) -> DiscoveryResult
where
    F: Fn(DiscoveryProgress),
{
    discover_scan_roots_with_exclusions(roots, &[], progress)
}

pub fn discover_scan_roots_with_exclusions<F>(
    roots: &[PathBuf],
    excluded_roots: &[PathBuf],
    progress: F,
) -> DiscoveryResult
where
    F: Fn(DiscoveryProgress),
{
    discover_scan_roots_with_exclusions_and_cancel(roots, excluded_roots, progress, || false)
}

pub fn discover_scan_roots_with_exclusions_and_cancel<F, C>(
    roots: &[PathBuf],
    excluded_roots: &[PathBuf],
    progress: F,
    is_cancelled: C,
) -> DiscoveryResult
where
    F: Fn(DiscoveryProgress),
    C: Fn() -> bool,
{
    let mut result = DiscoveryResult::new(roots.len());
    let mut scan_roots_by_key = HashMap::<String, PathBuf>::new();
    let mut source_dirs_by_key = HashMap::<String, PathBuf>::new();
    let mut last_drive = String::new();
    let normalized_excluded_roots = normalized_exclusion_roots(excluded_roots);

    'roots: for root in roots {
        if is_cancelled() {
            result.cancelled = true;
            break;
        }
        let drive_label = root.display().to_string();
        last_drive = drive_label.clone();
        emit_progress(&progress, &result, &drive_label, "正在搜索固定磁盘");

        let root_entries = match read_entries(root) {
            Ok(entries) => entries,
            Err(error) => {
                result.record_warning(root, error);
                continue;
            }
        };
        result.opened_drive_count = result.opened_drive_count.saturating_add(1);

        let mut queue = VecDeque::from([(root.clone(), Some(root_entries))]);
        while let Some((directory, cached_entries)) = queue.pop_front() {
            if is_cancelled() {
                result.cancelled = true;
                break 'roots;
            }
            result.visited_directory_count = result.visited_directory_count.saturating_add(1);
            if result
                .visited_directory_count
                .is_multiple_of(PROGRESS_INTERVAL_DIRECTORIES)
            {
                emit_progress(&progress, &result, &drive_label, "正在搜索无畏时刻目录");
            }
            let entries = match cached_entries {
                Some(entries) => entries,
                None => match read_entries(&directory) {
                    Ok(entries) => entries,
                    Err(error) => {
                        result.record_warning(&directory, error);
                        continue;
                    }
                },
            };

            for entry in entries {
                if is_cancelled() {
                    result.cancelled = true;
                    break 'roots;
                }
                let entry_type = match entry.file_type() {
                    Ok(entry_type) => entry_type,
                    Err(error) => {
                        result.record_warning(&entry.path(), error);
                        continue;
                    }
                };
                if !entry_type.is_dir() || entry_type.is_symlink() {
                    continue;
                }

                let path = entry.path();
                if is_excluded_directory(&path, &normalized_excluded_roots) {
                    continue;
                }
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        result.record_warning(&path, error);
                        continue;
                    }
                };
                if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
                    continue;
                }

                if is_candidate_name(&path) {
                    match candidate_has_direct_group_mp4(&path) {
                        Ok(true) => {
                            let source_key = crate::db::normalize_path(&path.display().to_string());
                            if source_dirs_by_key
                                .insert(source_key, path.clone())
                                .is_none()
                            {
                                result.validated_source_dir_count =
                                    result.validated_source_dir_count.saturating_add(1);
                                if let Some(parent) = path.parent() {
                                    let parent = parent.to_path_buf();
                                    scan_roots_by_key
                                        .entry(crate::db::normalize_path(
                                            &parent.display().to_string(),
                                        ))
                                        .or_insert(parent);
                                }
                                emit_progress(
                                    &progress,
                                    &result,
                                    &drive_label,
                                    "已发现无畏时刻素材目录",
                                );
                            }
                        }
                        Ok(false) => {}
                        Err(error) => result.record_warning(&path, error),
                    }
                    continue;
                }

                queue.push_back((path, None));
            }
        }
    }

    result.validated_source_dirs = source_dirs_by_key.into_values().collect();
    result.validated_source_dirs.sort_by_key(|path| {
        crate::db::normalize_path(&path.display().to_string()).to_ascii_lowercase()
    });
    result.scan_roots = scan_roots_by_key.into_values().collect();
    result.scan_roots.sort_by_key(|path| {
        crate::db::normalize_path(&path.display().to_string()).to_ascii_lowercase()
    });
    emit_progress(
        &progress,
        &result,
        &last_drive,
        if result.cancelled {
            "固定磁盘搜索已取消"
        } else {
            "固定磁盘搜索完成"
        },
    );
    result
}

pub fn fixed_drive_exclusion_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for variable in ["TEMP", "TMP"] {
        if let Some(path) = env::var_os(variable).filter(|value| !value.is_empty()) {
            push_unique_exclusion_root(&mut roots, PathBuf::from(path));
        }
    }
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        push_unique_exclusion_root(&mut roots, PathBuf::from(local_app_data).join("Temp"));
    }
    for variable in ["SystemRoot", "WINDIR"] {
        if let Some(windows_dir) = env::var_os(variable).filter(|value| !value.is_empty()) {
            push_unique_exclusion_root(&mut roots, PathBuf::from(windows_dir).join("Temp"));
        }
    }
    roots
}

fn push_unique_exclusion_root(roots: &mut Vec<PathBuf>, path: PathBuf) {
    let key = crate::db::normalize_path(&path.display().to_string());
    if !roots
        .iter()
        .any(|existing| crate::db::normalize_path(&existing.display().to_string()) == key)
    {
        roots.push(path);
    }
}

fn normalized_exclusion_roots(roots: &[PathBuf]) -> Vec<String> {
    roots
        .iter()
        .map(|root| {
            crate::db::normalize_path(&root.display().to_string())
                .trim_end_matches('/')
                .to_string()
        })
        .filter(|root| !root.is_empty())
        .collect()
}

fn is_excluded_directory(path: &Path, excluded_roots: &[String]) -> bool {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            SYSTEM_JUNK_DIRECTORY_NAMES
                .iter()
                .any(|excluded| name.eq_ignore_ascii_case(excluded))
        })
    {
        return true;
    }

    let normalized = crate::db::normalize_path(&path.display().to_string());
    excluded_roots.iter().any(|root| {
        normalized == *root
            || normalized
                .strip_prefix(root)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

fn emit_progress<F>(progress: &F, result: &DiscoveryResult, current_drive: &str, message: &str)
where
    F: Fn(DiscoveryProgress),
{
    progress(DiscoveryProgress {
        current_drive: current_drive.to_string(),
        visited_directory_count: result.visited_directory_count,
        validated_source_dir_count: result.validated_source_dir_count,
        message: message.to_string(),
    });
}

fn read_entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn is_candidate_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("wonderfulvideos"))
}

fn candidate_has_direct_group_mp4(path: &Path) -> Result<bool, String> {
    let mut first_error = None;
    for group in read_entries(path)? {
        let entry_type = match group.file_type() {
            Ok(entry_type) => entry_type,
            Err(error) => {
                first_error.get_or_insert_with(|| error.to_string());
                continue;
            }
        };
        if entry_type.is_file()
            && group
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
        {
            return Ok(true);
        }
        if !entry_type.is_dir() || entry_type.is_symlink() {
            continue;
        }

        let group_path = group.path();
        let metadata = match fs::symlink_metadata(&group_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                first_error.get_or_insert_with(|| error.to_string());
                continue;
            }
        };
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            continue;
        }

        let entries = match read_entries(&group_path) {
            Ok(entries) => entries,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        if entries.into_iter().any(|entry| {
            entry
                .file_type()
                .is_ok_and(|entry_type| entry_type.is_file())
                && entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
        }) {
            return Ok(true);
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(false),
    }
}

fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        has_reparse_attribute(metadata.file_attributes())
    }

    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn has_reparse_attribute(attributes: u32) -> bool {
    #[cfg(windows)]
    {
        attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    {
        attributes & 0x400 != 0
    }
}

fn is_fixed_drive_type(drive_type: u32) -> bool {
    #[cfg(windows)]
    {
        drive_type == windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED
    }

    #[cfg(not(windows))]
    {
        drive_type == 3
    }
}

#[cfg(windows)]
pub fn fixed_drive_roots() -> Result<Vec<PathBuf>, String> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};

    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDriveStringsW};

    let required = unsafe { GetLogicalDriveStringsW(0, ptr::null_mut()) };
    if required == 0 {
        return Err("无法枚举 Windows 磁盘".to_string());
    }

    let mut buffer = vec![0u16; required as usize + 1];
    let written = unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) };
    if written == 0 || written as usize >= buffer.len() {
        return Err("无法读取 Windows 磁盘列表".to_string());
    }

    let mut roots = Vec::new();
    let mut start = 0usize;
    while start < written as usize {
        let Some(relative_end) = buffer[start..].iter().position(|value| *value == 0) else {
            break;
        };
        if relative_end == 0 {
            break;
        }
        let end = start + relative_end;
        if is_fixed_drive_type(unsafe { GetDriveTypeW(buffer[start..=end].as_ptr()) }) {
            roots.push(PathBuf::from(OsString::from_wide(&buffer[start..end])));
        }
        start = end + 1;
    }
    roots.sort_by_key(|path| path.display().to_string().to_ascii_lowercase());
    Ok(roots)
}

#[cfg(not(windows))]
pub fn fixed_drive_roots() -> Result<Vec<PathBuf>, String> {
    Err("全电脑发现仅支持 Windows".to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    #[test]
    fn discovers_nested_valid_source_and_returns_its_parent_root() {
        let fixture = TestFixture::new("nested-valid");
        let scan_root = fixture.path().join("Archive");
        let group = scan_root.join("WonderfulVideos1001").join("match-a");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("ACE.MP4"), b"video").expect("clip should be written");

        let result = discover_scan_roots(&[fixture.path().to_path_buf()], |_| {});

        assert_eq!(result.scan_roots, vec![scan_root]);
        assert_eq!(result.validated_source_dir_count, 1);
        assert_eq!(result.opened_drive_count, 1);
    }

    #[test]
    fn discovers_source_with_mp4_at_source_root() {
        let fixture = TestFixture::new("source-root-mp4");
        let scan_root = fixture.path().join("Archive");
        let source = scan_root.join("wonderfulVideos1001");
        fs::create_dir_all(&source).expect("source should be created");
        fs::write(source.join("legacy.mp4"), b"video").expect("root clip should be written");

        let result = discover_scan_roots(&[fixture.path().to_path_buf()], |_| {});

        assert_eq!(result.scan_roots, vec![scan_root]);
        assert_eq!(result.validated_source_dirs, vec![source]);
        assert_eq!(result.validated_source_dir_count, 1);
    }

    #[test]
    fn rejects_candidate_without_direct_group_mp4() {
        let fixture = TestFixture::new("invalid-candidate");
        let nested = fixture
            .path()
            .join("wonderfulVideos1001")
            .join("match-a")
            .join("nested");
        fs::create_dir_all(&nested).expect("nested directory should be created");
        fs::write(nested.join("clip.mp4"), b"video").expect("clip should be written");

        let result = discover_scan_roots(&[fixture.path().to_path_buf()], |_| {});

        assert!(result.scan_roots.is_empty());
        assert_eq!(result.validated_source_dir_count, 0);
    }

    #[test]
    fn excludes_candidates_under_temporary_and_system_junk_roots() {
        let fixture = TestFixture::new("excluded-roots");
        let allowed_root = fixture.path().join("Archive");
        let excluded_root = fixture.path().join("Temp");
        for parent in [
            allowed_root.clone(),
            excluded_root.clone(),
            fixture.path().join("$Recycle.Bin").join("deleted"),
            fixture
                .path()
                .join("System Volume Information")
                .join("snapshot"),
        ] {
            let group = parent.join("wonderfulVideos1001").join("match-a");
            fs::create_dir_all(&group).expect("group should be created");
            fs::write(group.join("clip.mp4"), b"video").expect("clip should be written");
        }

        let result = discover_scan_roots_with_exclusions(
            &[fixture.path().to_path_buf()],
            &[excluded_root],
            |_| {},
        );

        assert_eq!(result.scan_roots, vec![allowed_root]);
        assert_eq!(result.validated_source_dir_count, 1);
    }

    #[test]
    fn deduplicates_shared_parent_and_bounds_read_warnings() {
        let fixture = TestFixture::new("dedupe-warnings");
        for suffix in ["1001", "1002"] {
            let group = fixture
                .path()
                .join(format!("wonderfulVideos{suffix}"))
                .join("match-a");
            fs::create_dir_all(&group).expect("group should be created");
            fs::write(group.join("clip.mp4"), b"video").expect("clip should be written");
        }
        let invalid_roots = (0..12)
            .map(|index| {
                let path = fixture.path().join(format!("not-a-directory-{index}"));
                fs::write(&path, b"file").expect("invalid root file should be written");
                path
            })
            .collect::<Vec<_>>();
        let mut roots = vec![fixture.path().to_path_buf()];
        roots.extend(invalid_roots);

        let result = discover_scan_roots(&roots, |_| {});

        assert_eq!(result.scan_roots, vec![fixture.path().to_path_buf()]);
        assert_eq!(result.validated_source_dir_count, 2);
        assert_eq!(result.skipped_directory_count, 12);
        assert_eq!(result.warnings.len(), MAX_WARNING_SAMPLES);
    }

    #[test]
    fn recognizes_windows_reparse_attributes() {
        assert!(has_reparse_attribute(0x400));
        assert!(!has_reparse_attribute(0));
    }

    #[test]
    fn recognizes_only_windows_fixed_drive_type() {
        assert!(is_fixed_drive_type(3));
        assert!(!is_fixed_drive_type(2));
        assert!(!is_fixed_drive_type(4));
    }

    #[test]
    fn reports_discovery_progress_metrics() {
        let fixture = TestFixture::new("progress");
        let group = fixture.path().join("wonderfulVideos1001").join("match-a");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("clip should be written");
        let events = std::cell::RefCell::new(Vec::new());

        let result = discover_scan_roots(&[fixture.path().to_path_buf()], |event| {
            events.borrow_mut().push(event);
        });

        assert!(events
            .borrow()
            .iter()
            .any(|event| event.visited_directory_count > 0));
        assert!(events
            .borrow()
            .iter()
            .any(|event| event.validated_source_dir_count == 1));
        assert_eq!(result.validated_source_dir_count, 1);
    }

    #[test]
    fn cancellation_stops_discovery_before_traversal() {
        let fixture = TestFixture::new("cancelled");
        let group = fixture.path().join("wonderfulVideos1001").join("match-a");
        fs::create_dir_all(&group).expect("group should be created");
        fs::write(group.join("clip.mp4"), b"video").expect("clip should be written");
        let events = std::cell::RefCell::new(Vec::new());

        let result = discover_scan_roots_with_exclusions_and_cancel(
            &[fixture.path().to_path_buf()],
            &[],
            |event| events.borrow_mut().push(event),
            || true,
        );

        assert!(result.cancelled);
        assert_eq!(result.opened_drive_count, 0);
        assert_eq!(result.visited_directory_count, 0);
        assert!(result.scan_roots.is_empty());
        assert_eq!(
            events.borrow().last().map(|event| event.message.as_str()),
            Some("固定磁盘搜索已取消")
        );
    }

    struct TestFixture {
        path: PathBuf,
    }

    impl TestFixture {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vhm-drive-discovery-{label}-{}-{unique}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path).expect("stale fixture should be removed");
            }
            fs::create_dir_all(&path).expect("fixture root should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            if self.path.exists() {
                fs::remove_dir_all(&self.path).expect("fixture should be removed");
            }
        }
    }
}
