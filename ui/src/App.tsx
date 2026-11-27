import { useCallback, useEffect, useMemo, useState } from "react";
import { Pencil, Play, RefreshCw, Trash2, Upload } from "lucide-react";
import { api, isTauriRuntime } from "./api";
import { formatOttTitle, HeroMarquee, MidBanner, OttCarousel, type OttCardItem } from "./components/OttUi";
import { DownloadsView } from "./screens/DownloadsView";
import { ExtensionsView } from "./screens/ExtensionsView";
import { MovieDetailView } from "./screens/MovieDetailView";
import { MoviesView } from "./screens/MoviesView";
import { PlayerView } from "./screens/PlayerView";
import { AboutView, BugReportView, LegalView } from "./screens/SettingsSubViews";
import { WatchHistoryView } from "./screens/WatchHistoryView";
import homeSvg from "./assets/figma/home.svg";
import homeActiveSvg from "./assets/figma/home-active.svg";
import gridSquareSvg from "./assets/figma/grid-square.svg";
import playlistSvg from "./assets/figma/playlist.svg";
import signalStreamSvg from "./assets/figma/signal-stream.svg";
import signalStreamActiveSvg from "./assets/figma/signal-stream-active.svg";
import userHeartSvg from "./assets/figma/user-heart.svg";
import userCircleSvg from "./assets/figma/user-circle.svg";
import type {
  AppSettings,
  Channel,
  DnsMode,
  DnsTestResult,
  DownloadTask,
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

interface HomeVodFeed {
  scraperValue: string;
  scraperName: string;
  posts: VegaPost[];
}

const DEFAULT_SETTINGS: AppSettings = {
  blackTheme: false,
  startupTab: "home",
  dnsMode: "cloudflare",
  customDohUrl: "",
  developerTestMode: false,
  externalPlayerPath: "",
  downloadDir: "",
};

export function App() {
  const [route, setRoute] = useState<Route>("home");
  const [providers, setProviders] = useState<Provider[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [history, setHistory] = useState<WatchHistoryEntry[]>([]);
  const [homeFeed, setHomeFeed] = useState<HomeVodFeed | null>(null);
  const [heroIdx, setHeroIdx] = useState(0);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
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

  // Live TV filters
  const [channelQuery, setChannelQuery] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string>("All");

  // DNS test state
  const [dnsTest, setDnsTest] = useState<DnsTestResult | null>(null);
  const [testingDns, setTestingDns] = useState(false);

  // Selected VOD post for detail view
  const [selectedVod, setSelectedVod] = useState<{
    post: VegaPost;
    scraperValue: string;
  } | null>(null);

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
      const firstFilter = cat.catalog[0]?.filter ?? "";
      const posts = await api.getVodPosts(target.value, firstFilter, 1);
      setHomeFeed({
        scraperValue: target.value,
        scraperName: target.display_name || target.value,
        posts,
      });
      setHeroIdx(0);
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
    const q = channelQuery.trim().toLowerCase();
    return channels.filter((c) => {
      const catOk = selectedCategory === "All" || c.category === selectedCategory;
      const qOk =
        !q ||
        c.name.toLowerCase().includes(q) ||
        c.category.toLowerCase().includes(q);
      return catOk && qOk;
    });
  }, [channels, channelQuery, selectedCategory]);

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
        onSecondary: () => setRoute("movies"),
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
          }),
      })),
    [channels]
  );

  const vodTallCards: OttCardItem[] = useMemo(() => {
    if (!homeFeed) return [];
    return homeFeed.posts.slice(0, 18).map((post) => ({
      id: `${post.link}-${post.title}`,
      title: formatOttTitle(post.title),
      image: post.image,
      onClick: () => {
        setSelectedVod({ post, scraperValue: homeFeed.scraperValue });
        setRoute("movie-detail");
      },
    }));
  }, [homeFeed]);

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
      setInspectInfo(`Detected ${res.providerType} (${res.sourceKind})`);
    } catch (err) {
      setInspectInfo("");
      setProviderError(String(err));
    }
  }

  async function handleAddProvider(e: React.FormEvent) {
    e.preventDefault();
    const trimmedUrl = newUrl.trim();
    if (!trimmedUrl) {
      setProviderError("Please enter a playlist, Vega manifest, @handle, or video URL.");
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
            `Vega provider saved. Loaded ${exts.length} extension(s) — open Movies or Extensions.`
          );
          void refreshHomeFeed(updated);
        } catch (vegaErr) {
          setProviderSuccess("Vega provider saved.");
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
    const next = { ...settings, ...patch };
    setSettings(next);
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
          onClick={() => setRoute("movies")}
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

            {vodTallCards.length > 0 && (
              <OttCarousel
                title={homeFeed ? `${homeFeed.scraperName} Catalogue` : "Trending Movies & Series"}
                variant="tall"
                items={vodTallCards}
                headerAction={
                  <button
                    type="button"
                    className="ott-glass-pill"
                    onClick={() => setRoute("movies")}
                  >
                    Browse All
                  </button>
                }
              />
            )}
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
              <div style={{ marginBottom: 18 }}>
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
              <div style={{ marginBottom: 20 }}>
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
                  <div className="grid-channels">
                    {filteredChannels.slice(0, 120).map((ch) => (
                      <button
                        type="button"
                        key={`${ch.name}-${ch.streamUrl}`}
                        className="card channel-card"
                        onClick={() =>
                          setActivePlayback({
                            title: ch.name,
                            link: ch.streamUrl,
                            streamUrl: ch.streamUrl,
                            headers: ch.headers,
                            imageUrl: ch.logoUrl,
                            isLive: true,
                          })
                        }
                      >
                        <div
                          style={{
                            display: "flex",
                            justifyContent: "space-between",
                            alignItems: "center",
                            gap: 8,
                            marginBottom: 8,
                          }}
                        >
                          <span style={{ fontWeight: 600 }}>{ch.name}</span>
                          <Play size={14} color="#ffc062" />
                        </div>
                        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                          <span className="badge">{ch.category}</span>
                          <span className="muted">{ch.streamType}</span>
                        </div>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}

            {route === "movies" && (
              <MoviesView
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
              <div style={{ display: "grid", gap: 20 }}>
                <div className="page-header" style={{ marginBottom: 0 }}>
                  <h1 className="page-title">Providers</h1>
                </div>

                <form className="card" onSubmit={handleAddProvider} style={{ display: "grid", gap: 12 }}>
                  <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
                    <strong>{editingUrl ? "Edit Provider" : "Add Provider"}</strong>
                    <select
                      className="input"
                      style={{ width: 140, padding: "7px 10px" }}
                      value={detectedType}
                      onChange={(e) => {
                        setDetectedType(e.target.value as ProviderType);
                        setManualTypeOverride(true);
                      }}
                    >
                      <option value="IPTV">IPTV</option>
                      <option value="VEGA">VEGA</option>
                      <option value="VIDEO">VIDEO</option>
                    </select>
                  </div>
                  <input
                    className="input"
                    placeholder="Title (optional)"
                    value={newTitle}
                    onChange={(e) => setNewTitle(e.target.value)}
                  />
                  <input
                    className="input"
                    placeholder="Playlist / Vega Manifest / @vega-org / Video URL"
                    value={newUrl}
                    onChange={(e) => void handleUrlChange(e.target.value)}
                  />
                  <input
                    className="input"
                    placeholder="Custom User-Agent (optional)"
                    value={newUa}
                    onChange={(e) => setNewUa(e.target.value)}
                  />
                  {inspectInfo && <div className="muted">{inspectInfo}</div>}
                  {providerSuccess && (
                    <div style={{ color: "var(--status-live)", fontSize: 13 }}>{providerSuccess}</div>
                  )}
                  {providerError && <div className="error-text">{providerError}</div>}
                  <div style={{ display: "flex", flexWrap: "wrap", gap: 10 }}>
                    <button
                      type="button"
                      className="btn"
                      disabled={savingProvider}
                      onClick={() => void handleInspect()}
                    >
                      Inspect URL
                    </button>
                    <label className="btn" style={{ cursor: "pointer" }}>
                      <Upload size={14} /> Import .m3u / .json
                      <input
                        type="file"
                        accept=".m3u,.m3u8,.json,.txt"
                        style={{ display: "none" }}
                        onChange={(e) => void handleImportLocalFile(e)}
                      />
                    </label>
                    {editingUrl && (
                      <button
                        type="button"
                        className="btn"
                        onClick={() => {
                          setEditingUrl(null);
                          setNewTitle("");
                          setNewUrl("");
                          setNewUa("");
                          setManualTypeOverride(false);
                        }}
                      >
                        Cancel Edit
                      </button>
                    )}
                    <button
                      type="submit"
                      className="btn btn-primary"
                      disabled={savingProvider}
                    >
                      {savingProvider ? "Saving..." : editingUrl ? "Update Provider" : "Save Provider"}
                    </button>
                  </div>
                </form>

                <div style={{ display: "grid", gap: 10 }}>
                  {providers.map((p) => (
                    <div
                      key={p.url}
                      className="card"
                      style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}
                    >
                      <div style={{ minWidth: 0 }}>
                        <div style={{ fontWeight: 600 }}>
                          {p.title} <span className="badge">{p.type}</span>
                          {p.type === "IPTV" && (
                            <span className="muted" style={{ marginLeft: 8 }}>
                              {p.channelCount} channels
                            </span>
                          )}
                        </div>
                        <div className="muted" style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
                          {p.url}
                        </div>
                      </div>
                      <div style={{ display: "flex", gap: 8 }}>
                        {p.type === "VIDEO" && (
                          <button
                            type="button"
                            className="btn btn-primary"
                            onClick={() =>
                              setActivePlayback({
                                title: p.title,
                                link: p.url,
                                streamUrl: p.url,
                                headers: p.userAgent ? { "User-Agent": p.userAgent } : undefined,
                              })
                            }
                          >
                            <Play size={14} /> Play
                          </button>
                        )}
                        <button
                          type="button"
                          className="btn"
                          title="Edit"
                          onClick={() => {
                            setEditingUrl(p.url);
                            setNewTitle(p.title);
                            setNewUrl(p.url);
                            setNewUa(p.userAgent ?? "");
                            setDetectedType(p.type);
                            setManualTypeOverride(true);
                            setProviderError("");
                            setProviderSuccess("");
                          }}
                        >
                          <Pencil size={14} />
                        </button>
                        <button type="button" className="btn" onClick={() => void handleToggleProvider(p)}>
                          {p.isActive ? "Active" : "Paused"}
                        </button>
                        <button type="button" className="btn" onClick={() => void handleRemoveProvider(p.url)}>
                          <Trash2 size={14} />
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {route === "settings" && (
              <div style={{ display: "grid", gap: 16 }}>
                <h1 className="page-title">Preferences</h1>
                <div className="card" style={{ display: "grid", gap: 14 }}>
                  <label style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
                    <span>Pure AMOLED Black Canvas</span>
                    <input
                      type="checkbox"
                      className="apple-switch"
                      checked={settings.blackTheme}
                      onChange={(e) => void updateSettings({ blackTheme: e.target.checked })}
                    />
                  </label>
                  <label style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
                    <span>Developer Test Mode (allow LAN Vega servers)</span>
                    <input
                      type="checkbox"
                      className="apple-switch"
                      checked={settings.developerTestMode}
                      onChange={(e) => void updateSettings({ developerTestMode: e.target.checked })}
                    />
                  </label>
                  <label style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
                    <span>Startup Tab</span>
                    <select
                      className="input"
                      style={{ maxWidth: 200 }}
                      value={settings.startupTab}
                      onChange={(e) =>
                        void updateSettings({
                          startupTab: e.target.value as AppSettings["startupTab"],
                        })
                      }
                    >
                      <option value="home">Home</option>
                      <option value="television">Live TV</option>
                      <option value="movies">Movies</option>
                      <option value="category">Categories</option>
                    </select>
                  </label>
                </div>

                <div className="card" style={{ display: "grid", gap: 10 }}>
                  <div style={{ fontWeight: 600 }}>External Player Executable (Optional)</div>
                  <input
                    className="input"
                    placeholder="Auto-detect VLC / mpv, or paste custom path"
                    value={settings.externalPlayerPath}
                    onChange={(e) => void updateSettings({ externalPlayerPath: e.target.value })}
                  />
                  <div style={{ fontWeight: 600, marginTop: 6 }}>Download Directory (Optional)</div>
                  <input
                    className="input"
                    placeholder="Default: Videos/IPTVMine Pro"
                    value={settings.downloadDir}
                    onChange={(e) => void updateSettings({ downloadDir: e.target.value })}
                  />
                </div>

                <div className="card" style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <div>
                    <div style={{ fontWeight: 600 }}>VOD & Extension Cache</div>
                    <div className="muted">Clear cached manifests, categories, and movie lists</div>
                  </div>
                  <button type="button" className="btn" onClick={() => void api.clearVodCache()}>
                    Clear VOD Cache
                  </button>
                </div>
              </div>
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
              <div style={{ display: "grid", gap: 16 }}>
                <div className="page-header" style={{ marginBottom: 0 }}>
                  <h1 className="page-title">DNS over HTTPS</h1>
                </div>
                <div className="card" style={{ display: "grid", gap: 10 }}>
                  {(["cloudflare", "google", "adguard", "custom", "off"] as DnsMode[]).map((mode) => (
                    <label key={mode} style={{ display: "flex", gap: 10, alignItems: "center" }}>
                      <input
                        type="radio"
                        name="dnsMode"
                        checked={settings.dnsMode === mode}
                        onChange={() => void updateSettings({ dnsMode: mode })}
                      />
                      <span style={{ textTransform: "capitalize" }}>{mode}</span>
                    </label>
                  ))}
                  {settings.dnsMode === "custom" && (
                    <input
                      className="input"
                      placeholder="https://dns.example.com/dns-query"
                      value={settings.customDohUrl}
                      onChange={(e) => void updateSettings({ customDohUrl: e.target.value })}
                    />
                  )}
                  <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={testingDns}
                      onClick={() => void runDnsTest(settings.dnsMode)}
                    >
                      {testingDns ? "Testing..." : "Test DNS Latency"}
                    </button>
                    {dnsTest && (
                      <span className="muted">
                        {dnsTest.success ? `Connected (${dnsTest.latencyMs} ms)` : "Lookup failed"}
                      </span>
                    )}
                  </div>
                </div>
              </div>
            )}
          </div>
        )}
      </main>

      {activePlayback && (
        <PlayerView
          request={activePlayback}
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
