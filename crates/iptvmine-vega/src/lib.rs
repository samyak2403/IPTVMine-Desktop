pub mod cheerio;
pub mod ssrf;

use crate::cheerio::{HtmlDomStore, SharedDomStore, CHEERIO_JS_SHIM};
use crate::ssrf::validate_scraper_url;
use iptvmine_core::{
    clean_title, is_playable_media_url, prioritize_streams, DetailEnrichment, EpisodeEnrichment,
    ProviderType, VegaCatalog, VegaDirectLink, VegaLink, VegaMeta, VegaPost, VegaProvider,
    VegaStream,
};
use iptvmine_m3u::{classify_payload, ScraperEndpoint, SourceKind};
use iptvmine_net::to_jsdelivr_mirror;
use iptvmine_storage::Storage;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{redirect::Policy, Client, Method};
use rquickjs::{function::Func, CaughtError, Context, Ctx, Promise, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use thiserror::Error;
use tokio::runtime::Handle;

const CHROME_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const MAX_JS_HEAP_BYTES: usize = 64 * 1024 * 1024;
const MAX_JS_STACK_BYTES: usize = 1024 * 1024;
const MAX_FETCH_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Error)]
pub enum VegaError {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("storage error: {0}")]
    Storage(#[from] iptvmine_storage::StorageError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("js runtime error: {0}")]
    Js(String),
    #[error("execution timed out or cancelled")]
    Timeout,
    #[error("invalid repository or provider: {0}")]
    InvalidInput(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResponse {
    pub catalog: Vec<VegaCatalog>,
    pub genres: Vec<VegaCatalog>,
}

#[derive(Clone)]
pub struct VegaRunner {
    storage: Arc<Storage>,
    http_client: Client,
    no_redirect_client: Client,
    base_urls: Arc<RwLock<HashMap<String, String>>>,
    base_urls_fetched: Arc<AtomicBool>,
    module_cache: Arc<Mutex<HashMap<String, HashMap<String, String>>>>,
    enrichment_cache: Arc<RwLock<HashMap<String, DetailEnrichment>>>,
    developer_test_mode: Arc<AtomicBool>,
}

impl VegaRunner {
    pub fn new(storage: Arc<Storage>, http_client: Client, developer_test_mode: bool) -> Self {
        let no_redirect_client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(45))
            .user_agent(CHROME_USER_AGENT)
            .build()
            .expect("no_redirect_client");

        Self {
            storage,
            http_client,
            no_redirect_client,
            base_urls: Arc::new(RwLock::new(HashMap::new())),
            base_urls_fetched: Arc::new(AtomicBool::new(false)),
            module_cache: Arc::new(Mutex::new(HashMap::new())),
            enrichment_cache: Arc::new(RwLock::new(HashMap::new())),
            developer_test_mode: Arc::new(AtomicBool::new(developer_test_mode)),
        }
    }

    pub fn set_developer_test_mode(&self, enabled: bool) {
        self.developer_test_mode.store(enabled, Ordering::Relaxed);
    }

    pub fn clear_module_cache(&self) {
        self.base_urls_fetched.store(false, Ordering::Relaxed);
        if let Ok(mut g) = self.module_cache.lock() {
            g.clear();
        }
        if let Ok(mut e) = self.enrichment_cache.write() {
            e.clear();
        }
    }

    pub fn resolve_repo_url(raw: &str) -> String {
        let trimmed = raw.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return String::new();
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            if lower.starts_with("https://github.com/") || lower.starts_with("http://github.com/") {
                let without_scheme = trimmed
                    .strip_prefix("https://github.com/")
                    .or_else(|| trimmed.strip_prefix("http://github.com/"))
                    .unwrap_or("");
                let parts: Vec<&str> = without_scheme.split('/').filter(|s| !s.is_empty()).collect();
                if parts.len() >= 2 {
                    let owner = parts[0];
                    let repo = parts[1].trim_end_matches(".git");
                    let branch = if parts.len() >= 4 && parts[2] == "tree" {
                        parts[3]
                    } else {
                        "main"
                    };
                    return format!("https://raw.githubusercontent.com/{owner}/{repo}/refs/heads/{branch}");
                }
            }
            return trimmed.to_string();
        }

        let handle = trimmed.trim_start_matches('@');
        if let Some((owner, rest)) = handle.split_once('/') {
            let (repo, branch) = rest.split_once('@').unwrap_or((rest, "main"));
            if !owner.is_empty() && !repo.is_empty() {
                return format!("https://raw.githubusercontent.com/{owner}/{repo}/refs/heads/{branch}");
            }
        }
        if handle.is_empty() {
            String::new()
        } else {
            format!("https://github.com/{handle}")
        }
    }

    fn parse_github_coords(url_str: &str) -> Option<(String, String, String)> {
        let trimmed = url_str.trim().trim_end_matches('/');
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("https://raw.githubusercontent.com/")
            .and_then(|_| trimmed.strip_prefix("https://raw.githubusercontent.com/"))
        {
            let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
            if parts.len() >= 3 {
                let owner = parts[0].to_string();
                let repo = parts[1].to_string();
                let branch = if parts.len() >= 4 && parts[2] == "refs" && parts.get(3) == Some(&"heads") {
                    parts.get(4).copied().unwrap_or("main").to_string()
                } else {
                    parts[2].to_string()
                };
                return Some((owner, repo, branch));
            }
        }
        None
    }

    async fn discover_owner_repo_roots(&self, owner: &str) -> Vec<String> {
        let clean = owner.trim().trim_start_matches('@');
        if clean.is_empty() {
            return Vec::new();
        }
        let api_url = format!(
            "https://api.github.com/users/{clean}/repos?per_page=10&sort=updated"
        );
        let Ok(body) = self.fetch_text(&api_url).await else {
            return Vec::new();
        };
        let Ok(Value::Array(repos)) = serde_json::from_str::<Value>(&body) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for item in repos {
            let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let branch = item
                .get("default_branch")
                .and_then(|b| b.as_str())
                .unwrap_or("main");
            out.push(format!(
                "https://raw.githubusercontent.com/{clean}/{name}/refs/heads/{branch}"
            ));
        }
        out
    }

    fn build_repo_urls(repo_url: &str) -> Vec<String> {
        let resolved = Self::resolve_repo_url(repo_url);
        if resolved.is_empty() {
            return Vec::new();
        }
        let lower = resolved.to_ascii_lowercase();
        let clean_root = if lower.split('?').next().unwrap_or(&lower).ends_with(".json") {
            resolved
                .rsplit_once('/')
                .map(|(parent, _)| parent.trim_end_matches('/').to_string())
                .unwrap_or(resolved)
        } else {
            resolved
        };
        let mut out = vec![clean_root.clone()];
        if let Some(mirror) = to_jsdelivr_mirror(&clean_root) {
            out.push(mirror);
        }
        out
    }

    fn first_active_vega_repo_url(&self) -> String {
        let Ok(providers) = self.storage.list_providers() else {
            return String::new();
        };
        let active: Vec<_> = providers
            .into_iter()
            .filter(|p| p.is_active && p.r#type == ProviderType::Vega)
            .collect();
        active
            .iter()
            .find(|p| !p.url.to_ascii_lowercase().ends_with(".json"))
            .or_else(|| active.first())
            .map(|p| p.url.trim().to_string())
            .unwrap_or_default()
    }

    fn record_scraper_endpoints(&self, entries: &HashMap<String, ScraperEndpoint>) {
        if let Ok(mut map) = self.base_urls.write() {
            for (k, ep) in entries {
                if !ep.url.trim().is_empty() {
                    map.insert(k.clone(), ep.url.clone());
                    if !ep.name.trim().is_empty() {
                        map.entry(ep.name.clone()).or_insert_with(|| ep.url.clone());
                    }
                }
            }
        }
    }

    async fn ensure_base_urls_fetched(&self, resolved_repo: &str) {
        if self.base_urls_fetched.load(Ordering::Relaxed) {
            return;
        }
        let mut sources = Vec::new();
        for root in Self::build_repo_urls(resolved_repo) {
            if let Some((owner, repo, branch)) = Self::parse_github_coords(&root) {
                let contents_url = format!(
                    "https://api.github.com/repos/{owner}/{repo}/contents?ref={branch}"
                );
                if let Ok(body) = self.fetch_text(&contents_url).await {
                    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&body) {
                        for item in items {
                            let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let dl_url = item
                                .get("download_url")
                                .and_then(|d| d.as_str())
                                .unwrap_or("");
                            if name.to_ascii_lowercase().ends_with(".json")
                                && !name.eq_ignore_ascii_case("manifest.json")
                                && !dl_url.is_empty()
                            {
                                sources.push(dl_url.to_string());
                            }
                        }
                    }
                }
            }
        }
        if let Ok(providers) = self.storage.list_providers() {
            for p in providers {
                if p.is_active && p.r#type == ProviderType::Vega {
                    let u = p.url.trim();
                    if u.to_ascii_lowercase().ends_with(".json") && !u.to_ascii_lowercase().ends_with("/manifest.json") {
                        sources.push(u.to_string());
                    }
                }
            }
        }

        let mut any_loaded = false;
        for src in sources {
            if let Ok(body) = self.fetch_text(&src).await {
                if let Some(det) = classify_payload(&body) {
                    if det.source_kind == SourceKind::VodScraperMap && !det.scraper_entries.is_empty() {
                        self.record_scraper_endpoints(&det.scraper_entries);
                        any_loaded = true;
                        break;
                    }
                }
            }
        }
        if any_loaded {
            self.base_urls_fetched.store(true, Ordering::Relaxed);
        }
    }

    pub async fn fetch_manifest(&self, repo_url: &str) -> Result<Vec<VegaProvider>, VegaError> {
        let trimmed = repo_url.trim();
        let lower_path = trimmed
            .to_ascii_lowercase()
            .split('?')
            .next()
            .unwrap_or("")
            .to_string();

        if lower_path.ends_with(".json") && !lower_path.ends_with("/manifest.json") {
            let mut direct = vec![trimmed.to_string()];
            if let Some(m) = to_jsdelivr_mirror(trimmed) {
                direct.push(m);
            }
            for url in direct {
                if let Ok(text) = self.fetch_text(&url).await {
                    if let Some(det) = classify_payload(&text) {
                        if det.source_kind == SourceKind::VodManifest && !det.manifest_entries.is_empty() {
                            return Ok(det.manifest_entries);
                        }
                        if det.source_kind == SourceKind::VodScraperMap && !det.scraper_entries.is_empty() {
                            self.record_scraper_endpoints(&det.scraper_entries);
                            return Ok(self.map_scraper_entries(&url, &det.scraper_entries).await);
                        }
                    }
                }
            }
        }

        let mut repos = Vec::new();
        let resolved = Self::resolve_repo_url(trimmed);
        if let Some(owner) = resolved
            .strip_prefix("https://github.com/")
            .filter(|s| !s.contains('/') && !s.is_empty())
        {
            for root in self.discover_owner_repo_roots(owner).await {
                repos.push(root.clone());
                if let Some(m) = to_jsdelivr_mirror(&root) {
                    repos.push(m);
                }
            }
        }
        repos.extend(Self::build_repo_urls(trimmed));
        self.fetch_manifest_from_roots(&repos).await
    }

    async fn map_scraper_entries(
        &self,
        source_url: &str,
        entries: &HashMap<String, ScraperEndpoint>,
    ) -> Vec<VegaProvider> {
        let mut candidate_roots = Self::build_repo_urls(source_url);
        if let Ok(providers) = self.storage.list_providers() {
            for p in providers {
                if p.is_active
                    && p.r#type == ProviderType::Vega
                    && !p.url.to_ascii_lowercase().ends_with(".json")
                {
                    candidate_roots.extend(Self::build_repo_urls(&p.url));
                }
            }
        }
        let base_manifest = Box::pin(self.fetch_manifest_from_roots(&candidate_roots))
            .await
            .unwrap_or_default();

        entries
            .iter()
            .map(|(key, ep)| {
                if let Some(existing) = base_manifest.iter().find(|m| {
                    m.value.eq_ignore_ascii_case(key)
                        || m.display_name.eq_ignore_ascii_case(&ep.name)
                        || m.display_name.eq_ignore_ascii_case(key)
                }) {
                    let mut cloned = existing.clone();
                    if !ep.name.trim().is_empty() {
                        cloned.display_name = ep.name.clone();
                    }
                    cloned
                } else {
                    VegaProvider {
                        display_name: if ep.name.trim().is_empty() {
                            key.clone()
                        } else {
                            ep.name.clone()
                        },
                        value: key.clone(),
                        version: "1.0".into(),
                        icon: String::new(),
                        r#type: "global".into(),
                        disabled: false,
                        is_installed: true,
                    }
                }
            })
            .collect()
    }

    async fn fetch_manifest_from_roots(
        &self,
        repos: &[String],
    ) -> Result<Vec<VegaProvider>, VegaError> {
        let mut last_err = None;
        for repo in repos {
            let url = if repo.to_ascii_lowercase().ends_with(".json") {
                repo.clone()
            } else {
                format!("{}/manifest.json", repo.trim_end_matches('/'))
            };
            match self.fetch_text(&url).await {
                Ok(text) => {
                    if let Some(det) = classify_payload(&text) {
                        if det.source_kind == SourceKind::VodScraperMap && !det.scraper_entries.is_empty() {
                            self.record_scraper_endpoints(&det.scraper_entries);
                            return Ok(self.map_scraper_entries(&url, &det.scraper_entries).await);
                        }
                        if !det.manifest_entries.is_empty() {
                            let resolved: Vec<VegaProvider> = det
                                .manifest_entries
                                .into_iter()
                                .map(|mut item| {
                                    item.icon = resolve_icon_url(repo, &item.icon);
                                    item
                                })
                                .collect();
                            return Ok(resolved);
                        }
                    }
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| VegaError::InvalidInput("Failed to fetch manifest".into())))
    }

    async fn fetch_text(&self, url: &str) -> Result<String, VegaError> {
        let dev_mode = self.developer_test_mode.load(Ordering::Relaxed);
        validate_scraper_url(url, dev_mode).map_err(VegaError::InvalidInput)?;
        let resp = self.http_client.get(url).send().await?.error_for_status()?;
        let bytes = resp.bytes().await?;
        if bytes.len() > MAX_FETCH_BYTES {
            return Err(VegaError::InvalidInput("Response exceeded 16 MB".into()));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn load_scraper_modules(
        &self,
        repo_url: &str,
        provider_value: &str,
    ) -> Result<HashMap<String, String>, VegaError> {
        let resolved_repo = {
            let r = Self::resolve_repo_url(repo_url);
            if r.is_empty() {
                Self::resolve_repo_url(&self.first_active_vega_repo_url())
            } else {
                r
            }
        };
        if resolved_repo.is_empty() {
            return Err(VegaError::InvalidInput(
                "No active Vega repository configured".into(),
            ));
        }

        self.ensure_base_urls_fetched(&resolved_repo).await;

        let cache_key = format!("{resolved_repo}::{provider_value}");
        if let Ok(guard) = self.module_cache.lock() {
            if let Some(hit) = guard.get(&cache_key) {
                return Ok(hit.clone());
            }
        }

        let repos = Self::build_repo_urls(&resolved_repo);
        let mut loaded = HashMap::new();
        for module in ["catalog", "posts", "meta", "stream", "episodes"] {
            for repo in &repos {
                let dist_url = format!("{repo}/dist/{provider_value}/{module}.js");
                if let Ok(code) = self.fetch_text(&dist_url).await {
                    if !code.trim().is_empty() {
                        loaded.insert(module.to_string(), code);
                        break;
                    }
                }
                let ts_url = format!("{repo}/providers/{provider_value}/{module}.ts");
                if let Ok(code) = self.fetch_text(&ts_url).await {
                    if !code.trim().is_empty() {
                        loaded.insert(module.to_string(), clean_typescript(&code));
                        break;
                    }
                }
            }
        }

        if loaded.is_empty() {
            return Err(VegaError::InvalidInput(format!(
                "No modules found for extension '{provider_value}'"
            )));
        }

        if let Ok(mut guard) = self.module_cache.lock() {
            guard.insert(cache_key, loaded.clone());
        }
        Ok(loaded)
    }

    pub async fn get_catalog(
        &self,
        repo_url: &str,
        provider_value: &str,
    ) -> Result<CatalogResponse, VegaError> {
        let modules = self.load_scraper_modules(repo_url, provider_value).await?;
        let invocation = r#"
            const mod = __vegaModules['catalog'] || {};
            let catalogArr = [];
            let genresArr = [];
            if (typeof mod === 'function') catalogArr = await mod();
            else if (mod.catalog) catalogArr = typeof mod.catalog === 'function' ? await mod.catalog() : mod.catalog;
            else if (mod.default) {
                if (typeof mod.default === 'function') catalogArr = await mod.default();
                else if (mod.default.catalog) catalogArr = typeof mod.default.catalog === 'function' ? await mod.default.catalog() : mod.default.catalog;
                else if (Array.isArray(mod.default)) catalogArr = mod.default;
            } else if (Array.isArray(mod)) catalogArr = mod;

            if (mod.genres) genresArr = typeof mod.genres === 'function' ? await mod.genres() : mod.genres;
            else if (mod.default && mod.default.genres) genresArr = typeof mod.default.genres === 'function' ? await mod.default.genres() : mod.default.genres;
            return JSON.stringify({ catalog: catalogArr || [], genres: genresArr || [] });
        "#;

        let json_str = self
            .eval_modules(repo_url, provider_value, &modules, invocation)
            .await?;
        let val: Value = serde_json::from_str(&json_str).unwrap_or_else(|_| json!({}));
        let catalog = val
            .get("catalog")
            .and_then(|v| serde_json::from_value::<Vec<VegaCatalog>>(v.clone()).ok())
            .unwrap_or_default();
        let genres = val
            .get("genres")
            .and_then(|v| serde_json::from_value::<Vec<VegaCatalog>>(v.clone()).ok())
            .unwrap_or_default();
        Ok(CatalogResponse { catalog, genres })
    }

    pub async fn get_posts(
        &self,
        repo_url: &str,
        provider_value: &str,
        filter: &str,
        page: i32,
    ) -> Result<Vec<VegaPost>, VegaError> {
        let modules = self.load_scraper_modules(repo_url, provider_value).await?;
        let filter_json = serde_json::to_string(filter)?;
        let pv_json = serde_json::to_string(provider_value)?;
        let invocation = format!(
            r#"
            const mod = __vegaModules['posts'];
            if (!mod) return '[]';
            let fn = typeof mod === 'function' ? mod
              : (typeof mod.getPosts === 'function' ? mod.getPosts
              : (mod.default && typeof mod.default === 'function' ? mod.default
              : (mod.default && typeof mod.default.getPosts === 'function' ? mod.default.getPosts : null)));
            if (!fn) return '[]';
            let resolvedFilter = {filter_json};
            if (!resolvedFilter) {{
              const catMod = __vegaModules['catalog'] || {{}};
              const catList = catMod.catalog || (catMod.default && catMod.default.catalog) || [];
              if (Array.isArray(catList) && catList[0] && catList[0].filter) resolvedFilter = catList[0].filter;
            }}
            const res = await fn({{
              filter: resolvedFilter,
              page: {page},
              provider: {pv_json},
              providerValue: {pv_json},
              signal: null,
              providerContext: globalThis.providerContext
            }});
            return JSON.stringify(res || []);
            "#
        );

        let json_str = self
            .eval_modules(repo_url, provider_value, &modules, &invocation)
            .await?;
        let mut posts: Vec<VegaPost> = serde_json::from_str(&json_str).unwrap_or_default();
        for p in &mut posts {
            p.title = clean_title(&p.title);
        }
        Ok(posts)
    }

    pub async fn get_search_posts(
        &self,
        repo_url: &str,
        provider_value: &str,
        query: &str,
        page: i32,
    ) -> Result<Vec<VegaPost>, VegaError> {
        let modules = self.load_scraper_modules(repo_url, provider_value).await?;
        let query_json = serde_json::to_string(query)?;
        let pv_json = serde_json::to_string(provider_value)?;
        let invocation = format!(
            r#"
            const mod = __vegaModules['posts'];
            if (!mod) return '[]';
            let fn = null;
            let isSearch = true;
            if (typeof mod.getSearchPosts === 'function') fn = mod.getSearchPosts;
            else if (mod.default && typeof mod.default.getSearchPosts === 'function') fn = mod.default.getSearchPosts;
            else if (typeof mod.getPosts === 'function') {{ fn = mod.getPosts; isSearch = false; }}
            else if (mod.default && typeof mod.default.getPosts === 'function') {{ fn = mod.default.getPosts; isSearch = false; }}
            else if (mod.default && typeof mod.default === 'function') fn = mod.default;
            if (!fn) return '[]';
            const q = {query_json};
            const args = isSearch
              ? {{ searchQuery: q, page: {page}, providerValue: {pv_json}, signal: null, providerContext: globalThis.providerContext }}
              : {{ filter: q, page: {page}, providerValue: {pv_json}, signal: null, providerContext: globalThis.providerContext }};
            const res = await fn(args);
            return JSON.stringify(res || []);
            "#
        );

        let json_str = self
            .eval_modules(repo_url, provider_value, &modules, &invocation)
            .await?;
        let mut posts: Vec<VegaPost> = serde_json::from_str(&json_str).unwrap_or_default();
        for p in &mut posts {
            p.title = clean_title(&p.title);
        }
        Ok(posts)
    }

    pub async fn get_meta(
        &self,
        repo_url: &str,
        provider_value: &str,
        link: &str,
    ) -> Result<VegaMeta, VegaError> {
        let modules = self.load_scraper_modules(repo_url, provider_value).await?;
        let link_json = serde_json::to_string(link)?;
        let pv_json = serde_json::to_string(provider_value)?;
        let invocation = format!(
            r#"
            const mod = __vegaModules['meta'];
            if (!mod) return '{{}}';
            let fn = typeof mod === 'function' ? mod
              : (typeof mod.getMeta === 'function' ? mod.getMeta
              : (mod.default && typeof mod.default === 'function' ? mod.default
              : (mod.default && typeof mod.default.getMeta === 'function' ? mod.default.getMeta : null)));
            if (!fn) return '{{}}';
            const res = await fn({{
              link: {link_json},
              provider: {pv_json},
              providerValue: {pv_json},
              providerContext: globalThis.providerContext
            }});
            return JSON.stringify(res || {{}});
            "#
        );

        let json_str = self
            .eval_modules(repo_url, provider_value, &modules, &invocation)
            .await?;
        let obj: Value = serde_json::from_str(&json_str).unwrap_or(Value::Null);
        let title = clean_title(
            pick_str_or_num(&obj, &["title", "name"])
                .as_deref()
                .unwrap_or(""),
        );
        let synopsis = pick_str_or_num(&obj, &["synopsis", "description", "plot", "overview"])
            .unwrap_or_default();
        let image = pick_str_or_num(&obj, &["image", "poster", "poster_path", "thumbnail"])
            .unwrap_or_default();
        let background = pick_str_or_num(&obj, &["background", "backdrop", "backdrop_path"]);
        let imdb_id = pick_str_or_num(&obj, &["imdbId", "imdb_id"]).unwrap_or_default();
        let r#type = pick_str_or_num(&obj, &["type"])
            .unwrap_or_else(|| "movie".to_string());
        let year = pick_str_or_num(&obj, &["year", "releaseInfo"]).or_else(|| {
            pick_str_or_num(&obj, &["released", "releaseDate", "date"])
                .map(|d| d.chars().take(4).collect())
                .filter(|y: &String| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()))
        });
        let rating = pick_str_or_num(&obj, &["imdbRating", "rating"]);
        let runtime = normalize_duration(
            pick_str_or_num(&obj, &["runtime", "duration"]).as_deref(),
        );
        let genres = parse_genres(&obj);
        let link_list = obj
            .get("linkList")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(|item| parse_vega_link_item(item, false)).collect())
            .unwrap_or_default();

        Ok(VegaMeta {
            title,
            synopsis,
            image,
            imdb_id,
            r#type,
            link_list,
            year,
            rating,
            runtime,
            genres,
            background,
        })
    }

    pub async fn get_episodes(
        &self,
        repo_url: &str,
        provider_value: &str,
        url: &str,
    ) -> Result<Vec<VegaLink>, VegaError> {
        let modules = self.load_scraper_modules(repo_url, provider_value).await?;
        let url_json = serde_json::to_string(url)?;
        let invocation = format!(
            r#"
            const mod = __vegaModules['episodes'];
            if (!mod) return '[]';
            let fn = typeof mod === 'function' ? mod
              : (typeof mod.getEpisodes === 'function' ? mod.getEpisodes
              : (mod.default && typeof mod.default === 'function' ? mod.default
              : (mod.default && typeof mod.default.getEpisodes === 'function' ? mod.default.getEpisodes : null)));
            if (!fn) return '[]';
            const res = await fn({{ url: {url_json}, providerContext: globalThis.providerContext }});
            return JSON.stringify(res || []);
            "#
        );

        let json_str = self
            .eval_modules(repo_url, provider_value, &modules, &invocation)
            .await?;
        let arr: Vec<Value> = serde_json::from_str(&json_str).unwrap_or_default();
        Ok(arr
            .iter()
            .map(|v| parse_vega_link_item(v, true))
            .collect())
    }

    pub async fn get_detail_enrichment(
        &self,
        imdb_id: &str,
        title: &str,
        media_type: &str,
    ) -> DetailEnrichment {
        let clean_imdb = imdb_id.trim();
        let normalized_imdb = if clean_imdb.starts_with("tt") {
            Some(clean_imdb.to_string())
        } else {
            None
        };
        let search_title = searchable_title(title);
        let kind = if media_type.eq_ignore_ascii_case("series")
            || media_type.eq_ignore_ascii_case("tv")
        {
            "series"
        } else {
            "movie"
        };

        let cache_key = format!(
            "{kind}:{}:{}",
            normalized_imdb.as_deref().unwrap_or(""),
            search_title.to_ascii_lowercase()
        );
        if let Ok(guard) = self.enrichment_cache.read() {
            if let Some(cached) = guard.get(&cache_key) {
                return cached.clone();
            }
        }

        let mut enrichment = DetailEnrichment::default();
        let resolved_imdb = match normalized_imdb {
            Some(id) => Some(id),
            None if !search_title.is_empty() => {
                self.search_cinemeta_imdb(&search_title, kind).await
            }
            None => None,
        };

        if kind == "series" && !search_title.is_empty() {
            let (cine, tvmaze) = tokio::join!(
                self.fetch_cinemeta_meta(resolved_imdb.as_deref(), kind),
                self.fetch_tvmaze_series(&search_title)
            );
            if let Some(c) = cine {
                enrichment = c;
            }
            if let Some(tm) = tvmaze {
                merge_enrichment(&mut enrichment, tm);
            }
        } else if let Some(c) = self.fetch_cinemeta_meta(resolved_imdb.as_deref(), kind).await {
            enrichment = c;
        }

        if enrichment.imdb_id.is_none() {
            enrichment.imdb_id = resolved_imdb;
        }

        if let Ok(mut guard) = self.enrichment_cache.write() {
            guard.insert(cache_key, enrichment.clone());
        }
        enrichment
    }

    async fn search_cinemeta_imdb(&self, title: &str, kind: &str) -> Option<String> {
        let encoded = urlencoding_component(title);
        let url = format!("https://v3-cinemeta.strem.io/catalog/{kind}/top/search={encoded}.json");
        let resp = self
            .http_client
            .get(url)
            .timeout(Duration::from_secs(7))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: Value = resp.json().await.ok()?;
        let metas = body.get("metas")?.as_array()?;
        for item in metas {
            if let Some(id) = pick_str_or_num(item, &["imdb_id", "id"]) {
                if id.starts_with("tt") {
                    return Some(id);
                }
            }
        }
        None
    }

    async fn fetch_cinemeta_meta(&self, imdb_id: Option<&str>, kind: &str) -> Option<DetailEnrichment> {
        let id = imdb_id?.trim();
        if !id.starts_with("tt") {
            return None;
        }
        let url = format!("https://v3-cinemeta.strem.io/meta/{kind}/{id}.json");
        let resp = self
            .http_client
            .get(url)
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let root: Value = resp.json().await.ok()?;
        let meta = root.get("meta")?;
        let show_runtime = normalize_duration(
            pick_str_or_num(meta, &["runtime"]).as_deref(),
        );
        let mut episodes = Vec::new();
        if let Some(videos) = meta.get("videos").and_then(|v| v.as_array()) {
            for v in videos {
                let season = v.get("season").and_then(|n| n.as_u64()).map(|n| n as u32);
                let episode = v
                    .get("episode")
                    .or_else(|| v.get("number"))
                    .and_then(|n| n.as_u64())
                    .map(|n| n as u32);
                let thumb = pick_str_or_num(v, &["thumbnail", "image"]);
                let name = pick_str_or_num(v, &["name", "title"]);
                let overview = pick_str_or_num(v, &["overview", "description", "synopsis"])
                    .map(|s| strip_html_tags(&s));
                let date = pick_str_or_num(v, &["firstAired", "released"])
                    .map(|d| d.chars().take(10).collect());
                let rating = pick_str_or_num(v, &["rating"]);
                let duration = normalize_duration(
                    pick_str_or_num(v, &["runtime", "duration"]).as_deref(),
                )
                .or_else(|| show_runtime.clone());
                if season.is_some() || episode.is_some() {
                    episodes.push(EpisodeEnrichment {
                        season,
                        episode,
                        title: name,
                        image: thumb,
                        overview,
                        duration,
                        date,
                        rating,
                    });
                }
            }
        }
        Some(DetailEnrichment {
            imdb_id: Some(id.to_string()),
            year: pick_str_or_num(meta, &["releaseInfo", "year"]),
            rating: pick_str_or_num(meta, &["imdbRating", "rating"]),
            runtime: show_runtime,
            genres: parse_genres(meta),
            background: pick_str_or_num(meta, &["background", "poster"]),
            episodes,
        })
    }

    async fn fetch_tvmaze_series(&self, title: &str) -> Option<DetailEnrichment> {
        let url = reqwest::Url::parse_with_params(
            "https://api.tvmaze.com/singlesearch/shows",
            &[("q", title), ("embed", "episodes")],
        )
        .ok()?;
        let resp = self
            .http_client
            .get(url)
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let root: Value = resp.json().await.ok()?;
        let show_runtime = normalize_duration(
            pick_str_or_num(&root, &["runtime", "averageRuntime"]).as_deref(),
        );
        let year = pick_str_or_num(&root, &["premiered"])
            .map(|d| d.chars().take(4).collect())
            .filter(|y: &String| !y.is_empty());
        let rating = root
            .get("rating")
            .and_then(|r| pick_str_or_num(r, &["average"]));
        let genres = parse_genres(&root);
        let background = root
            .get("image")
            .and_then(|img| pick_str_or_num(img, &["original", "medium"]));
        let imdb_id = root
            .get("externals")
            .and_then(|e| pick_str_or_num(e, &["imdb"]));

        let mut episodes = Vec::new();
        if let Some(ep_arr) = root
            .get("_embedded")
            .and_then(|e| e.get("episodes"))
            .and_then(|e| e.as_array())
        {
            for ep in ep_arr {
                let season = ep.get("season").and_then(|n| n.as_u64()).map(|n| n as u32);
                let episode = ep.get("number").and_then(|n| n.as_u64()).map(|n| n as u32);
                let image = ep
                    .get("image")
                    .and_then(|img| pick_str_or_num(img, &["medium", "original"]));
                let duration = normalize_duration(
                    pick_str_or_num(ep, &["runtime"]).as_deref(),
                )
                .or_else(|| show_runtime.clone());
                let rating = ep
                    .get("rating")
                    .and_then(|r| pick_str_or_num(r, &["average"]));
                let overview = pick_str_or_num(ep, &["summary"])
                    .map(|s| strip_html_tags(&s))
                    .filter(|s| !s.is_empty());
                episodes.push(EpisodeEnrichment {
                    season,
                    episode,
                    title: pick_str_or_num(ep, &["name", "title"]),
                    image,
                    overview,
                    duration,
                    date: pick_str_or_num(ep, &["airdate"]),
                    rating,
                });
            }
        }

        Some(DetailEnrichment {
            imdb_id,
            year,
            rating,
            runtime: show_runtime,
            genres,
            background,
            episodes,
        })
    }

    pub async fn get_stream(
        &self,
        repo_url: &str,
        provider_value: &str,
        link: &str,
        stream_type: &str,
    ) -> Result<Vec<VegaStream>, VegaError> {
        let trimmed = link.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.ends_with(".m3u8")
            || lower.ends_with(".mp4")
            || lower.ends_with(".mkv")
            || lower.ends_with(".ts")
            || lower.ends_with(".mpd")
            || lower.contains(".m3u8?")
            || lower.contains(".mp4?")
            || lower.contains(".mkv?")
        {
            return Ok(vec![VegaStream {
                server: "Direct Source".into(),
                link: trimmed.to_string(),
                r#type: "mp4".into(),
                quality: "HD".into(),
                ..Default::default()
            }]);
        }

        let modules = self.load_scraper_modules(repo_url, provider_value).await?;
        let link_json = serde_json::to_string(trimmed)?;
        let type_json = serde_json::to_string(stream_type)?;
        let invocation = format!(
            r#"
            const mod = __vegaModules['stream'];
            if (!mod) return '[]';
            let fn = typeof mod === 'function' ? mod
              : (typeof mod.getStream === 'function' ? mod.getStream
              : (mod.default && typeof mod.default === 'function' ? mod.default
              : (mod.default && typeof mod.default.getStream === 'function' ? mod.default.getStream : null)));
            if (!fn) {{
              for (const k of Object.keys(mod)) {{
                if (typeof mod[k] === 'function' && k !== 'default') {{ fn = mod[k]; break; }}
              }}
            }}
            if (!fn) return '[]';
            const res = await fn({{
              link: {link_json},
              type: {type_json},
              signal: null,
              providerContext: globalThis.providerContext
            }});
            const arr = Array.isArray(res) ? res : (res ? [res] : []);
            return JSON.stringify(arr);
            "#
        );

        let json_str = self
            .eval_modules(repo_url, provider_value, &modules, &invocation)
            .await?;
        let raw_arr: Vec<Value> = serde_json::from_str(&json_str).unwrap_or_default();
        let mut streams: Vec<VegaStream> = raw_arr
            .iter()
            .filter_map(parse_vega_stream_item)
            .collect();
        streams.retain(|s| !s.link.trim().is_empty());

        if streams.is_empty() && is_playable_media_url(trimmed) {
            streams.push(VegaStream {
                server: "Direct Source".into(),
                link: trimmed.to_string(),
                r#type: "mp4".into(),
                quality: "HD".into(),
                ..Default::default()
            });
        }

        Ok(prioritize_streams(streams))
    }

    /// Evaluates CommonJS Vega modules + async invocation inside an isolated QuickJS runtime
    /// with 64 MB heap cap and immediate interruptible cancellation.
    pub async fn eval_modules(
        &self,
        repo_url: &str,
        provider_value: &str,
        modules: &HashMap<String, String>,
        async_body: &str,
    ) -> Result<String, VegaError> {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        struct CancelGuard(Arc<AtomicBool>);
        impl Drop for CancelGuard {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let _guard = CancelGuard(cancel_flag.clone());

        let handle = Handle::current();
        let storage = self.storage.clone();
        let http_client = self.http_client.clone();
        let no_redirect_client = self.no_redirect_client.clone();
        let base_urls = self.base_urls.clone();
        let dev_mode = self.developer_test_mode.clone();
        let repo_owned = repo_url.to_string();
        let pv_owned = provider_value.to_string();
        let modules_owned = modules.clone();
        let body_owned = async_body.to_string();

        let join = tokio::task::spawn_blocking(move || -> Result<String, VegaError> {
            let rt = Runtime::new().map_err(|e| VegaError::Js(e.to_string()))?;
            rt.set_memory_limit(MAX_JS_HEAP_BYTES);
            rt.set_max_stack_size(MAX_JS_STACK_BYTES);
            let flag_ref = cancel_flag.clone();
            rt.set_interrupt_handler(Some(Box::new(move || flag_ref.load(Ordering::Relaxed))));

            let ctx = Context::full(&rt).map_err(|e| VegaError::Js(e.to_string()))?;
            let dom_store: SharedDomStore = Arc::new(Mutex::new(HtmlDomStore::default()));

            ctx.with(|ctx| -> Result<String, VegaError> {
                let globals = ctx.globals();

                // DOM host functions
                let ds1 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomParse",
                        Func::from(move |html: String| -> usize {
                            ds1.lock().map(|mut s| s.insert(&html)).unwrap_or(0)
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let ds2 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomSelect",
                        Func::from(move |doc_id: usize, node_id: isize, css: String| -> String {
                            let scope = if node_id < 0 { None } else { Some(node_id as usize) };
                            let items = ds2
                                .lock()
                                .map(|s| s.select(doc_id, scope, &css))
                                .unwrap_or_default();
                            serde_json::to_string(&items).unwrap_or_else(|_| "[]".into())
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let ds3 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomTraverse",
                        Func::from(
                            move |doc_id: usize, node_ids_json: String, axis: String, css: String| -> String {
                                let ids: Vec<usize> = serde_json::from_str(&node_ids_json).unwrap_or_default();
                                let filter = if css.trim().is_empty() { None } else { Some(css.as_str()) };
                                let items = ds3
                                    .lock()
                                    .map(|s| s.traverse(doc_id, &ids, &axis, filter))
                                    .unwrap_or_default();
                                serde_json::to_string(&items).unwrap_or_else(|_| "[]".into())
                            },
                        ),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let ds4 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomFilter",
                        Func::from(
                            move |doc_id: usize, node_ids_json: String, css: String, negate: bool| -> String {
                                let ids: Vec<usize> = serde_json::from_str(&node_ids_json).unwrap_or_default();
                                let items = ds4
                                    .lock()
                                    .map(|s| s.filter_nodes(doc_id, &ids, &css, negate))
                                    .unwrap_or_default();
                                serde_json::to_string(&items).unwrap_or_else(|_| "[]".into())
                            },
                        ),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let ds5 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomRemove",
                        Func::from(move |doc_id: usize, node_ids_json: String| {
                            let ids: Vec<usize> = serde_json::from_str(&node_ids_json).unwrap_or_default();
                            if let Ok(mut s) = ds5.lock() {
                                s.remove(doc_id, &ids);
                            }
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let ds6 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomClone",
                        Func::from(move |doc_id: usize, node_ids_json: String| -> String {
                            let ids: Vec<usize> = serde_json::from_str(&node_ids_json).unwrap_or_default();
                            let (new_doc, nodes) = ds6
                                .lock()
                                .map(|mut s| s.clone_nodes(doc_id, &ids))
                                .unwrap_or((doc_id, Vec::new()));
                            json!({ "docId": new_doc, "nodes": nodes }).to_string()
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let ds7 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomTexts",
                        Func::from(move |doc_id: usize, node_ids_json: String| -> String {
                            let ids: Vec<usize> = serde_json::from_str(&node_ids_json).unwrap_or_default();
                            ds7.lock().map(|s| s.texts(doc_id, &ids)).unwrap_or_default()
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let ds8 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomHtml",
                        Func::from(move |doc_id: usize, node_id: usize| -> String {
                            ds8.lock()
                                .map(|s| s.inner_html(doc_id, node_id))
                                .unwrap_or_default()
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let ds9 = dom_store.clone();
                globals
                    .set(
                        "__vegaDomRootHtml",
                        Func::from(move |doc_id: usize| -> String {
                            ds9.lock().map(|s| s.root_html(doc_id)).unwrap_or_default()
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                // URL, random/UUID, and timer sleep host functions
                globals
                    .set(
                        "__vegaParseUrl",
                        Func::from(move |raw: String, base: String| -> String {
                            let parsed = if base.trim().is_empty() {
                                url::Url::parse(raw.trim())
                            } else {
                                url::Url::parse(base.trim()).and_then(|b| b.join(raw.trim()))
                            };
                            match parsed {
                                Ok(u) => {
                                    let hostname = u.host_str().unwrap_or("").to_string();
                                    let port = u.port().map(|p| p.to_string()).unwrap_or_default();
                                    let host = if port.is_empty() {
                                        hostname.clone()
                                    } else {
                                        format!("{hostname}:{port}")
                                    };
                                    let protocol = format!("{}:", u.scheme());
                                    let search = u.query().map(|q| format!("?{q}")).unwrap_or_default();
                                    let hash = u.fragment().map(|f| format!("#{f}")).unwrap_or_default();
                                    json!({
                                        "ok": true,
                                        "href": u.as_str(),
                                        "origin": u.origin().ascii_serialization(),
                                        "protocol": protocol,
                                        "host": host,
                                        "hostname": hostname,
                                        "port": port,
                                        "pathname": u.path(),
                                        "search": search,
                                        "hash": hash
                                    })
                                    .to_string()
                                }
                                Err(e) => json!({ "ok": false, "error": e.to_string() }).to_string(),
                            }
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                globals
                    .set(
                        "__vegaRandomBytes",
                        Func::from(move |len: usize| -> Vec<u8> {
                            let capped = len.min(65536);
                            let mut out = Vec::with_capacity(capped);
                            while out.len() < capped {
                                out.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
                            }
                            out.truncate(capped);
                            out
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                globals
                    .set(
                        "__vegaUuidV4",
                        Func::from(move || -> String { uuid::Uuid::new_v4().to_string() }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let sleep_cancel = cancel_flag.clone();
                globals
                    .set(
                        "__vegaSleepMs",
                        Func::from(move |ms: u64| {
                            if !sleep_cancel.load(Ordering::Relaxed) {
                                std::thread::sleep(Duration::from_millis(ms.min(50)));
                            }
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                // Scoped kvStore host functions
                let st1 = storage.clone();
                let r1 = repo_owned.clone();
                let p1 = pv_owned.clone();
                globals
                    .set(
                        "__vegaKvGet",
                        Func::from(move |k: String| -> String {
                            st1.ext_kv_get(&r1, &p1, &k)
                                .ok()
                                .flatten()
                                .unwrap_or_default()
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let st2 = storage.clone();
                let r2 = repo_owned.clone();
                let p2 = pv_owned.clone();
                globals
                    .set(
                        "__vegaKvSet",
                        Func::from(move |k: String, v: String| {
                            let _ = st2.ext_kv_set(&r2, &p2, &k, &v);
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let st3 = storage.clone();
                let r3 = repo_owned.clone();
                let p3 = pv_owned.clone();
                globals
                    .set(
                        "__vegaKvDel",
                        Func::from(move |k: String| {
                            let _ = st3.ext_kv_delete(&r3, &p3, &k);
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                let st4 = storage.clone();
                let r4 = repo_owned.clone();
                let p4 = pv_owned.clone();
                globals
                    .set(
                        "__vegaKvClear",
                        Func::from(move || {
                            let _ = st4.ext_kv_clear(&r4, &p4);
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                // getBaseUrl host function (auto-populated from discovered repository endpoints)
                let bu = base_urls.clone();
                globals
                    .set(
                        "__vegaGetBaseUrl",
                        Func::from(move |name: String| -> String {
                            let clean = name.trim();
                            bu.read()
                                .ok()
                                .and_then(|m| {
                                    m.get(clean).cloned().or_else(|| {
                                        m.iter()
                                            .find(|(k, _)| k.eq_ignore_ascii_case(clean))
                                            .map(|(_, v)| v.clone())
                                    })
                                })
                                .unwrap_or_default()
                        }),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                // HTTP fetch host function (with SSRF check and maxRedirects=0 support)
                let h_handle = handle.clone();
                let hc = http_client.clone();
                let nrc = no_redirect_client.clone();
                let dm = dev_mode.clone();
                globals
                    .set(
                        "__vegaHttpFetch",
                        Func::from(
                            move |url: String, method: String, headers_json: String, body: String| -> String {
                                let test_mode = dm.load(Ordering::Relaxed);
                                if let Err(msg) = validate_scraper_url(&url, test_mode) {
                                    return json!({ "error": msg, "status": 0, "body": "", "headers": {}, "url": url }).to_string();
                                }
                                let headers_map: HashMap<String, String> =
                                    serde_json::from_str(&headers_json).unwrap_or_default();
                                let no_redirects = headers_map
                                    .get("x-max-redirects")
                                    .or_else(|| headers_map.get("X-Max-Redirects"))
                                    .map(|v| v == "0")
                                    .unwrap_or(false);

                                let client_ref = if no_redirects { &nrc } else { &hc };
                                let m = Method::from_bytes(method.trim().to_ascii_uppercase().as_bytes())
                                    .unwrap_or(Method::GET);

                                let mut hdrs = HeaderMap::new();
                                for (k, v) in headers_map {
                                    if k.eq_ignore_ascii_case("x-max-redirects") {
                                        continue;
                                    }
                                    if let (Ok(hn), Ok(hv)) = (
                                        HeaderName::from_bytes(k.as_bytes()),
                                        HeaderValue::from_str(&v),
                                    ) {
                                        hdrs.insert(hn, hv);
                                    }
                                }

                                let res = h_handle.block_on(async {
                                    let mut req = client_ref.request(m, &url).headers(hdrs);
                                    if !body.is_empty() {
                                        req = req.body(body);
                                    }
                                    let resp = req.send().await?;
                                    let status = resp.status().as_u16();
                                    let final_url = resp.url().to_string();
                                    let mut resp_headers = HashMap::new();
                                    for (k, v) in resp.headers() {
                                        if let Ok(vs) = v.to_str() {
                                            resp_headers.insert(k.as_str().to_ascii_lowercase(), vs.to_string());
                                        }
                                    }
                                    let bytes = resp.bytes().await?;
                                    let sliced = &bytes[..bytes.len().min(MAX_FETCH_BYTES)];
                                    let body_str = String::from_utf8_lossy(sliced).into_owned();
                                    Ok::<_, reqwest::Error>(json!({
                                        "status": status,
                                        "body": body_str,
                                        "headers": resp_headers,
                                        "url": final_url
                                    }))
                                });

                                match res {
                                    Ok(v) => v.to_string(),
                                    Err(e) => json!({
                                        "error": e.to_string(),
                                        "status": 0,
                                        "body": "",
                                        "headers": {},
                                        "url": url
                                    })
                                    .to_string(),
                                }
                            },
                        ),
                    )
                    .map_err(|e| VegaError::Js(e.to_string()))?;

                // Inject cheerio shim and Web/providerContext polyfills
                ctx.eval::<(), _>(CHEERIO_JS_SHIM)
                    .map_err(|e| caught_js_error(&ctx, "cheerio shim", e))?;

                let pv_json = serde_json::to_string(&pv_owned).unwrap_or_else(|_| "\"\"".into());
                let ua_json = serde_json::to_string(CHROME_USER_AGENT).unwrap();
                let init_ctx_js = format!(
                    r#"
                    globalThis.__vegaModules = {{}};
                    globalThis.providerGlobal = globalThis;
                    if (typeof globalThis.process === 'undefined') {{
                        globalThis.process = {{ env: {{}} }};
                    }}
                    if (typeof globalThis.console === 'undefined') {{
                        globalThis.console = {{ log: () => {{}}, warn: () => {{}}, error: () => {{}}, info: () => {{}} }};
                    }}
                    if (typeof globalThis.queueMicrotask === 'undefined') {{
                        globalThis.queueMicrotask = (fn) => Promise.resolve().then(fn);
                    }}

                    (function() {{
                        let nextTimerId = 1;
                        const timers = new Map();
                        globalThis.setTimeout = function(fn, delay) {{
                            const id = nextTimerId++;
                            const args = Array.prototype.slice.call(arguments, 2);
                            const ms = Math.max(0, Number(delay) || 0);
                            timers.set(id, {{ fn, args, dueAt: Date.now() + ms, interval: 0 }});
                            return id;
                        }};
                        globalThis.clearTimeout = function(id) {{ timers.delete(id); }};
                        globalThis.setInterval = function(fn, delay) {{
                            const id = nextTimerId++;
                            const args = Array.prototype.slice.call(arguments, 2);
                            const ms = Math.max(10, Number(delay) || 10);
                            timers.set(id, {{ fn, args, dueAt: Date.now() + ms, interval: ms }});
                            return id;
                        }};
                        globalThis.clearInterval = function(id) {{ timers.delete(id); }};
                        globalThis.__vegaStepTimers = function() {{
                            if (timers.size === 0) return -1;
                            const now = Date.now();
                            let minWait = Infinity;
                            for (const [id, t] of Array.from(timers.entries())) {{
                                if (t.dueAt <= now) {{
                                    if (t.interval > 0) {{
                                        t.dueAt = now + t.interval;
                                    }} else {{
                                        timers.delete(id);
                                    }}
                                    if (typeof t.fn === 'function') t.fn.apply(null, t.args);
                                    minWait = 0;
                                }} else {{
                                    minWait = Math.min(minWait, t.dueAt - now);
                                }}
                            }}
                            if (timers.size === 0) return 0;
                            return minWait === Infinity ? 0 : minWait;
                        }};
                    }})();

                    if (typeof globalThis.btoa === 'undefined') {{
                        const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=';
                        globalThis.btoa = function(input) {{
                            let str = String(input);
                            let output = '';
                            for (let block = 0, charCode, i = 0, map = chars; str.charAt(i | 0) || (map = '=', i % 1); output += map.charAt(63 & block >> 8 - i % 1 * 8)) {{
                                charCode = str.charCodeAt(i += 3 / 4);
                                block = block << 8 | charCode;
                            }}
                            return output;
                        }};
                        globalThis.atob = function(input) {{
                            let str = String(input).replace(/=+$/, '');
                            let output = '';
                            for (let bc = 0, bs = 0, buffer, i = 0; buffer = str.charAt(i++); ~buffer && (bs = bc % 4 ? bs * 64 + buffer : buffer, bc++ % 4) ? output += String.fromCharCode(255 & bs >> (-2 * bc & 6)) : 0) {{
                                buffer = chars.indexOf(buffer);
                            }}
                            return output;
                        }};
                    }}

                    if (typeof globalThis.TextEncoder === 'undefined') {{
                        globalThis.TextEncoder = class TextEncoder {{
                            constructor() {{ this.encoding = 'utf-8'; }}
                            encode(input) {{
                                const str = input === undefined ? '' : String(input);
                                const utf8 = unescape(encodeURIComponent(str));
                                const arr = new Uint8Array(utf8.length);
                                for (let i = 0; i < utf8.length; i++) arr[i] = utf8.charCodeAt(i);
                                return arr;
                            }}
                        }};
                    }}
                    if (typeof globalThis.TextDecoder === 'undefined') {{
                        globalThis.TextDecoder = class TextDecoder {{
                            constructor(enc) {{ this.encoding = (enc || 'utf-8').toLowerCase(); }}
                            decode(input) {{
                                if (!input) return '';
                                const bytes = input instanceof Uint8Array
                                    ? input
                                    : (input instanceof ArrayBuffer
                                        ? new Uint8Array(input)
                                        : (ArrayBuffer.isView(input) ? new Uint8Array(input.buffer, input.byteOffset, input.byteLength) : new Uint8Array(input)));
                                let bin = '';
                                for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
                                try {{ return decodeURIComponent(escape(bin)); }} catch (_e) {{ return bin; }}
                            }}
                        }};
                    }}

                    if (typeof globalThis.Buffer === 'undefined') {{
                        globalThis.Buffer = {{
                            isBuffer: (b) => b instanceof Uint8Array,
                            alloc: (n) => new Uint8Array(n),
                            concat: (list) => {{
                                const total = list.reduce((s, a) => s + a.length, 0);
                                const out = new Uint8Array(total);
                                let off = 0;
                                for (const a of list) {{ out.set(a, off); off += a.length; }}
                                return out;
                            }},
                            from: (val, enc) => {{
                                if (typeof val === 'string') {{
                                    if (enc === 'base64') {{
                                        const bin = atob(val);
                                        const arr = new Uint8Array(bin.length);
                                        for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
                                        return arr;
                                    }}
                                    if (enc === 'hex') {{
                                        const len = Math.floor(val.length / 2);
                                        const arr = new Uint8Array(len);
                                        for (let i = 0; i < len; i++) arr[i] = parseInt(val.substr(i * 2, 2), 16) || 0;
                                        return arr;
                                    }}
                                    return new TextEncoder().encode(val);
                                }}
                                return new Uint8Array(val || 0);
                            }}
                        }};
                        const nativeToStr = Uint8Array.prototype.toString;
                        Uint8Array.prototype.toString = function(enc) {{
                            if (enc === 'utf8' || enc === 'utf-8') return new TextDecoder().decode(this);
                            if (enc === 'base64') {{
                                let bin = '';
                                for (let i = 0; i < this.length; i++) bin += String.fromCharCode(this[i]);
                                return btoa(bin);
                            }}
                            if (enc === 'hex') {{
                                return Array.from(this, (b) => b.toString(16).padStart(2, '0')).join('');
                            }}
                            return nativeToStr.call(this);
                        }};
                    }}

                    if (typeof globalThis.crypto === 'undefined') {{
                        globalThis.crypto = {{
                            randomUUID: () => __vegaUuidV4(),
                            getRandomValues: (arr) => {{
                                if (!arr || typeof arr.length !== 'number') return arr;
                                const byteView = new Uint8Array(arr.buffer || arr, arr.byteOffset || 0, arr.byteLength || arr.length);
                                const bytes = __vegaRandomBytes(byteView.length);
                                for (let i = 0; i < byteView.length; i++) byteView[i] = bytes[i] || 0;
                                return arr;
                            }}
                        }};
                    }}

                    if (typeof globalThis.URLSearchParams === 'undefined') {{
                        globalThis.URLSearchParams = class URLSearchParams {{
                            constructor(init, onChange) {{
                                this._pairs = [];
                                this._onChange = typeof onChange === 'function' ? onChange : null;
                                if (typeof init === 'string') {{
                                    const clean = init.startsWith('?') ? init.slice(1) : init;
                                    if (clean) {{
                                        for (const part of clean.split('&')) {{
                                            if (!part) continue;
                                            const eq = part.indexOf('=');
                                            const k = decodeURIComponent((eq >= 0 ? part.slice(0, eq) : part).replace(/\+/g, ' '));
                                            const v = eq >= 0 ? decodeURIComponent(part.slice(eq + 1).replace(/\+/g, ' ')) : '';
                                            this._pairs.push([k, v]);
                                        }}
                                    }}
                                }} else if (init instanceof URLSearchParams) {{
                                    for (const [k, v] of init._pairs) this._pairs.push([k, v]);
                                }} else if (Array.isArray(init)) {{
                                    for (const pair of init) {{
                                        if (pair && pair.length >= 2) this._pairs.push([String(pair[0]), String(pair[1])]);
                                    }}
                                }} else if (init && typeof init === 'object') {{
                                    for (const [k, v] of Object.entries(init)) {{
                                        if (v !== undefined) this._pairs.push([String(k), String(v)]);
                                    }}
                                }}
                            }}
                            _notify() {{ if (this._onChange) this._onChange(this.toString()); }}
                            append(k, v) {{ this._pairs.push([String(k), String(v)]); this._notify(); }}
                            set(k, v) {{
                                const key = String(k), val = String(v);
                                let replaced = false;
                                this._pairs = this._pairs.filter(([pk]) => {{
                                    if (pk !== key) return true;
                                    if (!replaced) {{ replaced = true; return true; }}
                                    return false;
                                }});
                                if (replaced) {{
                                    for (const pair of this._pairs) if (pair[0] === key) {{ pair[1] = val; break; }}
                                }} else {{
                                    this._pairs.push([key, val]);
                                }}
                                this._notify();
                            }}
                            get(k) {{ const f = this._pairs.find(([pk]) => pk === String(k)); return f ? f[1] : null; }}
                            getAll(k) {{ return this._pairs.filter(([pk]) => pk === String(k)).map(([, v]) => v); }}
                            has(k) {{ return this._pairs.some(([pk]) => pk === String(k)); }}
                            delete(k) {{ this._pairs = this._pairs.filter(([pk]) => pk !== String(k)); this._notify(); }}
                            entries() {{ return this._pairs[Symbol.iterator](); }}
                            keys() {{ return this._pairs.map(([k]) => k)[Symbol.iterator](); }}
                            values() {{ return this._pairs.map(([, v]) => v)[Symbol.iterator](); }}
                            [Symbol.iterator]() {{ return this.entries(); }}
                            forEach(cb, thisArg) {{ for (const [k, v] of this._pairs) cb.call(thisArg, v, k, this); }}
                            toString() {{
                                return this._pairs.map(([k, v]) => encodeURIComponent(k).replace(/%20/g, '+') + '=' + encodeURIComponent(v).replace(/%20/g, '+')).join('&');
                            }}
                        }};
                    }}

                    if (typeof globalThis.URL === 'undefined') {{
                        globalThis.URL = class URL {{
                            constructor(input, base) {{
                                const parsed = JSON.parse(__vegaParseUrl(String(input || ''), base !== undefined ? String(base) : ''));
                                if (!parsed.ok) throw new TypeError('Invalid URL: ' + input + ' (' + parsed.error + ')');
                                this.href = parsed.href;
                                this.origin = parsed.origin;
                                this.protocol = parsed.protocol;
                                this.host = parsed.host;
                                this.hostname = parsed.hostname;
                                this.port = parsed.port;
                                this.pathname = parsed.pathname;
                                this.search = parsed.search;
                                this.hash = parsed.hash;
                                this.searchParams = new URLSearchParams(this.search, (qs) => {{
                                    this.search = qs ? '?' + qs : '';
                                    const baseNoQuery = this.origin + this.pathname + this.search + this.hash;
                                    this.href = baseNoQuery;
                                }});
                            }}
                            toString() {{ return this.href; }}
                            toJSON() {{ return this.href; }}
                        }};
                    }}

                    if (typeof globalThis.Headers === 'undefined') {{
                        globalThis.Headers = class Headers {{
                            constructor(init) {{
                                this._map = new Map();
                                if (init instanceof Headers) {{
                                    init.forEach((v, k) => this.append(k, v));
                                }} else if (Array.isArray(init)) {{
                                    for (const [k, v] of init) this.append(k, v);
                                }} else if (init && typeof init === 'object') {{
                                    for (const [k, v] of Object.entries(init)) {{
                                        if (v !== undefined && v !== null) this.set(k, v);
                                    }}
                                }}
                            }}
                            _norm(k) {{ return String(k).toLowerCase(); }}
                            get(k) {{ const e = this._map.get(this._norm(k)); return e ? e[1] : null; }}
                            has(k) {{ return this._map.has(this._norm(k)); }}
                            set(k, v) {{ this._map.set(this._norm(k), [String(k), String(v)]); }}
                            append(k, v) {{
                                const key = this._norm(k);
                                const prev = this._map.get(key);
                                this._map.set(key, [String(k), prev ? prev[1] + ', ' + String(v) : String(v)]);
                            }}
                            delete(k) {{ this._map.delete(this._norm(k)); }}
                            entries() {{ return Array.from(this._map.values()).map(([k, v]) => [k.toLowerCase(), v])[Symbol.iterator](); }}
                            keys() {{ return Array.from(this._map.keys())[Symbol.iterator](); }}
                            values() {{ return Array.from(this._map.values()).map(([, v]) => v)[Symbol.iterator](); }}
                            [Symbol.iterator]() {{ return this.entries(); }}
                            forEach(cb, thisArg) {{ for (const [k, v] of this.entries()) cb.call(thisArg, v, k, this); }}
                        }};
                    }}

                    if (typeof globalThis.FormData === 'undefined') {{
                        globalThis.FormData = class FormData {{
                            constructor() {{ this._entries = []; }}
                            append(k, v) {{ this._entries.push([String(k), String(v)]); }}
                            set(k, v) {{
                                const key = String(k);
                                this._entries = this._entries.filter(([ek]) => ek !== key);
                                this._entries.push([key, String(v)]);
                            }}
                            get(k) {{ const f = this._entries.find(([ek]) => ek === String(k)); return f ? f[1] : null; }}
                            getAll(k) {{ return this._entries.filter(([ek]) => ek === String(k)).map(([, v]) => v); }}
                            has(k) {{ return this._entries.some(([ek]) => ek === String(k)); }}
                            delete(k) {{ this._entries = this._entries.filter(([ek]) => ek !== String(k)); }}
                            entries() {{ return this._entries[Symbol.iterator](); }}
                            [Symbol.iterator]() {{ return this.entries(); }}
                            forEach(cb, thisArg) {{ for (const [k, v] of this._entries) cb.call(thisArg, v, k, this); }}
                        }};
                    }}

                    if (typeof globalThis.AbortController === 'undefined') {{
                        class AbortSignal {{
                            constructor() {{
                                this.aborted = false;
                                this.reason = undefined;
                                this.onabort = null;
                                this._listeners = [];
                            }}
                            addEventListener(type, fn) {{
                                if (type === 'abort' && typeof fn === 'function') this._listeners.push(fn);
                            }}
                            removeEventListener(type, fn) {{
                                if (type === 'abort') this._listeners = this._listeners.filter((f) => f !== fn);
                            }}
                        }}
                        globalThis.AbortSignal = AbortSignal;
                        globalThis.AbortController = class AbortController {{
                            constructor() {{ this.signal = new AbortSignal(); }}
                            abort(reason) {{
                                if (this.signal.aborted) return;
                                this.signal.aborted = true;
                                this.signal.reason = reason !== undefined ? reason : new Error('Aborted');
                                if (typeof this.signal.onabort === 'function') this.signal.onabort();
                                for (const fn of this.signal._listeners.slice()) fn();
                            }}
                        }};
                    }}

                    function __vegaNormalizeHeaders(raw) {{
                        const out = {{}};
                        if (!raw) return out;
                        if (raw instanceof Headers) {{
                            raw.forEach((v, k) => {{ out[k] = v; }});
                        }} else if (typeof raw === 'object') {{
                            for (const [k, v] of Object.entries(raw)) {{
                                if (v !== undefined && v !== null) out[k] = String(v);
                            }}
                        }}
                        return out;
                    }}

                    function __vegaSerializeBody(data, headersObj) {{
                        if (data === undefined || data === null) return '';
                        const hasContentType = () => Object.keys(headersObj).some((k) => k.toLowerCase() === 'content-type');
                        if (typeof FormData !== 'undefined' && data instanceof FormData) {{
                            const boundary = '----VegaBoundary' + Math.random().toString(16).slice(2);
                            if (!hasContentType()) headersObj['Content-Type'] = 'multipart/form-data; boundary=' + boundary;
                            let out = '';
                            for (const [k, v] of data.entries()) {{
                                out += '--' + boundary + '\r\nContent-Disposition: form-data; name="' + k.replace(/"/g, '\\"') + '"\r\n\r\n' + v + '\r\n';
                            }}
                            return out + '--' + boundary + '--\r\n';
                        }}
                        if (typeof URLSearchParams !== 'undefined' && data instanceof URLSearchParams) {{
                            if (!hasContentType()) headersObj['Content-Type'] = 'application/x-www-form-urlencoded;charset=UTF-8';
                            return data.toString();
                        }}
                        if (typeof data === 'string') return data;
                        if (!hasContentType()) headersObj['Content-Type'] = 'application/json';
                        return JSON.stringify(data);
                    }}

                    if (typeof globalThis.fetch === 'undefined') {{
                        globalThis.fetch = async function(url, init) {{
                            const cfg = init || {{}};
                            if (cfg.signal && cfg.signal.aborted) throw new Error('Request aborted');
                            const method = String(cfg.method || 'GET').toUpperCase();
                            const headers = __vegaNormalizeHeaders(cfg.headers);
                            const body = __vegaSerializeBody(cfg.body, headers);
                            const rawRes = JSON.parse(__vegaHttpFetch(String(url), method, JSON.stringify(headers), body));
                            if (rawRes.error) throw new Error(rawRes.error);
                            const resHeaders = new Headers(rawRes.headers || {{}});
                            const resObj = {{
                                ok: rawRes.status >= 200 && rawRes.status < 300,
                                status: rawRes.status,
                                statusText: rawRes.status >= 200 && rawRes.status < 300 ? 'OK' : 'Error',
                                url: rawRes.url || String(url),
                                redirected: Boolean(rawRes.url && rawRes.url !== String(url)),
                                headers: resHeaders,
                                text: async () => String(rawRes.body || ''),
                                json: async () => JSON.parse(rawRes.body || 'null'),
                                arrayBuffer: async () => new TextEncoder().encode(String(rawRes.body || '')).buffer,
                                clone: () => Object.assign({{}}, resObj)
                            }};
                            return resObj;
                        }};
                    }}

                    globalThis.providerContext = {{
                        providerValue: {pv_json},
                        commonHeaders: {{ "User-Agent": {ua_json} }},
                        getBaseUrl: async (name) => __vegaGetBaseUrl(String(name || "")),
                        openWebView: async (url, options) => {{
                            try {{
                                const reqHdrs = Object.assign({{ "User-Agent": {ua_json} }}, __vegaNormalizeHeaders(options && options.headers));
                                const rawRes = JSON.parse(__vegaHttpFetch(String(url || ""), "GET", JSON.stringify(reqHdrs), ""));
                                const setCookie = (rawRes.headers && (rawRes.headers["set-cookie"] || rawRes.headers["cookie"])) || "";
                                return {{
                                    data: String(rawRes.body || ""),
                                    cookies: setCookie,
                                    cookie: setCookie,
                                    cookieMap: {{}},
                                    userAgent: reqHdrs["User-Agent"] || {ua_json},
                                    url: rawRes.url || String(url || "")
                                }};
                            }} catch (_e) {{
                                return {{ data: "", cookies: "", cookie: "", cookieMap: {{}}, userAgent: {ua_json}, url: String(url || "") }};
                            }}
                        }},
                        kvStore: {{
                            get: async (k) => {{
                                const raw = __vegaKvGet(String(k));
                                if (!raw) return null;
                                try {{ return JSON.parse(raw); }} catch {{ return raw; }}
                            }},
                            set: async (k, v) => __vegaKvSet(String(k), typeof v === 'string' ? v : JSON.stringify(v)),
                            delete: async (k) => __vegaKvDel(String(k)),
                            remove: async (k) => __vegaKvDel(String(k)),
                            clear: async () => __vegaKvClear(),
                        }},
                        cheerio: {{
                            load: (html) => globalThis.__vegaCheerioLoad(html)
                        }},
                        axios: async function(configOrUrl, config) {{
                            let url = "", method = "GET", cfg = {{}};
                            if (typeof configOrUrl === 'string') {{
                                url = configOrUrl;
                                cfg = config || {{}};
                            }} else if (configOrUrl) {{
                                cfg = configOrUrl;
                                url = cfg.url || "";
                            }}
                            if (cfg.signal && cfg.signal.aborted) throw new Error('Request aborted');
                            method = cfg.method || method;
                            const headers = __vegaNormalizeHeaders(cfg.headers);
                            const data = cfg.data !== undefined ? cfg.data : null;
                            if (cfg.maxRedirects === 0 || cfg.maxRedirects === '0') {{
                                headers['x-max-redirects'] = '0';
                            }}
                            if (cfg.params && typeof cfg.params === 'object') {{
                                const sep = url.includes('?') ? '&' : '?';
                                const parts = [];
                                for (const [k, v] of Object.entries(cfg.params)) {{
                                    if (v !== null && v !== undefined) {{
                                        parts.push(encodeURIComponent(k) + '=' + encodeURIComponent(String(v)));
                                    }}
                                }}
                                if (parts.length) url += sep + parts.join('&');
                            }}
                            const bodyStr = __vegaSerializeBody(data, headers);
                            const rawRes = JSON.parse(__vegaHttpFetch(String(url), String(method), JSON.stringify(headers), bodyStr));
                            if (rawRes.error) throw new Error(rawRes.error);
                            let parsedBody = rawRes.body;
                            try {{ parsedBody = JSON.parse(rawRes.body); }} catch {{}}
                            const validate = cfg.validateStatus || ((s) => s >= 200 && s < 300);
                            if (!validate(rawRes.status)) {{
                                const err = new Error('Request failed with status code ' + rawRes.status);
                                err.response = {{ data: parsedBody, status: rawRes.status, headers: rawRes.headers || {{}}, config: {{ url }} }};
                                err.config = {{ url }};
                                throw err;
                            }}
                            return {{
                                data: parsedBody,
                                status: rawRes.status,
                                headers: rawRes.headers || {{}},
                                config: Object.assign({{}}, cfg, {{ url }}),
                                url: rawRes.url || url,
                                request: {{ responseURL: rawRes.url || url }}
                            }};
                        }}
                    }};
                    providerContext.axios.get = (u, c) => providerContext.axios(u, Object.assign({{}}, c || {{}}, {{ method: 'GET' }}));
                    providerContext.axios.post = (u, d, c) => providerContext.axios(u, Object.assign({{}}, c || {{}}, {{ method: 'POST', data: d }}));
                    providerContext.axios.put = (u, d, c) => providerContext.axios(u, Object.assign({{}}, c || {{}}, {{ method: 'PUT', data: d }}));
                    providerContext.axios.delete = (u, c) => providerContext.axios(u, Object.assign({{}}, c || {{}}, {{ method: 'DELETE' }}));
                    providerContext.axios.head = (u, c) => providerContext.axios(u, Object.assign({{}}, c || {{}}, {{ method: 'HEAD' }}));
                    providerContext.axios.patch = (u, d, c) => providerContext.axios(u, Object.assign({{}}, c || {{}}, {{ method: 'PATCH', data: d }}));
                    providerContext.axios.request = (c) => providerContext.axios(c);
                    "#
                );
                ctx.eval::<(), _>(init_ctx_js)
                    .map_err(|e| caught_js_error(&ctx, "init context", e))?;

                // Load CommonJS modules into __vegaModules
                for (mod_name, code) in &modules_owned {
                    let mod_name_json = serde_json::to_string(mod_name).unwrap();
                    let code_json = serde_json::to_string(code).unwrap();
                    let load_js = format!(
                        r#"
                        (function() {{
                            var exports = {{}};
                            var module = {{ exports: exports }};
                            var fn = new Function('exports', 'module', {code_json});
                            fn(exports, module);
                            var me = module.exports;
                            if (me && me.__esModule && me.default && typeof me.default === 'object') {{
                                me = Object.assign({{}}, me, me.default);
                            }}
                            __vegaModules[{mod_name_json}] = me || {{}};
                        }})();
                        "#
                    );
                    ctx.eval::<(), _>(load_js)
                        .map_err(|e| caught_js_error(&ctx, &format!("module '{mod_name}'"), e))?;
                }

                // Execute async invocation and drive promise + active timers to completion
                let wrapped = format!("(async function() {{ {body_owned} }})()");
                let promise: Promise = ctx
                    .eval(wrapped)
                    .map_err(|e| caught_js_error(&ctx, "invoke", e))?;
                loop {
                    if let Some(res) = promise.result::<String>() {
                        return res.map_err(|e| caught_js_error(&ctx, "await", e));
                    }
                    if ctx.execute_pending_job() {
                        continue;
                    }
                    if cancel_flag.load(Ordering::Relaxed) {
                        return Err(VegaError::Timeout);
                    }
                    let wait_ms: i64 = ctx.eval("__vegaStepTimers()").unwrap_or(-1);
                    if wait_ms < 0 {
                        return Err(VegaError::Js(
                            "await: Promise remained pending with no active jobs or timers".into(),
                        ));
                    }
                    if wait_ms > 0 {
                        std::thread::sleep(Duration::from_millis((wait_ms as u64).min(25)));
                    }
                }
            })
        });

        match tokio::time::timeout(DEFAULT_EXEC_TIMEOUT, join).await {
            Ok(Ok(inner)) => inner,
            Ok(Err(join_err)) => Err(VegaError::Js(join_err.to_string())),
            Err(_) => Err(VegaError::Timeout),
        }
    }
}

fn caught_js_error<'js>(ctx: &Ctx<'js>, stage: &str, err: rquickjs::Error) -> VegaError {
    match CaughtError::from_error(ctx, err) {
        CaughtError::Exception(ex) => {
            let msg = ex.message().unwrap_or_default();
            let stack = ex.stack().unwrap_or_default();
            if stack.trim().is_empty() {
                VegaError::Js(format!("{stage}: {msg}"))
            } else {
                VegaError::Js(format!("{stage}: {msg}\n{stack}"))
            }
        }
        other => VegaError::Js(format!("{stage}: {other}")),
    }
}

fn resolve_icon_url(repo: &str, raw_icon: &str) -> String {
    let icon = raw_icon.trim();
    if icon.is_empty() {
        return String::new();
    }
    let lower = icon.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return icon.to_string();
    }
    if icon.starts_with("//") {
        return format!("https:{icon}");
    }
    let clean_repo = repo.trim_end_matches('/');
    if icon.starts_with('/') {
        format!("{clean_repo}{icon}")
    } else {
        format!("{clean_repo}/{icon}")
    }
}

/// Lightweight TypeScript-to-JS preprocessor mirroring Android's `VegaProviderRunner.cleanTypeScript`.
pub fn clean_typescript(ts_code: &str) -> String {
    let mut js = ts_code.to_string();
    let patterns = [
        (r"(?m)//\s*@ts-(?:ignore|expect-error).*", ""),
        (r#"import\s+\{[^}]*\}\s+from\s+['"][^'"]*['"];?"#, ""),
        (r#"import\s+\*\s+as\s+\w+\s+from\s+['"][^'"]*['"];?"#, ""),
        (r#"import\s+\w+\s+from\s+['"][^'"]*['"];?"#, ""),
        (r#"import\s+['"][^'"]*['"];?"#, ""),
        (r"\bexport\s+default\s+", ""),
        (r"\bexport\s+async\s+function\b", "async function"),
        (r"\bexport\s+function\b", "function"),
        (r"\bexport\s+const\b", "const"),
        (r"\bexport\s+let\b", "let"),
        (r"\bexport\s+var\b", "var"),
        (r"\bexport\s+", ""),
        (r"\binterface\s+\w+[^{]*\{[^}]*\}", ""),
        (r"\btype\s+\w+\s*=[^;]+;", ""),
        (r"\)\s*:\s*Promise\s*<[^>]*>\s*=>", ") =>"),
        (r"\)\s*:\s*Promise\s*<[^>]*>\s*\{", ") {"),
    ];
    for (pat, rep) in patterns {
        if let Ok(re) = Regex::new(pat) {
            js = re.replace_all(&js, rep).into_owned();
        }
    }
    js
}

struct LinkMetaFields {
    image: Option<String>,
    duration: Option<String>,
    date: Option<String>,
    episode_number: Option<String>,
    season_number: Option<String>,
    size: Option<String>,
    quality: Option<String>,
    rating: Option<String>,
    overview: Option<String>,
}

fn extract_link_meta_fields(v: &Value, parent: Option<&LinkMetaFields>) -> LinkMetaFields {
    let parent_or = |mine: Option<String>, p: Option<&String>| mine.or_else(|| p.cloned());
    LinkMetaFields {
        image: parent_or(
            pick_str_or_num(v, &["image", "thumbnail", "poster", "poster_path", "backdrop"]),
            parent.and_then(|p| p.image.as_ref()),
        ),
        duration: parent_or(
            normalize_duration(pick_str_or_num(v, &["duration", "runtime", "time"]).as_deref()),
            parent.and_then(|p| p.duration.as_ref()),
        ),
        date: parent_or(
            pick_str_or_num(
                v,
                &[
                    "date",
                    "releaseDate",
                    "firstAired",
                    "airDate",
                    "airdate",
                    "aired",
                    "released",
                ],
            )
            .map(|d| d.chars().take(10).collect()),
            parent.and_then(|p| p.date.as_ref()),
        ),
        episode_number: parent_or(
            pick_str_or_num(v, &["episodeNumber", "episode", "ep", "number"]),
            parent.and_then(|p| p.episode_number.as_ref()),
        ),
        season_number: parent_or(
            pick_str_or_num(v, &["seasonNumber", "season"]),
            parent.and_then(|p| p.season_number.as_ref()),
        ),
        size: parent_or(
            pick_str_or_num(v, &["size", "fileSize", "filesize"]),
            parent.and_then(|p| p.size.as_ref()),
        ),
        quality: parent_or(
            pick_str_or_num(v, &["quality", "res", "resolution"]),
            parent.and_then(|p| p.quality.as_ref()),
        ),
        rating: parent_or(
            pick_str_or_num(v, &["rating", "imdbRating"]),
            parent.and_then(|p| p.rating.as_ref()),
        ),
        overview: parent_or(
            pick_str_or_num(v, &["overview", "description", "summary", "synopsis", "plot"])
                .map(|s| strip_html_tags(&s))
                .filter(|s| !s.is_empty()),
            parent.and_then(|p| p.overview.as_ref()),
        ),
    }
}

fn parse_vega_link_item(v: &Value, fallback_series_type: bool) -> VegaLink {
    let title = clean_title(pick_str_or_num(v, &["title", "name"]).as_deref().unwrap_or(""));
    let meta = extract_link_meta_fields(v, None);
    let direct_href = pick_str_or_num(v, &["link", "url"]);
    let explicit_ep_link = pick_str_or_num(v, &["episodesLink"]);

    let direct_links = if let Some(d_arr) = v.get("directLinks").and_then(|d| d.as_array()) {
        Some(
            d_arr
                .iter()
                .map(|dl| {
                    let dl_meta = extract_link_meta_fields(dl, Some(&meta));
                    VegaDirectLink {
                        title: clean_title(
                            pick_str_or_num(dl, &["title", "name"]).as_deref().unwrap_or(""),
                        ),
                        link: pick_str_or_num(dl, &["link", "url"]).unwrap_or_default(),
                        r#type: pick_str_or_num(dl, &["type"])
                            .unwrap_or_else(|| "movie".to_string()),
                        image: dl_meta.image,
                        duration: dl_meta.duration,
                        date: dl_meta.date,
                        episode_number: dl_meta.episode_number,
                        season_number: dl_meta.season_number,
                        size: dl_meta.size,
                        quality: dl_meta.quality,
                        rating: dl_meta.rating,
                        overview: dl_meta.overview,
                    }
                })
                .collect(),
        )
    } else if fallback_series_type {
        direct_href.as_ref().filter(|l| !l.is_empty()).map(|href| {
            vec![VegaDirectLink {
                title: title.clone(),
                link: href.clone(),
                r#type: pick_str_or_num(v, &["type"])
                    .unwrap_or_else(|| "series".to_string()),
                image: meta.image.clone(),
                duration: meta.duration.clone(),
                date: meta.date.clone(),
                episode_number: meta.episode_number.clone(),
                season_number: meta.season_number.clone(),
                size: meta.size.clone(),
                quality: meta.quality.clone(),
                rating: meta.rating.clone(),
                overview: meta.overview.clone(),
            }]
        })
    } else {
        None
    };

    VegaLink {
        title,
        episodes_link: if direct_links.is_some() {
            explicit_ep_link
        } else {
            explicit_ep_link.or(direct_href)
        },
        direct_links,
        image: meta.image,
        duration: meta.duration,
        date: meta.date,
        episode_number: meta.episode_number,
        season_number: meta.season_number,
        size: meta.size,
        quality: meta.quality,
        rating: meta.rating,
        overview: meta.overview,
    }
}

fn pick_str_or_num(v: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(val) = v.get(*key) else {
            continue;
        };
        match val {
            Value::String(s) => {
                let t = s.trim();
                if !t.is_empty() && !t.eq_ignore_ascii_case("null") {
                    return Some(t.to_string());
                }
            }
            Value::Number(n) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

fn parse_genres(v: &Value) -> Vec<String> {
    let raw = v.get("genres").or_else(|| v.get("genre"));
    let Some(val) = raw else {
        return Vec::new();
    };
    if let Some(arr) = val.as_array() {
        arr.iter()
            .filter_map(|x| x.as_str().map(str::trim))
            .filter(|s| !s.is_empty())
            .take(5)
            .map(str::to_string)
            .collect()
    } else if let Some(s) = val.as_str() {
        s.split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .take(5)
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    }
}

fn normalize_duration(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(mins) = trimmed.parse::<u32>() {
        if mins == 0 {
            return None;
        }
        return Some(if mins >= 60 {
            format!("{}h {}m", mins / 60, mins % 60)
        } else {
            format!("{mins}m")
        });
    }
    Some(trimmed.to_string())
}

fn searchable_title(raw: &str) -> String {
    let cleaned = clean_title(raw);
    let bracket_re = Regex::new(r"\[[^\]]*\]|\([^\)]*\)").expect("regex");
    let noise_re = Regex::new(
        r"(?i)\b(season\s*\d+|s\d{1,2}|episode\s*\d+|ep\s*\d+|part\s*\d+|hindi|english|tamil|telugu|kannada|malayalam|dubbed|dual\s*audio|multi\s*audio|web-dl|bluray|hdrip|hdcam|480p|720p|1080p|2160p|4k)\b.*",
    )
    .expect("regex");
    let no_brackets = bracket_re.replace_all(&cleaned, " ");
    let core = noise_re.replace_all(&no_brackets, " ");
    core.split(|c: char| matches!(c, '-' | '–' | '—' | ':' | '|'))
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_html_tags(raw: &str) -> String {
    let tag_re = Regex::new(r"<[^>]+>").expect("regex");
    tag_re
        .replace_all(raw, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn urlencoding_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push_str("%20");
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn merge_enrichment(dst: &mut DetailEnrichment, src: DetailEnrichment) {
    if dst.imdb_id.is_none() {
        dst.imdb_id = src.imdb_id;
    }
    if dst.year.is_none() {
        dst.year = src.year;
    }
    if dst.rating.is_none() {
        dst.rating = src.rating;
    }
    if dst.runtime.is_none() {
        dst.runtime = src.runtime;
    }
    if dst.genres.is_empty() {
        dst.genres = src.genres;
    }
    if dst.background.is_none() {
        dst.background = src.background;
    }

    let mut index: HashMap<(Option<u32>, Option<u32>), usize> = HashMap::new();
    for (idx, ep) in dst.episodes.iter().enumerate() {
        index.insert((ep.season, ep.episode), idx);
    }

    for ep in src.episodes {
        if let Some(&idx) = index.get(&(ep.season, ep.episode)) {
            let slot = &mut dst.episodes[idx];
            if slot.title.is_none() {
                slot.title = ep.title;
            }
            if slot.image.is_none() {
                slot.image = ep.image;
            }
            if slot.overview.is_none() {
                slot.overview = ep.overview;
            }
            if slot.duration.is_none() {
                slot.duration = ep.duration;
            }
            if slot.date.is_none() {
                slot.date = ep.date;
            }
            if slot.rating.is_none() {
                slot.rating = ep.rating;
            }
        } else {
            let idx = dst.episodes.len();
            index.insert((ep.season, ep.episode), idx);
            dst.episodes.push(ep);
        }
    }
}

fn format_bytes_human(bytes: u64) -> Option<String> {
    if bytes == 0 {
        return None;
    }
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * MB;
    if (bytes as f64) >= GB {
        Some(format!("{:.2} GB", bytes as f64 / GB))
    } else if (bytes as f64) >= MB {
        Some(format!("{:.0} MB", bytes as f64 / MB))
    } else {
        Some(format!("{} KB", (bytes / 1024).max(1)))
    }
}

fn parse_vega_stream_item(v: &Value) -> Option<VegaStream> {
    let mut base: VegaStream = serde_json::from_value(v.clone()).ok()?;
    if base.link.trim().is_empty() {
        return None;
    }
    if base.size.as_deref().map(str::trim).unwrap_or("").is_empty() {
        base.size = pick_str_or_num(v, &["size", "fileSize", "filesize"])
            .or_else(|| {
                v.get("behaviorHints")
                    .and_then(|bh| bh.get("videoSize"))
                    .and_then(|n| n.as_u64())
                    .and_then(format_bytes_human)
            });
    }
    Some(base)
}

/// Lists installed extensions that match active manifest providers.
pub fn merge_installed_flags(
    manifest: Vec<VegaProvider>,
    installed: &HashSet<String>,
) -> Vec<VegaProvider> {
    manifest
        .into_iter()
        .map(|mut p| {
            p.is_installed = installed.contains(&p.value);
            p
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iptvmine_core::DnsMode;
    use iptvmine_net::DohState;

    #[test]
    fn resolves_github_and_handle_repo_urls() {
        assert_eq!(
            VegaRunner::resolve_repo_url("@alice/custom-providers"),
            "https://raw.githubusercontent.com/alice/custom-providers/refs/heads/main"
        );
        assert_eq!(
            VegaRunner::resolve_repo_url("https://github.com/user/my-vega/tree/dev"),
            "https://raw.githubusercontent.com/user/my-vega/refs/heads/dev"
        );
    }

    #[tokio::test]
    async fn quickjs_executes_vega_modules_with_cheerio_and_kvstore() {
        let storage = Arc::new(Storage::open_in_memory().unwrap());
        let doh = DohState::new(DnsMode::Off, String::new());
        let runner = VegaRunner::new(storage, doh.build_http_client(), false);

        let mut modules = HashMap::new();
        modules.insert(
            "catalog".into(),
            r#"exports.catalog = [{ title: "Popular", filter: "/popular" }];"#.into(),
        );
        modules.insert(
            "posts".into(),
            r#"
            exports.getPosts = async function({ filter, providerContext }) {
                await providerContext.kvStore.set("lastFilter", filter);
                const saved = await providerContext.kvStore.get("lastFilter");
                const $ = providerContext.cheerio.load('<div class="card"><a href="/m/1">Demo Movie</a></div>');
                const title = $('.card a').text();
                const link = $('.card a').attr('href');
                return [{ title: title + " (" + saved + ")", link, image: "" }];
            };
            "#
            .into(),
        );

        let json_str = runner
            .eval_modules(
                "https://repo",
                "demo",
                &modules,
                r#"
                const res = await __vegaModules['posts'].getPosts({
                    filter: __vegaModules['catalog'].catalog[0].filter,
                    providerContext: globalThis.providerContext
                });
                return JSON.stringify(res);
                "#,
            )
            .await
            .unwrap();

        let posts: Vec<VegaPost> = serde_json::from_str(&json_str).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].title, "Demo Movie (/popular)");
        assert_eq!(posts[0].link, "/m/1");
    }

    #[tokio::test]
    async fn cheerio_supports_contains_and_global_atob_btoa_fetch() {
        let storage = Arc::new(Storage::open_in_memory().unwrap());
        let doh = DohState::new(DnsMode::Off, String::new());
        let runner = VegaRunner::new(storage, doh.build_http_client(), false);

        let mut modules = HashMap::new();
        modules.insert(
            "stream".into(),
            r#"
            exports.getStream = async function({ providerContext }) {
                const $ = providerContext.cheerio.load('<div><a href="https://fast.example/1.mp4">G-Direct Link</a></div>');
                const href = $("a:contains('G-Direct'), a[href*='fastdl']").attr('href');
                const encoded = btoa("ok");
                return [{ server: atob(encoded), link: href, type: "mp4", quality: "1080" }];
            };
            "#
            .into(),
        );

        let json_str = runner
            .eval_modules(
                "https://repo",
                "demo",
                &modules,
                r#"
                const res = await __vegaModules['stream'].getStream({ providerContext: globalThis.providerContext });
                return JSON.stringify(res);
                "#,
            )
            .await
            .unwrap();
        let streams: Vec<VegaStream> = serde_json::from_str(&json_str).unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].server, "ok");
        assert_eq!(streams[0].link, "https://fast.example/1.mp4");
    }

    #[tokio::test]
    async fn web_apis_dom_mutations_and_detailed_errors_work() {
        let storage = Arc::new(Storage::open_in_memory().unwrap());
        let doh = DohState::new(DnsMode::Off, String::new());
        let runner = VegaRunner::new(storage, doh.build_http_client(), false);

        let mut modules = HashMap::new();
        modules.insert(
            "meta".into(),
            r#"
            exports.getMeta = async function({ link, providerContext }) {
                const u = new URL(link, 'https://example.org/');
                const params = new URLSearchParams({ q: 'hello world', page: '2' });
                const h = new Headers({ 'X-Custom': 'val' });
                const fd = new FormData();
                fd.append('token', 'abc');
                const enc = new TextEncoder().encode('ok');
                const dec = new TextDecoder().decode(enc);
                const ac = new AbortController();
                await new Promise((r) => setTimeout(r, 15));

                const $ = providerContext.cheerio.load(`
                    <div class="box">
                        <script>bad()</script>
                        <h2>Storyline</h2>
                        <div class="plot"><span>Real</span> Plot</div>
                        <strong>Description:</strong>
                        <a href="/d/1">Link 1</a>
                        <a href="/d/2">Link 2</a>
                    </div>
                `);
                $('script').remove();
                const own = $('.box').clone();
                own.find('h2,strong,a').remove();
                const synopsis = own.find('.plot').text().trim();
                const nextPlot = $('h2').next('.plot').text().trim();
                const hrefs = [];
                for (const el of $('a')) {
                    if ($(el).is('a')) hrefs.push($(el).attr('href'));
                }
                const uuidOk = typeof crypto.randomUUID() === 'string' && !ac.signal.aborted;
                const b64 = Buffer.from('ok').toString('base64');
                return {
                    title: u.pathname + '?' + params.toString(),
                    synopsis: synopsis + '|' + nextPlot + '|' + h.get('x-custom') + '|' + fd.get('token') + '|' + dec + '|' + b64 + '|' + uuidOk,
                    image: hrefs.join(','),
                    imdbId: '',
                    type: 'movie',
                    linkList: []
                };
            };
            "#
            .into(),
        );

        let json_str = runner
            .eval_modules(
                "https://repo",
                "demo",
                &modules,
                r#"
                const res = await __vegaModules['meta'].getMeta({ link: '/post/1', providerContext: globalThis.providerContext });
                return JSON.stringify(res);
                "#,
            )
            .await
            .unwrap();
        let meta: VegaMeta = serde_json::from_str(&json_str).unwrap();
        assert_eq!(meta.title, "/post/1?q=hello+world&page=2");
        assert_eq!(meta.synopsis, "Real Plot|Real Plot|val|abc|ok|b2s=|true");
        assert_eq!(meta.image, "/d/1,/d/2");

        // Ensure JS errors include real exception text rather than generic QuickJS message
        let mut err_mods = HashMap::new();
        err_mods.insert(
            "posts".into(),
            r#"exports.getPosts = async () => { throw new Error('custom boom failure'); };"#.into(),
        );
        let err = runner
            .eval_modules(
                "https://repo",
                "demo",
                &err_mods,
                "return JSON.stringify(await __vegaModules['posts'].getPosts());",
            )
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("custom boom failure"), "unexpected error: {msg}");
    }

    #[test]
    fn parses_numeric_episode_and_stream_size_metadata() {
        let ep_val = json!({
            "title": "S1E1",
            "link": "series::tt0760437:1:1",
            "season": 1,
            "episode": 1,
            "runtime": 30,
            "airdate": "2005-12-27",
            "size": "420 MB"
        });
        let parsed = parse_vega_link_item(&ep_val, true);
        assert_eq!(parsed.season_number.as_deref(), Some("1"));
        assert_eq!(parsed.episode_number.as_deref(), Some("1"));
        assert_eq!(parsed.duration.as_deref(), Some("30m"));
        assert_eq!(parsed.date.as_deref(), Some("2005-12-27"));
        assert_eq!(parsed.size.as_deref(), Some("420 MB"));

        let dl = parsed.direct_links.as_ref().unwrap().first().unwrap();
        assert_eq!(dl.episode_number.as_deref(), Some("1"));
        assert_eq!(dl.duration.as_deref(), Some("30m"));

        let stream_val = json!({
            "server": "Toonix",
            "link": "https://cdn.example/ep1.mp4",
            "type": "mp4",
            "quality": "1080p",
            "behaviorHints": { "videoSize": 1_610_612_736_u64 }
        });
        let stream = parse_vega_stream_item(&stream_val).unwrap();
        assert_eq!(stream.size.as_deref(), Some("1.50 GB"));
    }
}
