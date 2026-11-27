import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowLeft, Calendar, Clock, Download, Film, HardDrive, Play, Star } from "lucide-react";
import { api } from "../api";
import { formatOttTitle } from "../components/OttUi";
import { extractRichCardMeta, type RichCardMeta } from "../utils/mediaBadges";
import playCircleSvg from "../assets/figma/play-circle.svg";
import type {
  DetailEnrichment,
  EpisodeItem,
  PlaybackRequest,
  VegaLink,
  VegaMeta,
  VegaPost,
} from "../types";

interface Props {
  post: VegaPost;
  scraperValue: string;
  onBack: () => void;
  onPlayStream: (req: PlaybackRequest) => void;
}

interface MediaCardRowProps {
  info: RichCardMeta;
  sourceLabel: string;
  resolving: boolean;
  onPlay: () => void;
  onDownload: () => void;
}

function MediaItemCard({
  info,
  sourceLabel,
  resolving,
  onPlay,
  onDownload,
}: MediaCardRowProps) {
  const epBadge = [info.seasonLabel, info.episodeLabel].filter(Boolean).join(" • ");
  const thumbCorner = info.durationLabel || info.episodeLabel || info.qualityBadge;

  return (
    <div className="ott-media-card">
      <button
        type="button"
        className="ott-media-thumb-btn"
        aria-label={`Play ${info.displayTitle}`}
        disabled={resolving}
        onClick={onPlay}
      >
        {info.thumbnailUrl ? (
          <img
            className="ott-media-thumb-img"
            src={info.thumbnailUrl}
            alt={info.displayTitle}
            loading="lazy"
          />
        ) : (
          <div className="ott-hero-ambient" />
        )}
        <div className="ott-media-thumb-scrim" />
        <span className="ott-media-thumb-play">
          <Play size={14} fill="currentColor" />
        </span>
        {thumbCorner && <span className="ott-media-thumb-tag">{thumbCorner}</span>}
      </button>

      <div className="ott-media-body">
        <div className="ott-media-title" title={info.displayTitle}>
          {info.displayTitle}
        </div>
        {info.overview && <p className="ott-media-overview">{info.overview}</p>}
        <div className="ott-meta-row">
          {epBadge && <span className="ott-meta-pill amber">{epBadge}</span>}
          {info.qualityBadge && (
            <span className="ott-meta-pill">
              <Film size={11} /> {info.qualityBadge}
            </span>
          )}
          {info.sizeBadge && (
            <span className="ott-meta-pill amber">
              <HardDrive size={11} /> {info.sizeBadge}
            </span>
          )}
          {info.durationLabel && (
            <span className="ott-meta-pill">
              <Clock size={11} /> {info.durationLabel}
            </span>
          )}
          {info.dateLabel && (
            <span className="ott-meta-pill">
              <Calendar size={11} /> {info.dateLabel}
            </span>
          )}
          {info.ratingLabel && (
            <span className="ott-meta-pill">
              <Star size={11} /> {info.ratingLabel}
            </span>
          )}
          {info.formatTags.map((tag) => (
            <span key={tag} className="ott-meta-pill">
              {tag}
            </span>
          ))}
          {sourceLabel && <span className="ott-meta-pill">{sourceLabel}</span>}
        </div>
      </div>

      <div className="ott-media-actions">
        <button type="button" className="btn" disabled={resolving} onClick={onDownload}>
          <Download size={14} /> Download
        </button>
        <button
          type="button"
          className="btn btn-primary"
          disabled={resolving}
          onClick={onPlay}
        >
          <Play size={14} /> Play
        </button>
      </div>
    </div>
  );
}

export function MovieDetailView({ post, scraperValue, onBack, onPlayStream }: Props) {
  const [meta, setMeta] = useState<VegaMeta | null>(null);
  const [enrichment, setEnrichment] = useState<DetailEnrichment | null>(null);
  const [selectedServerIdx, setSelectedServerIdx] = useState(0);
  const [lazyEpisodes, setLazyEpisodes] = useState<VegaLink[]>([]);
  const [loadingEpisodes, setLoadingEpisodes] = useState(false);
  const [loading, setLoading] = useState(true);
  const [resolving, setResolving] = useState(false);
  const [fallbackStatus, setFallbackStatus] = useState("");
  const [downloadNotice, setDownloadNotice] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<string>("vega://fallback_status", (e) => {
      setFallbackStatus(e.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let active = true;
    async function load() {
      setLoading(true);
      setEnrichment(null);
      setSelectedServerIdx(0);
      setError("");
      try {
        const res = await api.getVodMeta(scraperValue, post.link);
        if (!active) return;
        setMeta(res);
        // Fetch Cinemeta / TVMaze images, runtimes, dates, ratings & episode titles in background
        void api
          .getDetailEnrichment(
            res.imdbId || "",
            res.title || post.title,
            res.type || "movie"
          )
          .then((enriched) => {
            if (active) setEnrichment(enriched);
          })
          .catch(() => {
            /* keep base Vega metadata if offline or rate-limited */
          });
      } catch (err) {
        if (active) setError(String(err));
      } finally {
        if (active) setLoading(false);
      }
    }
    void load();
    return () => {
      active = false;
    };
  }, [post.link, post.title, scraperValue]);

  const activeServer = meta?.linkList[selectedServerIdx];

  useEffect(() => {
    if (!activeServer?.episodesLink || activeServer.directLinks?.length) {
      setLazyEpisodes([]);
      setLoadingEpisodes(false);
      return;
    }
    let active = true;
    async function loadEpisodes() {
      setLoadingEpisodes(true);
      try {
        const eps = await api.getVodEpisodes(
          scraperValue,
          activeServer!.episodesLink!
        );
        if (active) setLazyEpisodes(eps);
      } catch {
        if (active) setLazyEpisodes([]);
      } finally {
        if (active) setLoadingEpisodes(false);
      }
    }
    void loadEpisodes();
    return () => {
      active = false;
    };
  }, [activeServer, scraperValue]);

  async function resolveStream(
    link: string,
    itemTitle: string,
    itemType: string,
    episodes?: EpisodeItem[],
    episodeIdx?: number
  ) {
    setResolving(true);
    setFallbackStatus("Resolving stream links...");
    setError("");
    try {
      const res = await api.resolveVodStreamWithFallback({
        scraperValue,
        initialLink: link,
        initialType: itemType || meta?.type || "movie",
        itemTitle,
        movieTitle: meta?.title || post.title,
        currentMeta: meta,
      });
      if (!res) {
        setError("No playable streams found across available servers or extensions.");
      } else {
        onPlayStream({
          title: itemTitle ? `${meta?.title || post.title} - ${itemTitle}` : meta?.title || post.title,
          link: post.link,
          streamUrl: res.selectedStream.link,
          headers: res.selectedStream.headers ?? undefined,
          imageUrl: meta?.image || post.image,
          scraperValue: res.providerValue,
          isLive: false,
          mirrors: res.allStreams,
          episodes,
          currentEpisodeIndex: episodeIdx,
        });
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setResolving(false);
      setFallbackStatus("");
    }
  }

  async function queueDownloadForLink(link: string, itemTitle: string, itemType: string) {
    setResolving(true);
    setDownloadNotice("");
    setFallbackStatus("Resolving download stream...");
    setError("");
    try {
      const res = await api.resolveVodStreamWithFallback({
        scraperValue,
        initialLink: link,
        initialType: itemType || meta?.type || "movie",
        itemTitle,
        movieTitle: meta?.title || post.title,
        currentMeta: meta,
      });
      if (!res) {
        setError("No downloadable stream found.");
        return;
      }
      const title = itemTitle
        ? `${meta?.title || post.title} - ${itemTitle}`
        : meta?.title || post.title;
      await api.enqueueDownload(
        title,
        res.selectedStream.link,
        res.selectedStream.headers ?? undefined
      );
      setDownloadNotice(`Queued download: ${title}`);
    } catch (err) {
      setError(String(err));
    } finally {
      setResolving(false);
      setFallbackStatus("");
    }
  }

  return (
    <div>
      <div className="page-header">
        <button className="btn" onClick={onBack}>
          <ArrowLeft size={14} /> Back to Movies
        </button>
        <span className="badge">{scraperValue}</span>
      </div>

      {loading && <div className="shimmer" style={{ height: 260, marginBottom: 20 }} />}
      {error && (
        <div className="card error-text" style={{ marginBottom: 16 }}>
          {error}
        </div>
      )}

      {meta && (
        <>
          <div className="detail-hero-banner">
            {(enrichment?.background || meta.background || meta.image || post.image) && (
              <img
                className="detail-hero-bg"
                src={enrichment?.background || meta.background || meta.image || post.image}
                alt=""
                aria-hidden="true"
              />
            )}
            <img
              className="detail-poster"
              src={meta.image || post.image}
              alt={meta.title || post.title}
            />
            <div style={{ position: "relative", zIndex: 2, display: "grid", gap: 12, flex: 1 }}>
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
                {meta.type && <span className="badge">{meta.type.toUpperCase()}</span>}
                {(meta.year || enrichment?.year) && (
                  <span className="ott-meta-pill">{meta.year || enrichment?.year}</span>
                )}
                {(meta.runtime || enrichment?.runtime) && (
                  <span className="ott-meta-pill">
                    <Clock size={11} /> {meta.runtime || enrichment?.runtime}
                  </span>
                )}
                {(meta.rating || enrichment?.rating) && (
                  <span className="ott-meta-pill amber">
                    <Star size={11} /> {meta.rating || enrichment?.rating}
                  </span>
                )}
                {(meta.imdbId || enrichment?.imdbId) && (
                  <span className="ott-meta-pill">{meta.imdbId || enrichment?.imdbId}</span>
                )}
                {(meta.genres?.length ? meta.genres : enrichment?.genres ?? [])
                  .slice(0, 3)
                  .map((g) => (
                    <span key={g} className="ott-meta-pill">
                      {g}
                    </span>
                  ))}
              </div>
              <h1 className="ott-hero-title" style={{ fontSize: "clamp(1.75rem, 3vw, 2.9rem)" }}>
                {formatOttTitle(meta.title || post.title)}
              </h1>
              <p className="ott-hero-eyebrow" style={{ fontSize: 16 }}>
                {meta.type === "series" ? "Series • Ready to Stream" : "Now Streaming"}
              </p>
              <p className="muted" style={{ lineHeight: 1.55, margin: 0, maxWidth: 720 }}>
                {meta.synopsis || "No synopsis available."}
              </p>
              {(activeServer?.directLinks?.[0] || lazyEpisodes[0]?.directLinks?.[0]) && (
                <div className="ott-hero-actions" style={{ marginTop: 4 }}>
                  <button
                    type="button"
                    className="ott-play-cta"
                    disabled={resolving}
                    onClick={() => {
                      const dls = activeServer?.directLinks ?? [];
                      const preferredDl =
                        dls.find((d) => /\b1080p\b/i.test(d.title)) ??
                        dls.find((d) => !/\b(2160p|4k|uhd)\b/i.test(d.title) && !/\b([2-9]\d|\d{3,})\s*gb\b/i.test(d.title)) ??
                        dls[0];
                      const dl = preferredDl ?? lazyEpisodes[0]?.directLinks?.[0];
                      const title = preferredDl?.title ?? lazyEpisodes[0]?.title ?? "";
                      if (!dl) return;
                      void resolveStream(dl.link, title, dl.type || meta.type);
                    }}
                  >
                    <img className="ott-play-cta-icon" src={playCircleSvg} alt="" width={48} height={48} />
                    <span>{resolving ? "Connecting..." : "Play Now"}</span>
                  </button>
                </div>
              )}
              {resolving && (
                <div className="badge" style={{ width: "fit-content" }}>
                  {fallbackStatus || "Resolving..."}
                </div>
              )}
              {downloadNotice && (
                <div style={{ color: "var(--status-live)", fontSize: 13 }}>
                  {downloadNotice}
                </div>
              )}
            </div>
          </div>

          {meta.linkList.length > 0 && (
            <div className="chip-row">
              {meta.linkList.map((srv, idx) => (
                <button
                  key={`${srv.title}-${idx}`}
                  className={`chip ${selectedServerIdx === idx ? "active" : ""}`}
                  onClick={() => setSelectedServerIdx(idx)}
                >
                  {srv.title || `Server ${idx + 1}`}
                </button>
              ))}
            </div>
          )}

          {loadingEpisodes && (
            <div style={{ display: "grid", gap: 10, marginBottom: 10 }}>
              {[0, 1, 2].map((k) => (
                <div key={k} className="shimmer" style={{ height: 112 }} />
              ))}
            </div>
          )}

          <div style={{ display: "grid", gap: 10 }}>
            {(activeServer?.directLinks ?? []).map((dl, i, arr) => {
              const info = extractRichCardMeta({
                rawTitle: dl.title || `Source ${i + 1}`,
                linkUrl: dl.link,
                item: dl,
                serverTitle: activeServer?.title || "",
                fallbackImage: meta.image || post.image,
                fallbackBackdrop: enrichment?.background || meta.background || "",
                enrichment,
              });
              const epList: EpisodeItem[] = arr.map((d) => ({
                title: d.title || "Source",
                link: d.link,
                type: d.type || meta.type,
              }));
              return (
                <MediaItemCard
                  key={`${dl.link}-${i}`}
                  info={info}
                  sourceLabel={scraperValue}
                  resolving={resolving}
                  onDownload={() => void queueDownloadForLink(dl.link, info.displayTitle, dl.type)}
                  onPlay={() => void resolveStream(dl.link, info.displayTitle, dl.type, epList, i)}
                />
              );
            })}

            {lazyEpisodes.map((ep, idx, arr) => {
              const firstDirect = ep.directLinks?.[0];
              if (!firstDirect) return null;
              const info = extractRichCardMeta({
                rawTitle: ep.title || firstDirect.title || `Episode ${idx + 1}`,
                linkUrl: firstDirect.link,
                item: {
                  ...ep,
                  image: ep.image || firstDirect.image,
                  duration: ep.duration || firstDirect.duration,
                  date: ep.date || firstDirect.date,
                  size: ep.size || firstDirect.size,
                  quality: ep.quality || firstDirect.quality,
                  rating: ep.rating || firstDirect.rating,
                  overview: ep.overview || firstDirect.overview,
                  episodeNumber: ep.episodeNumber || firstDirect.episodeNumber,
                  seasonNumber: ep.seasonNumber || firstDirect.seasonNumber,
                },
                serverTitle: activeServer?.title || "",
                fallbackImage: meta.image || post.image,
                fallbackBackdrop: enrichment?.background || meta.background || "",
                enrichment,
              });
              const epList: EpisodeItem[] = arr
                .filter((e) => e.directLinks?.[0])
                .map((e) => ({
                  title: e.title,
                  link: e.directLinks![0].link,
                  type: e.directLinks![0].type || "series",
                }));
              return (
                <MediaItemCard
                  key={`${firstDirect.link}-${idx}`}
                  info={info}
                  sourceLabel={scraperValue}
                  resolving={resolving}
                  onDownload={() =>
                    void queueDownloadForLink(firstDirect.link, info.displayTitle, firstDirect.type)
                  }
                  onPlay={() =>
                    void resolveStream(firstDirect.link, info.displayTitle, firstDirect.type, epList, idx)
                  }
                />
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}
