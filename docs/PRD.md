# IPTVMine Desktop — Product Requirements Document (PRD)

**Document Status:** Draft v1.0  
**Reference Codebase:** `IPTVMine-Pro` (Android / Kotlin / Jetpack Compose / Media3)  
**Target Stack:** Rust Backend + Tauri v2 + React + TypeScript + Vite (Cargo-first workflow)  
**Target Platforms:** Windows (x86_64/ARM64), macOS (Universal), Linux (x86_64 AppImage/deb)

---

## 1. Executive Summary & Vision

**IPTVMine Desktop** brings the core capabilities of **IPTVMine Pro** to desktop operating systems as a fast, lightweight native application. Built on a **Rust + Tauri v2** core with a **React + TypeScript** frontend, it lets users aggregate Live TV playlists (M3U/M3U8/IPTV JSON), browse Video-on-Demand (VOD) catalogs via **Vega-compatible JS/TS provider extensions**, stream high-bitrate media (HLS, DASH, MP4, MKV, RTMP), manage offline VOD downloads, track watch history, and bypass ISP DNS blocks via built-in DNS-over-HTTPS (DoH).

> **Disclaimer (Carried from IPTVMine-Pro):** IPTVMine Desktop does not host, store, or provide any media content. All playlists, extensions, and media streams are user-configured and sourced directly by the user.

---

## 2. Goals & Non-Goals

### 2.1 Goals (Core Desktop Parity)
1. **Multi-Source Provider Management:** Support adding, editing, toggling, validating, and removing `IPTV` (M3U/M3U8/JSON), `VEGA` (VOD manifests, scraper maps, GitHub repos), and `VIDEO` (direct media URLs) providers with automatic format detection (`ProviderDetectionEngine` parity).
2. **Live TV & Category Browsing:** Parse massive M3U/JSON playlists (up to 100 MB), extract categories (`group-title`), logos, match/event metadata (`team1`, `team2`, `status`, `startTime`), filter adult/blocked content (`ContentFilter`), and support instant channel search.
3. **Vega JS/TS Extension Engine:** Execute Vega-compatible provider extensions (`catalog.ts/js`, `posts.ts/js`, `meta.ts/js`, `stream.ts/js`, `episodes.ts/js`) inside a sandboxed JS runtime with `providerContext` compatibility (`axios`, `cheerio`, `kvStore`, `getBaseUrl`, `openWebView`) and automatic multi-mirror/cross-extension fallback (`VegaStreamFallbackEngine`).
4. **Resilient Desktop Media Player:** Stream HLS (`.m3u8`), MPEG-DASH (`.mpd`), MP4/MKV/WebM (including 10-bit HEVC), and RTMP/RTMPS with multi-audio track selection, subtitle tracks, playback speed, Picture-in-Picture / Always-on-Top mini-player, keyboard shortcuts, and optional external player handoff (VLC / mpv).
5. **Offline VOD Downloads:** Concurrent download queue (max 5 concurrent tasks), pause/cancel/retry, progress & speed telemetry, persistent task recovery across restarts, and native OS Downloads/Videos folder storage.
6. **Watch History & Resume Playback:** Persist up to 100 entries with resume position (`> 90%` or `< 15s` remaining resets to `0`), search/filter, and single-entry or bulk clear.
7. **Privacy & Network Resilience (DoH):** Built-in DNS-over-HTTPS (`Cloudflare 1.1.1.1`, `Google 8.8.8.8`, `AdGuard`, `Custom DoH`, `System Off`) with hardcoded bootstrap IPs, 5-minute DNS cache, and latency tester.
8. **Appearance & Self-Update:** Dark & Pure AMOLED Black themes, configurable startup tab (`Home`, `Television`, `Movies`, `Category`), and GitHub Releases update checker/installer (`samyak2403/IPTVMine-Pro` or dedicated desktop repo).

### 2.2 Explicit Non-Goals (Out of Scope for v1)
- Mobile-only **QR Code Camera Scanner** (`QrScannerActivity`) — replaced on desktop by clipboard/file/drag-and-drop import (optional QR image decode may be added later, not camera capture).
- Android TV-only **Weather Dashboard**, **TV Speed Test**, and **Mobile-TV Local Pairing**.
- Android-specific foreground services, MediaStore APIs, and ExoPlayer/Media3 JNI wrappers.

---

## 3. Target Personas

| Persona | Needs | Primary Workflows |
| :--- | :--- | :--- |
| **Cord-Cutter / Live TV Viewer** | Fast M3U loading, channel category filtering, low-latency live stream playback, keyboard navigation | Add M3U URL → Browse Television/Categories → Play & switch channels |
| **VOD & Series Binge-Watcher** | Rich poster grids, synopsis/season/episode metadata, reliable stream mirrors, resume playback | Enable Vega extensions → Browse Movies/Search → Play with auto-fallback & resume |
| **Offline Commuter / Archivist** | Reliable multi-stream downloads to local disk, progress tracking, custom headers support | Open Movie Detail → Queue Download → Manage in Downloads view |
| **Extension Developer / Power User** | Local Vega provider testing, DoH bypass for ISP blocks, external player (VLC/mpv) launch | Configure DoH → Load local/GitHub provider → Inspect logs / open in mpv |

---

## 4. Functional Requirements

### FR-1: Onboarding & Provider Management
- **FR-1.1** On first launch with zero configured providers, route directly to `AddProvider` (matching `StartupTabPreferenceManager.resolveMobileStartRoute` behavior).
- **FR-1.2** Support `ProviderType`: `IPTV`, `VEGA`, `VIDEO`, storing `{ title, url, userAgent?, isActive, channelCount, type }`.
- **FR-1.3** Implement live quick-type detection (`detectQuick`) and async structural content inspection (`detectFromUrl`) with jsDelivr mirror fallback for GitHub raw URLs (`raw.githubusercontent.com` → `cdn.jsdelivr.net/gh/...`).
- **FR-1.4** Detect `VOD_SCRAPER_MAP` (`urls.json`, `modflix.json`, `movies.json`), `VOD_MANIFEST` (`manifest.json`), `IPTV_M3U`, `IPTV_JSON`, and `DIRECT_VIDEO`.
- **FR-1.5** Support toggling active state, editing URL/title/User-Agent, and deleting providers from `ProviderList`, triggering cache invalidation and channel refresh.

### FR-2: Live TV & M3U/JSON Channel Engine
- **FR-2.1** Parse `#EXTM3U` / `#EXTINF` attributes (`tvg-logo`, `group-title`, `tvg-name`, custom headers like `#EXTVLCOPT:http-user-agent`) and arbitrary IPTV JSON arrays (`channels`, `streams`, `matches`, `playlists`, `stations`).
- **FR-2.2** Enforce 100 MB max playlist response cap, content-type guard (reject `video/*`, `image/*`, `audio/*` binaries except `*mpegurl*`), 5-minute fetch cooldown, and per-URL exponential backoff (1 min → 1 hr cap) on failures.
- **FR-2.3** Apply `ContentFilter` to filter adult/NSFW categories and channel names by default, with optional user-managed custom blocked categories.
- **FR-2.4** Dynamically show/hide the `Television` navigation tab based on active IPTV provider presence and channel availability.

### FR-3: VOD & Vega Extension Runtime
- **FR-3.1** Fetch and cache Vega manifests and extension modules (`catalog`, `posts`, `meta`, `stream`, `episodes`) with two-tier caching (L1 in-memory LRU + L2 SQLite/disk store, MD5-keyed to avoid key collisions).
- **FR-3.2** Support installing/uninstalling individual scrapers (`ExtensionRepository` parity) and selecting the preferred active Vega provider.
- **FR-3.3** Transpile/strip basic TypeScript type annotations in extension scripts if raw `.ts` files are loaded (matching `VegaProviderRunner` TypeScript stripping), and execute them in an isolated JS runtime.
- **FR-3.4** Implement `VegaStreamFallbackEngine` 4-stage resolution:
  1. Direct chosen link (`prioritizeStreams` sorting mirrors like CF Worker, FastCloud, HubCloud, Gofile, Pixeldrain + highest quality).
  2. Other `directLinks` under the same server.
  3. Other servers (`linkList`) under the same title (including lazy `episodesLink` resolution).
  4. Cross-extension automatic search & match fallback across other installed extensions.

### FR-4: Desktop Media Player
- **FR-4.1** Play HLS, DASH, MP4, MKV (including 10-bit HEVC), WebM, AVI, FLV, and RTMP/RTMPS streams with custom HTTP headers (`User-Agent`, `Referer`, `Cookie`) passed per stream.
- **FR-4.2** Provide standard desktop transport controls: Play/Pause (`Space`/`K`), Seek ±10s (`Left`/`Right` or `J`/`L`), Volume/Mute (`Up`/`Down`/`M`), Fullscreen (`F`/`Esc`), Playback Speed (`0.5x`–`2.0x`), Aspect Ratio/Fit mode, Audio Track selector, Subtitle selector (embedded + external `.srt`/`.vtt`), and Always-on-Top Mini-Player / PiP.
- **FR-4.3** Support episode playlists with "Next Episode" / "Previous Episode" resolution via `VegaStreamResolver`.
- **FR-4.4** Provide an "Open in External Player" action (VLC, mpv, or system default) passing stream URL and headers when configured or when internal decoding fails.
- **FR-4.5** Auto-save playback progress periodically and on exit to `WatchHistoryRepository`.

### FR-5: Downloads Manager
- **FR-5.1** Queue VOD streams for local download with custom HTTP headers, capped at 5 concurrent active downloads.
- **FR-5.2** Track `id`, `title`, `downloadUrl`, `headers`, `savePath`, `progress` (0–100%), `speed`, `status` (`PENDING`, `DOWNLOADING`, `PAUSED`, `COMPLETED`, `FAILED`), `totalBytes`, `downloadedBytes`, `addedAt`.
- **FR-5.3** Resume unfinished `PENDING`/`DOWNLOADING` tasks on app restart (with HTTP `Range` support where servers allow).
- **FR-5.4** Save completed files to user-configured download directory (default: OS `Videos/IPTVMine Pro`), with actions to Play locally, Open Containing Folder, Retry, Cancel, or Delete.

### FR-6: Network, Privacy & Settings
- **FR-6.1** Provide DNS-over-HTTPS settings (`Cloudflare`, `Google`, `AdGuard`, `Custom DoH URL`, `Off`) with live connectivity + latency test (`testDns`).
- **FR-6.2** Provide Theme switcher (Dark `#121212` vs Pure AMOLED Black `#000000`), Startup Tab preference (`Home`, `Television`, `Movies`, `Category`), Cache Clear, About, Legal Disclaimers (`Privacy Policy`, `Terms & Conditions`, `Disclaimer` — resolving Android `TODO` gaps), Bug Report generator (copies diagnostic bundle / opens GitHub issue), and GitHub Release updater check.

---

## 5. Non-Functional Requirements

| Category | Requirement |
| :--- | :--- |
| **Performance** | Cold startup to interactive shell `< 1.5s`; L1 cached VOD grid switch `< 50ms`; M3U parsing of 10,000+ channels `< 500ms` in Rust. |
| **Memory** | Idle shell `< 150 MB` RAM; strict response size cap (100 MB) and bounded LRU caches (`200` post lists, `50` catalogs, `20` manifests). |
| **Build System** | **Cargo-first**: `cargo build` / `cargo tauri dev` / `cargo tauri build` orchestrate frontend build (`beforeBuildCommand` / `beforeDevCommand`) and bundle native binaries cleanly. |
| **Reliability** | Zero orphaned JS execution futures on navigation/provider switch; graceful network-drop banner & stream auto-retry; crash-safe SQLite/JSON persistence. |
| **Accessibility** | Full keyboard navigation (`Tab`, arrow keys, `Enter`, `Esc`, media keys) and ARIA-labeled controls across all views. |
