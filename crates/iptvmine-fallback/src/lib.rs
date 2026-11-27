use iptvmine_core::{
    clean_title, is_playable_media_url, prioritize_streams, Channel, VegaMeta, VegaPost,
    VegaStream,
};
use iptvmine_vega::VegaRunner;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio::time::timeout;
use url::Url;

const HOST_COOLDOWN: Duration = Duration::from_secs(120);
const MIN_TITLE_SIMILARITY: f64 = 0.45;
const MAX_LIVE_PROBES: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackResult {
    pub selected_stream: VegaStream,
    pub all_streams: Vec<VegaStream>,
    pub server_title: String,
    pub provider_value: String,
    pub is_fallback: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveFallbackResult {
    pub selected_stream: Option<VegaStream>,
    pub all_streams: Vec<VegaStream>,
    pub hidden_dead_count: usize,
    pub is_fallback: bool,
    pub message: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct HostHealth {
    successes: u32,
    failures: u32,
    last_failure: Option<Instant>,
    last_error: Option<String>,
}

impl HostHealth {
    fn score(&self, now: Instant) -> i32 {
        let mut pts = (self.successes.min(12) as i32) * 8 - (self.failures.min(10) as i32) * 14;
        if let Some(fail_at) = self.last_failure {
            if now.duration_since(fail_at) < HOST_COOLDOWN {
                pts -= 60;
            }
        }
        pts
    }

    fn is_in_cooldown(&self, now: Instant) -> bool {
        self.last_failure
            .map(|t| now.duration_since(t) < HOST_COOLDOWN)
            .unwrap_or(false)
    }
}

/// Algorithmic stream resolution, similarity matching, health ranking, and
/// silent dead-link filtering engine across both VOD extensions and Live TV playlists.
#[derive(Clone)]
pub struct StreamFallbackEngine {
    client: Client,
    host_health: Arc<RwLock<HashMap<String, HostHealth>>>,
    ext_health: Arc<RwLock<HashMap<String, HostHealth>>>,
}

impl StreamFallbackEngine {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            host_health: Arc::new(RwLock::new(HashMap::new())),
            ext_health: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn clear_health(&self) {
        if let Ok(mut g) = self.host_health.write() {
            g.clear();
        }
        if let Ok(mut g) = self.ext_health.write() {
            g.clear();
        }
    }

    fn host_key(url_str: &str) -> String {
        Url::parse(url_str.trim())
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
            .unwrap_or_default()
    }

    pub fn record_host_success(&self, url_str: &str) {
        let host = Self::host_key(url_str);
        if host.is_empty() {
            return;
        }
        if let Ok(mut map) = self.host_health.write() {
            let entry = map.entry(host).or_default();
            entry.successes = entry.successes.saturating_add(1);
            entry.last_failure = None;
            entry.last_error = None;
        }
    }

    pub fn record_host_failure(&self, url_str: &str, err: Option<String>) {
        let host = Self::host_key(url_str);
        if host.is_empty() {
            return;
        }
        if let Ok(mut map) = self.host_health.write() {
            let entry = map.entry(host).or_default();
            entry.failures = entry.failures.saturating_add(1);
            entry.last_failure = Some(Instant::now());
            entry.last_error = err;
        }
    }

    fn record_ext_result(&self, ext: &str, ok: bool) {
        let key = ext.trim().to_ascii_lowercase();
        if key.is_empty() {
            return;
        }
        if let Ok(mut map) = self.ext_health.write() {
            let entry = map.entry(key).or_default();
            if ok {
                entry.successes = entry.successes.saturating_add(1);
                entry.last_failure = None;
            } else {
                entry.failures = entry.failures.saturating_add(1);
                entry.last_failure = Some(Instant::now());
            }
        }
    }

    pub fn rank_streams(&self, streams: Vec<VegaStream>) -> Vec<VegaStream> {
        let prioritized = prioritize_streams(streams);
        let now = Instant::now();
        let health_map = self.host_health.read().ok();

        let mut scored: Vec<(usize, i32, VegaStream)> = prioritized
            .into_iter()
            .enumerate()
            .map(|(idx, s)| {
                let host = Self::host_key(&s.link);
                let health_bonus = health_map
                    .as_ref()
                    .and_then(|m| m.get(&host))
                    .map(|h| h.score(now))
                    .unwrap_or(0);
                // Preserve structural base score while penalizing degraded hosts
                let base_score = 1000_i32 - (idx as i32 * 5) + health_bonus;
                (idx, base_score, s)
            })
            .collect();

        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.into_iter().map(|(_, _, s)| s).collect()
    }

    pub fn rank_extensions(&self, installed: &[String], exclude: &str) -> Vec<String> {
        let now = Instant::now();
        let health_map = self.ext_health.read().ok();
        let mut ranked: Vec<(usize, i32, String)> = installed
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.eq_ignore_ascii_case(exclude))
            .map(|(idx, e)| {
                let h_score = health_map
                    .as_ref()
                    .and_then(|m| m.get(&e.to_ascii_lowercase()))
                    .map(|h| h.score(now))
                    .unwrap_or(0);
                // Keep caller's natural order first (descending base) blended with health telemetry
                let score = -(idx as i32 * 4) + h_score;
                (idx, score, e.clone())
            })
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked.into_iter().map(|(_, _, e)| e).collect()
    }

    async fn probe_stream_alive(
        client: &Client,
        url_str: &str,
        headers: &HashMap<String, String>,
    ) -> Result<(), String> {
        let lower = url_str.trim().to_ascii_lowercase();
        if lower.starts_with("rtmp://")
            || lower.starts_with("rtmps://")
            || lower.starts_with("rtsp://")
            || lower.starts_with("udp://")
            || lower.starts_with("rtp://")
        {
            return Ok(());
        }
        if !lower.starts_with("http://") && !lower.starts_with("https://") {
            return Err("Unsupported stream scheme".into());
        }

        let mut req = client.get(url_str).header("Range", "bytes=0-2047");
        for (k, v) in headers {
            if !k.eq_ignore_ascii_case("range") {
                req = req.header(k.as_str(), v.as_str());
            }
        }

        let resp = timeout(Duration::from_secs(5), req.send())
            .await
            .map_err(|_| "Stream connection timed out".to_string())?
            .map_err(|e| format!("Network error: {e}"))?;

        let status = resp.status();
        if status.is_success() || status.as_u16() == 206 {
            return Ok(());
        }
        Err(format!(
            "HTTP {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Error")
        ))
    }

    async fn try_resolve_link(
        &self,
        runner: &VegaRunner,
        repo_url: &str,
        scraper_value: &str,
        link: &str,
        stream_type: &str,
    ) -> Vec<VegaStream> {
        if link.trim().is_empty() {
            return Vec::new();
        }

        let mut streams = timeout(
            Duration::from_secs(25),
            runner.get_stream(repo_url, scraper_value, link, stream_type),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !s.link.trim().is_empty())
        .collect::<Vec<_>>();

        if streams.is_empty() {
            let alt = if stream_type.eq_ignore_ascii_case("movie") {
                "series"
            } else {
                "movie"
            };
            streams = timeout(
                Duration::from_secs(20),
                runner.get_stream(repo_url, scraper_value, link, alt),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.link.trim().is_empty())
            .collect::<Vec<_>>();
        }

        if streams.is_empty() && is_playable_media_url(link) {
            streams.push(VegaStream {
                server: "Direct Source".into(),
                link: link.trim().to_string(),
                r#type: infer_stream_ext(link),
                quality: "HD".into(),
                ..Default::default()
            });
        }

        // Filter out hosts known to be in recent hard cooldown if healthy alternatives exist
        let ranked = self.rank_streams(streams);
        let now = Instant::now();
        let healthy_only: Vec<VegaStream> = {
            let map = self.host_health.read().ok();
            ranked
                .iter()
                .filter(|s| {
                    let host = Self::host_key(&s.link);
                    !map.as_ref()
                        .and_then(|m| m.get(&host))
                        .map(|h| h.is_in_cooldown(now))
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        };

        if !healthy_only.is_empty() {
            healthy_only
        } else {
            ranked
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn resolve_vod<F>(
        &self,
        runner: &VegaRunner,
        initial_link: &str,
        initial_type: &str,
        item_title: &str,
        movie_title: &str,
        current_meta: Option<&VegaMeta>,
        repo_url: &str,
        current_scraper: &str,
        installed_extensions: &[String],
        mut on_status: F,
    ) -> Option<FallbackResult>
    where
        F: FnMut(&str),
    {
        let safe_type = if initial_type.eq_ignore_ascii_case("series") {
            "series"
        } else {
            "movie"
        };

        // 1. Direct target attempt
        on_status("Resolving stream links...");
        let initial_streams = self
            .try_resolve_link(runner, repo_url, current_scraper, initial_link, safe_type)
            .await;
        if let Some(first) = initial_streams.first().cloned() {
            self.record_ext_result(current_scraper, true);
            return Some(FallbackResult {
                selected_stream: first,
                all_streams: initial_streams,
                server_title: "Direct Link".into(),
                provider_value: current_scraper.to_string(),
                is_fallback: false,
                message: None,
            });
        }

        // 2. Sibling directLinks on the same server
        let all_servers = current_meta.map(|m| m.link_list.as_slice()).unwrap_or(&[]);
        let current_server = all_servers.iter().find(|s| {
            s.direct_links
                .as_ref()
                .map(|links| links.iter().any(|dl| dl.link == initial_link))
                .unwrap_or(false)
        });

        if let Some(server) = current_server {
            for dl in server.direct_links.as_deref().unwrap_or(&[]) {
                if dl.link == initial_link || dl.link.trim().is_empty() {
                    continue;
                }
                let display = if dl.title.trim().is_empty() {
                    server.title.as_str()
                } else {
                    dl.title.as_str()
                };
                on_status(&format!("Auto-trying mirror: {display}..."));
                let t = if dl.r#type.trim().is_empty() {
                    safe_type
                } else {
                    dl.r#type.as_str()
                };
                let streams = self
                    .try_resolve_link(runner, repo_url, current_scraper, &dl.link, t)
                    .await;
                if let Some(first) = streams.first().cloned() {
                    self.record_ext_result(current_scraper, true);
                    return Some(FallbackResult {
                        selected_stream: first,
                        all_streams: streams,
                        server_title: display.to_string(),
                        provider_value: current_scraper.to_string(),
                        is_fallback: true,
                        message: Some(format!("Auto-connected to mirror: {display}")),
                    });
                }
            }
        }

        // 3. Other servers under the same metadata item
        for server in all_servers {
            if current_server
                .map(|cs| cs.title == server.title)
                .unwrap_or(false)
            {
                continue;
            }
            for dl in server.direct_links.as_deref().unwrap_or(&[]) {
                if dl.link.trim().is_empty() {
                    continue;
                }
                on_status(&format!("Auto-trying server: {}...", server.title));
                let t = if dl.r#type.trim().is_empty() {
                    safe_type
                } else {
                    dl.r#type.as_str()
                };
                let streams = self
                    .try_resolve_link(runner, repo_url, current_scraper, &dl.link, t)
                    .await;
                if let Some(first) = streams.first().cloned() {
                    self.record_ext_result(current_scraper, true);
                    return Some(FallbackResult {
                        selected_stream: first,
                        all_streams: streams,
                        server_title: server.title.clone(),
                        provider_value: current_scraper.to_string(),
                        is_fallback: true,
                        message: Some(format!("Auto-switched to server: {}", server.title)),
                    });
                }
            }

            if let Some(ep_link) = server.episodes_link.as_deref().filter(|s| !s.trim().is_empty())
            {
                on_status(&format!("Checking server episodes: {}...", server.title));
                if let Ok(Ok(episodes)) = timeout(
                    Duration::from_secs(15),
                    runner.get_episodes(repo_url, current_scraper, ep_link),
                )
                .await
                {
                    let matched = episodes.iter().max_by(|a, b| {
                        let sa = title_similarity(item_title, &a.title);
                        let sb = title_similarity(item_title, &b.title);
                        sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    if let Some(ep) = matched.filter(|ep| {
                        title_similarity(item_title, &ep.title) >= 0.40
                            || ep.title.eq_ignore_ascii_case(item_title)
                    }) {
                        for dl in ep.direct_links.as_deref().unwrap_or(&[]) {
                            if dl.link.trim().is_empty() {
                                continue;
                            }
                            let t = if dl.r#type.trim().is_empty() {
                                safe_type
                            } else {
                                dl.r#type.as_str()
                            };
                            let streams = self
                                .try_resolve_link(runner, repo_url, current_scraper, &dl.link, t)
                                .await;
                            if let Some(first) = streams.first().cloned() {
                                self.record_ext_result(current_scraper, true);
                                return Some(FallbackResult {
                                    selected_stream: first,
                                    all_streams: streams,
                                    server_title: format!("{} - {}", server.title, ep.title),
                                    provider_value: current_scraper.to_string(),
                                    is_fallback: true,
                                    message: Some(format!(
                                        "Auto-switched to server: {}",
                                        server.title
                                    )),
                                });
                            }
                        }
                    }
                }
            }
        }

        // 4. Dynamic health-ranked cross-extension fallback using algorithmic title similarity
        self.record_ext_result(current_scraper, false);
        let others = self.rank_extensions(installed_extensions, current_scraper);
        let clean_query = {
            let m = get_searchable_title(movie_title);
            if m.is_empty() {
                get_searchable_title(item_title)
            } else {
                m
            }
        };

        if !clean_query.is_empty() {
            for ext in others {
                on_status(&format!("Auto-searching in {ext}..."));
                let results: Vec<VegaPost> = timeout(
                    Duration::from_secs(15),
                    runner.get_search_posts(repo_url, &ext, &clean_query, 1),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();

                let mut scored_posts: Vec<(f64, &VegaPost)> = results
                    .iter()
                    .map(|p| {
                        let candidate = get_searchable_title(&p.title);
                        let score = title_similarity(&clean_query, &candidate);
                        (score, p)
                    })
                    .filter(|(score, _)| *score >= MIN_TITLE_SIMILARITY)
                    .collect();
                scored_posts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

                let Some((_, matched_post)) = scored_posts.first().copied() else {
                    continue;
                };

                on_status(&format!("Matched `{}` in {ext}, resolving...", matched_post.title));
                let Ok(Ok(ext_meta)) = timeout(
                    Duration::from_secs(15),
                    runner.get_meta(repo_url, &ext, &matched_post.link),
                )
                .await
                else {
                    self.record_ext_result(&ext, false);
                    continue;
                };

                for server in &ext_meta.link_list {
                    for dl in server.direct_links.as_deref().unwrap_or(&[]) {
                        if dl.link.trim().is_empty() {
                            continue;
                        }
                        let t = if dl.r#type.trim().is_empty() {
                            safe_type
                        } else {
                            dl.r#type.as_str()
                        };
                        let streams = self
                            .try_resolve_link(runner, repo_url, &ext, &dl.link, t)
                            .await;
                        if let Some(first) = streams.first().cloned() {
                            self.record_ext_result(&ext, true);
                            return Some(FallbackResult {
                                selected_stream: first,
                                all_streams: streams,
                                server_title: format!("{ext} - {}", server.title),
                                provider_value: ext.clone(),
                                is_fallback: true,
                                message: Some(format!("Auto-connected via {ext} ({})", server.title)),
                            });
                        }
                    }
                }
                self.record_ext_result(&ext, false);
            }
        }

        // 5. Direct media fallback
        if is_playable_media_url(initial_link) {
            let direct = VegaStream {
                server: "Direct Source".into(),
                link: initial_link.to_string(),
                r#type: infer_stream_ext(initial_link),
                quality: "HD".into(),
                ..Default::default()
            };
            return Some(FallbackResult {
                selected_stream: direct.clone(),
                all_streams: vec![direct],
                server_title: "Direct Source".into(),
                provider_value: current_scraper.to_string(),
                is_fallback: false,
                message: None,
            });
        }

        None
    }

    /// Gathers normalized candidate streams for a Live TV channel, verifies reachability
    /// concurrently, automatically hides dead/offline links, and picks the healthiest stream.
    pub async fn resolve_live_channel(
        &self,
        selected: &Channel,
        all_channels: &[Channel],
    ) -> LiveFallbackResult {
        let target_key = normalize_channel_key(&selected.name);
        let mut candidates: Vec<(usize, f64, VegaStream)> = Vec::new();
        let mut seen_links = HashSet::new();

        let push_cand = |idx: usize,
                         sim: f64,
                         ch: &Channel,
                         out: &mut Vec<(usize, f64, VegaStream)>,
                         seen: &mut HashSet<String>| {
            let trimmed = ch.stream_url.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
                return;
            }
            let q = extract_channel_resolution(&ch.name);
            out.push((
                idx,
                sim,
                VegaStream {
                    server: if idx == 0 {
                        "Primary".into()
                    } else {
                        ch.name.clone()
                    },
                    link: trimmed.to_string(),
                    r#type: infer_stream_ext(trimmed),
                    quality: if q > 0 { format!("{q}p") } else { ch.stream_type.clone() },
                    headers: if ch.headers.is_empty() {
                        None
                    } else {
                        Some(ch.headers.clone())
                    },
                    ..Default::default()
                },
            ));
        };

        push_cand(0, 1.0, selected, &mut candidates, &mut seen_links);

        if !target_key.is_empty() {
            let mut matched: Vec<(f64, &Channel)> = all_channels
                .iter()
                .filter(|ch| ch.stream_url.trim() != selected.stream_url.trim())
                .filter_map(|ch| {
                    let ck = normalize_channel_key(&ch.name);
                    if ck == target_key {
                        Some((1.0, ch))
                    } else {
                        let sim = title_similarity(&target_key, &ck);
                        if sim >= 0.85 {
                            Some((sim, ch))
                        } else {
                            None
                        }
                    }
                })
                .collect();

            matched.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        extract_channel_resolution(&b.1.name)
                            .cmp(&extract_channel_resolution(&a.1.name))
                    })
            });

            for (i, (sim, ch)) in matched.into_iter().take(MAX_LIVE_PROBES).enumerate() {
                push_cand(i + 1, sim, ch, &mut candidates, &mut seen_links);
            }
        }

        let probe_targets: Vec<VegaStream> = candidates
            .into_iter()
            .take(MAX_LIVE_PROBES)
            .map(|(_, _, s)| s)
            .collect();

        let mut join_set = JoinSet::new();
        for (idx, stream) in probe_targets.into_iter().enumerate() {
            let client = self.client.clone();
            join_set.spawn(async move {
                let hdrs = stream.headers.clone().unwrap_or_default();
                let res = Self::probe_stream_alive(&client, &stream.link, &hdrs).await;
                (idx, stream, res)
            });
        }

        let mut checked = Vec::new();
        while let Some(joined) = join_set.join_next().await {
            if let Ok(item) = joined {
                checked.push(item);
            }
        }
        checked.sort_by_key(|(idx, _, _)| *idx);

        let mut healthy = Vec::new();
        let mut dead_errors = Vec::new();
        for (idx, stream, res) in checked {
            match res {
                Ok(()) => {
                    self.record_host_success(&stream.link);
                    healthy.push((idx, stream));
                }
                Err(err) => {
                    self.record_host_failure(&stream.link, Some(err.clone()));
                    dead_errors.push(err);
                }
            }
        }

        let hidden_dead_count = dead_errors.len();
        if !healthy.is_empty() {
            let primary_failed = healthy[0].0 != 0;
            let raw_streams: Vec<VegaStream> = healthy.into_iter().map(|(_, s)| s).collect();
            let ranked = if primary_failed {
                self.rank_streams(raw_streams)
            } else {
                let mut rest = self.rank_streams(raw_streams[1..].to_vec());
                let mut merged = vec![raw_streams[0].clone()];
                merged.append(&mut rest);
                merged
            };
            let first = ranked[0].clone();
            let msg = if primary_failed {
                Some(format!(
                    "Auto-switched to working mirror: {} ({hidden_dead_count} offline source(s) hidden)",
                    first.server
                ))
            } else if hidden_dead_count > 0 {
                Some(format!("Hid {hidden_dead_count} offline mirror(s)"))
            } else {
                None
            };
            return LiveFallbackResult {
                selected_stream: Some(first),
                all_streams: ranked,
                hidden_dead_count,
                is_fallback: primary_failed,
                message: msg,
                error: None,
            };
        }

        let reason = dead_errors
            .first()
            .cloned()
            .unwrap_or_else(|| "Channel stream offline or unreachable".into());
        LiveFallbackResult {
            selected_stream: None,
            all_streams: Vec::new(),
            hidden_dead_count,
            is_fallback: false,
            message: None,
            error: Some(if hidden_dead_count > 1 {
                format!("All {hidden_dead_count} sources offline ({reason})")
            } else {
                format!("Channel stream offline or unavailable ({reason})")
            }),
        }
    }
}

fn infer_stream_ext(link: &str) -> String {
    let lower = link.trim().to_ascii_lowercase();
    let clean = lower
        .split('?')
        .next()
        .unwrap_or(&lower)
        .split('#')
        .next()
        .unwrap_or(&lower);
    if clean.ends_with(".m3u8") || lower.contains("/hls/") {
        "m3u8".into()
    } else if clean.ends_with(".mpd") || lower.contains("/dash/") {
        "mpd".into()
    } else if let Some((_, ext)) = clean.rsplit_once('.') {
        if matches!(
            ext,
            "mp4" | "mkv" | "webm" | "mov" | "ts" | "flv" | "wmv" | "avi"
        ) {
            return ext.to_string();
        }
        "auto".into()
    } else {
        "auto".into()
    }
}

pub fn get_searchable_title(title: &str) -> String {
    static BRACKETS: OnceLock<Regex> = OnceLock::new();
    static PARENS: OnceLock<Regex> = OnceLock::new();
    static SEASON_EP: OnceLock<Regex> = OnceLock::new();
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static SEPS: OnceLock<Regex> = OnceLock::new();

    let base = clean_title(title);
    if base.is_empty() {
        return String::new();
    }
    let brackets = BRACKETS.get_or_init(|| Regex::new(r"\[.*?\]").unwrap());
    let parens = PARENS.get_or_init(|| Regex::new(r"\(.*?\)").unwrap());
    let season_ep = SEASON_EP.get_or_init(|| {
        Regex::new(r"(?i)\b(season\s*\d+|s\d+|episode\s*\d+|ep\s*\d+|part\s*\d+)\b.*").unwrap()
    });
    let tags = TAGS.get_or_init(|| {
        Regex::new(r"(?i)\b(hindi|english|tamil|telugu|kannada|malayalam|dubbed|dual audio|multi audio|org audio|web-dl|bluray|hdrip|hdcam|2160p|1080p|720p|480p|4k|hevc|x264|x265)\b.*").unwrap()
    });
    let seps = SEPS.get_or_init(|| Regex::new(r"[-–—:|]+").unwrap());

    let s = brackets.replace_all(&base, " ");
    let s = parens.replace_all(&s, " ");
    let s = season_ep.replace_all(&s, " ");
    let s = tags.replace_all(&s, " ");
    let s = seps.replace_all(&s, " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn normalize_channel_key(name: &str) -> String {
    static BRACKETS: OnceLock<Regex> = OnceLock::new();
    static PARENS: OnceLock<Regex> = OnceLock::new();
    static QUALITIES: OnceLock<Regex> = OnceLock::new();

    let base = clean_title(name).to_ascii_lowercase();
    let brackets = BRACKETS.get_or_init(|| Regex::new(r"\[.*?\]").unwrap());
    let parens = PARENS.get_or_init(|| Regex::new(r"\(.*?\)").unwrap());
    let qualities = QUALITIES
        .get_or_init(|| Regex::new(r"\b(fhd|uhd|hd|sd|4k|hevc|h\.?264|h\.?265)\b").unwrap());

    let s = brackets.replace_all(&base, " ");
    let s = parens.replace_all(&s, " ");
    let s = qualities.replace_all(&s, " ");
    let alpha: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    alpha.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn extract_channel_resolution(name: &str) -> u32 {
    static RES_RE: OnceLock<Regex> = OnceLock::new();
    let re = RES_RE.get_or_init(|| Regex::new(r"(?i)\b(2160|1440|1080|720|576|480|360)p\b").unwrap());
    re.captures(name)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u32>().ok())
        .unwrap_or(0)
}

pub fn title_similarity(a: &str, b: &str) -> f64 {
    let clean_a: Vec<String> = a
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let clean_b: Vec<String> = b
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    if clean_a.is_empty() || clean_b.is_empty() {
        return 0.0;
    }

    let joined_a = clean_a.join(" ");
    let joined_b = clean_b.join(" ");
    if joined_a == joined_b {
        return 1.0;
    }

    let set_a: HashSet<&str> = clean_a.iter().map(String::as_str).collect();
    let set_b: HashSet<&str> = clean_b.iter().map(String::as_str).collect();
    let intersection = set_a.intersection(&set_b).count() as f64;
    let dice = (2.0 * intersection) / ((set_a.len() + set_b.len()) as f64);

    if joined_a.contains(&joined_b) || joined_b.contains(&joined_a) {
        dice.max(0.75)
    } else {
        dice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_normalization_and_similarity_score_cleanly() {
        assert_eq!(
            get_searchable_title("Inception (2010) [Hindi Dual Audio] 1080p BluRay"),
            "Inception"
        );
        assert_eq!(
            get_searchable_title("Breaking Bad Season 2 Episode 5"),
            "Breaking Bad"
        );
        assert_eq!(normalize_channel_key("Aaj Tak HD (1080p) [Geo-blocked]"), "aaj tak");
        assert!(title_similarity("Inception", "Inception Dual Audio") >= 0.6);
        assert!(title_similarity("Inception", "Completely Different Movie") < 0.2);
    }

    #[test]
    fn dynamic_health_ranking_demotes_degraded_hosts_without_hardcoding() {
        let engine = StreamFallbackEngine::new(Client::new());
        let bad = VegaStream {
            server: "Server A".into(),
            link: "https://degraded.example.com/stream.m3u8".into(),
            r#type: "m3u8".into(),
            quality: "1080p".into(),
            ..Default::default()
        };
        let good = VegaStream {
            server: "Server B".into(),
            link: "https://healthy.example.net/stream.m3u8".into(),
            r#type: "m3u8".into(),
            quality: "720p".into(),
            ..Default::default()
        };

        engine.record_host_failure(&bad.link, Some("HTTP 404".into()));
        engine.record_host_success(&good.link);

        let ranked = engine.rank_streams(vec![bad.clone(), good.clone()]);
        assert_eq!(ranked[0].link, good.link);

        let exts = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
        engine.record_ext_result("Alpha", false);
        engine.record_ext_result("Beta", true);
        let ext_order = engine.rank_extensions(&exts, "Current");
        assert_eq!(ext_order[0], "Beta");
    }

    #[tokio::test]
    async fn live_fallback_hides_dead_404_mirrors_and_picks_healthy_stream() {
        use axum::{http::StatusCode, routing::get, Router};

        let app = Router::new()
            .route("/dead.m3u8", get(|| async { StatusCode::NOT_FOUND }))
            .route("/live.m3u8", get(|| async { (StatusCode::OK, "#EXTM3U\n") }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let selected = Channel {
            name: "News One (576p)".into(),
            logo_url: String::new(),
            stream_url: format!("http://{addr}/dead.m3u8"),
            stream_type: "HLS".into(),
            category: "News".into(),
            team1: String::new(),
            team2: String::new(),
            status: "LIVE".into(),
            start_time: String::new(),
            headers: HashMap::new(),
        };
        let mirror = Channel {
            name: "News One HD (1080p)".into(),
            stream_url: format!("http://{addr}/live.m3u8"),
            ..selected.clone()
        };
        let other = Channel {
            name: "Sports Channel (720p)".into(),
            stream_url: format!("http://{addr}/other.m3u8"),
            ..selected.clone()
        };

        let engine = StreamFallbackEngine::new(Client::new());
        let res = engine
            .resolve_live_channel(&selected, &[selected.clone(), mirror.clone(), other])
            .await;

        assert!(res.is_fallback);
        assert_eq!(res.hidden_dead_count, 1);
        assert_eq!(res.all_streams.len(), 1);
        assert_eq!(res.selected_stream.unwrap().link, mirror.stream_url);
        assert!(res.error.is_none());
    }
}
