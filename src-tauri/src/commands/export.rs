use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

use serde::Serialize;
use tauri::State;

use crate::{db, AppState};

const MAX_COLLISION_SUFFIX: u32 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedClip {
    pub clip_id: i64,
    pub file_name: String,
    pub destination_path: String,
    pub bytes_copied: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportClipFailure {
    pub clip_id: i64,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportClipsResult {
    pub requested: usize,
    pub exported: usize,
    pub failed: usize,
    pub destination_dir: String,
    pub exported_ids: Vec<i64>,
    /// Clip ids that no longer have a database row.
    pub missing_ids: Vec<i64>,
    /// Clip ids whose database row exists but whose source video is gone.
    pub missing_file_ids: Vec<i64>,
    pub exports: Vec<ExportedClip>,
    pub failures: Vec<ExportClipFailure>,
}

#[tauri::command]
pub async fn export_clips(
    state: State<'_, AppState>,
    clip_ids: Vec<i64>,
    destination_dir: String,
) -> Result<ExportClipsResult, String> {
    let database_path = state.database_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        export_clips_for_database(database_path, &clip_ids, destination_dir)
    })
    .await
    .map_err(|error| format!("导出任务异常终止：{error}"))?
}

pub(crate) fn export_clips_for_database(
    database_path: impl AsRef<Path>,
    clip_ids: &[i64],
    destination_dir: impl AsRef<Path>,
) -> Result<ExportClipsResult, String> {
    let destination_dir = validate_destination_dir(destination_dir.as_ref())?;
    let connection = db::open_database_read_only(database_path)?;
    let mut seen = HashSet::with_capacity(clip_ids.len());
    let unique_clip_ids = clip_ids
        .iter()
        .copied()
        .filter(|clip_id| seen.insert(*clip_id))
        .collect::<Vec<_>>();
    // Finish every fallible database read before the first file is created. A database error can
    // therefore remain a top-level command error without hiding files already exported by this
    // invocation.
    let targets = unique_clip_ids
        .into_iter()
        .map(|clip_id| {
            db::find_clip_file_target_by_id(&connection, clip_id).map(|target| (clip_id, target))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut result = ExportClipsResult {
        requested: targets.len(),
        exported: 0,
        failed: 0,
        destination_dir: destination_dir.to_string_lossy().into_owned(),
        exported_ids: Vec::new(),
        missing_ids: Vec::new(),
        missing_file_ids: Vec::new(),
        exports: Vec::new(),
        failures: Vec::new(),
    };

    for (clip_id, target) in targets {
        let Some(target) = target else {
            result.missing_ids.push(clip_id);
            push_failure(
                &mut result,
                clip_id,
                "clip-not-found",
                format!("未找到素材（ID {clip_id}）"),
            );
            continue;
        };

        match export_one_clip(&target, &destination_dir) {
            Ok(exported) => {
                result.exported_ids.push(clip_id);
                result.exports.push(ExportedClip {
                    clip_id,
                    file_name: exported
                        .destination_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    destination_path: exported.destination_path.to_string_lossy().into_owned(),
                    bytes_copied: exported.bytes_copied,
                });
            }
            Err(failure) => {
                if failure.code == "source-file-missing" {
                    result.missing_file_ids.push(clip_id);
                }
                push_failure(&mut result, clip_id, failure.code, failure.message);
            }
        }
    }

    result.exported = result.exports.len();
    result.failed = result.failures.len();
    debug_assert_eq!(result.requested, result.exported + result.failed);
    debug_assert_eq!(result.exported_ids.len(), result.exported);
    Ok(result)
}

#[derive(Debug)]
struct ExportedFile {
    destination_path: PathBuf,
    bytes_copied: u64,
}

#[derive(Debug)]
struct ItemFailure {
    code: &'static str,
    message: String,
}

fn export_one_clip(
    target: &db::ClipFileTarget,
    destination_dir: &Path,
) -> Result<ExportedFile, ItemFailure> {
    let source_path = match fs::canonicalize(&target.video_path) {
        Ok(path) => path,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(ItemFailure {
                code: "source-file-missing",
                message: format!("源视频不存在：{}", Path::new(&target.video_path).display()),
            });
        }
        Err(error) => {
            return Err(ItemFailure {
                code: "source-path-invalid",
                message: format!("无法验证源视频路径：{error}"),
            });
        }
    };
    let source_dir = fs::canonicalize(&target.source_dir_path).map_err(|error| ItemFailure {
        code: "source-path-invalid",
        message: format!("无法验证素材来源目录：{error}"),
    })?;
    if !target.extension.eq_ignore_ascii_case("mp4")
        || !source_path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
        || !source_path.is_file()
        || !source_dir.is_dir()
        || !source_path.starts_with(&source_dir)
    {
        return Err(ItemFailure {
            code: "unsafe-source",
            message: "源文件不是索引来源目录内的 MP4 视频，已拒绝导出".to_string(),
        });
    }

    let mut source_file = File::open(&source_path).map_err(|error| ItemFailure {
        code: if error.kind() == ErrorKind::NotFound {
            "source-file-missing"
        } else {
            "source-open-failed"
        },
        message: format!("无法打开源视频“{}”：{error}", source_path.display()),
    })?;
    copy_to_unique_destination(&mut source_file, destination_dir, &source_path)
}

fn copy_to_unique_destination(
    source_file: &mut impl Read,
    destination_dir: &Path,
    source_path: &Path,
) -> Result<ExportedFile, ItemFailure> {
    let (destination_path, mut destination_file) =
        create_unique_destination(destination_dir, source_path).map_err(|error| ItemFailure {
            code: "destination-create-failed",
            message: format!("无法在导出目录创建文件：{error}"),
        })?;

    let copy_result = (|| -> io::Result<u64> {
        let bytes_copied = io::copy(source_file, &mut destination_file)?;
        destination_file.flush()?;
        Ok(bytes_copied)
    })();
    drop(destination_file);

    match copy_result {
        Ok(bytes_copied) => Ok(ExportedFile {
            destination_path,
            bytes_copied,
        }),
        Err(error) => {
            let cleanup_error = fs::remove_file(&destination_path).err();
            let mut message = format!("复制视频“{}”失败：{error}", source_path.display());
            if let Some(cleanup_error) = cleanup_error {
                message.push_str(&format!(
                    "；且无法清理未完成文件“{}”：{cleanup_error}",
                    destination_path.display()
                ));
            }
            Err(ItemFailure {
                code: "copy-failed",
                message,
            })
        }
    }
}

fn validate_destination_dir(destination_dir: &Path) -> Result<PathBuf, String> {
    if destination_dir.as_os_str().is_empty() {
        return Err("导出目录不能为空".to_string());
    }
    if !destination_dir.is_absolute() {
        return Err("导出目录必须是绝对路径".to_string());
    }
    let canonical = fs::canonicalize(destination_dir)
        .map_err(|error| format!("无法访问导出目录“{}”：{error}", destination_dir.display()))?;
    if !canonical.is_dir() {
        return Err(format!("导出目标不是文件夹：{}", destination_dir.display()));
    }

    // Use the user-selected absolute spelling for returned paths (and for Windows paths avoid
    // exposing the verbatim `\\?\` prefix produced by canonicalize). Canonicalization above still
    // verifies that the target exists and resolves to a directory.
    Ok(destination_dir.to_path_buf())
}

fn create_unique_destination(
    destination_dir: &Path,
    source_path: &Path,
) -> io::Result<(PathBuf, File)> {
    let file_name = source_path
        .file_name()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "source video has no file name"))?;
    let file_stem = source_path.file_stem().unwrap_or(file_name);
    let extension = source_path.extension();

    for attempt in 1..=MAX_COLLISION_SUFFIX {
        let candidate_name = if attempt == 1 {
            file_name.to_os_string()
        } else {
            suffixed_file_name(file_stem, extension, attempt)
        };
        let candidate_path = destination_dir.join(candidate_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate_path)
        {
            Ok(file) => return Ok((candidate_path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        "too many files share the same name",
    ))
}

fn suffixed_file_name(file_stem: &OsStr, extension: Option<&OsStr>, suffix: u32) -> OsString {
    let mut file_name = file_stem.to_os_string();
    file_name.push(format!(" ({suffix})"));
    if let Some(extension) = extension {
        file_name.push(".");
        file_name.push(extension);
    }
    file_name
}

fn push_failure(result: &mut ExportClipsResult, clip_id: i64, code: &'static str, message: String) {
    result.failures.push(ExportClipFailure {
        clip_id,
        code,
        message,
    });
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{self, Read},
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{copy_to_unique_destination, export_clips_for_database};
    use crate::db::{self, ClipInput, SourceDirInput};

    #[test]
    fn export_copies_each_unique_clip_without_overwriting_name_collisions() {
        let fixture = ExportFixture::new();
        let first_id = fixture.add_clip("source-a", "ace.mp4", Some(b"first video"));
        let second_id = fixture.add_clip("source-b", "ace.mp4", Some(b"second video"));
        fs::write(fixture.destination.join("ace.mp4"), b"keep existing")
            .expect("collision fixture should be writable");
        let missing_id = second_id + 10_000;

        let result = export_clips_for_database(
            &fixture.database_path,
            &[first_id, first_id, second_id, missing_id],
            &fixture.destination,
        )
        .expect("batch export should complete");

        assert_eq!(result.requested, 3);
        assert_eq!(result.exported, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.requested, result.exported + result.failed);
        assert_eq!(result.exported_ids.len(), result.exports.len());
        assert_eq!(result.exports.len(), result.exported);
        assert_eq!(result.exported_ids, vec![first_id, second_id]);
        assert_eq!(result.missing_ids, vec![missing_id]);
        assert_eq!(result.failures[0].code, "clip-not-found");
        assert_eq!(
            fs::read(fixture.destination.join("ace.mp4")).unwrap(),
            b"keep existing"
        );
        assert_eq!(
            fs::read(fixture.destination.join("ace (2).mp4")).unwrap(),
            b"first video"
        );
        assert_eq!(
            fs::read(fixture.destination.join("ace (3).mp4")).unwrap(),
            b"second video"
        );
        assert_eq!(result.exports[0].file_name, "ace (2).mp4");
        assert_eq!(result.exports[0].bytes_copied, 11);
    }

    #[test]
    fn export_reports_missing_files_and_rejects_paths_outside_the_indexed_source() {
        let fixture = ExportFixture::new();
        let missing_id = fixture.add_clip("source-a", "gone.mp4", None);
        let source_dir = fixture.root.join("source-b");
        let outside_dir = fixture.root.join("outside");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();
        let outside_path = outside_dir.join("escape.mp4");
        fs::write(&outside_path, b"do not export").unwrap();
        let unsafe_id = fixture.index_clip(&source_dir, &outside_path, "escape.mp4");

        let result = export_clips_for_database(
            &fixture.database_path,
            &[missing_id, unsafe_id],
            &fixture.destination,
        )
        .expect("item failures should be structured");

        assert_eq!(result.exported, 0);
        assert_eq!(result.failed, 2);
        assert_eq!(result.missing_file_ids, vec![missing_id]);
        assert_eq!(result.failures[0].code, "source-file-missing");
        assert_eq!(result.failures[1].code, "unsafe-source");
        assert!(result
            .missing_file_ids
            .iter()
            .all(|id| result.failures.iter().any(|failure| failure.clip_id == *id)));
        assert!(!fixture.destination.join("escape.mp4").exists());
    }

    #[test]
    fn copy_failure_removes_the_partially_written_destination() {
        let fixture = ExportFixture::new();
        let source_path = fixture.root.join("partial.mp4");
        let mut source = FailingReader { emitted: false };

        let failure = copy_to_unique_destination(&mut source, &fixture.destination, &source_path)
            .unwrap_err();

        assert_eq!(failure.code, "copy-failed");
        assert!(!fixture.destination.join("partial.mp4").exists());
    }

    #[test]
    fn export_result_uses_frontend_contract_names() {
        let fixture = ExportFixture::new();
        let clip_id = fixture.add_clip("source", "contract.mp4", Some(b"video"));
        let result =
            export_clips_for_database(&fixture.database_path, &[clip_id], &fixture.destination)
                .unwrap();
        let json = serde_json::to_value(result).unwrap();

        assert_eq!(json["requested"], 1);
        assert_eq!(json["exported"], 1);
        assert_eq!(json["failed"], 0);
        assert!(json.get("destinationDir").is_some());
        assert_eq!(json["exportedIds"][0], clip_id);
        assert_eq!(json["exports"][0]["clipId"], clip_id);
        assert_eq!(json["exports"][0]["fileName"], "contract.mp4");
        assert_eq!(json["exports"][0]["bytesCopied"], 5);
    }

    #[test]
    fn export_rejects_a_missing_or_relative_destination_directory() {
        let fixture = ExportFixture::new();
        let clip_id = fixture.add_clip("source", "clip.mp4", Some(b"video"));

        let relative_error = export_clips_for_database(
            &fixture.database_path,
            &[clip_id],
            Path::new("relative-export"),
        )
        .unwrap_err();
        let missing_error = export_clips_for_database(
            &fixture.database_path,
            &[clip_id],
            fixture.root.join("does-not-exist"),
        )
        .unwrap_err();

        assert!(relative_error.contains("绝对路径"));
        assert!(missing_error.contains("无法访问导出目录"));
    }

    struct ExportFixture {
        root: PathBuf,
        database_path: PathBuf,
        destination: PathBuf,
    }

    impl ExportFixture {
        fn new() -> Self {
            let root = unique_temp_dir();
            let destination = root.join("destination");
            fs::create_dir_all(&destination).expect("destination should be created");
            let database_path = root.join("highlight-index.sqlite3");
            db::migrate_database(&database_path).expect("database should migrate");
            Self {
                root,
                database_path,
                destination,
            }
        }

        fn add_clip(&self, source_name: &str, file_name: &str, bytes: Option<&[u8]>) -> i64 {
            let source_dir = self.root.join(source_name);
            fs::create_dir_all(&source_dir).expect("source should be created");
            let clip_path = source_dir.join(file_name);
            if let Some(bytes) = bytes {
                fs::write(&clip_path, bytes).expect("clip should be written");
            }
            self.index_clip(&source_dir, &clip_path, file_name)
        }

        fn index_clip(&self, source_dir: &Path, clip_path: &Path, file_name: &str) -> i64 {
            let connection = db::open_database(&self.database_path).expect("database should open");
            let source = db::upsert_source_dir(
                &connection,
                SourceDirInput {
                    path: source_dir.to_string_lossy().as_ref(),
                    name: "Fixture",
                },
            )
            .expect("source should upsert");
            db::upsert_clip(
                &connection,
                ClipInput {
                    source_dir_id: source.id,
                    clip_group_id: None,
                    video_path: clip_path.to_string_lossy().as_ref(),
                    file_name,
                    file_size: 5,
                    modified_at: None,
                    duration_ms: None,
                    recorded_at: None,
                    cover_path: None,
                    cover_source: "missing",
                },
            )
            .expect("clip should upsert")
            .id
        }
    }

    impl Drop for ExportFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn unique_temp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("vhm-export-test-{}-{unique}", std::process::id()))
    }

    struct FailingReader {
        emitted: bool,
    }

    impl Read for FailingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.emitted {
                return Err(io::Error::other("fixture read failure"));
            }
            self.emitted = true;
            let bytes = b"partial";
            buffer[..bytes.len()].copy_from_slice(bytes);
            Ok(bytes.len())
        }
    }
}
