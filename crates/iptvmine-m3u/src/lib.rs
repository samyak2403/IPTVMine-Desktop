use iptvmine_core::{
    clean_title, detect_provider_quick, is_direct_video_url, Channel, ContentFilter,
    ProviderType, VegaProvider,
};
use iptvmine_net::to_jsdelivr_mirror;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_RESPONSE_SIZE: usize = 100 * 1024 * 1024; // 100 MB
const FETCH_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const BASE_BACKOFF: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(3600);

#[derive(Debug, Error)]
pub enum M3uError {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("blocked binary content type: {0}")]
    BlockedContentType(String),
    #[error("response exceeded 100 MB cap")]
    ResponseTooLarge,
    #[error("source is in backoff for {0}s")]
    InBackoff(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceKind {
    VodScraperMap,
    VodManifest,
    IptvM3u,
    IptvJson,
    DirectVideo,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScraperEndpoint {
    pub key: String,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionResult {
    pub provider_type: ProviderType,
    pub source_kind: SourceKind,
    pub scraper_entries: HashMap<String, ScraperEndpoint>,
    pub manifest_entries: Vec<VegaProvider>,
}

#[derive(Debug, Clone)]
struct FailureState {
    failure_count: u32,
    next_retry_at: Instant,
}

pub struct ChannelEngine {
    backoffs: Mutex<HashMap<String, FailureState>>,
    last_fetch: Mutex<Option<(Instant, Vec<String>)>>,
}

impl Default for ChannelEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelEngine {
    pub fn new() -> Self {
        Self {
            backoffs: Mutex::new(HashMap::new()),
            last_fetch: Mutex::new(None),
        }
    }

    pub fn clear_backoffs(&self) {
        if let Ok(mut b) = self.backoffs.lock() {
            b.clear();
        }
    }

    pub fn should_skip_cooldown(&self, urls: &[String]) -> bool {
        if let Ok(guard) = self.last_fetch.lock() {
            if let Some((at, prev_urls)) = &*guard {
                return at.elapsed() < FETCH_COOLDOWN && prev_urls == urls;
            }
        }
        false
    }

    pub fn mark_fetched(&self, urls: &[String]) {
        if let Ok(mut guard) = self.last_fetch.lock() {
            *guard = Some((Instant::now(), urls.to_vec()));
        }
    }

    pub fn check_backoff(&self, url: &str) -> Result<(), u64> {
        if let Ok(guard) = self.backoffs.lock() {
            if let Some(st) = guard.get(url) {
                let now = Instant::now();
                if now < st.next_retry_at {
                    return Err((st.next_retry_at - now).as_secs().max(1));
                }
            }
        }
        Ok(())
    }

    pub fn record_success(&self, url: &str) {
        if let Ok(mut guard) = self.backoffs.lock() {
            guard.remove(url);
        }
    }

    pub fn record_failure(&self, url: &str) {
        if let Ok(mut guard) = self.backoffs.lock() {
            let count = guard.get(url).map(|s| s.failure_count + 1).unwrap_or(1);
            let shift = (count - 1).min(6);
            let delay = (BASE_BACKOFF * (1u32 << shift)).min(MAX_BACKOFF);
            guard.insert(
                url.to_string(),
                FailureState {
                    failure_count: count,
                    next_retry_at: Instant::now() + delay,
                },
            );
        }
    }
}

pub fn is_blocked_content_type(raw_content_type: Option<&str>) -> bool {
    let ct = raw_content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if ct.is_empty() {
        return false;
    }
    const PLAYLIST_TYPES: &[&str] = &[
        "audio/x-mpegurl",
        "audio/mpegurl",
        "application/x-mpegurl",
        "application/mpegurl",
        "application/vnd.apple.mpegurl",
        "application/vnd.apple.mpegurl.audio",
        "application/octet-stream",
    ];
    if PLAYLIST_TYPES.contains(&ct.as_str())
        || ct.contains("mpegurl")
        || ct.starts_with("text/")
        || ct == "application/json"
        || ct.ends_with("+json")
        || ct == "application/xml"
        || ct.ends_with("+xml")
    {
        return false;
    }
    ct.starts_with("video/") || ct.starts_with("image/") || ct.starts_with("audio/")
}

pub async fn fetch_playlist_text(
    client: &Client,
    url: &str,
    user_agent: Option<&str>,
) -> Result<String, M3uError> {
    let mut req = client.get(url);
    if let Some(ua) = user_agent.filter(|s| !s.trim().is_empty()) {
        req = req.header("User-Agent", ua);
    }
    let mut resp = req.send().await?.error_for_status()?;
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if is_blocked_content_type(ct.as_deref()) {
        return Err(M3uError::BlockedContentType(ct.unwrap_or_default()));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_SIZE {
            return Err(M3uError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn parse_playlist(raw: &str, filter: &ContentFilter) -> Vec<Channel> {
    let trimmed = raw.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            let channels = parse_iptv_json(&val, filter);
            if !channels.is_empty() {
                return channels;
            }
        }
    }
    parse_m3u(trimmed, filter)
}

pub fn parse_m3u(raw: &str, filter: &ContentFilter) -> Vec<Channel> {
    let mut channels = Vec::new();
    let mut pending_name = String::new();
    let mut pending_logo = String::new();
    let mut pending_category = "Uncategorized".to_string();
    let mut pending_headers: HashMap<String, String> = HashMap::new();
    let mut has_extinf = false;

    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("#EXTM3U") {
            continue;
        }
        if line.starts_with("#EXTINF:") {
            has_extinf = true;
            pending_headers.clear();
            let (attrs, raw_title) = split_extinf(line);
            let sanitized_title = match raw_title.rsplit_once(',') {
                Some((left, right)) if left.contains("=\"") || left.contains("='") => right.trim(),
                _ => raw_title,
            };
            pending_name = clean_title(sanitized_title);
            pending_logo = ["tvg-logo", "logo", "tvg_logo", "x-logo", "channel-logo", "tvg-icon"]
                .iter()
                .find_map(|key| extract_attr(attrs, key))
                .filter(|val| !val.eq_ignore_ascii_case("null") && !val.eq_ignore_ascii_case("undefined"))
                .unwrap_or_default();
            pending_category = extract_attr(attrs, "group-title")
                .map(|c| clean_title(&c))
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| "Uncategorized".to_string());
            if pending_name.is_empty() {
                pending_name = extract_attr(attrs, "tvg-name")
                    .map(|n| clean_title(&n))
                    .unwrap_or_else(|| "Unnamed Channel".to_string());
            }
        } else if let Some(rest) = line.strip_prefix("#EXTVLCOPT:") {
            if let Some(ua) = rest.strip_prefix("http-user-agent=") {
                pending_headers.insert("User-Agent".into(), ua.trim().to_string());
            } else if let Some(ref_val) = rest.strip_prefix("http-referrer=") {
                pending_headers.insert("Referer".into(), ref_val.trim().to_string());
            }
        } else if line.starts_with('#') {
            continue;
        } else if is_stream_line(line) {
            let name = if has_extinf && !pending_name.is_empty() {
                pending_name.clone()
            } else {
                format!("Channel {}", channels.len() + 1)
            };
            let category = pending_category.clone();
            if !filter.should_block_content(Some(&name), Some(&category)) {
                let stream_type = detect_stream_type(line);
                channels.push(Channel {
                    name,
                    logo_url: pending_logo.clone(),
                    stream_url: line.to_string(),
                    stream_type,
                    category,
                    team1: String::new(),
                    team2: String::new(),
                    status: "LIVE".to_string(),
                    start_time: String::new(),
                    headers: pending_headers.clone(),
                });
            }
            has_extinf = false;
            pending_name.clear();
            pending_logo.clear();
            pending_category = "Uncategorized".to_string();
            pending_headers.clear();
        }
    }

    channels
}

fn split_extinf(line: &str) -> (&str, &str) {
    let mut quote_char: Option<char> = None;
    for (idx, ch) in line.char_indices() {
        match ch {
            '"' | '\'' if quote_char.is_none() => quote_char = Some(ch),
            q if quote_char == Some(q) => quote_char = None,
            ',' if quote_char.is_none() => return (&line[..idx], line[idx + 1..].trim()),
            _ => {}
        }
    }
    (line, "")
}

fn extract_attr(attrs: &str, key: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let key_lower = key.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut search_from = 0;

    while let Some(rel) = lower[search_from..].find(&key_lower) {
        let pos = search_from + rel;
        let boundary_ok = pos == 0 || {
            let prev = bytes[pos - 1];
            prev.is_ascii_whitespace() || prev == b':' || prev == b','
        };
        let mut cursor = pos + key_lower.len();
        if boundary_ok {
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < bytes.len() && bytes[cursor] == b'=' {
                cursor += 1;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                if cursor < bytes.len() && (bytes[cursor] == b'"' || bytes[cursor] == b'\'') {
                    let quote = bytes[cursor] as char;
                    let start = cursor + 1;
                    if let Some(end) = attrs[start..].find(quote) {
                        let value = attrs[start..start + end].trim();
                        if !value.is_empty() {
                            return Some(value.to_string());
                        }
                    }
                }
            }
        }
        search_from = pos + key_lower.len();
    }
    None
}

fn is_stream_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("rtmp://")
        || lower.starts_with("rtmps://")
        || lower.starts_with("rtsp://")
        || lower.starts_with("udp://")
        || lower.starts_with("rtp://")
}

fn detect_stream_type(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    if lower.contains(".m3u8") {
        "HLS".into()
    } else if lower.contains(".mpd") {
        "DASH".into()
    } else if lower.starts_with("rtmp://") || lower.starts_with("rtmps://") {
        "RTMP".into()
    } else {
        "HTTP".into()
    }
}

pub fn parse_iptv_json(val: &Value, filter: &ContentFilter) -> Vec<Channel> {
    let mut out = Vec::new();
    collect_json_channels(val, "Uncategorized", filter, &mut out);
    out
}

fn collect_json_channels(
    val: &Value,
    inherited_cat: &str,
    filter: &ContentFilter,
    out: &mut Vec<Channel>,
) {
    const URL_KEYS: &[&str] = &[
        "url",
        "link",
        "stream",
        "stream_url",
        "streamurl",
        "streamUrl",
        "source_url",
        "adfree_url",
        "dai_url",
        "file",
        "playback_url",
        "manifest",
        "hls",
        "m3u8",
        "playbackUrl",
        "streamLink",
    ];
    const NAME_KEYS: &[&str] = &[
        "name",
        "title",
        "channel",
        "channel_name",
        "channelName",
        "tvg-name",
        "tvg_name",
        "tvgName",
        "display_name",
    ];
    const LOGO_KEYS: &[&str] = &[
        "logo",
        "logo_url",
        "logoUrl",
        "tvg-logo",
        "tvg_logo",
        "tvgLogo",
        "icon",
        "image",
        "img",
        "thumbnail",
        "poster",
        "src",
    ];
    const CAT_KEYS: &[&str] = &[
        "group",
        "group-title",
        "group_title",
        "groupTitle",
        "category",
        "genre",
        "event_category",
        "categoryName",
        "type",
    ];
    const CONTAINER_KEYS: &[&str] = &[
        "channels",
        "streams",
        "data",
        "items",
        "list",
        "results",
        "result",
        "playlist",
        "playlists",
        "stations",
        "entries",
        "categories",
        "content",
        "matches",
    ];

    match val {
        Value::Array(arr) => {
            for item in arr {
                collect_json_channels(item, inherited_cat, filter, out);
            }
        }
        Value::Object(map) => {
            let stream_url = URL_KEYS
                .iter()
                .find_map(|k| map.get(*k).and_then(|v| v.as_str()))
                .map(str::trim)
                .filter(|s| is_stream_line(s));

            if let Some(url) = stream_url {
                let name = NAME_KEYS
                    .iter()
                    .find_map(|k| map.get(*k).and_then(|v| v.as_str()))
                    .map(clean_title)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("Channel {}", out.len() + 1));
                let logo_url = LOGO_KEYS
                    .iter()
                    .find_map(|k| map.get(*k).and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let category = CAT_KEYS
                    .iter()
                    .find_map(|k| map.get(*k).and_then(|v| v.as_str()))
                    .map(clean_title)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| inherited_cat.to_string());
                let team1 = map
                    .get("team_1")
                    .or_else(|| map.get("team1"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let team2 = map
                    .get("team_2")
                    .or_else(|| map.get("team2"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let status = map
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("LIVE")
                    .to_string();
                let start_time = map
                    .get("startTime")
                    .or_else(|| map.get("start_time"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if !filter.should_block_content(Some(&name), Some(&category)) {
                    out.push(Channel {
                        name,
                        logo_url,
                        stream_url: url.to_string(),
                        stream_type: detect_stream_type(url),
                        category,
                        team1,
                        team2,
                        status,
                        start_time,
                        headers: HashMap::new(),
                    });
                }
                return;
            }

            for (k, v) in map {
                let next_cat = if CONTAINER_KEYS.contains(&k.to_ascii_lowercase().as_str()) {
                    inherited_cat
                } else {
                    k.as_str()
                };
                collect_json_channels(v, next_cat, filter, out);
            }
        }
        _ => {}
    }
}

pub async fn inspect_provider_url(client: &Client, raw_url: &str) -> DetectionResult {
    let trimmed = raw_url.trim();
    if is_direct_video_url(trimmed) {
        return DetectionResult {
            provider_type: ProviderType::Video,
            source_kind: SourceKind::DirectVideo,
            scraper_entries: HashMap::new(),
            manifest_entries: Vec::new(),
        };
    }

    let lower = trimmed.to_ascii_lowercase();
    let path = lower.split('?').next().unwrap_or(&lower);
    if path.ends_with(".m3u") {
        return DetectionResult {
            provider_type: ProviderType::Iptv,
            source_kind: SourceKind::IptvM3u,
            scraper_entries: HashMap::new(),
            manifest_entries: Vec::new(),
        };
    }

    let mut candidates = Vec::new();
    if let Some(handle) = trimmed.strip_prefix('@') {
        if let Some((owner, rest)) = handle.split_once('/') {
            let (repo, branch) = rest.split_once('@').unwrap_or((rest, "main"));
            if !owner.is_empty() && !repo.is_empty() {
                let raw_root = format!(
                    "https://raw.githubusercontent.com/{owner}/{repo}/refs/heads/{branch}"
                );
                candidates.push(format!("{raw_root}/manifest.json"));
                if let Some(mirror) = to_jsdelivr_mirror(&raw_root) {
                    candidates.push(format!("{mirror}/manifest.json"));
                }
            }
        } else if !handle.is_empty() {
            let api_url = format!(
                "https://api.github.com/users/{handle}/repos?per_page=8&sort=updated"
            );
            if let Ok(resp) = client
                .get(&api_url)
                .header(reqwest::header::USER_AGENT, "IPTVMine-Desktop/0.1")
                .send()
                .await
            {
                if let Ok(Value::Array(repos)) = resp.json::<Value>().await {
                    for item in repos {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            let branch = item
                                .get("default_branch")
                                .and_then(|b| b.as_str())
                                .unwrap_or("main");
                            let raw_root = format!(
                                "https://raw.githubusercontent.com/{handle}/{name}/refs/heads/{branch}"
                            );
                            candidates.push(format!("{raw_root}/manifest.json"));
                        }
                    }
                }
            }
        }
    } else {
        candidates.push(trimmed.to_string());
        if let Some(mirror) = to_jsdelivr_mirror(trimmed) {
            candidates.push(mirror);
        }
        if !path.ends_with(".json") && !path.ends_with(".m3u") && !path.ends_with(".m3u8") {
            let clean_root = trimmed.trim_end_matches('/');
            candidates.push(format!("{clean_root}/manifest.json"));
            if let Some(mirror) = to_jsdelivr_mirror(clean_root) {
                candidates.push(format!("{mirror}/manifest.json"));
            }
        }
    }

    for candidate in candidates {
        if let Ok(resp) = client.get(&candidate).send().await {
            if !resp.status().is_success() {
                continue;
            }
            if let Ok(body) = resp.text().await {
                if let Some(res) = classify_payload(&body) {
                    return res;
                }
            }
        }
    }

    let fallback_type = detect_provider_quick(trimmed, "");
    DetectionResult {
        provider_type: fallback_type,
        source_kind: match fallback_type {
            ProviderType::Video => SourceKind::DirectVideo,
            ProviderType::Vega => SourceKind::VodManifest,
            ProviderType::Iptv => SourceKind::IptvM3u,
        },
        scraper_entries: HashMap::new(),
        manifest_entries: Vec::new(),
    }
}

pub fn classify_payload(body: &str) -> Option<DetectionResult> {
    let trimmed = body.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with("#EXTM3U") || trimmed.starts_with("#EXTINF") {
        return Some(DetectionResult {
            provider_type: ProviderType::Iptv,
            source_kind: SourceKind::IptvM3u,
            scraper_entries: HashMap::new(),
            manifest_entries: Vec::new(),
        });
    }

    let val: Value = serde_json::from_str(trimmed).ok()?;
    if let Value::Array(arr) = &val {
        if let Ok(manifest) = serde_json::from_value::<Vec<VegaProvider>>(Value::Array(arr.clone()))
        {
            if !manifest.is_empty() && manifest.iter().any(|m| !m.value.is_empty()) {
                return Some(DetectionResult {
                    provider_type: ProviderType::Vega,
                    source_kind: SourceKind::VodManifest,
                    scraper_entries: HashMap::new(),
                    manifest_entries: manifest,
                });
            }
        }
    }

    if let Value::Object(obj) = &val {
        let mut endpoints = HashMap::new();
        for (k, v) in obj {
            if let Some(url) = v.get("url").and_then(|u| u.as_str()) {
                let name = v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(k.as_str())
                    .to_string();
                endpoints.insert(
                    k.clone(),
                    ScraperEndpoint {
                        key: k.clone(),
                        name,
                        url: url.to_string(),
                    },
                );
            }
        }
        if !endpoints.is_empty() {
            return Some(DetectionResult {
                provider_type: ProviderType::Vega,
                source_kind: SourceKind::VodScraperMap,
                scraper_entries: endpoints,
                manifest_entries: Vec::new(),
            });
        }
    }

    let channels = parse_iptv_json(&val, &ContentFilter::default());
    if !channels.is_empty() {
        return Some(DetectionResult {
            provider_type: ProviderType::Iptv,
            source_kind: SourceKind::IptvJson,
            scraper_entries: HashMap::new(),
            manifest_entries: Vec::new(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extm3u_and_filters_adult_channels() {
        let m3u = r#"#EXTM3U
#EXTINF:-1 tvg-logo="https://img/news.png" group-title="News", World News HD
#EXTVLCOPT:http-user-agent=MyPlayer/2.0
https://cdn.example.com/news/master.m3u8
#EXTINF:-1 group-title="Adult 18+", Blocked Channel
https://cdn.example.com/blocked.m3u8
"#;
        let channels = parse_playlist(m3u, &ContentFilter::new());
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].name, "World News HD");
        assert_eq!(channels[0].logo_url, "https://img/news.png");
        assert_eq!(channels[0].category, "News");
        assert_eq!(channels[0].stream_type, "HLS");
        assert_eq!(
            channels[0].headers.get("User-Agent").map(String::as_str),
            Some("MyPlayer/2.0")
        );

        let variants = r#"#EXTM3U
#EXTINF:-1 tvg-logo='https://img/single.png' group-title="Sports", Sports Single
https://cdn.example.com/sports.m3u8
#EXTINF:-1 logo="https://img/fallback.png" group-title="Music", Music Fallback
https://cdn.example.com/music.m3u8
"#;
        let parsed = parse_playlist(variants, &ContentFilter::new());
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].logo_url, "https://img/single.png");
        assert_eq!(parsed[1].logo_url, "https://img/fallback.png");

        let broken_ua = r#"#EXTM3U
#EXTINF:-1 tvg-logo="https://img/4tv.png" group-title="News",like Gecko) Chrome/147.0.0.0 Safari/537.36" group-title="News",4TV News (576p)
https://cdn.example.com/4tv.m3u8
"#;
        let cleaned = parse_playlist(broken_ua, &ContentFilter::new());
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0].name, "4TV News (576p)");
        assert_eq!(cleaned[0].logo_url, "https://img/4tv.png");
    }

    #[test]
    fn classifies_vega_manifest_and_iptv_json() {
        let manifest = r#"[{"display_name":"Vega","value":"vega","version":"1.0"}]"#;
        let res = classify_payload(manifest).unwrap();
        assert_eq!(res.source_kind, SourceKind::VodManifest);
        assert_eq!(res.manifest_entries.len(), 1);

        let iptv_json = r#"{"channels":[{"name":"Sports 1","url":"https://live.example.com/1.m3u8","group":"Sports"}]}"#;
        let res2 = classify_payload(iptv_json).unwrap();
        assert_eq!(res2.source_kind, SourceKind::IptvJson);
    }

    #[test]
    fn blocked_content_type_rejects_mp4_but_allows_mpegurl() {
        assert!(is_blocked_content_type(Some("video/mp4")));
        assert!(!is_blocked_content_type(Some("application/x-mpegurl")));
        assert!(!is_blocked_content_type(Some("audio/x-mpegurl; charset=utf-8")));
    }
}
