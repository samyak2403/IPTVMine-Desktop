# IPTVMine Desktop — Feature Ticket List

**Document Status:** Draft v1.0  
**Build Workflow:** Cargo-first (`cargo test --workspace`, `cargo tauri dev`, `cargo tauri build`)  
**Scope:** Core Desktop Parity with `IPTVMine-Pro` (excluding Mobile QR Camera, TV Weather, TV Speed Test, Mobile-TV Pairing)

---

## Milestone Summary

| Milestone | Focus | Tickets |
| :--- | :--- | :--- |
| **M0** | Cargo Workspace, Tauri v2 & React/TS/Vite Scaffold | `DESK-001` – `DESK-002` |
| **M1** | Domain Models, SQLite Storage, L1/L2 Cache & DoH Networking | `DESK-003` – `DESK-005` |
| **M2** | Provider Detection, M3U/JSON Parser & Live TV UI | `DESK-006` – `DESK-008` |
| **M3** | Vega JS/TS Extension Runtime, Fallback Engine & VOD UI | `DESK-009` – `DESK-013` |
| **M4** | Desktop Media Player, Watch History & External Player | `DESK-014` – `DESK-016` |
| **M5** | Concurrent Downloads, Settings, Updater & Diagnostics | `DESK-017` – `DESK-020` |
| **M6** | Security Hardening, E2E Verification & Release Bundling | `DESK-021` – `DESK-022` |

---

## M0: Cargo Workspace, Tauri v2 & React/TS/Vite Scaffold

### `DESK-001`: Initialize Cargo Workspace & Tauri v2 Shell
- **Layer:** Root `Cargo.toml`, `src-tauri/`, `crates/*`
- **Description:** Create the Cargo workspace with modular crates (`iptvmine-core`, `iptvmine-net`, `iptvmine-m3u`, `iptvmine-vega`, `iptvmine-storage`, `iptvmine-downloader`, `iptvmine-updater`) and configure `src-tauri` with Tauri v2, strict CSP, and `capabilities/default.json`.
- **Verification / Done:** `cargo check --workspace` and `cargo test --workspace` succeed with 0 errors.

### `DESK-002`: Initialize React + TypeScript + Vite Frontend & Design Tokens
- **Layer:** `ui/`, `tauri.conf.json`
- **Reference:** `IPTVMine-Pro/app/.../ui/theme/Color.kt`, `MainActivity.kt`
- **Description:** Set up `ui/` with React, TypeScript, Vite, CSS variables for Standard Dark (`#121212`) and Pure AMOLED Black (`#000000`) themes, Left Navigation Rail shell, and `tauri.conf.json` `beforeDevCommand`/`beforeBuildCommand`.
- **Verification / Done:** `cargo tauri dev` builds `ui/` and opens the desktop shell window with theme switching and keyboard-accessible rail navigation.

---

## M1: Domain Models, SQLite Storage, L1/L2 Cache & DoH Networking

### `DESK-003`: Port Core Domain Models, Title Sanitizer & ContentFilter
- **Layer:** `crates/iptvmine-core`
- **Reference:** `Channel.kt`, `Provider.kt`, `VegaModels.kt`, `WatchHistoryEntry.kt`, `ContentFilter.kt`, `StreamUrlUtils.kt`
- **Description:** Implement Serde-serializable structs (`Channel`, `Provider`, `ProviderType`, `VegaProvider`, `VegaCatalog`, `VegaPost`, `VegaMeta`, `VegaLink`, `VegaDirectLink`, `VegaStream`, `WatchHistoryEntry`, `DownloadTask`), `prioritize_streams`, `is_playable_media_url`, PUA/control-char `clean_title`, and `ContentFilter`.
- **Verification / Done:** Unit tests verify `prioritize_streams` ranking, `is_playable_media_url`, PUA title stripping, and adult keyword/category blocking.

### `DESK-004`: Implement SQLite Persistence & Two-Tier Cache (`iptvmine-storage`)
- **Layer:** `crates/iptvmine-storage`
- **Reference:** `ProviderRepository.kt`, `ExtensionRepository.kt`, `MovieCacheStore.kt`, `WatchHistoryRepository.kt`
- **Description:** Create SQLite WAL schema and repository methods for providers, installed extensions, watch history (100-entry cap, `>90%` / `<15s` reset rule), downloads, per-extension `kvStore`, settings, and MD5-keyed L1 LRU (`200`/`50`/`20`) + L2 SQLite VOD cache (truncating cached post lists at 80 before L1/L2 write per `BUG-014`).
- **Verification / Done:** Integration tests verify CRUD, cache hit consistency between L1 and L2, and 100-entry history eviction.

### `DESK-005`: Implement DNS-over-HTTPS Resolver & HTTP Client (`iptvmine-net`)
- **Layer:** `crates/iptvmine-net`, `src-tauri` commands
- **Reference:** `DnsPreferenceManager.kt`
- **Description:** Implement `DnsMode` (`Cloudflare`, `Google`, `AdGuard`, `Custom`, `Off`), hardcoded bootstrap IP resolver to avoid circular deadlocks, `application/dns-json` DoH client with 5-minute TTL `DashMap` cache, `test_dns` latency check, persistent cookie jar, and jsDelivr GitHub mirror helper (`to_jsdelivr_mirror`).
- **Verification / Done:** Unit tests verify DoH JSON parsing, bootstrap bypass logic, and `test_dns` returns `(true, latency_ms)` / `(false, -1)` for empty custom URL.

---

## M2: Provider Detection, M3U/JSON Parser & Live TV UI

### `DESK-006`: Port ProviderDetectionEngine (`iptvmine-m3u` / `iptvmine-net`)
- **Layer:** `crates/iptvmine-m3u`, `src-tauri`
- **Reference:** `ProviderDetectionEngine.kt`
- **Description:** Implement `detect_quick` and async `inspect_provider_url` supporting `.m3u`/`.m3u8`, IPTV JSON (`channels`, `streams`, `matches`, `playlists`, `stations`), Vega `manifest.json`, `urls.json`/`modflix.json`/`movies.json` scraper maps, direct video URLs, and GitHub repo root resolution.
- **Verification / Done:** Unit tests cover all 5 `SourceKind` classifications against fixture payloads.

### `DESK-007`: Port Streaming M3U/JSON Channel Engine & Backoff (`iptvmine-m3u`)
- **Layer:** `crates/iptvmine-m3u`
- **Reference:** `ChannelsProvider.kt`
- **Description:** Implement streaming `#EXTM3U`/`#EXTINF` and IPTV JSON parser with `100 MB` response limit, `is_blocked_content_type` binary guard, `ContentFilter` integration, 5-minute `load_if_needed` cooldown, and per-URL exponential backoff (`60s` → `3600s`) cleared on explicit refresh.
- **Verification / Done:** Parser benchmarks parse 10,000+ channel M3U fixture in `< 500ms`; tests verify 100 MB cap, `video/mp4` rejection, and backoff timing.

### `DESK-008`: Build Provider Management & Live TV Frontend Screens
- **Layer:** `ui/`
- **Reference:** `ProviderListScreen.kt`, `AddProviderScreen.kt`, `AddProviderHelpScreen.kt`, `TelevisionScreen.kt`, `CategoryScreen.kt`, `CategoryDetailScreen.kt`
- **Description:** Implement `ProviderList`, `AddProvider` (with live `detect_quick` badge, URL inspection preview, and local `.m3u` file import), `AddProviderHelp`, conditional `Television` rail tab, Live TV channel grid with category chips + instant search (`Ctrl+K`), and `Category` / `CategoryDetail` views.
- **Verification / Done:** Adding an M3U URL or local `.m3u` file populates `Television` and `Category` views and persists across app restarts.

---

## M3: Vega JS/TS Extension Runtime, Fallback Engine & VOD UI

### `DESK-009`: Implement QuickJS Vega Runtime & `providerContext` Bridge (`iptvmine-vega`)
- **Layer:** `crates/iptvmine-vega`
- **Reference:** `VegaProviderRunner.kt`, `VEGA_DOCS.md`
- **Description:** Embed `rquickjs` with 64 MB heap limit, TS-to-JS annotation/ESM stripper, interruptible 45s execution timeout (cancelling immediately on drop), and `providerContext` (`axios` with `maxRedirects: 0` + `params`, `cheerio.load` DOM/CSS query bridge, scoped `kvStore`, `getBaseUrl`, `commonHeaders`).
- **Verification / Done:** Integration tests execute sample Vega `catalog`, `posts`, `meta`, `episodes`, and `stream` modules and verify cancellation aborts immediately without waiting 45s.

### `DESK-010`: Add Optional Challenge `openWebView` Window Bridge
- **Layer:** `src-tauri`, `crates/iptvmine-vega`
- **Reference:** `VegaProviderRunner.kt` (`openWebView`)
- **Description:** Support `providerContext.openWebView(url, options)` via a hidden, non-IPC Tauri auxiliary window when a scraper requires browser JS challenge clearance, syncing resulting cookies back into `iptvmine-net` cookie jar.
- **Verification / Done:** Calling `openWebView` on a test page returns `{ data, cookies, userAgent, url }` and isolates the auxiliary window from app Tauri IPC.

### `DESK-011`: Implement `VegaStreamFallbackEngine` (`iptvmine-vega`)
- **Layer:** `crates/iptvmine-vega`
- **Reference:** `VegaStreamFallbackEngine.kt`, `VegaStreamResolver.kt`
- **Description:** Port 4-stage resolution (direct link → sibling `directLinks` → alternate servers & `episodesLink` → cross-extension search fallback across installed extensions) emitting `vega://fallback_status` progress events to the UI.
- **Verification / Done:** Unit/mock tests verify automatic progression across all 4 fallback stages and early stop on first playable stream.

### `DESK-012`: Build Extensions Manager & VOD Browse/Search UI
- **Layer:** `ui/`
- **Reference:** `ExtensionsScreen.kt`, `MoviesScreen.kt`, `CategoryMoviesScreen.kt`, `MovieSearchScreen.kt`
- **Description:** Implement `ExtensionsScreen` (install/uninstall scrapers, preferred Vega provider), `MoviesScreen` (scraper switcher, horizontal catalog rails, shimmer skeletons), `CategoryMoviesScreen` (paginated grid), and `MovieSearchScreen` (debounced search).
- **Verification / Done:** Switching scrapers or categories loads cached data immediately (`L1/L2`) and fetches paginated posts cleanly.

### `DESK-013`: Build `MovieDetailScreen` & Stream Resolution UX
- **Layer:** `ui/`
- **Reference:** `MovieDetailScreen.kt`
- **Description:** Implement hero backdrop/poster, synopsis, IMDb badge, server/season selector, episode/mirror list, **Play** with live fallback status indicator, **Download** trigger, and Watch History resume progress bar.
- **Verification / Done:** Selecting an episode triggers `resolve_stream_with_fallback`, displays live fallback status, and opens Player or queues Download.

---

## M4: Desktop Media Player, Watch History & External Player

### `DESK-014`: Implement Desktop Media Player Backend & Local Stream Proxy
- **Layer:** `src-tauri` / `crates/iptvmine-media`
- **Reference:** `PlayerActivity.kt`, `RtmpAwareDataSource.kt`
- **Description:** Implement primary playback controller:
  - **Recommended:** Embedded `libmpv` (or managed `mpv` IPC sidecar) supporting HLS, DASH, MP4, MKV, 10-bit HEVC, RTMP/RTMPS, custom HTTP headers, multi-audio, subtitles, and playback speed.
  - **Fallback/WebView mode:** Localhost HTTP header-injecting proxy + `hls.js` for standard HLS/MP4 streams.
  - Also implement `launch_external_player` (direct `Command::new` argv spawn for VLC / mpv with `--http-header-fields` / `--user-agent`, zero shell interpolation).
- **Verification / Done:** Plays HLS (with custom `Referer`/`User-Agent`), MP4, and 10-bit HEVC MKV test streams; `launch_external_player` opens VLC/mpv cleanly.

### `DESK-015`: Build `PlayerScreen` UI, Shortcuts & Error Recovery Modal
- **Layer:** `ui/`
- **Reference:** `PlayerScreen.kt`, `PlayerActivity.kt`, `BUGS.md` (`BUG-002`)
- **Description:** Build player viewport with auto-hiding HUD (`3s`), transport controls, seekbar, audio/subtitle/speed/aspect popovers, Next/Prev Episode controls (`VegaStreamResolver`), Always-on-Top Mini-Player toggle, keyboard shortcuts (`Space/K`, `Left/Right`, `J/L`, `Up/Down`, `M`, `F`, `P`, `N`), and decode/network error modal offering `Retry`, `Next Mirror`, or `Open in External Player`.
- **Verification / Done:** All keyboard shortcuts, track switches, Next Episode resolution, and external-player recovery actions work as specified.

### `DESK-016`: Build `WatchHistoryScreen` & Home "Continue Watching"
- **Layer:** `ui/`
- **Reference:** `WatchHistoryScreen.kt`, `HomeScreen.kt`
- **Description:** Wire periodic playback progress saves (`save_watch_progress`), `HomeScreen` Continue Watching rail + active provider cards, and `WatchHistoryScreen` with progress bars, resume click, delete entry, and Clear All confirmation.
- **Verification / Done:** Closing player mid-stream saves position; reopening resumes at saved timestamp (or `0` if `>90%` / `<15s` remaining).

---

## M5: Concurrent Downloads, Settings, Updater & Diagnostics

### `DESK-017`: Implement Concurrent Download Manager (`iptvmine-downloader`)
- **Layer:** `crates/iptvmine-downloader`, `src-tauri`
- **Reference:** `DownloadManager.kt`, `DownloadTask.kt`
- **Description:** Implement Tokio download worker pool (`Semaphore(5)`), sanitized filenames, directory confinement, HTTP `Range` resume, startup auto-resume of `PENDING`/`DOWNLOADING` tasks, and throttled `downloads://progress` IPC events.
- **Verification / Done:** Tests verify max-5 concurrency, path traversal rejection, startup recovery, and cancel/delete cleanup.

### `DESK-018`: Build `DownloadsScreen` & Offline Playback Integration
- **Layer:** `ui/`
- **Reference:** `DownloadsScreen.kt`, `VideoListScreen.kt`
- **Description:** Build Downloads queue UI with progress bars, speed (`MB/s`), Pause/Resume/Cancel/Retry/Delete, Open Folder, and Play Local File actions; add non-blocking offline banner when network is disconnected.
- **Verification / Done:** Disconnecting network shows offline banner while completed local downloads remain playable.

### `DESK-019`: Build Settings, DNS Settings, Legal, About & Bug Report Screens
- **Layer:** `ui/`, `src-tauri`
- **Reference:** `SettingsScreen.kt`, `DnsSettingsScreen.kt`, `AboutScreen.kt`, `BugReportScreen.kt`, `LegalDocumentScreen.kt`, `SupportScreen.kt`, `DonateScreen.kt`
- **Description:** Implement all Settings sub-screens including live DoH latency tester, Theme & Startup Tab preferences, complete `Disclaimer` / `Privacy Policy` / `Terms & Conditions` tabs (fixing Android `TODO`s), and redacted `BugReport` diagnostics exporter.
- **Verification / Done:** DNS latency test updates UI per mode; diagnostic export contains zero raw cookies/headers/URLs.

### `DESK-020`: Implement GitHub Release Updater (`iptvmine-updater`)
- **Layer:** `crates/iptvmine-updater`, `ui/`
- **Reference:** `GithubReleaseService.kt`, `GithubUpdater.kt`
- **Description:** Query GitHub Releases API (`releases/latest`), compare SemVer against `CARGO_PKG_VERSION`, match OS installer asset (`.msi`/`.exe`, `.dmg`, `.AppImage`/`.deb`), and display release notes + download/update prompt in `About` / startup check.
- **Verification / Done:** Unit tests parse release JSON, compare SemVer tags (`v1.2.0` > `1.1.9`), and select the correct OS asset.

---

## M6: Security Hardening, E2E Verification & Release Bundling

### `DESK-021`: Security & Sandboxing Audit
- **Layer:** Workspace-wide
- **Description:** Verify Tauri v2 capability scopes, CSP headers, QuickJS heap/timeout/SSRF rules (including Developer Test Mode toggle), download path confinement, and external player argv sanitization against `docs/SECURITY_AND_ACCESS.md`.
- **Verification / Done:** Security checklist passes; automated tests confirm SSRF, path traversal, and `file://` URL rejection.

### `DESK-022`: Cross-Platform Cargo Build & Packaging Verification
- **Layer:** `Cargo.toml`, `tauri.conf.json`, CI/build scripts
- **Description:** Verify clean `cargo test --workspace`, `ui` TypeScript typecheck (`tsc --noEmit`), and `cargo tauri build` producing runnable desktop installer artifacts.
- **Verification / Done:** `cargo test --workspace`, `npm --prefix ui run build`, and `cargo tauri build` exit with code `0`.
