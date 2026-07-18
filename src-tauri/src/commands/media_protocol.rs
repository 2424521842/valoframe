use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use tauri::http::{
    self,
    header::{
        ACCEPT_RANGES, ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE,
        CONTENT_TYPE, RANGE,
    },
    Method, StatusCode,
};

use super::ClipMediaResponse;
use crate::{db, thumbnail};

const CLIP_MEDIA_PROTOCOL_PATH_PREFIX: &str = "clip";
const COVER_MEDIA_PROTOCOL_PATH_PREFIX: &str = "cover";
const CLIP_MEDIA_CONTENT_TYPE: &str = "video/mp4";
const COVER_MEDIA_CONTENT_TYPE: &str = "image/jpeg";
pub(super) const FILE_NOT_FOUND_MESSAGE: &str = "文件不存在";
pub(super) const MAX_MEDIA_CHUNK_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ByteRange {
    pub(super) start: u64,
    pub(super) end: u64,
}

pub fn clip_media_protocol_response(
    database_path: &str,
    thumbnail_cache_root: &Path,
    request: http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let media_path = percent_decode_path(request.uri().path().trim_start_matches('/'));

    if let Ok(clip_id) = media_id_from_path(&media_path, CLIP_MEDIA_PROTOCOL_PATH_PREFIX) {
        let clip = {
            let connection = match db::open_database_read_only(database_path) {
                Ok(connection) => connection,
                Err(message) => return text_response(StatusCode::INTERNAL_SERVER_ERROR, &message),
            };
            match db::find_clip_media_paths_by_id(&connection, clip_id) {
                Ok(clip) => clip,
                Err(message) => return text_response(StatusCode::NOT_FOUND, &message),
            }
        };

        if !clip.extension.eq_ignore_ascii_case("mp4") {
            return text_response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "不支持预览");
        }

        let range_header = request
            .headers()
            .get(RANGE)
            .and_then(|value| value.to_str().ok());
        return media_file_response(Path::new(&clip.video_path), request.method(), range_header);
    }

    if let Ok(clip_id) = media_id_from_path(&media_path, COVER_MEDIA_PROTOCOL_PATH_PREFIX) {
        let clip = {
            let connection = match db::open_database_read_only(database_path) {
                Ok(connection) => connection,
                Err(message) => return text_response(StatusCode::INTERNAL_SERVER_ERROR, &message),
            };
            match db::find_clip_media_paths_by_id(&connection, clip_id) {
                Ok(clip) => clip,
                Err(message) => return text_response(StatusCode::NOT_FOUND, &message),
            }
        };

        if clip.cover_source == "file" {
            if let Some(cover_path) = clip.cover_path.as_deref() {
                return cover_file_response(Path::new(cover_path), request.method(), false);
            }
        }

        let Some(cache_file) = clip.generated_cover_file.as_deref() else {
            return text_response(StatusCode::NOT_FOUND, FILE_NOT_FOUND_MESSAGE);
        };
        let generated_path =
            match thumbnail::resolve_ready_cache_file(thumbnail_cache_root, cache_file) {
                Ok(path) => path,
                Err(_) => return text_response(StatusCode::NOT_FOUND, FILE_NOT_FOUND_MESSAGE),
            };
        return cover_file_response(&generated_path, request.method(), true);
    }

    let message = clip_id_from_media_request(&request)
        .map(|_| "invalid clip media path".to_string())
        .unwrap_or_else(|error| error);

    text_response(StatusCode::BAD_REQUEST, &message)
}

fn cover_file_response(
    cover_path: &Path,
    method: &Method,
    validate_generated: bool,
) -> http::Response<Vec<u8>> {
    if method != Method::GET && method != Method::HEAD {
        return text_response(StatusCode::METHOD_NOT_ALLOWED, "不支持的请求方法");
    }
    let mut file = match File::open(cover_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return text_response(StatusCode::NOT_FOUND, FILE_NOT_FOUND_MESSAGE);
        }
        Err(error) => {
            return text_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("无法读取封面: {error}"),
            );
        }
    };

    let content_length = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return text_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("无法读取封面信息: {error}"),
            );
        }
    };
    if content_length > thumbnail::MAX_THUMBNAIL_BYTES {
        return text_response(StatusCode::PAYLOAD_TOO_LARGE, "封面文件过大");
    }
    if validate_generated && thumbnail::validate_generated_jpeg(cover_path).is_err() {
        return text_response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "缓存封面格式无效");
    }

    let body = if method == Method::HEAD {
        Vec::new()
    } else {
        let capacity = match usize::try_from(content_length) {
            Ok(capacity) => capacity,
            Err(_) => return text_response(StatusCode::PAYLOAD_TOO_LARGE, "封面文件过大"),
        };
        let mut body = Vec::with_capacity(capacity);
        if let Err(error) = file.read_to_end(&mut body) {
            return text_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("无法读取封面: {error}"),
            );
        }
        body
    };

    http::Response::builder()
        .status(StatusCode::OK)
        .header(ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(CONTENT_TYPE, COVER_MEDIA_CONTENT_TYPE)
        .header(CACHE_CONTROL, "no-cache")
        .header("x-content-type-options", "nosniff")
        .header(CONTENT_LENGTH, content_length)
        .body(body)
        .expect("cover response should build")
}

fn media_id_from_path(path: &str, prefix: &str) -> Result<i64, String> {
    let Some(raw_id) = path.strip_prefix(&format!("{prefix}/")) else {
        return Err("invalid clip media path".to_string());
    };

    if raw_id.is_empty() {
        return Err("missing clip id".to_string());
    }
    if raw_id.contains('/') {
        return Err("invalid clip media path".to_string());
    }

    raw_id
        .parse::<i64>()
        .map_err(|_| "invalid clip id".to_string())
}

pub(super) fn get_clip_media_for_database(
    database_path: impl AsRef<Path>,
    clip_id: i64,
) -> Result<ClipMediaResponse, String> {
    let connection = db::open_database_read_only(database_path)?;
    let clip = db::find_clip_media_paths_by_id(&connection, clip_id)?;

    if !clip_file_is_playable(&clip) {
        return Ok(ClipMediaResponse {
            clip_id,
            playable: false,
            media_path: None,
            message: Some(FILE_NOT_FOUND_MESSAGE.to_string()),
        });
    }

    Ok(ClipMediaResponse {
        clip_id,
        playable: true,
        media_path: Some(media_path_for_clip_id(clip.id)),
        message: None,
    })
}

pub(super) fn parse_media_range(
    range_header: Option<&str>,
    file_len: u64,
) -> Result<Option<ByteRange>, String> {
    let Some(range_header) = range_header
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let Some(range_spec) = range_header.strip_prefix("bytes=") else {
        return Err("invalid range unit".to_string());
    };

    if file_len == 0 || range_spec.contains(',') {
        return Err("range not satisfiable".to_string());
    }

    let Some((start_raw, end_raw)) = range_spec.split_once('-') else {
        return Err("invalid range".to_string());
    };

    if start_raw.is_empty() {
        let suffix_len = end_raw
            .parse::<u64>()
            .map_err(|_| "invalid range suffix".to_string())?;
        if suffix_len == 0 {
            return Err("range not satisfiable".to_string());
        }

        let length = suffix_len.min(file_len);
        return Ok(Some(ByteRange {
            start: file_len - length,
            end: file_len - 1,
        }));
    }

    let start = start_raw
        .parse::<u64>()
        .map_err(|_| "invalid range start".to_string())?;
    let end = if end_raw.is_empty() {
        file_len - 1
    } else {
        end_raw
            .parse::<u64>()
            .map_err(|_| "invalid range end".to_string())?
            .min(file_len - 1)
    };

    if start > end || start >= file_len {
        return Err("range not satisfiable".to_string());
    }

    Ok(Some(ByteRange { start, end }))
}

fn media_file_response(
    clip_path: &Path,
    method: &Method,
    range_header: Option<&str>,
) -> http::Response<Vec<u8>> {
    let mut file = match File::open(clip_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return text_response(StatusCode::NOT_FOUND, FILE_NOT_FOUND_MESSAGE);
        }
        Err(error) => {
            return text_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("无法读取文件: {error}"),
            );
        }
    };
    let file_len = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return text_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("无法读取文件信息: {error}"),
            );
        }
    };
    let range = match parse_media_range(range_header, file_len) {
        Ok(range) => range,
        Err(_) => return range_not_satisfiable_response(file_len),
    };

    match range {
        Some(range) => partial_content_response(&mut file, file_len, range, method),
        // Tauri buffers custom-protocol bodies, so a large no-Range bootstrap uses the same
        // bounded partial response that prompts media players to request subsequent ranges.
        None if file_len > MAX_MEDIA_CHUNK_BYTES => partial_content_response(
            &mut file,
            file_len,
            ByteRange {
                start: 0,
                end: file_len - 1,
            },
            method,
        ),
        None => full_content_response(&mut file, file_len, method),
    }
}

fn range_not_satisfiable_response(file_len: u64) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_RANGE, format!("bytes */{file_len}"))
        .header(CONTENT_LENGTH, 0)
        .body(Vec::new())
        .expect("range response should build")
}

fn partial_content_response(
    file: &mut File,
    file_len: u64,
    range: ByteRange,
    method: &Method,
) -> http::Response<Vec<u8>> {
    let range = ByteRange {
        start: range.start,
        end: range
            .end
            .min(range.start.saturating_add(MAX_MEDIA_CHUNK_BYTES - 1))
            .min(file_len - 1),
    };
    let content_len = range.end - range.start + 1;
    let body = if method == Method::HEAD {
        Vec::new()
    } else {
        match read_file_range(file, range.start, content_len) {
            Ok(body) => body,
            Err(error) => {
                return text_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("无法读取文件片段: {error}"),
                );
            }
        }
    };

    http::Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(CONTENT_TYPE, CLIP_MEDIA_CONTENT_TYPE)
        .header(ACCEPT_RANGES, "bytes")
        .header(
            CONTENT_RANGE,
            format!("bytes {}-{}/{}", range.start, range.end, file_len),
        )
        .header(CONTENT_LENGTH, content_len)
        .body(body)
        .expect("partial media response should build")
}

fn full_content_response(
    file: &mut File,
    file_len: u64,
    method: &Method,
) -> http::Response<Vec<u8>> {
    debug_assert!(file_len <= MAX_MEDIA_CHUNK_BYTES);
    let body = if method == Method::HEAD {
        Vec::new()
    } else {
        match read_file_range(file, 0, file_len) {
            Ok(body) => body,
            Err(error) => {
                return text_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("无法读取文件: {error}"),
                );
            }
        }
    };

    http::Response::builder()
        .status(StatusCode::OK)
        .header(ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(CONTENT_TYPE, CLIP_MEDIA_CONTENT_TYPE)
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_LENGTH, file_len)
        .body(body)
        .expect("media response should build")
}

fn read_file_range(file: &mut File, start: u64, length: u64) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(start))?;
    let length = usize::try_from(length).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "media range does not fit in memory",
        )
    })?;
    let mut body = vec![0; length];
    file.read_exact(&mut body)?;
    Ok(body)
}

fn text_response(status: StatusCode, message: &str) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header(ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(CACHE_CONTROL, "no-cache")
        .header("x-content-type-options", "nosniff")
        .body(message.as_bytes().to_vec())
        .expect("text response should build")
}

pub(super) fn clip_id_from_media_request(request: &http::Request<Vec<u8>>) -> Result<i64, String> {
    let path = percent_decode_path(request.uri().path().trim_start_matches('/'));
    media_id_from_path(&path, CLIP_MEDIA_PROTOCOL_PATH_PREFIX)
}

fn percent_decode_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }

        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn clip_file_is_playable(clip: &db::ClipMediaPaths) -> bool {
    clip.extension.eq_ignore_ascii_case("mp4") && Path::new(&clip.video_path).is_file()
}

fn media_path_for_clip_id(clip_id: i64) -> String {
    format!("{CLIP_MEDIA_PROTOCOL_PATH_PREFIX}/{clip_id}")
}
