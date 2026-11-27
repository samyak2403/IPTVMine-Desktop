import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export function isTauriRuntime(): boolean {
  const w = window as unknown as {
    __TAURI_INTERNALS__?: { invoke?: unknown };
    __TAURI__?: { core?: { invoke?: unknown } };
  };
  return Boolean(w.__TAURI_INTERNALS__?.invoke || w.__TAURI__?.core?.invoke);
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    throw new Error(
      "Desktop backend is unavailable in a browser tab. Please use the IPTVMine Desktop application window."
    );
  }
  return tauriInvoke<T>(cmd, args);
}
import type {
  AppSettings,
  CatalogResponse,
  Channel,
  DetailEnrichment,
  DetectionResult,
  DnsMode,
  DnsTestResult,
  DownloadTask,
  FallbackResult,
  FfmpegStatus,
  MediaProbeResult,
  Provider,
  ProviderType,
  ProxiedStream,
  UpdateCheckResult,
  VegaLink,
  VegaMeta,
  VegaPost,
  VegaProvider,
  WatchHistoryEntry,
} from "./types";

export const api = {
  listProviders: () => invoke<Provider[]>("list_providers"),
  addProvider: (provider: Provider) =>
    invoke<Provider[]>("add_provider", { provider }),
  updateProvider: (oldUrl: string | null, provider: Provider) =>
    invoke<Provider[]>("update_provider", { oldUrl, provider }),
  removeProvider: (url: string) =>
    invoke<Provider[]>("remove_provider", { url }),
  importLocalPlaylist: (title: string, content: string) =>
    invoke<Provider[]>("import_local_playlist", { title, content }),
  detectProviderType: (url: string, title: string) =>
    invoke<ProviderType>("detect_provider_type", { url, title }),
  inspectProvider: (url: string) =>
    invoke<DetectionResult>("inspect_provider", { url }),
  loadChannels: (forceRefresh = false) =>
    invoke<Channel[]>("load_channels", { forceRefresh }),
  getChannelCategories: () => invoke<string[]>("get_channel_categories"),
  listVegaExtensions: (providerUrl?: string, forceRefresh = false) =>
    invoke<VegaProvider[]>("list_vega_extensions", {
      providerUrl: providerUrl ?? null,
      forceRefresh,
    }),
  setExtensionInstalled: (value: string, installed: boolean) =>
    invoke<string[]>("set_extension_installed", { value, installed }),
  getVodCatalog: (
    scraperValue: string,
    providerUrl?: string,
    forceRefresh = false
  ) =>
    invoke<CatalogResponse>("get_vod_catalog", {
      providerUrl: providerUrl ?? null,
      scraperValue,
      forceRefresh,
    }),
  getVodPosts: (
    scraperValue: string,
    filter: string,
    page: number,
    providerUrl?: string,
    forceRefresh = false
  ) =>
    invoke<VegaPost[]>("get_vod_posts", {
      providerUrl: providerUrl ?? null,
      scraperValue,
      filter,
      page,
      forceRefresh,
    }),
  searchVodPosts: (
    scraperValue: string,
    query: string,
    page: number,
    providerUrl?: string
  ) =>
    invoke<VegaPost[]>("search_vod_posts", {
      providerUrl: providerUrl ?? null,
      scraperValue,
      query,
      page,
    }),
  getVodMeta: (scraperValue: string, link: string, providerUrl?: string) =>
    invoke<VegaMeta>("get_vod_meta", {
      providerUrl: providerUrl ?? null,
      scraperValue,
      link,
    }),
  getVodEpisodes: (scraperValue: string, url: string, providerUrl?: string) =>
    invoke<VegaLink[]>("get_vod_episodes", {
      providerUrl: providerUrl ?? null,
      scraperValue,
      url,
    }),
  getDetailEnrichment: (imdbId: string, title: string, mediaType: string) =>
    invoke<DetailEnrichment>("get_detail_enrichment", {
      imdbId,
      title,
      mediaType,
    }),
  resolveVodStreamWithFallback: (params: {
    providerUrl?: string;
    scraperValue: string;
    initialLink: string;
    initialType: string;
    itemTitle: string;
    movieTitle: string;
    currentMeta?: VegaMeta | null;
  }) =>
    invoke<FallbackResult | null>("resolve_vod_stream_with_fallback", {
      providerUrl: params.providerUrl ?? null,
      scraperValue: params.scraperValue,
      initialLink: params.initialLink,
      initialType: params.initialType,
      itemTitle: params.itemTitle,
      movieTitle: params.movieTitle,
      currentMeta: params.currentMeta ?? null,
    }),
  clearVodCache: () => invoke<void>("clear_vod_cache"),
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  saveAppSettings: (settings: AppSettings) =>
    invoke<AppSettings>("save_app_settings", { settings }),
  testDns: (mode: DnsMode, customUrl?: string) =>
    invoke<DnsTestResult>("test_dns", { mode, customUrl }),
  getWatchHistory: () => invoke<WatchHistoryEntry[]>("get_watch_history"),
  saveWatchProgress: (entry: WatchHistoryEntry) =>
    invoke<void>("save_watch_progress", { entry }),
  getResumePositionMs: (link: string) =>
    invoke<number>("get_resume_position_ms", { link }),
  deleteWatchHistoryEntry: (link: string) =>
    invoke<void>("delete_watch_history_entry", { link }),
  clearWatchHistory: () => invoke<void>("clear_watch_history"),
  prepareStreamProxy: (streamUrl: string, headers?: Record<string, string>) =>
    invoke<ProxiedStream>("prepare_stream_proxy", {
      streamUrl,
      headers: headers ?? null,
    }),
  getFfmpegStatus: () => invoke<FfmpegStatus>("get_ffmpeg_status"),
  checkFfmpegUpstreamStatus: () => invoke<FfmpegStatus>("check_ffmpeg_upstream_status"),
  installFfmpegRuntime: () => invoke<FfmpegStatus>("install_ffmpeg_runtime"),
  probeMediaInfo: (target: string, headers?: Record<string, string>) =>
    invoke<MediaProbeResult>("probe_media_info", {
      target,
      headers: headers ?? null,
    }),
  launchExternalPlayer: (
    streamUrl: string,
    title: string,
    headers?: Record<string, string>
  ) =>
    invoke<string>("launch_external_player", {
      streamUrl,
      title,
      headers: headers ?? null,
    }),
  setAlwaysOnTop: (enabled: boolean) =>
    invoke<void>("set_always_on_top", { enabled }),
  enqueueDownload: (
    title: string,
    downloadUrl: string,
    headers?: Record<string, string>
  ) =>
    invoke<DownloadTask>("enqueue_download", {
      title,
      downloadUrl,
      headers: headers ?? null,
    }),
  listDownloads: () => invoke<DownloadTask[]>("list_downloads"),
  pauseDownload: (id: string) =>
    invoke<DownloadTask[]>("pause_download", { id }),
  resumeDownload: (id: string) =>
    invoke<DownloadTask[]>("resume_download", { id }),
  deleteDownload: (id: string, removeFile: boolean) =>
    invoke<DownloadTask[]>("delete_download", { id, removeFile }),
  openDownloadFolder: (path: string) =>
    invoke<void>("open_download_folder", { path }),
  getDefaultDownloadDir: () => invoke<string>("get_default_download_dir"),
  checkForUpdates: () => invoke<UpdateCheckResult>("check_for_updates"),
  getDiagnosticsBundle: () => invoke<string>("get_diagnostics_bundle"),
};
