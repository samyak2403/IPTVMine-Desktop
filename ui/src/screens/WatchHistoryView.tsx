import { useEffect, useId, useMemo, useState } from "react";
import { Clock, Film, Play, Search, Trash2, X } from "lucide-react";
import { api } from "../api";
import { formatOttTitle } from "../components/OttUi";
import type { PlaybackRequest, WatchHistoryEntry } from "../types";

interface Props {
  onResume: (req: PlaybackRequest) => void;
}

function formatDurationMs(ms: number): string {
  const totalSec = Math.max(0, Math.floor(ms / 1000));
  const hrs = Math.floor(totalSec / 3600);
  const mins = Math.floor((totalSec % 3600) / 60);
  const secs = totalSec % 60;
  if (hrs > 0) {
    return `${hrs}h ${mins}m`;
  }
  if (mins > 0) {
    return `${mins}m ${secs.toString().padStart(2, "0")}s`;
  }
  return `${secs}s`;
}

function formatLastWatched(ms: number): string {
  if (!ms || ms <= 0) return "";
  const date = new Date(ms);
  const now = new Date();
  const diffMin = Math.floor((now.getTime() - ms) / 60_000);
  if (diffMin >= 0 && diffMin < 1) return "Just now";
  if (diffMin >= 1 && diffMin < 60) return `${diffMin}m ago`;

  const startToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startYesterday = startToday - 86_400_000;
  const timeStr = date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

  if (ms >= startToday) return `Today • ${timeStr}`;
  if (ms >= startYesterday) return `Yesterday • ${timeStr}`;
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

function extractQualityTag(rawTitle: string, streamUrl: string): string {
  const match = `${rawTitle} ${streamUrl}`.match(/\b(2160p|4k|uhd|1080p|720p|576p|480p|360p)\b/i);
  return match ? match[1].toUpperCase() : "";
}

function formatHistoryTitle(raw: string): string {
  const base = (formatOttTitle(raw) || raw)
    .replace(/[\s\-–—:(\[/|]+$/g, "")
    .trim();
  const parts = base.split(/\s+-\s+/);
  if (parts.length === 2) {
    const a = parts[0].trim();
    const b = parts[1].trim();
    const norm = (s: string) => s.replace(/\(\d{4}\)/g, "").replace(/\s+/g, " ").trim().toLowerCase();
    if (norm(a) && (norm(a) === norm(b) || norm(a).startsWith(norm(b)))) {
      return a;
    }
  }
  return base || raw;
}

function HistoryThumb({
  src,
  alt,
  pct,
  onClick,
}: {
  src: string;
  alt: string;
  pct: number;
  onClick: () => void;
}) {
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);

  return (
    <button
      type="button"
      className="hist-thumb-btn"
      onClick={onClick}
      aria-label={`Resume ${alt}`}
    >
      {src && !failed ? (
        <img
          src={src}
          alt={alt}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          className={`hist-thumb-img ${loaded ? "loaded" : ""}`}
          onLoad={() => setLoaded(true)}
          onError={() => setFailed(true)}
        />
      ) : (
        <div className="hist-thumb-fallback" aria-hidden="true">
          <Film size={22} />
        </div>
      )}
      <div className="hist-thumb-scrim" aria-hidden="true" />
      <span className="hist-thumb-play" aria-hidden="true">
        <Play size={14} fill="currentColor" style={{ marginLeft: 1 }} />
      </span>
      {pct > 0 && (
        <div className="hist-thumb-progress" aria-hidden="true">
          <div className="hist-thumb-progress-fill" style={{ width: `${pct}%` }} />
        </div>
      )}
    </button>
  );
}

export function WatchHistoryView({ onResume }: Props) {
  const headingId = useId();
  const [items, setItems] = useState<WatchHistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [confirmClearAll, setConfirmClearAll] = useState(false);

  useEffect(() => {
    let active = true;
    async function load() {
      setLoading(true);
      try {
        const rows = await api.getWatchHistory();
        if (active) setItems(rows);
      } finally {
        if (active) setLoading(false);
      }
    }
    void load();
    return () => {
      active = false;
    };
  }, []);

  const filteredItems = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter(
      (i) =>
        i.title.toLowerCase().includes(q) ||
        i.scraperValue.toLowerCase().includes(q)
    );
  }, [items, query]);

  async function handleDelete(link: string) {
    await api.deleteWatchHistoryEntry(link);
    setItems((prev) => prev.filter((i) => i.link !== link));
  }

  async function handleClearAll() {
    await api.clearWatchHistory();
    setConfirmClearAll(false);
    setItems([]);
  }

  return (
    <section className="hist-screen" aria-labelledby={headingId}>
      <header className="hist-header">
        <div className="hist-title-block">
          <h1 id={headingId} className="hist-title">
            Watch History
          </h1>
          <p className="hist-subtitle">
            Pick up right where you paused across movies, episodes, and live streams.
          </p>
        </div>

        <div className="hist-header-actions">
          <div className="pref-summary-pill">
            <span
              className={`pref-summary-dot ${items.length > 0 ? "is-live" : "is-amber"}`}
              aria-hidden="true"
            />
            <span>
              {items.length} {items.length === 1 ? "Title" : "Titles"}
            </span>
          </div>

          {items.length > 0 &&
            (confirmClearAll ? (
              <div className="prov-delete-confirm">
                <button
                  type="button"
                  className="btn prov-confirm-danger"
                  onClick={() => void handleClearAll()}
                >
                  Confirm Clear All
                </button>
                <button
                  type="button"
                  className="btn prov-compact-btn"
                  onClick={() => setConfirmClearAll(false)}
                >
                  Cancel
                </button>
              </div>
            ) : (
              <button
                type="button"
                className="btn"
                onClick={() => setConfirmClearAll(true)}
              >
                <Trash2 size={14} />
                <span>Clear All</span>
              </button>
            ))}
        </div>
      </header>

      {items.length > 0 && (
        <div className="hist-search-wrap">
          <Search size={15} className="hist-search-icon" aria-hidden="true" />
          <input
            type="text"
            role="searchbox"
            aria-label="Filter watch history"
            className="input hist-search-input"
            placeholder="Filter watch history by title or source..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          {query.length > 0 && (
            <button
              type="button"
              className="dns-clear-btn"
              aria-label="Clear history search"
              onClick={() => setQuery("")}
            >
              <X size={14} />
            </button>
          )}
        </div>
      )}

      {loading && (
        <div className="hist-skeleton-list" aria-busy="true" aria-label="Loading watch history">
          {[0, 1, 2, 3].map((k) => (
            <div key={k} className="shimmer hist-skeleton-row" />
          ))}
        </div>
      )}

      {!loading && items.length === 0 && (
        <div className="hist-empty-card">
          <div className="hist-empty-icon" aria-hidden="true">
            <Clock size={22} />
          </div>
          <div className="hist-empty-title">No Watch History Yet</div>
          <div className="hist-empty-desc">
            Play any movie, episode, or stream and your exact resume timestamp will appear here automatically.
          </div>
        </div>
      )}

      {!loading && items.length > 0 && filteredItems.length === 0 && (
        <div className="hist-empty-card">
          <div className="hist-empty-title">No Matching Titles</div>
          <div className="hist-empty-desc">
            Nothing in your watch history matched &ldquo;{query.trim()}&rdquo;.
          </div>
        </div>
      )}

      {!loading && filteredItems.length > 0 && (
        <div className="hist-group">
          {filteredItems.map((item) => {
            const pct =
              item.durationMs > 0
                ? Math.min(100, Math.round((item.positionMs / item.durationMs) * 100))
                : 0;
            const remainingMs =
              item.durationMs > item.positionMs
                ? item.durationMs - item.positionMs
                : 0;
            const displayTitle = formatHistoryTitle(item.title);
            const qualityTag = extractQualityTag(item.title, item.streamUrl);
            const watchedWhen = formatLastWatched(item.lastWatchedMs);

            const resumePlayback = () =>
              onResume({
                title: item.title,
                link: item.link,
                streamUrl: item.streamUrl,
                imageUrl: item.imageUrl,
                providerUrl: item.providerUrl,
                scraperValue: item.scraperValue,
              });

            return (
              <div key={item.link} className="hist-row">
                <HistoryThumb
                  src={item.imageUrl}
                  alt={displayTitle}
                  pct={pct}
                  onClick={resumePlayback}
                />

                <div className="hist-row-body">
                  <div className="hist-row-top">
                    <button
                      type="button"
                      className="hist-title-btn"
                      title={item.title}
                      onClick={resumePlayback}
                    >
                      {displayTitle}
                    </button>
                    {qualityTag && <span className="ott-meta-pill amber">{qualityTag}</span>}
                    {item.scraperValue && (
                      <span className="ott-meta-pill">{item.scraperValue}</span>
                    )}
                  </div>

                  <div className="hist-row-meta">
                    <span className="dns-tabular">
                      {item.positionMs > 0
                        ? `${formatDurationMs(item.positionMs)} watched`
                        : "Started"}
                    </span>
                    {remainingMs > 0 && (
                      <>
                        <span className="hist-meta-dot" aria-hidden="true">•</span>
                        <span className="dns-tabular">{formatDurationMs(remainingMs)} left</span>
                      </>
                    )}
                    {pct > 0 && (
                      <>
                        <span className="hist-meta-dot" aria-hidden="true">•</span>
                        <span className="hist-pct-label dns-tabular">{pct}%</span>
                      </>
                    )}
                    {watchedWhen && (
                      <>
                        <span className="hist-meta-dot" aria-hidden="true">•</span>
                        <span>{watchedWhen}</span>
                      </>
                    )}
                  </div>

                  {pct > 0 && (
                    <div className="hist-inline-progress" aria-hidden="true">
                      <div
                        className="hist-inline-progress-fill"
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                  )}
                </div>

                <div className="hist-row-actions">
                  <button
                    type="button"
                    className="btn btn-primary hist-resume-btn"
                    onClick={resumePlayback}
                  >
                    <Play size={14} fill="currentColor" />
                    <span>Resume</span>
                  </button>
                  <button
                    type="button"
                    className="btn prov-icon-btn"
                    aria-label={`Remove ${displayTitle} from watch history`}
                    title="Remove from history"
                    onClick={() => void handleDelete(item.link)}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
