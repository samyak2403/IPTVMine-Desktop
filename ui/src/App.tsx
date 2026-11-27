import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Play, RefreshCw } from "lucide-react";
import { api, isTauriRuntime } from "./api";
import { ChannelLogo } from "./components/ChannelLogo";
import { formatOttTitle, HeroMarquee, MidBanner, OttCarousel, type OttCardItem } from "./components/OttUi";
import { DownloadsView } from "./screens/DownloadsView";
import { DnsSettingsView } from "./screens/DnsSettingsView";
import { ExtensionsView } from "./screens/ExtensionsView";
import { MovieDetailView } from "./screens/MovieDetailView";
import { MoviesView } from "./screens/MoviesView";
import { PlayerView } from "./screens/PlayerView";
import { PreferencesView } from "./screens/PreferencesView";
import { ProvidersView } from "./screens/ProvidersView";
import { AboutView, BugReportView, LegalView } from "./screens/SettingsSubViews";
import { WatchHistoryView } from "./screens/WatchHistoryView";
import homeSvg from "./assets/drawable/home.svg";
import homeActiveSvg from "./assets/drawable/home-active.svg";
import gridSquareSvg from "./assets/drawable/grid-square.svg";
import playlistSvg from "./assets/drawable/playlist.svg";
import signalStreamSvg from "./assets/drawable/signal-stream.svg";
import signalStreamActiveSvg from "./assets/drawable/signal-stream-active.svg";
import userHeartSvg from "./assets/drawable/user-heart.svg";
import userCircleSvg from "./assets/drawable/user-circle.svg";
import type {
  AppSettings,
  Channel,
  DnsMode,
  DnsTestResult,
  DownloadTask,
  FfmpegStatus,
  PlaybackRequest,
  Provider,
  ProviderType,
  VegaPost,
  WatchHistoryEntry,
} from "./types";

type Route =
  | "home"
  | "television"
  | "movies"
  | "movie-detail"
  | "extensions"
  | "category"
  | "downloads"
  | "providers"
  | "settings"
  | "dns"
  | "history"
  | "about"
  | "bug-report"
  | "legal";

interface HomeVodSection {
  title: string;
  filter: string;
  posts: VegaPost[];
}

interface HomeVodFeed {
  scraperValue: string;
  scraperName: string;
  posts: VegaPost[];
  sections: HomeVodSection[];
}

const DEFAULT_SETTINGS: AppSettings = {
  blackTheme: false,
  startupTab: "home",
  dnsMode: "cloudflare",
  customDohUrl: "",
  developerTestMode: false,
  externalPlayerPath: "",
  downloadDir: "",
  ffmpegEnabled: true,
  ffmpegPath: "",
};

export function App() {
  const [route, setRoute] = useState<Route>("home");
  const [providers, setProviders] = useState<Provider[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [history, setHistory] = useState<WatchHistoryEntry[]>([]);
  const [homeFeed, setHomeFeed] = useState<HomeVodFeed | null>(null);
  const [heroIdx, setHeroIdx] = useState(0);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const settingsRef = useRef<AppSettings>(DEFAULT_SETTINGS);
  settingsRef.current = settings;
  const [isOnline, setIsOnline] = useState<boolean>(navigator.onLine);
  const [loadingChannels, setLoadingChannels] = useState(false);

  // Add Provider form state
  const [newTitle, setNewTitle] = useState("");
  const [newUrl, setNewUrl] = useState("");
  const [newUa, setNewUa] = useState("");
  const [detectedType, setDetectedType] = useState<ProviderType>("IPTV");
  const [manualTypeOverride, setManualTypeOverride] = useState(false);
  const [inspectInfo, setInspectInfo] = useState<string>("");
  const [savingProvider, setSavingProvider] = useState(false);
  const [providerError, setProviderError] = useState("");
  const [providerSuccess, setProviderSuccess] = useState("");
  const [editingUrl, setEditingUrl] = useState<string | null>(null);
  const [initError, setInitError] = useState("");
  const [downloads, setDownloads] = useState<DownloadTask[]>([]);

  // Live TV filters & progressive channel loading
  const LIVE_TV_PAGE_SIZE = 120;
  const [channelQuery, setChannelQuery] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string>("All");
  const [visibleChannelsCount, setVisibleChannelsCount] = useState(LIVE_TV_PAGE_SIZE);
  const liveSentinelRef = useRef<HTMLDivElement>(null);

  // DNS test state
  const [dnsTest, setDnsTest] = useState<DnsTestResult | null>(null);
  const [testingDns, setTestingDns] = useState(false);

  // FFmpeg runtime status state
  const [ffmpegStatus, setFfmpegStatus] = useState<FfmpegStatus | null>(null);
  const [installingFfmpeg, setInstallingFfmpeg] = useState(false);
  const [ffmpegError, setFfmpegError] = useState("");

  // Selected VOD post for detail view
  const [selectedVod, setSelectedVod] = useState<{
    post: VegaPost;
    scraperValue: string;
  } | null>(null);
  const [moviesInitialFilter, setMoviesInitialFilter] = useState<string | undefined>(undefined);

  // Active full-screen playback overlay
  const [activePlayback, setActivePlayback] = useState<PlaybackRequest | null>(null);

  useEffect(() => {
    const onOnline = () => setIsOnline(true);
    const onOffline = () => setIsOnline(false);
    window.addEventListener("online", onOnline);
    window.addEventListener("offline", onOffline);
    return () => {
      window.removeEventListener("online", onOnline);
      window.removeEventListener("offline", onOffline);
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = settings.blackTheme
      ? "amoled"
      : "dark";
  }, [settings.blackTheme]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<FfmpegStatus>("ffmpeg://status", (e) => {
      setFfmpegStatus(e.payload);
      setInstallingFfmpeg(Boolean(e.payload.installing));
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  const refreshHomeFeed = useCallback(async (provList: Provider[]) => {
    const activeVega = provList.filter((p) => p.isActive && p.type === "VEGA");
    if (activeVega.length === 0) {
      setHomeFeed(null);
      return;
    }
    try {
      const exts = await api.listVegaExtensions();
      const installed = exts.filter((e) => e.is_installed && !e.disabled);
      const target = installed[0] ?? exts.find((e) => !e.disabled);
      if (!target) {
        setHomeFeed(null);
        return;
      }
      const cat = await api.getVodCatalog(target.value);
      const scraperName = target.display_name || target.value;
      const seenFilters = new Set<string>();
      const rawCandidates = [
        ...cat.catalog,
        ...(cat.catalog.length <= 4 ? (cat.genres ?? []) : []),
      ];
      const candidates = rawCandidates
        .filter((item) => {
          const title = item.title.trim();
          const filterKey = item.filter.trim();
          if (!title || seenFilters.has(filterKey)) return false;
          seenFilters.add(filterKey);
          return true;
        })
        .slice(0, 10);

      if (candidates.length === 0) {
        const posts = await api.getVodPosts(target.value, "", 1);
        setHomeFeed({
          scraperValue: target.value,
          scraperName,
          posts,
          sections:
            posts.length > 0
              ? [{ title: `${scraperName} Catalogue`, filter: "", posts }]
              : [],
        });
        setHeroIdx(0);
        return;
      }

      const first = candidates[0];
      const firstPosts = await api.getVodPosts(target.value, first.filter, 1).catch(() => [] as VegaPost[]);
      const initialSection: HomeVodSection | null =
        firstPosts.length > 0
          ? { title: first.title.trim(), filter: first.filter, posts: firstPosts }
          : null;

      setHomeFeed({
        scraperValue: target.value,
        scraperName,
        posts: firstPosts,
        sections: initialSection ? [initialSection] : [],
      });
      setHeroIdx(0);

      if (candidates.length > 1) {
        const restResults = await Promise.allSettled(
          candidates.slice(1).map(async (item) => {
            const posts = await api.getVodPosts(target.value, item.filter, 1);
            return { title: item.title.trim(), filter: item.filter, posts } as HomeVodSection;
          })
        );
        const extraSections = restResults
          .filter((r): r is PromiseFulfilledResult<HomeVodSection> => r.status === "fulfilled" && r.value.posts.length > 0)
          .map((r) => r.value);

        if (extraSections.length > 0) {
          setHomeFeed((prev) => {
            if (!prev || prev.scraperValue !== target.value) return prev;
            const merged = initialSection ? [initialSection, ...extraSections] : extraSections;
            return {
              ...prev,
              posts: prev.posts.length > 0 ? prev.posts : (merged[0]?.posts ?? []),
              sections: merged,
            };
          });
        }
      }
    } catch {
      // Keep Home responsive even if upstream scraper is temporarily unreachable
    }
  }, []);

  useEffect(() => {
    async function init() {
      try {
        const [prov, cfg, hist, dls] = await Promise.all([
          api.listProviders(),
          api.getAppSettings(),
          api.getWatchHistory(),
          api.listDownloads(),
        ]);
        setProviders(prov);
        setSettings(cfg);
        setHistory(hist);
        setDownloads(dls);
        void api.checkFfmpegUpstreamStatus().then(setFfmpegStatus).catch(() => {
          void api.getFfmpegStatus().then(setFfmpegStatus).catch(() => {});
        });
        if (prov.length === 0) {
          setRoute("providers");
        } else {
          setRoute(cfg.startupTab as Route);
        }
        void refreshHomeFeed(prov);
        if (prov.some((p) => p.isActive && p.type === "IPTV")) {
          setLoadingChannels(true);
          try {
            setChannels(await api.loadChannels(false));
          } catch (err) {
            setProviderError(String(err));
          } finally {
            setLoadingChannels(false);
          }
        }
      } catch (err) {
        setInitError(String(err));
      }
    }
    void init();
  }, [refreshHomeFeed]);

  const categories = useMemo(() => {
    const set = new Set<string>();
    channels.forEach((c) => set.add(c.category));
    return ["All", ...Array.from(set).sort()];
  }, [channels]);

  const filteredChannels = useMemo(() => {
    const activeCat = categories.includes(selectedCategory) ? selectedCategory : "All";
    const q = channelQuery.trim().toLowerCase();
    return channels.filter((c) => {
      const catOk = activeCat === "All" || c.category === activeCat;
      const qOk =
        !q ||
        c.name.toLowerCase().includes(q) ||
        c.category.toLowerCase().includes(q);
      return catOk && qOk;
    });
  }, [channels, channelQuery, selectedCategory, categories]);

  useEffect(() => {
    setVisibleChannelsCount(LIVE_TV_PAGE_SIZE);
  }, [channelQuery, selectedCategory]);

  useEffect(() => {
    if (route !== "television" || visibleChannelsCount >= filteredChannels.length) {
      return;
    }
    const sentinel = liveSentinelRef.current;
    if (!sentinel) return;
    const root = sentinel.closest(".ott-main") as HTMLElement | null;
    const loadNextBatch = () =>
      setVisibleChannelsCount((prev) =>
        Math.min(filteredChannels.length, prev + LIVE_TV_PAGE_SIZE)
      );
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          loadNextBatch();
        }
      },
      { root, rootMargin: "900px 0px" }
    );
    observer.observe(sentinel);
    const onScroll = () => {
      if (!root) return;
      if (root.scrollHeight - (root.scrollTop + root.clientHeight) <= 900) {
        loadNextBatch();
      }
    };
    root?.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      observer.disconnect();
      root?.removeEventListener("scroll", onScroll);
    };
  }, [route, visibleChannelsCount, filteredChannels.length]);

  const activeDownloadsCount = useMemo(
    () => downloads.filter((d) => d.status === "DOWNLOADING" || d.status === "PENDING").length,
    [downloads]
  );

  const heroSlides = useMemo(() => {
    if (homeFeed && homeFeed.posts.length > 0) {
      return homeFeed.posts.slice(0, 5).map((post) => ({
        title: formatOttTitle(post.title),
        eyebrow: `Now Streaming • ${homeFeed.scraperName}`,
        image: post.image,
        ctaLabel: "Play Now",
        secondaryLabel: "Explore Movies",
        onPrimary: () => {
          setSelectedVod({ post, scraperValue: homeFeed.scraperValue });
          setRoute("movie-detail");
        },
        onSecondary: () => {
          setMoviesInitialFilter(undefined);
          setRoute("movies");
        },
      }));
    }
    if (history.length > 0) {
      return history.slice(0, 4).map((entry) => ({
        title: entry.title,
        eyebrow: "Continue Watching",
        image: entry.imageUrl,
        ctaLabel: "Resume",
        secondaryLabel: "Watch History",
        onPrimary: () =>
          setActivePlayback({
            title: entry.title,
            link: entry.link,
            streamUrl: entry.streamUrl,
            imageUrl: entry.imageUrl,
            providerUrl: entry.providerUrl,
            scraperValue: entry.scraperValue,
          }),
        onSecondary: () => setRoute("history"),
      }));
    }
    if (channels.length > 0) {
      return channels.slice(0, 4).map((ch) => ({
        title: ch.name,
        eyebrow: `Live Now • ${ch.category}`,
        image: ch.logoUrl ?? "",
        ctaLabel: "Watch Live",
        secondaryLabel: "All Channels",
        onPrimary: () =>
          setActivePlayback({
            title: ch.name,
            link: ch.streamUrl,
            streamUrl: ch.streamUrl,
            headers: ch.headers,
            imageUrl: ch.logoUrl,
            isLive: true,
            channel: ch,
          }),
        onSecondary: () => setRoute("television"),
      }));
    }
    const firstVideo = providers.find((p) => p.isActive && p.type === "VIDEO");
    if (firstVideo) {
      return [
        {
          title: firstVideo.title,
          eyebrow: "Now Streaming",
          image: "",
          ctaLabel: "Play Now",
          secondaryLabel: "Providers",
          onPrimary: () =>
            setActivePlayback({
              title: firstVideo.title,
              link: firstVideo.url,
              streamUrl: firstVideo.url,
              headers: firstVideo.userAgent ? { "User-Agent": firstVideo.userAgent } : undefined,
            }),
          onSecondary: () => setRoute("providers"),
        },
      ];
    }
    return [];
  }, [channels, history, homeFeed, providers]);

  const currentHero = heroSlides[heroIdx] ?? heroSlides[0];

  const continueCards: OttCardItem[] = useMemo(
    () =>
      history.slice(0, 12).map((item) => ({
        id: item.link,
        title: item.title,
        subtitle:
          item.durationMs > 0
            ? `${Math.min(100, Math.round((item.positionMs / item.durationMs) * 100))}% watched`
            : "Resume",
        image: item.imageUrl,
        progressPct:
          item.durationMs > 0
            ? Math.min(100, Math.round((item.positionMs / item.durationMs) * 100))
            : 0,
        onClick: () =>
          setActivePlayback({
            title: item.title,
            link: item.link,
            streamUrl: item.streamUrl,
            imageUrl: item.imageUrl,
            providerUrl: item.providerUrl,
            scraperValue: item.scraperValue,
          }),
      })),
    [history]
  );

  const liveCards: OttCardItem[] = useMemo(
    () =>
      channels.slice(0, 14).map((ch) => ({
        id: `${ch.name}-${ch.streamUrl}`,
        title: ch.name,
        subtitle: `${ch.category} • ${ch.streamType}`,
        badge: "LIVE",
        image: ch.logoUrl ?? "",
        onClick: () =>
          setActivePlayback({
            title: ch.name,
            link: ch.streamUrl,
            streamUrl: ch.streamUrl,
            headers: ch.headers,
            imageUrl: ch.logoUrl,
            isLive: true,
            channel: ch,
          }),
      })),
    [channels]
  );

  const vodCategorySections = useMemo(() => {
    if (!homeFeed) return [];
    const sourceSections =
      homeFeed.sections.length > 0
        ? homeFeed.sections
        : homeFeed.posts.length > 0
          ? [{ title: `${homeFeed.scraperName} Catalogue`, filter: "", posts: homeFeed.posts }]
          : [];
    return sourceSections.map((sec) => ({
      title: sec.title,
      filter: sec.filter,
      items: sec.posts.slice(0, 18).map((post) => ({
        id: `${sec.filter}-${post.link}-${post.title}`,
        title: formatOttTitle(post.title),
        image: post.image,
        aspectRatio: post.aspectRatio ?? undefined,
        onClick: () => {
          setSelectedVod({ post, scraperValue: homeFeed.scraperValue });
          setRoute("movie-detail");
        },
      })),
    }));
  }, [homeFeed]);

  const liveCategorySummaries = useMemo(() => {
    const counts = new Map<string, number>();
    for (const ch of channels) {
      const cat = ch.category?.trim();
      if (!cat || cat === "All") continue;
      counts.set(cat, (counts.get(cat) ?? 0) + 1);
    }
    return Array.from(counts.entries())
      .map(([name, count]) => ({ name, count }))
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
  }, [channels]);

  const vodLandscapeCards: OttCardItem[] = useMemo(() => {
    if (!homeFeed) return [];
    const slice = homeFeed.posts.slice(1, 10);
    return (slice.length > 0 ? slice : homeFeed.posts).map((post) => ({
      id: `land-${post.link}-${post.title}`,
      title: formatOttTitle(post.title),
      subtitle: homeFeed.scraperName,
      image: post.image,
      onClick: () => {
        setSelectedVod({ post, scraperValue: homeFeed.scraperValue });
        setRoute("movie-detail");
      },
    }));
  }, [homeFeed]);

  const midBannerPost = homeFeed?.posts[2] ?? homeFeed?.posts[1] ?? homeFeed?.posts[0] ?? null;

  async function handleUrlChange(val: string) {
    setNewUrl(val);
    setInspectInfo("");
    setProviderError("");
    setProviderSuccess("");
    if (val.trim() && !manualTypeOverride) {
      try {
        setDetectedType(await api.detectProviderType(val, newTitle));
      } catch {
        // Ignore heuristic errors while typing
      }
    }
  }

  async function handleInspect() {
    if (!newUrl.trim()) return;
    setInspectInfo("Inspecting...");
    setProviderError("");
    try {
      const res = await api.inspectProvider(newUrl.trim());
      setDetectedType(res.providerType);
      setManualTypeOverride(false);
      setInspectInfo(
        `Detected ${res.providerType === "VEGA" ? "SOURCE" : res.providerType} (${res.sourceKind})`
      );
    } catch (err) {
      setInspectInfo("");
      setProviderError(String(err));
    }
  }

  async function handleAddProvider(e: React.FormEvent) {
    e.preventDefault();
    const trimmedUrl = newUrl.trim();
    if (!trimmedUrl) {
      setProviderError("Please enter a playlist, extension source, @handle, or video URL.");
      return;
    }
    if (savingProvider) return;

    setSavingProvider(true);
    setProviderError("");
    setProviderSuccess("");

    try {
      let resolvedType = detectedType;
      if (!manualTypeOverride && !inspectInfo && /^https?:\/\//i.test(trimmedUrl)) {
        try {
          const inspected = await api.inspectProvider(trimmedUrl);
          resolvedType = inspected.providerType;
          setDetectedType(resolvedType);
        } catch {
          // Fall back to quick-detected type if inspection fails
        }
      }

      const payload: Provider = {
        title: newTitle.trim() || trimmedUrl,
        url: trimmedUrl,
        userAgent: newUa.trim() || null,
        isActive: true,
        channelCount: 0,
        type: resolvedType,
      };
      const updated = editingUrl
        ? await api.updateProvider(editingUrl, payload)
        : await api.addProvider(payload);

      setProviders(updated);
      setEditingUrl(null);
      setNewTitle("");
      setNewUrl("");
      setNewUa("");
      setInspectInfo("");
      setManualTypeOverride(false);

      if (resolvedType === "IPTV") {
        setLoadingChannels(true);
        try {
          const loaded = await api.loadChannels(true);
          setChannels(loaded);
          setProviders(await api.listProviders());
          setProviderSuccess(`Provider saved. Loaded ${loaded.length} Live TV channels.`);
        } catch (loadErr) {
          setProviderSuccess("Provider saved.");
          setProviderError(`Playlist fetch failed: ${String(loadErr)}`);
        } finally {
          setLoadingChannels(false);
        }
      } else if (resolvedType === "VEGA") {
        try {
          const exts = await api.listVegaExtensions(trimmedUrl, true);
          setProviderSuccess(
            `Source saved. Loaded ${exts.length} extension(s) — open Movies or Extensions.`
          );
          void refreshHomeFeed(updated);
        } catch (vegaErr) {
          setProviderSuccess("Source saved.");
          setProviderError(`Manifest fetch failed: ${String(vegaErr)}`);
        }
      } else {
        setProviderSuccess("Direct video source saved. Click Play below to watch.");
      }
    } catch (err) {
      setProviderError(`Failed to save provider: ${String(err)}`);
    } finally {
      setSavingProvider(false);
    }
  }

  async function handleImportLocalFile(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setSavingProvider(true);
    setProviderError("");
    setProviderSuccess("");
    try {
      const content = await file.text();
      const title = file.name.replace(/\.[^.]+$/, "");
      const updated = await api.importLocalPlaylist(title, content);
      setProviders(updated);
      const loaded = await api.loadChannels(true);
      setChannels(loaded);
      setProviders(await api.listProviders());
      setProviderSuccess(`Imported "${file.name}" (${loaded.length} channels).`);
    } catch (err) {
      setProviderError(`Failed to import file: ${String(err)}`);
    } finally {
      setSavingProvider(false);
    }
  }

  async function handleToggleProvider(p: Provider) {
    setProviderError("");
    try {
      const updated = await api.updateProvider(null, {
        ...p,
        isActive: !p.isActive,
      });
      setProviders(updated);
      setChannels(await api.loadChannels(true));
      setProviders(await api.listProviders());
      void refreshHomeFeed(updated);
    } catch (err) {
      setProviderError(String(err));
    }
  }

  async function handleRemoveProvider(url: string) {
    setProviderError("");
    try {
      const updated = await api.removeProvider(url);
      setProviders(updated);
      setChannels(await api.loadChannels(true));
      void refreshHomeFeed(updated);
    } catch (err) {
      setProviderError(String(err));
    }
  }

  async function updateSettings(patch: Partial<AppSettings>) {
    const next = { ...settingsRef.current, ...patch };
    settingsRef.current = next;
    setSettings(next);
    if ("dnsMode" in patch || "customDohUrl" in patch) {
      setDnsTest(null);
    }
    await api.saveAppSettings(next);
  }

  async function runDnsTest(mode: DnsMode) {
    setTestingDns(true);
    try {
      setDnsTest(await api.testDns(mode, settings.customDohUrl));
    } finally {
      setTestingDns(false);
    }
  }

  const isBrowseActive = route === "movies" || route === "movie-detail" || route === "category";
  const isLibraryActive = route === "extensions" || route === "history";
  const isAccountActive =
    route === "providers" ||
    route === "settings" ||
    route === "dns" ||
    route === "about" ||
    route === "bug-report" ||
    route === "legal";

  return (
    <div className="ott-shell">
      {/* Figma Product_Nav Left Floating Gradient Icon Rail */}
      <nav className="ott-nav-rail" aria-label="Primary Navigation">
        <button
          type="button"
          className={`ott-nav-item ${route === "home" ? "active" : ""}`}
          aria-label="Home"
          onClick={() => setRoute("home")}
        >
          <img
            className="ott-nav-icon"
            src={route === "home" ? homeActiveSvg : homeSvg}
            alt=""
            width={56}
            height={56}
          />
          <span className="ott-nav-tooltip">Home</span>
        </button>

        <button
          type="button"
          className={`ott-nav-item ${isBrowseActive ? "active" : ""}`}
          aria-label="Movies & Browse"
          onClick={() => {
            setMoviesInitialFilter(undefined);
            setRoute("movies");
          }}
        >
          <img className="ott-nav-icon" src={gridSquareSvg} alt="" width={56} height={56} />
          <span className="ott-nav-tooltip">Movies & Browse</span>
        </button>

        <button
          type="button"
          className={`ott-nav-item ${isLibraryActive ? "active" : ""}`}
          aria-label="Extensions & History"
          onClick={() => setRoute("extensions")}
        >
          <img className="ott-nav-icon" src={playlistSvg} alt="" width={56} height={56} />
          <span className="ott-nav-tooltip">Extensions & History</span>
        </button>

        <button
          type="button"
          className={`ott-nav-item ${route === "television" ? "active" : ""}`}
          aria-label="Live TV"
          onClick={() => setRoute("television")}
        >
          <img
            className="ott-nav-icon"
            src={route === "television" ? signalStreamActiveSvg : signalStreamSvg}
            alt=""
            width={56}
            height={56}
          />
          <span className="ott-nav-tooltip">Live TV ({channels.length})</span>
        </button>

        <button
          type="button"
          className={`ott-nav-item ${route === "downloads" ? "active" : ""}`}
          aria-label="Downloads"
          onClick={() => setRoute("downloads")}
        >
          <img className="ott-nav-icon" src={userHeartSvg} alt="" width={56} height={56} />
          {activeDownloadsCount > 0 && (
            <span className="ott-nav-badge">{activeDownloadsCount}</span>
          )}
          <span className="ott-nav-tooltip">Downloads</span>
        </button>

        <button
          type="button"
          className={`ott-nav-item ${isAccountActive ? "active" : ""}`}
          aria-label="Providers & Settings"
          onClick={() => setRoute(providers.length === 0 ? "providers" : "settings")}
        >
          <img className="ott-nav-icon" src={userCircleSvg} alt="" width={56} height={56} />
          <span className="ott-nav-tooltip">Providers & Settings</span>
        </button>
      </nav>

      {/* Non-intrusive translucent top-right alerts */}
      {(!isOnline || !isTauriRuntime() || initError) && (
        <div className="ott-status-strip">
          {!isOnline && (
            <div className="ott-status-pill" style={{ color: "var(--status-warn)" }}>
              Offline • Local downloads playable
            </div>
          )}
          {!isTauriRuntime() && (
            <div className="ott-status-pill" style={{ color: "var(--status-error)" }}>
              Browser mode • Native Tauri window required
            </div>
          )}
          {initError && (
            <div className="ott-status-pill" style={{ color: "var(--status-error)" }}>
              Backend error: {initError}
            </div>
          )}
        </div>
      )}

      <main className="ott-main">
        {route === "home" && (
          <div className="ott-page-home">
            {currentHero ? (
              <HeroMarquee
                title={currentHero.title}
                eyebrow={currentHero.eyebrow}
                backdropUrl={currentHero.image || undefined}
                ctaLabel={currentHero.ctaLabel}
                secondaryLabel={currentHero.secondaryLabel}
                onPrimaryAction={currentHero.onPrimary}
                onSecondaryAction={currentHero.onSecondary}
                slideCount={heroSlides.length}
                activeSlide={heroIdx}
                onSelectSlide={setHeroIdx}
              />
            ) : (
              <HeroMarquee
                title="IPTVMINE"
                eyebrow={
                  providers.length > 0
                    ? `${providers.length} Provider Ready`
                    : "Connect a Stream Source"
                }
                ctaLabel={providers.length > 0 ? "Browse Movies" : "Add Provider"}
                secondaryLabel="Manage Providers"
                onPrimaryAction={() =>
                  setRoute(providers.length > 0 ? "movies" : "providers")
                }
                onSecondaryAction={() => setRoute("providers")}
              />
            )}

            {continueCards.length > 0 && (
              <OttCarousel
                title="Continue Watching"
                variant="landscape"
                items={continueCards}
                headerAction={
                  <button
                    type="button"
                    className="ott-glass-pill"
                    onClick={() => setRoute("history")}
                  >
                    View All
                  </button>
                }
              />
            )}

            {continueCards.length === 0 && vodLandscapeCards.length > 0 && (
              <OttCarousel
                title="Featured Releases"
                variant="landscape"
                items={vodLandscapeCards}
                headerAction={
                  <button
                    type="button"
                    className="ott-glass-pill"
                    onClick={() => setRoute("movies")}
                  >
                    Open Movies
                  </button>
                }
              />
            )}

            {liveCards.length > 0 && (
              <OttCarousel
                title="Live Television"
                variant="landscape"
                items={liveCards}
                headerAction={
                  <button
                    type="button"
                    className="ott-glass-pill"
                    onClick={() => setRoute("television")}
                  >
                    All {channels.length} Channels
                  </button>
                }
              />
            )}

            {liveCategorySummaries.length > 0 && (
              <section className="ott-section" aria-label="Live TV Categories">
                <div className="ott-section-head">
                  <h2 className="ott-section-title">Live TV Categories</h2>
                  <button
                    type="button"
                    className="ott-glass-pill"
                    onClick={() => setRoute("category")}
                  >
                    All Categories
                  </button>
                </div>
                <div className="ott-cat-strip">
                  {liveCategorySummaries.slice(0, 24).map((cat) => (
                    <button
                      type="button"
                      key={cat.name}
                      className="ott-cat-chip"
                      onClick={() => {
                        setSelectedCategory(cat.name);
                        setRoute("television");
                      }}
                    >
                      <span>{cat.name}</span>
                      <span className="ott-cat-count">{cat.count}</span>
                    </button>
                  ))}
                </div>
              </section>
            )}

            {midBannerPost && homeFeed && (
              <MidBanner
                title={formatOttTitle(midBannerPost.title)}
                eyebrow={`Now Streaming • ${homeFeed.scraperName}`}
                imageUrl={midBannerPost.image}
                ctaLabel="Play Now"
                onAction={() => {
                  setSelectedVod({ post: midBannerPost, scraperValue: homeFeed.scraperValue });
                  setRoute("movie-detail");
                }}
              />
            )}

            {vodCategorySections.map((sec) => (
              <OttCarousel
                key={sec.filter || sec.title}
                title={sec.title}
                variant="tall"
                items={sec.items}
                headerAction={
                  <button
                    type="button"
                    className="ott-glass-pill"
                    onClick={() => {
                      setMoviesInitialFilter(sec.filter);
                      setRoute("movies");
                    }}
                  >
                    Browse
                  </button>
                }
              />
            ))}
          </div>
        )}

        {route !== "home" && (
          <div className="ott-page">
            {isBrowseActive && route !== "movie-detail" && (
              <div style={{ marginBottom: 18 }}>
                <div className="segmented-bar">
                  <button
                    type="button"
                    className={`seg-btn ${route === "movies" ? "active" : ""}`}
                    onClick={() => setRoute("movies")}
                  >
                    Movies & VOD
                  </button>
                  <button
                    type="button"
                    className={`seg-btn ${route === "category" ? "active" : ""}`}
                    onClick={() => setRoute("category")}
                  >
                    Live TV Categories
                  </button>
                </div>
              </div>
            )}

            {isLibraryActive && (
              <div style={{ maxWidth: 1120, margin: "0 auto 18px" }}>
                <div className="segmented-bar">
                  <button
                    type="button"
                    className={`seg-btn ${route === "extensions" ? "active" : ""}`}
                    onClick={() => setRoute("extensions")}
                  >
                    Extensions
                  </button>
                  <button
                    type="button"
                    className={`seg-btn ${route === "history" ? "active" : ""}`}
                    onClick={() => setRoute("history")}
                  >
                    Watch History
                  </button>
                </div>
              </div>
            )}

            {isAccountActive && (
              <div className="ott-account-nav-wrap">
                <div className="segmented-bar">
                  <button
                    type="button"
                    className={`seg-btn ${route === "providers" ? "active" : ""}`}
                    onClick={() => setRoute("providers")}
                  >
                    Providers
                  </button>
                  <button
                    type="button"
                    className={`seg-btn ${route === "settings" ? "active" : ""}`}
                    onClick={() => setRoute("settings")}
                  >
                    Preferences
                  </button>
                  <button
                    type="button"
                    className={`seg-btn ${route === "dns" ? "active" : ""}`}
                    onClick={() => setRoute("dns")}
                  >
                    DoH DNS
                  </button>
                  <button
                    type="button"
                    className={`seg-btn ${route === "about" ? "active" : ""}`}
                    onClick={() => setRoute("about")}
                  >
                    About
                  </button>
                  <button
                    type="button"
                    className={`seg-btn ${route === "bug-report" ? "active" : ""}`}
                    onClick={() => setRoute("bug-report")}
                  >
                    Diagnostics
                  </button>
                  <button
                    type="button"
                    className={`seg-btn ${route === "legal" ? "active" : ""}`}
                    onClick={() => setRoute("legal")}
                  >
                    Legal
                  </button>
                </div>
              </div>
            )}

            {route === "television" && (
              <div>
                <div className="page-header">
                  <h1 className="page-title">Live TV ({filteredChannels.length})</h1>
                  <button
                    type="button"
                    className="btn"
                    onClick={async () => {
                      setLoadingChannels(true);
                      try {
                        setChannels(await api.loadChannels(true));
                      } finally {
                        setLoadingChannels(false);
                      }
                    }}
                  >
                    <RefreshCw size={14} /> {loadingChannels ? "Refreshing..." : "Refresh"}
                  </button>
                </div>
                <div style={{ display: "flex", gap: 12, marginBottom: 18 }}>
                  <input
                    className="input"
                    placeholder="Search channels..."
                    value={channelQuery}
                    onChange={(e) => setChannelQuery(e.target.value)}
                  />
                  <select
                    className="input"
                    style={{ maxWidth: 220 }}
                    value={selectedCategory}
                    onChange={(e) => setSelectedCategory(e.target.value)}
                  >
                    {categories.map((cat) => (
                      <option key={cat} value={cat}>
                        {cat}
                      </option>
                    ))}
                  </select>
                </div>
                {filteredChannels.length === 0 ? (
                  <div className="card" style={{ display: "grid", gap: 10 }}>
                    <div className="muted">
                      No Live TV channels loaded yet. Add an M3U / IPTV playlist in Providers.
                    </div>
                    <div>
                      <button
                        type="button"
                        className="btn btn-primary"
                        onClick={() => setRoute("providers")}
                      >
                        Add IPTV Provider
                      </button>
                    </div>
                  </div>
                ) : (
                  <>
                    <div className="live-vcard-grid">
                      {filteredChannels.slice(0, visibleChannelsCount).map((ch, idx) => {
                        const numericRes = ch.name.match(/\b(\d{3,4}p|4k|uhd)\b/i);
                        const fallbackTier = !numericRes ? ch.name.match(/\b(fhd|hd)\b/i) : null;
                        const resolutionLabel = (numericRes?.[1] ?? fallbackTier?.[1] ?? "").toUpperCase();
                        const primaryCategories = ch.category
                          .split(/[;,]/)
                          .map((part) => part.trim())
                          .filter(Boolean)
                          .slice(0, 2)
                          .join(" · ");

                        return (
                          <button
                            type="button"
                            key={`${ch.streamUrl}-${ch.name}-${idx}`}
                            className="live-vcard"
                            onClick={() =>
                              setActivePlayback({
                                title: ch.name,
                                link: ch.streamUrl,
                                streamUrl: ch.streamUrl,
                                headers: ch.headers,
                                imageUrl: ch.logoUrl,
                                isLive: true,
                                channel: ch,
                              })
                            }
                          >
                            <div className="live-vcard-frame">
                              <div className="live-vcard-top">
                                <span className="live-vcard-pill is-live">
                                  <span className="live-vcard-dot" aria-hidden="true" />
                                  <span>{ch.streamType || "LIVE"}</span>
                                </span>
                                {resolutionLabel && (
                                  <span className="live-vcard-pill">{resolutionLabel}</span>
                                )}
                              </div>

                              <div className="live-vcard-stage">
                                <div className="live-vcard-emblem">
                                  <ChannelLogo src={ch.logoUrl} name={ch.name} size={74} />
                                </div>
                              </div>

                              <span className="apple-poster-play" aria-hidden="true">
                                <Play size={15} fill="currentColor" style={{ marginLeft: 1 }} />
                              </span>
                              <div className="apple-poster-rim" />
                            </div>

                            <div className="live-vcard-caption">
                              <h3 className="live-vcard-title" title={ch.name}>
                                {ch.name}
                              </h3>
                              <span
                                className="live-vcard-meta"
                                title={ch.category || "Live Channel"}
                              >
                                {primaryCategories || "Live Channel"}
                              </span>
                            </div>
                          </button>
                        );
                      })}
                    </div>

                    <div
                      ref={liveSentinelRef}
                      style={{
                        marginTop: 26,
                        display: "flex",
                        flexDirection: "column",
                        alignItems: "center",
                        gap: 10,
                      }}
                    >
                      <div className="muted" style={{ fontSize: 13 }}>
                        {visibleChannelsCount < filteredChannels.length
                          ? `Showing ${visibleChannelsCount.toLocaleString()} of ${filteredChannels.length.toLocaleString()} channels`
                          : `All ${filteredChannels.length.toLocaleString()} channels loaded`}
                      </div>
                      {visibleChannelsCount < filteredChannels.length && (
                        <div style={{ display: "flex", gap: 10, flexWrap: "wrap", justifyContent: "center" }}>
                          <button
                            type="button"
                            className="btn"
                            onClick={() =>
                              setVisibleChannelsCount((prev) =>
                                Math.min(filteredChannels.length, prev + LIVE_TV_PAGE_SIZE)
                              )
                            }
                          >
                            Load More (+{Math.min(LIVE_TV_PAGE_SIZE, filteredChannels.length - visibleChannelsCount)})
                          </button>
                          <button
                            type="button"
                            className="btn"
                            onClick={() => setVisibleChannelsCount(filteredChannels.length)}
                          >
                            Show All ({filteredChannels.length.toLocaleString()})
                          </button>
                        </div>
                      )}
                    </div>
                  </>
                )}
              </div>
            )}

            {route === "movies" && (
              <MoviesView
                initialFilter={moviesInitialFilter}
                onSelectPost={(post, scraperValue) => {
                  setSelectedVod({ post, scraperValue });
                  setRoute("movie-detail");
                }}
                onOpenExtensions={() => setRoute("extensions")}
              />
            )}

            {route === "movie-detail" && selectedVod && (
              <MovieDetailView
                post={selectedVod.post}
                scraperValue={selectedVod.scraperValue}
                onBack={() => setRoute("movies")}
                onPlayStream={(req) => setActivePlayback(req)}
              />
            )}

            {route === "extensions" && (
              <ExtensionsView
                providers={providers}
                onProvidersChanged={(updated) => {
                  setProviders(updated);
                  void refreshHomeFeed(updated);
                }}
                onExtensionsChanged={() => void refreshHomeFeed(providers)}
                onOpenMovies={() => setRoute("movies")}
              />
            )}

            {route === "category" && (
              <div>
                <div className="page-header">
                  <h1 className="page-title">Live Categories</h1>
                </div>
                {categories.filter((c) => c !== "All").length === 0 ? (
                  <div className="card muted">
                    No channel categories yet. Add an active IPTV playlist to populate categories.
                  </div>
                ) : (
                  <div className="grid-channels">
                    {categories
                      .filter((c) => c !== "All")
                      .map((cat) => (
                        <button
                          type="button"
                          key={cat}
                          className="card channel-card"
                          onClick={() => {
                            setSelectedCategory(cat);
                            setRoute("television");
                          }}
                        >
                          <div style={{ fontWeight: 600 }}>{cat}</div>
                          <div className="muted" style={{ marginTop: 4 }}>
                            {channels.filter((c) => c.category === cat).length} channels
                          </div>
                        </button>
                      ))}
                  </div>
                )}
              </div>
            )}

            {route === "providers" && (
              <ProvidersView
                providers={providers}
                editingUrl={editingUrl}
                newTitle={newTitle}
                newUrl={newUrl}
                newUa={newUa}
                detectedType={detectedType}
                manualTypeOverride={manualTypeOverride}
                inspectInfo={inspectInfo}
                savingProvider={savingProvider}
                providerError={providerError}
                providerSuccess={providerSuccess}
                onUrlChange={(val) => void handleUrlChange(val)}
                onTitleChange={setNewTitle}
                onUaChange={setNewUa}
                onTypeSelect={(type) => {
                  setDetectedType(type);
                  setManualTypeOverride(true);
                }}
                onInspect={() => void handleInspect()}
                onSubmit={(e) => void handleAddProvider(e)}
                onImportLocalFile={(e) => void handleImportLocalFile(e)}
                onCancelEdit={() => {
                  setEditingUrl(null);
                  setNewTitle("");
                  setNewUrl("");
                  setNewUa("");
                  setInspectInfo("");
                  setProviderError("");
                  setManualTypeOverride(false);
                }}
                onEditProvider={(p) => {
                  setEditingUrl(p.url);
                  setNewTitle(p.title);
                  setNewUrl(p.url);
                  setNewUa(p.userAgent ?? "");
                  setDetectedType(p.type);
                  setManualTypeOverride(true);
                  setInspectInfo("");
                  setProviderError("");
                  setProviderSuccess("");
                }}
                onToggleProvider={(p) => void handleToggleProvider(p)}
                onRemoveProvider={(url) => void handleRemoveProvider(url)}
                onPlayVideo={(req) => setActivePlayback(req)}
              />
            )}

            {route === "settings" && (
              <PreferencesView
                settings={settings}
                ffmpegStatus={ffmpegStatus}
                installingFfmpeg={installingFfmpeg}
                ffmpegError={ffmpegError}
                onUpdateSettings={updateSettings}
                onFfmpegStatusChange={setFfmpegStatus}
                onInstallingChange={setInstallingFfmpeg}
                onFfmpegErrorChange={setFfmpegError}
              />
            )}

            {route === "downloads" && (
              <DownloadsView
                onPlayLocal={(req) => setActivePlayback(req)}
                onTasksChange={setDownloads}
              />
            )}

            {route === "history" && (
              <WatchHistoryView onResume={(req) => setActivePlayback(req)} />
            )}

            {route === "about" && <AboutView onBack={() => setRoute("settings")} />}
            {route === "bug-report" && <BugReportView onBack={() => setRoute("settings")} />}
            {route === "legal" && <LegalView onBack={() => setRoute("settings")} />}

            {route === "dns" && (
              <DnsSettingsView
                dnsMode={settings.dnsMode}
                customDohUrl={settings.customDohUrl}
                dnsTest={dnsTest}
                testingDns={testingDns}
                onSelectMode={(mode) => void updateSettings({ dnsMode: mode })}
                onCustomUrlChange={(customDohUrl) => void updateSettings({ customDohUrl })}
                onRunTest={(mode) => void runDnsTest(mode)}
              />
            )}
          </div>
        )}
      </main>

      {activePlayback && (
        <PlayerView
          request={activePlayback}
          ffmpegEnabled={settings.ffmpegEnabled}
          onClose={() => {
            setActivePlayback(null);
            void api.getWatchHistory().then(setHistory).catch(() => {});
          }}
          onEpisodeResolved={(nextReq) => setActivePlayback(nextReq)}
        />
      )}
    </div>
  );
}

export default App;
