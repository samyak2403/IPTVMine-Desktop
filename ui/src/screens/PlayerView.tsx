import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import Hls from "hls.js";
import {
  ArrowLeft,
  Check,
  Cpu,
  Download,
  ExternalLink,
  Keyboard,
  Layers,
  Loader2,
  Maximize,
  Minimize,
  Pause,
  PictureInPicture2,
  Play,
  RotateCcw,
  RotateCw,
  SkipForward,
  SlidersHorizontal,
  Subtitles,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { api } from "../api";
import { ChannelLogo } from "../components/ChannelLogo";
import type {
  FfmpegStatus,
  MediaProbeResult,
  PlaybackRequest,
  ProxiedStream,
  VegaStream,
} from "../types";

interface Props {
  request: PlaybackRequest;
  ffmpegEnabled?: boolean;
  onClose: () => void;
  onEpisodeResolved?: (nextReq: PlaybackRequest) => void;
}

type PopoverKind = "mirrors" | "tracks" | "settings" | null;
type FitMode = "contain" | "cover" | "fill";
type BezelState = {
  key: number;
  label: string;
  kind: "play" | "pause" | "seek-back" | "seek-fwd" | "volume" | "mute";
  instant: boolean;
} | null;

const SPEED_OPTIONS = [0.5, 1, 1.25, 1.5, 2] as const;
const FIT_OPTIONS: Array<{ value: FitMode; label: string }> = [
  { value: "contain", label: "Fit" },
  { value: "cover", label: "Fill" },
  { value: "fill", label: "Stretch" },
];

const SHORTCUTS: Array<{ keys: string; label: string }> = [
  { keys: "Space / K", label: "Play / Pause" },
  { keys: "← / →", label: "Seek ±10 seconds" },
  { keys: "J / L", label: "Seek ±30 seconds" },
  { keys: "↑ / ↓", label: "Volume ±5%" },
  { keys: "M", label: "Mute / Unmute" },
  { keys: "F / Double-click", label: "Fullscreen" },
  { keys: "P", label: "Float on Top" },
  { keys: "N", label: "Next Episode" },
  { keys: "? / Esc", label: "Shortcuts / Back" },
];

function formatTime(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return "00:00";
  const total = Math.floor(sec);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

function rubberband(overshoot: number, dimension = 24, constant = 0.35): number {
  if (overshoot === 0) return 0;
  return (overshoot * dimension * constant) / (dimension + constant * Math.abs(overshoot));
}

function needsContainerRemux(url: string): boolean {
  const lower = url.toLowerCase();
  return (
    /^(rtmp|rtmps|rtsp|udp|rtp):\/\//.test(lower) ||
    /\.(mkv|avi|flv|wmv|ts)(\?|#|$)/.test(lower)
  );
}

export function PlayerView({ request, ffmpegEnabled = true, onClose, onEpisodeResolved }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const hlsRef = useRef<Hls | null>(null);
  const hideTimerRef = useRef<number | null>(null);
  const bezelTimerRef = useRef<number | null>(null);
  const iconMorphTimerRef = useRef<number | null>(null);
  const waitingForFfmpegRef = useRef(false);

  const scrubRef = useRef<HTMLDivElement>(null);
  const scrubPointerIdRef = useRef<number | null>(null);
  const volRef = useRef<HTMLDivElement>(null);
  const volPointerIdRef = useRef<number | null>(null);

  const tipDelayTimerRef = useRef<number | null>(null);
  const tipCooldownTimerRef = useRef<number | null>(null);
  const tipWarmRef = useRef(false);
  const liveCheckedUrlRef = useRef("");

  const [liveMirrors, setLiveMirrors] = useState<VegaStream[] | null>(null);

  const mirrors: VegaStream[] =
    liveMirrors && liveMirrors.length > 0
      ? liveMirrors
      : request.mirrors && request.mirrors.length > 0
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
  const [buffering, setBuffering] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [bufferedEnd, setBufferedEnd] = useState(0);
  const [showRemaining, setShowRemaining] = useState(true);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [fitMode, setFitMode] = useState<FitMode>("contain");

  const [hudVisible, setHudVisible] = useState(true);
  const [hudInstant, setHudInstant] = useState(false);
  const [iconMorphing, setIconMorphing] = useState(false);
  const [bezel, setBezel] = useState<BezelState>(null);

  const [isFullscreen, setIsFullscreen] = useState(false);
  const [alwaysOnTop, setAlwaysOnTop] = useState(false);
  const [openPopover, setOpenPopover] = useState<PopoverKind>(null);
  const [showShortcuts, setShowShortcuts] = useState(false);

  const [activeTip, setActiveTip] = useState<{ id: string; text: string; instant: boolean } | null>(
    null
  );

  const [scrubDragFrac, setScrubDragFrac] = useState<number | null>(null);
  const [scrubHoverFrac, setScrubHoverFrac] = useState<number | null>(null);
  const [scrubEdgeOffsetPx, setScrubEdgeOffsetPx] = useState(0);
  const [volDragging, setVolDragging] = useState(false);

  const [levels, setLevels] = useState<Array<{ index: number; label: string }>>([]);
  const [selectedLevel, setSelectedLevel] = useState(-1);
  const [audioTracks, setAudioTracks] = useState<Array<{ index: number; label: string }>>([]);
  const [selectedAudio, setSelectedAudio] = useState(0);
  const [subtitleTracks, setSubtitleTracks] = useState<Array<{ index: number; label: string }>>([]);
  const [selectedSubtitle, setSelectedSubtitle] = useState(-1);

  const [ffmpegStatus, setFfmpegStatus] = useState<FfmpegStatus | null>(null);
  const [installingFfmpeg, setInstallingFfmpeg] = useState(false);
  const [probeResult, setProbeResult] = useState<MediaProbeResult | null>(null);
  const [proxySession, setProxySession] = useState<ProxiedStream | null>(null);
  const [useFfmpeg, setUseFfmpeg] = useState<boolean>(
    () => ffmpegEnabled && needsContainerRemux(activeStream.link)
  );
  const [ffmpegSeekSec, setFfmpegSeekSec] = useState(0);
  const [ffmpegAudioIdx, setFfmpegAudioIdx] = useState(0);
  const [ffmpegReadyTick, setFfmpegReadyTick] = useState(0);
  const [ffmpegForceTranscode, setFfmpegForceTranscode] = useState(false);

  const [errorMsg, setErrorMsg] = useState("");
  const [statusNotice, setStatusNotice] = useState("");
  const [resolvingNext, setResolvingNext] = useState(false);

  const hasNextEpisode =
    !!request.episodes &&
    request.currentEpisodeIndex !== undefined &&
    request.currentEpisodeIndex + 1 < request.episodes.length;

  const baseDuration =
    useFfmpeg && probeResult?.durationSec ? probeResult.durationSec : duration;
  const effectiveDuration = Number.isFinite(baseDuration) && baseDuration > 0 ? baseDuration : 0;
  const livePlaybackTime = useFfmpeg ? ffmpegSeekSec + currentTime : currentTime;
  const displayTime =
    scrubDragFrac !== null && effectiveDuration > 0
      ? scrubDragFrac * effectiveDuration
      : livePlaybackTime;

  const progressFrac =
    effectiveDuration > 0
      ? Math.max(0, Math.min(1, displayTime / effectiveDuration))
      : 0;
  const bufferedFrac =
    effectiveDuration > 0
      ? Math.max(0, Math.min(1, (useFfmpeg ? ffmpegSeekSec + bufferedEnd : bufferedEnd) / effectiveDuration))
      : 0;

  const isOverlayHoldingHud = Boolean(openPopover || showShortcuts || errorMsg || !playing);
  const effectiveHudVisible = hudVisible || isOverlayHoldingHud;

  function bumpHud(source: "pointer" | "keyboard" = "pointer") {
    setHudInstant(source === "keyboard");
    setHudVisible(true);
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = window.setTimeout(() => {
      if (
        videoRef.current &&
        !videoRef.current.paused &&
        !openPopover &&
        !showShortcuts &&
        !errorMsg &&
        scrubPointerIdRef.current === null
      ) {
        setHudInstant(false);
        setHudVisible(false);
      }
    }, 2800);
  }

  function showBezel(
    kind: NonNullable<BezelState>["kind"],
    label: string,
    instant = false
  ) {
    if (bezelTimerRef.current) window.clearTimeout(bezelTimerRef.current);
    setBezel({ key: Date.now(), kind, label, instant });
    bezelTimerRef.current = window.setTimeout(() => setBezel(null), 650);
  }

  function triggerPlayPause(source: "pointer" | "keyboard") {
    const vid = videoRef.current;
    if (!vid) return;
    if (source === "pointer") {
      if (iconMorphTimerRef.current) window.clearTimeout(iconMorphTimerRef.current);
      setIconMorphing(true);
      iconMorphTimerRef.current = window.setTimeout(() => setIconMorphing(false), 160);
    }
    if (vid.paused) {
      void vid.play().catch(() => {});
      if (source === "keyboard") showBezel("play", "Playing", true);
    } else {
      vid.pause();
      if (source === "keyboard") showBezel("pause", "Paused", true);
    }
  }

  function handleTipEnter(id: string, text: string) {
    if (openPopover) return;
    if (tipDelayTimerRef.current) window.clearTimeout(tipDelayTimerRef.current);
    if (tipCooldownTimerRef.current) window.clearTimeout(tipCooldownTimerRef.current);
    if (tipWarmRef.current) {
      setActiveTip({ id, text, instant: true });
      return;
    }
    tipDelayTimerRef.current = window.setTimeout(() => {
      tipWarmRef.current = true;
      setActiveTip({ id, text, instant: false });
    }, 340);
  }

  function handleTipLeave() {
    if (tipDelayTimerRef.current) window.clearTimeout(tipDelayTimerRef.current);
    setActiveTip(null);
    if (tipWarmRef.current) {
      if (tipCooldownTimerRef.current) window.clearTimeout(tipCooldownTimerRef.current);
      tipCooldownTimerRef.current = window.setTimeout(() => {
        tipWarmRef.current = false;
      }, 450);
    }
  }

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<FfmpegStatus>("ffmpeg://status", (e) => {
      setFfmpegStatus(e.payload);
      setInstallingFfmpeg(Boolean(e.payload.installing));
      if (e.payload.installed && waitingForFfmpegRef.current) {
        waitingForFfmpegRef.current = false;
        setFfmpegReadyTick((n) => n + 1);
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!openPopover) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null;
      if (!target?.closest("[data-player-popover-anchor]")) {
        setOpenPopover(null);
      }
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [openPopover]);

  useEffect(() => {
    const onFsChange = () => setIsFullscreen(Boolean(document.fullscreenElement));
    document.addEventListener("fullscreenchange", onFsChange);
    return () => document.removeEventListener("fullscreenchange", onFsChange);
  }, []);

  async function saveCurrentProgress() {
    const vid = videoRef.current;
    if (!vid || request.isLive) return;
    const rawTimeSec = useFfmpeg ? ffmpegSeekSec + (vid.currentTime || 0) : vid.currentTime || 0;
    const rawDurSec =
      useFfmpeg && probeResult?.durationSec
        ? probeResult.durationSec
        : Number.isFinite(vid.duration)
          ? vid.duration
          : 0;
    const posMs = Math.floor(rawTimeSec * 1000);
    const durMs = Math.floor(rawDurSec * 1000);
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
    setMirrorIdx(0);
    setLiveMirrors(null);
    liveCheckedUrlRef.current = "";
  }, [request.link, request.streamUrl]);

  useEffect(() => {
    setFfmpegSeekSec(0);
    setFfmpegAudioIdx(0);
    setSelectedAudio(0);
    setSelectedSubtitle(-1);
    setSubtitleTracks([]);
    setProbeResult(null);
    setBufferedEnd(0);
    setScrubDragFrac(null);
    setFfmpegForceTranscode(false);
    setUseFfmpeg(ffmpegEnabled && needsContainerRemux(activeStream.link));
  }, [activeStream.link, mirrorIdx, request.link, ffmpegEnabled]);

  useEffect(() => {
    let cancelled = false;

    async function startStream() {
      setErrorMsg("");
      setBuffering(true);
      setLevels([]);
      if (!useFfmpeg) {
        setAudioTracks([]);
      }
      setSelectedLevel(-1);

      const video = videoRef.current;
      if (!video) return;

      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }

      const ffStatus = await api.getFfmpegStatus().catch(() => null);
      if (cancelled) return;
      setFfmpegStatus(ffStatus);
      if (ffStatus?.installing) {
        setInstallingFfmpeg(true);
      }

      if (
        request.isLive &&
        request.channel &&
        liveCheckedUrlRef.current !== request.channel.streamUrl
      ) {
        liveCheckedUrlRef.current = request.channel.streamUrl;
        setStatusNotice("Checking live sources...");
        const liveRes = await api
          .resolveLiveChannelWithFallback(request.channel)
          .catch(() => null);
        if (cancelled) return;
        if (liveRes) {
          if (liveRes.error || !liveRes.selectedStream) {
            setBuffering(false);
            setStatusNotice("");
            setErrorMsg(
              `${liveRes.error || "Channel stream offline or unavailable"}. Try another channel or Open in External Player (VLC/mpv).`
            );
            return;
          }
          if (liveRes.allStreams.length > 0) {
            setLiveMirrors(liveRes.allStreams);
            if (liveRes.message) {
              setStatusNotice(liveRes.message);
              window.setTimeout(() => setStatusNotice(""), 3500);
            } else {
              setStatusNotice("");
            }
            if (liveRes.allStreams[0].link !== activeStream.link) {
              setMirrorIdx(0);
              return;
            }
          }
        }
      }

      const mergedHeaders = {
        ...(request.headers ?? {}),
        ...(activeStream.headers ?? {}),
      };

      const lower = activeStream.link.toLowerCase();
      if (lower.startsWith("rtmp://") || lower.startsWith("rtmps://")) {
        if (!ffStatus?.installed) {
          setBuffering(false);
          if (ffStatus?.installing) {
            waitingForFfmpegRef.current = true;
          }
          setErrorMsg(
            ffStatus?.installing
              ? "Setting up FFmpeg automatically... Playback will start as soon as setup finishes, or open in VLC / mpv."
              : "RTMP streams require FFmpeg or VLC / mpv."
          );
          return;
        }
      }

      let proxied: ProxiedStream;
      try {
        proxied = await api.prepareStreamProxy(activeStream.link, mergedHeaders);
        if (cancelled) return;
        setProxySession(proxied);
      } catch (err) {
        setBuffering(false);
        if (!cancelled) setErrorMsg(String(err));
        return;
      }

      const resumeMs = request.isLive
        ? 0
        : await api.getResumePositionMs(request.link).catch(() => 0);
      if (cancelled) return;

      let probe = probeResult;
      const shouldProbe =
        ffmpegEnabled &&
        ffStatus?.installed &&
        (useFfmpeg ||
          needsContainerRemux(activeStream.link) ||
          /\.(mp4|mkv|avi|mov|ts|flv|wmv)(\?|#|$)/i.test(lower) ||
          (!lower.includes(".m3u8") && !request.isLive));

      if (shouldProbe && !probe) {
        probe = await api
          .probeMediaInfo(activeStream.link, mergedHeaders)
          .catch(() => null);
        if (cancelled) return;
        if (probe) {
          setProbeResult(probe);
          if (probe.audioTracks.length > 0) {
            setAudioTracks(
              probe.audioTracks.map((t) => ({
                index: t.index,
                label: `${t.title} (${t.codec.toUpperCase()})`,
              }))
            );
          }
          setSubtitleTracks(
            probe.subtitleTracks
              .filter((s) => s.textBased)
              .map((s) => ({
                index: s.index,
                label: `${s.title} (${s.codec.toUpperCase()})`,
              }))
          );
          if (!useFfmpeg && probe.needsTranscode) {
            setStatusNotice(probe.needsReason || "Remuxing with FFmpeg...");
            setUseFfmpeg(true);
            return;
          }
        }
      }

      if (useFfmpeg) {
        if (!ffStatus?.installed) {
          setBuffering(false);
          if (ffStatus?.installing) {
            waitingForFfmpegRef.current = true;
            setStatusNotice("Setting up FFmpeg automatically...");
            return;
          }
          setErrorMsg(
            "This format requires FFmpeg for in-app playback. Setting up FFmpeg or open in VLC/mpv."
          );
          return;
        }
        const initialSeek =
          ffmpegSeekSec > 0
            ? ffmpegSeekSec
            : resumeMs > 5000 &&
                (!probe?.durationSec || resumeMs / 1000 < probe.durationSec - 5)
              ? Math.floor(resumeMs / 1000)
              : 0;
        if (initialSeek !== ffmpegSeekSec && ffmpegSeekSec === 0 && initialSeek > 0) {
          setFfmpegSeekSec(initialSeek);
          return;
        }
        const vc = (probe?.videoCodec || "").toLowerCase();
        const copyVideo = !ffmpegForceTranscode && (!vc || vc === "h264" || vc === "avc1");
        const transcodeUrl = `${proxied.transcodeBaseUrl}&audioTrack=${ffmpegAudioIdx}&startSec=${ffmpegSeekSec}&copyVideo=${copyVideo}`;
        setStatusNotice(
          copyVideo ? "FFmpeg Remux (Direct Video)" : `FFmpeg Live Transcode (${vc.toUpperCase()})`
        );
        video.src = transcodeUrl;
        video.onloadedmetadata = () => {
          video.playbackRate = speed;
          void video.play().catch(() => {});
        };
        return;
      }

      setStatusNotice("");
      const playUrl = proxied.proxyUrl;
      const looksHls =
        lower.includes(".m3u8") ||
        lower.includes("/hls/") ||
        activeStream.type.toLowerCase() === "m3u8" ||
        activeStream.type.toLowerCase() === "hls";

      const attachResume = () => {
        if (resumeMs > 0 && video.duration && resumeMs / 1000 < video.duration - 5) {
          video.currentTime = resumeMs / 1000;
        }
        video.playbackRate = speed;
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
            setBuffering(false);
            const httpCode = data.response?.code;
            if (httpCode === 404 || httpCode === 410) {
              hls.destroy();
              hlsRef.current = null;
              setStatusNotice("");
              setErrorMsg(
                `Channel stream offline or unavailable (HTTP ${httpCode}). Try another channel, mirror, or VLC/mpv.`
              );
              return;
            }
            if (data.type === Hls.ErrorTypes.NETWORK_ERROR && !looksHls) {
              hls.destroy();
              hlsRef.current = null;
              video.src = playUrl;
              video.onloadedmetadata = attachResume;
            } else if (ffStatus?.installed && !useFfmpeg) {
              hls.destroy();
              hlsRef.current = null;
              setStatusNotice("Auto-switching to FFmpeg Remux...");
              setUseFfmpeg(true);
            } else {
              setErrorMsg(
                `Playback failed (${data.details}). Enable FFmpeg or open in VLC/mpv.`
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
  }, [activeStream.link, mirrorIdx, request.link, useFfmpeg, ffmpegSeekSec, ffmpegAudioIdx, ffmpegReadyTick, ffmpegForceTranscode]);

  useEffect(() => {
    const id = window.setInterval(() => {
      void saveCurrentProgress();
    }, 10000);
    return () => window.clearInterval(id);
  }, [activeStream.link, request.link, useFfmpeg, ffmpegSeekSec]);

  useEffect(() => {
    return () => {
      void api.setAlwaysOnTop(false).catch(() => {});
    };
  }, []);

  function seekRelative(deltaSec: number, source: "pointer" | "keyboard" = "keyboard") {
    const target = Math.max(
      0,
      effectiveDuration > 0 ? Math.min(effectiveDuration, livePlaybackTime + deltaSec) : livePlaybackTime + deltaSec
    );
    if (useFfmpeg) {
      setFfmpegSeekSec(Math.floor(target));
    } else if (videoRef.current) {
      videoRef.current.currentTime = target;
      setCurrentTime(target);
    }
    if (source === "keyboard") {
      showBezel(
        deltaSec < 0 ? "seek-back" : "seek-fwd",
        deltaSec < 0 ? `${deltaSec}s` : `+${deltaSec}s`,
        true
      );
    }
  }

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT") return;
      const vid = videoRef.current;
      if (!vid) return;
      bumpHud("keyboard");

      if (e.key === "?") {
        e.preventDefault();
        setShowShortcuts((v) => !v);
        setOpenPopover(null);
        return;
      }

      switch (e.key.toLowerCase()) {
        case " ":
        case "k":
          e.preventDefault();
          triggerPlayPause("keyboard");
          break;
        case "arrowleft":
          e.preventDefault();
          seekRelative(-10, "keyboard");
          break;
        case "arrowright":
          e.preventDefault();
          seekRelative(10, "keyboard");
          break;
        case "j":
          seekRelative(-30, "keyboard");
          break;
        case "l":
          seekRelative(30, "keyboard");
          break;
        case "arrowup":
          e.preventDefault();
          setVolume((v) => {
            const next = Math.min(1, +(v + 0.05).toFixed(2));
            vid.volume = next;
            vid.muted = next === 0;
            setMuted(next === 0);
            showBezel("volume", `Volume ${Math.round(next * 100)}%`, true);
            return next;
          });
          break;
        case "arrowdown":
          e.preventDefault();
          setVolume((v) => {
            const next = Math.max(0, +(v - 0.05).toFixed(2));
            vid.volume = next;
            vid.muted = next === 0;
            setMuted(next === 0);
            showBezel(
              next === 0 ? "mute" : "volume",
              next === 0 ? "Muted" : `Volume ${Math.round(next * 100)}%`,
              true
            );
            return next;
          });
          break;
        case "m":
          vid.muted = !vid.muted;
          setMuted(vid.muted);
          showBezel(
            vid.muted ? "mute" : "volume",
            vid.muted ? "Muted" : `Volume ${Math.round(volume * 100)}%`,
            true
          );
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
          if (showShortcuts) {
            setShowShortcuts(false);
          } else if (openPopover) {
            setOpenPopover(null);
          } else if (document.fullscreenElement) {
            void document.exitFullscreen();
          } else {
            handleExit();
          }
          break;
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [hasNextEpisode, alwaysOnTop, useFfmpeg, livePlaybackTime, effectiveDuration, volume, openPopover, showShortcuts]);

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

  async function handleInstallFfmpegAndRetry() {
    setInstallingFfmpeg(true);
    setErrorMsg("");
    setStatusNotice("Setting up latest FFmpeg automatically...");
    try {
      const status = await api.installFfmpegRuntime();
      setFfmpegStatus(status);
      setStatusNotice("FFmpeg ready. Starting playback...");
      setUseFfmpeg(true);
    } catch (err) {
      setErrorMsg(String(err));
      setStatusNotice("");
    } finally {
      setInstallingFfmpeg(false);
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

  // ── 1:1 Pointer Events for Timeline Scrubber ──────────────────────────────

  function computeTrackFraction(clientX: number, el: HTMLDivElement | null) {
    if (!el) return { frac: 0, overshootPx: 0 };
    const rect = el.getBoundingClientRect();
    const rawX = clientX - rect.left;
    const frac = Math.max(0, Math.min(1, rawX / Math.max(1, rect.width)));
    let overshootPx = 0;
    if (rawX < 0) overshootPx = rubberband(rawX, 24, 0.35);
    else if (rawX > rect.width) overshootPx = rubberband(rawX - rect.width, 24, 0.35);
    return { frac, overshootPx: Math.max(-7, Math.min(7, overshootPx)) };
  }

  function onScrubPointerDown(e: React.PointerEvent<HTMLDivElement>) {
    if ((e.button !== 0 && e.pointerType === "mouse") || request.isLive || effectiveDuration <= 0) {
      return;
    }
    e.currentTarget.setPointerCapture(e.pointerId);
    scrubPointerIdRef.current = e.pointerId;
    const { frac, overshootPx } = computeTrackFraction(e.clientX, scrubRef.current);
    setScrubDragFrac(frac);
    setScrubHoverFrac(frac);
    setScrubEdgeOffsetPx(overshootPx);
    bumpHud("pointer");
  }

  function onScrubPointerMove(e: React.PointerEvent<HTMLDivElement>) {
    if (request.isLive || effectiveDuration <= 0) return;
    const { frac, overshootPx } = computeTrackFraction(e.clientX, scrubRef.current);
    setScrubHoverFrac(frac);
    if (scrubPointerIdRef.current === e.pointerId) {
      setScrubDragFrac(frac);
      setScrubEdgeOffsetPx(overshootPx);
      bumpHud("pointer");
    }
  }

  function finishScrub(e: React.PointerEvent<HTMLDivElement>, commit: boolean) {
    if (scrubPointerIdRef.current !== e.pointerId) return;
    scrubPointerIdRef.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    const targetFrac = scrubDragFrac;
    setScrubDragFrac(null);
    setScrubEdgeOffsetPx(0);
    if (commit && targetFrac !== null && effectiveDuration > 0) {
      const targetSec = targetFrac * effectiveDuration;
      if (useFfmpeg) {
        setFfmpegSeekSec(Math.floor(targetSec));
      } else if (videoRef.current) {
        videoRef.current.currentTime = targetSec;
        setCurrentTime(targetSec);
      }
    }
  }

  // ── 1:1 Pointer Events for Volume Capsule ─────────────────────────────────

  function applyVolumeFraction(clientX: number) {
    const { frac } = computeTrackFraction(clientX, volRef.current);
    const next = +frac.toFixed(2);
    setVolume(next);
    setMuted(next === 0);
    if (videoRef.current) {
      videoRef.current.volume = next;
      videoRef.current.muted = next === 0;
    }
  }

  function onVolPointerDown(e: React.PointerEvent<HTMLDivElement>) {
    if (e.button !== 0 && e.pointerType === "mouse") return;
    e.currentTarget.setPointerCapture(e.pointerId);
    volPointerIdRef.current = e.pointerId;
    setVolDragging(true);
    applyVolumeFraction(e.clientX);
    bumpHud("pointer");
  }

  function onVolPointerMove(e: React.PointerEvent<HTMLDivElement>) {
    if (volPointerIdRef.current !== e.pointerId) return;
    applyVolumeFraction(e.clientX);
    bumpHud("pointer");
  }

  function onVolPointerUp(e: React.PointerEvent<HTMLDivElement>) {
    if (volPointerIdRef.current !== e.pointerId) return;
    volPointerIdRef.current = null;
    setVolDragging(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
  }

  const activeBanner =
    statusNotice || (buffering && !errorMsg ? "Buffering stream..." : "");
  const previewFrac = scrubDragFrac ?? scrubHoverFrac;
  const volFrac = muted ? 0 : volume;
  const totalTrackCount = audioTracks.length + subtitleTracks.length;

  return (
    <div
      ref={containerRef}
      className={`player-shell${!effectiveHudVisible ? " hud-idle" : ""}`}
      onPointerMove={() => bumpHud("pointer")}
    >
      {/* Background Video Stage (pushes subtly back behind sheets) */}
      <div
        className={`player-stage${errorMsg || showShortcuts ? " sheet-open" : ""}`}
        onClick={() => {
          if (openPopover) {
            setOpenPopover(null);
          } else {
            triggerPlayPause("pointer");
          }
          bumpHud("pointer");
        }}
        onDoubleClick={() => void toggleFullscreen()}
      >
        <video
          ref={videoRef}
          className="player-video"
          crossOrigin="anonymous"
          style={{ objectFit: fitMode }}
          onPlay={() => {
            setPlaying(true);
            setBuffering(false);
            bumpHud("pointer");
          }}
          onPlaying={() => setBuffering(false)}
          onWaiting={() => setBuffering(true)}
          onCanPlay={() => setBuffering(false)}
          onPause={() => {
            setPlaying(false);
            setHudVisible(true);
            void saveCurrentProgress();
          }}
          onProgress={() => {
            const vid = videoRef.current;
            if (!vid || vid.buffered.length === 0) return;
            const t = vid.currentTime;
            for (let i = 0; i < vid.buffered.length; i++) {
              if (t >= vid.buffered.start(i) - 1 && t <= vid.buffered.end(i)) {
                setBufferedEnd(vid.buffered.end(i));
                return;
              }
            }
            setBufferedEnd(vid.buffered.end(vid.buffered.length - 1));
          }}
          onTimeUpdate={() => {
            if (videoRef.current) setCurrentTime(videoRef.current.currentTime);
          }}
          onDurationChange={() => {
            if (videoRef.current) setDuration(videoRef.current.duration || 0);
          }}
          onError={() => {
            void (async () => {
              setBuffering(false);
              if (!useFfmpeg && ffmpegStatus?.installed) {
                if (hlsRef.current) {
                  hlsRef.current.destroy();
                  hlsRef.current = null;
                }
                setStatusNotice("Auto-switching to FFmpeg Remux...");
                setUseFfmpeg(true);
                return;
              }
              if (!hlsRef.current) {
                if (useFfmpeg && proxySession) {
                  const reason = await fetch(proxySession.proxyUrl.replace("/stream", "/transcode-error"))
                    .then((r) => (r.ok ? r.text() : ""))
                    .catch(() => "");
                  if (reason.trim()) {
                    setStatusNotice("");
                    setErrorMsg(
                      `${reason.trim()}. Try another mirror or Open in External Player (VLC/mpv).`
                    );
                    return;
                  }
                  if (!ffmpegForceTranscode) {
                    setStatusNotice("Transcoding stream to H.264...");
                    setFfmpegForceTranscode(true);
                    return;
                  }
                }
                setStatusNotice("");
                setErrorMsg(
                  ffmpegStatus?.installed
                    ? "Unable to decode this stream even with FFmpeg. Try another mirror or Open in External Player (VLC/mpv)."
                    : "WebView could not decode this format. Setup FFmpeg automatically or open in VLC/mpv."
                );
              }
            })();
          }}
        >
          {proxySession && selectedSubtitle >= 0 && (
            <track
              key={`sub-${selectedSubtitle}-${proxySession.sessionId}`}
              kind="subtitles"
              src={`${proxySession.subtitleBaseUrl}&track=${selectedSubtitle}`}
              srcLang="en"
              label={subtitleTracks.find((s) => s.index === selectedSubtitle)?.label || "Subtitles"}
              default
            />
          )}
        </video>
      </div>

      {/* Soft Edge Vignettes (contrast over bright video frames) */}
      <div
        className={`player-scrim-top ${effectiveHudVisible ? "visible" : "hidden"}${hudInstant ? " instant" : ""}`}
      />
      <div
        className={`player-scrim-bottom ${effectiveHudVisible ? "visible" : "hidden"}${hudInstant ? " instant" : ""}`}
      />

      {/* Dynamic Top Status / Buffering Pill */}
      {activeBanner && !errorMsg && (
        <div className="player-status-pill" role="status" aria-live="polite">
          <Loader2 size={14} className="player-spinner" color="#ffc062" />
          <span>{activeBanner}</span>
        </div>
      )}

      {/* Center Action Bezel (instant on keyboard, quiet on fade) */}
      {bezel && !errorMsg && (
        <div key={bezel.key} className="player-bezel" data-instant={bezel.instant ? "true" : "false"}>
          {bezel.kind === "play" && <Play size={18} fill="#ffc062" color="#ffc062" />}
          {bezel.kind === "pause" && <Pause size={18} fill="#ffc062" color="#ffc062" />}
          {bezel.kind === "seek-back" && <RotateCcw size={18} color="#ffc062" />}
          {bezel.kind === "seek-fwd" && <RotateCw size={18} color="#ffc062" />}
          {bezel.kind === "volume" && <Volume2 size={18} color="#ffc062" />}
          {bezel.kind === "mute" && <VolumeX size={18} color="#ff8080" />}
          <span>{bezel.label}</span>
        </div>
      )}

      {/* Floating Top Chrome Bar */}
      <div
        className={`player-hud-top ${effectiveHudVisible ? "visible" : "hidden"}${hudInstant ? " instant" : ""}`}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 12, minWidth: 0 }}>
          <button type="button" className="player-btn" onClick={handleExit} aria-label="Back">
            <ArrowLeft size={15} />
            <span>Back</span>
          </button>
          <ChannelLogo src={request.imageUrl} name={request.title} size={38} hideOnMissing />
          <div style={{ minWidth: 0 }}>
            <div className="player-title">{request.title}</div>
            <div className="player-subtitle">
              <span>{activeStream.server}</span>
              {activeStream.quality && <span>• {activeStream.quality}</span>}
              {probeResult?.videoCodec && <span>• {probeResult.videoCodec.toUpperCase()}</span>}
              {useFfmpeg && <span style={{ color: "var(--accent-amber)" }}>• FFmpeg</span>}
            </div>
          </div>
        </div>

        <div className="player-cluster">
          {mirrors.length > 1 && (
            <div className="player-btn-wrap" data-player-popover-anchor>
              <button
                type="button"
                className={`player-btn${openPopover === "mirrors" ? " active" : ""}`}
                onClick={() => setOpenPopover((p) => (p === "mirrors" ? null : "mirrors"))}
                onMouseEnter={() => handleTipEnter("mirrors", "Switch Mirror")}
                onMouseLeave={handleTipLeave}
                aria-label="Mirrors"
              >
                <Layers size={14} />
                <span>{activeStream.server || `Mirror ${mirrorIdx + 1}`}</span>
              </button>
              {activeTip?.id === "mirrors" && openPopover !== "mirrors" && (
                <div className="player-tooltip" data-side="bottom" data-instant={activeTip.instant ? "true" : "false"}>
                  {activeTip.text}
                </div>
              )}
              {openPopover === "mirrors" && (
                <div className="player-popover" data-anchor="top-right">
                  <div className="player-menu-label">Stream Mirrors ({mirrors.length})</div>
                  {mirrors.map((m, i) => (
                    <button
                      type="button"
                      key={`${m.link}-${i}`}
                      className={`player-menu-item${mirrorIdx === i ? " active" : ""}`}
                      onClick={() => {
                        setMirrorIdx(i);
                        setOpenPopover(null);
                      }}
                    >
                      <span>
                        {m.server || `Mirror ${i + 1}`} ({m.quality || "HD"})
                      </span>
                      {mirrorIdx === i && <Check size={14} />}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}

          <div className="player-btn-wrap">
            <button
              type="button"
              className={`player-btn${alwaysOnTop ? " active" : ""}`}
              onClick={() => void togglePin()}
              onMouseEnter={() => handleTipEnter("pin", "Float on Top (P)")}
              onMouseLeave={handleTipLeave}
              aria-label="Float on Top"
            >
              <PictureInPicture2 size={14} />
            </button>
            {activeTip?.id === "pin" && (
              <div className="player-tooltip" data-side="bottom" data-instant={activeTip.instant ? "true" : "false"}>
                {activeTip.text}
              </div>
            )}
          </div>

          <div className="player-btn-wrap">
            <button
              type="button"
              className={`player-btn${showShortcuts ? " active" : ""}`}
              onClick={() => {
                setOpenPopover(null);
                setShowShortcuts((s) => !s);
              }}
              onMouseEnter={() => handleTipEnter("shortcuts", "Keyboard Shortcuts (?)")}
              onMouseLeave={handleTipLeave}
              aria-label="Keyboard Shortcuts"
            >
              <Keyboard size={14} />
            </button>
            {activeTip?.id === "shortcuts" && (
              <div className="player-tooltip" data-side="bottom" data-instant={activeTip.instant ? "true" : "false"}>
                {activeTip.text}
              </div>
            )}
          </div>

          <div className="player-btn-wrap">
            <button
              type="button"
              className="player-btn"
              onClick={() => void openExternal()}
              onMouseEnter={() => handleTipEnter("external", "Open in VLC / mpv")}
              onMouseLeave={handleTipLeave}
              aria-label="Open in External Player"
            >
              <ExternalLink size={14} />
              <span>External</span>
            </button>
            {activeTip?.id === "external" && (
              <div className="player-tooltip" data-side="bottom" data-instant={activeTip.instant ? "true" : "false"}>
                {activeTip.text}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Floating Bottom Control Deck */}
      <div
        className={`player-hud-bottom ${effectiveHudVisible ? "visible" : "hidden"}${hudInstant ? " instant" : ""}`}
        onClick={(e) => e.stopPropagation()}
      >
        {/* 1:1 Pointer-Captured Scrub Bar */}
        {!request.isLive && (
          <div
            ref={scrubRef}
            role="slider"
            aria-label="Playback Timeline"
            aria-valuemin={0}
            aria-valuemax={Math.round(effectiveDuration)}
            aria-valuenow={Math.round(displayTime)}
            tabIndex={0}
            className={`player-scrub-wrap${scrubDragFrac !== null ? " dragging" : ""}`}
            onPointerDown={onScrubPointerDown}
            onPointerMove={onScrubPointerMove}
            onPointerUp={(e) => finishScrub(e, true)}
            onPointerCancel={(e) => finishScrub(e, false)}
            onPointerLeave={() => {
              if (scrubPointerIdRef.current === null) setScrubHoverFrac(null);
            }}
          >
            {previewFrac !== null && effectiveDuration > 0 && (
              <div
                className="player-scrub-preview"
                style={{
                  left: `clamp(34px, ${(previewFrac * 100).toFixed(2)}%, calc(100% - 34px))`,
                }}
              >
                {formatTime(previewFrac * effectiveDuration)}
              </div>
            )}
            <div
              className="player-scrub-rail"
              style={
                scrubEdgeOffsetPx !== 0
                  ? { transform: `translateX(${scrubEdgeOffsetPx}px) scaleY(1.75)` }
                  : undefined
              }
            >
              <div className="player-scrub-buffer" style={{ width: `${bufferedFrac * 100}%` }} />
              <div className="player-scrub-fill" style={{ width: `${progressFrac * 100}%` }} />
              <div className="player-scrub-thumb" style={{ left: `${progressFrac * 100}%` }} />
            </div>
          </div>
        )}

        <div className="player-controls-row">
          {/* Left Transport & Volume Capsule */}
          <div className="player-cluster">
            <div className="player-btn-wrap">
              <button
                type="button"
                className="player-btn player-play-btn"
                aria-label={playing ? "Pause" : "Play"}
                onClick={() => triggerPlayPause("pointer")}
                onMouseEnter={() => handleTipEnter("play", playing ? "Pause (Space)" : "Play (Space)")}
                onMouseLeave={handleTipLeave}
              >
                <span className={`player-icon-swap${iconMorphing ? " morphing" : ""}`}>
                  {playing ? <Pause size={17} fill="currentColor" /> : <Play size={17} fill="currentColor" />}
                </span>
              </button>
              {activeTip?.id === "play" && (
                <div className="player-tooltip" data-side="top" data-instant={activeTip.instant ? "true" : "false"}>
                  {activeTip.text}
                </div>
              )}
            </div>

            {!request.isLive && (
              <>
                <div className="player-btn-wrap">
                  <button
                    type="button"
                    className="player-btn"
                    onClick={() => seekRelative(-10, "pointer")}
                    onMouseEnter={() => handleTipEnter("back10", "Back 10s (←)")}
                    onMouseLeave={handleTipLeave}
                    aria-label="Seek back 10 seconds"
                  >
                    <RotateCcw size={14} />
                    <span>10</span>
                  </button>
                  {activeTip?.id === "back10" && (
                    <div className="player-tooltip" data-side="top" data-instant={activeTip.instant ? "true" : "false"}>
                      {activeTip.text}
                    </div>
                  )}
                </div>

                <div className="player-btn-wrap">
                  <button
                    type="button"
                    className="player-btn"
                    onClick={() => seekRelative(10, "pointer")}
                    onMouseEnter={() => handleTipEnter("fwd10", "Forward 10s (→)")}
                    onMouseLeave={handleTipLeave}
                    aria-label="Seek forward 10 seconds"
                  >
                    <RotateCw size={14} />
                    <span>10</span>
                  </button>
                  {activeTip?.id === "fwd10" && (
                    <div className="player-tooltip" data-side="top" data-instant={activeTip.instant ? "true" : "false"}>
                      {activeTip.text}
                    </div>
                  )}
                </div>
              </>
            )}

            <div className="player-btn-wrap">
              <button
                type="button"
                className="player-btn"
                onClick={() => {
                  const vid = videoRef.current;
                  if (!vid) return;
                  vid.muted = !vid.muted;
                  setMuted(vid.muted);
                }}
                onMouseEnter={() => handleTipEnter("mute", muted ? "Unmute (M)" : "Mute (M)")}
                onMouseLeave={handleTipLeave}
                aria-label={muted || volume === 0 ? "Unmute" : "Mute"}
              >
                {muted || volume === 0 ? <VolumeX size={15} /> : <Volume2 size={15} />}
              </button>
              {activeTip?.id === "mute" && (
                <div className="player-tooltip" data-side="top" data-instant={activeTip.instant ? "true" : "false"}>
                  {activeTip.text}
                </div>
              )}
            </div>

            <div
              ref={volRef}
              role="slider"
              aria-label="Volume"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(volFrac * 100)}
              tabIndex={0}
              className={`player-vol-wrap${volDragging ? " dragging" : ""}`}
              onPointerDown={onVolPointerDown}
              onPointerMove={onVolPointerMove}
              onPointerUp={onVolPointerUp}
              onPointerCancel={onVolPointerUp}
            >
              <div className="player-vol-rail">
                <div className="player-vol-fill" style={{ width: `${volFrac * 100}%` }} />
                <div className="player-vol-thumb" style={{ left: `${volFrac * 100}%` }} />
              </div>
            </div>

            {request.isLive ? (
              <span className="player-live-pill">
                <span className="player-live-dot" /> LIVE
              </span>
            ) : (
              <button
                type="button"
                className="player-time-readout"
                onClick={() => setShowRemaining((r) => !r)}
                title="Click to toggle remaining time"
              >
                {effectiveDuration > 0
                  ? showRemaining
                    ? `${formatTime(displayTime)} / -${formatTime(Math.max(0, effectiveDuration - displayTime))}`
                    : `${formatTime(displayTime)} / ${formatTime(effectiveDuration)}`
                  : formatTime(displayTime)}
              </button>
            )}
          </div>

          {/* Right Track, Quality & Fullscreen Controls */}
          <div className="player-cluster">
            {hasNextEpisode && (
              <div className="player-btn-wrap">
                <button
                  type="button"
                  className="player-btn"
                  disabled={resolvingNext}
                  onClick={() => void playNextEpisode()}
                  onMouseEnter={() => handleTipEnter("next", "Next Episode (N)")}
                  onMouseLeave={handleTipLeave}
                >
                  <SkipForward size={14} />
                  <span>Next</span>
                </button>
                {activeTip?.id === "next" && (
                  <div className="player-tooltip" data-side="top" data-instant={activeTip.instant ? "true" : "false"}>
                    {activeTip.text}
                  </div>
                )}
              </div>
            )}

            {totalTrackCount > 0 && (
              <div className="player-btn-wrap" data-player-popover-anchor>
                <button
                  type="button"
                  className={`player-btn${openPopover === "tracks" || selectedSubtitle >= 0 ? " active" : ""}`}
                  onClick={() => setOpenPopover((p) => (p === "tracks" ? null : "tracks"))}
                  onMouseEnter={() => handleTipEnter("tracks", "Audio & Subtitles")}
                  onMouseLeave={handleTipLeave}
                  aria-label="Audio and Subtitles"
                >
                  <Subtitles size={15} />
                  <span>Audio & CC</span>
                </button>
                {activeTip?.id === "tracks" && openPopover !== "tracks" && (
                  <div className="player-tooltip" data-side="top" data-instant={activeTip.instant ? "true" : "false"}>
                    {activeTip.text}
                  </div>
                )}
                {openPopover === "tracks" && (
                  <div className="player-popover" data-anchor="bottom-right">
                    {audioTracks.length > 0 && (
                      <>
                        <div className="player-menu-label">Audio Track</div>
                        {audioTracks.map((a) => {
                          const active = (useFfmpeg ? ffmpegAudioIdx : selectedAudio) === a.index;
                          return (
                            <button
                              type="button"
                              key={a.index}
                              className={`player-menu-item${active ? " active" : ""}`}
                              onClick={() => {
                                setSelectedAudio(a.index);
                                if (useFfmpeg) {
                                  setFfmpegSeekSec(Math.floor(livePlaybackTime));
                                  setFfmpegAudioIdx(a.index);
                                } else if (hlsRef.current) {
                                  hlsRef.current.audioTrack = a.index;
                                } else if (ffmpegStatus?.installed) {
                                  setFfmpegSeekSec(Math.floor(livePlaybackTime));
                                  setFfmpegAudioIdx(a.index);
                                  setUseFfmpeg(true);
                                }
                                setOpenPopover(null);
                              }}
                            >
                              <span>{a.label}</span>
                              {active && <Check size={14} />}
                            </button>
                          );
                        })}
                      </>
                    )}
                    {subtitleTracks.length > 0 && (
                      <>
                        <div className="player-menu-label" style={{ marginTop: 4 }}>
                          Subtitles
                        </div>
                        <button
                          type="button"
                          className={`player-menu-item${selectedSubtitle === -1 ? " active" : ""}`}
                          onClick={() => {
                            setSelectedSubtitle(-1);
                            setOpenPopover(null);
                          }}
                        >
                          <span>Off</span>
                          {selectedSubtitle === -1 && <Check size={14} />}
                        </button>
                        {subtitleTracks.map((s) => (
                          <button
                            type="button"
                            key={s.index}
                            className={`player-menu-item${selectedSubtitle === s.index ? " active" : ""}`}
                            onClick={() => {
                              setSelectedSubtitle(s.index);
                              setOpenPopover(null);
                            }}
                          >
                            <span>{s.label}</span>
                            {selectedSubtitle === s.index && <Check size={14} />}
                          </button>
                        ))}
                      </>
                    )}
                  </div>
                )}
              </div>
            )}

            <div className="player-btn-wrap" data-player-popover-anchor>
              <button
                type="button"
                className={`player-btn${openPopover === "settings" ? " active" : ""}`}
                onClick={() => setOpenPopover((p) => (p === "settings" ? null : "settings"))}
                onMouseEnter={() => handleTipEnter("settings", "Playback Settings")}
                onMouseLeave={handleTipLeave}
                aria-label="Playback Settings"
              >
                <SlidersHorizontal size={14} />
                {speed !== 1 && <span>{speed}x</span>}
              </button>
              {activeTip?.id === "settings" && openPopover !== "settings" && (
                <div className="player-tooltip" data-side="top" data-instant={activeTip.instant ? "true" : "false"}>
                  {activeTip.text}
                </div>
              )}
              {openPopover === "settings" && (
                <div className="player-popover" data-anchor="bottom-right">
                  {!request.isLive && (
                    <>
                      <div className="player-menu-label">Playback Speed</div>
                      <div className="player-seg-group">
                        {SPEED_OPTIONS.map((r) => (
                          <button
                            type="button"
                            key={r}
                            className={`player-seg-item${speed === r ? " active" : ""}`}
                            onClick={() => {
                              setSpeed(r);
                              if (videoRef.current) videoRef.current.playbackRate = r;
                            }}
                          >
                            {r}x
                          </button>
                        ))}
                      </div>
                    </>
                  )}

                  <div className="player-menu-label">Aspect Ratio</div>
                  <div className="player-seg-group">
                    {FIT_OPTIONS.map((opt) => (
                      <button
                        type="button"
                        key={opt.value}
                        className={`player-seg-item${fitMode === opt.value ? " active" : ""}`}
                        onClick={() => setFitMode(opt.value)}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>

                  {levels.length > 0 && (
                    <>
                      <div className="player-menu-label">Stream Quality</div>
                      <button
                        type="button"
                        className={`player-menu-item${selectedLevel === -1 ? " active" : ""}`}
                        onClick={() => {
                          setSelectedLevel(-1);
                          if (hlsRef.current) hlsRef.current.currentLevel = -1;
                        }}
                      >
                        <span>Auto</span>
                        {selectedLevel === -1 && <Check size={14} />}
                      </button>
                      {levels.map((l) => (
                        <button
                          type="button"
                          key={l.index}
                          className={`player-menu-item${selectedLevel === l.index ? " active" : ""}`}
                          onClick={() => {
                            setSelectedLevel(l.index);
                            if (hlsRef.current) hlsRef.current.currentLevel = l.index;
                          }}
                        >
                          <span>{l.label}</span>
                          {selectedLevel === l.index && <Check size={14} />}
                        </button>
                      ))}
                    </>
                  )}

                  {ffmpegStatus?.installed && (
                    <>
                      <div className="player-menu-label">Engine</div>
                      <button
                        type="button"
                        className={`player-menu-item${useFfmpeg ? " active" : ""}`}
                        onClick={() => {
                          setFfmpegSeekSec(useFfmpeg ? 0 : Math.floor(livePlaybackTime));
                          setUseFfmpeg((prev) => !prev);
                          setOpenPopover(null);
                        }}
                      >
                        <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                          <Cpu size={14} /> FFmpeg Remux Engine
                        </span>
                        {useFfmpeg && <Check size={14} />}
                      </button>
                    </>
                  )}
                </div>
              )}
            </div>

            <div className="player-btn-wrap">
              <button
                type="button"
                className={`player-btn${isFullscreen ? " active" : ""}`}
                onClick={() => void toggleFullscreen()}
                onMouseEnter={() => handleTipEnter("fs", isFullscreen ? "Exit Fullscreen (F)" : "Fullscreen (F)")}
                onMouseLeave={handleTipLeave}
                aria-label={isFullscreen ? "Exit Fullscreen" : "Fullscreen"}
              >
                {isFullscreen ? <Minimize size={14} /> : <Maximize size={14} />}
              </button>
              {activeTip?.id === "fs" && (
                <div className="player-tooltip" data-side="top" data-instant={activeTip.instant ? "true" : "false"}>
                  {activeTip.text}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Centered Keyboard Shortcuts Modal */}
      {showShortcuts && (
        <div
          className="player-sheet-backdrop"
          onClick={() => setShowShortcuts(false)}
        >
          <div className="player-sheet" onClick={(e) => e.stopPropagation()}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, fontWeight: 700, fontSize: 16 }}>
                <Keyboard size={17} color="#ffc062" />
                <span>Keyboard Shortcuts</span>
              </div>
              <button
                type="button"
                className="player-btn"
                style={{ minWidth: 32, height: 32, padding: 0 }}
                onClick={() => setShowShortcuts(false)}
                aria-label="Close Shortcuts"
              >
                <X size={15} />
              </button>
            </div>
            <div style={{ display: "grid", gap: 8 }}>
              {SHORTCUTS.map((s) => (
                <div
                  key={s.keys}
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    fontSize: 13,
                    padding: "6px 0",
                    borderBottom: "1px solid rgba(255,255,255,0.06)",
                  }}
                >
                  <span style={{ color: "var(--text-secondary)" }}>{s.label}</span>
                  <kbd
                    style={{
                      padding: "3px 8px",
                      borderRadius: 7,
                      background: "rgba(255,255,255,0.09)",
                      border: "1px solid rgba(255,255,255,0.14)",
                      fontFamily: "inherit",
                      fontSize: 11.5,
                      fontWeight: 700,
                      color: "#ffffff",
                    }}
                  >
                    {s.keys}
                  </kbd>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {/* Centered Error Recovery Sheet */}
      {errorMsg && (
        <div className="player-sheet-backdrop">
          <div className="player-sheet">
            <h2 style={{ margin: 0, fontSize: 18, letterSpacing: "-0.02em" }}>Playback Error</h2>
            <div className="error-text">{errorMsg}</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
              {ffmpegStatus?.installed ? (
                !useFfmpeg && (
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={() => {
                      setErrorMsg("");
                      setUseFfmpeg(true);
                    }}
                  >
                    <Cpu size={14} /> Play with FFmpeg Remux
                  </button>
                )
              ) : (
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={installingFfmpeg || Boolean(ffmpegStatus?.installing)}
                  onClick={() => void handleInstallFfmpegAndRetry()}
                >
                  <Download size={14} />{" "}
                  {installingFfmpeg || ffmpegStatus?.installing
                    ? "Setting up FFmpeg..."
                    : "Install / Retry FFmpeg"}
                </button>
              )}
              <button
                type="button"
                className="btn"
                onClick={() => {
                  setErrorMsg("");
                  setFfmpegReadyTick((n) => n + 1);
                }}
              >
                <RotateCcw size={14} /> Retry
              </button>
              {mirrorIdx + 1 < mirrors.length && (
                <button
                  type="button"
                  className="btn"
                  onClick={() => {
                    setErrorMsg("");
                    setMirrorIdx((i) => i + 1);
                  }}
                >
                  Try Next Mirror
                </button>
              )}
              <button type="button" className="btn" onClick={() => void openExternal()}>
                <ExternalLink size={14} /> Open in VLC / mpv
              </button>
              <button type="button" className="btn" onClick={handleExit}>
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
