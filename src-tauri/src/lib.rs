use iptvmine_core::{
    derive_provider_title, detect_provider_quick, is_valid_stream_scheme, sanitize_filename,
    AppSettings, Channel, ContentFilter, DetailEnrichment, DnsMode, DownloadTask, Provider,
    ProviderType, VegaLink, VegaMeta, VegaPost, VegaProvider, WatchHistoryEntry,
};
use iptvmine_downloader::{open_containing_folder, resolve_download_dir, DownloadManager};
use iptvmine_m3u::{
    fetch_playlist_text, inspect_provider_url, parse_playlist, ChannelEngine, DetectionResult,
};
use iptvmine_media::{
    check_ffmpeg_status_with_upstream, detect_ffmpeg_status,
    install_latest_ffmpeg as download_latest_ffmpeg, launch_external_player as spawn_external_player,
    probe_media, FfmpegStatus, MediaProbeResult, ProxiedStream, StreamProxy,
};
use iptvmine_net::{DnsTestResult, DohState};
use iptvmine_storage::Storage;
use iptvmine_updater::{check_latest_release, UpdateCheckResult};
use iptvmine_vega::fallback::{resolve_with_auto_fallback, FallbackResult};
use iptvmine_vega::{merge_installed_flags, CatalogResponse, VegaRunner};
use reqwest::Client;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

pub struct AppState {
    pub storage: Arc<Storage>,
    pub doh: DohState,
    pub http_client: Client,
    pub channel_engine: ChannelEngine,
    pub vega_runner: VegaRunner,
    pub stream_proxy: StreamProxy,
    pub download_manager: DownloadManager,
    pub data_dir: PathBuf,
    pub cached_channels: RwLock<Vec<Channel>>,
    pub content_filter: ContentFilter,
    pub ffmpeg_install_lock: tokio::sync::Mutex<()>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn resolve_active_vega_url(storage: &Storage, requested: Option<&str>) -> Result<String, String> {
    if let Some(u) = requested.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(u.to_string());
    }
    let providers = storage.list_providers().map_err(|e| e.to_string())?;
    providers
        .into_iter()
        .find(|p| p.is_active && p.r#type == ProviderType::Vega)
        .map(|p| p.url)
        .ok_or_else(|| "No active Vega provider configured".to_string())
}

#[tauri::command]
fn list_providers(state: State<'_, AppState>) -> Result<Vec<Provider>, String> {
    let list = state.storage.list_providers().map_err(|e| e.to_string())?;
    tracing::info!("list_providers -> {} entries", list.len());
    Ok(list)
}

#[tauri::command]
fn add_provider(mut provider: Provider, state: State<'_, AppState>) -> Result<Vec<Provider>, String> {
    provider.url = provider.url.trim().to_string();
    if provider.url.is_empty() {
        return Err("Provider URL cannot be empty".into());
    }
    if provider.title.trim().is_empty() || provider.title.trim() == provider.url {
        provider.title = derive_provider_title(&provider.url);
    } else {
        provider.title = provider.title.trim().to_string();
    }
    tracing::info!("add_provider: title='{}' url='{}' type={:?}", provider.title, provider.url, provider.r#type);
    state
        .storage
        .upsert_provider(&provider)
        .map_err(|e| {
            tracing::error!("upsert_provider failed: {e}");
            e.to_string()
        })?;
    state.channel_engine.clear_backoffs();
    state.vega_runner.clear_module_cache();
    state.storage.list_providers().map_err(|e| e.to_string())
}

#[tauri::command]
fn update_provider(
    old_url: Option<String>,
    provider: Provider,
    state: State<'_, AppState>,
) -> Result<Vec<Provider>, String> {
    match old_url {
        Some(prev) if prev != provider.url => state
            .storage
            .replace_provider_url(&prev, &provider)
            .map_err(|e| e.to_string())?,
        _ => state
            .storage
            .upsert_provider(&provider)
            .map_err(|e| e.to_string())?,
    }
    state.channel_engine.clear_backoffs();
    state.vega_runner.clear_module_cache();
    state.storage.list_providers().map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_provider(url: String, state: State<'_, AppState>) -> Result<Vec<Provider>, String> {
    state
        .storage
        .remove_provider(&url)
        .map_err(|e| e.to_string())?;
    state.channel_engine.clear_backoffs();
    state.vega_runner.clear_module_cache();
    state.storage.list_providers().map_err(|e| e.to_string())
}

#[tauri::command]
fn detect_provider_type(url: String, title: String) -> ProviderType {
    detect_provider_quick(&url, &title)
}

#[tauri::command]
async fn inspect_provider(url: String, state: State<'_, AppState>) -> Result<DetectionResult, String> {
    if !is_valid_stream_scheme(&url) && !url.trim().starts_with('@') {
        return Err("URL must use http://, https://, or @handle".into());
    }
    Ok(inspect_provider_url(&state.http_client, &url).await)
}

#[tauri::command]
fn import_local_playlist(
    title: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<Vec<Provider>, String> {
    if content.trim().is_empty() {
        return Err("Imported file is empty".into());
    }
    let dir = state.data_dir.join("playlists");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = sanitize_filename(if title.trim().is_empty() {
        "local_playlist"
    } else {
        title.trim()
    });
    let file_name = format!("{}_{}.m3u", stem, now_ms());
    let path = dir.join(&file_name);
    std::fs::write(&path, &content).map_err(|e| e.to_string())?;

    let channels = parse_playlist(&content, &state.content_filter);
    let provider = Provider {
        title: if title.trim().is_empty() {
            stem
        } else {
            title.trim().to_string()
        },
        url: format!("local://{file_name}"),
        user_agent: None,
        is_active: true,
        channel_count: channels.len(),
        r#type: ProviderType::Iptv,
    };
    state
        .storage
        .upsert_provider(&provider)
        .map_err(|e| e.to_string())?;
    state.storage.list_providers().map_err(|e| e.to_string())
}

#[tauri::command]
async fn load_channels(
    force_refresh: bool,
    state: State<'_, AppState>,
) -> Result<Vec<Channel>, String> {
    if force_refresh {
        state.channel_engine.clear_backoffs();
    }
    let providers = state.storage.list_providers().map_err(|e| e.to_string())?;
    let active_iptv: Vec<Provider> = providers
        .into_iter()
        .filter(|p| p.is_active && p.r#type == ProviderType::Iptv)
        .collect();

    if active_iptv.is_empty() {
        if let Ok(mut guard) = state.cached_channels.write() {
            guard.clear();
        }
        return Ok(Vec::new());
    }

    let urls: Vec<String> = active_iptv.iter().map(|p| p.url.clone()).collect();
    if !force_refresh && state.channel_engine.should_skip_cooldown(&urls) {
        if let Ok(guard) = state.cached_channels.read() {
            if !guard.is_empty() {
                return Ok(guard.clone());
            }
        }
    }

    state.channel_engine.mark_fetched(&urls);
    let mut all_channels = Vec::new();
    let mut errors = Vec::new();

    for mut provider in active_iptv {
        if let Some(rel) = provider.url.strip_prefix("local://") {
            let clean_name = rel.replace(['/', '\\'], "");
            let path = state.data_dir.join("playlists").join(clean_name);
            match std::fs::read_to_string(&path) {
                Ok(body) => {
                    let parsed = parse_playlist(&body, &state.content_filter);
                    provider.channel_count = parsed.len();
                    let _ = state.storage.upsert_provider(&provider);
                    all_channels.extend(parsed);
                }
                Err(e) => errors.push(format!("{}: {}", provider.title, e)),
            }
            continue;
        }
        if let Err(wait_s) = state.channel_engine.check_backoff(&provider.url) {
            tracing::warn!("Skipping {} (backoff {}s)", provider.url, wait_s);
            continue;
        }
        tracing::info!("Fetching IPTV playlist: {}", provider.url);
        match fetch_playlist_text(&state.http_client, &provider.url, provider.user_agent.as_deref()).await {
            Ok(body) => {
                state.channel_engine.record_success(&provider.url);
                let parsed = parse_playlist(&body, &state.content_filter);
                tracing::info!("Parsed {} channels from {}", parsed.len(), provider.url);
                provider.channel_count = parsed.len();
                let _ = state.storage.upsert_provider(&provider);
                all_channels.extend(parsed);
            }
            Err(e) => {
                tracing::error!("Failed fetching {}: {}", provider.url, e);
                state.channel_engine.record_failure(&provider.url);
                errors.push(format!("{}: {}", provider.title, e));
            }
        }
    }

    if let Ok(mut guard) = state.cached_channels.write() {
        *guard = all_channels.clone();
    }
    if all_channels.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(all_channels)
}

#[tauri::command]
fn get_channel_categories(state: State<'_, AppState>) -> Vec<String> {
    let Ok(guard) = state.cached_channels.read() else {
        return Vec::new();
    };
    let set: BTreeSet<String> = guard.iter().map(|c| c.category.clone()).collect();
    set.into_iter().collect()
}

// ── Vega Extensions & VOD Commands (M3) ───────────────────────────────────────

#[tauri::command]
async fn list_vega_extensions(
    provider_url: Option<String>,
    force_refresh: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<VegaProvider>, String> {
    let repo_url = resolve_active_vega_url(&state.storage, provider_url.as_deref())?;
    let installed = state
        .storage
        .get_installed_extensions()
        .map_err(|e| e.to_string())?;

    if !force_refresh.unwrap_or(false) {
        if let Ok(Some(cached)) = state.storage.get_cached_scrapers(&repo_url) {
            return Ok(merge_installed_flags(cached, &installed));
        }
    }

    let fetched = state
        .vega_runner
        .fetch_manifest(&repo_url)
        .await
        .map_err(|e| e.to_string())?;
    let _ = state
        .storage
        .put_cached_scrapers(&repo_url, &fetched, now_ms());
    Ok(merge_installed_flags(fetched, &installed))
}

#[tauri::command]
fn set_extension_installed(
    value: String,
    installed: bool,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    state
        .storage
        .set_extension_installed(&value, installed)
        .map_err(|e| e.to_string())?;
    let set = state
        .storage
        .get_installed_extensions()
        .map_err(|e| e.to_string())?;
    Ok(set.into_iter().collect())
}

#[tauri::command]
async fn get_vod_catalog(
    provider_url: Option<String>,
    scraper_value: String,
    force_refresh: Option<bool>,
    state: State<'_, AppState>,
) -> Result<CatalogResponse, String> {
    let repo_url = resolve_active_vega_url(&state.storage, provider_url.as_deref())?;
    if !force_refresh.unwrap_or(false) {
        if let Ok(Some(cached)) = state.storage.get_cached_catalog(&repo_url, &scraper_value) {
            return Ok(CatalogResponse {
                catalog: cached,
                genres: Vec::new(),
            });
        }
    }
    let res = state
        .vega_runner
        .get_catalog(&repo_url, &scraper_value)
        .await
        .map_err(|e| e.to_string())?;
    let _ = state
        .storage
        .put_cached_catalog(&repo_url, &scraper_value, &res.catalog, now_ms());
    Ok(res)
}

#[tauri::command]
async fn get_vod_posts(
    provider_url: Option<String>,
    scraper_value: String,
    filter: String,
    page: i32,
    force_refresh: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<VegaPost>, String> {
    let repo_url = resolve_active_vega_url(&state.storage, provider_url.as_deref())?;
    if page <= 1 && !force_refresh.unwrap_or(false) {
        if let Ok(Some(cached)) =
            state
                .storage
                .get_cached_posts(&repo_url, &scraper_value, &filter)
        {
            return Ok(cached);
        }
    }
    let posts = state
        .vega_runner
        .get_posts(&repo_url, &scraper_value, &filter, page)
        .await
        .map_err(|e| e.to_string())?;
    if page <= 1 && !posts.is_empty() {
        let _ =
            state
                .storage
                .put_cached_posts(&repo_url, &scraper_value, &filter, &posts, now_ms());
    }
    Ok(posts)
}

#[tauri::command]
async fn search_vod_posts(
    provider_url: Option<String>,
    scraper_value: String,
    query: String,
    page: i32,
    state: State<'_, AppState>,
) -> Result<Vec<VegaPost>, String> {
    let repo_url = resolve_active_vega_url(&state.storage, provider_url.as_deref())?;
    state
        .vega_runner
        .get_search_posts(&repo_url, &scraper_value, &query, page)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_vod_meta(
    provider_url: Option<String>,
    scraper_value: String,
    link: String,
    state: State<'_, AppState>,
) -> Result<VegaMeta, String> {
    let repo_url = resolve_active_vega_url(&state.storage, provider_url.as_deref())?;
    state
        .vega_runner
        .get_meta(&repo_url, &scraper_value, &link)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_vod_episodes(
    provider_url: Option<String>,
    scraper_value: String,
    url: String,
    state: State<'_, AppState>,
) -> Result<Vec<VegaLink>, String> {
    let repo_url = resolve_active_vega_url(&state.storage, provider_url.as_deref())?;
    state
        .vega_runner
        .get_episodes(&repo_url, &scraper_value, &url)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_detail_enrichment(
    imdb_id: String,
    title: String,
    media_type: String,
    state: State<'_, AppState>,
) -> Result<DetailEnrichment, String> {
    Ok(state
        .vega_runner
        .get_detail_enrichment(&imdb_id, &title, &media_type)
        .await)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn resolve_vod_stream_with_fallback(
    app: AppHandle,
    provider_url: Option<String>,
    scraper_value: String,
    initial_link: String,
    initial_type: String,
    item_title: String,
    movie_title: String,
    current_meta: Option<VegaMeta>,
    state: State<'_, AppState>,
) -> Result<Option<FallbackResult>, String> {
    let repo_url = resolve_active_vega_url(&state.storage, provider_url.as_deref())?;
    let installed: Vec<String> = state
        .storage
        .get_installed_extensions()
        .map(|s| s.into_iter().collect())
        .unwrap_or_default();

    let res = resolve_with_auto_fallback(
        &state.vega_runner,
        &initial_link,
        &initial_type,
        &item_title,
        &movie_title,
        current_meta.as_ref(),
        &repo_url,
        &scraper_value,
        &installed,
        |status| {
            let _ = app.emit("vega://fallback_status", status);
        },
    )
    .await;

    Ok(res)
}

#[tauri::command]
fn clear_vod_cache(state: State<'_, AppState>) -> Result<(), String> {
    state.vega_runner.clear_module_cache();
    state.storage.clear_vod_cache().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state.storage.get_app_settings().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_app_settings(
    app: AppHandle,
    settings: AppSettings,
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    state
        .storage
        .save_app_settings(&settings)
        .map_err(|e| e.to_string())?;
    state
        .doh
        .update(settings.dns_mode, settings.custom_doh_url.clone());
    state
        .vega_runner
        .set_developer_test_mode(settings.developer_test_mode);
    let ff = if settings.ffmpeg_path.trim().is_empty() {
        None
    } else {
        Some(settings.ffmpeg_path.clone())
    };
    state.stream_proxy.set_ffmpeg_path(ff.clone());
    state.download_manager.set_ffmpeg_path(ff.clone());
    if settings.ffmpeg_enabled && !detect_ffmpeg_status(ff.as_deref()).installed {
        let h = app.clone();
        tauri::async_runtime::spawn(async move {
            let st = h.state::<AppState>();
            let _ = ensure_ffmpeg_ready_internal(&st, Some(&h), false).await;
        });
    }
    Ok(settings)
}

async fn ensure_ffmpeg_ready_internal(
    state: &AppState,
    app: Option<&AppHandle>,
    force_install: bool,
) -> Result<FfmpegStatus, String> {
    let _guard = state.ffmpeg_install_lock.lock().await;
    let settings = state.storage.get_app_settings().unwrap_or_default();
    let custom = if settings.ffmpeg_path.trim().is_empty() {
        None
    } else {
        Some(settings.ffmpeg_path.as_str())
    };
    let mut status = check_ffmpeg_status_with_upstream(&state.http_client, custom).await;

    let should_auto_install = !status.installed && (settings.ffmpeg_enabled || force_install);
    let should_auto_update = status.installed
        && (force_install || (status.source == "managed" && status.update_available && settings.ffmpeg_enabled));

    if should_auto_install || should_auto_update {
        if let Some(handle) = app {
            let mut installing_status = status.clone();
            installing_status.installing = true;
            let _ = handle.emit("ffmpeg://status", installing_status);
        }
        status = download_latest_ffmpeg(&state.http_client)
            .await
            .map_err(|e| {
                if let Some(handle) = app {
                    let _ = handle.emit("ffmpeg://status", status.clone());
                }
                e.to_string()
            })?;
        let ff = custom.map(str::to_string);
        state.stream_proxy.set_ffmpeg_path(ff.clone());
        state.download_manager.set_ffmpeg_path(ff);
    }

    if let Some(handle) = app {
        let _ = handle.emit("ffmpeg://status", status.clone());
    }
    Ok(status)
}

#[tauri::command]
fn get_ffmpeg_status(state: State<'_, AppState>) -> FfmpegStatus {
    let settings = state.storage.get_app_settings().unwrap_or_default();
    let custom = if settings.ffmpeg_path.trim().is_empty() {
        None
    } else {
        Some(settings.ffmpeg_path.as_str())
    };
    detect_ffmpeg_status(custom)
}

#[tauri::command]
async fn check_ffmpeg_upstream_status(state: State<'_, AppState>) -> Result<FfmpegStatus, String> {
    let settings = state.storage.get_app_settings().unwrap_or_default();
    let custom = if settings.ffmpeg_path.trim().is_empty() {
        None
    } else {
        Some(settings.ffmpeg_path.as_str())
    };
    Ok(check_ffmpeg_status_with_upstream(&state.http_client, custom).await)
}

#[tauri::command]
async fn install_ffmpeg_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FfmpegStatus, String> {
    ensure_ffmpeg_ready_internal(&state, Some(&app), true).await
}

#[tauri::command]
async fn probe_media_info(
    target: String,
    headers: Option<HashMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<MediaProbeResult, String> {
    let settings = state.storage.get_app_settings().unwrap_or_default();
    let custom = if settings.ffmpeg_path.trim().is_empty() {
        None
    } else {
        Some(settings.ffmpeg_path.as_str())
    };
    let status = detect_ffmpeg_status(custom);
    let Some(probe_exe) = status.ffprobe_path else {
        return Err("ffprobe is not installed. Install FFmpeg in Preferences to inspect tracks and codecs.".into());
    };
    probe_media(
        std::path::Path::new(&probe_exe),
        &target,
        &headers.unwrap_or_default(),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn test_dns(
    mode: DnsMode,
    custom_url: Option<String>,
    state: State<'_, AppState>,
) -> Result<DnsTestResult, String> {
    Ok(state.doh.test_dns(mode, custom_url.as_deref()).await)
}

#[tauri::command]
fn get_watch_history(state: State<'_, AppState>) -> Result<Vec<WatchHistoryEntry>, String> {
    state.storage.get_watch_history().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_watch_progress(
    entry: WatchHistoryEntry,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .storage
        .save_watch_progress(&entry)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_watch_history(state: State<'_, AppState>) -> Result<(), String> {
    state
        .storage
        .clear_watch_history()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_resume_position_ms(link: String, state: State<'_, AppState>) -> Result<i64, String> {
    state
        .storage
        .get_resume_position_ms(&link)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_watch_history_entry(link: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .storage
        .delete_watch_history_entry(&link)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn prepare_stream_proxy(
    stream_url: String,
    headers: Option<HashMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<ProxiedStream, String> {
    if std::path::Path::new(stream_url.trim()).is_file() {
        return state
            .stream_proxy
            .register_local_file(&stream_url)
            .map_err(|e| e.to_string());
    }
    state
        .stream_proxy
        .register_session(&stream_url, headers.unwrap_or_default())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn enqueue_download(
    app: AppHandle,
    title: String,
    download_url: String,
    headers: Option<HashMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<DownloadTask, String> {
    let settings = state.storage.get_app_settings().unwrap_or_default();
    let custom_dir = if settings.download_dir.trim().is_empty() {
        None
    } else {
        Some(settings.download_dir.as_str())
    };
    state
        .download_manager
        .enqueue(&title, &download_url, headers, custom_dir, move |task| {
            let _ = app.emit("downloads://progress", task);
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadTask>, String> {
    state.storage.list_downloads().map_err(|e| e.to_string())
}

#[tauri::command]
fn pause_download(id: String, state: State<'_, AppState>) -> Result<Vec<DownloadTask>, String> {
    state
        .download_manager
        .pause_or_cancel(&id, true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn resume_download(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<DownloadTask>, String> {
    state
        .download_manager
        .resume(&id, move |task| {
            let _ = app.emit("downloads://progress", task);
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_download(
    id: String,
    remove_file: bool,
    state: State<'_, AppState>,
) -> Result<Vec<DownloadTask>, String> {
    state
        .download_manager
        .delete(&id, remove_file)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_download_folder(path: String) -> Result<(), String> {
    open_containing_folder(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_default_download_dir(state: State<'_, AppState>) -> String {
    let settings = state.storage.get_app_settings().unwrap_or_default();
    let custom = if settings.download_dir.trim().is_empty() {
        None
    } else {
        Some(settings.download_dir.as_str())
    };
    resolve_download_dir(custom).to_string_lossy().into_owned()
}

#[tauri::command]
async fn check_for_updates(state: State<'_, AppState>) -> Result<UpdateCheckResult, String> {
    check_latest_release(
        &state.http_client,
        "samyak2403",
        "IPTVMine-Pro",
        env!("CARGO_PKG_VERSION"),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_diagnostics_bundle(state: State<'_, AppState>) -> Result<String, String> {
    let settings = state.storage.get_app_settings().unwrap_or_default();
    let providers = state.storage.list_providers().unwrap_or_default();
    let extensions = state.storage.get_installed_extensions().unwrap_or_default();
    let downloads = state.storage.list_downloads().unwrap_or_default();
    let history = state.storage.get_watch_history().unwrap_or_default();
    let channels_len = state.cached_channels.read().map(|g| g.len()).unwrap_or(0);

    // Strictly redacted diagnostic summary (zero URLs, headers, or cookies)
    Ok(format!(
        "IPTVMine Desktop Diagnostics\n\
         Version: {}\n\
         OS: {} ({})\n\
         DNS Mode: {:?}\n\
         Theme: {}\n\
         Developer Test Mode: {}\n\
         Providers Configured: {} (Active: {})\n\
         Loaded Channels: {}\n\
         Installed Extensions: {}\n\
         Downloads: {}\n\
         Watch History Entries: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        settings.dns_mode,
        if settings.black_theme { "AMOLED Black" } else { "Dark" },
        settings.developer_test_mode,
        providers.len(),
        providers.iter().filter(|p| p.is_active).count(),
        channels_len,
        extensions.len(),
        downloads.len(),
        history.len(),
    ))
}

#[tauri::command]
fn launch_external_player(
    stream_url: String,
    title: String,
    headers: Option<HashMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let settings = state.storage.get_app_settings().unwrap_or_default();
    let custom_path = if settings.external_player_path.trim().is_empty() {
        None
    } else {
        Some(settings.external_player_path.as_str())
    };
    spawn_external_player(&stream_url, &title, &headers.unwrap_or_default(), custom_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_always_on_top(window: WebviewWindow, enabled: bool) -> Result<(), String> {
    window.set_always_on_top(enabled).map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn show_fatal_error(title: &str, message: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            hwnd: *mut core::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            u_type: u32,
        ) -> i32;
    }
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let caption: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    // MB_OK (0x0) | MB_ICONERROR (0x10)
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
    }
}

#[cfg(not(target_os = "windows"))]
fn show_fatal_error(_title: &str, _message: &str) {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        let log_path = std::env::temp_dir().join("iptvmine-panic.log");
        let _ = std::fs::write(&log_path, format!("{info}\n"));
        eprintln!("PANIC: {info}");
    }));
    let _ = tracing_subscriber::fmt().try_init();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let db_path = data_dir.join("iptvmine.db");
            let storage = Arc::new(Storage::open(&db_path)?);
            let settings = storage.get_app_settings().unwrap_or_default();
            let doh = DohState::new(settings.dns_mode, settings.custom_doh_url.clone());
            let http_client = doh.build_http_client();
            let vega_runner = VegaRunner::new(
                storage.clone(),
                http_client.clone(),
                settings.developer_test_mode,
            );
            let stream_proxy =
                tauri::async_runtime::block_on(StreamProxy::start(http_client.clone()));
            let custom_dl_dir = if settings.download_dir.trim().is_empty() {
                None
            } else {
                Some(settings.download_dir.as_str())
            };
            let download_manager =
                DownloadManager::new(storage.clone(), http_client.clone(), custom_dl_dir);
            let custom_ff = if settings.ffmpeg_path.trim().is_empty() {
                None
            } else {
                Some(settings.ffmpeg_path.clone())
            };
            stream_proxy.set_ffmpeg_path(custom_ff.clone());
            download_manager.set_ffmpeg_path(custom_ff);

            app.manage(AppState {
                storage,
                doh,
                http_client,
                channel_engine: ChannelEngine::new(),
                vega_runner,
                stream_proxy,
                download_manager,
                data_dir,
                cached_channels: RwLock::new(Vec::new()),
                content_filter: ContentFilter::new(),
                ffmpeg_install_lock: tokio::sync::Mutex::new(()),
            });

            let handle_for_ff = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle_for_ff.state::<AppState>();
                let _ = ensure_ffmpeg_ready_internal(&state, Some(&handle_for_ff), false).await;
            });

            let handle_for_dl = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle_for_dl.state::<AppState>();
                let h = handle_for_dl.clone();
                state.download_manager.resume_pending_on_startup(move |task| {
                    let _ = h.emit("downloads://progress", task);
                });
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_providers,
            add_provider,
            update_provider,
            remove_provider,
            import_local_playlist,
            detect_provider_type,
            inspect_provider,
            load_channels,
            get_channel_categories,
            list_vega_extensions,
            set_extension_installed,
            get_vod_catalog,
            get_vod_posts,
            search_vod_posts,
            get_vod_meta,
            get_vod_episodes,
            get_detail_enrichment,
            resolve_vod_stream_with_fallback,
            clear_vod_cache,
            get_app_settings,
            save_app_settings,
            test_dns,
            get_watch_history,
            save_watch_progress,
            clear_watch_history,
            get_resume_position_ms,
            delete_watch_history_entry,
            prepare_stream_proxy,
            get_ffmpeg_status,
            check_ffmpeg_upstream_status,
            install_ffmpeg_runtime,
            probe_media_info,
            enqueue_download,
            list_downloads,
            pause_download,
            resume_download,
            delete_download,
            open_download_folder,
            get_default_download_dir,
            check_for_updates,
            get_diagnostics_bundle,
            launch_external_player,
            set_always_on_top,
        ])
        .run(tauri::generate_context!());

    if let Err(err) = result {
        let msg = format!("IPTVMine Desktop failed to start:\n{err}");
        let log_path = std::env::temp_dir().join("iptvmine-panic.log");
        let _ = std::fs::write(&log_path, format!("{msg}\n"));
        eprintln!("{msg}");
        show_fatal_error("IPTVMine Desktop Error", &msg);
    }
}
