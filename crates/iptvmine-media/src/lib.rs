pub mod ffmpeg;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Router,
};
pub use ffmpeg::{
    check_ffmpeg_status_with_upstream, detect_ffmpeg_status, install_latest_ffmpeg, probe_media,
    resolve_ffmpeg_exe, resolve_ffprobe_exe, FfmpegStatus, MediaAudioTrack, MediaProbeResult,
    MediaSubtitleTrack,
};
use iptvmine_core::is_valid_stream_scheme;
use reqwest::header::{HeaderMap as ReqHeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, ReadBuf};
use tokio::process::{Child, ChildStdout, Command as TokioCommand};
use tokio_util::io::ReaderStream;
use url::Url;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("invalid stream URL: {0}")]
    InvalidUrl(String),
    #[error("external player not found; install VLC or mpv or configure path in Settings")]
    ExternalPlayerNotFound,
    #[error("ffmpeg error: {0}")]
    Ffmpeg(String),
    #[error("failed to launch media process: {0}")]
    Spawn(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
struct ProxySession {
    base_url: String,
    headers: HashMap<String, String>,
}

#[derive(Clone)]
pub struct StreamProxy {
    port: u16,
    sessions: Arc<RwLock<HashMap<String, ProxySession>>>,
    custom_ffmpeg_path: Arc<RwLock<Option<String>>>,
}

#[derive(Clone)]
struct ProxyAppState {
    client: Client,
    sessions: Arc<RwLock<HashMap<String, ProxySession>>>,
    custom_ffmpeg_path: Arc<RwLock<Option<String>>>,
    transcode_errors: Arc<RwLock<HashMap<String, String>>>,
}

#[derive(Debug, Deserialize)]
struct StreamQuery {
    session: String,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscodeQuery {
    session: String,
    audio_track: Option<usize>,
    start_sec: Option<f64>,
    copy_video: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SubtitleQuery {
    session: String,
    track: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxiedStream {
    pub proxy_url: String,
    pub transcode_base_url: String,
    pub subtitle_base_url: String,
    pub session_id: String,
}

impl StreamProxy {
    pub async fn start(client: Client) -> Self {
        let sessions = Arc::new(RwLock::new(HashMap::new()));
        let custom_ffmpeg_path = Arc::new(RwLock::new(None));
        let state = ProxyAppState {
            client,
            sessions: sessions.clone(),
            custom_ffmpeg_path: custom_ffmpeg_path.clone(),
            transcode_errors: Arc::new(RwLock::new(HashMap::new())),
        };

        let app = Router::new()
            .route("/stream", get(handle_stream))
            .route("/local", get(handle_local_file))
            .route("/transcode", get(handle_transcode))
            .route("/transcode-error", get(handle_transcode_error))
            .route("/subtitles", get(handle_subtitles))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local stream proxy");
        let addr: SocketAddr = listener.local_addr().expect("proxy local_addr");

        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            port: addr.port(),
            sessions,
            custom_ffmpeg_path,
        }
    }

    pub fn set_ffmpeg_path(&self, path: Option<String>) {
        if let Ok(mut guard) = self.custom_ffmpeg_path.write() {
            *guard = path.filter(|s| !s.trim().is_empty());
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn register_session(
        &self,
        stream_url: &str,
        headers: HashMap<String, String>,
    ) -> Result<ProxiedStream, MediaError> {
        let trimmed = stream_url.trim();
        if !is_valid_stream_scheme(trimmed) {
            return Err(MediaError::InvalidUrl(
                "Stream URL must use http/https/rtmp/rtsp/udp/rtp".into(),
            ));
        }

        let session_id = Uuid::new_v4().to_string();
        if let Ok(mut map) = self.sessions.write() {
            if map.len() > 64 {
                map.clear();
            }
            map.insert(
                session_id.clone(),
                ProxySession {
                    base_url: trimmed.to_string(),
                    headers,
                },
            );
        }

        let proxy_url = format!(
            "http://127.0.0.1:{}/stream?session={}",
            self.port, session_id
        );
        Ok(ProxiedStream {
            proxy_url,
            transcode_base_url: format!(
                "http://127.0.0.1:{}/transcode?session={}",
                self.port, session_id
            ),
            subtitle_base_url: format!(
                "http://127.0.0.1:{}/subtitles?session={}",
                self.port, session_id
            ),
            session_id,
        })
    }

    pub fn register_local_file(&self, file_path: &str) -> Result<ProxiedStream, MediaError> {
        let p = Path::new(file_path.trim());
        if !p.is_file() {
            return Err(MediaError::InvalidUrl("Local media file does not exist".into()));
        }
        let session_id = Uuid::new_v4().to_string();
        if let Ok(mut map) = self.sessions.write() {
            if map.len() > 64 {
                map.clear();
            }
            map.insert(
                session_id.clone(),
                ProxySession {
                    base_url: p.to_string_lossy().into_owned(),
                    headers: HashMap::new(),
                },
            );
        }
        let proxy_url = format!(
            "http://127.0.0.1:{}/local?session={}",
            self.port, session_id
        );
        Ok(ProxiedStream {
            proxy_url,
            transcode_base_url: format!(
                "http://127.0.0.1:{}/transcode?session={}",
                self.port, session_id
            ),
            subtitle_base_url: format!(
                "http://127.0.0.1:{}/subtitles?session={}",
                self.port, session_id
            ),
            session_id,
        })
    }
}

async fn handle_stream(
    State(state): State<ProxyAppState>,
    Query(query): Query<StreamQuery>,
    req_headers: HeaderMap,
) -> Response {
    let session = {
        let Ok(map) = state.sessions.read() else {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "session lock failed");
        };
        map.get(&query.session).cloned()
    };

    let Some(session) = session else {
        return error_response(StatusCode::NOT_FOUND, "unknown stream session");
    };

    let target_url = query
        .url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&session.base_url);

    if !target_url.starts_with("http://") && !target_url.starts_with("https://") {
        return error_response(StatusCode::BAD_REQUEST, "only http/https proxying is supported");
    }

    let mut headers = ReqHeaderMap::new();
    for (k, v) in &session.headers {
        if let (Ok(hn), Ok(hv)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            headers.insert(hn, hv);
        }
    }
    if let Some(range) = req_headers.get(header::RANGE) {
        if let Ok(rv) = HeaderValue::from_bytes(range.as_bytes()) {
            headers.insert(reqwest::header::RANGE, rv);
        }
    }

    let upstream = match state.client.get(target_url).headers(headers).send().await {
        Ok(resp) => resp,
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e.to_string()),
    };

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    let final_url = upstream.url().clone();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let content_range = upstream
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let accept_ranges = upstream
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let content_length = upstream.content_length();

    let is_m3u8 = content_type.to_ascii_lowercase().contains("mpegurl")
        || final_url
            .path()
            .to_ascii_lowercase()
            .ends_with(".m3u8");

    if is_m3u8 {
        let Ok(text) = upstream.text().await else {
            return error_response(StatusCode::BAD_GATEWAY, "failed reading playlist");
        };
        let rewritten = rewrite_m3u8(&text, &final_url, &query.session);
        let mut builder = Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CACHE_CONTROL, "no-store");
        if let Some(b) = builder.headers_mut() {
            let _ = b;
        }
        return builder
            .body(Body::from(rewritten))
            .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "build error"));
    }

    let stream = upstream.bytes_stream();
    let mut builder = Response::builder()
        .status(status)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::CONTENT_TYPE,
            if content_type.is_empty() {
                "application/octet-stream"
            } else {
                &content_type
            },
        );

    if let Some(cr) = content_range {
        builder = builder.header(header::CONTENT_RANGE, cr);
    }
    if let Some(ar) = accept_ranges {
        builder = builder.header(header::ACCEPT_RANGES, ar);
    }
    if let Some(cl) = content_length {
        builder = builder.header(header::CONTENT_LENGTH, cl.to_string());
    }

    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "stream error"))
}

fn error_response(status: StatusCode, msg: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(Body::from(msg.to_string()))
        .unwrap()
}

async fn handle_local_file(
    State(state): State<ProxyAppState>,
    Query(query): Query<StreamQuery>,
    req_headers: HeaderMap,
) -> Response {
    let file_path = {
        let Ok(map) = state.sessions.read() else {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "session lock failed");
        };
        map.get(&query.session).map(|s| s.base_url.clone())
    };
    let Some(path) = file_path else {
        return error_response(StatusCode::NOT_FOUND, "unknown local media session");
    };
    let Ok(mut file) = tokio::fs::File::open(&path).await else {
        return error_response(StatusCode::NOT_FOUND, "file not found");
    };
    let Ok(meta) = file.metadata().await else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "metadata error");
    };
    let total = meta.len();
    let content_type = match Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        _ => "video/mp4",
    };

    if let Some(range_val) = req_headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        if let Some(spec) = range_val.strip_prefix("bytes=") {
            let mut parts = spec.split('-');
            let start = parts.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let end = parts
                .next()
                .and_then(|s| if s.is_empty() { None } else { s.parse::<u64>().ok() })
                .unwrap_or_else(|| total.saturating_sub(1))
                .min(total.saturating_sub(1));

            if start <= end && start < total {
                let len = end - start + 1;
                if file.seek(std::io::SeekFrom::Start(start)).await.is_ok() {
                    let stream = ReaderStream::new(file.take(len));
                    return Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                        .header(header::CONTENT_TYPE, content_type)
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
                        .header(header::CONTENT_LENGTH, len.to_string())
                        .body(Body::from_stream(stream))
                        .unwrap();
                }
            }
        }
    }

    let stream = ReaderStream::new(file);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, total.to_string())
        .body(Body::from_stream(stream))
        .unwrap()
}

pub fn rewrite_m3u8(playlist: &str, base_url: &Url, session_id: &str) -> String {
    let mut out = String::with_capacity(playlist.len() + 256);
    for line in playlist.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        if trimmed.starts_with('#') {
            out.push_str(&rewrite_tag_uri(trimmed, base_url, session_id));
            out.push('\n');
        } else {
            let abs = base_url
                .join(trimmed)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| trimmed.to_string());
            out.push_str(&proxy_link(session_id, &abs));
            out.push('\n');
        }
    }
    out
}

fn rewrite_tag_uri(line: &str, base_url: &Url, session_id: &str) -> String {
    let Some(idx) = line.find("URI=\"") else {
        return line.to_string();
    };
    let start = idx + 5;
    let Some(end_rel) = line[start..].find('"') else {
        return line.to_string();
    };
    let raw_uri = &line[start..start + end_rel];
    let abs = base_url
        .join(raw_uri)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| raw_uri.to_string());
    let proxied = proxy_link(session_id, &abs);
    format!("{}{}{}", &line[..start], proxied, &line[start + end_rel..])
}

fn proxy_link(session_id: &str, target_url: &str) -> String {
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("session", session_id)
        .append_pair("url", target_url)
        .finish();
    format!("/stream?{encoded}")
}

/// Launches VLC or mpv directly (zero shell interpolation) with stream URL and optional headers.
pub fn launch_external_player(
    stream_url: &str,
    title: &str,
    headers: &HashMap<String, String>,
    configured_path: Option<&str>,
) -> Result<String, MediaError> {
    let trimmed_url = stream_url.trim();
    if !is_valid_stream_scheme(trimmed_url) && !Path::new(trimmed_url).exists() {
        return Err(MediaError::InvalidUrl(
            "Only valid stream URLs or existing local files can be opened".into(),
        ));
    }

    let exe = find_external_player(configured_path).ok_or(MediaError::ExternalPlayerNotFound)?;
    let exe_name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut cmd = Command::new(&exe);
    if exe_name.contains("mpv") {
        if !title.trim().is_empty() {
            cmd.arg(format!("--force-media-title={}", title.trim()));
        }
        let header_fields: Vec<String> = headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        if !header_fields.is_empty() {
            cmd.arg(format!("--http-header-fields={}", header_fields.join(",")));
        }
        cmd.arg("--").arg(trimmed_url);
    } else if exe_name.contains("vlc") {
        if let Some(ua) = headers.get("User-Agent").or_else(|| headers.get("user-agent")) {
            cmd.arg(format!(":http-user-agent={ua}"));
        }
        if let Some(ref_val) = headers.get("Referer").or_else(|| headers.get("referer")) {
            cmd.arg(format!(":http-referrer={ref_val}"));
        }
        if !title.trim().is_empty() {
            cmd.arg(format!("--meta-title={}", title.trim()));
        }
        cmd.arg("--").arg(trimmed_url);
    } else {
        cmd.arg("--").arg(trimmed_url);
    }

    cmd.spawn()?;
    Ok(exe.to_string_lossy().into_owned())
}

fn find_external_player(configured_path: Option<&str>) -> Option<PathBuf> {
    if let Some(custom) = configured_path.map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }

    let candidates = [
        "mpv",
        "vlc",
        r"C:\Program Files\VideoLAN\VLC\vlc.exe",
        r"C:\Program Files (x86)\VideoLAN\VLC\vlc.exe",
        r"C:\Program Files\mpv\mpv.exe",
        "/usr/bin/mpv",
        "/usr/bin/vlc",
        "/Applications/VLC.app/Contents/MacOS/VLC",
        "/Applications/IINA.app/Contents/MacOS/IINA",
    ];

    for cand in candidates {
        let p = PathBuf::from(cand);
        if p.is_file() {
            return Some(p);
        }
        if !cand.contains(['\\', '/']) {
            if let Ok(path_env) = std::env::var("PATH") {
                for dir in std::env::split_paths(&path_env) {
                    let full = dir.join(if cfg!(windows) {
                        format!("{cand}.exe")
                    } else {
                        cand.to_string()
                    });
                    if full.is_file() {
                        return Some(full);
                    }
                }
            }
        }
    }
    None
}

struct ChildStdoutReader {
    _child: Child,
    stdout: ChildStdout,
    prefix: Vec<u8>,
    prefix_pos: usize,
}

impl AsyncRead for ChildStdoutReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.prefix_pos < self.prefix.len() {
            let remaining = &self.prefix[self.prefix_pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            self.prefix_pos += to_copy;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}

fn summarize_ffmpeg_stderr(raw: &str) -> String {
    let last = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Error opening input file "))
        .next_back()
        .unwrap_or("Stream could not be decoded by FFmpeg");
    let cleaned = last
        .strip_prefix("Error opening input files: ")
        .or_else(|| last.strip_prefix("Error opening input: "))
        .unwrap_or(last);
    if cleaned.starts_with('[') {
        if let Some((_, rest)) = cleaned.split_once("] ") {
            return rest.trim().to_string();
        }
    }
    cleaned.to_string()
}

async fn handle_transcode_error(
    State(state): State<ProxyAppState>,
    Query(query): Query<StreamQuery>,
) -> Response {
    let msg = state
        .transcode_errors
        .read()
        .ok()
        .and_then(|map| map.get(&query.session).cloned())
        .unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(msg))
        .unwrap()
}

async fn handle_transcode(
    State(state): State<ProxyAppState>,
    Query(query): Query<TranscodeQuery>,
) -> Response {
    let session = {
        let Ok(map) = state.sessions.read() else {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "session lock failed");
        };
        map.get(&query.session).cloned()
    };
    let Some(session) = session else {
        return error_response(StatusCode::NOT_FOUND, "unknown media session");
    };

    if let Ok(mut errs) = state.transcode_errors.write() {
        errs.remove(&query.session);
    }

    let custom_ffmpeg = state
        .custom_ffmpeg_path
        .read()
        .ok()
        .and_then(|g| g.clone());
    let Some(ffmpeg_exe) = ffmpeg::resolve_ffmpeg_exe(custom_ffmpeg.as_deref()) else {
        return error_response(
            StatusCode::PRECONDITION_REQUIRED,
            "FFmpeg is not installed. Setup FFmpeg in Preferences to play this format.",
        );
    };

    let args = ffmpeg::build_transcode_args(
        &session.base_url,
        &session.headers,
        query.audio_track.unwrap_or(0),
        query.start_sec.unwrap_or(0.0).max(0.0),
        query.copy_video.unwrap_or(true),
    );

    let mut cmd = TokioCommand::new(&ffmpeg_exe);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    ffmpeg::apply_no_window_tokio(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let Some(mut stdout) = child.stdout.take() else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "ffmpeg stdout unavailable");
    };
    let mut stderr = child.stderr.take();

    let mut first_buf = vec![0u8; 8192];
    let first_read = tokio::time::timeout(Duration::from_secs(12), stdout.read(&mut first_buf)).await;
    let first_len = match first_read {
        Ok(Ok(n)) if n > 0 => n,
        Ok(Ok(_)) => {
            let mut err_buf = Vec::new();
            if let Some(mut se) = stderr.take() {
                let _ = tokio::time::timeout(Duration::from_secs(2), se.read_to_end(&mut err_buf)).await;
            }
            let summary = summarize_ffmpeg_stderr(&String::from_utf8_lossy(&err_buf));
            if let Ok(mut errs) = state.transcode_errors.write() {
                if errs.len() > 64 {
                    errs.clear();
                }
                errs.insert(query.session.clone(), summary.clone());
            }
            let _ = child.kill().await;
            return error_response(StatusCode::BAD_GATEWAY, &summary);
        }
        Ok(Err(e)) => {
            let summary = e.to_string();
            if let Ok(mut errs) = state.transcode_errors.write() {
                errs.insert(query.session.clone(), summary.clone());
            }
            let _ = child.kill().await;
            return error_response(StatusCode::BAD_GATEWAY, &summary);
        }
        Err(_) => {
            let summary = "Stream timed out waiting for FFmpeg frames".to_string();
            if let Ok(mut errs) = state.transcode_errors.write() {
                errs.insert(query.session.clone(), summary.clone());
            }
            let _ = child.kill().await;
            return error_response(StatusCode::GATEWAY_TIMEOUT, &summary);
        }
    };

    // Drain stderr asynchronously so ffmpeg never blocks writing diagnostics while streaming.
    if let Some(mut se) = stderr.take() {
        tokio::spawn(async move {
            let mut sink = tokio::io::sink();
            let _ = tokio::io::copy(&mut se, &mut sink).await;
        });
    }

    first_buf.truncate(first_len);
    let reader = ChildStdoutReader {
        _child: child,
        stdout,
        prefix: first_buf,
        prefix_pos: 0,
    };
    let stream = ReaderStream::with_capacity(reader, 64 * 1024);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "stream error"))
}

async fn handle_subtitles(
    State(state): State<ProxyAppState>,
    Query(query): Query<SubtitleQuery>,
) -> Response {
    let session = {
        let Ok(map) = state.sessions.read() else {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "session lock failed");
        };
        map.get(&query.session).cloned()
    };
    let Some(session) = session else {
        return error_response(StatusCode::NOT_FOUND, "unknown media session");
    };

    let custom_ffmpeg = state
        .custom_ffmpeg_path
        .read()
        .ok()
        .and_then(|g| g.clone());
    let Some(ffmpeg_exe) = ffmpeg::resolve_ffmpeg_exe(custom_ffmpeg.as_deref()) else {
        return error_response(StatusCode::PRECONDITION_REQUIRED, "FFmpeg is not installed");
    };

    let args = ffmpeg::build_subtitle_vtt_args(
        &session.base_url,
        &session.headers,
        query.track.unwrap_or(0),
    );
    let mut cmd = TokioCommand::new(&ffmpeg_exe);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    ffmpeg::apply_no_window_tokio(&mut cmd);

    let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(20), cmd.output()).await else {
        return error_response(StatusCode::GATEWAY_TIMEOUT, "subtitle extraction timed out");
    };
    if !out.status.success() || out.stdout.is_empty() {
        return error_response(StatusCode::NOT_FOUND, "subtitle track unavailable");
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CONTENT_TYPE, "text/vtt; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(out.stdout))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "build error"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_m3u8_segments_and_key_uris_through_proxy() {
        let base = Url::parse("https://cdn.example.com/hls/master.m3u8").unwrap();
        let raw = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"keys/enc.key\"\n720p/index.m3u8\nhttps://other.cdn/seg1.ts\n";
        let out = rewrite_m3u8(raw, &base, "sess-1");
        assert!(out.contains("URI=\"/stream?session=sess-1&url=https%3A%2F%2Fcdn.example.com%2Fhls%2Fkeys%2Fenc.key\""));
        assert!(out.contains("/stream?session=sess-1&url=https%3A%2F%2Fcdn.example.com%2Fhls%2F720p%2Findex.m3u8"));
        assert!(out.contains("/stream?session=sess-1&url=https%3A%2F%2Fother.cdn%2Fseg1.ts"));
    }

    #[test]
    fn rejects_invalid_scheme_for_external_player() {
        let err = launch_external_player("javascript:alert(1)", "x", &HashMap::new(), None);
        assert!(matches!(err, Err(MediaError::InvalidUrl(_))));
    }

    #[test]
    fn parses_ffprobe_tracks_and_builds_sanitized_ffmpeg_args() {
        let sample = serde_json::json!({
            "format": {
                "format_name": "matroska,webm",
                "duration": "1320.5",
                "bit_rate": "2400000",
                "size": "420000000"
            },
            "streams": [
                { "index": 0, "codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080 },
                { "index": 1, "codec_type": "audio", "codec_name": "ac3", "channels": 6, "tags": { "language": "hin", "title": "Hindi 5.1" } },
                { "index": 2, "codec_type": "audio", "codec_name": "aac", "channels": 2, "tags": { "language": "eng" } },
                { "index": 3, "codec_type": "subtitle", "codec_name": "subrip", "tags": { "language": "eng", "title": "English" } }
            ]
        });
        let parsed = ffmpeg::parse_ffprobe_json(&sample, "https://cdn.example/video.mkv");
        assert!(parsed.needs_transcode);
        assert_eq!(parsed.audio_tracks.len(), 2);
        assert_eq!(parsed.audio_tracks[0].title, "Hindi 5.1");
        assert_eq!(parsed.audio_tracks[1].title, "ENG");
        assert_eq!(parsed.subtitle_tracks.len(), 1);
        assert!(parsed.subtitle_tracks[0].text_based);

        let mut hdrs = HashMap::new();
        hdrs.insert("User-Agent".into(), "Agent\r\nInjected: bad".into());
        hdrs.insert("Referer".into(), "https://ref.example/".into());
        let args = ffmpeg::build_transcode_args("https://cdn.example/video.mkv", &hdrs, 1, 95.5, true);
        assert!(args.windows(2).any(|w| w == ["-ss", "95.50"]));
        assert!(args.windows(2).any(|w| w == ["-user_agent", "AgentInjected: bad"]));
        assert!(args.windows(2).any(|w| w == ["-map", "0:a:1?"]));
        assert!(args.windows(2).any(|w| w == ["-c:v", "copy"]));

        let tags = serde_json::json!([
            { "name": "v0.6.1" },
            { "name": "n9.2-dev" },
            { "name": "n8.1" },
            { "name": "n9.1" }
        ]);
        let picked = ffmpeg::pick_latest_upstream_ffmpeg_tag(&tags).unwrap();
        assert_eq!(picked, ("n9.1".into(), "9.1".into()));
        assert_eq!(ffmpeg::parse_semver_tuple("ffmpeg version 9.1-essentials_build"), Some((9, 1, 0)));

        let sample_err = "[in#0 @ 0x1] Error opening input: Server returned 404 Not Found\nError opening input file https://x.example/live.m3u8.\nError opening input files: Server returned 404 Not Found\n";
        assert_eq!(
            summarize_ffmpeg_stderr(sample_err),
            "Server returned 404 Not Found"
        );
    }
}
