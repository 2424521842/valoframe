//! User-consented issue feedback: sanitized diagnostics, optional sample frames and video,
//! delivered either by direct upload to the operator's HTTPS endpoint or as a local package.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{db, thumbnail, AppState};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const MAX_FEEDBACK_DESCRIPTION_CHARS: usize = 2000;
pub const MAX_FEEDBACK_CONTACT_CHARS: usize = 200;
pub const MAX_FEEDBACK_ENDPOINT_CHARS: usize = 300;
pub const MAX_FEEDBACK_VIDEO_BYTES: u64 = 1024 * 1024 * 1024;
pub const FEEDBACK_SCHEMA_VERSION: u32 = 1;
pub const FEEDBACK_UPLOAD_TIMEOUT_SECS: u64 = 15 * 60;

const SAMPLE_FRAME_RATIOS: [f64; 3] = [0.10, 0.50, 0.90];
const SAMPLE_FRAME_MIN_SEEK_SECONDS: f64 = 0.3;
const SAMPLE_FRAME_MIN_GAP_SECONDS: f64 = 1.0;
const SAMPLE_FRAME_MAX_WIDTH: u32 = 960;
const FFMPEG_FRAME_TIMEOUT: Duration = Duration::from_secs(60);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FEEDBACK_PROGRESS_EVENT: &str = "feedback-progress";
const FEEDBACK_CACHE_DIR_NAME: &str = "feedback";
const MIN_ENDPOINT_LENGTH: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeedbackCategory {
    Mismatch,
    Playback,
    Metadata,
    Other,
}

impl FeedbackCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mismatch => "mismatch",
            Self::Playback => "playback",
            Self::Metadata => "metadata",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitFeedbackInput {
    pub clip_id: i64,
    pub category: FeedbackCategory,
    pub description: String,
    pub contact: String,
    pub include_frames: bool,
    pub include_video: bool,
    pub endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSubmitResult {
    pub report_id: String,
    /// uploaded: package delivered to the configured endpoint; needs-save: package is waiting
    /// on disk for the frontend to offer the save dialog.
    pub status: String,
    pub package_path: Option<String>,
    pub suggested_file_name: Option<String>,
    pub total_bytes: u64,
    pub included_items: Vec<String>,
    pub message: String,
    pub upload_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackProgressEvent {
    pub report_id: String,
    pub phase: String,
    pub message: String,
    pub uploaded_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFeedbackPackageResult {
    pub destination_path: String,
    pub total_bytes: u64,
}

#[derive(Debug, Clone)]
struct ValidatedSubmitInput {
    clip_id: i64,
    category: FeedbackCategory,
    description: String,
    contact: String,
    include_frames: bool,
    include_video: bool,
    endpoint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackDiagnostic {
    schema_version: u32,
    generated_at: String,
    app_version: String,
    platform: String,
    clip: db::FeedbackClipSnapshot,
    sibling_clips: Vec<db::FeedbackSiblingClip>,
    file_check: FeedbackFileCheck,
    package: FeedbackPackageManifest,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackFileCheck {
    exists: bool,
    is_file: bool,
    size_bytes: Option<u64>,
    modified_at: Option<String>,
    indexed_size_bytes: i64,
    indexed_modified_at: Option<String>,
    size_matches: bool,
    modified_at_matches: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackPackageManifest {
    frames_requested: bool,
    video_requested: bool,
    ffmpeg_available: bool,
    frames_captured: usize,
    video_attached: bool,
    frame_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackReportMeta {
    schema_version: u32,
    report_id: String,
    app_version: String,
    platform: String,
    clip_id: i64,
    category: String,
    description: String,
    contact: String,
    submitted_at: String,
    package_file_name: String,
    package_bytes: u64,
}

struct BuiltFeedbackPackage {
    report_id: String,
    package_path: PathBuf,
    suggested_file_name: String,
    total_bytes: u64,
    included_items: Vec<String>,
    report_meta: FeedbackReportMeta,
}

#[tauri::command]
pub async fn submit_feedback(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SubmitFeedbackInput,
) -> Result<FeedbackSubmitResult, String> {
    let validated = validate_submit_input(&input)?;
    let database_path = state.database_path.clone();
    let report_id = generate_report_id();
    let app_version = app.package_info().version.to_string();
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| format!("无法解析应用资源目录：{error}"))?;
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法解析应用缓存目录：{error}"))?;

    let build_app = app.clone();
    let build_report_id = report_id.clone();
    let build_input = validated.clone();
    let built = tauri::async_runtime::spawn_blocking(move || {
        build_feedback_package(
            Some(&build_app),
            &build_report_id,
            &database_path,
            &resource_dir,
            &cache_root,
            &app_version,
            &build_input,
        )
    })
    .await
    .map_err(|error| format!("反馈任务异常终止：{error}"))??;

    let (status, message, upload_error, package_path);
    if validated.endpoint.is_empty() {
        status = "needs-save";
        message = "诊断包已生成，请选择保存位置后手动发送给开发者。".to_string();
        upload_error = None;
        package_path = Some(built.package_path.clone());
    } else {
        match upload_feedback_package(&app, &validated.endpoint, &built).await {
            Ok(()) => {
                let _ = fs::remove_file(&built.package_path);
                status = "uploaded";
                message = "问题反馈已上传，感谢你的帮助！".to_string();
                upload_error = None;
                package_path = None;
            }
            Err(error) => {
                status = "needs-save";
                message = format!("自动上传失败（{error}），已改为保存诊断包文件。");
                upload_error = Some(error.clone());
                package_path = Some(built.package_path.clone());
            }
        }
    }

    Ok(FeedbackSubmitResult {
        report_id: built.report_id.clone(),
        status: status.to_string(),
        package_path: package_path.map(|path| path.to_string_lossy().into_owned()),
        suggested_file_name: Some(built.suggested_file_name.clone()),
        total_bytes: built.total_bytes,
        included_items: built.included_items.clone(),
        message: message.to_string(),
        upload_error,
    })
}

#[tauri::command]
pub async fn save_feedback_package(
    app: AppHandle,
    package_path: String,
    destination_path: String,
) -> Result<SaveFeedbackPackageResult, String> {
    let package_path = validate_feedback_package_path(&app, &package_path)?;
    let destination = destination_path.trim();
    if destination.is_empty() {
        return Err("保存位置不能为空".to_string());
    }
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("保存位置必须是绝对路径".to_string());
    }
    let total_bytes = fs::copy(&package_path, &destination)
        .map_err(|error| format!("保存诊断包失败：{error}"))?;
    let _ = fs::remove_file(&package_path);
    Ok(SaveFeedbackPackageResult {
        destination_path: destination.to_string_lossy().into_owned(),
        total_bytes,
    })
}

#[tauri::command]
pub async fn discard_feedback_package(app: AppHandle, package_path: String) -> Result<(), String> {
    let package_path = validate_feedback_package_path(&app, &package_path)?;
    let _ = fs::remove_file(&package_path);
    Ok(())
}

fn validate_submit_input(input: &SubmitFeedbackInput) -> Result<ValidatedSubmitInput, String> {
    if input.clip_id <= 0 {
        return Err("无效的素材 ID".to_string());
    }
    let description = input.description.trim().to_string();
    if description.is_empty() {
        return Err("请描述你遇到的问题（例如：视频画面是另一局的内容）".to_string());
    }
    if description.chars().count() > MAX_FEEDBACK_DESCRIPTION_CHARS {
        return Err(format!(
            "问题描述不能超过 {MAX_FEEDBACK_DESCRIPTION_CHARS} 字"
        ));
    }
    let contact = input.contact.trim().to_string();
    if contact.chars().count() > MAX_FEEDBACK_CONTACT_CHARS {
        return Err(format!("联系方式不能超过 {MAX_FEEDBACK_CONTACT_CHARS} 字"));
    }
    let endpoint = normalize_feedback_endpoint(&input.endpoint)?;
    Ok(ValidatedSubmitInput {
        clip_id: input.clip_id,
        category: input.category,
        description,
        contact,
        include_frames: input.include_frames,
        include_video: input.include_video,
        endpoint,
    })
}

pub(crate) fn normalize_feedback_endpoint(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.chars().count() > MAX_FEEDBACK_ENDPOINT_CHARS {
        return Err(format!(
            "反馈接口地址不能超过 {MAX_FEEDBACK_ENDPOINT_CHARS} 字"
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    let https = lower.starts_with("https://");
    let local_http = lower.starts_with("http://localhost")
        || lower.starts_with("http://127.0.0.1")
        || lower.starts_with("http://[::1]");
    if !(https || local_http) {
        return Err("反馈接口必须以 https:// 开头；本机测试可使用 http://localhost".to_string());
    }
    if trimmed.len() < MIN_ENDPOINT_LENGTH {
        return Err("反馈接口地址不完整".to_string());
    }
    Ok(trimmed.to_string())
}

#[allow(clippy::too_many_arguments)]
fn build_feedback_package(
    app: Option<&AppHandle>,
    report_id: &str,
    database_path: &str,
    resource_dir: &Path,
    cache_root: &Path,
    app_version: &str,
    input: &ValidatedSubmitInput,
) -> Result<BuiltFeedbackPackage, String> {
    emit_progress(app, report_id, "building", "正在收集素材信息", 0, None);
    let connection = db::open_database_read_only(database_path)?;
    let snapshot = db::find_feedback_clip_snapshot_by_id(&connection, input.clip_id)?
        .ok_or_else(|| format!("未找到素材（ID {}）", input.clip_id))?;
    let sibling_clips = db::list_feedback_sibling_clips(
        &connection,
        snapshot.id,
        snapshot.clip_group_id,
        snapshot.match_id.as_deref(),
    )?;
    drop(connection);

    let video_path = PathBuf::from(&snapshot.video_path);
    let file_check = check_feedback_file(
        &video_path,
        snapshot.file_size,
        snapshot.modified_at.as_deref(),
    );

    let package_dir = feedback_package_dir(cache_root, report_id);
    let frames_dir = package_dir.join("frames");
    let mut included_items = vec!["诊断元数据（对局与素材信息）".to_string()];
    let mut frame_notes = Vec::new();
    let mut captured_frames: Vec<PathBuf> = Vec::new();
    let ffmpeg_executable = thumbnail::FfmpegThumbnailGenerator::resolve(resource_dir)
        .executable()
        .map(Path::to_path_buf);
    let ffmpeg_available = ffmpeg_executable.is_some();

    if input.include_frames {
        emit_progress(app, report_id, "building", "正在抽取视频采样帧", 0, None);
        match ffmpeg_executable.as_deref() {
            Some(ffmpeg) => {
                fs::create_dir_all(&frames_dir)
                    .map_err(|error| format!("无法创建采样帧目录：{error}"))?;
                let times = sample_frame_times(snapshot.duration_ms.map(|ms| ms as f64 / 1000.0));
                for (index, seek_seconds) in times.iter().enumerate() {
                    let output = frames_dir.join(format!("frame-{:02}.jpg", index + 1));
                    match extract_sample_frame(ffmpeg, &video_path, *seek_seconds, &output) {
                        Ok(_) => captured_frames.push(output),
                        Err(error) => frame_notes.push(format!("第 {} 帧失败：{error}", index + 1)),
                    }
                }
                if !captured_frames.is_empty() {
                    included_items.push(format!("{} 张采样帧", captured_frames.len()));
                }
            }
            None => frame_notes.push("未找到 FFmpeg，已跳过采样帧".to_string()),
        }
    }

    let mut video_attached = false;
    if input.include_video {
        if !file_check.exists || !file_check.is_file {
            frame_notes.push("源视频文件不存在，未附带视频本体".to_string());
        } else if file_check.size_bytes.unwrap_or(0) > MAX_FEEDBACK_VIDEO_BYTES {
            return Err("视频文件超过 1 GiB 上限，无法附带完整视频；请取消勾选后重试".to_string());
        } else {
            video_attached = true;
            included_items.push(format!("完整视频（{}）", snapshot.file_name));
        }
    }

    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("无法生成时间戳：{error}"))?;
    let platform = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let manifest = FeedbackPackageManifest {
        frames_requested: input.include_frames,
        video_requested: input.include_video,
        ffmpeg_available,
        frames_captured: captured_frames.len(),
        video_attached,
        frame_notes,
    };
    let diagnostic = FeedbackDiagnostic {
        schema_version: FEEDBACK_SCHEMA_VERSION,
        generated_at: generated_at.clone(),
        app_version: app_version.to_string(),
        platform: platform.clone(),
        clip: snapshot.clone(),
        sibling_clips,
        file_check,
        package: manifest,
    };

    let zip_name = format!(
        "valoframe-feedback-{}-{}.zip",
        input.clip_id,
        unix_seconds()
    );
    let package_path = package_dir.join(&zip_name);
    fs::create_dir_all(&package_dir).map_err(|error| format!("无法创建反馈临时目录：{error}"))?;

    emit_progress(app, report_id, "building", "正在打包诊断数据", 0, None);
    write_feedback_zip(
        &package_path,
        &diagnostic,
        &snapshot.file_name,
        &video_path,
        video_attached,
        &captured_frames,
    )?;
    if frames_dir.exists() {
        let _ = fs::remove_dir_all(&frames_dir);
    }

    let total_bytes = fs::metadata(&package_path)
        .map_err(|error| format!("无法读取诊断包大小：{error}"))?
        .len();
    let report_meta = FeedbackReportMeta {
        schema_version: FEEDBACK_SCHEMA_VERSION,
        report_id: report_id.to_string(),
        app_version: app_version.to_string(),
        platform,
        clip_id: input.clip_id,
        category: input.category.as_str().to_string(),
        description: input.description.clone(),
        contact: input.contact.clone(),
        submitted_at: generated_at,
        package_file_name: zip_name.clone(),
        package_bytes: total_bytes,
    };

    Ok(BuiltFeedbackPackage {
        report_id: report_id.to_string(),
        package_path,
        suggested_file_name: zip_name,
        total_bytes,
        included_items,
        report_meta,
    })
}

fn check_feedback_file(
    path: &Path,
    indexed_size: i64,
    indexed_modified_at: Option<&str>,
) -> FeedbackFileCheck {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            let size_bytes = Some(metadata.len());
            let modified_at = metadata.modified().ok().map(format_system_time);
            FeedbackFileCheck {
                exists: true,
                is_file: true,
                size_bytes,
                modified_at: modified_at.clone(),
                indexed_size_bytes: indexed_size,
                indexed_modified_at: indexed_modified_at.map(str::to_owned),
                size_matches: size_bytes == Some(indexed_size.max(0) as u64),
                modified_at_matches: same_second(modified_at.as_deref(), indexed_modified_at),
            }
        }
        Ok(_) => FeedbackFileCheck {
            exists: true,
            is_file: false,
            size_bytes: None,
            modified_at: None,
            indexed_size_bytes: indexed_size,
            indexed_modified_at: indexed_modified_at.map(str::to_owned),
            size_matches: false,
            modified_at_matches: false,
        },
        Err(_) => FeedbackFileCheck {
            exists: false,
            is_file: false,
            size_bytes: None,
            modified_at: None,
            indexed_size_bytes: indexed_size,
            indexed_modified_at: indexed_modified_at.map(str::to_owned),
            size_matches: false,
            modified_at_matches: false,
        },
    }
}

fn format_system_time(system_time: SystemTime) -> String {
    OffsetDateTime::from(system_time)
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::new())
}

fn same_second(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left_prefix = left.get(..19);
            let right_prefix = right.get(..19);
            left_prefix.is_some() && left_prefix == right_prefix
        }
        _ => false,
    }
}

fn sample_frame_times(duration_seconds: Option<f64>) -> Vec<f64> {
    let Some(duration) = duration_seconds.filter(|value| value.is_finite() && *value > 0.0) else {
        return vec![SAMPLE_FRAME_MIN_SEEK_SECONDS];
    };
    let mut times: Vec<f64> = SAMPLE_FRAME_RATIOS
        .iter()
        .map(|ratio| {
            (ratio * duration).clamp(
                SAMPLE_FRAME_MIN_SEEK_SECONDS,
                (duration - SAMPLE_FRAME_MIN_SEEK_SECONDS).max(SAMPLE_FRAME_MIN_SEEK_SECONDS),
            )
        })
        .collect();
    times.dedup_by(|left, right| (*left - *right).abs() < SAMPLE_FRAME_MIN_GAP_SECONDS);
    if times.is_empty() {
        times.push(0.0);
    }
    times
}

fn extract_sample_frame(
    ffmpeg: &Path,
    source: &Path,
    seek_seconds: f64,
    output: &Path,
) -> Result<u64, String> {
    let mut command = Command::new(ffmpeg);
    command
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(format!("{seek_seconds:.3}"))
        .arg("-i")
        .arg(source)
        .arg("-frames:v")
        .arg("1")
        .arg("-vf")
        .arg(format!(
            "scale={SAMPLE_FRAME_MAX_WIDTH}:-2:force_original_aspect_ratio=decrease"
        ))
        .arg("-q:v")
        .arg("5")
        .arg("-y")
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 FFmpeg：{error}"))?;
    let started_at = Instant::now();
    let status = loop {
        if started_at.elapsed() >= FFMPEG_FRAME_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("采样帧提取超时".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("等待 FFmpeg 失败：{error}"));
            }
        }
    };
    if !status.success() {
        return Err("FFmpeg 提取采样帧失败".to_string());
    }
    let mut header = [0u8; 2];
    let mut file = File::open(output).map_err(|error| format!("无法打开采样帧：{error}"))?;
    let read = file
        .read(&mut header)
        .map_err(|error| format!("无法读取采样帧：{error}"))?;
    if read < 2 || header != [0xFF, 0xD8] {
        return Err("采样帧输出不是有效的 JPEG".to_string());
    }
    Ok(file
        .metadata()
        .map_err(|error| format!("无法读取采样帧大小：{error}"))?
        .len())
}

#[allow(clippy::too_many_arguments)]
fn write_feedback_zip(
    zip_path: &Path,
    diagnostic: &FeedbackDiagnostic,
    video_file_name: &str,
    video_path: &Path,
    video_attached: bool,
    frames: &[PathBuf],
) -> Result<(), String> {
    let file = File::create(zip_path).map_err(|error| format!("无法创建诊断包：{error}"))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    let diagnostic_json = serde_json::to_string_pretty(diagnostic)
        .map_err(|error| format!("无法序列化诊断数据：{error}"))?;
    writer
        .start_file("diagnostic.json", options)
        .map_err(|error| format!("无法写入诊断包：{error}"))?;
    writer
        .write_all(diagnostic_json.as_bytes())
        .map_err(|error| format!("无法写入诊断包：{error}"))?;

    for (index, frame) in frames.iter().enumerate() {
        writer
            .start_file(format!("frames/frame-{:02}.jpg", index + 1), options)
            .map_err(|error| format!("无法写入诊断包：{error}"))?;
        let mut frame_file =
            File::open(frame).map_err(|error| format!("无法打开采样帧：{error}"))?;
        std::io::copy(&mut frame_file, &mut writer)
            .map_err(|error| format!("无法写入采样帧：{error}"))?;
    }

    if video_attached {
        writer
            .start_file(format!("video/{video_file_name}"), options)
            .map_err(|error| format!("无法写入诊断包：{error}"))?;
        let mut video =
            File::open(video_path).map_err(|error| format!("无法打开源视频：{error}"))?;
        std::io::copy(&mut video, &mut writer)
            .map_err(|error| format!("无法写入视频本体：{error}"))?;
    }

    writer
        .finish()
        .map_err(|error| format!("无法完成诊断包：{error}"))?;
    Ok(())
}

async fn upload_feedback_package(
    app: &AppHandle,
    endpoint: &str,
    built: &BuiltFeedbackPackage,
) -> Result<(), String> {
    emit_progress(
        Some(app),
        &built.report_id,
        "uploading",
        "正在上传诊断包",
        0,
        Some(built.total_bytes),
    );
    let report_json = serde_json::to_vec(&built.report_meta)
        .map_err(|error| format!("无法序列化反馈信息：{error}"))?;
    let progress = Arc::new(UploadProgress {
        app: app.clone(),
        report_id: built.report_id.clone(),
        total_bytes: built.total_bytes,
        uploaded: AtomicU64::new(0),
        last_percent: AtomicU64::new(0),
        last_emit_millis: Mutex::new(0),
    });
    let file = tokio::fs::File::open(&built.package_path)
        .await
        .map_err(|error| format!("无法打开诊断包：{error}"))?;
    let (boundary, prefix, tail) = multipart_body_parts(&report_json, &built.suggested_file_name);
    let reader = MultipartReader::new(prefix, tail, file, built.total_bytes, Arc::clone(&progress));
    let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(reader));
    // reqwest ships with no default crypto provider here; building a Client without this panics.
    crate::http_client::ensure_crypto_provider();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(FEEDBACK_UPLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("无法初始化上传客户端：{error}"))?;
    let response = client
        .post(endpoint)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .map_err(|error| format!("上传失败：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        let response_body = response.text().await.unwrap_or_default();
        let trimmed = response_body.trim();
        let mut detail = trimmed.chars().take(200).collect::<String>();
        if trimmed.chars().count() > 200 {
            detail.push('…');
        }
        return Err(if detail.is_empty() {
            format!("接口返回 {status}")
        } else {
            format!("接口返回 {status}：{detail}")
        });
    }
    progress.finish();
    Ok(())
}

/// Builds the raw multipart/form-data body: a JSON report part followed by the zip package
/// part. Returns the boundary, the prefix bytes (through the start of the file part), and the
/// trailing bytes.
fn multipart_body_parts(report_json: &[u8], file_name: &str) -> (String, Vec<u8>, Vec<u8>) {
    let boundary = format!("----ValoframeFeedback{}", unix_millis());
    let mut prefix = Vec::new();
    prefix.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    prefix.extend_from_slice(b"Content-Disposition: form-data; name=\"report\"\r\n");
    prefix.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
    prefix.extend_from_slice(report_json);
    prefix.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    prefix.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"package\"; filename=\"{file_name}\"\r\n")
            .as_bytes(),
    );
    prefix.extend_from_slice(b"Content-Type: application/zip\r\n\r\n");
    let tail = format!("\r\n--{boundary}--\r\n").into_bytes();
    (boundary, prefix, tail)
}

struct UploadProgress {
    app: AppHandle,
    report_id: String,
    total_bytes: u64,
    uploaded: AtomicU64,
    last_percent: AtomicU64,
    last_emit_millis: Mutex<u64>,
}

impl UploadProgress {
    fn add(&self, delta: u64) {
        let uploaded = self.uploaded.fetch_add(delta, Ordering::Relaxed) + delta;
        let denominator = self.total_bytes.max(1);
        let percent = uploaded.saturating_mul(100) / denominator;
        let last_percent = self.last_percent.load(Ordering::Relaxed);
        if percent <= last_percent {
            return;
        }
        let now_millis = unix_millis();
        let mut last_emit = self
            .last_emit_millis
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if percent - last_percent >= 1 || now_millis.saturating_sub(*last_emit) >= 500 {
            self.last_percent.store(percent, Ordering::Relaxed);
            *last_emit = now_millis;
            emit_progress(
                Some(&self.app),
                &self.report_id,
                "uploading",
                "正在上传诊断包",
                uploaded,
                Some(self.total_bytes),
            );
        }
    }

    fn finish(&self) {
        emit_progress(
            Some(&self.app),
            &self.report_id,
            "uploading",
            "正在上传诊断包",
            self.total_bytes,
            Some(self.total_bytes),
        );
    }
}

/// Streaming multipart body reader: emits the prefix (report part and file-part headers),
/// then the package file bytes with progress reporting, then the closing boundary.
struct MultipartReader {
    prefix: std::io::Cursor<Vec<u8>>,
    tail: std::io::Cursor<Vec<u8>>,
    file: Option<tokio::fs::File>,
    progress: Arc<UploadProgress>,
    file_remaining: u64,
    phase: MultipartPhase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MultipartPhase {
    Prefix,
    File,
    Tail,
    Done,
}

impl MultipartReader {
    fn new(
        prefix: Vec<u8>,
        tail: Vec<u8>,
        file: tokio::fs::File,
        file_length: u64,
        progress: Arc<UploadProgress>,
    ) -> Self {
        Self {
            prefix: std::io::Cursor::new(prefix),
            tail: std::io::Cursor::new(tail),
            file: Some(file),
            progress,
            file_remaining: file_length,
            phase: MultipartPhase::Prefix,
        }
    }
}

impl tokio::io::AsyncRead for MultipartReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        loop {
            let before = buffer.filled().len();
            match self.phase {
                MultipartPhase::Prefix => {
                    let poll = std::pin::Pin::new(&mut self.prefix).poll_read(context, buffer);
                    if let std::task::Poll::Ready(Ok(())) = poll {
                        if buffer.filled().len() == before {
                            self.phase = MultipartPhase::File;
                            continue;
                        }
                    }
                    return poll;
                }
                MultipartPhase::File => {
                    let Some(file) = self.file.as_mut() else {
                        self.phase = MultipartPhase::Tail;
                        continue;
                    };
                    let poll = std::pin::Pin::new(file).poll_read(context, buffer);
                    if let std::task::Poll::Ready(Ok(())) = poll {
                        let delta = buffer.filled().len().saturating_sub(before);
                        self.file_remaining = self.file_remaining.saturating_sub(delta as u64);
                        if delta > 0 {
                            self.progress.add(delta as u64);
                        }
                        if self.file_remaining == 0 {
                            self.file = None;
                            self.phase = MultipartPhase::Tail;
                            if buffer.filled().len() == before {
                                continue;
                            }
                        }
                    }
                    return poll;
                }
                MultipartPhase::Tail => {
                    let poll = std::pin::Pin::new(&mut self.tail).poll_read(context, buffer);
                    if let std::task::Poll::Ready(Ok(())) = poll {
                        if buffer.filled().len() == before {
                            self.phase = MultipartPhase::Done;
                            continue;
                        }
                    }
                    return poll;
                }
                MultipartPhase::Done => return std::task::Poll::Ready(Ok(())),
            }
        }
    }
}

fn emit_progress(
    app: Option<&AppHandle>,
    report_id: &str,
    phase: &str,
    message: &str,
    uploaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let Some(app) = app else { return };
    let event = FeedbackProgressEvent {
        report_id: report_id.to_string(),
        phase: phase.to_string(),
        message: message.to_string(),
        uploaded_bytes,
        total_bytes: total_bytes.unwrap_or(0),
    };
    if let Err(error) = app.emit(FEEDBACK_PROGRESS_EVENT, &event) {
        eprintln!("feedback-progress emit failed for {report_id} during {phase}: {error}");
    }
}

fn feedback_package_dir(cache_root: &Path, report_id: &str) -> PathBuf {
    cache_root.join(FEEDBACK_CACHE_DIR_NAME).join(report_id)
}

fn validate_feedback_package_path(app: &AppHandle, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("诊断包路径为空".to_string());
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() || !path.is_file() {
        return Err("诊断包文件不存在".to_string());
    }
    let canonical = fs::canonicalize(&path).map_err(|_| "无法解析诊断包路径".to_string())?;
    if canonical
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("zip")
    {
        return Err("诊断包必须是 zip 文件".to_string());
    }
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法解析应用缓存目录：{error}"))?;
    let feedback_root = fs::canonicalize(cache_root.join(FEEDBACK_CACHE_DIR_NAME))
        .map_err(|_| "反馈缓存目录不存在".to_string())?;
    if !canonical.starts_with(&feedback_root) {
        return Err("诊断包路径不在允许的反馈缓存目录内".to_string());
    }
    Ok(canonical)
}

fn generate_report_id() -> String {
    format!("vhm-{}-{}", std::process::id(), unix_millis())
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::db::{self, ClipGroupInput, ClipInput, SourceDirInput};

    fn raw_input() -> SubmitFeedbackInput {
        SubmitFeedbackInput {
            clip_id: 1,
            category: FeedbackCategory::Mismatch,
            description: "画面不是当前对局".to_string(),
            contact: String::new(),
            include_frames: true,
            include_video: true,
            endpoint: String::new(),
        }
    }

    fn fixture_input(clip_id: i64) -> ValidatedSubmitInput {
        ValidatedSubmitInput {
            clip_id,
            category: FeedbackCategory::Mismatch,
            description: "画面不是当前对局".to_string(),
            contact: String::new(),
            include_frames: true,
            include_video: true,
            endpoint: String::new(),
        }
    }

    struct FeedbackFixture {
        root: PathBuf,
        source_dir: PathBuf,
        source_id: i64,
        database_path: PathBuf,
        cache_root: PathBuf,
    }

    impl FeedbackFixture {
        fn new() -> Self {
            let root = unique_temp_dir();
            fs::create_dir_all(&root).expect("fixture root should be created");
            let database_path = root.join("highlight-index.sqlite3");
            db::migrate_database(&database_path).expect("database should migrate");
            let source_dir = root.join("source-a");
            fs::create_dir_all(&source_dir).expect("source should be created");
            let connection = db::open_database(&database_path).expect("database should open");
            let source = db::upsert_source_dir(
                &connection,
                SourceDirInput {
                    path: source_dir.to_string_lossy().as_ref(),
                    name: "FixtureSource",
                },
            )
            .expect("source should upsert");
            let cache_root = root.join("cache");
            fs::create_dir_all(&cache_root).expect("cache should be created");
            Self {
                root,
                source_dir,
                source_id: source.id,
                database_path,
                cache_root,
            }
        }

        fn add_group(&self, key: &str) -> i64 {
            let connection = db::open_database(&self.database_path).expect("database should open");
            db::upsert_clip_group(
                &connection,
                ClipGroupInput {
                    source_dir_id: self.source_id,
                    group_key: key,
                    display_name: "FixtureGroup",
                },
            )
            .expect("group should upsert")
            .id
        }

        fn add_clip(&self, group_id: Option<i64>, file_name: &str, bytes: Option<&[u8]>) -> i64 {
            let clip_path = self.source_dir.join(file_name);
            if let Some(bytes) = bytes {
                fs::write(&clip_path, bytes).expect("clip should be written");
            }
            let connection = db::open_database(&self.database_path).expect("database should open");
            db::upsert_clip(
                &connection,
                ClipInput {
                    source_dir_id: self.source_id,
                    clip_group_id: group_id,
                    video_path: clip_path.to_string_lossy().as_ref(),
                    file_name,
                    file_size: bytes.map(|value| value.len() as i64).unwrap_or(5),
                    modified_at: None,
                    duration_ms: Some(30_000),
                    recorded_at: None,
                    cover_path: None,
                    cover_source: "missing",
                },
            )
            .expect("clip should upsert")
            .id
        }
    }

    impl Drop for FeedbackFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn unique_temp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("vhm-feedback-test-{}-{unique}", std::process::id()))
    }

    #[test]
    fn endpoint_normalization_accepts_https_and_localhost_http_only() {
        assert_eq!(normalize_feedback_endpoint("").unwrap(), "");
        assert_eq!(normalize_feedback_endpoint("  ").unwrap(), "");
        assert_eq!(
            normalize_feedback_endpoint("https://example.com/api/feedback").unwrap(),
            "https://example.com/api/feedback",
        );
        assert_eq!(
            normalize_feedback_endpoint("http://localhost:8080/feedback").unwrap(),
            "http://localhost:8080/feedback",
        );
        assert_eq!(
            normalize_feedback_endpoint("http://127.0.0.1/feedback").unwrap(),
            "http://127.0.0.1/feedback",
        );
        assert_eq!(
            normalize_feedback_endpoint("http://[::1]/feedback").unwrap(),
            "http://[::1]/feedback",
        );

        assert!(normalize_feedback_endpoint("http://example.com/feedback").is_err());
        assert!(normalize_feedback_endpoint("ftp://example.com/feedback").is_err());
        assert!(normalize_feedback_endpoint("https://").is_err());
        let too_long = format!("https://example.com/{}", "a".repeat(300));
        assert!(normalize_feedback_endpoint(&too_long).is_err());
    }

    #[test]
    fn input_validation_enforces_field_limits() {
        let mut input = raw_input();
        input.description = "   ".to_string();
        assert!(validate_submit_input(&input).is_err());

        input.description = "长".repeat(2001);
        assert!(validate_submit_input(&input).is_err());
        input.description = "长".repeat(2000);
        assert!(validate_submit_input(&input).is_ok());

        input.contact = "联".repeat(201);
        assert!(validate_submit_input(&input).is_err());
        input.contact = String::new();

        input.clip_id = 0;
        assert!(validate_submit_input(&input).is_err());
        input.clip_id = 1;

        input.endpoint = "http://example.com/x".to_string();
        assert!(validate_submit_input(&input).is_err());
    }

    #[test]
    fn package_zip_is_sanitized_and_carries_requested_attachments() {
        let fixture = FeedbackFixture::new();
        let group_id = fixture.add_group("match-1");
        let first_id = fixture.add_clip(Some(group_id), "测试视频.mp4", Some(b"fake video bytes"));
        let _sibling_id = fixture.add_clip(Some(group_id), "sibling.mp4", Some(b"more video"));
        let input = fixture_input(first_id);

        let built = build_feedback_package(
            None,
            "test-report",
            &fixture.database_path.to_string_lossy(),
            &fixture.root,
            &fixture.cache_root,
            "0.0.0",
            &input,
        )
        .expect("package should build without ffmpeg");

        let archive_file = File::open(&built.package_path).expect("package should exist");
        let mut archive = zip::ZipArchive::new(archive_file).expect("package should be a zip");
        let names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "diagnostic.json"));
        assert!(names.iter().any(|name| name == "video/测试视频.mp4"));

        let mut diagnostic_file = archive
            .by_name("diagnostic.json")
            .expect("diagnostic entry should exist");
        let mut raw = String::new();
        diagnostic_file
            .read_to_string(&mut raw)
            .expect("diagnostic should be readable");
        let value: serde_json::Value =
            serde_json::from_str(&raw).expect("diagnostic should be valid JSON");
        for forbidden in [
            "openid",
            "note",
            "extractedText",
            "filePath",
            "videoPath",
            "matchAccountId",
            "normalizedPath",
        ] {
            assert!(
                !raw.contains(forbidden),
                "diagnostic must not contain {forbidden}"
            );
        }
        assert_eq!(value["clip"]["fileName"], "测试视频.mp4");
        assert_eq!(value["clip"]["sourceDirDisplayName"], "FixtureSource");
        assert_eq!(value["siblingClips"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["fileCheck"]["exists"], true);
        assert_eq!(value["fileCheck"]["sizeMatches"], true);
        assert_eq!(value["package"]["videoAttached"], true);
        assert_eq!(value["package"]["ffmpegAvailable"], false);
        assert_eq!(value["package"]["framesCaptured"], 0);
        assert!(built
            .included_items
            .iter()
            .any(|item| item.contains("完整视频")));
    }

    #[test]
    fn missing_source_video_degrades_to_notes_without_failing() {
        let fixture = FeedbackFixture::new();
        let clip_id = fixture.add_clip(None, "gone.mp4", None);
        let input = fixture_input(clip_id);

        let built = build_feedback_package(
            None,
            "test-report",
            &fixture.database_path.to_string_lossy(),
            &fixture.root,
            &fixture.cache_root,
            "0.0.0",
            &input,
        )
        .expect("missing video should degrade instead of failing");

        let archive_file = File::open(&built.package_path).unwrap();
        let mut archive = zip::ZipArchive::new(archive_file).unwrap();
        assert!(archive.by_name("diagnostic.json").is_ok());
        assert!(archive.by_name("video/gone.mp4").is_err());
        let mut diagnostic_file = archive.by_name("diagnostic.json").unwrap();
        let mut raw = String::new();
        diagnostic_file.read_to_string(&mut raw).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["fileCheck"]["exists"], false);
        assert_eq!(value["package"]["videoAttached"], false);
        assert!(!value["package"]["frameNotes"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sample_frame_times_cover_head_middle_tail_and_fallback() {
        assert_eq!(
            sample_frame_times(None),
            vec![SAMPLE_FRAME_MIN_SEEK_SECONDS]
        );
        let times = sample_frame_times(Some(100.0));
        assert_eq!(times.len(), 3);
        assert!((times[0] - 10.0).abs() < 1e-6);
        assert!((times[1] - 50.0).abs() < 1e-6);
        assert!((times[2] - 90.0).abs() < 1e-6);
        let short = sample_frame_times(Some(1.0));
        assert!(!short.is_empty());
        assert!(short.iter().all(|time| time.is_finite()));
    }
}
