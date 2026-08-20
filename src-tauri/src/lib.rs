mod app_updates;
mod commands;
mod critical_tasks;
pub mod db;
pub mod display_names;
pub mod drive_discovery;
pub(crate) mod file_identity;
pub mod highlight_log_parser;
pub mod leveldb_reader;
pub mod metadata;
pub mod metadata_ingest;
pub mod scan_coordinator;
pub mod scanner;
mod thumbnail;
pub mod wonderful_db;
pub mod wonderful_ingest;

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tauri::Manager;

const RELEASE_SMOKE_ROOT_ENV: &str = "VHM_RELEASE_SMOKE_ROOT";
const RELEASE_SMOKE_ROOT_PREFIX: &str = "vhm-release-smoke-";
const RELEASE_SMOKE_MARKER_FILE: &str = ".vhm-release-smoke-root";
const RELEASE_SMOKE_MARKER_CONTENT: &str = "vhm-release-smoke-root-v1";

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) database_path: String,
    pub(crate) scan_coordinator: Arc<scan_coordinator::ScanCoordinator>,
    pub(crate) thumbnail_queue: Arc<thumbnail::ThumbnailQueue>,
    pub(crate) critical_tasks: Arc<critical_tasks::CriticalTaskGate>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let startup_recovery_dir = Arc::new(Mutex::new(None::<PathBuf>));
    let startup_recovery_dir_for_setup = Arc::clone(&startup_recovery_dir);
    let primary_window = Arc::new(Mutex::new(None::<tauri::WebviewWindow>));
    let primary_window_for_single_instance = Arc::clone(&primary_window);
    let primary_window_for_setup = Arc::clone(&primary_window);
    let app = tauri::Builder::default()
        .register_uri_scheme_protocol("clip-media", |context, request| {
            let state = context.app_handle().state::<AppState>();
            commands::clip_media_protocol_response(
                &state.database_path,
                state.thumbnail_queue.cache_root(),
                request,
            )
        })
        .plugin(tauri_plugin_single_instance::init(move |app, _args, _cwd| {
            // On Windows the plugin callback is entered re-entrantly from its WM_COPYDATA
            // coordination window. Queue UI work onto Tauri's event loop instead of mutating the
            // webview window from inside that window procedure.
            let primary_window = Arc::clone(&primary_window_for_single_instance);
            if let Err(error) = app.run_on_main_thread(move || {
                let window = primary_window
                    .lock()
                    .ok()
                    .and_then(|window| window.as_ref().cloned());
                let Some(window) = window else {
                    eprintln!("Single-instance handoff could not access the primary window handle.");
                    return;
                };
                if let Err(error) = window.show() {
                    eprintln!("Single-instance handoff could not show the main window: {error}");
                }
                if let Err(error) = window.unminimize() {
                    eprintln!(
                        "Single-instance handoff could not restore the main window: {error}"
                    );
                }
                if let Err(error) = window.set_focus() {
                    eprintln!("Single-instance handoff could not focus the main window: {error}");
                }
            }) {
                eprintln!("Single-instance handoff could not queue main-window recovery: {error}");
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            let release_smoke_root =
                resolve_release_smoke_root(app.handle()).map_err(std::io::Error::other)?;
            let (data_dir, cache_root) = match release_smoke_root.as_ref() {
                Some(root) => (
                    root.join("data"),
                    root.join("cache").join("thumbnails"),
                ),
                None => (
                    app.path()
                        .app_data_dir()
                        .map_err(std::io::Error::other)?,
                    app.path()
                        .app_cache_dir()
                        .map_err(std::io::Error::other)?
                        .join("thumbnails"),
                ),
            };
            if let Ok(mut recovery_dir) = startup_recovery_dir_for_setup.lock() {
                *recovery_dir = Some(data_dir.clone());
            }
            let database_path =
                db::initialize_database_in(&data_dir).map_err(std::io::Error::other)?;
            {
                let connection =
                    db::open_database(&database_path).map_err(std::io::Error::other)?;
                scanner::recover_interrupted_scan_runs(&connection)
                    .map_err(std::io::Error::other)?;
                let delete_recovery = db::recover_pending_clip_deletions(&connection)
                    .map_err(std::io::Error::other)?;
                if delete_recovery.attempted > 0 {
                    eprintln!(
                        "Permanent-delete recovery attempted {}, completed {}, pending {}, blocked {}, failures {}.",
                        delete_recovery.attempted,
                        delete_recovery.deleted_ids.len(),
                        delete_recovery.pending_ids.len(),
                        delete_recovery.blocked_ids.len(),
                        delete_recovery.failures.len(),
                    );
                    for failure in &delete_recovery.failures {
                        eprintln!(
                            "Permanent-delete recovery for clip {} could not converge: {}",
                            failure.clip_id, failure.message
                        );
                    }
                }
            }
            let resource_dir = app.path().resource_dir().map_err(std::io::Error::other)?;
            let generator = Arc::new(thumbnail::FfmpegThumbnailGenerator::resolve(&resource_dir));
            let thumbnail_queue = thumbnail::ThumbnailQueue::start(
                app.handle().clone(),
                database_path.clone(),
                cache_root,
                generator,
            )
            .map_err(std::io::Error::other)?;

            let app_state = AppState {
                database_path: database_path.display().to_string(),
                scan_coordinator: Arc::new(scan_coordinator::ScanCoordinator::default()),
                thumbnail_queue,
                critical_tasks: Arc::new(critical_tasks::CriticalTaskGate::default()),
            };
            app.manage(app_state);
            app.manage(app_updates::AppUpdateState::default());

            let window_config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or_else(|| std::io::Error::other("main window configuration is missing"))?;
            let mut window_builder =
                tauri::WebviewWindowBuilder::from_config(app.handle(), &window_config)?;
            if let Some(root) = release_smoke_root {
                window_builder = window_builder.data_directory(root.join("webview2"));
            }
            // Do not expose a Win32 HWND until Tauri has finished building the webview and the
            // single-instance callback owns a stable handle. Otherwise an immediate second launch
            // can arrive in the visible-HWND / unregistered-window race.
            let window = window_builder.visible(false).build()?;
            *primary_window_for_setup
                .lock()
                .map_err(|_| std::io::Error::other("primary window state lock is poisoned"))? =
                Some(window.clone());
            window.show()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping_backend,
            commands::scan_default_aclos_dir,
            commands::scan_custom_dir,
            commands::scan_roots,
            commands::discover_and_scan_fixed_drives,
            commands::get_scan_status,
            commands::cancel_scan,
            commands::get_scan_summary,
            commands::ensure_clip_thumbnails,
            commands::retry_clip_thumbnails,
            commands::get_thumbnail_status,
            commands::list_clips,
            commands::list_clip_page,
            commands::list_review_clip_page,
            commands::get_library_facets,
            commands::get_clip_detail,
            commands::list_sources,
            commands::register_scan_source,
            commands::set_scan_source_enabled,
            commands::preview_scan_source_relocation,
            commands::relocate_scan_source,
            commands::sync_scan_source,
            commands::sync_enabled_sources,
            commands::request_startup_source_sync,
            commands::list_pending_manual_clips,
            commands::import_pending_manual_clip,
            commands::set_pending_manual_clip_ignored,
            commands::list_tags,
            commands::create_tag,
            commands::update_tag,
            commands::delete_tag,
            commands::set_clip_favorite,
            commands::set_clips_favorite,
            commands::set_clip_review_decision,
            commands::reset_clip_review_decision,
            commands::restore_clip_review_state,
            commands::set_clip_trashed,
            commands::set_clips_trashed,
            commands::remove_clip_from_index,
            commands::remove_clips_from_index,
            commands::delete_clips_permanently,
            commands::update_clip_note,
            commands::add_tag_to_clip,
            commands::add_tag_to_clips,
            commands::remove_tag_from_clip,
            commands::remove_tag_from_clips,
            commands::get_clip_media,
            commands::open_clip_location,
            commands::open_clip_externally,
            commands::copy_clip_path,
            commands::export_clips,
            commands::submit_feedback,
            commands::save_feedback_package,
            commands::discard_feedback_package,
            app_updates::get_app_update_runtime_info,
            app_updates::check_for_app_update,
            app_updates::download_app_update,
            app_updates::cancel_app_update_download,
            app_updates::discard_app_update,
            app_updates::install_app_update
        ])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(error) => {
            let recovery_dir = startup_recovery_dir
                .lock()
                .ok()
                .and_then(|path| path.clone());
            show_startup_failure(&error.to_string(), recovery_dir.as_deref());
            return;
        }
    };

    app.run(|app_handle, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            app_handle.state::<AppState>().thumbnail_queue.shutdown();
        }
    });
}

fn show_startup_failure(error: &str, recovery_dir: Option<&Path>) {
    let presentation = startup_failure_presentation(error, recovery_dir);
    let message = presentation.message;

    eprintln!("{message}");

    #[cfg(windows)]
    {
        if env::var_os(RELEASE_SMOKE_ROOT_ENV).is_some() {
            return;
        }

        use std::{os::windows::ffi::OsStrExt, process::Command};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_OK, MB_TASKMODAL, MB_YESNO,
        };

        let wide_message = std::ffi::OsStr::new(&message)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let wide_title = std::ffi::OsStr::new(presentation.title)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let style = if presentation.offer_recovery_directory {
            MB_YESNO | MB_ICONERROR | MB_DEFBUTTON2 | MB_TASKMODAL
        } else {
            MB_OK | MB_ICONERROR | MB_TASKMODAL
        };
        let response = unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide_message.as_ptr(),
                wide_title.as_ptr(),
                style,
            )
        };
        if presentation.offer_recovery_directory && response == IDYES {
            if let Some(path) = recovery_dir {
                let _ = Command::new("explorer.exe").arg(path).spawn();
            }
        }
    }
}

struct StartupFailurePresentation {
    title: &'static str,
    message: String,
    offer_recovery_directory: bool,
}

fn startup_failure_presentation(
    error: &str,
    recovery_dir: Option<&Path>,
) -> StartupFailurePresentation {
    let error_summary = error.chars().take(2_000).collect::<String>();
    if let Some(path) = recovery_dir {
        StartupFailurePresentation {
            title: "瓦刻 · 数据库恢复",
            message: format!(
                "瓦刻无法安全打开。\n\n错误：{error_summary}\n\n为避免继续损坏数据，瓦刻已停止启动且不会自动覆盖数据库。迁移前备份位于：\n{}\n\n选择“是”可打开该目录；请保留数据库及 backups 子目录后再进行恢复。",
                path.display()
            ),
            offer_recovery_directory: true,
        }
    } else {
        StartupFailurePresentation {
            title: "瓦刻 · 启动失败",
            message: format!(
                "瓦刻在数据库初始化前启动失败。\n\n错误：{error_summary}\n\n应用数据库和原始视频未被修改。请重新启动应用；若问题持续，请检查安装文件或联系维护者。"
            ),
            offer_recovery_directory: false,
        }
    }
}

/// Returns an isolated runtime root for an explicitly armed release smoke test.
///
/// Tauri resolves Windows application directories through `SHGetKnownFolderPath`, so changing
/// `APPDATA` or `LOCALAPPDATA` in a child process does not isolate the database or thumbnail cache.
/// This override is deliberately narrow and fail-closed: the directory must already exist, have a
/// distinctive generated name, and contain a marker with the exact expected content.
fn resolve_release_smoke_root(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    let Some(raw_root) = env::var_os(RELEASE_SMOKE_ROOT_ENV) else {
        return Ok(None);
    };

    let requested_root = PathBuf::from(raw_root);
    if !requested_root.is_absolute() {
        return Err(format!("{RELEASE_SMOKE_ROOT_ENV} must be an absolute path"));
    }

    let requested_metadata = fs::symlink_metadata(&requested_root).map_err(|error| {
        format!(
            "reading {RELEASE_SMOKE_ROOT_ENV} '{}' failed: {error}",
            requested_root.display()
        )
    })?;
    if metadata_is_reparse_point(&requested_metadata) {
        return Err(format!(
            "{RELEASE_SMOKE_ROOT_ENV} must not be a symbolic link or reparse point"
        ));
    }

    let root = requested_root.canonicalize().map_err(|error| {
        format!(
            "canonicalizing {RELEASE_SMOKE_ROOT_ENV} '{}' failed: {error}",
            requested_root.display()
        )
    })?;
    let protected_paths = [app.path().app_data_dir(), app.path().app_cache_dir()]
        .map(|path| {
            path.map_err(|error| error.to_string())
                .and_then(|path| canonicalize_allow_missing(&path))
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    validate_release_smoke_root(&root, &protected_paths)?;

    Ok(Some(root))
}

fn validate_release_smoke_root(root: &Path, protected_paths: &[PathBuf]) -> Result<(), String> {
    let has_expected_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(RELEASE_SMOKE_ROOT_PREFIX));
    if !has_expected_name {
        return Err(format!(
            "{RELEASE_SMOKE_ROOT_ENV} must name a '{RELEASE_SMOKE_ROOT_PREFIX}*' directory"
        ));
    }

    let metadata = fs::metadata(root).map_err(|error| {
        format!(
            "reading {RELEASE_SMOKE_ROOT_ENV} '{}' failed: {error}",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!("{RELEASE_SMOKE_ROOT_ENV} must be a directory"));
    }

    let marker_path = root.join(RELEASE_SMOKE_MARKER_FILE);
    let entries = fs::read_dir(root)
        .map_err(|error| {
            format!(
                "reading release smoke root '{}' failed: {error}",
                root.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "reading release smoke root '{}' failed: {error}",
                root.display()
            )
        })?;
    if entries.len() != 1 || entries[0].file_name() != RELEASE_SMOKE_MARKER_FILE {
        return Err(format!(
            "{RELEASE_SMOKE_ROOT_ENV} must be fresh and contain only {RELEASE_SMOKE_MARKER_FILE}"
        ));
    }

    let marker_metadata = fs::symlink_metadata(&marker_path).map_err(|error| {
        format!(
            "reading release smoke marker '{}' failed: {error}",
            marker_path.display()
        )
    })?;
    if !marker_metadata.is_file() || metadata_is_reparse_point(&marker_metadata) {
        return Err(format!(
            "release smoke marker '{}' must be a regular non-reparse file",
            marker_path.display()
        ));
    }
    let marker = fs::read_to_string(&marker_path).map_err(|error| {
        format!(
            "reading release smoke marker '{}' failed: {error}",
            marker_path.display()
        )
    })?;
    if marker.trim() != RELEASE_SMOKE_MARKER_CONTENT {
        return Err(format!(
            "release smoke marker '{}' has unexpected content",
            marker_path.display()
        ));
    }

    for protected_path in protected_paths {
        if root == protected_path
            || root.starts_with(protected_path)
            || protected_path.starts_with(root)
        {
            return Err(format!(
                "{RELEASE_SMOKE_ROOT_ENV} must not overlap the real application directory '{}'",
                protected_path.display()
            ));
        }
    }

    Ok(())
}

fn canonicalize_allow_missing(path: &Path) -> Result<PathBuf, String> {
    let mut cursor = path.to_path_buf();
    let mut missing_segments = Vec::new();

    loop {
        match cursor.canonicalize() {
            Ok(mut canonical) => {
                for segment in missing_segments.iter().rev() {
                    canonical.push(segment);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let segment = cursor.file_name().ok_or_else(|| {
                    format!("cannot canonicalize protected path '{}'", path.display())
                })?;
                missing_segments.push(segment.to_os_string());
                cursor = cursor
                    .parent()
                    .ok_or_else(|| {
                        format!("cannot canonicalize protected path '{}'", path.display())
                    })?
                    .to_path_buf();
            }
            Err(error) => {
                return Err(format!(
                    "canonicalizing protected path '{}' failed: {error}",
                    path.display()
                ));
            }
        }
    }
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
mod release_smoke_tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn startup_failures_before_database_initialization_do_not_claim_database_damage() {
        let presentation =
            startup_failure_presentation("failed to initialize plugin `updater`", None);

        assert_eq!(presentation.title, "瓦刻 · 启动失败");
        assert!(!presentation.offer_recovery_directory);
        assert!(presentation.message.contains("数据库初始化前"));
        assert!(presentation.message.contains("数据库和原始视频未被修改"));
        assert!(!presentation.message.contains("迁移前备份"));
    }

    #[test]
    fn database_failures_keep_the_recovery_directory_action() {
        let recovery_dir = Path::new("fixture-data");
        let presentation =
            startup_failure_presentation("database migration failed", Some(recovery_dir));

        assert_eq!(presentation.title, "瓦刻 · 数据库恢复");
        assert!(presentation.offer_recovery_directory);
        assert!(presentation.message.contains("迁移前备份"));
        assert!(presentation.message.contains("fixture-data"));
    }

    #[test]
    fn release_smoke_root_is_marker_gated_and_rejects_protected_paths() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after the Unix epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "{RELEASE_SMOKE_ROOT_PREFIX}{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("release smoke fixture should be created");
        let root = root
            .canonicalize()
            .expect("release smoke fixture should canonicalize");

        let missing_marker = validate_release_smoke_root(&root, &[])
            .expect_err("an unarmed release smoke root must be rejected");
        assert!(missing_marker.contains("must be fresh"));

        fs::write(
            root.join(RELEASE_SMOKE_MARKER_FILE),
            RELEASE_SMOKE_MARKER_CONTENT,
        )
        .expect("release smoke marker should be written");
        validate_release_smoke_root(&root, &[])
            .expect("an explicitly armed release smoke root should be accepted");

        let unexpected_path = root.join("unexpected");
        fs::write(&unexpected_path, "must be rejected")
            .expect("unexpected fixture entry should be written");
        let not_fresh = validate_release_smoke_root(&root, &[])
            .expect_err("a non-fresh release smoke root must be rejected");
        assert!(not_fresh.contains("must be fresh"));
        fs::remove_file(&unexpected_path).expect("unexpected fixture entry should be removed");

        let protected = root.join("data");
        let overlap = validate_release_smoke_root(&root, &[protected])
            .expect_err("a root that contains a protected path must be rejected");
        assert!(overlap.contains("must not overlap"));

        fs::remove_dir_all(&root).expect("release smoke fixture should be removed");
    }
}
