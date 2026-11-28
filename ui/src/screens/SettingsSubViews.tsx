import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import {
  ArrowLeft,
  Bug,
  Check,
  ChevronRight,
  Copy,
  Cpu,
  Database,
  ExternalLink,
  FileText,
  Film,
  Keyboard,
  Monitor,
  Radio,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Tv,
} from "lucide-react";
import { api } from "../api";
import appIconPng from "../assets/drawable/app-icon.png";
import type { UpdateCheckResult } from "../types";

interface AboutProps {
  onBack: () => void;
  onOpenProviders?: () => void;
  onOpenLiveTv?: () => void;
  onOpenMovies?: () => void;
  onOpenDns?: () => void;
}

interface GuideStep {
  id: string;
  icon: ReactNode;
  highlight?: boolean;
  title: string;
  desc: string;
  cta?: string;
  onAction?: () => void;
}

/**
 * Displays IPTVMine Desktop identity, step-by-step IPTV playback instructions, player shortcuts, and updates.
 */
export function AboutView({
  onBack,
  onOpenProviders,
  onOpenLiveTv,
  onOpenMovies,
  onOpenDns,
}: AboutProps) {
  const headingId = useId();
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<UpdateCheckResult | null>(null);
  const [error, setError] = useState("");

  async function checkUpdates() {
    setChecking(true);
    setError("");
    try {
      setResult(await api.checkForUpdates());
    } catch (err) {
      setError(String(err));
    } finally {
      setChecking(false);
    }
  }

  const guideSteps: GuideStep[] = [
    {
      id: "add-source",
      icon: <Tv size={18} />,
      highlight: true,
      title: "1. Add an IPTV Playlist or Stream Source",
      desc: "Open Providers to paste an M3U / M3U8 or IPTV JSON playlist URL, import a local .m3u / .m3u8 / .json file from disk, connect a Source repository (@handle or GitHub URL), or enter a direct stream link.",
      cta: "Providers",
      onAction: onOpenProviders,
    },
    {
      id: "watch-live",
      icon: <Radio size={18} />,
      title: "2. Browse & Play Live TV Channels",
      desc: "Go to Live TV or Live Categories to search channels by name or filter by category. Click any channel card to stream HLS, DASH, RTMP, or MPEG-TS feeds smoothly through the local proxy.",
      cta: "Live TV",
      onAction: onOpenLiveTv,
    },
    {
      id: "watch-vod",
      icon: <Film size={18} />,
      title: "3. Stream Movies, Series & Save Offline",
      desc: "Install sources inside Extensions, browse Movies & Series, choose a quality mirror or external player (VLC / mpv), or click Download for offline viewing and Watch History resume.",
      cta: "Movies",
      onAction: onOpenMovies,
    },
    {
      id: "doh-bypass",
      icon: <ShieldCheck size={18} />,
      title: "4. Bypass Blocked Hosts with DoH DNS",
      desc: "If an IPTV playlist or media host fails to resolve on ISP DNS, switch DNS-over-HTTPS in DoH DNS (Cloudflare, Google, AdGuard, or a custom resolver URL).",
      cta: "DoH DNS",
      onAction: onOpenDns,
    },
  ];

  const capabilities = [
    {
      id: "engine",
      icon: <Cpu size={18} />,
      title: "Playback Engine & Formats",
      desc: "Supports M3U / EXTM3U, IPTV JSON, HLS (.m3u8), MPEG-DASH (.mpd), MP4, MKV, AC3/DTS & RTMP remuxing via built-in FFmpeg and parallel HTTP Range caching.",
    },
    {
      id: "shortcuts",
      icon: <Keyboard size={18} />,
      title: "Player Keyboard Controls",
      desc: "Space / K play-pause • Left / Right (±10s) & J / L (±30s) seek • Up / Down & M volume/mute • F fullscreen • P pin window • N next episode • ? help • Esc close.",
    },
    {
      id: "privacy",
      icon: <Database size={18} />,
      title: "Local Storage & Zero Telemetry",
      desc: "All playlists, extensions, watch progress, download tasks, and settings live locally on your computer in SQLite with no tracking or analytics.",
    },
  ];

  const platforms = [
    { name: "Windows x64", arch: "x86_64", pkg: ".msi / .exe (NSIS)", status: "Stable" },
    { name: "Windows ARM64", arch: "aarch64", pkg: ".exe (NSIS)", status: "Stable" },
    { name: "macOS Universal", arch: "Intel + Apple Silicon", pkg: ".dmg / .app", status: "Stable" },
    { name: "Linux Debian / Ubuntu x64", arch: "x86_64", pkg: ".deb", status: "Stable" },
    { name: "Linux RPM x64", arch: "x86_64", pkg: ".rpm (Fedora / RHEL / openSUSE)", status: "Stable" },
    { name: "Linux AppImage x64", arch: "x86_64", pkg: ".AppImage (Portable)", status: "Stable" },
    { name: "LG webOS 23+", arch: "Smart TV Web App", pkg: ".ipk + D-pad navigation", status: "Experimental" },
    { name: "Samsung Tizen TV 8.0+", arch: "Smart TV Widget", pkg: ".wgt + remote support", status: "Experimental" },
  ] as const;

  return (
    <section className="pref-screen" aria-labelledby={headingId}>
      <header className="pref-header">
        <div className="about-brand-head">
          <img
            className="about-brand-logo"
            src={appIconPng}
            alt="IPTVMine Desktop Logo"
            width={60}
            height={60}
          />
          <div className="pref-title-block">
            <h1 id={headingId} className="pref-title">
              About IPTVMine Desktop
            </h1>
            <p className="pref-subtitle">
              Desktop IPTV, Live TV, Movie &amp; Series streaming client built with Rust, Tauri v2, and React.
            </p>
          </div>
        </div>

        <div className="pref-summary-pill">
          <span
            className={`pref-summary-dot ${result?.updateAvailable ? "is-amber" : "is-live"}`}
            aria-hidden="true"
          />
          <span>
            {result
              ? result.updateAvailable
                ? `Update v${result.latestVersion}`
                : `v0.1.0 • Up to date`
              : "v0.1.0 • Desktop"}
          </span>
        </div>
      </header>

      <div className="pref-section">
        <div className="pref-section-label">How to Play IPTV &amp; Streams</div>
        <div className="pref-group">
          {guideSteps.map((step) => (
            <div key={step.id} className="pref-row">
              <div
                className={`pref-row-icon ${step.highlight ? "is-active" : ""}`}
                aria-hidden="true"
              >
                {step.icon}
              </div>
              <div className="pref-row-copy">
                <div className="pref-row-head">
                  <span className="pref-row-title">{step.title}</span>
                </div>
                <div className="pref-row-desc">{step.desc}</div>
              </div>
              {step.onAction && step.cta ? (
                <button
                  type="button"
                  className="btn about-nav-btn"
                  onClick={step.onAction}
                >
                  <span>{step.cta}</span>
                  <ChevronRight size={14} aria-hidden="true" />
                </button>
              ) : (
                <span aria-hidden="true" />
              )}
            </div>
          ))}
        </div>
      </div>

      <div className="pref-section">
        <div className="pref-section-label">Player Engine &amp; Controls</div>
        <div className="pref-group">
          {capabilities.map((cap) => (
            <div key={cap.id} className="pref-row">
              <div className="pref-row-icon" aria-hidden="true">
                {cap.icon}
              </div>
              <div className="pref-row-copy">
                <div className="pref-row-title">{cap.title}</div>
                <div className="pref-row-desc">{cap.desc}</div>
              </div>
              <span aria-hidden="true" />
            </div>
          ))}
        </div>
      </div>

      <div className="pref-section">
        <div className="pref-section-label">Supported Platform Architectures</div>
        <div className="pref-group">
          {platforms.map((plat) => (
            <div key={plat.name} className="pref-row">
              <div
                className={`pref-row-icon ${plat.status === "Stable" ? "is-active" : ""}`}
                aria-hidden="true"
              >
                {plat.arch.startsWith("Smart TV") ? <Tv size={18} /> : <Monitor size={18} />}
              </div>
              <div className="pref-row-copy">
                <div className="pref-row-head">
                  <span className="pref-row-title">{plat.name}</span>
                  <span
                    className={`ott-meta-pill ${plat.status === "Experimental" ? "amber" : ""}`}
                  >
                    {plat.status}
                  </span>
                </div>
                <div className="pref-row-desc">
                  {plat.arch} • {plat.pkg}
                </div>
              </div>
              <span aria-hidden="true" />
            </div>
          ))}
        </div>
      </div>

      <div className="pref-section">
        <div className="pref-section-label">Version &amp; Updates</div>
        <div className="pref-group">
          <div className="pref-row is-wrap">
            <div className="pref-row-icon is-active" aria-hidden="true">
              <Sparkles size={18} />
            </div>
            <div className="pref-row-copy">
              <div className="pref-row-title">IPTVMine Desktop v0.1.0</div>
              <div className="pref-row-desc">
                Rust + Tauri v2 + React 18 + TypeScript • QuickJS Source Runtime
              </div>
            </div>
            <div className="pref-action-buttons">
              <button
                type="button"
                className="btn btn-primary about-nav-btn"
                disabled={checking}
                onClick={() => void checkUpdates()}
              >
                <RefreshCw
                  size={14}
                  className={checking ? "is-spinning" : ""}
                  aria-hidden="true"
                />
                <span>{checking ? "Checking..." : "Check for Updates"}</span>
              </button>
              <button
                type="button"
                className="btn about-nav-btn"
                onClick={onBack}
              >
                <ArrowLeft size={14} aria-hidden="true" />
                <span>Preferences</span>
              </button>
            </div>
          </div>

          {(error || result) && (
            <div className="pref-row is-actions">
              {error && <div className="pref-feedback is-error">{error}</div>}
              {result && (
                <div className="about-update-card">
                  <div className="pref-row-head">
                    <span className="pref-row-title">
                      {result.updateAvailable
                        ? `Update Available: v${result.latestVersion}`
                        : `Up to date (latest v${result.latestVersion})`}
                    </span>
                    {result.releaseName && (
                      <span className="pref-row-desc">{result.releaseName}</span>
                    )}
                  </div>
                  {result.releaseNotes && (
                    <pre className="about-release-notes">{result.releaseNotes}</pre>
                  )}
                  {result.downloadUrl && (
                    <div>
                      <a
                        className="btn btn-primary pref-ext-pill"
                        href={result.downloadUrl}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <ExternalLink size={14} aria-hidden="true" />
                        <span>Download {result.assetName || "Installer"}</span>
                      </a>
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

interface DiagnosticMetric {
  label: string;
  value: string;
}

function parseDiagnosticMetrics(raw: string): DiagnosticMetric[] {
  const rows: DiagnosticMetric[] = [];
  for (const line of raw.split(/\r?\n/)) {
    const idx = line.indexOf(":");
    if (idx <= 0) continue;
    const label = line.slice(0, idx).trim();
    const value = line.slice(idx + 1).trim();
    if (label && value) {
      rows.push({ label, value });
    }
  }
  return rows;
}

/**
 * Apple System Settings style redacted system diagnostics and GitHub issue submission view.
 */
export function BugReportView({ onBack }: { onBack: () => void }) {
  const headingId = useId();
  const [diagnostics, setDiagnostics] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const copyTimerRef = useRef<number | null>(null);

  const loadDiagnostics = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const bundle = await api.getDiagnosticsBundle();
      setDiagnostics(bundle);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadDiagnostics();
    return () => {
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    };
  }, [loadDiagnostics]);

  const metrics = useMemo(() => parseDiagnosticMetrics(diagnostics), [diagnostics]);

  const issueHref = useMemo(() => {
    const body = diagnostics
      ? `## Environment\n\`\`\`text\n${diagnostics.trim()}\n\`\`\`\n\n## Steps to Reproduce\n1. \n\n## Expected Behavior\n\n## Actual Behavior\n`
      : "";
    const params = new URLSearchParams();
    params.set("title", "[Bug]: ");
    if (body) params.set("body", body);
    return `https://github.com/samyak2403/IPTVMine-Desktop/issues/new?${params.toString()}`;
  }, [diagnostics]);

  async function copyDiagnostics() {
    if (!diagnostics) return;
    try {
      await navigator.clipboard.writeText(diagnostics);
    } catch {
      const area = document.createElement("textarea");
      area.value = diagnostics;
      area.setAttribute("readonly", "");
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      document.execCommand("copy");
      document.body.removeChild(area);
    }
    setCopied(true);
    if (copyTimerRef.current !== null) {
      window.clearTimeout(copyTimerRef.current);
    }
    copyTimerRef.current = window.setTimeout(() => setCopied(false), 2200);
  }

  return (
    <section className="pref-screen" aria-labelledby={headingId}>
      <header className="pref-header">
        <div className="pref-title-block">
          <h1 id={headingId} className="pref-title">
            Diagnostics &amp; Bug Report
          </h1>
          <p className="pref-subtitle">
            Review your redacted runtime snapshot before copying logs or opening a GitHub bug report.
          </p>
        </div>

        <div className="pref-summary-pill">
          <span
            className={`pref-summary-dot ${error || loading ? "is-amber" : "is-live"}`}
            aria-hidden="true"
          />
          <span>
            {loading
              ? "Collecting Snapshot..."
              : error
                ? "Diagnostics Unavailable"
                : `${metrics.length} Fields • Redacted`}
          </span>
        </div>
      </header>

      <div className="pref-section">
        <div className="pref-section-label">Runtime Snapshot &amp; Privacy</div>
        <div className="pref-group">
          <div className="pref-row">
            <div className="pref-row-icon is-active" aria-hidden="true">
              <ShieldCheck size={18} />
            </div>
            <div className="pref-row-copy">
              <div className="pref-row-title">Automatic Privacy Redaction</div>
              <div className="pref-row-desc">
                All playlist URLs, custom User-Agent strings, headers, and cookies are stripped locally before generating this summary.
              </div>
            </div>
            <button
              type="button"
              className="btn about-nav-btn"
              onClick={onBack}
            >
              <ArrowLeft size={14} aria-hidden="true" />
              <span>Preferences</span>
            </button>
          </div>

          {metrics.length > 0 && (
            <div className="pref-row is-actions">
              <div className="diag-metrics-grid">
                {metrics.map((m) => (
                  <div key={m.label} className="diag-metric-card">
                    <span className="diag-metric-label">{m.label}</span>
                    <span className="diag-metric-value" title={m.value}>
                      {m.value}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>

      <div className="pref-section">
        <div className="pref-section-label">Diagnostic Bundle &amp; Issue Submission</div>
        <div className="pref-group">
          <div className="pref-row is-wrap">
            <div className="pref-row-icon" aria-hidden="true">
              <FileText size={18} />
            </div>
            <div className="pref-row-copy">
              <div className="pref-row-title">Redacted Diagnostic Summary</div>
              <div className="pref-row-desc">
                Copy this plain-text bundle to attach environment details cleanly to any support thread.
              </div>
            </div>
            <div className="pref-action-buttons">
              <button
                type="button"
                className="btn about-nav-btn"
                disabled={loading}
                onClick={() => void loadDiagnostics()}
              >
                <RefreshCw
                  size={14}
                  className={loading ? "is-spinning" : ""}
                  aria-hidden="true"
                />
                <span>{loading ? "Refreshing..." : "Refresh"}</span>
              </button>
              <button
                type="button"
                className="btn btn-primary about-nav-btn"
                disabled={loading || !diagnostics}
                onClick={() => void copyDiagnostics()}
              >
                {copied ? (
                  <Check size={14} aria-hidden="true" />
                ) : (
                  <Copy size={14} aria-hidden="true" />
                )}
                <span>{copied ? "Copied!" : "Copy Diagnostics"}</span>
              </button>
            </div>
          </div>

          <div className="pref-row is-actions">
            {error ? (
              <div className="pref-feedback is-error" role="alert">
                Failed to load diagnostics: {error}
              </div>
            ) : (
              <pre className="diag-code-box" aria-label="Redacted diagnostic bundle">
                {diagnostics || "Collecting redacted system diagnostics..."}
              </pre>
            )}
          </div>

          <div className="pref-row is-wrap">
            <div className="pref-row-icon is-active" aria-hidden="true">
              <Bug size={18} />
            </div>
            <div className="pref-row-copy">
              <div className="pref-row-title">Open a Bug Report on GitHub</div>
              <div className="pref-row-desc">
                Launches GitHub Issues pre-filled with your redacted environment details so issues can be reproduced quickly.
              </div>
            </div>
            <div className="pref-action-buttons">
              <a
                className="btn about-nav-btn pref-ext-pill"
                href={issueHref}
                target="_blank"
                rel="noreferrer"
              >
                <ExternalLink size={14} aria-hidden="true" />
                <span>Open GitHub Issue</span>
              </a>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
