import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import playCircleSvg from "../assets/figma/play-circle.svg";

export function formatOttTitle(raw: string): string {
  const cleaned = raw
    .replace(/\[.*?\]/g, " ")
    .replace(/\b(WEB-DL|WEBRip|BluRay|HDRip|HDTV|1080p|720p|480p|2160p|4K|x264|x265|HEVC|DD5\.1|ESubs|Dual Audio|Full Movie)\b.*/i, "")
    .replace(/[|–—]+.*$/, "")
    .replace(/\s+/g, " ")
    .trim();
  return cleaned || raw.trim();
}

export interface OttCardItem {
  id: string;
  title: string;
  subtitle?: string;
  image?: string;
  progressPct?: number;
  badge?: string;
  onClick: () => void;
}

interface HeroProps {
  title: string;
  eyebrow?: string;
  backdropUrl?: string;
  ctaLabel?: string;
  secondaryLabel?: string;
  onPrimaryAction: () => void;
  onSecondaryAction?: () => void;
  slideCount?: number;
  activeSlide?: number;
  onSelectSlide?: (idx: number) => void;
}

export function HeroMarquee({
  title,
  eyebrow = "Now Streaming",
  backdropUrl,
  ctaLabel = "Play Now",
  secondaryLabel,
  onPrimaryAction,
  onSecondaryAction,
  slideCount = 0,
  activeSlide = 0,
  onSelectSlide,
}: HeroProps) {
  return (
    <section className="ott-hero" aria-label="Featured">
      <div className="ott-hero-backdrop-wrap">
        {backdropUrl ? (
          <img
            className="ott-hero-backdrop"
            src={backdropUrl}
            alt={title}
          />
        ) : (
          <div className="ott-hero-ambient" />
        )}
      </div>
      <div className="ott-hero-fade-left" />
      <div className="ott-hero-fade-bottom" />

      <div className="ott-hero-lockup">
        <h1 className="ott-hero-title">{title}</h1>
        <p className="ott-hero-eyebrow">{eyebrow}</p>

        <div className="ott-hero-actions">
          <button
            type="button"
            className="ott-play-cta"
            onClick={onPrimaryAction}
          >
            <img
              className="ott-play-cta-icon"
              src={playCircleSvg}
              alt=""
              width={48}
              height={48}
            />
            <span>{ctaLabel}</span>
          </button>

          {secondaryLabel && onSecondaryAction && (
            <button
              type="button"
              className="ott-glass-pill"
              onClick={onSecondaryAction}
            >
              {secondaryLabel}
            </button>
          )}
        </div>

        {slideCount > 1 && onSelectSlide && (
          <div className="ott-hero-dots" role="tablist" aria-label="Featured slides">
            {Array.from({ length: slideCount }).map((_, idx) => (
              <button
                key={idx}
                type="button"
                role="tab"
                aria-selected={activeSlide === idx}
                aria-label={`Featured item ${idx + 1}`}
                className={`ott-hero-dot ${activeSlide === idx ? "active" : ""}`}
                onClick={() => onSelectSlide(idx)}
              />
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

interface MidBannerProps {
  title: string;
  eyebrow?: string;
  imageUrl?: string;
  ctaLabel?: string;
  onAction: () => void;
}

export function MidBanner({
  title,
  eyebrow = "Now Streaming",
  imageUrl,
  ctaLabel = "Play Now",
  onAction,
}: MidBannerProps) {
  return (
    <div className="ott-mid-banner-wrap">
      <div className="ott-mid-banner">
        {imageUrl && (
          <img className="ott-mid-banner-img" src={imageUrl} alt={title} loading="lazy" />
        )}
        <div className="ott-mid-banner-scrim" />
        <div className="ott-mid-banner-content">
          <h2 className="ott-hero-title" style={{ fontSize: "clamp(1.85rem, 3.3vw, 3.4rem)" }}>
            {title}
          </h2>
          <p className="ott-hero-eyebrow" style={{ fontSize: "clamp(1rem, 1.5vw, 1.45rem)" }}>
            {eyebrow}
          </p>
          <button
            type="button"
            className="ott-play-cta"
            onClick={onAction}
          >
            <img
              className="ott-play-cta-icon"
              src={playCircleSvg}
              alt=""
              width={48}
              height={48}
            />
            <span>{ctaLabel}</span>
          </button>
        </div>
      </div>
    </div>
  );
}

interface CarouselProps {
  title: string;
  variant: "landscape" | "tall";
  items: OttCardItem[];
  headerAction?: ReactNode;
}

export function OttCarousel({ title, variant, items, headerAction }: CarouselProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const dragState = useRef<{
    active: boolean;
    pointerId: number;
    startX: number;
    startScroll: number;
    moved: boolean;
    samples: Array<{ x: number; t: number }>;
    rafId: number;
  }>({
    active: false,
    pointerId: -1,
    startX: 0,
    startScroll: 0,
    moved: false,
    samples: [],
    rafId: 0,
  });

  if (items.length === 0) return null;

  function stopMomentum() {
    if (dragState.current.rafId) {
      cancelAnimationFrame(dragState.current.rafId);
      dragState.current.rafId = 0;
    }
  }

  function startMomentum(velocityPxPerMs: number) {
    const track = trackRef.current;
    if (!track || Math.abs(velocityPxPerMs) < 0.05) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    let v = velocityPxPerMs;
    let last = performance.now();

    function step(now: number) {
      const dt = Math.min(32, now - last);
      last = now;
      if (!track) return;
      track.scrollLeft -= v * dt;
      // Exponential momentum decay aligned with Apple UIKit scroll deceleration
      v *= Math.pow(0.994, dt);
      if (Math.abs(v) > 0.03) {
        dragState.current.rafId = requestAnimationFrame(step);
      } else {
        dragState.current.rafId = 0;
      }
    }

    dragState.current.rafId = requestAnimationFrame(step);
  }

  function handlePointerDown(e: React.PointerEvent<HTMLDivElement>) {
    if (e.button !== 0) return;
    const track = trackRef.current;
    if (!track) return;
    // Interrupt any active momentum glide right on touch/pointer down
    stopMomentum();
    dragState.current.active = true;
    dragState.current.pointerId = e.pointerId;
    dragState.current.startX = e.clientX;
    dragState.current.startScroll = track.scrollLeft;
    dragState.current.moved = false;
    dragState.current.samples = [{ x: e.clientX, t: performance.now() }];
  }

  function handlePointerMove(e: React.PointerEvent<HTMLDivElement>) {
    const st = dragState.current;
    const track = trackRef.current;
    if (!st.active || !track) return;
    const dx = e.clientX - st.startX;
    if (!st.moved && Math.abs(dx) > 8) {
      st.moved = true;
      try {
        track.setPointerCapture(e.pointerId);
      } catch {
        // Ignore if capture is unsupported
      }
    }
    if (st.moved) {
      track.scrollLeft = st.startScroll - dx;
      const now = performance.now();
      st.samples.push({ x: e.clientX, t: now });
      if (st.samples.length > 5) st.samples.shift();
    }
  }

  function handlePointerEnd(e: React.PointerEvent<HTMLDivElement>) {
    const st = dragState.current;
    const track = trackRef.current;
    if (!st.active) return;
    st.active = false;
    try {
      if (track?.hasPointerCapture(e.pointerId)) {
        track.releasePointerCapture(e.pointerId);
      }
    } catch {
      // Ignore
    }
    if (st.moved && st.samples.length >= 2) {
      const first = st.samples[0];
      const last = st.samples[st.samples.length - 1];
      const dt = Math.max(8, last.t - first.t);
      const velocity = (last.x - first.x) / dt;
      startMomentum(velocity);
      window.setTimeout(() => {
        st.moved = false;
      }, 0);
    }
  }

  function scrollByStep(dir: -1 | 1) {
    const track = trackRef.current;
    if (!track) return;
    stopMomentum();
    const amount = Math.max(320, Math.round(track.clientWidth * 0.72));
    track.scrollBy({ left: dir * amount, behavior: "smooth" });
  }

  const cardClass = variant === "tall" ? "ott-tall-card" : "ott-landscape-card";

  return (
    <section className="ott-section">
      <div className="ott-section-head">
        <h2 className="ott-section-title">{title}</h2>
        <div className="ott-carousel-controls">
          {headerAction}
          <button
            type="button"
            className="ott-icon-circle"
            aria-label={`Scroll ${title} left`}
            onClick={() => scrollByStep(-1)}
          >
            <ChevronLeft size={16} />
          </button>
          <button
            type="button"
            className="ott-icon-circle"
            aria-label={`Scroll ${title} right`}
            onClick={() => scrollByStep(1)}
          >
            <ChevronRight size={16} />
          </button>
        </div>
      </div>

      <div
        ref={trackRef}
        className="ott-carousel-track"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerEnd}
        onPointerCancel={handlePointerEnd}
      >
        {items.map((item) => (
          <OttCard
            key={item.id}
            item={item}
            cardClass={cardClass}
            canClick={() => !dragState.current.moved}
          />
        ))}
      </div>
    </section>
  );
}

function OttCard({
  item,
  cardClass,
  canClick,
}: {
  item: OttCardItem;
  cardClass: string;
  canClick: () => boolean;
}) {
  const [imgFailed, setImgFailed] = useState(false);

  useEffect(() => {
    setImgFailed(false);
  }, [item.image]);

  return (
    <button
      type="button"
      className={cardClass}
      onClick={() => {
        if (canClick()) item.onClick();
      }}
    >
      {item.image && !imgFailed ? (
        <img
          className="ott-card-media"
          src={item.image}
          alt={item.title}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          draggable={false}
          onError={() => setImgFailed(true)}
        />
      ) : (
        <div
          className="ott-card-media"
          style={{
            background:
              "radial-gradient(circle at 75% 22%, rgba(255,192,98,0.22), rgba(24,24,28,1) 70%)",
          }}
        />
      )}
      <div className="ott-card-scrim" />
      <div className="ott-card-caption">
        {item.badge && (
          <span className="badge" style={{ width: "fit-content" }}>
            {item.badge}
          </span>
        )}
        <h3 className="ott-tall-title">{item.title}</h3>
        {item.subtitle && <div className="ott-card-sub">{item.subtitle}</div>}
        {item.progressPct !== undefined && item.progressPct > 0 && (
          <div className="ott-progress-bar">
            <div
              className="ott-progress-fill"
              style={{ width: `${Math.min(100, item.progressPct)}%` }}
            />
          </div>
        )}
      </div>
    </button>
  );
}
