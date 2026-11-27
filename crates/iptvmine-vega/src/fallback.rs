use crate::VegaRunner;
use iptvmine_core::{is_playable_media_url, prioritize_streams, VegaMeta, VegaPost, VegaStream};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;

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

const PRIORITY_ORDER: &[&str] = &[
    "Vega",
    "Moviesmod",
    "UhdMovies",
    "1cinevood",
    "Katmovie",
    "hdhub",
    "TopMovies",
    "Multi",
    "movieBoxWeb",
];

/// Port of `TitleUtils.getSearchableTitle` from IPTVMine-Pro.
pub fn get_searchable_title(title: &str) -> String {
    let base = iptvmine_core::clean_title(title);
    if base.is_empty() {
        return String::new();
    }
    let brackets = regex::Regex::new(r"\[.*?\]").unwrap();
    let parens = regex::Regex::new(r"\(.*?\)").unwrap();
    let season_ep = regex::Regex::new(
        r"(?i)\b(season\s*\d+|s\d+|episode\s*\d+|ep\s*\d+|part\s*\d+)\b.*",
    )
    .unwrap();
    let tags = regex::Regex::new(
        r"(?i)\b(hindi|english|tamil|telugu|kannada|malayalam|dubbed|dual audio|multi audio|org audio|web-dl|bluray|hdrip|hdcam|720p|1080p|480p|4k)\b.*",
    )
    .unwrap();
    let seps = regex::Regex::new(r"[-–—:|]+").unwrap();

    let s = brackets.replace_all(&base, "");
    let s = parens.replace_all(&s, "");
    let s = season_ep.replace_all(&s, "");
    let s = tags.replace_all(&s, "");
    let s = seps.replace_all(&s, " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn try_resolve_link(
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

    streams = prioritize_streams(streams);

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
        streams = prioritize_streams(streams);
    }

    let lower = link.to_ascii_lowercase();
    if streams.is_empty()
        && is_playable_media_url(link)
        && !lower.contains("pixeldrain")
        && !lower.contains("pdrain")
    {
        let ext = lower
            .split('?')
            .next()
            .unwrap_or(&lower)
            .split('#')
            .next()
            .unwrap_or(&lower)
            .rsplit('.')
            .next()
            .unwrap_or("mp4")
            .to_string();
        streams.push(VegaStream {
            server: "Direct Source".into(),
            link: link.trim().to_string(),
            r#type: ext,
            quality: "HD".into(),
            ..Default::default()
        });
    }

    streams
}

#[allow(clippy::too_many_arguments)]
pub async fn resolve_with_auto_fallback<F>(
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

    // 1. Direct attempt
    on_status("Resolving stream links...");
    let initial_streams =
        try_resolve_link(runner, repo_url, current_scraper, initial_link, safe_type).await;
    if let Some(first) = initial_streams.first().cloned() {
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
        if let Some(d_links) = &server.direct_links {
            for dl in d_links {
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
                let streams = try_resolve_link(runner, repo_url, current_scraper, &dl.link, t).await;
                if let Some(first) = streams.first().cloned() {
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
    }

    // 3. Other servers under the same meta
    for server in all_servers {
        if current_server.map(|cs| cs.title == server.title).unwrap_or(false) {
            continue;
        }
        if let Some(d_links) = &server.direct_links {
            for dl in d_links {
                if dl.link.trim().is_empty() {
                    continue;
                }
                on_status(&format!("Auto-trying server: {}...", server.title));
                let t = if dl.r#type.trim().is_empty() {
                    safe_type
                } else {
                    dl.r#type.as_str()
                };
                let streams = try_resolve_link(runner, repo_url, current_scraper, &dl.link, t).await;
                if let Some(first) = streams.first().cloned() {
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
        }

        if let Some(ep_link) = server.episodes_link.as_deref().filter(|s| !s.trim().is_empty()) {
            on_status(&format!("Checking server episodes: {}...", server.title));
            if let Ok(Ok(episodes)) = timeout(
                Duration::from_secs(15),
                runner.get_episodes(repo_url, current_scraper, ep_link),
            )
            .await
            {
                let target_lower = item_title.to_ascii_lowercase();
                let matched = episodes.iter().find(|ep| {
                    let ep_l = ep.title.to_ascii_lowercase();
                    ep_l == target_lower
                        || (target_lower.contains("episode")
                            && target_lower
                                .split("episode")
                                .nth(1)
                                .map(|s| s.trim().chars().take(4).collect::<String>())
                                .map(|sub| !sub.is_empty() && ep_l.contains(&sub))
                                .unwrap_or(false))
                });
                if let Some(ep) = matched {
                    if let Some(d_links) = &ep.direct_links {
                        for dl in d_links {
                            if dl.link.trim().is_empty() {
                                continue;
                            }
                            let t = if dl.r#type.trim().is_empty() {
                                safe_type
                            } else {
                                dl.r#type.as_str()
                            };
                            let streams =
                                try_resolve_link(runner, repo_url, current_scraper, &dl.link, t)
                                    .await;
                            if let Some(first) = streams.first().cloned() {
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
    }

    // 4. Cross-extension fallback
    let mut others: Vec<String> = installed_extensions
        .iter()
        .filter(|e| !e.eq_ignore_ascii_case(current_scraper))
        .cloned()
        .collect();
    others.sort_by_key(|ext| {
        PRIORITY_ORDER
            .iter()
            .position(|p| p.eq_ignore_ascii_case(ext))
            .unwrap_or(999)
    });

    let clean_query = {
        let m = get_searchable_title(movie_title);
        if m.is_empty() {
            get_searchable_title(item_title)
        } else {
            m
        }
    };

    if !clean_query.is_empty() {
        let words: Vec<String> = clean_query
            .to_ascii_lowercase()
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .map(str::to_string)
            .collect();

        for ext in others {
            on_status(&format!("Searching in {ext}..."));
            let results: Vec<VegaPost> = timeout(
                Duration::from_secs(15),
                runner.get_search_posts(repo_url, &ext, &clean_query, 1),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();

            let Some(matched_post) = results
                .iter()
                .find(|p| {
                    let p_low = p.title.to_ascii_lowercase();
                    words.iter().any(|w| p_low.contains(w))
                })
                .or_else(|| results.first())
            else {
                continue;
            };

            on_status(&format!("Found in {ext}, loading streams..."));
            let Ok(Ok(ext_meta)) = timeout(
                Duration::from_secs(15),
                runner.get_meta(repo_url, &ext, &matched_post.link),
            )
            .await
            else {
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
                    let streams = try_resolve_link(runner, repo_url, &ext, &dl.link, t).await;
                    if let Some(first) = streams.first().cloned() {
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
        }
    }

    // 5. Direct media fallback
    let lower_init = initial_link.to_ascii_lowercase();
    if is_playable_media_url(initial_link)
        && !lower_init.contains("pixeldrain")
        && !lower_init.contains("pdrain")
    {
        let ext = lower_init
            .split('?')
            .next()
            .unwrap_or(&lower_init)
            .split('#')
            .next()
            .unwrap_or(&lower_init)
            .rsplit('.')
            .next()
            .unwrap_or("mp4")
            .to_string();
        let direct = VegaStream {
            server: "Direct Source".into(),
            link: initial_link.to_string(),
            r#type: ext,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searchable_title_strips_audio_and_quality_tags() {
        assert_eq!(
            get_searchable_title("Inception (2010) [Hindi Dual Audio] 1080p BluRay"),
            "Inception"
        );
        assert_eq!(
            get_searchable_title("Breaking Bad Season 2 Episode 5"),
            "Breaking Bad"
        );
    }
}
