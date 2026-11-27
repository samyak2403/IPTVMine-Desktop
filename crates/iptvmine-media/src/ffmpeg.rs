use crate::MediaError;
use iptvmine_core::is_valid_stream_scheme;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command as TokioCommand;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_FFMPEG_ARCHIVE_BYTES: u64 = 250 * 1024 * 1024;
pub const UPSTREAM_FFMPEG_REPO: &str = "https://github.com/FFmpeg/FFmpeg";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub short_version: Option<String>,
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
    pub source: String,
    pub upstream_tag: Option<String>,
    pub upstream_version: Option<String>,
    pub update_available: bool,
    pub upstream_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAudioTrack {
    pub index: usize,
    pub stream_index: usize,
    pub codec: String,
    pub language: String,
    pub title: String,
    pub channels: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSubtitleTrack {
    pub index: usize,
    pub stream_index: usize,
    pub codec: String,
    pub language: String,
    pub title: String,
    pub text_based: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProbeResult {
    pub format_name: String,
    pub duration_sec: Option<f64>,
    pub bit_rate: Option<u64>,
    pub size_bytes: Option<u64>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub needs_transcode: bool,
    pub needs_reason: Option<String>,
    pub audio_tracks: Vec<MediaAudioTrack>,
    pub subtitle_tracks: Vec<MediaSubtitleTrack>,
}

pub fn apply_no_window_std(cmd: &mut StdCommand) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = cmd;
}

pub fn apply_no_window_tokio(cmd: &mut TokioCommand) {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = cmd;
}

pub fn managed_bin_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.samyak.iptvminedesktop")
        .join("bin")
}

fn exe_suffix(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

fn find_sibling_ffprobe(ffmpeg_exe: &Path) -> Option<PathBuf> {
    let probe_name = exe_suffix("ffprobe");
    ffmpeg_exe
        .parent()
        .map(|p| p.join(&probe_name))
        .filter(|p| p.is_file())
}

fn find_in_path(binary_name: &str) -> Option<PathBuf> {
    let file_name = exe_suffix(binary_name);
    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        let cand = dir.join(&file_name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn resolve_candidate(custom_path: Option<&str>) -> Option<(PathBuf, Option<PathBuf>, &'static str)> {
    let ffmpeg_name = exe_suffix("ffmpeg");

    if let Some(custom) = custom_path.map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(custom);
        let exe = if p.is_dir() { p.join(&ffmpeg_name) } else { p };
        if exe.is_file() {
            let probe = find_sibling_ffprobe(&exe).or_else(|| find_in_path("ffprobe"));
            return Some((exe, probe, "custom"));
        }
    }

    let managed = managed_bin_dir().join(&ffmpeg_name);
    if managed.is_file() {
        let probe = find_sibling_ffprobe(&managed).or_else(|| find_in_path("ffprobe"));
        return Some((managed, probe, "managed"));
    }

    if let Ok(cur) = std::env::current_exe() {
        if let Some(bundled) = cur.parent().map(|d| d.join("bin").join(&ffmpeg_name)) {
            if bundled.is_file() {
                let probe = find_sibling_ffprobe(&bundled).or_else(|| find_in_path("ffprobe"));
                return Some((bundled, probe, "bundled"));
            }
        }
    }

    if let Some(sys) = find_in_path("ffmpeg") {
        let probe = find_sibling_ffprobe(&sys).or_else(|| find_in_path("ffprobe"));
        return Some((sys, probe, "system"));
    }

    #[cfg(target_os = "windows")]
    {
        let mut common = vec![
            PathBuf::from(r"C:\ffmpeg\bin"),
            PathBuf::from(r"C:\Program Files\ffmpeg\bin"),
            PathBuf::from(r"C:\ProgramData\chocolatey\bin"),
        ];
        if let Ok(profile) = std::env::var("USERPROFILE") {
            common.push(PathBuf::from(&profile).join(r"scoop\apps\ffmpeg\current\bin"));
            common.push(PathBuf::from(&profile).join(r"scoop\shims"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            common.push(PathBuf::from(&local).join(r"Microsoft\WinGet\Links"));
        }
        for dir in common {
            let exe = dir.join(&ffmpeg_name);
            if exe.is_file() {
                let probe = find_sibling_ffprobe(&exe).or_else(|| find_in_path("ffprobe"));
                return Some((exe, probe, "system"));
            }
        }
    }

    None
}

fn read_ffmpeg_version(ffmpeg_exe: &Path) -> Option<String> {
    let mut cmd = StdCommand::new(ffmpeg_exe);
    cmd.arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    apply_no_window_std(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let first_line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if first_line.is_empty() {
        None
    } else {
        Some(first_line)
    }
}

pub fn parse_semver_tuple(raw: &str) -> Option<(u32, u32, u32)> {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    if lower.contains("-dev") {
        return None;
    }
    let clean = lower
        .trim_start_matches("ffmpeg version ")
        .trim_start_matches('n')
        .split(['-', ' ', '_'])
        .next()?;
    let mut parts = clean.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next().unwrap_or("0").parse().ok()?;
    let patch: u32 = parts.next().unwrap_or("0").parse().ok()?;
    if major == 0 {
        return None;
    }
    Some((major, minor, patch))
}

pub fn pick_latest_upstream_ffmpeg_tag(tags_json: &Value) -> Option<(String, String)> {
    let arr = tags_json.as_array()?;
    let mut best: Option<((u32, u32, u32), String, String)> = None;
    for entry in arr {
        let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        if !name.starts_with('n') {
            continue;
        }
        let Some(ver) = parse_semver_tuple(name) else {
            continue;
        };
        let clean_ver = name.trim_start_matches('n').to_string();
        if best.as_ref().map(|(b, _, _)| ver > *b).unwrap_or(true) {
            best = Some((ver, name.to_string(), clean_ver));
        }
    }
    best.map(|(_, tag, ver)| (tag, ver))
}

pub async fn fetch_upstream_ffmpeg_tag(client: &Client) -> Option<(String, String)> {
    let resp = client
        .get("https://api.github.com/repos/FFmpeg/FFmpeg/tags?per_page=30")
        .header(reqwest::header::USER_AGENT, "IPTVMine-Desktop/0.1")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: Value = resp.json().await.ok()?;
    pick_latest_upstream_ffmpeg_tag(&val)
}

pub fn detect_ffmpeg_status(custom_path: Option<&str>) -> FfmpegStatus {
    let Some((ffmpeg_exe, ffprobe_exe, source)) = resolve_candidate(custom_path) else {
        return FfmpegStatus {
            installed: false,
            version: None,
            short_version: None,
            ffmpeg_path: None,
            ffprobe_path: None,
            source: "none".into(),
            upstream_tag: None,
            upstream_version: None,
            update_available: false,
            upstream_url: UPSTREAM_FFMPEG_REPO.into(),
        };
    };

    let version = read_ffmpeg_version(&ffmpeg_exe);
    let short_version = version.as_deref().and_then(|v| {
        parse_semver_tuple(v).map(|(ma, mi, pa)| {
            if pa > 0 {
                format!("{ma}.{mi}.{pa}")
            } else {
                format!("{ma}.{mi}")
            }
        })
    });

    FfmpegStatus {
        installed: version.is_some(),
        version,
        short_version,
        ffmpeg_path: Some(ffmpeg_exe.to_string_lossy().into_owned()),
        ffprobe_path: ffprobe_exe.map(|p| p.to_string_lossy().into_owned()),
        source: source.into(),
        upstream_tag: None,
        upstream_version: None,
        update_available: false,
        upstream_url: UPSTREAM_FFMPEG_REPO.into(),
    }
}

pub async fn check_ffmpeg_status_with_upstream(
    client: &Client,
    custom_path: Option<&str>,
) -> FfmpegStatus {
    let mut status = detect_ffmpeg_status(custom_path);
    if let Some((tag, ver)) = fetch_upstream_ffmpeg_tag(client).await {
        let installed_ver = status
            .short_version
            .as_deref()
            .and_then(parse_semver_tuple);
        let up_ver = parse_semver_tuple(&ver);
        status.update_available = match (installed_ver, up_ver) {
            (Some(inst), Some(up)) => inst < up,
            _ => false,
        };
        status.upstream_tag = Some(tag);
        status.upstream_version = Some(ver);
    }
    status
}

pub fn resolve_ffmpeg_exe(custom_path: Option<&str>) -> Option<PathBuf> {
    resolve_candidate(custom_path).map(|(f, _, _)| f)
}

pub fn resolve_ffprobe_exe(custom_path: Option<&str>) -> Option<PathBuf> {
    resolve_candidate(custom_path).and_then(|(_, p, _)| p)
}

/// Downloads and installs the latest official FFmpeg Windows release build into app local storage.
pub async fn install_latest_ffmpeg(client: &Client) -> Result<FfmpegStatus, MediaError> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = client;
        return Err(MediaError::Ffmpeg(
            "Automatic FFmpeg installer is supported on Windows. Please install `ffmpeg` via Homebrew or your system package manager.".into(),
        ));
    }

    #[cfg(target_os = "windows")]
    {
        let upstream = fetch_upstream_ffmpeg_tag(client).await;
        let target_ver = upstream.as_ref().map(|(_, v)| v.as_str());
        let download_url = resolve_latest_windows_ffmpeg_zip_url(client, target_ver).await?;
        let bin_dir = managed_bin_dir();
        tokio::fs::create_dir_all(&bin_dir).await?;

        let zip_path = bin_dir.join("_ffmpeg_latest.zip");
        let extract_dir = bin_dir.join("_ffmpeg_extract");
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        tokio::fs::create_dir_all(&extract_dir).await?;

        let mut resp = client
            .get(&download_url)
            .header(reqwest::header::USER_AGENT, "IPTVMine-Desktop/0.1")
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .map_err(|e| MediaError::Ffmpeg(format!("FFmpeg download failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(MediaError::Ffmpeg(format!(
                "FFmpeg download returned HTTP {}",
                resp.status()
            )));
        }

        let mut file = tokio::fs::File::create(&zip_path).await?;
        let mut total_written: u64 = 0;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| MediaError::Ffmpeg(format!("FFmpeg stream read error: {e}")))?
        {
            total_written += chunk.len() as u64;
            if total_written > MAX_FFMPEG_ARCHIVE_BYTES {
                drop(file);
                let _ = tokio::fs::remove_file(&zip_path).await;
                return Err(MediaError::Ffmpeg(
                    "FFmpeg archive exceeded maximum allowed size".into(),
                ));
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);

        // Extract using Windows native bsdtar
        let mut tar_cmd = TokioCommand::new("tar.exe");
        tar_cmd
            .arg("-xf")
            .arg(&zip_path)
            .arg("-C")
            .arg(&extract_dir);
        apply_no_window_tokio(&mut tar_cmd);
        let tar_out = tar_cmd
            .output()
            .await
            .map_err(|e| MediaError::Ffmpeg(format!("Failed running tar.exe: {e}")))?;

        if !tar_out.status.success() {
            let err_txt = String::from_utf8_lossy(&tar_out.stderr);
            let _ = tokio::fs::remove_file(&zip_path).await;
            let _ = tokio::fs::remove_dir_all(&extract_dir).await;
            return Err(MediaError::Ffmpeg(format!(
                "Failed extracting FFmpeg archive: {err_txt}"
            )));
        }

        let ffmpeg_src = find_file_recursive(&extract_dir, "ffmpeg.exe").ok_or_else(|| {
            MediaError::Ffmpeg("ffmpeg.exe not found inside downloaded release archive".into())
        })?;
        let ffprobe_src = find_file_recursive(&extract_dir, "ffprobe.exe");

        let ffmpeg_dst = bin_dir.join("ffmpeg.exe");
        let ffprobe_dst = bin_dir.join("ffprobe.exe");

        move_or_copy_replace(&ffmpeg_src, &ffmpeg_dst).await?;
        if let Some(probe_src) = ffprobe_src {
            move_or_copy_replace(&probe_src, &ffprobe_dst).await?;
        }

        let _ = tokio::fs::remove_file(&zip_path).await;
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;

        let mut status = check_ffmpeg_status_with_upstream(client, None).await;
        if !status.installed {
            return Err(MediaError::Ffmpeg(
                "Installed FFmpeg binary failed version verification".into(),
            ));
        }
        if status.upstream_tag.is_none() {
            if let Some((t, v)) = upstream {
                status.upstream_tag = Some(t);
                status.upstream_version = Some(v);
            }
        }
        Ok(status)
    }
}

#[cfg(target_os = "windows")]
async fn resolve_latest_windows_ffmpeg_zip_url(
    client: &Client,
    upstream_version: Option<&str>,
) -> Result<String, MediaError> {
    // Official Windows FFmpeg builds compiled directly from https://github.com/FFmpeg/FFmpeg
    let repos = [
        ("https://api.github.com/repos/GyanD/codexffmpeg/releases/latest", "essentials_build.zip"),
        ("https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest", "win64-gpl.zip"),
    ];

    for (api_url, preferred_suffix) in repos {
        let Ok(resp) = client
            .get(api_url)
            .header(reqwest::header::USER_AGENT, "IPTVMine-Desktop/0.1")
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .timeout(Duration::from_secs(15))
            .send()
            .await
        else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(release) = resp.json::<Value>().await else {
            continue;
        };
        if let Some(assets) = release.get("assets").and_then(|a| a.as_array()) {
            // Prefer exact matching upstream FFmpeg/FFmpeg tag asset first (e.g. ffmpeg-9.1-essentials_build.zip)
            if let Some(ver) = upstream_version {
                for asset in assets {
                    let name = asset
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let url = asset
                        .get("browser_download_url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("");
                    if name.contains(ver)
                        && name.ends_with(".zip")
                        && !name.contains("shared")
                        && (name.contains("essentials") || name.contains("win64-gpl"))
                        && url.starts_with("https://")
                    {
                        return Ok(url.to_string());
                    }
                }
            }
            for asset in assets {
                let name = asset
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let url = asset
                    .get("browser_download_url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("");
                if name.ends_with(preferred_suffix)
                    && !name.contains("shared")
                    && url.starts_with("https://")
                {
                    return Ok(url.to_string());
                }
            }
        }
    }

    Ok("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip".to_string())
}

#[cfg(target_os = "windows")]
fn find_file_recursive(dir: &Path, filename: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() {
            if p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.eq_ignore_ascii_case(filename))
                .unwrap_or(false)
            {
                return Some(p);
            }
        } else if p.is_dir() {
            if let Some(found) = find_file_recursive(&p, filename) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
async fn move_or_copy_replace(src: &Path, dst: &Path) -> Result<(), MediaError> {
    if dst.exists() {
        let _ = tokio::fs::remove_file(dst).await;
    }
    if tokio::fs::rename(src, dst).await.is_err() {
        tokio::fs::copy(src, dst).await?;
    }
    Ok(())
}

/// Sanitizes custom HTTP headers and formats `-user_agent` and `-headers` safely for FFmpeg/FFprobe.
pub fn format_ffmpeg_headers(headers: &HashMap<String, String>) -> (Option<String>, Option<String>) {
    let mut user_agent: Option<String> = None;
    let mut header_lines = Vec::new();
    let mut keys: Vec<&String> = headers.keys().collect();
    keys.sort();

    for key in keys {
        let Some(val) = headers.get(key) else {
            continue;
        };
        let clean_k: String = key.chars().filter(|c| !matches!(c, '\r' | '\n' | '\0')).collect();
        let clean_v: String = val.chars().filter(|c| !matches!(c, '\r' | '\n' | '\0')).collect();
        if clean_k.trim().is_empty() || clean_v.trim().is_empty() {
            continue;
        }
        if clean_k.eq_ignore_ascii_case("user-agent") {
            user_agent = Some(clean_v);
        } else {
            header_lines.push(format!("{clean_k}: {clean_v}"));
        }
    }

    let formatted = if header_lines.is_empty() {
        None
    } else {
        Some(format!("{}\r\n", header_lines.join("\r\n")))
    };
    (user_agent, formatted)
}

fn validate_media_target(target: &str) -> Result<(), MediaError> {
    let trimmed = target.trim();
    if is_valid_stream_scheme(trimmed) || Path::new(trimmed).is_file() {
        Ok(())
    } else {
        Err(MediaError::InvalidUrl(
            "Target must be a valid stream URL or local media file".into(),
        ))
    }
}

pub async fn probe_media(
    ffprobe_path: &Path,
    target: &str,
    headers: &HashMap<String, String>,
) -> Result<MediaProbeResult, MediaError> {
    validate_media_target(target)?;
    let trimmed = target.trim();
    let is_http = trimmed.starts_with("http://") || trimmed.starts_with("https://");

    let mut cmd = TokioCommand::new(ffprobe_path);
    cmd.args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams"]);
    if is_http {
        cmd.args(["-rw_timeout", "8000000"]);
        let (ua, hdrs) = format_ffmpeg_headers(headers);
        if let Some(u) = ua {
            cmd.args(["-user_agent", &u]);
        }
        if let Some(h) = hdrs {
            cmd.args(["-headers", &h]);
        }
    }
    cmd.arg("-i").arg(trimmed);
    apply_no_window_tokio(&mut cmd);

    let out = tokio::time::timeout(Duration::from_secs(14), cmd.output())
        .await
        .map_err(|_| MediaError::Ffmpeg("ffprobe timed out".into()))?
        .map_err(MediaError::Spawn)?;

    if !out.status.success() {
        return Err(MediaError::Ffmpeg(
            "ffprobe could not inspect the media stream".into(),
        ));
    }

    let root: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| MediaError::Ffmpeg(format!("Invalid ffprobe JSON: {e}")))?;
    Ok(parse_ffprobe_json(&root, trimmed))
}

pub fn parse_ffprobe_json(root: &Value, target: &str) -> MediaProbeResult {
    let format = root.get("format");
    let format_name = format
        .and_then(|f| f.get("format_name"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let duration_sec = format
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0);
    let bit_rate = format
        .and_then(|f| f.get("bit_rate"))
        .and_then(|b| b.as_str())
        .and_then(|s| s.parse::<u64>().ok());
    let size_bytes = format
        .and_then(|f| f.get("size"))
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<u64>().ok());

    let mut video_codec: Option<String> = None;
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut audio_tracks = Vec::new();
    let mut subtitle_tracks = Vec::new();

    if let Some(streams) = root.get("streams").and_then(|s| s.as_array()) {
        for s in streams {
            let codec_type = s.get("codec_type").and_then(|v| v.as_str()).unwrap_or("");
            let codec_name = s
                .get("codec_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let stream_index = s.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let tags = s.get("tags");
            let lang = tags
                .and_then(|t| t.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("und")
                .to_string();
            let title = tags
                .and_then(|t| t.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            match codec_type {
                "video" if video_codec.is_none() => {
                    let is_attached_pic = s
                        .get("disposition")
                        .and_then(|d| d.get("attached_pic"))
                        .and_then(|v| v.as_u64())
                        == Some(1);
                    if !is_attached_pic && !codec_name.is_empty() {
                        video_codec = Some(codec_name);
                        width = s.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
                        height = s.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);
                    }
                }
                "audio" => {
                    let channels = s.get("channels").and_then(|v| v.as_u64()).unwrap_or(2) as u32;
                    let idx = audio_tracks.len();
                    let label_title = if !title.is_empty() {
                        title
                    } else if lang != "und" {
                        lang.to_uppercase()
                    } else {
                        format!("Audio {}", idx + 1)
                    };
                    audio_tracks.push(MediaAudioTrack {
                        index: idx,
                        stream_index,
                        codec: codec_name,
                        language: lang,
                        title: label_title,
                        channels,
                    });
                }
                "subtitle" => {
                    let idx = subtitle_tracks.len();
                    let text_based = matches!(
                        codec_name.as_str(),
                        "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "mov_text" | "text"
                    );
                    let label_title = if !title.is_empty() {
                        title
                    } else if lang != "und" {
                        lang.to_uppercase()
                    } else {
                        format!("Subtitle {}", idx + 1)
                    };
                    subtitle_tracks.push(MediaSubtitleTrack {
                        index: idx,
                        stream_index,
                        codec: codec_name,
                        language: lang,
                        title: label_title,
                        text_based,
                    });
                }
                _ => {}
            }
        }
    }

    let (needs_transcode, needs_reason) =
        evaluate_transcode_need(target, &format_name, video_codec.as_deref(), &audio_tracks);

    MediaProbeResult {
        format_name,
        duration_sec,
        bit_rate,
        size_bytes,
        video_codec,
        width,
        height,
        needs_transcode,
        needs_reason,
        audio_tracks,
        subtitle_tracks,
    }
}

fn evaluate_transcode_need(
    target: &str,
    format_name: &str,
    video_codec: Option<&str>,
    audio_tracks: &[MediaAudioTrack],
) -> (bool, Option<String>) {
    let lower_target = target.to_ascii_lowercase();
    let lower_fmt = format_name.to_ascii_lowercase();

    if lower_target.starts_with("rtmp://")
        || lower_target.starts_with("rtmps://")
        || lower_target.starts_with("rtsp://")
        || lower_target.starts_with("udp://")
        || lower_target.starts_with("rtp://")
    {
        return (true, Some("Transport stream remuxed via FFmpeg".into()));
    }

    if lower_fmt.contains("matroska")
        || lower_fmt.contains("avi")
        || lower_fmt.contains("flv")
        || lower_fmt.contains("asf")
        || lower_fmt.contains("mpegts")
        || lower_target.contains(".mkv")
        || lower_target.contains(".avi")
        || lower_target.contains(".flv")
        || lower_target.contains(".wmv")
    {
        return (true, Some("Container remuxed for WebView playback".into()));
    }

    if let Some(vc) = video_codec {
        if !matches!(vc, "h264" | "avc1" | "vp8" | "vp9" | "av1") {
            return (
                true,
                Some(format!("Video codec `{vc}` transcoded to H.264")),
            );
        }
    }

    if let Some(first_audio) = audio_tracks.first() {
        if !matches!(
            first_audio.codec.as_str(),
            "aac" | "mp3" | "opus" | "vorbis" | "flac"
        ) {
            return (
                true,
                Some(format!(
                    "Audio codec `{}` converted to AAC",
                    first_audio.codec
                )),
            );
        }
    }

    (false, None)
}

pub fn build_transcode_args(
    target: &str,
    headers: &HashMap<String, String>,
    audio_track: usize,
    start_sec: f64,
    copy_video: bool,
) -> Vec<String> {
    let trimmed = target.trim();
    let is_http = trimmed.starts_with("http://") || trimmed.starts_with("https://");
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];

    if start_sec > 0.5 && !trimmed.starts_with("rtmp") && !trimmed.starts_with("udp") {
        args.push("-ss".into());
        args.push(format!("{start_sec:.2}"));
    }

    if is_http {
        args.extend([
            "-reconnect".into(),
            "1".into(),
            "-reconnect_streamed".into(),
            "1".into(),
            "-reconnect_delay_max".into(),
            "5".into(),
        ]);
        let (ua, hdrs) = format_ffmpeg_headers(headers);
        if let Some(u) = ua {
            args.extend(["-user_agent".into(), u]);
        }
        if let Some(h) = hdrs {
            args.extend(["-headers".into(), h]);
        }
    }

    args.extend([
        "-i".into(),
        trimmed.to_string(),
        "-map".into(),
        "0:v:0?".into(),
        "-map".into(),
        format!("0:a:{audio_track}?"),
        "-sn".into(),
    ]);

    if copy_video {
        args.extend(["-c:v".into(), "copy".into()]);
    } else {
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "veryfast".into(),
            "-crf".into(),
            "22".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
        ]);
    }

    args.extend([
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "192k".into(),
        "-ac".into(),
        "2".into(),
        "-f".into(),
        "mp4".into(),
        "-movflags".into(),
        "frag_keyframe+empty_moov+default_base_moof+omit_tfhd_offset".into(),
        "-reset_timestamps".into(),
        "1".into(),
        "pipe:1".into(),
    ]);

    args
}

pub fn build_subtitle_vtt_args(
    target: &str,
    headers: &HashMap<String, String>,
    sub_track: usize,
) -> Vec<String> {
    let trimmed = target.trim();
    let is_http = trimmed.starts_with("http://") || trimmed.starts_with("https://");
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];

    if is_http {
        let (ua, hdrs) = format_ffmpeg_headers(headers);
        if let Some(u) = ua {
            args.extend(["-user_agent".into(), u]);
        }
        if let Some(h) = hdrs {
            args.extend(["-headers".into(), h]);
        }
    }

    args.extend([
        "-i".into(),
        trimmed.to_string(),
        "-map".into(),
        format!("0:s:{sub_track}"),
        "-c:s".into(),
        "webvtt".into(),
        "-f".into(),
        "webvtt".into(),
        "pipe:1".into(),
    ]);
    args
}

pub fn build_hls_download_args(
    url: &str,
    headers: &HashMap<String, String>,
    dest: &Path,
) -> Vec<String> {
    let mut args = vec!["-y".into(), "-hide_banner".into(), "-loglevel".into(), "error".into()];
    let (ua, hdrs) = format_ffmpeg_headers(headers);
    if let Some(u) = ua {
        args.extend(["-user_agent".into(), u]);
    }
    if let Some(h) = hdrs {
        args.extend(["-headers".into(), h]);
    }
    args.extend([
        "-i".into(),
        url.trim().to_string(),
        "-c".into(),
        "copy".into(),
        "-bsf:a".into(),
        "aac_adtstoasc".into(),
        "-f".into(),
        "mp4".into(),
        "-progress".into(),
        "pipe:1".into(),
        dest.to_string_lossy().into_owned(),
    ]);
    args
}
