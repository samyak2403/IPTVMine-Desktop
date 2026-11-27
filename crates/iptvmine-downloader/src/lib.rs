use iptvmine_core::{clean_title, is_valid_stream_scheme, sanitize_filename, DownloadStatus, DownloadTask};
use iptvmine_media::ffmpeg::{
    apply_no_window_tokio, build_hls_download_args, probe_media, resolve_ffmpeg_exe,
    resolve_ffprobe_exe,
};
use iptvmine_storage::Storage;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RANGE};
use reqwest::Client;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::Semaphore;
use uuid::Uuid;

const MAX_CONCURRENT_DOWNLOADS: usize = 5;
const ALLOWED_EXTS: &[&str] = &["mp4", "mkv", "webm", "ts", "m4v", "avi", "mov", "mp3", "aac"];

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("invalid download URL: {0}")]
    InvalidUrl(String),
    #[error("storage error: {0}")]
    Storage(#[from] iptvmine_storage::StorageError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("path escaped configured download directory")]
    PathTraversal,
    #[error("download task not found")]
    NotFound,
}

#[derive(Clone)]
pub struct DownloadManager {
    storage: Arc<Storage>,
    client: Client,
    semaphore: Arc<Semaphore>,
    default_dir: PathBuf,
    cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    custom_ffmpeg_path: Arc<RwLock<Option<String>>>,
}

impl DownloadManager {
    pub fn new(storage: Arc<Storage>, client: Client, custom_dir: Option<&str>) -> Self {
        let default_dir = resolve_download_dir(custom_dir);
        let _ = std::fs::create_dir_all(&default_dir);
        let _ = storage.reset_unfinished_downloads_to_pending();
        Self {
            storage,
            client,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS)),
            default_dir,
            cancel_flags: Arc::new(Mutex::new(HashMap::new())),
            custom_ffmpeg_path: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_ffmpeg_path(&self, path: Option<String>) {
        if let Ok(mut guard) = self.custom_ffmpeg_path.write() {
            *guard = path.filter(|s| !s.trim().is_empty());
        }
    }

    pub fn resume_pending_on_startup<F>(&self, on_event: F)
    where
        F: Fn(&DownloadTask) + Send + Sync + 'static,
    {
        let Ok(tasks) = self.storage.list_downloads() else {
            return;
        };
        let cb = Arc::new(on_event);
        for task in tasks {
            if task.status == DownloadStatus::Pending {
                self.spawn_worker(task, cb.clone());
            }
        }
    }

    pub fn enqueue<F>(
        &self,
        title: &str,
        download_url: &str,
        headers: Option<HashMap<String, String>>,
        custom_dir: Option<&str>,
        on_event: F,
    ) -> Result<DownloadTask, DownloadError>
    where
        F: Fn(&DownloadTask) + Send + Sync + 'static,
    {
        let trimmed = download_url.trim();
        if !is_valid_stream_scheme(trimmed)
            || (!trimmed.to_ascii_lowercase().starts_with("http://")
                && !trimmed.to_ascii_lowercase().starts_with("https://"))
        {
            return Err(DownloadError::InvalidUrl(
                "Downloads require an http:// or https:// URL".into(),
            ));
        }

        let base_dir = resolve_download_dir(custom_dir).canonicalize().unwrap_or_else(|_| {
            let _ = std::fs::create_dir_all(&self.default_dir);
            self.default_dir.clone()
        });
        let _ = std::fs::create_dir_all(&base_dir);

        let cleaned = clean_title(title);
        let stem = sanitize_filename(if cleaned.is_empty() { "video" } else { &cleaned });
        let ext = extract_allowed_extension(trimmed);
        let now = now_ms();
        let filename = format!("{stem}_{now}.{ext}");
        let dest = base_dir.join(&filename);
        if !dest.starts_with(&base_dir) {
            return Err(DownloadError::PathTraversal);
        }

        let task = DownloadTask {
            id: Uuid::new_v4().to_string(),
            title: if cleaned.is_empty() { stem } else { cleaned },
            download_url: trimmed.to_string(),
            headers,
            save_path: dest.to_string_lossy().into_owned(),
            progress: 0.0,
            speed: String::new(),
            status: DownloadStatus::Pending,
            total_bytes: 0,
            downloaded_bytes: 0,
            added_at_ms: now,
        };

        self.storage.upsert_download(&task)?;
        let cb = Arc::new(on_event);
        cb(&task);
        self.spawn_worker(task.clone(), cb);
        Ok(task)
    }

    pub fn pause_or_cancel(&self, id: &str, mark_paused: bool) -> Result<Vec<DownloadTask>, DownloadError> {
        if let Ok(flags) = self.cancel_flags.lock() {
            if let Some(flag) = flags.get(id) {
                flag.store(true, Ordering::Relaxed);
            }
        }
        let tasks = self.storage.list_downloads()?;
        if let Some(mut task) = tasks.into_iter().find(|t| t.id == id) {
            task.status = if mark_paused {
                DownloadStatus::Paused
            } else {
                DownloadStatus::Failed
            };
            task.speed.clear();
            self.storage.upsert_download(&task)?;
        }
        Ok(self.storage.list_downloads()?)
    }

    pub fn resume<F>(&self, id: &str, on_event: F) -> Result<Vec<DownloadTask>, DownloadError>
    where
        F: Fn(&DownloadTask) + Send + Sync + 'static,
    {
        let tasks = self.storage.list_downloads()?;
        let mut task = tasks
            .into_iter()
            .find(|t| t.id == id)
            .ok_or(DownloadError::NotFound)?;

        task.status = DownloadStatus::Pending;
        task.speed.clear();
        self.storage.upsert_download(&task)?;
        let cb = Arc::new(on_event);
        cb(&task);
        self.spawn_worker(task, cb);
        Ok(self.storage.list_downloads()?)
    }

    pub fn delete(&self, id: &str, remove_file: bool) -> Result<Vec<DownloadTask>, DownloadError> {
        if let Ok(flags) = self.cancel_flags.lock() {
            if let Some(flag) = flags.get(id) {
                flag.store(true, Ordering::Relaxed);
            }
        }
        let tasks = self.storage.list_downloads()?;
        if remove_file {
            if let Some(task) = tasks.into_iter().find(|t| t.id == id) {
                let p = Path::new(&task.save_path);
                if p.is_file() {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
        self.storage.delete_download(id)?;
        Ok(self.storage.list_downloads()?)
    }

    fn spawn_worker<F>(&self, mut task: DownloadTask, on_event: Arc<F>)
    where
        F: Fn(&DownloadTask) + Send + Sync + 'static,
    {
        let storage = self.storage.clone();
        let client = self.client.clone();
        let semaphore = self.semaphore.clone();
        let cancel_flags = self.cancel_flags.clone();
        let custom_ffmpeg_path = self.custom_ffmpeg_path.clone();

        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = cancel_flags.lock() {
            map.insert(task.id.clone(), flag.clone());
        }

        tokio::spawn(async move {
            let Ok(_permit) = semaphore.acquire().await else {
                return;
            };

            // Verify task wasn't paused/deleted while waiting in queue
            let current_status = storage
                .list_downloads()
                .ok()
                .and_then(|list| list.into_iter().find(|t| t.id == task.id))
                .map(|t| t.status);
            if current_status != Some(DownloadStatus::Pending) {
                if let Ok(mut map) = cancel_flags.lock() {
                    map.remove(&task.id);
                }
                return;
            }

            task.status = DownloadStatus::Downloading;
            let _ = storage.upsert_download(&task);
            on_event(&task);

            let custom_ff = custom_ffmpeg_path.read().ok().and_then(|g| g.clone());
            let result = if is_segmented_playlist_url(&task.download_url) {
                run_hls_ffmpeg_download(
                    custom_ff.as_deref(),
                    &mut task,
                    &flag,
                    &storage,
                    &*on_event,
                )
                .await
            } else {
                run_http_download(&client, &mut task, &flag, &storage, &*on_event).await
            };
            if let Ok(mut map) = cancel_flags.lock() {
                map.remove(&task.id);
            }

            // Re-read status in case pause/delete happened mid-flight
            let latest_status = storage
                .list_downloads()
                .ok()
                .and_then(|list| list.into_iter().find(|t| t.id == task.id))
                .map(|t| t.status);

            match (result, latest_status) {
                (Ok(()), Some(DownloadStatus::Downloading)) => {
                    task.status = DownloadStatus::Completed;
                    task.progress = 100.0;
                    task.speed.clear();
                    let _ = storage.upsert_download(&task);
                    on_event(&task);
                }
                (Err(_), Some(DownloadStatus::Downloading)) => {
                    task.status = DownloadStatus::Failed;
                    task.speed.clear();
                    let _ = storage.upsert_download(&task);
                    on_event(&task);
                }
                _ => {
                    if let Ok(Some(fresh)) = storage
                        .list_downloads()
                        .map(|l| l.into_iter().find(|t| t.id == task.id))
                    {
                        on_event(&fresh);
                    }
                }
            }
        });
    }
}

async fn run_http_download<F>(
    client: &Client,
    task: &mut DownloadTask,
    cancel_flag: &AtomicBool,
    storage: &Storage,
    on_event: &F,
) -> Result<(), DownloadError>
where
    F: Fn(&DownloadTask),
{
    let dest = Path::new(&task.save_path);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let existing_len = tokio::fs::metadata(dest).await.map(|m| m.len()).unwrap_or(0);
    let mut headers = HeaderMap::new();
    if let Some(custom) = &task.headers {
        for (k, v) in custom {
            if let (Ok(hn), Ok(hv)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(hn, hv);
            }
        }
    }
    if existing_len > 0 {
        if let Ok(val) = HeaderValue::from_str(&format!("bytes={existing_len}-")) {
            headers.insert(RANGE, val);
        }
    }

    let mut resp = client
        .get(&task.download_url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(DownloadError::Io(std::io::Error::other(format!(
            "HTTP {}",
            resp.status()
        ))));
    }

    let is_partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let base_offset = if is_partial { existing_len } else { 0 };
    if let Some(cl) = resp.content_length() {
        task.total_bytes = base_offset + cl;
    }
    task.downloaded_bytes = base_offset;

    let file = if is_partial {
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest)
            .await?
    } else {
        tokio::fs::File::create(dest).await?
    };
    let mut writer = tokio::io::BufWriter::new(file);

    let mut last_tick = Instant::now();
    let mut bytes_since_tick: u64 = 0;

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?
    {
        if cancel_flag.load(Ordering::Relaxed) {
            writer.flush().await?;
            return Err(DownloadError::Io(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "cancelled",
            )));
        }
        writer.write_all(&chunk).await?;
        let n = chunk.len() as u64;
        task.downloaded_bytes += n;
        bytes_since_tick += n;

        let elapsed = last_tick.elapsed();
        if elapsed >= Duration::from_millis(250) {
            let bps = (bytes_since_tick as f64 / elapsed.as_secs_f64()) as u64;
            task.speed = format_speed(bps);
            if task.total_bytes > 0 {
                task.progress = ((task.downloaded_bytes as f64 / task.total_bytes as f64) * 100.0)
                    .min(100.0) as f32;
            }
            let _ = storage.upsert_download(task);
            on_event(task);
            last_tick = Instant::now();
            bytes_since_tick = 0;
        }
    }

    writer.flush().await?;
    Ok(())
}

fn is_segmented_playlist_url(url_str: &str) -> bool {
    let lower = url_str
        .split('?')
        .next()
        .unwrap_or(url_str)
        .split('#')
        .next()
        .unwrap_or(url_str)
        .to_ascii_lowercase();
    lower.ends_with(".m3u8") || lower.ends_with(".mpd") || url_str.to_ascii_lowercase().contains(".m3u8?")
}

async fn run_hls_ffmpeg_download<F>(
    custom_ffmpeg_path: Option<&str>,
    task: &mut DownloadTask,
    cancel_flag: &AtomicBool,
    storage: &Storage,
    on_event: &F,
) -> Result<(), DownloadError>
where
    F: Fn(&DownloadTask),
{
    let Some(ffmpeg_exe) = resolve_ffmpeg_exe(custom_ffmpeg_path) else {
        return Err(DownloadError::Io(std::io::Error::other(
            "HLS (.m3u8) downloads require FFmpeg. Download FFmpeg in Preferences.",
        )));
    };

    let hdrs = task.headers.clone().unwrap_or_default();
    let known_duration = if let Some(ffprobe_exe) = resolve_ffprobe_exe(custom_ffmpeg_path) {
        probe_media(&ffprobe_exe, &task.download_url, &hdrs)
            .await
            .ok()
            .and_then(|p| p.duration_sec)
            .unwrap_or(0.0)
    } else {
        0.0
    };

    let dest = Path::new(&task.save_path);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let args = build_hls_download_args(&task.download_url, &hdrs, dest);
    let mut cmd = TokioCommand::new(&ffmpeg_exe);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    apply_no_window_tokio(&mut cmd);

    let mut child = cmd.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("ffmpeg stdout unavailable"))?;
    let mut lines = BufReader::new(stdout).lines();

    let mut last_emit = Instant::now();
    let mut out_bytes: u64 = 0;
    let mut out_time_sec: f64 = 0.0;
    let mut current_speed = String::new();

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            return Err(DownloadError::Io(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "cancelled",
            )));
        }

        match tokio::time::timeout(Duration::from_millis(250), lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                if let Some((k, v)) = line.split_once('=') {
                    match k.trim() {
                        "total_size" => {
                            if let Ok(b) = v.trim().parse::<u64>() {
                                out_bytes = b;
                            }
                        }
                        "out_time_us" => {
                            if let Ok(us) = v.trim().parse::<f64>() {
                                out_time_sec = us / 1_000_000.0;
                            }
                        }
                        "speed" => {
                            let s = v.trim();
                            if !s.is_empty() && s != "N/A" {
                                current_speed = format!("FFmpeg {s}");
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(e)) => return Err(DownloadError::Io(e)),
            Err(_) => {}
        }

        if last_emit.elapsed() >= Duration::from_millis(400) {
            task.downloaded_bytes = out_bytes;
            if !current_speed.is_empty() {
                task.speed = current_speed.clone();
            }
            if known_duration > 0.0 {
                task.progress = ((out_time_sec / known_duration) * 100.0).min(99.5) as f32;
            }
            let _ = storage.upsert_download(task);
            on_event(task);
            last_emit = Instant::now();
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        return Err(DownloadError::Io(std::io::Error::other(format!(
            "ffmpeg remux exited with {}",
            status
        ))));
    }
    if let Ok(meta) = tokio::fs::metadata(dest).await {
        task.downloaded_bytes = meta.len();
        task.total_bytes = meta.len();
    }
    Ok(())
}

pub fn resolve_download_dir(custom_dir: Option<&str>) -> PathBuf {
    if let Some(custom) = custom_dir.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(custom);
    }
    dirs::video_dir()
        .or_else(dirs::download_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("IPTVMine Pro")
}

pub fn open_containing_folder(path: &str) -> Result<(), DownloadError> {
    let p = Path::new(path);
    let target = if p.exists() {
        p.to_path_buf()
    } else if let Some(parent) = p.parent().filter(|d| d.exists()) {
        parent.to_path_buf()
    } else {
        return Err(DownloadError::NotFound);
    };

    #[cfg(target_os = "windows")]
    {
        if target.is_file() {
            std::process::Command::new("explorer")
                .arg(format!("/select,{}", target.display()))
                .spawn()?;
        } else {
            std::process::Command::new("explorer").arg(&target).spawn()?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if target.is_file() {
            cmd.arg("-R");
        }
        cmd.arg(&target).spawn()?;
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let dir = if target.is_file() {
            target.parent().unwrap_or(&target)
        } else {
            &target
        };
        std::process::Command::new("xdg-open").arg(dir).spawn()?;
    }
    Ok(())
}

fn extract_allowed_extension(url_str: &str) -> &str {
    let clean = url_str
        .split('?')
        .next()
        .unwrap_or(url_str)
        .split('#')
        .next()
        .unwrap_or(url_str);
    let ext = clean.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    ALLOWED_EXTS
        .iter()
        .copied()
        .find(|allowed| *allowed == ext)
        .unwrap_or("mp4")
}

fn format_speed(bps: u64) -> String {
    if bps >= 1024 * 1024 {
        format!("{:.1} MB/s", bps as f64 / (1024.0 * 1024.0))
    } else if bps >= 1024 {
        format!("{} KB/s", bps / 1024)
    } else {
        format!("{bps} B/s")
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_allowlisted_extension_or_defaults_to_mp4() {
        assert_eq!(
            extract_allowed_extension("https://cdn.example.com/movie.mkv?token=1"),
            "mkv"
        );
        assert_eq!(
            extract_allowed_extension("https://cdn.example.com/stream.exe"),
            "mp4"
        );
    }

    #[test]
    fn rejects_non_http_download_urls() {
        let storage = Arc::new(Storage::open_in_memory().unwrap());
        let mgr = DownloadManager::new(storage, Client::new(), None);
        let res = mgr.enqueue("Test", "file:///c:/windows/win.ini", None, None, |_| {});
        assert!(matches!(res, Err(DownloadError::InvalidUrl(_))));
    }
}
