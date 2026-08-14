//! Persistent, single-worker thumbnail generation and cache management.
//!
//! Source videos and source-provided covers are always treated as read-only. The only writable
//! paths used by this module live directly below the application thumbnail cache directory.

use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::db;

pub(crate) const THUMBNAIL_EVENT_NAME: &str = "thumbnail-progress";
pub(crate) const MAX_THUMBNAIL_COMMAND_IDS: usize = 200;
pub(crate) const MAX_THUMBNAIL_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const CACHE_HIGH_WATER_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const CACHE_LOW_WATER_BYTES: u64 = 450 * 1024 * 1024;

const FFMPEG_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const STALE_PART_AGE: Duration = Duration::from_secs(60 * 60);
const IDLE_QUEUE_POLL: Duration = Duration::from_secs(5);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GeneratorAvailability {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThumbnailGenerationError {
    pub(crate) code: &'static str,
    pub(crate) retryable: bool,
}

impl ThumbnailGenerationError {
    fn cancelled() -> Self {
        Self {
            code: "thumbnail-cancelled",
            retryable: true,
        }
    }

    fn timeout() -> Self {
        Self {
            code: "ffmpeg-timeout",
            retryable: true,
        }
    }
}

pub(crate) trait ThumbnailGenerator: Send + Sync + 'static {
    fn availability(&self) -> GeneratorAvailability;

    fn unavailable_error_code(&self) -> Option<&'static str>;

    fn generate(
        &self,
        source_path: &Path,
        temporary_output: &Path,
        cancellation: &AtomicBool,
    ) -> Result<u64, ThumbnailGenerationError>;

    fn cancel(&self);
}

/// FFmpeg-backed generator that never consults PATH and never invokes a shell.
pub(crate) struct FfmpegThumbnailGenerator {
    executable: Option<PathBuf>,
    unavailable_error_code: Option<&'static str>,
    current_child: Mutex<Option<Child>>,
    timeout: Duration,
}

impl FfmpegThumbnailGenerator {
    pub(crate) fn resolve(resource_dir: &Path) -> Self {
        let (executable, error_code) = resolve_ffmpeg_executable(
            std::env::var_os("VHM_FFMPEG_PATH"),
            resource_dir,
            ffmpeg_file_name(),
        );
        Self {
            executable,
            unavailable_error_code: error_code,
            current_child: Mutex::new(None),
            timeout: FFMPEG_TIMEOUT,
        }
    }

    fn run_attempt(
        &self,
        source_path: &Path,
        temporary_output: &Path,
        seek_seconds: &str,
        cancellation: &AtomicBool,
    ) -> Result<u64, ThumbnailGenerationError> {
        let Some(executable) = self.executable.as_deref() else {
            return Err(ThumbnailGenerationError {
                code: self.unavailable_error_code.unwrap_or("ffmpeg-unavailable"),
                retryable: false,
            });
        };

        remove_cache_file_if_present(temporary_output).map_err(|_| ThumbnailGenerationError {
            code: "thumbnail-temp-cleanup-failed",
            retryable: true,
        })?;

        let mut command = Command::new(executable);
        command
            .arg("-nostdin")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-ss")
            .arg(seek_seconds)
            .arg("-i")
            .arg(source_path)
            .arg("-frames:v")
            .arg("1")
            .arg("-vf")
            .arg("scale=480:-2:force_original_aspect_ratio=decrease")
            .arg("-q:v")
            .arg("5")
            .arg(temporary_output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        let child = command.spawn().map_err(|error| ThumbnailGenerationError {
            code: if error.kind() == std::io::ErrorKind::NotFound {
                "ffmpeg-unavailable"
            } else {
                "ffmpeg-spawn-failed"
            },
            retryable: error.kind() != std::io::ErrorKind::NotFound,
        })?;
        *self
            .current_child
            .lock()
            .expect("ffmpeg child lock poisoned") = Some(child);

        let started_at = Instant::now();
        let status = loop {
            if cancellation.load(Ordering::Acquire) {
                self.terminate_current_child();
                return Err(ThumbnailGenerationError::cancelled());
            }
            if started_at.elapsed() >= self.timeout {
                self.terminate_current_child();
                return Err(ThumbnailGenerationError::timeout());
            }

            let poll_result = {
                let mut child = self
                    .current_child
                    .lock()
                    .expect("ffmpeg child lock poisoned");
                let Some(child) = child.as_mut() else {
                    return Err(ThumbnailGenerationError::cancelled());
                };
                child.try_wait()
            };
            match poll_result {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(_) => {
                    self.terminate_current_child();
                    return Err(ThumbnailGenerationError {
                        code: "ffmpeg-wait-failed",
                        retryable: true,
                    });
                }
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        };
        self.current_child
            .lock()
            .expect("ffmpeg child lock poisoned")
            .take();

        if !status.success() {
            return Err(ThumbnailGenerationError {
                code: "ffmpeg-exit-failed",
                retryable: true,
            });
        }

        validate_generated_jpeg(temporary_output)
    }

    fn terminate_current_child(&self) {
        let mut current = self
            .current_child
            .lock()
            .expect("ffmpeg child lock poisoned");
        if let Some(child) = current.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        current.take();
    }
}

impl ThumbnailGenerator for FfmpegThumbnailGenerator {
    fn availability(&self) -> GeneratorAvailability {
        if self.executable.is_some() {
            GeneratorAvailability::Available
        } else {
            GeneratorAvailability::Unavailable
        }
    }

    fn unavailable_error_code(&self) -> Option<&'static str> {
        self.unavailable_error_code
    }

    fn generate(
        &self,
        source_path: &Path,
        temporary_output: &Path,
        cancellation: &AtomicBool,
    ) -> Result<u64, ThumbnailGenerationError> {
        let first = self.run_attempt(source_path, temporary_output, "0.5", cancellation);
        match first {
            Ok(size) => Ok(size),
            Err(error)
                if error.retryable
                    && error.code != "ffmpeg-timeout"
                    && error.code != "thumbnail-cancelled" =>
            {
                self.run_attempt(source_path, temporary_output, "0", cancellation)
            }
            Err(error) => Err(error),
        }
    }

    fn cancel(&self) {
        self.terminate_current_child();
    }
}

fn ffmpeg_file_name() -> &'static OsStr {
    #[cfg(windows)]
    {
        OsStr::new("ffmpeg.exe")
    }
    #[cfg(not(windows))]
    {
        OsStr::new("ffmpeg")
    }
}

fn resolve_ffmpeg_executable(
    configured: Option<OsString>,
    resource_dir: &Path,
    executable_name: &OsStr,
) -> (Option<PathBuf>, Option<&'static str>) {
    if let Some(configured) = configured {
        let configured = PathBuf::from(configured);
        if !configured.is_absolute() {
            return (None, Some("ffmpeg-path-not-absolute"));
        }
        return canonical_regular_file(&configured)
            .map(|path| (Some(path), None))
            .unwrap_or((None, Some("ffmpeg-unavailable")));
    }

    let bundled = resource_dir.join("bin").join(executable_name);
    canonical_regular_file(&bundled)
        .map(|path| (Some(path), None))
        .unwrap_or((None, Some("ffmpeg-unavailable")))
}

fn canonical_regular_file(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    fs::canonicalize(path).ok()
}

pub(crate) fn prepare_cache_root(cache_root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(cache_root)
        .map_err(|error| format!("cannot create thumbnail cache directory: {error}"))?;
    fs::canonicalize(cache_root)
        .map_err(|error| format!("cannot resolve thumbnail cache directory: {error}"))
}

pub(crate) fn cache_file_name(clip_id: i64, fingerprint: &str) -> Result<String, String> {
    if clip_id <= 0 || !valid_fingerprint(fingerprint) {
        return Err("invalid thumbnail cache identity".to_string());
    }
    Ok(format!("{clip_id}-{fingerprint}.jpg"))
}

pub(crate) fn resolve_ready_cache_file(
    cache_root: &Path,
    cache_file: &str,
) -> Result<PathBuf, String> {
    validate_cache_basename(cache_file)?;
    let root =
        fs::canonicalize(cache_root).map_err(|_| "thumbnail-cache-unavailable".to_string())?;
    let candidate = root.join(cache_file);
    let metadata =
        fs::symlink_metadata(&candidate).map_err(|_| "thumbnail-cache-file-missing".to_string())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("thumbnail-cache-file-invalid".to_string());
    }
    let canonical =
        fs::canonicalize(&candidate).map_err(|_| "thumbnail-cache-file-invalid".to_string())?;
    if canonical.parent() != Some(root.as_path()) {
        return Err("thumbnail-cache-path-escape".to_string());
    }
    Ok(canonical)
}

pub(crate) fn validate_cache_basename(cache_file: &str) -> Result<(), String> {
    let path = Path::new(cache_file);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err("invalid thumbnail cache basename".to_string());
    }
    let Some(stem) = cache_file.strip_suffix(".jpg") else {
        return Err("invalid thumbnail cache extension".to_string());
    };
    let Some((clip_id, fingerprint)) = stem.split_once('-') else {
        return Err("invalid thumbnail cache basename".to_string());
    };
    if clip_id.is_empty()
        || !clip_id.bytes().all(|byte| byte.is_ascii_digit())
        || clip_id.parse::<i64>().ok().filter(|id| *id > 0).is_none()
        || !valid_fingerprint(fingerprint)
    {
        return Err("invalid thumbnail cache basename".to_string());
    }
    Ok(())
}

fn valid_fingerprint(fingerprint: &str) -> bool {
    fingerprint.len() == 64
        && fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(crate) fn temporary_cache_file(cache_root: &Path, clip_id: i64) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    cache_root.join(format!(
        ".part-{clip_id}-{}-{nonce}.jpg",
        std::process::id()
    ))
}

pub(crate) fn validate_generated_jpeg(path: &Path) -> Result<u64, ThumbnailGenerationError> {
    let mut file = File::open(path).map_err(|_| ThumbnailGenerationError {
        code: "thumbnail-output-missing",
        retryable: true,
    })?;
    let size = file
        .metadata()
        .map_err(|_| ThumbnailGenerationError {
            code: "thumbnail-output-invalid",
            retryable: true,
        })?
        .len();
    if !(5..=MAX_THUMBNAIL_BYTES).contains(&size) {
        return Err(ThumbnailGenerationError {
            code: "thumbnail-output-size-invalid",
            retryable: true,
        });
    }

    let mut prefix = [0_u8; 3];
    file.read_exact(&mut prefix)
        .map_err(|_| ThumbnailGenerationError {
            code: "thumbnail-output-invalid",
            retryable: true,
        })?;
    let mut suffix = [0_u8; 2];
    file.seek(SeekFrom::End(-2))
        .and_then(|_| file.read_exact(&mut suffix))
        .map_err(|_| ThumbnailGenerationError {
            code: "thumbnail-output-invalid",
            retryable: true,
        })?;
    if prefix != [0xff, 0xd8, 0xff] || suffix != [0xff, 0xd9] {
        return Err(ThumbnailGenerationError {
            code: "thumbnail-output-not-jpeg",
            retryable: true,
        });
    }
    Ok(size)
}

pub(crate) fn atomic_install_cache_file(
    temporary_output: &Path,
    final_output: &Path,
) -> Result<(), String> {
    let temporary_parent = temporary_output.parent();
    let final_parent = final_output.parent();
    if temporary_parent.is_none() || temporary_parent != final_parent {
        return Err("thumbnail cache rename crossed directories".to_string());
    }
    if final_output.exists() {
        validate_generated_jpeg(final_output)
            .map(|_| ())
            .map_err(|error| error.code.to_string())?;
        remove_cache_file_if_present(temporary_output)
            .map_err(|error| format!("cannot remove duplicate thumbnail temp file: {error}"))?;
        return Ok(());
    }
    fs::rename(temporary_output, final_output)
        .map_err(|error| format!("cannot install thumbnail cache file: {error}"))
}

pub(crate) fn remove_cache_file_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Removes stale worker temp files. It never recurses and never follows links or directories.
pub(crate) fn cleanup_stale_parts(cache_root: &Path, now: SystemTime) -> Result<usize, String> {
    let mut removed = 0;
    let entries = fs::read_dir(cache_root)
        .map_err(|error| format!("cannot enumerate thumbnail cache: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot inspect thumbnail cache: {error}"))?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(".part-") || !file_name.ends_with(".jpg") {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("cannot inspect thumbnail cache entry: {error}"))?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_PART_AGE);
        if stale {
            remove_cache_file_if_present(&entry.path())
                .map_err(|error| format!("cannot remove stale thumbnail temp file: {error}"))?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GeneratorStatus {
    /// Reserved by the public wire contract for clients observing service initialization.
    #[allow(dead_code)]
    Unknown,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThumbnailProgressEvent {
    pub(crate) clip_id: i64,
    pub(crate) status: String,
    pub(crate) revision: Option<String>,
    pub(crate) error_code: Option<String>,
}

impl From<db::ThumbnailStatus> for ThumbnailProgressEvent {
    fn from(value: db::ThumbnailStatus) -> Self {
        Self {
            clip_id: value.clip_id,
            status: value.status,
            revision: value.revision,
            error_code: value.error_code,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThumbnailServiceStatus {
    pub(crate) generator_status: GeneratorStatus,
    pub(crate) pending_count: i64,
    pub(crate) running_count: i64,
    pub(crate) ready_count: i64,
    pub(crate) failed_count: i64,
    pub(crate) unavailable_count: i64,
    pub(crate) evicted_count: i64,
    pub(crate) cache_bytes: i64,
    pub(crate) processing_clip_id: Option<i64>,
    pub(crate) last_error_code: Option<String>,
}

type ThumbnailEventSink = Arc<dyn Fn(ThumbnailProgressEvent) + Send + Sync>;

/// Persistent queue supervisor. The channel carries only a coalesced wake token; all work items
/// remain in SQLite, so queue memory is bounded regardless of library size.
pub(crate) struct ThumbnailQueue {
    database_path: PathBuf,
    cache_root: PathBuf,
    generator: Arc<dyn ThumbnailGenerator>,
    event_sink: ThumbnailEventSink,
    wake_sender: SyncSender<()>,
    reconcile_requested: AtomicBool,
    shutdown_requested: AtomicBool,
    processing_clip_id: AtomicI64,
    last_error_code: Mutex<Option<String>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    worker_done: Mutex<Option<Receiver<()>>>,
    #[cfg(test)]
    reconcile_count: AtomicUsize,
}

impl ThumbnailQueue {
    pub(crate) fn start(
        app: AppHandle,
        database_path: PathBuf,
        cache_root: PathBuf,
        generator: Arc<dyn ThumbnailGenerator>,
    ) -> Result<Arc<Self>, String> {
        let event_app = app.clone();
        Self::start_with_sink(
            database_path,
            cache_root,
            generator,
            Arc::new(move |event| {
                let _ = event_app.emit(THUMBNAIL_EVENT_NAME, event);
            }),
        )
    }

    fn start_with_sink(
        database_path: PathBuf,
        cache_root: PathBuf,
        generator: Arc<dyn ThumbnailGenerator>,
        event_sink: ThumbnailEventSink,
    ) -> Result<Arc<Self>, String> {
        let cache_root = prepare_cache_root(&cache_root)?;
        let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let queue = Arc::new(Self {
            database_path,
            cache_root,
            generator,
            event_sink,
            wake_sender,
            reconcile_requested: AtomicBool::new(false),
            shutdown_requested: AtomicBool::new(false),
            processing_clip_id: AtomicI64::new(-1),
            last_error_code: Mutex::new(None),
            worker: Mutex::new(None),
            worker_done: Mutex::new(Some(done_receiver)),
            #[cfg(test)]
            reconcile_count: AtomicUsize::new(0),
        });
        let weak_queue = Arc::downgrade(&queue);
        let worker = thread::Builder::new()
            .name("vhm-thumbnail-worker".to_string())
            .spawn(move || {
                if let Some(queue) = weak_queue.upgrade() {
                    queue.worker_loop(wake_receiver);
                }
                let _ = done_sender.try_send(());
            })
            .map_err(|error| format!("cannot start thumbnail worker: {error}"))?;
        *queue.worker.lock().expect("thumbnail worker lock poisoned") = Some(worker);
        queue.reconcile_and_wake();
        Ok(queue)
    }

    pub(crate) fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub(crate) fn reconcile_and_wake(&self) {
        self.reconcile_requested.store(true, Ordering::Release);
        self.wake();
    }

    fn wake(&self) {
        let _ = self.wake_sender.try_send(());
    }

    pub(crate) fn ensure_clip_thumbnails(
        &self,
        clip_ids: &[i64],
    ) -> Result<db::ThumbnailEnsureResult, String> {
        validate_command_clip_ids(clip_ids)?;
        let connection = db::open_database(&self.database_path)?;
        let result = db::ensure_clip_thumbnails(&connection, clip_ids)?;
        drop(connection);
        self.emit_statuses(result.changed);
        self.wake();
        Ok(result.counts)
    }

    pub(crate) fn retry_clip_thumbnails(
        &self,
        clip_ids: &[i64],
    ) -> Result<db::ThumbnailEnsureResult, String> {
        validate_command_clip_ids(clip_ids)?;
        let connection = db::open_database(&self.database_path)?;
        let result = db::retry_clip_thumbnails(&connection, clip_ids)?;
        drop(connection);
        self.emit_statuses(result.changed);
        self.wake();
        Ok(result.counts)
    }

    pub(crate) fn status(&self) -> Result<ThumbnailServiceStatus, String> {
        let connection = db::open_database_read_only(&self.database_path)?;
        let persisted = db::get_thumbnail_queue_status(&connection)?;
        let processing_clip_id = match self.processing_clip_id.load(Ordering::Acquire) {
            value if value > 0 => Some(value),
            _ => None,
        };
        Ok(ThumbnailServiceStatus {
            generator_status: self.generator_status(),
            pending_count: persisted.pending,
            running_count: persisted.running,
            ready_count: persisted.ready,
            failed_count: persisted.failed,
            unavailable_count: persisted.unavailable,
            evicted_count: persisted.evicted,
            cache_bytes: persisted.cache_bytes,
            processing_clip_id,
            last_error_code: self
                .last_error_code
                .lock()
                .expect("thumbnail error lock poisoned")
                .clone(),
        })
    }

    pub(crate) fn shutdown(&self) {
        if self.shutdown_requested.swap(true, Ordering::AcqRel) {
            return;
        }
        self.generator.cancel();
        self.wake();

        let completed = self
            .worker_done
            .lock()
            .expect("thumbnail done lock poisoned")
            .take()
            .is_some_and(|done| done.recv_timeout(WORKER_SHUTDOWN_TIMEOUT).is_ok());
        let worker = self
            .worker
            .lock()
            .expect("thumbnail worker lock poisoned")
            .take();
        if completed {
            if let Some(worker) = worker {
                let _ = worker.join();
            }
        }
    }

    fn worker_loop(&self, wake_receiver: Receiver<()>) {
        if let Err(error) = self.recover_interrupted_jobs() {
            self.record_error(&error);
        }
        if let Err(error) = self.maintain_cache() {
            self.record_error(&error);
        }

        loop {
            if self.shutdown_requested.load(Ordering::Acquire) {
                break;
            }
            let received_wake = match wake_receiver.recv_timeout(IDLE_QUEUE_POLL) {
                Ok(()) => {
                    loop {
                        match wake_receiver.try_recv() {
                            Ok(()) => continue,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    true
                }
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if self.shutdown_requested.load(Ordering::Acquire) {
                break;
            }

            let reconcile = received_wake && self.reconcile_requested.swap(false, Ordering::AcqRel);
            if reconcile {
                if let Err(error) = self.reconcile_all() {
                    self.record_error(&error);
                    continue;
                }
                if let Err(error) = self.maintain_cache() {
                    self.record_error(&error);
                }
            }
            if self.generator.availability() == GeneratorAvailability::Unavailable {
                if let Err(error) = self.persist_global_unavailable() {
                    self.record_error(&error);
                }
                continue;
            }

            if let Err(error) = self.process_due_jobs() {
                self.record_error(&error);
            }
            if self.shutdown_requested.load(Ordering::Acquire) {
                break;
            }
            if reconcile {
                if let Err(error) = self.maintain_cache() {
                    self.record_error(&error);
                }
            }
        }

        self.processing_clip_id.store(-1, Ordering::Release);
        self.generator.cancel();
        if let Ok(connection) = db::open_database(&self.database_path) {
            let _ = db::recover_running_thumbnail_jobs(&connection);
        }
    }

    fn recover_interrupted_jobs(&self) -> Result<(), String> {
        let connection = db::open_database(&self.database_path)?;
        db::delete_orphan_thumbnail_rows(&connection)?;
        db::recover_running_thumbnail_jobs(&connection)?;
        if self.generator.availability() == GeneratorAvailability::Available {
            db::recover_unavailable_thumbnail_jobs(&connection)?;
        }
        Ok(())
    }

    fn reconcile_all(&self) -> Result<(), String> {
        #[cfg(test)]
        self.reconcile_count.fetch_add(1, Ordering::AcqRel);
        let connection = db::open_database(&self.database_path)?;
        // Startup/scan reconciliation may touch 10k+ clips. Those callers already refresh their
        // page/status snapshots, so emitting every pending/suppressed row would create an IPC
        // event storm. Per-item worker transitions and selected (<=200) commands still emit.
        db::reconcile_clip_thumbnails(&connection, None, false)?;
        Ok(())
    }

    fn persist_global_unavailable(&self) -> Result<(), String> {
        let error_code = self
            .generator
            .unavailable_error_code()
            .unwrap_or("ffmpeg-unavailable");
        let connection = db::open_database(&self.database_path)?;
        db::mark_pending_thumbnails_unavailable(&connection, error_code)?;
        self.set_last_error(Some(error_code.to_string()));
        Ok(())
    }

    fn process_due_jobs(&self) -> Result<(), String> {
        // A previous iteration can only leave a row running after an unexpected database/cache
        // infrastructure error. Single-worker recovery makes that claim eligible again instead
        // of requiring an application restart.
        let recovery_connection = db::open_database(&self.database_path)?;
        db::recover_running_thumbnail_jobs(&recovery_connection)?;
        let persisted_cache_bytes = db::get_thumbnail_queue_status(&recovery_connection)?
            .cache_bytes
            .max(0) as u64;
        drop(recovery_connection);
        let mut cache_bytes = persisted_cache_bytes;
        if cache_bytes > CACHE_HIGH_WATER_BYTES {
            self.maintain_cache()?;
            let connection = db::open_database_read_only(&self.database_path)?;
            cache_bytes = db::get_thumbnail_queue_status(&connection)?
                .cache_bytes
                .max(0) as u64;
        }
        loop {
            if self.shutdown_requested.load(Ordering::Acquire) {
                return Ok(());
            }
            let connection = db::open_database(&self.database_path)?;
            let now = sqlite_now(&connection)?;
            let Some(job) = db::claim_next_thumbnail_job(&connection, &now)? else {
                return Ok(());
            };
            drop(connection);

            self.processing_clip_id
                .store(job.clip_id, Ordering::Release);
            self.emit(ThumbnailProgressEvent {
                clip_id: job.clip_id,
                status: "running".to_string(),
                revision: None,
                error_code: None,
            });
            let result = self.process_job(&job);
            self.processing_clip_id.store(-1, Ordering::Release);
            let added_bytes = match result {
                Ok(added_bytes) => added_bytes,
                Err(error) => {
                    if self
                        .persist_job_failure(&job, "thumbnail-service-error", true)
                        .is_err()
                    {
                        if let Ok(connection) = db::open_database(&self.database_path) {
                            let _ = db::recover_running_thumbnail_jobs(&connection);
                        }
                    }
                    return Err(error);
                }
            };
            cache_bytes = cache_bytes.saturating_add(added_bytes);
            // Full filesystem repair is intentionally not run for every item. We track the
            // committed ready-byte delta and only enumerate ready files when the high-water mark
            // is crossed, keeping 10k-item queues linear while still enforcing the disk budget.
            if cache_bytes > CACHE_HIGH_WATER_BYTES {
                self.maintain_cache()?;
                let connection = db::open_database_read_only(&self.database_path)?;
                cache_bytes = db::get_thumbnail_queue_status(&connection)?
                    .cache_bytes
                    .max(0) as u64;
            }
        }
    }

    fn process_job(&self, job: &db::ThumbnailJob) -> Result<u64, String> {
        let source_path = Path::new(&job.video_path);
        if !source_path.is_file() {
            return self
                .persist_job_failure(job, "source-file-missing", false)
                .map(|_| 0);
        }
        let cache_file = cache_file_name(job.clip_id, &job.fingerprint)?;
        let final_output = self.cache_root.join(&cache_file);
        let mut installed_new_file = false;

        let existing_cache_size = resolve_ready_cache_file(&self.cache_root, &cache_file)
            .ok()
            .and_then(|path| validate_generated_jpeg(&path).ok());
        let byte_size = match existing_cache_size {
            Some(size) => size,
            None => {
                remove_cache_file_if_present(&final_output).map_err(|error| {
                    format!("cannot remove invalid thumbnail cache file: {error}")
                })?;
                let temporary_output = temporary_cache_file(&self.cache_root, job.clip_id);
                let generated = self.generator.generate(
                    source_path,
                    &temporary_output,
                    &self.shutdown_requested,
                );
                match generated {
                    Ok(size) => {
                        if let Err(error) =
                            atomic_install_cache_file(&temporary_output, &final_output)
                        {
                            let _ = remove_cache_file_if_present(&temporary_output);
                            return self
                                .persist_job_failure(job, stable_cache_error_code(&error), true)
                                .map(|_| 0);
                        }
                        installed_new_file = true;
                        size
                    }
                    Err(error) => {
                        let _ = remove_cache_file_if_present(&temporary_output);
                        if error.code == "thumbnail-cancelled"
                            && self.shutdown_requested.load(Ordering::Acquire)
                        {
                            return Ok(0);
                        }
                        return self
                            .persist_job_failure(job, error.code, error.retryable)
                            .map(|_| 0);
                    }
                }
            }
        };

        let committed_bytes = byte_size;
        let byte_size = i64::try_from(committed_bytes)
            .map_err(|_| "thumbnail byte size exceeds database range".to_string())?;
        // The row leaves the running state inside complete_thumbnail_job_if_current
        // or the reconcile fallbacks below. Clear the live processing hint first so
        // ready_count == total can never be observed together with a processing clip.
        self.processing_clip_id.store(-1, Ordering::Release);
        let connection = db::open_database(&self.database_path)?;
        let completed = db::complete_thumbnail_job_if_current(
            &connection,
            job,
            &cache_file,
            byte_size,
            &job.fingerprint,
        )?;
        if completed {
            self.set_last_error(None);
            self.emit(ThumbnailProgressEvent {
                clip_id: job.clip_id,
                status: "ready".to_string(),
                revision: Some(job.fingerprint.clone()),
                error_code: None,
            });
        } else if installed_new_file {
            let _ = remove_cache_file_if_present(&final_output);
            let reconciled = db::reconcile_clip_thumbnails(
                &connection,
                Some(std::slice::from_ref(&job.clip_id)),
                false,
            )?;
            drop(connection);
            self.emit_statuses(reconciled.changed);
            return Ok(0);
        } else {
            let reconciled = db::reconcile_clip_thumbnails(
                &connection,
                Some(std::slice::from_ref(&job.clip_id)),
                false,
            )?;
            drop(connection);
            self.emit_statuses(reconciled.changed);
            return Ok(0);
        }
        Ok(committed_bytes)
    }

    fn persist_job_failure(
        &self,
        job: &db::ThumbnailJob,
        error_code: &'static str,
        retryable: bool,
    ) -> Result<(), String> {
        // The row leaves the running state inside this call. Clear the live
        // processing hint first so persisted counts and processing_clip_id can
        // never be observed inconsistent with each other.
        self.processing_clip_id.store(-1, Ordering::Release);
        let connection = db::open_database(&self.database_path)?;
        let (status, next_attempt_at) =
            if let Some(modifier) = retry_delay_modifier(job.attempt_count, retryable) {
                ("pending", Some(sqlite_future(&connection, modifier)?))
            } else {
                ("failed", None)
            };
        let changed = db::fail_thumbnail_job_if_current(
            &connection,
            job.clip_id,
            &job.fingerprint,
            status,
            error_code,
            None,
            next_attempt_at.as_deref(),
        )?;
        if changed {
            self.set_last_error(Some(error_code.to_string()));
            self.emit(ThumbnailProgressEvent {
                clip_id: job.clip_id,
                status: status.to_string(),
                revision: None,
                error_code: Some(error_code.to_string()),
            });
        }
        Ok(())
    }

    fn maintain_cache(&self) -> Result<(), String> {
        self.maintain_cache_with_limits(CACHE_HIGH_WATER_BYTES, CACHE_LOW_WATER_BYTES)
    }

    fn maintain_cache_with_limits(&self, high_water: u64, low_water: u64) -> Result<(), String> {
        self.maintain_cache_with_limits_using(high_water, low_water, remove_cache_file_if_present)
    }

    fn maintain_cache_with_limits_using<F>(
        &self,
        high_water: u64,
        low_water: u64,
        remove_evicted_file: F,
    ) -> Result<(), String>
    where
        F: Fn(&Path) -> std::io::Result<()>,
    {
        if low_water > high_water {
            return Err("thumbnail cache low-water mark exceeds high-water mark".to_string());
        }
        cleanup_stale_parts(&self.cache_root, SystemTime::now())?;
        let connection = db::open_database(&self.database_path)?;
        db::delete_orphan_thumbnail_rows(&connection)?;
        let ready = db::list_ready_thumbnail_cache_refs(&connection)?;
        let referenced = ready
            .iter()
            .map(|item| item.cache_file.as_str())
            .collect::<HashSet<_>>();

        // Only delete files with our strict generated-cache naming scheme. Unknown files,
        // directories, links and source paths are never touched.
        for entry in fs::read_dir(&self.cache_root)
            .map_err(|error| format!("cannot enumerate thumbnail cache: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("cannot inspect thumbnail cache: {error}"))?;
            let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if validate_cache_basename(&file_name).is_err()
                || referenced.contains(file_name.as_str())
            {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("cannot inspect thumbnail cache entry: {error}"))?;
            if metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
                remove_cache_file_if_present(&entry.path())
                    .map_err(|error| format!("cannot remove orphan thumbnail cache: {error}"))?;
            }
        }

        let mut actual_ready = Vec::new();
        let mut total_bytes = 0_u64;
        for item in ready {
            match resolve_ready_cache_file(&self.cache_root, &item.cache_file) {
                Ok(path) => match validate_generated_jpeg(&path) {
                    Ok(size) => {
                        total_bytes = total_bytes.saturating_add(size);
                        actual_ready.push((item, path, size));
                    }
                    Err(_) => {
                        if db::mark_thumbnail_cache_missing_if_current(
                            &connection,
                            item.clip_id,
                            &item.revision,
                        )? {
                            let _ = remove_cache_file_if_present(&path);
                            self.emit(ThumbnailProgressEvent {
                                clip_id: item.clip_id,
                                status: "pending".to_string(),
                                revision: None,
                                error_code: Some("thumbnail-cache-invalid".to_string()),
                            });
                        }
                    }
                },
                Err(_) => {
                    if db::mark_thumbnail_cache_missing_if_current(
                        &connection,
                        item.clip_id,
                        &item.revision,
                    )? {
                        self.emit(ThumbnailProgressEvent {
                            clip_id: item.clip_id,
                            status: "pending".to_string(),
                            revision: None,
                            error_code: Some("thumbnail-cache-missing".to_string()),
                        });
                    }
                }
            }
        }

        if total_bytes > high_water {
            for (item, path, size) in actual_ready {
                if total_bytes <= low_water {
                    break;
                }
                // On Windows an image response may temporarily hold the file open. Keep the DB
                // ready reference when deletion fails and continue with later candidates; this
                // preserves consistency while still making progress toward the low-water mark.
                if remove_evicted_file(&path).is_err() {
                    continue;
                }
                total_bytes = total_bytes.saturating_sub(size);
                if db::mark_thumbnail_evicted_if_current(&connection, item.clip_id, &item.revision)?
                {
                    self.emit(ThumbnailProgressEvent {
                        clip_id: item.clip_id,
                        status: "evicted".to_string(),
                        revision: None,
                        error_code: None,
                    });
                }
            }
        }
        Ok(())
    }

    fn generator_status(&self) -> GeneratorStatus {
        match self.generator.availability() {
            GeneratorAvailability::Available => GeneratorStatus::Available,
            GeneratorAvailability::Unavailable => GeneratorStatus::Unavailable,
        }
    }

    fn emit_statuses(&self, statuses: Vec<db::ThumbnailStatus>) {
        for status in statuses {
            self.emit(status.into());
        }
    }

    fn emit(&self, event: ThumbnailProgressEvent) {
        (self.event_sink)(event);
    }

    fn record_error(&self, _message: &str) {
        self.set_last_error(Some("thumbnail-service-error".to_string()));
    }

    fn set_last_error(&self, value: Option<String>) {
        *self
            .last_error_code
            .lock()
            .expect("thumbnail error lock poisoned") = value;
    }
}

impl Drop for ThumbnailQueue {
    fn drop(&mut self) {
        self.shutdown_requested.store(true, Ordering::Release);
        self.generator.cancel();
        let _ = self.wake_sender.try_send(());
    }
}

fn validate_command_clip_ids(clip_ids: &[i64]) -> Result<(), String> {
    if clip_ids.len() > MAX_THUMBNAIL_COMMAND_IDS {
        return Err(format!(
            "thumbnail commands accept at most {MAX_THUMBNAIL_COMMAND_IDS} clip ids"
        ));
    }
    let unique = clip_ids.iter().copied().collect::<HashSet<_>>();
    if unique.iter().any(|clip_id| *clip_id <= 0) {
        return Err("thumbnail clip ids must be positive".to_string());
    }
    Ok(())
}

fn sqlite_now(connection: &rusqlite::Connection) -> Result<String, String> {
    connection
        .query_row("SELECT CURRENT_TIMESTAMP", [], |row| row.get(0))
        .map_err(|error| format!("reading thumbnail queue time: {error}"))
}

fn sqlite_future(connection: &rusqlite::Connection, modifier: &str) -> Result<String, String> {
    connection
        .query_row("SELECT datetime('now', ?1)", [modifier], |row| row.get(0))
        .map_err(|error| format!("reading thumbnail retry time: {error}"))
}

fn stable_cache_error_code(error: &str) -> &'static str {
    if error.contains("rename") || error.contains("install") {
        "thumbnail-cache-install-failed"
    } else {
        "thumbnail-cache-failed"
    }
}

fn retry_delay_modifier(attempt_count: i64, retryable: bool) -> Option<&'static str> {
    if !retryable {
        return None;
    }
    match attempt_count {
        0 | 1 => Some("+1 minute"),
        2 => Some("+10 minutes"),
        3 => Some("+1 hour"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    static NEXT_TEMP_DIR_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn ffmpeg_resolution_never_accepts_path_search_or_relative_override() {
        let root = unique_temp_dir();
        fs::create_dir_all(root.join("bin")).unwrap();
        let executable = root.join("bin").join(ffmpeg_file_name());
        fs::write(&executable, b"fixture executable").unwrap();

        let (bundled, error) = resolve_ffmpeg_executable(None, &root, ffmpeg_file_name());
        assert_eq!(bundled, fs::canonicalize(&executable).ok());
        assert_eq!(error, None);

        let (relative, error) =
            resolve_ffmpeg_executable(Some(OsString::from("ffmpeg")), &root, ffmpeg_file_name());
        assert_eq!(relative, None);
        assert_eq!(error, Some("ffmpeg-path-not-absolute"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_basename_and_resolution_reject_traversal_and_non_jpeg_names() {
        let root = unique_temp_dir();
        let root = prepare_cache_root(&root).unwrap();
        let fingerprint = "a".repeat(64);
        let valid = cache_file_name(7, &fingerprint).unwrap();
        fs::write(root.join(&valid), jpeg_fixture()).unwrap();
        assert_eq!(
            resolve_ready_cache_file(&root, &valid).unwrap(),
            fs::canonicalize(root.join(&valid)).unwrap()
        );

        for invalid in [
            "../7-deadbeef.jpg",
            "7/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.jpg",
            "7-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.jpg",
            "7-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.jpeg",
            "0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.jpg",
        ] {
            assert!(validate_cache_basename(invalid).is_err(), "{invalid}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generated_output_requires_bounded_complete_jpeg() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).unwrap();
        let valid = root.join("valid.jpg");
        fs::write(&valid, jpeg_fixture()).unwrap();
        assert_eq!(validate_generated_jpeg(&valid).unwrap(), 8);

        let invalid = root.join("invalid.jpg");
        fs::write(&invalid, b"not jpeg").unwrap();
        assert_eq!(
            validate_generated_jpeg(&invalid).unwrap_err().code,
            "thumbnail-output-not-jpeg"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_install_never_targets_a_different_directory() {
        let root = unique_temp_dir();
        let other = unique_temp_dir();
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&other).unwrap();
        let temporary = root.join(".part-1.jpg");
        fs::write(&temporary, jpeg_fixture()).unwrap();
        assert!(atomic_install_cache_file(&temporary, &other.join("final.jpg")).is_err());
        assert!(temporary.is_file());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn retry_schedule_has_three_backoff_windows_then_stops() {
        assert_eq!(retry_delay_modifier(1, true), Some("+1 minute"));
        assert_eq!(retry_delay_modifier(2, true), Some("+10 minutes"));
        assert_eq!(retry_delay_modifier(3, true), Some("+1 hour"));
        assert_eq!(retry_delay_modifier(4, true), None);
        assert_eq!(retry_delay_modifier(1, false), None);
    }

    #[test]
    fn persistent_queue_generates_multiple_jobs_with_single_concurrency() {
        let fixture = QueueFixture::new(4);
        let generator = Arc::new(FakeGenerator::new(false));
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let queue = ThumbnailQueue::start_with_sink(
            fixture.database_path.clone(),
            fixture.cache_root.clone(),
            generator.clone(),
            Arc::new(move |event| sink_events.lock().unwrap().push(event)),
        )
        .unwrap();

        wait_until(Duration::from_secs(8), || {
            queue.status().is_ok_and(|status| status.ready_count == 4)
        });
        assert_eq!(generator.generated.load(AtomicOrdering::Acquire), 4);
        assert_eq!(generator.max_active.load(AtomicOrdering::Acquire), 1);
        assert_eq!(queue.status().unwrap().processing_clip_id, None);
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.status == "ready")
                .count(),
            4
        );
        queue.shutdown();
    }

    #[test]
    fn shutdown_cancels_active_generation_and_recovers_running_row() {
        let fixture = QueueFixture::new(1);
        let generator = Arc::new(FakeGenerator::new(true));
        let queue = ThumbnailQueue::start_with_sink(
            fixture.database_path.clone(),
            fixture.cache_root.clone(),
            generator.clone(),
            Arc::new(|_| {}),
        )
        .unwrap();
        wait_until(Duration::from_secs(8), || {
            queue
                .status()
                .is_ok_and(|status| status.processing_clip_id.is_some())
        });

        let started_at = Instant::now();
        queue.shutdown();
        assert!(started_at.elapsed() < WORKER_SHUTDOWN_TIMEOUT);
        assert!(generator.cancelled.load(Ordering::Acquire));
        let connection = db::open_database(&fixture.database_path).unwrap();
        let status = db::get_thumbnail_status(&connection, fixture.clip_ids[0])
            .unwrap()
            .unwrap();
        assert_eq!(status.status, "pending");
    }

    #[test]
    fn source_change_during_generation_rejects_stale_ready_and_requeues_new_fingerprint() {
        let fixture = QueueFixture::new(1);
        let generator = Arc::new(StaleWindowGenerator::default());
        let queue = ThumbnailQueue::start_with_sink(
            fixture.database_path.clone(),
            fixture.cache_root.clone(),
            generator.clone(),
            Arc::new(|_| {}),
        )
        .unwrap();
        wait_until(Duration::from_secs(8), || {
            generator.started.load(Ordering::Acquire)
        });

        let connection = db::open_database(&fixture.database_path).unwrap();
        connection
            .execute(
                "UPDATE clips SET size_bytes = 99, modified_at = '200' WHERE id = ?1",
                [fixture.clip_ids[0]],
            )
            .unwrap();
        drop(connection);
        generator.release.store(true, Ordering::Release);

        wait_until(Duration::from_secs(8), || {
            queue.status().is_ok_and(|status| status.ready_count == 1)
                && generator.calls.load(AtomicOrdering::Acquire) >= 2
        });
        let connection = db::open_database(&fixture.database_path).unwrap();
        let clip = db::find_clip_by_id(&connection, fixture.clip_ids[0]).unwrap();
        let expected = db::thumbnail_fingerprint(&clip.normalized_path, 99, Some("200"));
        assert_eq!(clip.thumbnail_revision.as_deref(), Some(expected.as_str()));
        assert_eq!(
            fs::read_dir(queue.cache_root())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| validate_cache_basename(name).is_ok())
                })
                .count(),
            1,
            "stale fingerprint artifact must be removed"
        );
        queue.shutdown();
    }

    #[test]
    fn missing_generator_degrades_all_pending_jobs_without_spawning_work() {
        let fixture = QueueFixture::new(3);
        let queue = ThumbnailQueue::start_with_sink(
            fixture.database_path.clone(),
            fixture.cache_root.clone(),
            Arc::new(UnavailableGenerator),
            Arc::new(|_| {}),
        )
        .unwrap();
        wait_until(Duration::from_secs(8), || {
            queue
                .status()
                .is_ok_and(|status| status.unavailable_count == 3)
        });
        let status = queue.status().unwrap();
        assert_eq!(status.generator_status, GeneratorStatus::Unavailable);
        assert_eq!(status.pending_count, 0);
        assert_eq!(status.processing_clip_id, None);
        assert_eq!(
            status.last_error_code.as_deref(),
            Some("ffmpeg-unavailable")
        );
        queue.shutdown();
    }

    #[test]
    fn cache_budget_is_enforced_to_low_water_and_marks_evicted_rows() {
        let fixture = QueueFixture::new(3);
        seed_ready_cache_rows(&fixture);
        let queue = ThumbnailQueue::start_with_sink(
            fixture.database_path.clone(),
            fixture.cache_root.clone(),
            Arc::new(UnavailableGenerator),
            Arc::new(|_| {}),
        )
        .unwrap();
        wait_until(Duration::from_secs(8), || {
            queue.status().is_ok_and(|status| {
                status.ready_count == 3
                    && status.last_error_code.as_deref() == Some("ffmpeg-unavailable")
            })
        });

        queue.maintain_cache_with_limits(16, 8).unwrap();
        let status = queue.status().unwrap();
        assert_eq!(status.ready_count, 1);
        assert_eq!(status.evicted_count, 2);
        assert_eq!(status.cache_bytes, 8);
        let remaining = fs::read_dir(queue.cache_root())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| validate_cache_basename(name).is_ok())
            })
            .count();
        assert_eq!(remaining, 1);
        queue.shutdown();
    }

    #[test]
    fn eviction_delete_failure_keeps_ready_reference_and_tries_later_candidates() {
        let fixture = QueueFixture::new(3);
        seed_ready_cache_rows(&fixture);
        let queue = ThumbnailQueue::start_with_sink(
            fixture.database_path.clone(),
            fixture.cache_root.clone(),
            Arc::new(UnavailableGenerator),
            Arc::new(|_| {}),
        )
        .unwrap();
        wait_until(Duration::from_secs(8), || {
            queue.status().is_ok_and(|status| {
                status.ready_count == 3
                    && status.last_error_code.as_deref() == Some("ffmpeg-unavailable")
            })
        });
        let connection = db::open_database(&fixture.database_path).unwrap();
        let oldest = db::list_ready_thumbnail_cache_refs(&connection)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        drop(connection);
        let undeletable = fs::canonicalize(queue.cache_root().join(&oldest.cache_file)).unwrap();

        queue
            .maintain_cache_with_limits_using(16, 8, |path| {
                if path == undeletable {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "simulated Windows sharing violation",
                    ))
                } else {
                    remove_cache_file_if_present(path)
                }
            })
            .unwrap();
        let status = queue.status().unwrap();
        assert_eq!(status.ready_count, 1);
        assert_eq!(status.evicted_count, 2);
        assert!(undeletable.is_file());
        let connection = db::open_database(&fixture.database_path).unwrap();
        assert_eq!(
            db::get_thumbnail_status(&connection, oldest.clip_id)
                .unwrap()
                .unwrap()
                .status,
            "ready",
            "failed deletion must not drop the only DB reference"
        );
        queue.shutdown();
    }

    #[test]
    fn startup_maintenance_repairs_ready_rows_with_corrupt_jpeg_cache() {
        let fixture = QueueFixture::new(1);
        seed_ready_cache_rows(&fixture);
        let connection = db::open_database(&fixture.database_path).unwrap();
        let ready = db::list_ready_thumbnail_cache_refs(&connection).unwrap();
        drop(connection);
        let corrupt_path = fixture.cache_root.join(&ready[0].cache_file);
        fs::write(&corrupt_path, b"corrupt cache").unwrap();

        let queue = ThumbnailQueue::start_with_sink(
            fixture.database_path.clone(),
            fixture.cache_root.clone(),
            Arc::new(UnavailableGenerator),
            Arc::new(|_| {}),
        )
        .unwrap();
        wait_until(Duration::from_secs(8), || {
            queue
                .status()
                .is_ok_and(|status| status.unavailable_count == 1)
        });
        assert!(!corrupt_path.exists());
        assert_eq!(queue.status().unwrap().ready_count, 0);
        queue.shutdown();
    }

    #[test]
    fn progress_and_status_dtos_use_the_fixed_camel_case_contract() {
        let event = serde_json::to_value(ThumbnailProgressEvent {
            clip_id: 7,
            status: "ready".to_string(),
            revision: Some("revision".to_string()),
            error_code: None,
        })
        .unwrap();
        assert_eq!(event["clipId"], 7);
        assert_eq!(event["errorCode"], serde_json::Value::Null);

        let command = serde_json::to_value(db::ThumbnailEnsureResult {
            requested: 4,
            queued: 3,
            already_queued: 1,
            skipped: 0,
        })
        .unwrap();
        assert_eq!(command["requested"], 4);
        assert_eq!(command["alreadyQueued"], 1);

        let status = serde_json::to_value(ThumbnailServiceStatus {
            generator_status: GeneratorStatus::Available,
            pending_count: 1,
            running_count: 2,
            ready_count: 3,
            failed_count: 4,
            unavailable_count: 5,
            evicted_count: 6,
            cache_bytes: 7,
            processing_clip_id: None,
            last_error_code: None,
        })
        .unwrap();
        assert_eq!(status["generatorStatus"], "available");
        for key in [
            "pendingCount",
            "runningCount",
            "readyCount",
            "failedCount",
            "unavailableCount",
            "evictedCount",
            "cacheBytes",
            "processingClipId",
            "lastErrorCode",
        ] {
            assert!(status.get(key).is_some(), "missing status key {key}");
        }
    }

    #[test]
    fn command_validation_is_bounded_and_rejects_invalid_ids() {
        assert!(validate_command_clip_ids(&[1, 1, 2]).is_ok());
        assert!(validate_command_clip_ids(&[0]).is_err());
        assert!(validate_command_clip_ids(&(1..=201).collect::<Vec<_>>()).is_err());
        assert!(validate_command_clip_ids(&vec![1; 201]).is_err());
    }

    #[test]
    fn selected_ensure_wake_does_not_full_reconcile_a_10k_library() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("large-library.sqlite3");
        let cache_root = root.join("cache").join("thumbnails");
        db::migrate_database(&database_path).unwrap();
        let connection = db::open_database(&database_path).unwrap();
        let source = db::upsert_source_dir(
            &connection,
            db::SourceDirInput {
                path: "D:\\ReadOnlyLargeFixture",
                name: "Large fixture",
            },
        )
        .unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        {
            let mut insert = transaction
                .prepare(
                    "INSERT INTO clips (
                        source_dir_id, file_path, normalized_path, file_name, extension,
                        size_bytes, modified_at, cover_source, file_status
                     ) VALUES (?1, ?2, ?2, ?3, 'mp4', 13, '100', 'missing', 'available')",
                )
                .unwrap();
            for index in 0..10_000 {
                let path = format!("D:\\ReadOnlyLargeFixture\\clip-{index}.mp4");
                let name = format!("clip-{index}.mp4");
                insert
                    .execute(rusqlite::params![source.id, path, name])
                    .unwrap();
            }
        }
        transaction.commit().unwrap();
        drop(connection);

        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let queue = ThumbnailQueue::start_with_sink(
            database_path.clone(),
            cache_root,
            Arc::new(UnavailableGenerator),
            Arc::new(move |event| sink_events.lock().unwrap().push(event)),
        )
        .unwrap();
        wait_until(Duration::from_secs(20), || {
            queue
                .status()
                .is_ok_and(|status| status.unavailable_count == 10_000)
        });
        assert_eq!(queue.reconcile_count.load(AtomicOrdering::Acquire), 1);

        let selected = (1..=50).collect::<Vec<_>>();
        let result = queue.ensure_clip_thumbnails(&selected).unwrap();
        assert_eq!(result.requested, 50);
        thread::sleep(Duration::from_millis(300));
        assert_eq!(
            queue.reconcile_count.load(AtomicOrdering::Acquire),
            1,
            "visible-page ensure must wake due jobs without scanning the full library"
        );
        assert!(
            events.lock().unwrap().is_empty(),
            "full-library reconciliation must not emit one IPC event per clip"
        );
        queue.shutdown();
        fs::remove_dir_all(root).unwrap();
    }

    struct FakeGenerator {
        blocking: bool,
        active: AtomicUsize,
        max_active: AtomicUsize,
        generated: AtomicUsize,
        cancelled: AtomicBool,
    }

    impl FakeGenerator {
        fn new(blocking: bool) -> Self {
            Self {
                blocking,
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                generated: AtomicUsize::new(0),
                cancelled: AtomicBool::new(false),
            }
        }
    }

    impl ThumbnailGenerator for FakeGenerator {
        fn availability(&self) -> GeneratorAvailability {
            GeneratorAvailability::Available
        }

        fn unavailable_error_code(&self) -> Option<&'static str> {
            None
        }

        fn generate(
            &self,
            _source_path: &Path,
            temporary_output: &Path,
            cancellation: &AtomicBool,
        ) -> Result<u64, ThumbnailGenerationError> {
            let active = self.active.fetch_add(1, AtomicOrdering::AcqRel) + 1;
            self.max_active.fetch_max(active, AtomicOrdering::AcqRel);
            if self.blocking {
                while !cancellation.load(Ordering::Acquire)
                    && !self.cancelled.load(Ordering::Acquire)
                {
                    thread::sleep(Duration::from_millis(5));
                }
            } else {
                thread::sleep(Duration::from_millis(10));
            }
            if cancellation.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire) {
                self.active.fetch_sub(1, AtomicOrdering::AcqRel);
                return Err(ThumbnailGenerationError::cancelled());
            }
            fs::write(temporary_output, jpeg_fixture()).unwrap();
            self.generated.fetch_add(1, AtomicOrdering::AcqRel);
            self.active.fetch_sub(1, AtomicOrdering::AcqRel);
            Ok(jpeg_fixture().len() as u64)
        }

        fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
        }
    }

    struct UnavailableGenerator;

    impl ThumbnailGenerator for UnavailableGenerator {
        fn availability(&self) -> GeneratorAvailability {
            GeneratorAvailability::Unavailable
        }

        fn unavailable_error_code(&self) -> Option<&'static str> {
            Some("ffmpeg-unavailable")
        }

        fn generate(
            &self,
            _source_path: &Path,
            _temporary_output: &Path,
            _cancellation: &AtomicBool,
        ) -> Result<u64, ThumbnailGenerationError> {
            panic!("unavailable generator must never be invoked")
        }

        fn cancel(&self) {}
    }

    #[derive(Default)]
    struct StaleWindowGenerator {
        started: AtomicBool,
        release: AtomicBool,
        calls: AtomicUsize,
    }

    impl ThumbnailGenerator for StaleWindowGenerator {
        fn availability(&self) -> GeneratorAvailability {
            GeneratorAvailability::Available
        }

        fn unavailable_error_code(&self) -> Option<&'static str> {
            None
        }

        fn generate(
            &self,
            _source_path: &Path,
            temporary_output: &Path,
            cancellation: &AtomicBool,
        ) -> Result<u64, ThumbnailGenerationError> {
            let call = self.calls.fetch_add(1, AtomicOrdering::AcqRel);
            if call == 0 {
                self.started.store(true, Ordering::Release);
                while !self.release.load(Ordering::Acquire) && !cancellation.load(Ordering::Acquire)
                {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            if cancellation.load(Ordering::Acquire) {
                return Err(ThumbnailGenerationError::cancelled());
            }
            fs::write(temporary_output, jpeg_fixture()).unwrap();
            Ok(jpeg_fixture().len() as u64)
        }

        fn cancel(&self) {
            self.release.store(true, Ordering::Release);
        }
    }

    struct QueueFixture {
        root: PathBuf,
        database_path: PathBuf,
        cache_root: PathBuf,
        clip_ids: Vec<i64>,
    }

    impl QueueFixture {
        fn new(clip_count: usize) -> Self {
            let root = unique_temp_dir();
            let source_root = root.join("read-only-source");
            let cache_root = root.join("cache").join("thumbnails");
            fs::create_dir_all(&source_root).unwrap();
            let database_path = root.join("library.sqlite3");
            db::migrate_database(&database_path).unwrap();
            let connection = db::open_database(&database_path).unwrap();
            let source = db::upsert_source_dir(
                &connection,
                db::SourceDirInput {
                    path: source_root.to_string_lossy().as_ref(),
                    name: "Read-only fixture",
                },
            )
            .unwrap();
            let mut clip_ids = Vec::new();
            for index in 0..clip_count {
                let file_name = format!("clip-{index}.mp4");
                let video_path = source_root.join(&file_name);
                fs::write(&video_path, b"video fixture").unwrap();
                let clip = db::upsert_clip(
                    &connection,
                    db::ClipInput {
                        source_dir_id: source.id,
                        clip_group_id: None,
                        video_path: video_path.to_string_lossy().as_ref(),
                        file_name: &file_name,
                        file_size: 13,
                        modified_at: Some("100"),
                        duration_ms: None,
                        recorded_at: None,
                        cover_path: None,
                        cover_source: "missing",
                    },
                )
                .unwrap();
                clip_ids.push(clip.id);
            }
            drop(connection);
            Self {
                root,
                database_path,
                cache_root,
                clip_ids,
            }
        }
    }

    impl Drop for QueueFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn seed_ready_cache_rows(fixture: &QueueFixture) {
        fs::create_dir_all(&fixture.cache_root).unwrap();
        let connection = db::open_database(&fixture.database_path).unwrap();
        db::ensure_clip_thumbnails(&connection, &fixture.clip_ids).unwrap();
        while let Some(job) =
            db::claim_next_thumbnail_job(&connection, "2026-07-16 00:00:00").unwrap()
        {
            let cache_file = cache_file_name(job.clip_id, &job.fingerprint).unwrap();
            fs::write(fixture.cache_root.join(&cache_file), jpeg_fixture()).unwrap();
            assert!(db::complete_thumbnail_job_if_current(
                &connection,
                &job,
                &cache_file,
                jpeg_fixture().len() as i64,
                &job.fingerprint,
            )
            .unwrap());
        }
    }

    fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + timeout;
        while !predicate() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for thumbnail worker"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn jpeg_fixture() -> &'static [u8] {
        &[0xff, 0xd8, 0xff, 0xe0, 0, 0, 0xff, 0xd9]
    }

    fn unique_temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_TEMP_DIR_ID.fetch_add(1, AtomicOrdering::Relaxed);
        std::env::temp_dir().join(format!(
            "vhm-thumbnail-test-{}-{nonce}-{sequence}",
            std::process::id()
        ))
    }
}
