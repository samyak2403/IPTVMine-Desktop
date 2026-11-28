import type {
  AppSettings,
  Channel,
  DetectionResult,
  DnsMode,
  DnsTestResult,
  FfmpegStatus,
  LiveFallbackResult,
  Provider,
  ProviderType,
  ProxiedStream,
  UpdateCheckResult,
  WatchHistoryEntry,
} from "./types";

const STORAGE_PROVIDERS = "iptvmine.tv.providers";
const STORAGE_SETTINGS = "iptvmine.tv.settings";
const STORAGE_HISTORY = "iptvmine.tv.history";
const STORAGE_LOCAL_PREFIX = "iptvmine.tv.local:";

const VIDEO_EXTS = [
  ".mp4",
  ".mkv",
  ".avi",
  ".mov",
  ".wmv",
  ".flv",
  ".webm",
  ".mpg",
  ".mpeg",
  ".3gp",
  ".ogv",
  ".m4v",
];

let cachedChannels: Channel[] = [];

const DEFAULT_TV_SETTINGS: AppSettings = {
  blackTheme: true,
  startupTab: "home",
  dnsMode: "off",
  customDohUrl: "",
  developerTestMode: false,
  externalPlayerPath: "",
  downloadDir: "",
  ffmpegEnabled: false,
  ffmpegPath: "",
};

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Ignore storage quota errors
  }
}

function detectProviderTypeQuick(inputUrl: string, title = ""): ProviderType {
  const trimmed = inputUrl.trim();
  const path = trimmed.toLowerCase().split(/[?#]/)[0] ?? "";
  if (VIDEO_EXTS.some((ext) => path.endsWith(ext))) return "VIDEO";
  if (path.endsWith(".m3u") || path.endsWith(".m3u8")) return "IPTV";
  if (
    trimmed.startsWith("@") ||
    title.toLowerCase().includes("manifest") ||
    path.endsWith("/manifest.json") ||
    path.endsWith(".json") ||
    ((path.startsWith("https://github.com/") || path.startsWith("http://github.com/")) &&
      !path.endsWith(".m3u"))
  ) {
    return "VEGA";
  }
  return "IPTV";
}

function deriveTitle(rawUrl: string): string {
  const trimmed = rawUrl.trim().replace(/\/+$/, "");
  if (trimmed.startsWith("@")) return trimmed.slice(1) || "Provider";
  try {
    const u = new URL(trimmed);
    return u.hostname.replace(/^www\./, "") || "Provider";
  } catch {
    return trimmed || "Provider";
  }
}

function detectStreamType(url: string): string {
  const lower = url.toLowerCase();
  if (lower.includes(".m3u8")) return "HLS";
  if (lower.includes(".mpd")) return "DASH";
  if (lower.startsWith("rtmp://") || lower.startsWith("rtmps://")) return "RTMP";
  return "HTTP";
}

function extractAttr(attrs: string, key: string): string {
  const re = new RegExp(`(?:^|[\\s:,])${key}\\s*=\\s*(?:"([^"]*)"|'([^']*)')`, "i");
  const match = attrs.match(re);
  return (match?.[1] ?? match?.[2] ?? "").trim();
}

function parseM3uText(raw: string): Channel[] {
  const trimmed = raw.replace(/^\uFEFF/, "").trim();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      const parsed = JSON.parse(trimmed) as unknown;
      const fromJson = collectJsonChannels(parsed, "Uncategorized");
      if (fromJson.length > 0) return fromJson;
    } catch {
      // Fall back to M3U parser
    }
  }

  const out: Channel[] = [];
  let pendingName = "";
  let pendingLogo = "";
  let pendingCategory = "Uncategorized";
  let pendingHeaders: Record<string, string> = {};
  let hasExtinf = false;

  for (const rawLine of trimmed.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#EXTM3U")) continue;
    if (line.startsWith("#EXTINF:")) {
      hasExtinf = true;
      pendingHeaders = {};
      const commaIdx = line.lastIndexOf(",");
      const attrs = commaIdx >= 0 ? line.slice(0, commaIdx) : line;
      const rawTitle = commaIdx >= 0 ? line.slice(commaIdx + 1).trim() : "";
      pendingLogo =
        extractAttr(attrs, "tvg-logo") ||
        extractAttr(attrs, "logo") ||
        extractAttr(attrs, "tvg-icon");
      pendingCategory = extractAttr(attrs, "group-title") || "Uncategorized";
      pendingName = rawTitle || extractAttr(attrs, "tvg-name") || "";
    } else if (line.startsWith("#EXTVLCOPT:")) {
      const rest = line.slice("#EXTVLCOPT:".length);
      if (rest.startsWith("http-user-agent=")) {
        pendingHeaders["User-Agent"] = rest.slice("http-user-agent=".length).trim();
      } else if (rest.startsWith("http-referrer=")) {
        pendingHeaders["Referer"] = rest.slice("http-referrer=".length).trim();
      }
    } else if (!line.startsWith("#") && /^(https?|rtmps?|rtsp|udp|rtp):\/\//i.test(line)) {
      out.push({
        name: hasExtinf && pendingName ? pendingName : `Channel ${out.length + 1}`,
        logoUrl: pendingLogo,
        streamUrl: line,
        streamType: detectStreamType(line),
        category: pendingCategory,
        team1: "",
        team2: "",
        status: "LIVE",
        startTime: "",
        headers: { ...pendingHeaders },
      });
      hasExtinf = false;
      pendingName = "";
      pendingLogo = "";
      pendingCategory = "Uncategorized";
      pendingHeaders = {};
    }
  }
  return out;
}

function collectJsonChannels(node: unknown, inheritedCat: string): Channel[] {
  const out: Channel[] = [];
  if (Array.isArray(node)) {
    for (const item of node) out.push(...collectJsonChannels(item, inheritedCat));
    return out;
  }
  if (!node || typeof node !== "object") return out;

  const obj = node as Record<string, unknown>;
  const urlKey = ["url", "link", "streamUrl", "stream_url", "stream", "hls", "m3u8"].find(
    (k) => typeof obj[k] === "string" && /^(https?|rtmps?|rtsp):\/\//i.test(String(obj[k]).trim())
  );
  if (urlKey) {
    const streamUrl = String(obj[urlKey]).trim();
    const nameKey = ["name", "title", "channelName", "channel", "tvg-name"].find(
      (k) => typeof obj[k] === "string" && String(obj[k]).trim().length > 0
    );
    const logoKey = ["logoUrl", "logo", "tvg-logo", "icon", "image"].find(
      (k) => typeof obj[k] === "string" && String(obj[k]).trim().length > 0
    );
    const catKey = ["category", "group-title", "group", "genre"].find(
      (k) => typeof obj[k] === "string" && String(obj[k]).trim().length > 0
    );
    out.push({
      name: nameKey ? String(obj[nameKey]).trim() : `Channel ${out.length + 1}`,
      logoUrl: logoKey ? String(obj[logoKey]).trim() : "",
      streamUrl,
      streamType: detectStreamType(streamUrl),
      category: catKey ? String(obj[catKey]).trim() : inheritedCat,
      team1: "",
      team2: "",
      status: "LIVE",
      startTime: "",
      headers: {},
    });
    return out;
  }

  for (const [k, v] of Object.entries(obj)) {
    const container = ["channels", "streams", "data", "items", "list", "results"].includes(
      k.toLowerCase()
    );
    out.push(...collectJsonChannels(v, container ? inheritedCat : k));
  }
  return out;
}

function saveProviderEntry(providers: Provider[], item: Provider, oldUrl?: string | null): Provider[] {
  const normalized: Provider = {
    ...item,
    url: item.url.trim(),
    title: item.title.trim() || deriveTitle(item.url),
  };
  const next = oldUrl
    ? providers.map((p) => (p.url === oldUrl ? normalized : p))
    : providers.some((p) => p.url === normalized.url)
      ? providers.map((p) => (p.url === normalized.url ? normalized : p))
      : [normalized, ...providers];
  writeJson(STORAGE_PROVIDERS, next);
  return next;
}

function defaultFfmpegTvStatus(): FfmpegStatus {
  return {
    installed: false,
    installing: false,
    version: null,
    shortVersion: null,
    ffmpegPath: null,
    ffprobePath: null,
    source: "tv-web",
    upstreamTag: null,
    upstreamVersion: null,
    updateAvailable: false,
    upstreamUrl: "https://github.com/FFmpeg/FFmpeg",
  };
}

/**
 * Provides a standalone browser / Smart TV (webOS & Tizen) implementation when Tauri IPC is absent.
 */
export async function invokeTvFallback<T>(
  cmd: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  switch (cmd) {
    case "list_providers":
      return readJson<Provider[]>(STORAGE_PROVIDERS, []) as T;

    case "add_provider": {
      const prov = args.provider as Provider;
      if (!prov.url.trim()) throw new Error("Provider URL cannot be empty");
      return saveProviderEntry(readJson<Provider[]>(STORAGE_PROVIDERS, []), prov) as T;
    }

    case "update_provider":
      return saveProviderEntry(
        readJson<Provider[]>(STORAGE_PROVIDERS, []),
        args.provider as Provider,
        (args.oldUrl as string | null | undefined) ?? null
      ) as T;

    case "remove_provider": {
      const targetUrl = String(args.url ?? "");
      if (targetUrl.startsWith("local://")) {
        window.localStorage.removeItem(`${STORAGE_LOCAL_PREFIX}${targetUrl}`);
      }
      const next = readJson<Provider[]>(STORAGE_PROVIDERS, []).filter((p) => p.url !== targetUrl);
      writeJson(STORAGE_PROVIDERS, next);
      return next as T;
    }

    case "import_local_playlist": {
      const content = String(args.content ?? "");
      if (!content.trim()) throw new Error("Imported file is empty");
      const rawTitle = String(args.title ?? "").trim() || "local_playlist";
      const keyUrl = `local://${Date.now()}_${rawTitle.replace(/[^\w.-]+/g, "_")}.m3u`;
      window.localStorage.setItem(`${STORAGE_LOCAL_PREFIX}${keyUrl}`, content);
      const parsed = parseM3uText(content);
      return saveProviderEntry(readJson<Provider[]>(STORAGE_PROVIDERS, []), {
        title: rawTitle,
        url: keyUrl,
        userAgent: null,
        isActive: true,
        channelCount: parsed.length,
        type: "IPTV",
      }) as T;
    }

    case "detect_provider_type":
      return detectProviderTypeQuick(String(args.url ?? ""), String(args.title ?? "")) as T;

    case "inspect_provider": {
      const url = String(args.url ?? "");
      const providerType = detectProviderTypeQuick(url, "");
      const sourceKind =
        providerType === "VIDEO"
          ? "DIRECT_VIDEO"
          : providerType === "VEGA"
            ? "VOD_MANIFEST"
            : "IPTV_M3U";
      return { providerType, sourceKind } satisfies DetectionResult as T;
    }

    case "load_channels": {
      const providers = readJson<Provider[]>(STORAGE_PROVIDERS, []);
      const activeIptv = providers.filter((p) => p.isActive && p.type === "IPTV");
      const all: Channel[] = [];
      const errors: string[] = [];
      const updated = [...providers];

      for (const p of activeIptv) {
        try {
          const text = p.url.startsWith("local://")
            ? window.localStorage.getItem(`${STORAGE_LOCAL_PREFIX}${p.url}`) ?? ""
            : await fetch(p.url).then((r) => {
                if (!r.ok) throw new Error(`HTTP ${r.status}`);
                return r.text();
              });
          const parsed = parseM3uText(text);
          all.push(...parsed);
          const idx = updated.findIndex((item) => item.url === p.url);
          if (idx >= 0) updated[idx] = { ...updated[idx], channelCount: parsed.length };
        } catch (err) {
          errors.push(`${p.title}: ${String(err)}`);
        }
      }
      writeJson(STORAGE_PROVIDERS, updated);
      cachedChannels = all;
      if (all.length === 0 && errors.length > 0) {
        throw new Error(errors.join("; "));
      }
      return all as T;
    }

    case "get_channel_categories":
      return Array.from(new Set(cachedChannels.map((c) => c.category))).sort() as T;

    case "get_app_settings":
      return readJson<AppSettings>(STORAGE_SETTINGS, DEFAULT_TV_SETTINGS) as T;

    case "save_app_settings": {
      const next = { ...DEFAULT_TV_SETTINGS, ...(args.settings as AppSettings) };
      writeJson(STORAGE_SETTINGS, next);
      return next as T;
    }

    case "get_watch_history":
      return readJson<WatchHistoryEntry[]>(STORAGE_HISTORY, []).sort(
        (a, b) => b.lastWatchedMs - a.lastWatchedMs
      ) as T;

    case "save_watch_progress": {
      const entry = args.entry as WatchHistoryEntry;
      const list = readJson<WatchHistoryEntry[]>(STORAGE_HISTORY, []).filter(
        (h) => h.link !== entry.link
      );
      list.unshift(entry);
      writeJson(STORAGE_HISTORY, list.slice(0, 100));
      return undefined as T;
    }

    case "get_resume_position_ms": {
      const link = String(args.link ?? "");
      const found = readJson<WatchHistoryEntry[]>(STORAGE_HISTORY, []).find(
        (h) => h.link === link
      );
      return (found?.positionMs ?? 0) as T;
    }

    case "delete_watch_history_entry": {
      const link = String(args.link ?? "");
      writeJson(
        STORAGE_HISTORY,
        readJson<WatchHistoryEntry[]>(STORAGE_HISTORY, []).filter((h) => h.link !== link)
      );
      return undefined as T;
    }

    case "clear_watch_history":
      writeJson(STORAGE_HISTORY, []);
      return undefined as T;

    case "prepare_stream_proxy": {
      const streamUrl = String(args.streamUrl ?? "");
      return {
        proxyUrl: streamUrl,
        transcodeBaseUrl: "",
        subtitleBaseUrl: "",
        sessionId: "tv-direct",
      } satisfies ProxiedStream as T;
    }

    case "resolve_live_channel_with_fallback": {
      const ch = args.channel as Channel;
      return {
        selectedStream: {
          server: "Direct",
          link: ch.streamUrl,
          type: ch.streamType,
          quality: "",
          headers: ch.headers,
        },
        allStreams: [
          {
            server: "Direct",
            link: ch.streamUrl,
            type: ch.streamType,
            quality: "",
            headers: ch.headers,
          },
        ],
        hiddenDeadCount: 0,
        isFallback: false,
      } satisfies LiveFallbackResult as T;
    }

    case "get_ffmpeg_status":
    case "check_ffmpeg_upstream_status":
      return defaultFfmpegTvStatus() as T;

    case "list_downloads":
      return [] as T;

    case "get_default_download_dir":
      return "" as T;

    case "set_always_on_top":
    case "clear_vod_cache":
      return undefined as T;

    case "test_dns": {
      const mode = (args.mode as DnsMode) ?? "cloudflare";
      const start = performance.now();
      const target =
        mode === "google"
          ? "https://dns.google/resolve?name=example.com&type=A"
          : mode === "adguard"
            ? "https://dns.adguard-dns.com/resolve?name=example.com&type=A"
            : mode === "custom"
              ? String(args.customUrl ?? "").trim()
              : "https://cloudflare-dns.com/dns-query?name=example.com&type=A";
      if (mode === "off" || !target) {
        return { success: mode === "off", latencyMs: 0 } satisfies DnsTestResult as T;
      }
      const resp = await fetch(target, { headers: { Accept: "application/dns-json" } });
      return {
        success: resp.ok,
        latencyMs: Math.round(performance.now() - start),
      } satisfies DnsTestResult as T;
    }

    case "check_for_updates": {
      const resp = await fetch(
        "https://api.github.com/repos/samyak2403/IPTVMine-Desktop/releases/latest",
        { headers: { Accept: "application/vnd.github.v3+json" } }
      );
      if (!resp.ok) throw new Error(`GitHub API returned status ${resp.status}`);
      const rel = (await resp.json()) as {
        tag_name?: string;
        name?: string;
        body?: string;
        html_url?: string;
      };
      const latest = String(rel.tag_name ?? "0.1.0").replace(/^v/i, "");
      return {
        currentVersion: "0.1.0",
        latestVersion: latest,
        updateAvailable: latest !== "0.1.0",
        releaseName: rel.name ?? rel.tag_name ?? "",
        releaseNotes: rel.body ?? "",
        releaseUrl: rel.html_url ?? "",
        downloadUrl: rel.html_url ?? null,
        assetName: null,
      } satisfies UpdateCheckResult as T;
    }

    case "get_diagnostics_bundle": {
      const providers = readJson<Provider[]>(STORAGE_PROVIDERS, []);
      const history = readJson<WatchHistoryEntry[]>(STORAGE_HISTORY, []);
      const settings = readJson<AppSettings>(STORAGE_SETTINGS, DEFAULT_TV_SETTINGS);
      return [
        "IPTVMine Smart TV / Web Diagnostics",
        "Version: 0.1.0",
        `User-Agent: ${navigator.userAgent}`,
        `Theme: ${settings.blackTheme ? "AMOLED Black" : "Dark"}`,
        `Providers Configured: ${providers.length} (Active: ${providers.filter((p) => p.isActive).length})`,
        `Loaded Channels: ${cachedChannels.length}`,
        `Watch History Entries: ${history.length}`,
      ].join("\n") as T;
    }

    default:
      throw new Error(
        `"${cmd}" requires the native IPTVMine Desktop backend (not available in Smart TV / Web mode).`
      );
  }
}
