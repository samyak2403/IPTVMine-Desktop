# IPTVMine Desktop — Frontend Specification Document

**Document Status:** Draft v1.0  
**Frontend Stack:** React 18+ / TypeScript / Vite (bundled by Cargo via `tauri.conf.json`)  
**UI Paradigm:** Desktop-first Widescreen Shell (Left Navigation Rail + Top Context Bar + Content Canvas + Dedicated Player View)

---

## 1. Visual Design System & Tokens

Ported directly from `IPTVMine-Pro` (`ui/theme/Color.kt`, `Type.kt`, `ThemePreferenceManager.kt`) and adapted for desktop mouse, keyboard, and 1080p/4K displays:

### 1.1 Color Palettes

| Token | Standard Dark Theme (Default) | Pure AMOLED Black Theme | Usage |
| :--- | :--- | :--- | :--- |
| `--bg-canvas` | `#121212` | `#000000` | Main window & route canvas |
| `--bg-surface` | `#1E1E1E` | `#0A0A0A` | Sidebar rail, cards, top bar |
| `--bg-surface-elevated` | `#2D2D2D` | `#161616` | Inputs, modals, hover states, popovers |
| `--border-subtle` | `#2C2C2C` | `#242424` | Card outlines, dividers |
| `--text-primary` | `#FFFFFF` | `#F3F3F3` | Headings, channel/movie titles |
| `--text-secondary` | `#CCCCCC` | `#A5A5A5` | Subtitles, metadata, hints |
| `--accent-primary` | `#26A69A` | `#26A69A` | Active rail indicator, primary toggles |
| `--accent-secondary` | `#03DAC5` | `#03DAC5` | Badges, progress bars, highlights |
| `--cta-violet` | `#6200EA` | `#6200EA` | Primary action buttons (`Play`, `Save`) |
| `--cta-violet-hover` | `#3700B3` | `#3700B3` | Hover/pressed state for violet CTA |
| `--focus-ring` | `#448AFF` | `#448AFF` | Keyboard focus ring (`2px solid`) |
| `--status-live` | `#64DD17` | `#64DD17` | Live match / connected indicator |
| `--status-error` | `#F44336` | `#F44336` | Stream error / failed download |
| `--status-warn` | `#FFD600` | `#FFD600` | Fallback notice / warning badge |

### 1.2 Typography & Spacing
- **Font Family:** System UI stack (`Inter`, `Segoe UI Variable`, `SF Pro Text`, `system-ui, sans-serif`); Monospace (`JetBrains Mono`, `Consolas`, monospace) for URLs, DoH endpoints, and logs.
- **Scale:**
  - Page Title: `24px / 700`
  - Section Header: `18px / 600`
  - Card Title: `14px / 600` (clamped to 2 lines)
  - Body / Metadata: `14px / 400`
  - Micro Badge / Chip: `11px / 600`
- **Grid Spacing:** `16px`–`24px` page padding, `16px` card grid gaps, responsive auto-fill columns (`minmax(160px, 1fr)` for 2:3 VOD posters; `minmax(240px, 1fr)` for 16:9 Live TV cards).

---

## 2. Application Shell & Navigation Architecture

### 2.1 Desktop Window Shell
Unlike Android's bottom navigation bar, desktop uses a persistent **Left Navigation Rail** (`72px` collapsed / `220px` expanded, collapsible) + **Contextual Top Bar** (`56px`):
- **Primary Rail Items** (matching `MainActivity.kt` `Screen` items):
  1. `Home` (`/home`)
  2. `Television` (`/television`) — **conditionally shown** when an active `IPTV` provider has channels (matching `StartupTabPreferenceManager.shouldShowTelevisionTab`).
  3. `Movies` (`/movies`)
  4. `Category` (`/category`)
  5. `Downloads` (`/downloads`) — promoted to quick-access badge in rail footer (with active download count) + Settings.
  6. `Settings` (`/settings`)
- **Offline Handling:** Instead of Android's full-screen blocking `NoInternetScreen`, desktop shows a **non-blocking top status banner** (`Offline — Live streams unavailable; Local Downloads remain playable`) so users can still watch offline downloads or inspect settings.

### 2.2 Complete Route Map (Desktop Parity)

| Route | View Name | Purpose & Key Elements |
| :--- | :--- | :--- |
| `/home` | `HomeScreen` | Continue Watching horizontal row (`WatchHistory`), Active Provider status summary, Quick Live TV / Featured VOD rails, Empty State CTA → `/providers/add`. |
| `/television` | `TelevisionScreen` | Live TV channel grid with sticky category chips, instant search input (`Ctrl+F`), provider filter, and channel cards (`logo`, `name`, `category`, `team1 vs team2`, `LIVE` badge). |
| `/movies` | `MoviesScreen` | Active Vega scraper selector dropdown/chips, catalog category horizontal rails (or infinite-scroll grid), shimmer skeletons, quick link to `/movies/search`. |
| `/movies/category` | `CategoryMoviesScreen` | Paginated grid for a specific Vega catalog filter (`categoryName`, `categoryFilter`, `providerUrl`, `scraperValue`). |
| `/movies/search` | `MovieSearchScreen` | Cross-catalog / selected-scraper search with debounced query, pagination, and recent queries. |
| `/movies/detail` | `MovieDetailScreen` | Backdrop + poster hero, synopsis, IMDb ID badge, server/season selector tabs, episode/mirror list, **Play** (with real-time `VegaStreamFallbackEngine` status toast/pill), **Download** button, and Resume progress indicator. |
| `/category` | `CategoryScreen` | IPTV category index cards with channel counts. |
| `/category/:name` | `CategoryDetailScreen` | Filtered channels within a selected IPTV category. |
| `/player` | `PlayerScreen` | Full-viewport video canvas, auto-hiding HUD overlay, transport controls, seekbar, audio/subtitle/speed/aspect popovers, Next/Prev Episode controls, Mini-Player / Always-on-Top toggle, External Player fallback modal. |
| `/providers` | `ProviderListScreen` | List of configured `IPTV` / `VEGA` / `VIDEO` providers with active toggle, channel count, edit modal, delete confirmation, and `+ Add Provider` button. |
| `/providers/add` | `AddProviderScreen` | Title, URL, optional User-Agent, live quick-detection pill (`IPTV` / `VEGA` / `VIDEO`), "Inspect URL" async preview (`VodManifest`, `IptvM3u`, etc.), local `.m3u` file picker, and Help link (`/providers/help`). |
| `/providers/help` | `AddProviderHelpScreen` | Guide for adding M3U playlists, Vega manifests, and GitHub raw URLs. |
| `/settings` | `SettingsScreen` | Grouped settings: Content (`Providers`, `Extensions`, `Downloads`, `Watch History`), Appearance (`AMOLED Black Theme`, `Startup Tab`), Network (`DNS over HTTPS`), Updates & Diagnostics (`Check Updates`, `Report Bug`, `About`, `Legal`). |
| `/settings/extensions` | `ExtensionsScreen` | Installed vs Available Vega scrapers from active Vega manifest, install/uninstall toggles, preferred scraper selection. |
| `/settings/downloads` | `DownloadsScreen` | Active/Completed/Failed download queue with progress bars, speed (`MB/s`), Pause/Resume/Cancel/Retry/Delete/Open Folder/Play buttons. |
| `/settings/history` | `WatchHistoryScreen` | Watch history cards with progress bar, timestamp, Resume click, individual delete, and "Clear All" confirmation. |
| `/settings/dns` | `DnsSettingsScreen` | Radio cards for `Cloudflare`, `Google`, `AdGuard`, `Custom DoH`, `Off`, custom URL input, and "Test Latency" button showing `OK (42 ms)` / `Failed`. |
| `/settings/about` | `AboutScreen` | App version, Rust/Tauri runtime info, GitHub repo link, "Check for Updates" trigger. |
| `/settings/bug-report` | `BugReportScreen` | Generates sanitized diagnostic summary (OS, app version, DNS mode, provider counts—redacting URLs/cookies) with "Copy to Clipboard" and "Open GitHub Issue". |
| `/settings/legal` | `LegalDocumentScreen` | Tabbed viewer for `Disclaimer`, `Privacy Policy`, and `Terms & Conditions`. |

---

## 3. Interaction, Keyboard & Player UX

### 3.1 Global & List Keyboard Shortcuts
- `Ctrl+K` / `/`: Focus search on active view (`Television` or `MovieSearch`).
- `Ctrl+1..5`: Jump between primary rail tabs (`Home`, `Television`, `Movies`, `Category`, `Settings`).
- `Esc`: Close open modal/popover or exit fullscreen player.
- Focus rings (`--focus-ring: #448AFF`) visible on keyboard navigation (`:focus-visible`).

### 3.2 Media Player Controls & Shortcuts
- **HUD Auto-Hide:** Controls fade out after `3s` of mouse inactivity during playback; reappear on mouse move or key press.
- **Shortcuts:**
  - `Space` / `K`: Play / Pause
  - `Left` / `Right`: Seek `-10s` / `+10s`
  - `J` / `L`: Seek `-30s` / `+30s`
  - `Up` / `Down`: Volume `+5%` / `-5%`
  - `M`: Mute / Unmute
  - `F` / Double-click: Toggle Fullscreen
  - `P`: Toggle Always-on-Top Mini-Player
  - `N`: Next Episode (when episode playlist is present)
- **Error Recovery Modal:** If stream decoding or network fails after fallback attempts, present a clear dialog with:
  1. `Retry Stream`
  2. `Try Next Mirror` (if alternatives exist)
  3. `Open in External Player (VLC / mpv)`
  4. `Close Player`

---

## 4. State Management & Data Flow

- **Server State (`TanStack Query` or typed hook cache):** Wraps Tauri `invoke` calls (`providers`, `channels`, `catalog`, `posts`, `meta`) with stale-while-revalidate semantics matching Rust's L1/L2 cache.
- **Client UI State (`Zustand`):** Theme (`dark` / `amoled`), sidebar collapsed state, active player overlay state, and live download progress map fed by `listen('downloads://progress')`.
- **Resilience:** Every async view renders deterministic **Shimmer Skeletons** during initial load, **Empty States** with actionable CTAs when no providers/extensions exist, and **Inline Retry Banners** on transient errors.
