pub mod ffmpeg;

use axum::{
    body::{Body, Bytes},
    extract::{Query, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::Response,
    routing::get,
    Router,
};
pub use ffmpeg::{
    check_ffmpeg_status_with_upstream, detect_ffmpeg_status, install_latest_ffmpeg, probe_media,
    probe_media_with_input, resolve_ffmpeg_exe, resolve_ffprobe_exe, FfmpegStatus,
    MediaAudioTrack, MediaProbeResult, MediaSubtitleTrack,
};
use iptvmine_core::is_valid_stream_scheme;
use reqwest::header::{HeaderMap as ReqHeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, ReadBuf};
use tokio::process::{Child, ChildStdout, Command as TokioCommand};
use tokio::task::JoinSet;
use tokio_util::io::ReaderStream;
use url::Url;
use uuid::Uuid;

const PREFETCH_CHUNK_BYTES: u64 = 512 * 1024;
const PREFETCH_WORKERS: usize = 10;
const PREFETCH_AHEAD_CHUNKS: u64 = 28;
const SESSION_CHUNK_CACHE_MAX: usize = 96;

#[derive(Debug, Clone)]
struct RangeFileMeta {
    total_len: u64,
    content_type: String,
}

#[derive(Default)]
struct ChunkLru {
    map: HashMap<u64, Bytes>,
    order: VecDeque<u64>,
}

#[derive(Default)]
struct ChunkCache {
    meta: RwLock<Option<RangeFileMeta>>,
    unsupported: AtomicBool,
    chunks: Mutex<ChunkLru>,
}

impl ChunkCache {
    fn get_meta(&self) -> Option<RangeFileMeta> {
        self.meta.read().ok().and_then(|g| g.clone())
    }

    fn set_meta(&self, meta: RangeFileMeta) {
        if let Ok(mut g) = self.meta.write() {
            *g = Some(meta);
        }
    }

    fn get(&self, idx: u64) -> Option<Bytes> {
        self.chunks.lock().ok().and_then(|g| g.map.get(&idx).cloned())
    }

    fn insert(&self, idx: u64, data: Bytes) {
        let Ok(mut g) = self.chunks.lock() else {
            return;
        };
        if g.map.contains_key(&idx) {
            return;
        }
        if g.order.len() >= SESSION_CHUNK_CACHE_MAX {
            if let Some(oldest) = g.order.pop_front() {
                g.map.remove(&oldest);
            }
        }
        g.order.push_back(idx);
        g.map.insert(idx, data);
    }
}

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

#[derive(Clone)]
struct ProxySession {
    base_url: String,
    headers: HashMap<String, String>,
    chunk_cache: Arc<ChunkCache>,
}

#[derive(Clone)]
pub struct StreamProxy {
    port: u16,
    sessions: Arc<RwLock<HashMap<String, ProxySession>>>,
    custom_ffmpeg_path: Arc<RwLock<Option<String>>>,
}

#[derive(Clone)]
struct ProxyAppState {
    port: u16,
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local stream proxy");
        let addr: SocketAddr = listener.local_addr().expect("proxy local_addr");
        let port = addr.port();

        let state = ProxyAppState {
            port,
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

        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            port,
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
                    chunk_cache: Arc::new(ChunkCache::default()),
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
                    chunk_cache: Arc::new(ChunkCache::default()),
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

fn build_upstream_headers(session_headers: &HashMap<String, String>) -> ReqHeaderMap {
    let mut headers = ReqHeaderMap::new();
    for (k, v) in session_headers {
        if let (Ok(hn), Ok(hv)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            headers.insert(hn, hv);
        }
    }
    headers
}

fn parse_byte_range(hdr: &str) -> Option<(u64, Option<u64>)> {
    let spec = hdr.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') || spec.starts_with('-') {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    let start = start_s.trim().parse::<u64>().ok()?;
    let end = if end_s.trim().is_empty() {
        None
    } else {
        let e = end_s.trim().parse::<u64>().ok()?;
        if e < start {
            return None;
        }
        Some(e)
    };
    Some((start, end))
}

fn parse_content_range_header(hdr: &str) -> Option<(u64, u64, u64)> {
    let rest = hdr.trim().strip_prefix("bytes ")?.trim();
    let (range_part, total_part) = rest.split_once('/')?;
    if total_part.trim() == "*" {
        return None;
    }
    let total = total_part.trim().parse::<u64>().ok()?;
    let (s, e) = range_part.split_once('-')?;
    let start = s.trim().parse::<u64>().ok()?;
    let end = e.trim().parse::<u64>().ok()?;
    if end < start || total <= end {
        return None;
    }
    Some((start, end, total))
}

fn build_passthrough_stream_response(
    upstream: reqwest::Response,
    status: StatusCode,
    content_type: &str,
    content_range: Option<String>,
    accept_ranges: Option<String>,
    content_length: Option<u64>,
) -> Response {
    let stream = upstream.bytes_stream();
    let mut builder = Response::builder()
        .status(status)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::CONTENT_TYPE,
            if content_type.is_empty() {
                "application/octet-stream"
            } else {
                content_type
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

async fn fetch_range_chunk_with_retry(
    client: &Client,
    url: &str,
    base_headers: &ReqHeaderMap,
    start: u64,
    end: u64,
) -> Result<Bytes, std::io::Error> {
    let range_val = format!("bytes={start}-{end}");
    for attempt in 0..3 {
        let mut req_headers = base_headers.clone();
        if let Ok(hv) = HeaderValue::from_str(&range_val) {
            req_headers.insert(reqwest::header::RANGE, hv);
        }
        match client.get(url).headers(req_headers).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(b) => return Ok(b),
                Err(e) if attempt == 2 => return Err(std::io::Error::other(e.to_string())),
                Err(_) => {}
            },
            Ok(resp) if attempt == 2 => {
                return Err(std::io::Error::other(format!(
                    "upstream returned HTTP {}",
                    resp.status()
                )));
            }
            Err(e) if attempt == 2 => return Err(std::io::Error::other(e.to_string())),
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    unreachable!()
}

async fn forward_single_stream(
    client: &Client,
    target_url: &str,
    session_headers: &HashMap<String, String>,
    session_id: &str,
    req_headers: &HeaderMap,
) -> Response {
    let mut headers = build_upstream_headers(session_headers);
    if let Some(range) = req_headers.get(header::RANGE) {
        if let Ok(rv) = HeaderValue::from_bytes(range.as_bytes()) {
            headers.insert(reqwest::header::RANGE, rv);
        }
    }

    let upstream = match client.get(target_url).headers(headers).send().await {
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
        || final_url.path().to_ascii_lowercase().ends_with(".m3u8");

    if is_m3u8 {
        let Ok(text) = upstream.text().await else {
            return error_response(StatusCode::BAD_GATEWAY, "failed reading playlist");
        };
        let rewritten = rewrite_m3u8(&text, &final_url, session_id);
        return Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(rewritten))
            .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "build error"));
    }

    build_passthrough_stream_response(
        upstream,
        status,
        &content_type,
        content_range,
        accept_ranges,
        content_length,
    )
}

async fn handle_stream(
    method: Method,
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

    if method == Method::HEAD {
        let headers = build_upstream_headers(&session.headers);
        let upstream = match state.client.head(target_url).headers(headers).send().await {
            Ok(resp) => resp,
            Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e.to_string()),
        };
        let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
        let ct = upstream
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let ar = upstream
            .headers()
            .get(reqwest::header::ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let cl = upstream.content_length();
        let mut builder = Response::builder()
            .status(status)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CONTENT_TYPE, ct);
        if let Some(a) = ar {
            builder = builder.header(header::ACCEPT_RANGES, a);
        }
        if let Some(l) = cl {
            builder = builder.header(header::CONTENT_LENGTH, l.to_string());
        }
        return builder
            .body(Body::empty())
            .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "head error"));
    }

    let range_hdr = req_headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let parsed_range = range_hdr.and_then(parse_byte_range);
    let can_prefetch = query.url.is_none()
        && !target_url.to_ascii_lowercase().contains(".m3u8")
        && (range_hdr.is_none() || parsed_range.is_some())
        && !session.chunk_cache.unsupported.load(Ordering::Relaxed);

    if !can_prefetch {
        return forward_single_stream(
            &state.client,
            target_url,
            &session.headers,
            &query.session,
            &req_headers,
        )
        .await;
    }

    let start = parsed_range.map(|(s, _)| s).unwrap_or(0);
    let first_chunk_idx = start / PREFETCH_CHUNK_BYTES;
    let first_chunk_start = first_chunk_idx * PREFETCH_CHUNK_BYTES;

    let (meta, first_chunk_bytes) =
        if let (Some(m), Some(cached)) = (session.chunk_cache.get_meta(), session.chunk_cache.get(first_chunk_idx)) {
            (m, cached)
        } else {
            let chunk_end_req = session
                .chunk_cache
                .get_meta()
                .map(|m| (first_chunk_start + PREFETCH_CHUNK_BYTES - 1).min(m.total_len.saturating_sub(1)))
                .unwrap_or(first_chunk_start + PREFETCH_CHUNK_BYTES - 1);
            let mut headers = build_upstream_headers(&session.headers);
            if let Ok(hv) = HeaderValue::from_str(&format!("bytes={first_chunk_start}-{chunk_end_req}")) {
                headers.insert(reqwest::header::RANGE, hv);
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

            if !status.is_success() {
                return error_response(status, &format!("upstream returned HTTP {status}"));
            }

            let is_m3u8 = content_type.to_ascii_lowercase().contains("mpegurl")
                || final_url.path().to_ascii_lowercase().ends_with(".m3u8");
            if is_m3u8 {
                let Ok(text) = upstream.text().await else {
                    return error_response(StatusCode::BAD_GATEWAY, "failed reading playlist");
                };
                let rewritten = rewrite_m3u8(&text, &final_url, &query.session);
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
                    .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .header(header::CACHE_CONTROL, "no-store")
                    .body(Body::from(rewritten))
                    .unwrap_or_else(|_| {
                        error_response(StatusCode::INTERNAL_SERVER_ERROR, "build error")
                    });
            }

            if status != StatusCode::PARTIAL_CONTENT {
                session.chunk_cache.unsupported.store(true, Ordering::Relaxed);
                return build_passthrough_stream_response(
                    upstream,
                    status,
                    &content_type,
                    content_range,
                    accept_ranges,
                    content_length,
                );
            }

            let Some((cr_start, _cr_end, total_len)) =
                content_range.as_deref().and_then(parse_content_range_header)
            else {
                session.chunk_cache.unsupported.store(true, Ordering::Relaxed);
                return build_passthrough_stream_response(
                    upstream,
                    status,
                    &content_type,
                    content_range,
                    accept_ranges,
                    content_length,
                );
            };
            if cr_start != first_chunk_start || total_len <= PREFETCH_CHUNK_BYTES {
                if total_len > PREFETCH_CHUNK_BYTES {
                    session.chunk_cache.unsupported.store(true, Ordering::Relaxed);
                }
                return build_passthrough_stream_response(
                    upstream,
                    status,
                    &content_type,
                    content_range,
                    accept_ranges,
                    content_length,
                );
            }

            let bytes = match upstream.bytes().await {
                Ok(b) => b,
                Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e.to_string()),
            };
            let resolved_meta = RangeFileMeta {
                total_len,
                content_type: if content_type.is_empty() {
                    "application/octet-stream".into()
                } else {
                    content_type
                },
            };
            session.chunk_cache.set_meta(resolved_meta.clone());
            session.chunk_cache.insert(first_chunk_idx, bytes.clone());
            (resolved_meta, bytes)
        };

    let total = meta.total_len;
    if start >= total {
        return Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CONTENT_RANGE, format!("bytes */{total}"))
            .body(Body::empty())
            .unwrap();
    }

    let req_end = parsed_range
        .and_then(|(_, e)| e)
        .unwrap_or(total - 1)
        .min(total - 1);
    if req_end < start {
        return Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CONTENT_RANGE, format!("bytes */{total}"))
            .body(Body::empty())
            .unwrap();
    }

    let resp_len = req_end - start + 1;
    let first_offset = (start - first_chunk_start) as usize;
    let avail = first_chunk_bytes.len().saturating_sub(first_offset);
    let first_take = avail.min(resp_len as usize);
    let first_slice = first_chunk_bytes.slice(first_offset..first_offset + first_take);

    let resp_status = if parsed_range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let mut builder = Response::builder()
        .status(resp_status)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CONTENT_TYPE, meta.content_type.as_str())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, resp_len.to_string());
    if parsed_range.is_some() {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{req_end}/{total}"));
    }

    if (first_take as u64) >= resp_len {
        return builder
            .body(Body::from(first_slice))
            .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "stream error"));
    }

    let (mut writer, reader) = tokio::io::duplex(2 * 1024 * 1024);

    let client = state.client.clone();
    let target = target_url.to_string();
    let base_headers = build_upstream_headers(&session.headers);
    let cache = session.chunk_cache.clone();
    let start_next_chunk = first_chunk_idx + 1;
    let last_chunk = req_end / PREFETCH_CHUNK_BYTES;

    tokio::spawn(async move {
        if writer.write_all(&first_slice).await.is_err() {
            return;
        }

        let mut next_to_emit = start_next_chunk;
        let mut next_to_schedule = start_next_chunk;
        let mut ready: BTreeMap<u64, Bytes> = BTreeMap::new();
        let mut in_flight: JoinSet<(u64, Result<Bytes, std::io::Error>)> = JoinSet::new();

        while next_to_emit <= last_chunk {
            if !ready.contains_key(&next_to_emit) {
                if let Some(cached) = cache.get(next_to_emit) {
                    ready.insert(next_to_emit, cached);
                    if next_to_schedule <= next_to_emit {
                        next_to_schedule = next_to_emit + 1;
                    }
                }
            }

            while in_flight.len() < PREFETCH_WORKERS
                && next_to_schedule <= last_chunk
                && next_to_schedule < next_to_emit + PREFETCH_AHEAD_CHUNKS
            {
                let c_idx = next_to_schedule;
                next_to_schedule += 1;
                if let Some(cached) = cache.get(c_idx) {
                    ready.insert(c_idx, cached);
                    continue;
                }
                let c_start = c_idx * PREFETCH_CHUNK_BYTES;
                let c_end = (c_start + PREFETCH_CHUNK_BYTES - 1).min(total - 1);
                let cli = client.clone();
                let u = target.clone();
                let h = base_headers.clone();
                in_flight.spawn(async move {
                    let res = fetch_range_chunk_with_retry(&cli, &u, &h, c_start, c_end).await;
                    (c_idx, res)
                });
            }

            if let Some(chunk_data) = ready.remove(&next_to_emit) {
                let c_start = next_to_emit * PREFETCH_CHUNK_BYTES;
                let needed_end = (req_end + 1 - c_start).min(chunk_data.len() as u64) as usize;
                let slice = chunk_data.slice(..needed_end);
                if writer.write_all(&slice).await.is_err() {
                    break;
                }
                next_to_emit += 1;
                continue;
            }

            match in_flight.join_next().await {
                Some(Ok((c_idx, Ok(data)))) => {
                    cache.insert(c_idx, data.clone());
                    ready.insert(c_idx, data);
                }
                Some(Ok((_, Err(_)))) | Some(Err(_)) | None => {
                    // Dropping writer before Content-Length causes HTTP client/hyper to abort incomplete body.
                    break;
                }
            }
        }
    });

    let stream = ReaderStream::with_capacity(reader, 256 * 1024);
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
    method: axum::http::Method,
    State(state): State<ProxyAppState>,
    Query(query): Query<TranscodeQuery>,
) -> Response {
    if method == axum::http::Method::HEAD {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CONTENT_TYPE, "video/mp4")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::empty())
            .unwrap();
    }

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

    let use_range_proxy = (session.base_url.starts_with("http://")
        || session.base_url.starts_with("https://"))
        && !session.base_url.to_ascii_lowercase().contains(".m3u8");
    let ffmpeg_input = if use_range_proxy {
        format!("http://127.0.0.1:{}/stream?session={}", state.port, query.session)
    } else {
        session.base_url.clone()
    };
    let empty_headers = HashMap::new();
    let ffmpeg_headers = if use_range_proxy {
        &empty_headers
    } else {
        &session.headers
    };

    let args = ffmpeg::build_transcode_args(
        &ffmpeg_input,
        ffmpeg_headers,
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
    let first_read = tokio::time::timeout(Duration::from_secs(35), stdout.read(&mut first_buf)).await;
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
        assert!(args.windows(2).any(|w| w == ["-rw_timeout", "15000000"]));
        assert!(args.windows(2).any(|w| w == ["-probesize", "1048576"]));
        assert!(args.windows(2).any(|w| w == ["-user_agent", "AgentInjected: bad"]));
        assert!(args.windows(2).any(|w| w == ["-map", "0:a:1?"]));
        assert!(args.windows(2).any(|w| w == ["-c:v", "copy"]));

        let transcode_args =
            ffmpeg::build_transcode_args("https://cdn.example/video.mkv", &hdrs, 0, 0.0, false);
        assert!(transcode_args.windows(2).any(|w| w == ["-preset", "ultrafast"]));
        assert!(transcode_args.windows(2).any(|w| w == ["-tune", "zerolatency"]));

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

    #[tokio::test]
    async fn transcode_proxy_handles_head_and_upstream_404_cleanly() {
        let Some(_) = ffmpeg::resolve_ffmpeg_exe(None) else {
            return;
        };
        let dead_app = Router::new().route("/missing.mkv", get(|| async { StatusCode::NOT_FOUND }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, dead_app).await;
        });

        let client = Client::new();
        let proxy = StreamProxy::start(client.clone()).await;
        let sess = proxy
            .register_session(&format!("http://{dead_addr}/missing.mkv"), HashMap::new())
            .unwrap();

        let head_resp = client
            .head(format!("{}&audioTrack=0&startSec=0&copyVideo=true", sess.transcode_base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(head_resp.status(), reqwest::StatusCode::OK);

        let trans_resp = client
            .get(format!("{}&audioTrack=0&startSec=0&copyVideo=true", sess.transcode_base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(trans_resp.status(), reqwest::StatusCode::BAD_GATEWAY);

        let err_text = client
            .get(format!("http://127.0.0.1:{}/transcode-error?session={}", proxy.port(), sess.session_id))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(err_text.contains("404"), "expected 404 in /transcode-error, got `{err_text}`");
    }

    #[tokio::test]
    async fn parallel_range_proxy_reassembles_chunks_and_caches_reads() {
        use std::sync::atomic::AtomicUsize;

        let payload: Vec<u8> = (0..(2_500_000usize))
            .map(|i| (i % 251) as u8)
            .collect();
        let data = Arc::new(payload);
        let hits = Arc::new(AtomicUsize::new(0));

        let srv_data = data.clone();
        let srv_hits = hits.clone();
        let app = Router::new().route(
            "/movie.mp4",
            get(move |req_headers: HeaderMap| {
                let d = srv_data.clone();
                let h = srv_hits.clone();
                async move {
                    h.fetch_add(1, Ordering::Relaxed);
                    let total = d.len() as u64;
                    let range = req_headers
                        .get(header::RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(parse_byte_range);
                    if let Some((s, e_opt)) = range {
                        let e = e_opt.unwrap_or(total - 1).min(total - 1);
                        let slice = d[s as usize..=e as usize].to_vec();
                        Response::builder()
                            .status(StatusCode::PARTIAL_CONTENT)
                            .header(header::CONTENT_TYPE, "video/mp4")
                            .header(header::ACCEPT_RANGES, "bytes")
                            .header(header::CONTENT_RANGE, format!("bytes {s}-{e}/{total}"))
                            .header(header::CONTENT_LENGTH, slice.len().to_string())
                            .body(Body::from(slice))
                            .unwrap()
                    } else {
                        Response::builder()
                            .status(StatusCode::OK)
                            .header(header::CONTENT_TYPE, "video/mp4")
                            .header(header::ACCEPT_RANGES, "bytes")
                            .header(header::CONTENT_LENGTH, total.to_string())
                            .body(Body::from(d.as_ref().clone()))
                            .unwrap()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = Client::new();
        let proxy = StreamProxy::start(client.clone()).await;
        let sess = proxy
            .register_session(&format!("http://{addr}/movie.mp4"), HashMap::new())
            .unwrap();

        let full = client.get(&sess.proxy_url).send().await.unwrap();
        assert_eq!(full.status(), reqwest::StatusCode::OK);
        let got = full.bytes().await.unwrap();
        assert_eq!(got.len(), data.len());
        assert_eq!(got.as_ref(), data.as_slice());

        let before_cached_hits = hits.load(Ordering::Relaxed);
        let probe = client
            .get(&sess.proxy_url)
            .header(reqwest::header::RANGE, "bytes=0-1")
            .send()
            .await
            .unwrap();
        assert_eq!(probe.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert_eq!(probe.bytes().await.unwrap().as_ref(), &data[0..2]);
        assert_eq!(
            hits.load(Ordering::Relaxed),
            before_cached_hits,
            "cached byte probe should not trigger upstream request"
        );
    }
}
