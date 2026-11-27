use iptvmine_core::DnsMode;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use thiserror::Error;

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CHROME_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Debug, Error)]
pub enum NetError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid DoH URL: {0}")]
    InvalidDohUrl(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsTestResult {
    pub success: bool,
    pub latency_ms: i64,
}

#[derive(Debug, Deserialize)]
struct DohResponse {
    #[serde(rename = "Answer", default)]
    answer: Vec<DohAnswer>,
}

#[derive(Debug, Deserialize)]
struct DohAnswer {
    #[serde(default = "default_type")]
    r#type: u16,
    #[serde(default)]
    data: String,
}

fn default_type() -> u16 {
    1
}

fn is_bootstrap_or_ip(host: &str) -> bool {
    let lower = host.trim().to_ascii_lowercase();
    if lower.parse::<IpAddr>().is_ok() {
        return true;
    }
    matches!(
        lower.as_str(),
        "1.1.1.1"
            | "1.0.0.1"
            | "cloudflare-dns.com"
            | "one.one.one.one"
            | "8.8.8.8"
            | "8.8.4.4"
            | "dns.google"
            | "94.140.14.14"
            | "94.140.15.15"
            | "dns.adguard-dns.com"
            | "localhost"
    )
}

fn bootstrap_ips(host: &str) -> Option<Vec<IpAddr>> {
    match host.to_ascii_lowercase().as_str() {
        "cloudflare-dns.com" | "one.one.one.one" => Some(vec![
            "104.16.249.249".parse().unwrap(),
            "104.16.248.249".parse().unwrap(),
            "1.1.1.1".parse().unwrap(),
            "1.0.0.1".parse().unwrap(),
        ]),
        "dns.google" => Some(vec![
            "8.8.8.8".parse().unwrap(),
            "8.8.4.4".parse().unwrap(),
        ]),
        "dns.adguard-dns.com" => Some(vec![
            "94.140.14.14".parse().unwrap(),
            "94.140.15.15".parse().unwrap(),
        ]),
        _ => None,
    }
}

/// Bootstrap DNS resolver pinning well-known DoH endpoints to avoid ISP DNS blocking / loops.
struct BootstrapDns;

impl Resolve for BootstrapDns {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            if let Some(ips) = bootstrap_ips(&host) {
                let addrs: Vec<SocketAddr> = ips.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect();
                let iter: Addrs = Box::new(addrs.into_iter());
                return Ok(iter);
            }
            let resolved = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let iter: Addrs = Box::new(resolved.collect::<Vec<_>>().into_iter());
            Ok(iter)
        })
    }
}

#[derive(Clone)]
pub struct DohState {
    mode: Arc<RwLock<DnsMode>>,
    custom_url: Arc<RwLock<String>>,
    cache: Arc<RwLock<HashMap<String, (Vec<IpAddr>, Instant)>>>,
    bootstrap_client: Client,
}

impl DohState {
    pub fn new(mode: DnsMode, custom_url: String) -> Self {
        let bootstrap_client = Client::builder()
            .dns_resolver(Arc::new(BootstrapDns))
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .user_agent("IPTVMineDesktop-DoH/1.0")
            .build()
            .expect("bootstrap DoH client");

        Self {
            mode: Arc::new(RwLock::new(mode)),
            custom_url: Arc::new(RwLock::new(custom_url)),
            cache: Arc::new(RwLock::new(HashMap::new())),
            bootstrap_client,
        }
    }

    pub fn update(&self, mode: DnsMode, custom_url: String) {
        if let Ok(mut m) = self.mode.write() {
            *m = mode;
        }
        if let Ok(mut u) = self.custom_url.write() {
            *u = custom_url.trim().to_string();
        }
        if let Ok(mut c) = self.cache.write() {
            c.clear();
        }
    }

    pub fn build_http_client(&self) -> Client {
        let resolver = Arc::new(AppDohResolver {
            state: self.clone(),
        });
        Client::builder()
            .dns_resolver(resolver)
            .cookie_store(true)
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(45))
            .user_agent(CHROME_USER_AGENT)
            .build()
            .expect("app http client")
    }

    pub async fn test_dns(&self, mode: DnsMode, custom_url: Option<&str>) -> DnsTestResult {
        if mode == DnsMode::Off {
            let start = Instant::now();
            let ok = tokio::net::lookup_host(("google.com", 443)).await.is_ok();
            return DnsTestResult {
                success: ok,
                latency_ms: if ok { start.elapsed().as_millis() as i64 } else { 0 },
            };
        }

        let target = custom_url.unwrap_or("").trim();
        if mode == DnsMode::Custom && target.is_empty() {
            return DnsTestResult {
                success: false,
                latency_ms: -1,
            };
        }

        let start = Instant::now();
        for host in ["google.com", "cloudflare.com", "wikipedia.org"] {
            if !self.perform_doh_lookup(mode, host, target).await.is_empty() {
                return DnsTestResult {
                    success: true,
                    latency_ms: start.elapsed().as_millis() as i64,
                };
            }
        }
        DnsTestResult {
            success: false,
            latency_ms: 0,
        }
    }

    async fn perform_doh_lookup(&self, mode: DnsMode, hostname: &str, custom_url: &str) -> Vec<IpAddr> {
        let candidates: Vec<String> = match mode {
            DnsMode::Cloudflare => vec![
                format!("https://cloudflare-dns.com/dns-query?name={hostname}&type=A"),
                format!("https://one.one.one.one/dns-query?name={hostname}&type=A"),
                format!("https://1.1.1.1/dns-query?name={hostname}&type=A"),
                format!("https://1.0.0.1/dns-query?name={hostname}&type=A"),
            ],
            DnsMode::Google => vec![
                format!("https://dns.google/resolve?name={hostname}&type=A"),
                format!("https://8.8.8.8/resolve?name={hostname}&type=A"),
                format!("https://8.8.4.4/resolve?name={hostname}&type=A"),
            ],
            DnsMode::Adguard => vec![
                format!("https://dns.adguard-dns.com/resolve?name={hostname}&type=A"),
                format!("https://94.140.14.14/resolve?name={hostname}&type=A"),
            ],
            DnsMode::Custom => {
                let base = custom_url.trim();
                if base.is_empty() || !base.to_ascii_lowercase().starts_with("https://") {
                    return Vec::new();
                }
                let sep = if base.contains('?') { '&' } else { '?' };
                vec![format!("{base}{sep}name={hostname}&type=A")]
            }
            DnsMode::Off => return Vec::new(),
        };

        for url in candidates {
            let Ok(resp) = self
                .bootstrap_client
                .get(&url)
                .header("Accept", "application/dns-json")
                .send()
                .await
            else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(parsed) = resp.json::<DohResponse>().await else {
                continue;
            };
            let ips: Vec<IpAddr> = parsed
                .answer
                .into_iter()
                .filter(|a| a.r#type == 1 || a.r#type == 28)
                .filter_map(|a| a.data.trim().parse::<IpAddr>().ok())
                .collect();
            if !ips.is_empty() {
                return ips;
            }
        }
        Vec::new()
    }
}

struct AppDohResolver {
    state: DohState,
}

impl Resolve for AppDohResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let state = self.state.clone();
        Box::pin(async move {
            let mode = state.mode.read().map(|m| *m).unwrap_or(DnsMode::Cloudflare);
            if mode == DnsMode::Off || is_bootstrap_or_ip(&host) {
                let resolved = tokio::net::lookup_host((host.as_str(), 0)).await?;
                let iter: Addrs = Box::new(resolved.collect::<Vec<_>>().into_iter());
                return Ok(iter);
            }

            if let Ok(cache) = state.cache.read() {
                if let Some((ips, at)) = cache.get(&host) {
                    if at.elapsed() < CACHE_TTL {
                        let addrs: Vec<SocketAddr> =
                            ips.iter().map(|ip| SocketAddr::new(*ip, 0)).collect();
                        let iter: Addrs = Box::new(addrs.into_iter());
                        return Ok(iter);
                    }
                }
            }

            let custom = state
                .custom_url
                .read()
                .map(|u| u.clone())
                .unwrap_or_default();
            let resolved_ips = state.perform_doh_lookup(mode, &host, &custom).await;
            if !resolved_ips.is_empty() {
                if let Ok(mut cache) = state.cache.write() {
                    cache.insert(host, (resolved_ips.clone(), Instant::now()));
                }
                let addrs: Vec<SocketAddr> = resolved_ips
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .collect();
                let iter: Addrs = Box::new(addrs.into_iter());
                return Ok(iter);
            }

            let fallback = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let iter: Addrs = Box::new(fallback.collect::<Vec<_>>().into_iter());
            Ok(iter)
        })
    }
}

/// Converts a raw.githubusercontent.com repo root URL to its jsDelivr CDN mirror.
pub fn to_jsdelivr_mirror(raw_github_url: &str) -> Option<String> {
    let trimmed = raw_github_url.trim().trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("https://raw.githubusercontent.com/")
        .or_else(|| trimmed.strip_prefix("http://raw.githubusercontent.com/"))?;
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() == 3 {
        Some(format!(
            "https://cdn.jsdelivr.net/gh/{}/{}@{}",
            parts[0], parts[1], parts[2]
        ))
    } else if parts.len() == 5 && parts[2] == "refs" && parts[3] == "heads" {
        Some(format!(
            "https://cdn.jsdelivr.net/gh/{}/{}@{}",
            parts[0], parts[1], parts[4]
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsdelivr_mirror_converts_raw_github_urls() {
        assert_eq!(
            to_jsdelivr_mirror("https://raw.githubusercontent.com/samyak/vega-providers/main/"),
            Some("https://cdn.jsdelivr.net/gh/samyak/vega-providers@main".into())
        );
        assert_eq!(
            to_jsdelivr_mirror(
                "https://raw.githubusercontent.com/samyak/vega-providers/refs/heads/main"
            ),
            Some("https://cdn.jsdelivr.net/gh/samyak/vega-providers@main".into())
        );
        assert_eq!(to_jsdelivr_mirror("https://example.com/foo"), None);
    }

    #[tokio::test]
    async fn custom_empty_doh_returns_minus_one() {
        let state = DohState::new(DnsMode::Custom, String::new());
        let res = state.test_dns(DnsMode::Custom, Some("")).await;
        assert!(!res.success);
        assert_eq!(res.latency_ms, -1);
    }
}
