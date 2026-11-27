import { useCallback, useEffect, useId, useRef, useState } from "react";
import {
  Check,
  Compass,
  Cpu,
  Database,
  Download,
  ExternalLink,
  Film,
  FolderDown,
  Moon,
  RefreshCw,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import { api } from "../api";
import type { AppSettings, FfmpegStatus, StartupTab } from "../types";

const STARTUP_TABS: { value: StartupTab; label: string }[] = [
  { value: "home", label: "Home" },
  { value: "television", label: "Live TV" },
  { value: "movies", label: "Movies" },
  { value: "category", label: "Categories" },
];

interface Props {
  settings: AppSettings;
  ffmpegStatus: FfmpegStatus | null;
  installingFfmpeg: boolean;
  ffmpegError: string;
  onUpdateSettings: (patch: Partial<AppSettings>) => Promise<void>;
  onFfmpegStatusChange: (status: FfmpegStatus) => void;
  onInstallingChange: (installing: boolean) => void;
  onFfmpegErrorChange: (msg: string) => void;
}

export function PreferencesView({
  settings,
  ffmpegStatus,
  installingFfmpeg,
  ffmpegError,
  onUpdateSettings,
  onFfmpegStatusChange,
  onInstallingChange,
  onFfmpegErrorChange,
}: Props) {
  const headingId = useId();
  const extPlayerId = useId();
  const downloadDirId = useId();
  const ffmpegPathId = useId();

  const [defaultDownloadDir, setDefaultDownloadDir] = useState("");
  const [checkingFfmpeg, setCheckingFfmpeg] = useState(false);
  const [ffmpegCheckMsg, setFfmpegCheckMsg] = useState("");
  const [clearingCache, setClearingCache] = useState(false);
  const [cacheCleared, setCacheCleared] = useState(false);
  const cacheTimerRef = useRef<number | null>(null);
  const ffmpegMsgTimerRef = useRef<number | null>(null);

  useEffect(() => {
    let active = true;
    void api
      .getDefaultDownloadDir()
      .then((dir) => {
        if (active && dir) setDefaultDownloadDir(dir);
      })
      .catch(() => {});
    return () => {
      active = false;
      if (cacheTimerRef.current !== null) window.clearTimeout(cacheTimerRef.current);
      if (ffmpegMsgTimerRef.current !== null) window.clearTimeout(ffmpegMsgTimerRef.current);
    };
  }, [settings.downloadDir]);

  const handleStartupKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLButtonElement>, idx: number) => {
      if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
      e.preventDefault();
      const delta = e.key === "ArrowRight" ? 1 : -1;
      const nextIdx = (idx + delta + STARTUP_TABS.length) % STARTUP_TABS.length;
      void onUpdateSettings({ startupTab: STARTUP_TABS[nextIdx].value });
      const group = e.currentTarget.closest('[role="radiogroup"]');
      const nextBtn = group?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[nextIdx];
      nextBtn?.focus();
    },
    [onUpdateSettings]
  );

  async function handleCheckFfmpeg() {
    setCheckingFfmpeg(true);
    setFfmpegCheckMsg("");
    onFfmpegErrorChange("");
    try {
      const next = await api.checkFfmpegUpstreamStatus();
      onFfmpegStatusChange(next);
      const summary = next.installed
        ? next.updateAvailable && next.upstreamVersion
          ? `Update v${next.upstreamVersion} is ready to install`
          : `FFmpeg${next.shortVersion ? ` v${next.shortVersion}` : ""} is up to date`
        : next.upstreamVersion
          ? `Latest upstream release: v${next.upstreamVersion}`
          : "Status refreshed";
      setFfmpegCheckMsg(summary);
      if (ffmpegMsgTimerRef.current !== null) window.clearTimeout(ffmpegMsgTimerRef.current);
      ffmpegMsgTimerRef.current = window.setTimeout(() => setFfmpegCheckMsg(""), 4500);
    } catch (err) {
      onFfmpegErrorChange(String(err));
    } finally {
      setCheckingFfmpeg(false);
    }
  }

  async function handleInstallFfmpeg() {
    onInstallingChange(true);
    setFfmpegCheckMsg("");
    onFfmpegErrorChange("");
    try {
      const next = await api.installFfmpegRuntime();
      onFfmpegStatusChange(next);
      setFfmpegCheckMsg(
        next.shortVersion ? `Configured FFmpeg v${next.shortVersion}` : "FFmpeg runtime verified"
      );
      if (ffmpegMsgTimerRef.current !== null) window.clearTimeout(ffmpegMsgTimerRef.current);
      ffmpegMsgTimerRef.current = window.setTimeout(() => setFfmpegCheckMsg(""), 4500);
    } catch (err) {
      onFfmpegErrorChange(String(err));
    } finally {
      onInstallingChange(false);
    }
  }

  async function handleClearCache() {
    setClearingCache(true);
    try {
      await api.clearVodCache();
      setCacheCleared(true);
      if (cacheTimerRef.current !== null) window.clearTimeout(cacheTimerRef.current);
      cacheTimerRef.current = window.setTimeout(() => setCacheCleared(false), 3200);
    } finally {
      setClearingCache(false);
    }
  }

  const busyFfmpeg = installingFfmpeg || Boolean(ffmpegStatus?.installing) || checkingFfmpeg;

  return (
    <section className="pref-screen" aria-labelledby={headingId}>
      <header className="pref-header">
        <div className="pref-title-block">
          <h1 id={headingId} className="pref-title">
            Preferences
          </h1>
          <p className="pref-subtitle">
            Personalize appearance, launch screen, external player & download folders, and media transcoding.
          </p>
        </div>

        <div className="pref-summary-pill">
          <span
            className={`pref-summary-dot ${
              settings.ffmpegEnabled && ffmpegStatus?.installed ? "is-live" : "is-amber"
            }`}
            aria-hidden="true"
          />
          <span>{settings.blackTheme ? "AMOLED Canvas" : "Dark Glass"}</span>
          <span className="pref-summary-sep" aria-hidden="true" />
          <span>
            {!settings.ffmpegEnabled
              ? "FFmpeg Off"
              : ffmpegStatus?.installed
                ? `FFmpeg ${ffmpegStatus.shortVersion ? `v${ffmpegStatus.shortVersion}` : "Ready"}`
                : "FFmpeg Pending"}
          </span>
        </div>
      </header>

      {/* Group 1: Appearance & Startup */}
      <div className="pref-section">
        <div className="pref-section-label">Appearance & Startup</div>
        <div className="pref-group">
          <div className="pref-row">
            <span className={`pref-row-icon ${settings.blackTheme ? "is-active" : ""}`} aria-hidden="true">
              <Moon size={17} strokeWidth={2} />
            </span>
            <div className="pref-row-copy">
              <label htmlFor="pref-amoled-toggle" className="pref-row-title">
                Pure AMOLED Black Canvas
              </label>
              <span className="pref-row-desc">
                Use true #000000 background surfaces for OLED displays and darker viewing rooms.
              </span>
            </div>
            <input
              id="pref-amoled-toggle"
              type="checkbox"
              className="apple-switch"
              checked={settings.blackTheme}
              onChange={(e) => void onUpdateSettings({ blackTheme: e.target.checked })}
            />
          </div>

          <div className="pref-row is-wrap">
            <span className="pref-row-icon" aria-hidden="true">
              <Compass size={17} strokeWidth={2} />
            </span>
            <div className="pref-row-copy">
              <div className="pref-row-title">Startup Tab</div>
              <span className="pref-row-desc">
                Choose the default section shown immediately when IPTVMine Desktop opens.
              </span>
            </div>
            <div
              className="pref-seg-control"
              role="radiogroup"
              aria-label="Startup Tab"
            >
              {STARTUP_TABS.map((tab, idx) => {
                const active = settings.startupTab === tab.value;
                return (
                  <button
                    key={tab.value}
                    type="button"
                    role="radio"
                    aria-checked={active}
                    className={`pref-seg-btn ${active ? "active" : ""}`}
                    onClick={() => void onUpdateSettings({ startupTab: tab.value })}
                    onKeyDown={(e) => handleStartupKeyDown(e, idx)}
                  >
                    {tab.label}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="pref-row">
            <span
              className={`pref-row-icon ${settings.developerTestMode ? "is-active" : ""}`}
              aria-hidden="true"
            >
              <Terminal size={17} strokeWidth={2} />
            </span>
            <div className="pref-row-copy">
              <label htmlFor="pref-dev-toggle" className="pref-row-title">
                Developer Test Mode
              </label>
              <span className="pref-row-desc">
                Allow LAN and loopback HTTP Vega manifests during local extension development.
              </span>
            </div>
            <input
              id="pref-dev-toggle"
              type="checkbox"
              className="apple-switch"
              checked={settings.developerTestMode}
              onChange={(e) => void onUpdateSettings({ developerTestMode: e.target.checked })}
            />
          </div>
        </div>
      </div>

      {/* Group 2: Media Paths */}
      <div className="pref-section">
        <div className="pref-section-label">Playback & Storage Paths</div>
        <div className="pref-group">
          <div className="pref-row is-field">
            <span className="pref-row-icon" aria-hidden="true">
              <Film size={17} strokeWidth={2} />
            </span>
            <div className="pref-field-body">
              <label htmlFor={extPlayerId} className="pref-row-title">
                External Player Executable (Optional)
              </label>
              <span className="pref-row-desc">
                Leave empty to auto-detect VLC or mpv, or paste a custom player executable path.
              </span>
              <ClearablePathInput
                id={extPlayerId}
                value={settings.externalPlayerPath}
                placeholder="Auto-detect VLC / mpv, or paste custom path"
                clearLabel="Clear external player path"
                onChange={(externalPlayerPath) => void onUpdateSettings({ externalPlayerPath })}
              />
            </div>
          </div>

          <div className="pref-row is-field">
            <span className="pref-row-icon" aria-hidden="true">
              <FolderDown size={17} strokeWidth={2} />
            </span>
            <div className="pref-field-body">
              <label htmlFor={downloadDirId} className="pref-row-title">
                Download Directory (Optional)
              </label>
              <span className="pref-row-desc">
                {defaultDownloadDir
                  ? `Active folder: ${defaultDownloadDir}`
                  : "Default: Videos/IPTVMine Pro"}
              </span>
              <ClearablePathInput
                id={downloadDirId}
                value={settings.downloadDir}
                placeholder={defaultDownloadDir || "Default: Videos/IPTVMine Pro"}
                clearLabel="Reset download directory to default"
                onChange={(downloadDir) => void onUpdateSettings({ downloadDir })}
              />
            </div>
          </div>
        </div>
      </div>

      {/* Group 3: FFmpeg Media Engine */}
      <div className="pref-section">
        <div className="pref-section-label">Media Engine</div>
        <div className="pref-group">
          <div className="pref-row">
            <span
              className={`pref-row-icon ${settings.ffmpegEnabled ? "is-active" : ""}`}
              aria-hidden="true"
            >
              <Cpu size={17} strokeWidth={2} />
            </span>
            <div className="pref-row-copy">
              <div className="pref-row-head">
                <span className="pref-row-title">FFmpeg Media Engine</span>
                {!settings.ffmpegEnabled ? (
                  <span className="ott-meta-pill">Disabled</span>
                ) : installingFfmpeg || ffmpegStatus?.installing ? (
                  <span className="ott-meta-pill amber">Auto-Installing Latest...</span>
                ) : ffmpegStatus?.installed ? (
                  <span className="ott-meta-pill amber">
                    Ready {ffmpegStatus.shortVersion ? `v${ffmpegStatus.shortVersion}` : ""} • {ffmpegStatus.source}
                  </span>
                ) : (
                  <span className="ott-meta-pill">Auto-Install Pending</span>
                )}
                {ffmpegStatus?.upstreamVersion && (
                  <a
                    href={ffmpegStatus.upstreamUrl || "https://github.com/FFmpeg/FFmpeg"}
                    target="_blank"
                    rel="noreferrer"
                    className="ott-meta-pill pref-ext-pill"
                  >
                    <span>Upstream {ffmpegStatus.upstreamTag || `v${ffmpegStatus.upstreamVersion}`}</span>
                    <ExternalLink size={10} aria-hidden="true" />
                  </a>
                )}
                {ffmpegStatus?.updateAvailable && ffmpegStatus.upstreamVersion && (
                  <span className="ott-meta-pill amber">
                    Update v{ffmpegStatus.upstreamVersion}
                  </span>
                )}
              </div>
              <span className="pref-row-desc">
                {!settings.ffmpegEnabled
                  ? "FFmpeg auto-remuxing, multi-audio probing, subtitles & HLS downloads are paused."
                  : installingFfmpeg || ffmpegStatus?.installing
                    ? "Automatically downloading and configuring latest FFmpeg in the background."
                    : ffmpegStatus?.installed
                      ? ffmpegStatus.version || ffmpegStatus.ffmpegPath || "Configured and active."
                      : "Enables MKV/AC3/DTS/RTMP remuxing, multi-audio tracks, subtitles, and HLS downloads."}
              </span>
            </div>
            <div className="pref-switch-cluster">
              <span className="pref-switch-hint">Default On</span>
              <input
                type="checkbox"
                aria-label="Enable FFmpeg Media Engine by default"
                className="apple-switch"
                checked={settings.ffmpegEnabled}
                onChange={(e) => void onUpdateSettings({ ffmpegEnabled: e.target.checked })}
              />
            </div>
          </div>

          <div className="pref-row is-actions">
            <div className="pref-action-buttons">
              <button
                type="button"
                className="btn"
                disabled={busyFfmpeg}
                onClick={() => void handleCheckFfmpeg()}
              >
                <RefreshCw size={14} className={checkingFfmpeg ? "dns-spin" : ""} />
                <span>{checkingFfmpeg ? "Checking Upstream..." : "Check FFmpeg/FFmpeg"}</span>
              </button>
              <button
                type="button"
                className="btn btn-primary"
                disabled={busyFfmpeg}
                onClick={() => void handleInstallFfmpeg()}
              >
                <Download size={14} />
                <span>
                  {installingFfmpeg || ffmpegStatus?.installing
                    ? "Installing Latest..."
                    : ffmpegStatus?.installed
                      ? ffmpegStatus.updateAvailable && ffmpegStatus.upstreamVersion
                        ? `Update to v${ffmpegStatus.upstreamVersion}`
                        : "Reinstall / Verify Latest"
                      : `Install Latest${ffmpegStatus?.upstreamVersion ? ` (v${ffmpegStatus.upstreamVersion})` : ""}`}
                </span>
              </button>
            </div>
            {ffmpegCheckMsg && !ffmpegError && (
              <span className="pref-feedback is-ok" role="status">
                <Check size={14} /> {ffmpegCheckMsg}
              </span>
            )}
            {ffmpegError && (
              <div className="pref-feedback is-error" role="alert">
                {ffmpegError}
              </div>
            )}
          </div>

          <div className="pref-row is-field">
            <span className="pref-row-icon is-spacer" aria-hidden="true" />
            <div className="pref-field-body">
              <label htmlFor={ffmpegPathId} className="pref-row-title">
                Custom FFmpeg Path (Optional)
              </label>
              <span className="pref-row-desc">
                Auto-managed by default, or point to a local ffmpeg.exe path or bin directory.
              </span>
              <ClearablePathInput
                id={ffmpegPathId}
                value={settings.ffmpegPath}
                placeholder="Auto-installed by default, or paste custom ffmpeg.exe path / bin dir"
                clearLabel="Clear custom FFmpeg path"
                onChange={async (ffmpegPath) => {
                  await onUpdateSettings({ ffmpegPath });
                  void api
                    .checkFfmpegUpstreamStatus()
                    .then(onFfmpegStatusChange)
                    .catch(() => {
                      void api.getFfmpegStatus().then(onFfmpegStatusChange).catch(() => {});
                    });
                }}
              />
            </div>
          </div>
        </div>
      </div>

      {/* Group 4: Storage & Cache */}
      <div className="pref-section">
        <div className="pref-section-label">Storage & Cache</div>
        <div className="pref-group">
          <div className="pref-row">
            <span className="pref-row-icon" aria-hidden="true">
              <Database size={17} strokeWidth={2} />
            </span>
            <div className="pref-row-copy">
              <div className="pref-row-title">VOD & Extension Cache</div>
              <span className="pref-row-desc">
                Clear cached extension manifests, catalog feeds, categories, and movie lists.
              </span>
            </div>
            <div className="pref-cache-actions">
              {cacheCleared && (
                <span className="pref-feedback is-ok" role="status">
                  <Check size={14} /> Cleared
                </span>
              )}
              <button
                type="button"
                className="btn"
                disabled={clearingCache}
                onClick={() => void handleClearCache()}
              >
                <Trash2 size={14} />
                <span>{clearingCache ? "Clearing..." : "Clear VOD Cache"}</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function ClearablePathInput({
  id,
  value,
  placeholder,
  clearLabel,
  onChange,
}: {
  id: string;
  value: string;
  placeholder: string;
  clearLabel: string;
  onChange: (value: string) => void | Promise<void>;
}) {
  return (
    <div className="pref-input-wrap">
      <input
        id={id}
        className="input pref-path-input"
        placeholder={placeholder}
        value={value}
        onChange={(e) => void onChange(e.target.value)}
      />
      {value.length > 0 && (
        <button
          type="button"
          className="dns-clear-btn"
          aria-label={clearLabel}
          onClick={() => void onChange("")}
        >
          <X size={14} />
        </button>
      )}
    </div>
  );
}
