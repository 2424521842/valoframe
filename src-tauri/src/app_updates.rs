use std::{
    io::Cursor,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};

#[cfg(windows)]
use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::Write,
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    time::SystemTime,
};

use semver::Version;
use serde::Serialize;
use tauri::{ipc::Channel, AppHandle, Manager, State, Url};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};
use time::format_description::well_known::Rfc3339;
use tokio::sync::oneshot;

use crate::AppState;

pub const STABLE_UPDATE_ENDPOINT: &str =
    "https://github.com/2424521842/valoframe/releases/latest/download/latest.json";
const STABLE_RELEASE_OWNER: &str = "2424521842";
const STABLE_RELEASE_REPOSITORY: &str = "valoframe";
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_UPDATE_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RELEASE_NOTES_CHARS: usize = 12_000;
#[cfg(windows)]
const WINDOWS_UPDATER_TEMP_PREFIX: &str = "valoframe-updater-";
#[cfg(windows)]
const WINDOWS_UPDATER_MARKER_FILE: &str = ".valoframe-updater-owned";
#[cfg(windows)]
const WINDOWS_UPDATER_MARKER_CONTENT: &[u8] = b"valoframe-updater-v1";
#[cfg(windows)]
const WINDOWS_UPDATER_STALE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateRuntimeInfo {
    pub current_version: String,
    pub channel: &'static str,
    pub endpoint: &'static str,
    pub configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateMetadata {
    pub current_version: String,
    pub version: String,
    pub notes: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum AppUpdateDownloadEvent {
    Started {
        #[serde(rename = "contentLength")]
        content_length: Option<u64>,
    },
    Progress {
        #[serde(rename = "chunkLength")]
        chunk_length: usize,
    },
    Verifying,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateCommandError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl AppUpdateCommandError {
    fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum AppUpdatePhase {
    #[default]
    Idle,
    Checking,
    Available,
    Downloading,
    Downloaded,
    Installing,
}

#[derive(Default)]
struct AppUpdateSession {
    phase: AppUpdatePhase,
    pending: Option<Update>,
    downloaded_bytes: Option<Vec<u8>>,
    cancel_download: Option<oneshot::Sender<()>>,
    cancel_requested: bool,
}

#[derive(Default)]
pub struct AppUpdateState {
    session: Mutex<AppUpdateSession>,
}

#[tauri::command]
pub fn get_app_update_runtime_info(app: AppHandle) -> AppUpdateRuntimeInfo {
    #[cfg(windows)]
    cleanup_stale_windows_installer_directories();

    AppUpdateRuntimeInfo {
        current_version: app.package_info().version.to_string(),
        channel: "stable",
        endpoint: STABLE_UPDATE_ENDPOINT,
        configured: embedded_public_key().is_some(),
    }
}

#[tauri::command]
pub async fn check_for_app_update(
    app: AppHandle,
    update_state: State<'_, AppUpdateState>,
) -> Result<Option<AppUpdateMetadata>, AppUpdateCommandError> {
    let public_key = embedded_public_key().ok_or_else(|| {
        AppUpdateCommandError::new(
            "updater-not-configured",
            "当前构建未嵌入经批准的更新公钥，无法检查正式更新",
            false,
        )
    })?;

    {
        let mut session = lock_recover(&update_state.session);
        begin_update_check(&mut session)?;
    }

    let checked: Result<Option<Update>, AppUpdateCommandError> = async {
        let endpoint = STABLE_UPDATE_ENDPOINT.parse().map_err(|error| {
            AppUpdateCommandError::new(
                "updater-not-configured",
                format!("稳定更新端点无效：{error}"),
                false,
            )
        })?;
        let thumbnail_queue = app.state::<AppState>().thumbnail_queue.clone();
        let before_exit_app = app.clone();
        let updater = app
            .updater_builder()
            .pubkey(public_key)
            .endpoints(vec![endpoint])
            .map_err(map_updater_error)?
            .timeout(UPDATE_CHECK_TIMEOUT)
            .on_before_exit(move || {
                thumbnail_queue.shutdown();
                before_exit_app.cleanup_before_exit();
            })
            .build()
            .map_err(map_updater_error)?;
        updater.check().await.map_err(map_updater_error)
    }
    .await;
    match checked {
        Ok(Some(mut update)) => {
            if let Err(error) = ensure_stable_upgrade(&update.current_version, &update.version) {
                let mut session = lock_recover(&update_state.session);
                session.phase = AppUpdatePhase::Idle;
                session.pending = None;
                return Err(error);
            }
            if let Err(error) = ensure_manifest_version_is_canonical(&update) {
                let mut session = lock_recover(&update_state.session);
                session.phase = AppUpdatePhase::Idle;
                session.pending = None;
                return Err(error);
            }
            if let Err(error) = validate_stable_download_url(&update.download_url, &update.version)
            {
                let mut session = lock_recover(&update_state.session);
                session.phase = AppUpdatePhase::Idle;
                session.pending = None;
                return Err(error);
            }
            update.timeout = Some(UPDATE_DOWNLOAD_TIMEOUT);
            let metadata = metadata_from_update(&update);
            let mut session = lock_recover(&update_state.session);
            session.phase = AppUpdatePhase::Available;
            session.pending = Some(update);
            Ok(Some(metadata))
        }
        Ok(None) => {
            let mut session = lock_recover(&update_state.session);
            session.phase = AppUpdatePhase::Idle;
            session.pending = None;
            Ok(None)
        }
        Err(error) => {
            lock_recover(&update_state.session).phase = AppUpdatePhase::Idle;
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn download_app_update(
    update_state: State<'_, AppUpdateState>,
    on_event: Channel<AppUpdateDownloadEvent>,
) -> Result<(), AppUpdateCommandError> {
    let (update, cancel_receiver) = {
        let mut session = lock_recover(&update_state.session);
        if session.phase != AppUpdatePhase::Available {
            return Err(AppUpdateCommandError::new(
                "no-pending-update",
                "没有可下载的更新，请先重新检查",
                true,
            ));
        }
        let update = session.pending.clone().ok_or_else(|| {
            AppUpdateCommandError::new("no-pending-update", "更新会话已失效，请重新检查", true)
        })?;
        let (cancel_sender, cancel_receiver) = oneshot::channel();
        session.phase = AppUpdatePhase::Downloading;
        session.downloaded_bytes = None;
        session.cancel_download = Some(cancel_sender);
        session.cancel_requested = false;
        (update, cancel_receiver)
    };

    let started_channel = on_event.clone();
    let progress_channel = on_event.clone();
    let verifying_channel = on_event.clone();
    let finished_channel = on_event;
    let (size_limit_sender, size_limit_receiver) = oneshot::channel();
    let size_limit_exceeded = Arc::new(AtomicBool::new(false));
    let callback_size_limit_exceeded = Arc::clone(&size_limit_exceeded);
    let mut size_limit_sender = Some(size_limit_sender);
    let mut received_bytes = 0_u64;
    let mut started = false;
    let mut download = Box::pin(update.download(
        move |chunk_length, content_length| {
            if !started {
                let _ = started_channel.send(AppUpdateDownloadEvent::Started { content_length });
                started = true;
            }
            if update_download_size_exceeded(&mut received_bytes, chunk_length, content_length) {
                callback_size_limit_exceeded.store(true, Ordering::Release);
                if let Some(sender) = size_limit_sender.take() {
                    let _ = sender.send(());
                }
                return;
            }
            let _ = progress_channel.send(AppUpdateDownloadEvent::Progress { chunk_length });
        },
        move || {
            let _ = verifying_channel.send(AppUpdateDownloadEvent::Verifying);
        },
    ));
    let mut cancel_receiver = Box::pin(cancel_receiver);
    let mut size_limit_receiver = Box::pin(size_limit_receiver);

    let result = tokio::select! {
        _ = &mut cancel_receiver => Err(download_cancelled_error()),
        _ = &mut size_limit_receiver => Err(update_package_too_large_error()),
        result = &mut download => result.map_err(map_updater_error),
    };
    let result = if size_limit_exceeded.load(Ordering::Acquire) {
        Err(update_package_too_large_error())
    } else {
        result
    };
    let result = result.and_then(|bytes| {
        validate_signed_windows_update_archive(&bytes, &update.version)?;
        Ok(bytes)
    });

    let mut session = lock_recover(&update_state.session);
    let committed = commit_download_result(&mut session, result);
    drop(session);
    if committed.is_ok() {
        let _ = finished_channel.send(AppUpdateDownloadEvent::Finished);
    }
    committed
}

#[tauri::command]
pub fn cancel_app_update_download(update_state: State<'_, AppUpdateState>) -> bool {
    request_download_cancellation(&mut lock_recover(&update_state.session))
}

#[tauri::command]
pub fn discard_app_update(
    update_state: State<'_, AppUpdateState>,
) -> Result<bool, AppUpdateCommandError> {
    discard_update_session(&mut lock_recover(&update_state.session))
}

#[tauri::command]
pub fn install_app_update(
    _app: AppHandle,
    app_state: State<'_, AppState>,
    update_state: State<'_, AppUpdateState>,
) -> Result<(), AppUpdateCommandError> {
    {
        let session = lock_recover(&update_state.session);
        if session.phase != AppUpdatePhase::Downloaded
            || session.pending.is_none()
            || session.downloaded_bytes.is_none()
        {
            return Err(AppUpdateCommandError::new(
                "update-not-downloaded",
                "更新包尚未完成下载和签名验证",
                true,
            ));
        }
    }

    let _install_lease = app_state
        .critical_tasks
        .begin_update_install()
        .map_err(|snapshot| {
            AppUpdateCommandError::new("critical-task-busy", snapshot.busy_message(), true)
        })?;

    let (update, bytes) = {
        let mut session = lock_recover(&update_state.session);
        let update = session.pending.clone().ok_or_else(|| {
            AppUpdateCommandError::new("no-pending-update", "更新会话已失效，请重新检查", true)
        })?;
        let bytes = session.downloaded_bytes.take().ok_or_else(|| {
            AppUpdateCommandError::new(
                "update-not-downloaded",
                "更新包尚未完成下载和签名验证",
                true,
            )
        })?;
        session.phase = AppUpdatePhase::Installing;
        (update, bytes)
    };

    #[cfg(windows)]
    let install_result = launch_verified_windows_installer(&bytes, &update.version);
    #[cfg(not(windows))]
    let install_result = update.install(&bytes).map_err(map_updater_error);

    if let Err(error) = install_result {
        let mut session = lock_recover(&update_state.session);
        restore_downloaded_after_install_failure(&mut session, bytes);
        return Err(error);
    }

    #[cfg(windows)]
    {
        app_state.thumbnail_queue.shutdown();
        _app.cleanup_before_exit();
        std::process::exit(0)
    }

    #[cfg(not(windows))]
    {
        _app.restart();
        Ok(())
    }
}

fn begin_update_check(session: &mut AppUpdateSession) -> Result<(), AppUpdateCommandError> {
    match session.phase {
        AppUpdatePhase::Checking | AppUpdatePhase::Downloading | AppUpdatePhase::Installing => {
            return Err(AppUpdateCommandError::new(
                "updater-busy",
                "更新任务正在进行，请等待当前操作结束",
                true,
            ));
        }
        AppUpdatePhase::Available => {
            return Err(AppUpdateCommandError::new(
                "update-already-available",
                "已有稳定更新等待下载，不能用新的检查覆盖；请先下载或重启应用",
                false,
            ));
        }
        AppUpdatePhase::Downloaded => {
            return Err(AppUpdateCommandError::new(
                "update-already-downloaded",
                "已有通过签名验证的更新包等待安装，不能用新的检查覆盖",
                false,
            ));
        }
        AppUpdatePhase::Idle => {}
    }

    session.phase = AppUpdatePhase::Checking;
    session.pending = None;
    session.downloaded_bytes = None;
    session.cancel_download = None;
    session.cancel_requested = false;
    Ok(())
}

fn discard_update_session(session: &mut AppUpdateSession) -> Result<bool, AppUpdateCommandError> {
    match session.phase {
        AppUpdatePhase::Idle => Ok(false),
        AppUpdatePhase::Available | AppUpdatePhase::Downloaded => {
            session.pending = None;
            session.downloaded_bytes = None;
            session.cancel_download = None;
            session.cancel_requested = false;
            session.phase = AppUpdatePhase::Idle;
            Ok(true)
        }
        AppUpdatePhase::Checking | AppUpdatePhase::Downloading | AppUpdatePhase::Installing => {
            Err(AppUpdateCommandError::new(
                "updater-busy",
                "更新任务正在进行，请等待当前操作结束",
                true,
            ))
        }
    }
}

fn restore_downloaded_after_install_failure(session: &mut AppUpdateSession, bytes: Vec<u8>) {
    session.downloaded_bytes = Some(bytes);
    session.phase = AppUpdatePhase::Downloaded;
}

fn request_download_cancellation(session: &mut AppUpdateSession) -> bool {
    if session.phase != AppUpdatePhase::Downloading || session.cancel_requested {
        return false;
    }
    let Some(sender) = session.cancel_download.take() else {
        return false;
    };
    if sender.send(()).is_err() {
        return false;
    }
    session.cancel_requested = true;
    true
}

fn commit_download_result(
    session: &mut AppUpdateSession,
    result: Result<Vec<u8>, AppUpdateCommandError>,
) -> Result<(), AppUpdateCommandError> {
    session.cancel_download = None;
    let cancel_requested = std::mem::take(&mut session.cancel_requested);
    if cancel_requested {
        session.downloaded_bytes = None;
        session.phase = AppUpdatePhase::Available;
        return Err(download_cancelled_error());
    }

    match result {
        Ok(bytes) => {
            session.downloaded_bytes = Some(bytes);
            session.phase = AppUpdatePhase::Downloaded;
            Ok(())
        }
        Err(error) => {
            session.downloaded_bytes = None;
            session.phase = AppUpdatePhase::Available;
            Err(error)
        }
    }
}

fn download_cancelled_error() -> AppUpdateCommandError {
    AppUpdateCommandError::new(
        "update-download-cancelled",
        "已取消更新下载；当前版本未发生变化",
        true,
    )
}

fn update_package_too_large_error() -> AppUpdateCommandError {
    AppUpdateCommandError::new(
        "update-package-too-large",
        "更新包超过 512 MiB 安全上限，已停止下载",
        false,
    )
}

fn update_download_size_exceeded(
    received_bytes: &mut u64,
    chunk_length: usize,
    content_length: Option<u64>,
) -> bool {
    *received_bytes = (*received_bytes).saturating_add(chunk_length as u64);
    content_length.is_some_and(|length| length > MAX_UPDATE_PACKAGE_BYTES)
        || *received_bytes > MAX_UPDATE_PACKAGE_BYTES
}

fn embedded_public_key() -> Option<&'static str> {
    option_env!("VALOFRAME_UPDATER_PUBLIC_KEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

fn ensure_stable_upgrade(
    current_version: &str,
    candidate_version: &str,
) -> Result<(), AppUpdateCommandError> {
    let candidate = Version::parse(candidate_version).map_err(|error| {
        AppUpdateCommandError::new(
            "invalid-update-metadata",
            format!("更新版本号无效：{error}"),
            false,
        )
    })?;
    if !candidate.pre.is_empty() {
        return Err(AppUpdateCommandError::new(
            "prerelease-update-refused",
            "稳定更新通道拒绝 prerelease 版本",
            false,
        ));
    }
    if !candidate.build.is_empty() || candidate.to_string() != candidate_version {
        return Err(AppUpdateCommandError::new(
            "invalid-update-metadata",
            "稳定更新版本号必须是规范的 MAJOR.MINOR.PATCH，不能包含构建元数据",
            false,
        ));
    }
    let current = Version::parse(current_version).map_err(|error| {
        AppUpdateCommandError::new(
            "invalid-update-metadata",
            format!("当前应用版本号无效：{error}"),
            false,
        )
    })?;
    if candidate <= current {
        return Err(AppUpdateCommandError::new(
            "update-not-newer",
            "稳定更新通道拒绝同版本或降级安装",
            false,
        ));
    }
    Ok(())
}

fn ensure_manifest_version_is_canonical(update: &Update) -> Result<(), AppUpdateCommandError> {
    ensure_raw_manifest_version_is_canonical(&update.raw_json, &update.version)
}

fn ensure_raw_manifest_version_is_canonical(
    raw_json: &serde_json::Value,
    candidate_version: &str,
) -> Result<(), AppUpdateCommandError> {
    let raw_version = raw_json
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            AppUpdateCommandError::new("invalid-update-metadata", "更新清单缺少字符串版本号", false)
        })?;
    if raw_version != candidate_version {
        return Err(AppUpdateCommandError::new(
            "invalid-update-metadata",
            "稳定更新清单必须使用规范的 MAJOR.MINOR.PATCH 版本号",
            false,
        ));
    }
    Ok(())
}

fn validate_stable_download_url(
    download_url: &Url,
    candidate_version: &str,
) -> Result<(), AppUpdateCommandError> {
    let valid_origin = download_url.scheme() == "https"
        && download_url.host_str() == Some("github.com")
        && download_url.username().is_empty()
        && download_url.password().is_none()
        && download_url.port().is_none()
        && download_url.query().is_none()
        && download_url.fragment().is_none();
    let segments = download_url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    let expected_tag = format!("v{candidate_version}");
    let valid_path = segments.len() == 6
        && segments[0] == STABLE_RELEASE_OWNER
        && segments[1] == STABLE_RELEASE_REPOSITORY
        && segments[2] == "releases"
        && segments[3] == "download"
        && segments[4] == expected_tag
        && valid_updater_asset_segment(segments[5]);

    if !valid_origin || !valid_path {
        return Err(AppUpdateCommandError::new(
            "invalid-update-metadata",
            "更新包地址未严格绑定到获批的 GitHub 稳定版本资产",
            false,
        ));
    }
    Ok(())
}

fn valid_updater_asset_segment(segment: &str) -> bool {
    const SUFFIX: &str = ".nsis.zip";
    if segment.len() <= SUFFIX.len() || !segment.ends_with(SUFFIX) {
        return false;
    }
    let lower = segment.to_ascii_lowercase();
    !lower.contains("%2f") && !lower.contains("%5c") && !lower.contains("%00")
}

fn validate_signed_windows_update_archive(
    bytes: &[u8],
    candidate_version: &str,
) -> Result<(), AppUpdateCommandError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| {
        AppUpdateCommandError::new(
            "invalid-update-package",
            "已签名更新包不是有效的 NSIS ZIP 归档",
            false,
        )
    })?;
    if archive.len() != 1 {
        return Err(AppUpdateCommandError::new(
            "invalid-update-package",
            "已签名更新包必须只包含一个根级 NSIS 安装器",
            false,
        ));
    }

    let installer = archive.by_index(0).map_err(|_| {
        AppUpdateCommandError::new("invalid-update-package", "无法读取已签名更新包目录", false)
    })?;
    validate_windows_installer_entry(
        installer.name(),
        installer.enclosed_name().as_deref(),
        installer.is_file(),
        installer.size(),
        candidate_version,
    )
}

fn validate_windows_installer_entry(
    name: &str,
    enclosed_name: Option<&Path>,
    is_file: bool,
    uncompressed_size: u64,
    candidate_version: &str,
) -> Result<(), AppUpdateCommandError> {
    let expected_name = expected_windows_installer_name(candidate_version);
    if !is_file || name != expected_name || enclosed_name != Some(Path::new(&expected_name)) {
        return Err(AppUpdateCommandError::new(
            "invalid-update-package",
            "已签名更新包内的安装器名称或目录与候选版本不匹配",
            false,
        ));
    }
    if uncompressed_size == 0 || uncompressed_size > MAX_UPDATE_PACKAGE_BYTES {
        return Err(AppUpdateCommandError::new(
            "invalid-update-package",
            "已签名更新包内的安装器大小无效或超过 512 MiB 安全上限",
            false,
        ));
    }
    Ok(())
}

fn expected_windows_installer_name(candidate_version: &str) -> String {
    format!("瓦刻_{candidate_version}_x64-setup.exe")
}

#[cfg(windows)]
struct PreparedWindowsInstaller {
    path: PathBuf,
    temp_dir: tempfile::TempDir,
}

#[cfg(windows)]
fn prepare_verified_windows_installer(
    bytes: &[u8],
    candidate_version: &str,
) -> Result<PreparedWindowsInstaller, AppUpdateCommandError> {
    cleanup_stale_windows_installer_directories();

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| {
        AppUpdateCommandError::new(
            "invalid-update-package",
            "已签名更新包不是有效的 NSIS ZIP 归档",
            false,
        )
    })?;
    if archive.len() != 1 {
        return Err(AppUpdateCommandError::new(
            "invalid-update-package",
            "已签名更新包必须只包含一个根级 NSIS 安装器",
            false,
        ));
    }

    let expected_name = expected_windows_installer_name(candidate_version);
    let mut installer = archive.by_index(0).map_err(|_| {
        AppUpdateCommandError::new("invalid-update-package", "无法读取已签名更新包目录", false)
    })?;
    validate_windows_installer_entry(
        installer.name(),
        installer.enclosed_name().as_deref(),
        installer.is_file(),
        installer.size(),
        candidate_version,
    )?;

    let temp_dir = tempfile::Builder::new()
        .prefix(WINDOWS_UPDATER_TEMP_PREFIX)
        .tempdir()
        .map_err(map_installer_preparation_error)?;
    let marker_path = temp_dir.path().join(WINDOWS_UPDATER_MARKER_FILE);
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker_path)
        .map_err(map_installer_preparation_error)?;
    marker
        .write_all(WINDOWS_UPDATER_MARKER_CONTENT)
        .map_err(map_installer_preparation_error)?;
    marker.sync_all().map_err(map_installer_preparation_error)?;
    drop(marker);

    let path = temp_dir.path().join(expected_name);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(map_installer_preparation_error)?;
    let copied = std::io::copy(
        &mut std::io::Read::take(&mut installer, MAX_UPDATE_PACKAGE_BYTES + 1),
        &mut output,
    )
    .map_err(map_installer_preparation_error)?;
    if copied == 0 || copied > MAX_UPDATE_PACKAGE_BYTES {
        return Err(AppUpdateCommandError::new(
            "invalid-update-package",
            "更新安装器解压后大小无效或超过 512 MiB 安全上限",
            false,
        ));
    }
    output.sync_all().map_err(map_installer_preparation_error)?;
    drop(output);

    Ok(PreparedWindowsInstaller { path, temp_dir })
}

#[cfg(windows)]
fn launch_verified_windows_installer(
    bytes: &[u8],
    candidate_version: &str,
) -> Result<(), AppUpdateCommandError> {
    use windows_sys::{
        w,
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOW},
    };

    let prepared = prepare_verified_windows_installer(bytes, candidate_version)?;
    let file = encode_windows_wide(prepared.path.as_os_str());
    let parameters = windows_nsis_parameters(std::env::args_os().skip(1));
    let parameters = encode_windows_wide(parameters.as_os_str());
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            w!("open"),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOW,
        )
    } as isize;

    finish_windows_installer_launch(prepared, result)
}

#[cfg(windows)]
fn finish_windows_installer_launch(
    prepared: PreparedWindowsInstaller,
    result: isize,
) -> Result<(), AppUpdateCommandError> {
    if shell_execute_succeeded(result) {
        let _persisted_installer_directory = prepared.temp_dir.keep();
        return Ok(());
    }

    let cleanup_suffix = prepared
        .temp_dir
        .close()
        .err()
        .map(|error| format!("；临时安装器清理也失败：{error}"))
        .unwrap_or_default();
    Err(AppUpdateCommandError::new(
        "installer-launch-failed",
        format!(
            "Windows 未能启动更新安装器（ShellExecuteW 返回 {result}），已保留通过签名验证的更新包，可重试安装{cleanup_suffix}"
        ),
        true,
    ))
}

#[cfg(windows)]
fn shell_execute_succeeded(result: isize) -> bool {
    result > 32
}

#[cfg(windows)]
fn encode_windows_wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn windows_nsis_parameters(args: impl IntoIterator<Item = OsString>) -> OsString {
    let mut parameters = OsString::from("/P /R /UPDATE /ARGS");
    for arg in args {
        parameters.push(" ");
        parameters.push(escape_nsis_current_exe_arg(arg.as_os_str()));
    }
    parameters
}

// Mirrors the pinned updater's NSIS escaping: quotes whitespace, empty arguments and `/` so
// forwarded application arguments cannot be reinterpreted as installer switches.
#[cfg(windows)]
fn escape_nsis_current_exe_arg(arg: &OsStr) -> String {
    let arg = arg.to_string_lossy();
    let mut escaped = Vec::new();
    let quote = arg
        .chars()
        .any(|character| matches!(character, ' ' | '\t' | '/'))
        || arg.is_empty();
    if quote {
        escaped.push('"');
    }

    let mut backslashes = 0_usize;
    for character in arg.chars() {
        if character == '\\' {
            backslashes += 1;
        } else {
            if character == '"' {
                escaped.extend((0..=backslashes).map(|_| '\\'));
            }
            backslashes = 0;
        }
        escaped.push(character);
    }
    if quote {
        escaped.extend((0..backslashes).map(|_| '\\'));
        escaped.push('"');
    }
    escaped.into_iter().collect()
}

#[cfg(windows)]
fn cleanup_stale_windows_installer_directories() {
    let _ = cleanup_stale_windows_installer_directories_in(
        &std::env::temp_dir(),
        WINDOWS_UPDATER_STALE_AGE,
    );
}

#[cfg(windows)]
fn cleanup_stale_windows_installer_directories_in(
    temp_root: &Path,
    minimum_age: Duration,
) -> std::io::Result<usize> {
    // A just-launched NSIS process can still hold its executable open. The age gate avoids racing
    // it, while the marker, exact contents and reparse-point checks keep deletion narrowly scoped.
    let mut removed = 0;
    for entry in fs::read_dir(temp_root)? {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.file_type().is_dir() || metadata_is_reparse_point(&metadata) {
            continue;
        }
        let Some(directory_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !directory_name.starts_with(WINDOWS_UPDATER_TEMP_PREFIX) {
            continue;
        }
        let Ok(modified_at) = metadata.modified() else {
            continue;
        };
        let Ok(age) = SystemTime::now().duration_since(modified_at) else {
            continue;
        };
        if age < minimum_age || !is_owned_windows_installer_directory(&entry.path()) {
            continue;
        }
        if fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(windows)]
fn is_owned_windows_installer_directory(path: &Path) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    let mut marker_found = false;
    let mut installer_found = false;
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            return false;
        };
        if !metadata.file_type().is_file() || metadata_is_reparse_point(&metadata) {
            return false;
        }
        let name = entry.file_name();
        if name == OsStr::new(WINDOWS_UPDATER_MARKER_FILE) {
            if marker_found || !has_valid_windows_updater_marker(&entry.path()) {
                return false;
            }
            marker_found = true;
        } else if is_owned_windows_installer_filename(&name) {
            if installer_found {
                return false;
            }
            installer_found = true;
        } else {
            return false;
        }
    }
    marker_found && installer_found
}

#[cfg(windows)]
fn has_valid_windows_updater_marker(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_file()
        && !metadata_is_reparse_point(&metadata)
        && metadata.len() == WINDOWS_UPDATER_MARKER_CONTENT.len() as u64
        && matches!(fs::read(path), Ok(content) if content == WINDOWS_UPDATER_MARKER_CONTENT)
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(windows)]
fn is_owned_windows_installer_filename(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(version) = name
        .strip_prefix("瓦刻_")
        .and_then(|name| name.strip_suffix("_x64-setup.exe"))
    else {
        return false;
    };
    let Ok(parsed) = Version::parse(version) else {
        return false;
    };
    parsed.pre.is_empty()
        && parsed.build.is_empty()
        && parsed.to_string() == version
        && expected_windows_installer_name(version) == name
}

#[cfg(windows)]
fn map_installer_preparation_error(error: std::io::Error) -> AppUpdateCommandError {
    AppUpdateCommandError::new(
        "installer-preparation-failed",
        format!("无法安全准备更新安装器：{error}"),
        true,
    )
}

fn metadata_from_update(update: &Update) -> AppUpdateMetadata {
    AppUpdateMetadata {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        notes: sanitize_release_notes(update.body.as_deref().unwrap_or("暂无发布说明")),
        published_at: update.date.and_then(format_published_at),
    }
}

fn format_published_at(date: time::OffsetDateTime) -> Option<String> {
    date.format(&Rfc3339).ok()
}

fn sanitize_release_notes(notes: &str) -> String {
    notes
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .take(MAX_RELEASE_NOTES_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn map_updater_error(error: UpdaterError) -> AppUpdateCommandError {
    let message = error.to_string();
    match error {
        UpdaterError::Minisign(_) | UpdaterError::Base64(_) | UpdaterError::SignatureUtf8(_) => {
            AppUpdateCommandError::new(
                "update-signature-invalid",
                format!("更新包签名验证失败：{message}"),
                false,
            )
        }
        UpdaterError::Semver(_)
        | UpdaterError::Serialization(_)
        | UpdaterError::TargetNotFound(_)
        | UpdaterError::TargetsNotFound(_)
        | UpdaterError::InvalidUpdaterFormat => AppUpdateCommandError::new(
            "invalid-update-metadata",
            format!("更新元数据无效：{message}"),
            false,
        ),
        UpdaterError::Network(_)
        | UpdaterError::Reqwest(_)
        | UpdaterError::ReleaseNotFound
        | UpdaterError::Http(_) => AppUpdateCommandError::new(
            "update-network-error",
            format!("无法连接稳定更新服务：{message}"),
            true,
        ),
        _ => AppUpdateCommandError::new(
            "update-operation-failed",
            format!("更新操作失败：{message}"),
            true,
        ),
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    fn updater_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer
                .start_file(*name, options)
                .expect("fixture entry should start");
            writer
                .write_all(bytes)
                .expect("fixture entry should be written");
        }
        writer
            .finish()
            .expect("fixture archive should finish")
            .into_inner()
    }

    #[test]
    fn base_tauri_config_contains_an_inert_deserializable_updater_bootstrap() {
        let root: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("base Tauri configuration should be valid JSON");
        let bootstrap = root
            .pointer("/plugins/updater")
            .cloned()
            .expect("updater bootstrap must be an object instead of Tauri's null default");
        let config: tauri_plugin_updater::Config = serde_json::from_value(bootstrap)
            .expect("updater bootstrap must deserialize during application startup");

        assert!(config.pubkey.is_empty());
        assert!(config.endpoints.is_empty());
        assert!(config.windows.is_none());
        assert!(!config.dangerous_insecure_transport_protocol);
        assert!(!config.dangerous_accept_invalid_certs);
        assert!(!config.dangerous_accept_invalid_hostnames);
    }

    #[test]
    fn stable_channel_only_accepts_higher_non_prerelease_versions() {
        assert!(ensure_stable_upgrade("0.2.0", "0.2.1").is_ok());
        assert_eq!(
            ensure_stable_upgrade("0.2.0", "0.3.0-rc.1")
                .expect_err("prerelease should fail")
                .code,
            "prerelease-update-refused"
        );
        assert_eq!(
            ensure_stable_upgrade("0.2.0", "tomorrow")
                .expect_err("invalid semver should fail")
                .code,
            "invalid-update-metadata"
        );
        assert_eq!(
            ensure_stable_upgrade("0.2.0", "0.2.0")
                .expect_err("same version should fail")
                .code,
            "update-not-newer"
        );
        assert_eq!(
            ensure_stable_upgrade("0.2.0", "0.1.9")
                .expect_err("downgrade should fail")
                .code,
            "update-not-newer"
        );
        for invalid in ["v0.2.1", "0.2.1+build.7", "01.2.1", "0.2.1 "] {
            assert_eq!(
                ensure_stable_upgrade("0.2.0", invalid)
                    .expect_err("non-canonical stable version should fail")
                    .code,
                "invalid-update-metadata"
            );
        }
        assert!(ensure_raw_manifest_version_is_canonical(
            &serde_json::json!({ "version": "0.2.1" }),
            "0.2.1"
        )
        .is_ok());
        assert_eq!(
            ensure_raw_manifest_version_is_canonical(
                &serde_json::json!({ "version": "v0.2.1" }),
                "0.2.1"
            )
            .expect_err("the plugin-normalized v prefix must still be rejected")
            .code,
            "invalid-update-metadata"
        );
    }

    #[test]
    fn stable_download_url_is_bound_to_the_candidate_github_release() {
        let valid = Url::parse(
            "https://github.com/2424521842/valoframe/releases/download/v0.2.1/%E7%93%A6%E5%88%BB_0.2.1_x64-setup.nsis.zip",
        )
        .unwrap();
        assert!(validate_stable_download_url(&valid, "0.2.1").is_ok());

        for invalid in [
            "http://github.com/2424521842/valoframe/releases/download/v0.2.1/update.nsis.zip",
            "https://github.com.evil.invalid/2424521842/valoframe/releases/download/v0.2.1/update.nsis.zip",
            "https://user@github.com/2424521842/valoframe/releases/download/v0.2.1/update.nsis.zip",
            "https://github.com/2424521842/valoframe/releases/download/v0.2.1/update.nsis.zip?mirror=1",
            "https://github.com/2424521842/valoframe/releases/download/v0.2.1/update.nsis.zip#fragment",
            "https://github.com/2424521842/valoframe/releases/download/v0.2.0/update.nsis.zip",
            "https://github.com/2424521842/valoframe/releases/download/v0.2.1/nested/update.nsis.zip",
            "https://github.com/2424521842/valoframe/releases/download/v0.2.1/update.zip",
            "https://github.com/2424521842/valoframe/releases/download/v0.2.1/update%2fescape.nsis.zip",
        ] {
            let url = Url::parse(invalid).unwrap();
            assert_eq!(
                validate_stable_download_url(&url, "0.2.1")
                    .expect_err("unapproved updater URL should fail")
                    .code,
                "invalid-update-metadata",
                "URL unexpectedly accepted: {url}"
            );
        }
    }

    #[test]
    fn signed_archive_must_contain_only_the_exact_candidate_installer() {
        let valid = updater_archive(&[("瓦刻_0.2.1_x64-setup.exe", b"MZ")]);
        assert!(validate_signed_windows_update_archive(&valid, "0.2.1").is_ok());

        for invalid in [
            updater_archive(&[("瓦刻_0.2.0_x64-setup.exe", b"MZ")]),
            updater_archive(&[("nested/瓦刻_0.2.1_x64-setup.exe", b"MZ")]),
            updater_archive(&[
                ("瓦刻_0.2.1_x64-setup.exe", b"MZ"),
                ("extra.txt", b"unexpected"),
            ]),
        ] {
            assert_eq!(
                validate_signed_windows_update_archive(&invalid, "0.2.1")
                    .expect_err("replayed or ambiguous updater archive should fail")
                    .code,
                "invalid-update-package"
            );
        }

        let expected_name = expected_windows_installer_name("0.2.1");
        for invalid_size in [0, MAX_UPDATE_PACKAGE_BYTES + 1] {
            assert_eq!(
                validate_windows_installer_entry(
                    &expected_name,
                    Some(Path::new(&expected_name)),
                    true,
                    invalid_size,
                    "0.2.1",
                )
                .expect_err("an empty or oversized extracted installer should fail")
                .code,
                "invalid-update-package"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn verified_installer_is_written_to_a_unique_temp_path_and_launch_failure_cleans_it() {
        let bytes = updater_archive(&[("瓦刻_0.2.1_x64-setup.exe", b"MZ installer")]);
        let prepared = prepare_verified_windows_installer(&bytes, "0.2.1")
            .expect("verified installer should be prepared");
        let path = prepared.path.clone();
        let second = prepare_verified_windows_installer(&bytes, "0.2.1")
            .expect("each retry should get its own directory");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("瓦刻_0.2.1_x64-setup.exe")
        );
        assert_ne!(path.parent(), second.path.parent());
        assert_eq!(std::fs::read(&path).unwrap(), b"MZ installer");

        let error = finish_windows_installer_launch(prepared, 32)
            .expect_err("ShellExecuteW results up to 32 must be failures");
        finish_windows_installer_launch(second, 0)
            .expect_err("a zero ShellExecuteW result must be a failure");
        assert_eq!(error.code, "installer-launch-failed");
        assert!(error.retryable);
        assert!(!path.exists(), "failed launch must remove temporary files");
        assert!(!shell_execute_succeeded(0));
        assert!(!shell_execute_succeeded(32));
        assert!(shell_execute_succeeded(33));
    }

    #[cfg(windows)]
    #[test]
    fn nsis_arguments_are_forwarded_after_the_updater_marker_with_plugin_compatible_escaping() {
        let parameters = windows_nsis_parameters([
            OsString::from("--flag"),
            OsString::from("C:/clip one.mp4"),
            OsString::new(),
        ]);
        assert_eq!(
            parameters.to_string_lossy(),
            "/P /R /UPDATE /ARGS --flag \"C:/clip one.mp4\" \"\""
        );
    }

    #[cfg(windows)]
    #[test]
    fn stale_cleanup_only_removes_exact_owned_updater_directories() {
        let root = tempfile::tempdir().unwrap();
        let owned = root.path().join("valoframe-updater-owned");
        fs::create_dir(&owned).unwrap();
        fs::write(
            owned.join(WINDOWS_UPDATER_MARKER_FILE),
            WINDOWS_UPDATER_MARKER_CONTENT,
        )
        .unwrap();
        fs::write(owned.join("瓦刻_0.2.1_x64-setup.exe"), b"MZ").unwrap();

        let unowned = root.path().join("valoframe-updater-unowned");
        fs::create_dir(&unowned).unwrap();
        fs::write(
            unowned.join(WINDOWS_UPDATER_MARKER_FILE),
            WINDOWS_UPDATER_MARKER_CONTENT,
        )
        .unwrap();
        fs::write(unowned.join("瓦刻_0.2.1_x64-setup.exe"), b"MZ").unwrap();
        fs::write(unowned.join("keep.txt"), b"not updater-owned").unwrap();

        assert_eq!(
            cleanup_stale_windows_installer_directories_in(root.path(), Duration::ZERO).unwrap(),
            1
        );
        assert!(!owned.exists());
        assert!(unowned.exists());
    }

    #[test]
    fn downloaded_session_cannot_be_overwritten_by_a_new_check() {
        let mut session = AppUpdateSession {
            phase: AppUpdatePhase::Downloaded,
            downloaded_bytes: Some(vec![1, 2, 3]),
            ..Default::default()
        };

        let error = begin_update_check(&mut session)
            .expect_err("a verified download must be preserved until install");
        assert_eq!(error.code, "update-already-downloaded");
        assert_eq!(session.phase, AppUpdatePhase::Downloaded);
        assert_eq!(
            session.downloaded_bytes.as_deref(),
            Some([1, 2, 3].as_slice())
        );
    }

    #[test]
    fn install_failure_restores_the_verified_download_for_retry() {
        let mut session = AppUpdateSession {
            phase: AppUpdatePhase::Installing,
            ..Default::default()
        };

        restore_downloaded_after_install_failure(&mut session, vec![1, 2, 3]);

        assert_eq!(session.phase, AppUpdatePhase::Downloaded);
        assert_eq!(session.downloaded_bytes, Some(vec![1, 2, 3]));
    }

    #[test]
    fn available_session_cannot_be_overwritten_by_a_new_check() {
        let mut session = AppUpdateSession {
            phase: AppUpdatePhase::Available,
            ..Default::default()
        };

        let error = begin_update_check(&mut session)
            .expect_err("an available candidate must be preserved for download or retry");
        assert_eq!(error.code, "update-already-available");
        assert_eq!(session.phase, AppUpdatePhase::Available);
    }

    #[test]
    fn discard_available_update_resets_the_entire_session() {
        let (cancel_sender, mut cancel_receiver) = oneshot::channel();
        let mut session = AppUpdateSession {
            phase: AppUpdatePhase::Available,
            downloaded_bytes: Some(vec![1, 2, 3]),
            cancel_download: Some(cancel_sender),
            cancel_requested: true,
            ..Default::default()
        };

        assert!(discard_update_session(&mut session).unwrap());
        assert_eq!(session.phase, AppUpdatePhase::Idle);
        assert!(session.pending.is_none());
        assert!(session.downloaded_bytes.is_none());
        assert!(session.cancel_download.is_none());
        assert!(!session.cancel_requested);
        assert_eq!(
            cancel_receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        );
    }

    #[test]
    fn discard_downloaded_update_releases_verified_bytes() {
        let mut session = AppUpdateSession {
            phase: AppUpdatePhase::Downloaded,
            downloaded_bytes: Some(vec![7; 1024]),
            ..Default::default()
        };

        assert!(discard_update_session(&mut session).unwrap());
        assert_eq!(session.phase, AppUpdatePhase::Idle);
        assert!(session.downloaded_bytes.is_none());
    }

    #[test]
    fn discard_idle_update_is_a_noop() {
        let mut session = AppUpdateSession::default();

        assert!(!discard_update_session(&mut session).unwrap());
        assert_eq!(session.phase, AppUpdatePhase::Idle);
    }

    #[test]
    fn discard_busy_update_is_retryable_and_does_not_mutate_the_session() {
        for phase in [
            AppUpdatePhase::Checking,
            AppUpdatePhase::Downloading,
            AppUpdatePhase::Installing,
        ] {
            let (cancel_sender, _cancel_receiver) = oneshot::channel();
            let mut session = AppUpdateSession {
                phase,
                downloaded_bytes: Some(vec![4, 5, 6]),
                cancel_download: Some(cancel_sender),
                cancel_requested: true,
                ..Default::default()
            };

            let error = discard_update_session(&mut session)
                .expect_err("an in-flight updater operation must not be discarded");
            assert_eq!(error.code, "updater-busy");
            assert!(error.retryable);
            assert_eq!(session.phase, phase);
            assert_eq!(
                session.downloaded_bytes.as_deref(),
                Some([4, 5, 6].as_slice())
            );
            assert!(session.cancel_download.is_some());
            assert!(session.cancel_requested);
        }
    }

    #[test]
    fn accepted_cancellation_wins_even_after_download_future_completes() {
        let (sender, _receiver) = oneshot::channel();
        let mut session = AppUpdateSession {
            phase: AppUpdatePhase::Downloading,
            cancel_download: Some(sender),
            ..Default::default()
        };

        assert!(request_download_cancellation(&mut session));
        let error = commit_download_result(&mut session, Ok(vec![1, 2, 3]))
            .expect_err("accepted cancellation must prevent the downloaded commit");
        assert_eq!(error.code, "update-download-cancelled");
        assert_eq!(session.phase, AppUpdatePhase::Available);
        assert!(session.downloaded_bytes.is_none());
        assert!(!session.cancel_requested);
    }

    #[test]
    fn update_package_size_limit_checks_headers_and_accumulated_chunks() {
        let mut received = 0;
        assert!(update_download_size_exceeded(
            &mut received,
            1,
            Some(MAX_UPDATE_PACKAGE_BYTES + 1)
        ));

        let mut received = MAX_UPDATE_PACKAGE_BYTES - 1;
        assert!(!update_download_size_exceeded(&mut received, 1, None));
        assert!(update_download_size_exceeded(&mut received, 1, None));
        assert_eq!(
            update_package_too_large_error().code,
            "update-package-too-large"
        );
        assert!(!update_package_too_large_error().retryable);
    }

    #[test]
    fn release_notes_are_bounded_and_strip_control_characters() {
        let notes = format!("说明\0\n{}", "x".repeat(MAX_RELEASE_NOTES_CHARS + 100));
        let sanitized = sanitize_release_notes(&notes);
        assert!(!sanitized.contains('\0'));
        assert!(sanitized.chars().count() <= MAX_RELEASE_NOTES_CHARS);
        assert_eq!(
            format_published_at(time::OffsetDateTime::UNIX_EPOCH).as_deref(),
            Some("1970-01-01T00:00:00Z")
        );
    }

    #[test]
    fn updater_errors_keep_network_signature_and_metadata_failures_distinct() {
        let offline = map_updater_error(UpdaterError::Network("offline".to_string()));
        assert_eq!(offline.code, "update-network-error");
        assert!(offline.retryable);

        let signature = map_updater_error(UpdaterError::Minisign(
            minisign_verify::Error::InvalidSignature,
        ));
        assert_eq!(signature.code, "update-signature-invalid");
        assert!(!signature.retryable);

        let metadata = map_updater_error(UpdaterError::InvalidUpdaterFormat);
        assert_eq!(metadata.code, "invalid-update-metadata");
        assert!(!metadata.retryable);
    }
}
