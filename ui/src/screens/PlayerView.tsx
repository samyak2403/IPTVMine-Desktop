import { useEffect, useRef, useState } from "react";
import Hls from "hls.js";
import {
  ArrowLeft,
  ExternalLink,
  Maximize,
  Minimize,
  Pause,
  PictureInPicture2,
  Play,
  RotateCcw,
  SkipForward,
  Volume2,
  VolumeX,
} from "lucide-react";
import { api } from "../api";
import type { PlaybackRequest, VegaStream } from "../types";

interface Props {
  request: PlaybackRequest;
  onClose: () => void;
  onEpisodeResolved?: (nextReq: PlaybackRequest) => void;
}

function formatTime(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return "00:00";
  const total = Math.floor(sec);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

export function PlayerView({ request, onClose, onEpisodeResolved }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const hlsRef = useRef<Hls | null>(null);
  const hideTimerRef = useRef<number | null>(null);

  const mirrors: VegaStream[] =
    request.mirrors && request.mirrors.length > 0
      ? request.mirrors
      : [
          {
            server: "Primary",
            link: request.streamUrl,
            type: request.isLive ? "m3u8" : "auto",
            quality: "HD",
            headers: request.headers ?? null,
          },
        ];

  const [mirrorIdx, setMirrorIdx] = useState(0);
  const activeStream = mirrors[mirrorIdx] ?? mirrors[0];

  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [fitMode, setFitMode] = useState<"contain" | "cover" | "fill">("contain");
  const [hudVisible, setHudVisible] = useState(true);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [alwaysOnTop, setAlwaysOnTop] = useState(false);

  const [levels, setLevels] = useState<Array<{ index: number; label: string }>>([]);
  const [selectedLevel, setSelectedLevel] = useState(-1);
  const [audioTracks, setAudioTracks] = useState<Array<{ index: number; label: string }>>([]);
  const [selectedAudio, setSelectedAudio] = useState(0);

  const [errorMsg, setErrorMsg] = useState("");
  const [statusNotice, setStatusNotice] = useState("");
  const [resolvingNext, setResolvingNext] = useState(false);

  const hasNextEpisode =
    !!request.episodes &&
    request.currentEpisodeIndex !== undefined &&
    request.currentEpisodeIndex + 1 < request.episodes.length;

  function bumpHud() {
    setHudVisible(true);
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = window.setTimeout(() => {
      if (videoRef.current && !videoRef.current.paused) {
        setHudVisible(false);
      }
    }, 3000);
  }

  async function saveCurrentProgress() {
    const vid = videoRef.current;
    if (!vid || request.isLive) return;
    const posMs = Math.floor((vid.currentTime || 0) * 1000);
    const durMs = Number.isFinite(vid.duration) ? Math.floor(vid.duration * 1000) : 0;
    if (posMs <= 0) return;
    await api.saveWatchProgress({
      title: request.title,
      link: request.link,
      streamUrl: activeStream.link,
      imageUrl: request.imageUrl ?? "",
      providerUrl: request.providerUrl ?? "",
      scraperValue: request.scraperValue ?? "",
      positionMs: posMs,
      durationMs: durMs,
      lastWatchedMs: Date.now(),
    });
  }

  useEffect(() => {
    let cancelled = false;

    async function startStream() {
      setErrorMsg("");
      setLevels([]);
      setAudioTracks([]);
      setSelectedLevel(-1);

      const video = videoRef.current;
      if (!video) return;

      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }

      const lower = activeStream.link.toLowerCase();
      if (lower.startsWith("rtmp://") || lower.startsWith("rtmps://")) {
        setErrorMsg("RTMP streams require an external player (VLC or mpv).");
        return;
      }

      let playUrl = activeStream.link;
      try {
        const mergedHeaders = {
          ...(request.headers ?? {}),
          ...(activeStream.headers ?? {}),
        };
        const proxied = await api.prepareStreamProxy(activeStream.link, mergedHeaders);
        if (cancelled) return;
        playUrl = proxied.proxyUrl;
      } catch (err) {
        if (!cancelled) setErrorMsg(String(err));
        return;
      }

      const resumeMs = request.isLive
        ? 0
        : await api.getResumePositionMs(request.link).catch(() => 0);
      if (cancelled) return;

      const looksHls =
        lower.includes(".m3u8") ||
        lower.includes("/hls/") ||
        activeStream.type.toLowerCase() === "m3u8" ||
        activeStream.type.toLowerCase() === "hls";

      const attachResume = () => {
        if (resumeMs > 0 && video.duration && resumeMs / 1000 < video.duration - 5) {
          video.currentTime = resumeMs / 1000;
        }
        void video.play().catch(() => {});
      };

      if (Hls.isSupported() && (looksHls || !lower.match(/\.(mp4|mkv|webm|mov)(\?|$)/))) {
        const hls = new Hls({
          enableWorker: true,
          lowLatencyMode: Boolean(request.isLive),
        });
        hlsRef.current = hls;
        hls.loadSource(playUrl);
        hls.attachMedia(video);

        hls.on(Hls.Events.MANIFEST_PARSED, () => {
          setLevels(
            hls.levels.map((lvl, idx) => ({
              index: idx,
              label: lvl.height ? `${lvl.height}p` : `Level ${idx + 1}`,
            }))
          );
          attachResume();
        });

        hls.on(Hls.Events.AUDIO_TRACKS_UPDATED, (_, data) => {
          setAudioTracks(
            data.audioTracks.map((t, idx) => ({
              index: idx,
              label: t.name || t.lang || `Audio ${idx + 1}`,
            }))
          );
        });

        hls.on(Hls.Events.ERROR, (_, data) => {
          if (data.fatal) {
            if (data.type === Hls.ErrorTypes.NETWORK_ERROR && !looksHls) {
              // Fallback to direct <video> tag if stream is MP4/MKV without extension
              hls.destroy();
              hlsRef.current = null;
              video.src = playUrl;
              video.onloadedmetadata = attachResume;
            } else {
              setErrorMsg(
                `Playback failed (${data.details}). Try another mirror or open in VLC/mpv.`
              );
            }
          }
        });
      } else {
        video.src = playUrl;
        video.onloadedmetadata = attachResume;
      }
    }

    void startStream();
    return () => {
      cancelled = true;
      void saveCurrentProgress();
      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }
    };
  }, [activeStream.link, mirrorIdx, request.link]);

  useEffect(() => {
    const id = window.setInterval(() => {
      void saveCurrentProgress();
    }, 10000);
    return () => window.clearInterval(id);
  }, [activeStream.link, request.link]);

  useEffect(() => {
    return () => {
      void api.setAlwaysOnTop(false).catch(() => {});
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT") return;
      const vid = videoRef.current;
      if (!vid) return;
      bumpHud();

      switch (e.key.toLowerCase()) {
        case " ":
        case "k":
          e.preventDefault();
          if (vid.paused) void vid.play();
          else vid.pause();
          break;
        case "arrowleft":
          e.preventDefault();
          vid.currentTime = Math.max(0, vid.currentTime - 10);
          break;
        case "arrowright":
          e.preventDefault();
          vid.currentTime = vid.currentTime + 10;
          break;
        case "j":
          vid.currentTime = Math.max(0, vid.currentTime - 30);
          break;
        case "l":
          vid.currentTime = vid.currentTime + 30;
          break;
        case "arrowup":
          e.preventDefault();
          setVolume((v) => {
            const next = Math.min(1, +(v + 0.05).toFixed(2));
            vid.volume = next;
            return next;
          });
          break;
        case "arrowdown":
          e.preventDefault();
          setVolume((v) => {
            const next = Math.max(0, +(v - 0.05).toFixed(2));
            vid.volume = next;
            return next;
          });
          break;
        case "m":
          vid.muted = !vid.muted;
          setMuted(vid.muted);
          break;
        case "f":
          void toggleFullscreen();
          break;
        case "p":
          void togglePin();
          break;
        case "n":
          if (hasNextEpisode) void playNextEpisode();
          break;
        case "escape":
          if (document.fullscreenElement) {
            void document.exitFullscreen();
          } else {
            handleExit();
          }
          break;
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [hasNextEpisode, alwaysOnTop]);

  function handleExit() {
    void saveCurrentProgress();
    void api.setAlwaysOnTop(false).catch(() => {});
    onClose();
  }

  async function toggleFullscreen() {
    if (!containerRef.current) return;
    if (!document.fullscreenElement) {
      await containerRef.current.requestFullscreen();
      setIsFullscreen(true);
    } else {
      await document.exitFullscreen();
      setIsFullscreen(false);
    }
  }

  async function togglePin() {
    const next = !alwaysOnTop;
    await api.setAlwaysOnTop(next);
    setAlwaysOnTop(next);
  }

  async function openExternal() {
    try {
      const merged = {
        ...(request.headers ?? {}),
        ...(activeStream.headers ?? {}),
      };
      const exe = await api.launchExternalPlayer(
        activeStream.link,
        request.title,
        merged
      );
      setStatusNotice(`Opened in ${exe}`);
      window.setTimeout(() => setStatusNotice(""), 3000);
    } catch (err) {
      setErrorMsg(String(err));
    }
  }

  async function playNextEpisode() {
    if (!hasNextEpisode || !request.episodes || request.currentEpisodeIndex === undefined) {
      return;
    }
    const nextIdx = request.currentEpisodeIndex + 1;
    const ep = request.episodes[nextIdx];
    setResolvingNext(true);
    setStatusNotice(`Loading ${ep.title}...`);
    try {
      const res = await api.resolveVodStreamWithFallback({
        providerUrl: request.providerUrl,
        scraperValue: request.scraperValue ?? "",
        initialLink: ep.link,
        initialType: ep.type,
        itemTitle: ep.title,
        movieTitle: request.title,
      });
      if (res && onEpisodeResolved) {
        onEpisodeResolved({
          ...request,
          title: ep.title,
          link: ep.link,
          streamUrl: res.selectedStream.link,
          headers: res.selectedStream.headers ?? undefined,
          mirrors: res.allStreams,
          currentEpisodeIndex: nextIdx,
        });
      } else {
        setErrorMsg(`Could not resolve stream for ${ep.title}.`);
      }
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setResolvingNext(false);
      setStatusNotice("");
    }
  }

  return (
    <div
      ref={containerRef}
      className="player-shell"
      onMouseMove={bumpHud}
      onDoubleClick={() => void toggleFullscreen()}
    >
      <video
        ref={videoRef}
        className="player-video"
        style={{ objectFit: fitMode }}
        onPlay={() => setPlaying(true)}
        onPause={() => {
          setPlaying(false);
          setHudVisible(true);
          void saveCurrentProgress();
        }}
        onTimeUpdate={() => {
          if (videoRef.current) setCurrentTime(videoRef.current.currentTime);
        }}
        onDurationChange={() => {
          if (videoRef.current) setDuration(videoRef.current.duration || 0);
        }}
        onError={() => {
          if (!hlsRef.current) {
            setErrorMsg(
              "Unable to decode this stream inside WebView. Try another mirror or Open in External Player (VLC/mpv)."
            );
          }
        }}
      />

      {/* Top HUD */}
      <div className={`player-hud-top ${hudVisible ? "visible" : "hidden"}`}>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <button className="btn" onClick={handleExit}>
            <ArrowLeft size={15} /> Back
          </button>
          <div>
            <div style={{ fontWeight: 700, fontSize: 16 }}>{request.title}</div>
            <div className="muted">
              {activeStream.server} {activeStream.quality ? `• ${activeStream.quality}` : ""}
            </div>
          </div>
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          {statusNotice && <span className="badge">{statusNotice}</span>}
          {mirrors.length > 1 && (
            <select
              className="input"
              style={{ width: 170, padding: "6px 10px" }}
              value={mirrorIdx}
              onChange={(e) => setMirrorIdx(Number(e.target.value))}
            >
              {mirrors.map((m, i) => (
                <option key={`${m.link}-${i}`} value={i}>
                  {m.server || `Mirror ${i + 1}`} ({m.quality || "HD"})
                </option>
              ))}
            </select>
          )}
          <button className="btn" onClick={() => void openExternal()} title="Open in VLC / mpv">
            <ExternalLink size={14} /> External
          </button>
        </div>
      </div>

      {/* Bottom HUD */}
      <div className={`player-hud-bottom ${hudVisible ? "visible" : "hidden"}`}>
        {!request.isLive && (
          <input
            type="range"
            min={0}
            max={duration || 0}
            value={Math.min(currentTime, duration || 0)}
            onChange={(e) => {
              const t = Number(e.target.value);
              if (videoRef.current) videoRef.current.currentTime = t;
              setCurrentTime(t);
            }}
            style={{ width: "100%", accentColor: "#ffc062" }}
          />
        )}

        <div className="player-controls-row">
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <button
              className="btn btn-primary"
              onClick={() => {
                const vid = videoRef.current;
                if (!vid) return;
                if (vid.paused) void vid.play();
                else vid.pause();
              }}
            >
              {playing ? <Pause size={16} /> : <Play size={16} />}
            </button>

            <button
              className="btn"
              onClick={() => {
                const vid = videoRef.current;
                if (!vid) return;
                vid.muted = !vid.muted;
                setMuted(vid.muted);
              }}
            >
              {muted || volume === 0 ? <VolumeX size={15} /> : <Volume2 size={15} />}
            </button>

            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={muted ? 0 : volume}
              onChange={(e) => {
                const v = Number(e.target.value);
                setVolume(v);
                setMuted(v === 0);
                if (videoRef.current) {
                  videoRef.current.volume = v;
                  videoRef.current.muted = v === 0;
                }
              }}
              style={{ width: 90, accentColor: "#ffc062" }}
            />

            <span className="muted" style={{ minWidth: 145, fontVariantNumeric: "tabular-nums" }}>
              {request.isLive
                ? "LIVE"
                : duration > 0
                  ? `${formatTime(currentTime)} / ${formatTime(duration)} (-${formatTime(Math.max(0, duration - currentTime))})`
                  : formatTime(currentTime)}
            </span>
          </div>

          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            {hasNextEpisode && (
              <button
                className="btn"
                disabled={resolvingNext}
                onClick={() => void playNextEpisode()}
              >
                <SkipForward size={14} /> Next Episode
              </button>
            )}

            {levels.length > 0 && (
              <select
                className="input"
                style={{ width: 105, padding: "6px 8px" }}
                value={selectedLevel}
                onChange={(e) => {
                  const lvl = Number(e.target.value);
                  setSelectedLevel(lvl);
                  if (hlsRef.current) hlsRef.current.currentLevel = lvl;
                }}
              >
                <option value={-1}>Auto</option>
                {levels.map((l) => (
                  <option key={l.index} value={l.index}>
                    {l.label}
                  </option>
                ))}
              </select>
            )}

            {audioTracks.length > 1 && (
              <select
                className="input"
                style={{ width: 120, padding: "6px 8px" }}
                value={selectedAudio}
                onChange={(e) => {
                  const idx = Number(e.target.value);
                  setSelectedAudio(idx);
                  if (hlsRef.current) hlsRef.current.audioTrack = idx;
                }}
              >
                {audioTracks.map((a) => (
                  <option key={a.index} value={a.index}>
                    {a.label}
                  </option>
                ))}
              </select>
            )}

            {!request.isLive && (
              <select
                className="input"
                style={{ width: 80, padding: "6px 8px" }}
                value={speed}
                onChange={(e) => {
                  const sp = Number(e.target.value);
                  setSpeed(sp);
                  if (videoRef.current) videoRef.current.playbackRate = sp;
                }}
              >
                {[0.5, 1, 1.25, 1.5, 2].map((r) => (
                  <option key={r} value={r}>
                    {r}x
                  </option>
                ))}
              </select>
            )}

            <button
              className="btn"
              onClick={() =>
                setFitMode((m) =>
                  m === "contain" ? "cover" : m === "cover" ? "fill" : "contain"
                )
              }
            >
              {fitMode}
            </button>

            <button
              className={`btn ${alwaysOnTop ? "btn-primary" : ""}`}
              onClick={() => void togglePin()}
              title="Always on Top (P)"
            >
              <PictureInPicture2 size={14} />
            </button>

            <button className="btn" onClick={() => void toggleFullscreen()} title="Fullscreen (F)">
              {isFullscreen ? <Minimize size={14} /> : <Maximize size={14} />}
            </button>
          </div>
        </div>
      </div>

      {/* Error Recovery Modal */}
      {errorMsg && (
        <div className="player-error-backdrop">
          <div className="card" style={{ maxWidth: 460, display: "grid", gap: 14 }}>
            <h2 style={{ margin: 0, fontSize: 18 }}>Playback Error</h2>
            <div className="error-text">{errorMsg}</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
              <button
                className="btn btn-primary"
                onClick={() => {
                  setErrorMsg("");
                  setMirrorIdx((i) => i);
                }}
              >
                <RotateCcw size={14} /> Retry
              </button>
              {mirrorIdx + 1 < mirrors.length && (
                <button
                  className="btn"
                  onClick={() => {
                    setErrorMsg("");
                    setMirrorIdx((i) => i + 1);
                  }}
                >
                  Try Next Mirror
                </button>
              )}
              <button className="btn" onClick={() => void openExternal()}>
                <ExternalLink size={14} /> Open in VLC / mpv
              </button>
              <button className="btn" onClick={handleExit}>
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
