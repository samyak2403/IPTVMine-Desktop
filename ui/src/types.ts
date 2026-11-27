export type ProviderType = "IPTV" | "VEGA" | "VIDEO";

export interface Provider {
  title: string;
  url: string;
  userAgent?: string | null;
  isActive: boolean;
  channelCount: number;
  type: ProviderType;
}

export interface Channel {
  name: string;
  logoUrl: string;
  streamUrl: string;
  streamType: string;
  category: string;
  team1: string;
  team2: string;
  status: string;
  startTime: string;
  headers: Record<string, string>;
}

export type DnsMode = "cloudflare" | "google" | "adguard" | "custom" | "off";
export type StartupTab = "home" | "television" | "movies" | "category";

export interface AppSettings {
  blackTheme: boolean;
  startupTab: StartupTab;
  dnsMode: DnsMode;
  customDohUrl: string;
  developerTestMode: boolean;
  externalPlayerPath: string;
  downloadDir: string;
  ffmpegEnabled: boolean;
  ffmpegPath: string;
}

export interface FfmpegStatus {
  installed: boolean;
  installing?: boolean;
  version?: string | null;
  shortVersion?: string | null;
  ffmpegPath?: string | null;
  ffprobePath?: string | null;
  source: string;
  upstreamTag?: string | null;
  upstreamVersion?: string | null;
  updateAvailable: boolean;
  upstreamUrl: string;
}

export interface MediaAudioTrack {
  index: number;
  streamIndex: number;
  codec: string;
  language: string;
  title: string;
  channels: number;
}

export interface MediaSubtitleTrack {
  index: number;
  streamIndex: number;
  codec: string;
  language: string;
  title: string;
  textBased: boolean;
}

export interface MediaProbeResult {
  formatName: string;
  durationSec?: number | null;
  bitRate?: number | null;
  sizeBytes?: number | null;
  videoCodec?: string | null;
  width?: number | null;
  height?: number | null;
  needsTranscode: boolean;
  needsReason?: string | null;
  audioTracks: MediaAudioTrack[];
  subtitleTracks: MediaSubtitleTrack[];
}

export interface DnsTestResult {
  success: boolean;
  latencyMs: number;
}

export interface WatchHistoryEntry {
  title: string;
  link: string;
  streamUrl: string;
  imageUrl: string;
  providerUrl: string;
  scraperValue: string;
  positionMs: number;
  durationMs: number;
  lastWatchedMs: number;
}

export interface DetectionResult {
  providerType: ProviderType;
  sourceKind: string;
}

export interface VegaProvider {
  display_name: string;
  value: string;
  version: string;
  icon: string;
  type: string;
  disabled: boolean;
  is_installed: boolean;
}

export interface VegaCatalog {
  title: string;
  filter: string;
}

export interface CatalogResponse {
  catalog: VegaCatalog[];
  genres: VegaCatalog[];
}

export interface VegaPost {
  title: string;
  link: string;
  image: string;
}

export interface VegaDirectLink {
  title: string;
  link: string;
  type: string;
  image?: string | null;
  duration?: string | null;
  date?: string | null;
  episodeNumber?: string | null;
  seasonNumber?: string | null;
  size?: string | null;
  quality?: string | null;
  rating?: string | null;
  overview?: string | null;
}

export interface VegaLink {
  title: string;
  episodesLink?: string | null;
  directLinks?: VegaDirectLink[] | null;
  image?: string | null;
  duration?: string | null;
  date?: string | null;
  episodeNumber?: string | null;
  seasonNumber?: string | null;
  size?: string | null;
  quality?: string | null;
  rating?: string | null;
  overview?: string | null;
}

export interface VegaMeta {
  title: string;
  synopsis: string;
  image: string;
  imdbId: string;
  type: string;
  linkList: VegaLink[];
  year?: string | null;
  rating?: string | null;
  runtime?: string | null;
  genres?: string[];
  background?: string | null;
}

export interface EpisodeEnrichment {
  season?: number | null;
  episode?: number | null;
  title?: string | null;
  image?: string | null;
  overview?: string | null;
  duration?: string | null;
  date?: string | null;
  rating?: string | null;
}

export interface DetailEnrichment {
  imdbId?: string | null;
  year?: string | null;
  rating?: string | null;
  runtime?: string | null;
  genres: string[];
  background?: string | null;
  episodes: EpisodeEnrichment[];
}

export interface VegaStream {
  server: string;
  link: string;
  type: string;
  quality: string;
  size?: string | null;
  headers?: Record<string, string> | null;
}

export interface FallbackResult {
  selectedStream: VegaStream;
  allStreams: VegaStream[];
  serverTitle: string;
  providerValue: string;
  isFallback: boolean;
  message?: string | null;
}

export interface LiveFallbackResult {
  selectedStream?: VegaStream | null;
  allStreams: VegaStream[];
  hiddenDeadCount: number;
  isFallback: boolean;
  message?: string | null;
  error?: string | null;
}

export interface ProxiedStream {
  proxyUrl: string;
  transcodeBaseUrl: string;
  subtitleBaseUrl: string;
  sessionId: string;
}

export interface EpisodeItem {
  title: string;
  link: string;
  type: string;
}

export interface PlaybackRequest {
  title: string;
  link: string;
  streamUrl: string;
  headers?: Record<string, string>;
  imageUrl?: string;
  providerUrl?: string;
  scraperValue?: string;
  isLive?: boolean;
  channel?: Channel;
  mirrors?: VegaStream[];
  episodes?: EpisodeItem[];
  currentEpisodeIndex?: number;
}

export type DownloadStatus =
  | "PENDING"
  | "DOWNLOADING"
  | "PAUSED"
  | "COMPLETED"
  | "FAILED";

export interface DownloadTask {
  id: string;
  title: string;
  downloadUrl: string;
  headers?: Record<string, string> | null;
  savePath: string;
  progress: number;
  speed: string;
  status: DownloadStatus;
  totalBytes: number;
  downloadedBytes: number;
  addedAtMs: number;
}

export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  releaseName: string;
  releaseNotes: string;
  releaseUrl: string;
  downloadUrl?: string | null;
  assetName?: string | null;
}
