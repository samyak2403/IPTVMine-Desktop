use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum ProviderType {
    #[default]
    Iptv,
    Vega,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(default)]
    pub channel_count: usize,
    #[serde(default)]
    pub r#type: ProviderType,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub name: String,
    #[serde(default)]
    pub logo_url: String,
    pub stream_url: String,
    #[serde(default = "default_http")]
    pub stream_type: String,
    #[serde(default = "default_uncategorized")]
    pub category: String,
    #[serde(default)]
    pub team1: String,
    #[serde(default)]
    pub team2: String,
    #[serde(default = "default_unknown")]
    pub status: String,
    #[serde(default)]
    pub start_time: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_http() -> String {
    "HTTP".to_string()
}

fn default_uncategorized() -> String {
    "Uncategorized".to_string()
}

fn default_unknown() -> String {
    "UNKNOWN".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VegaProvider {
    pub display_name: String,
    pub value: String,
    pub version: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default = "default_global")]
    pub r#type: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default = "default_true")]
    pub is_installed: bool,
}

fn default_global() -> String {
    "global".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VegaCatalog {
    pub title: String,
    pub filter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VegaPost {
    pub title: String,
    pub link: String,
    #[serde(default)]
    pub image: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaDirectLink {
    pub title: String,
    pub link: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub episode_number: Option<String>,
    #[serde(default)]
    pub season_number: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaLink {
    pub title: String,
    #[serde(default)]
    pub episodes_link: Option<String>,
    #[serde(default)]
    pub direct_links: Option<Vec<VegaDirectLink>>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub episode_number: Option<String>,
    #[serde(default)]
    pub season_number: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaMeta {
    pub title: String,
    #[serde(default)]
    pub synopsis: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub imdb_id: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub link_list: Vec<VegaLink>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub background: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeEnrichment {
    #[serde(default)]
    pub season: Option<u32>,
    #[serde(default)]
    pub episode: Option<u32>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailEnrichment {
    #[serde(default)]
    pub imdb_id: Option<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub episodes: Vec<EpisodeEnrichment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VegaStream {
    pub server: String,
    pub link: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchHistoryEntry {
    pub title: String,
    pub link: String,
    pub stream_url: String,
    #[serde(default)]
    pub image_url: String,
    #[serde(default)]
    pub provider_url: String,
    #[serde(default)]
    pub scraper_value: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub last_watched_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DownloadStatus {
    Pending,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    pub title: String,
    pub download_url: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    pub save_path: String,
    pub progress: f32,
    pub speed: String,
    pub status: DownloadStatus,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub added_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DnsMode {
    #[default]
    Cloudflare,
    Google,
    Adguard,
    Custom,
    Off,
}

impl DnsMode {
    pub fn key(&self) -> &'static str {
        match self {
            DnsMode::Cloudflare => "cloudflare",
            DnsMode::Google => "google",
            DnsMode::Adguard => "adguard",
            DnsMode::Custom => "custom",
            DnsMode::Off => "off",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key.trim().to_ascii_lowercase().as_str() {
            "google" => DnsMode::Google,
            "adguard" => DnsMode::Adguard,
            "custom" => DnsMode::Custom,
            "off" => DnsMode::Off,
            _ => DnsMode::Cloudflare,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StartupTab {
    #[default]
    Home,
    Television,
    Movies,
    Category,
}

fn default_true_val() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub black_theme: bool,
    #[serde(default)]
    pub startup_tab: StartupTab,
    #[serde(default)]
    pub dns_mode: DnsMode,
    #[serde(default)]
    pub custom_doh_url: String,
    #[serde(default)]
    pub developer_test_mode: bool,
    #[serde(default)]
    pub external_player_path: String,
    #[serde(default)]
    pub download_dir: String,
    #[serde(default = "default_true_val")]
    pub ffmpeg_enabled: bool,
    #[serde(default)]
    pub ffmpeg_path: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            black_theme: false,
            startup_tab: StartupTab::Home,
            dns_mode: DnsMode::Cloudflare,
            custom_doh_url: String::new(),
            developer_test_mode: false,
            external_player_path: String::new(),
            download_dir: String::new(),
            ffmpeg_enabled: true,
            ffmpeg_path: String::new(),
        }
    }
}

/// Removes Private Use Area code points and non-whitespace ISO control characters,
/// collapsing whitespace runs to single spaces.
pub fn clean_title(raw: &str) -> String {
    let filtered: String = raw
        .chars()
        .filter_map(|ch| {
            let cp = ch as u32;
            let is_pua = (0xE000..=0xF8FF).contains(&cp)
                || (0xF0000..=0xFFFFD).contains(&cp)
                || (0x100000..=0x10FFFD).contains(&cp);
            if is_pua {
                None
            } else if ch.is_whitespace() {
                Some(' ')
            } else if ch.is_control() {
                None
            } else {
                Some(ch)
            }
        })
        .collect();

    filtered.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Algorithmically prioritizes valid media streams by streaming capability, transport security,
/// direct media playability, and descending quality resolution (zero hardcoded mirror brands).
pub fn prioritize_streams(streams: Vec<VegaStream>) -> Vec<VegaStream> {
    if streams.is_empty() {
        return streams;
    }

    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<VegaStream> = streams
        .into_iter()
        .filter(|s| !s.link.trim().is_empty() && seen.insert(s.link.clone()))
        .collect();

    unique.sort_by(|a, b| {
        let rank = |s: &VegaStream| -> u8 {
            let link = s.link.trim().to_ascii_lowercase();
            let clean = link
                .split('?')
                .next()
                .unwrap_or(&link)
                .split('#')
                .next()
                .unwrap_or(&link);
            let is_https = link.starts_with("https://");
            let is_adaptive = clean.ends_with(".m3u8")
                || clean.ends_with(".mpd")
                || link.contains("/hls/")
                || link.contains("/dash/")
                || matches!(s.r#type.trim().to_ascii_lowercase().as_str(), "m3u8" | "hls" | "mpd" | "dash");
            let is_direct = crate::is_direct_video_url(clean);

            if is_adaptive && is_https {
                0
            } else if is_direct && is_https {
                1
            } else if is_https {
                2
            } else if is_adaptive || is_direct || link.starts_with("http://") {
                3
            } else {
                4
            }
        };

        rank(a)
            .cmp(&rank(b))
            .then_with(|| parse_quality(&b.quality).cmp(&parse_quality(&a.quality)))
            .then_with(|| b.quality.cmp(&a.quality))
    });

    unique
}

fn parse_quality(q: &str) -> u32 {
    let digits: String = q.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().unwrap_or(0)
}

/// Checks structurally whether a URL is a direct media stream or playlist without hardcoded hostnames.
pub fn is_playable_media_url(url_str: &str) -> bool {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let clean = lower
        .split('?')
        .next()
        .unwrap_or(&lower)
        .split('#')
        .next()
        .unwrap_or(&lower);

    const MEDIA_EXTS: &[&str] = &[
        ".m3u8", ".mp4", ".mkv", ".mpd", ".ts", ".webm", ".avi", ".mov", ".flv", ".wmv", ".3gp",
        ".m4v",
    ];

    MEDIA_EXTS.iter().any(|ext| clean.ends_with(ext))
        || clean.contains(".m3u8")
        || lower.contains("/hls/")
        || lower.contains(".m3u8")
        || clean.contains(".mpd")
        || lower.contains("/dash/")
        || lower.contains(".mpd")
        || lower.starts_with("rtmp://")
        || lower.starts_with("rtmps://")
        || lower.starts_with("rtsp://")
        || lower.starts_with("udp://")
        || lower.starts_with("rtp://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_title_strips_pua_and_collapses_spaces() {
        let input = "\u{E001}  Movie\t\nTitle \u{F8FF}  HD ";
        assert_eq!(clean_title(input), "Movie Title HD");
    }

    #[test]
    fn prioritize_streams_orders_mirrors_and_quality() {
        let streams = vec![
            VegaStream {
                server: "Legacy HTTP".into(),
                link: "http://cdn.example.com/1080.mp4".into(),
                r#type: "mp4".into(),
                quality: "1080".into(),
                ..Default::default()
            },
            VegaStream {
                server: "Direct 720".into(),
                link: "https://cdn.example.com/720.mp4".into(),
                r#type: "mp4".into(),
                quality: "720".into(),
                ..Default::default()
            },
            VegaStream {
                server: "Adaptive 1080".into(),
                link: "https://cdn.example.com/master.m3u8".into(),
                r#type: "m3u8".into(),
                quality: "1080".into(),
                ..Default::default()
            },
        ];

        let sorted = prioritize_streams(streams);
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].server, "Adaptive 1080");
        assert_eq!(sorted[1].server, "Direct 720");
        assert_eq!(sorted[2].server, "Legacy HTTP");
    }

    #[test]
    fn playable_media_url_detects_streams_and_rejects_html() {
        assert!(is_playable_media_url("https://cdn.example.com/master.m3u8?token=abc"));
        assert!(is_playable_media_url("https://cdn.example.com/video.mp4"));
        assert!(is_playable_media_url("rtmp://live.example.com/app/stream"));
        assert!(!is_playable_media_url("https://example.com/watch/movie-123"));
        assert!(!is_playable_media_url("   "));
    }

    #[test]
    fn provider_json_roundtrip_with_camel_case_and_type() {
        let json = r#"{
            "title": "Test",
            "url": "https://example.com/list.m3u",
            "userAgent": null,
            "isActive": true,
            "channelCount": 0,
            "type": "IPTV"
        }"#;
        let parsed: Provider = serde_json::from_str(json).expect("deserialize Provider");
        assert_eq!(parsed.r#type, ProviderType::Iptv);
        let out = serde_json::to_string(&parsed).unwrap();
        assert!(out.contains("\"type\":\"IPTV\""));
    }

    #[test]
    fn app_settings_defaults_ffmpeg_enabled_on_for_legacy_json() {
        let defaults = AppSettings::default();
        assert!(defaults.ffmpeg_enabled);

        let legacy_json = r#"{"blackTheme": true, "startupTab": "movies"}"#;
        let parsed: AppSettings =
            serde_json::from_str(legacy_json).expect("deserialize legacy AppSettings");
        assert!(parsed.black_theme);
        assert_eq!(parsed.startup_tab, StartupTab::Movies);
        assert!(parsed.ffmpeg_enabled);
        assert!(parsed.ffmpeg_path.is_empty());
    }
}
