# IPTVMine Desktop — Security & Access Document

**Document Status:** Draft v1.0  
**Scope:** Threat Model, Tauri v2 Capabilities, JS Extension Sandboxing, Network/DoH Hardening, Filesystem & Process Execution Controls

---

## 1. Security Model & Threat Surface

IPTVMine Desktop processes **untrusted third-party inputs** across four primary boundaries:
1. **Remote M3U/JSON Playlists & Direct URLs** (potentially huge payloads, malformed tags, malicious URLs, or binary files disguised as playlists).
2. **Third-Party Vega JS/TS Extensions** (arbitrary JavaScript fetched from GitHub/external URLs and executed locally).
3. **Stream Playback & Downloads** (redirect chains, custom HTTP headers, arbitrary filenames, exotic codecs/containers).
4. **WebView ↔ Rust IPC Boundary** (preventing a compromised remote image/markup or XSS vector in the UI from gaining unrestricted OS filesystem or shell access).

**Core Principle:** The React WebView holds **zero direct OS privileges**. All network, filesystem, database, and subprocess actions live in Rust behind typed, validated Tauri commands.

---

## 2. Tauri v2 Capabilities & Access Control (`capabilities/`)

### 2.1 Principle of Least Privilege
- Disable Tauri `withGlobalTauri` injection if not required, or bind strictly to `tauri://localhost` / `http://localhost:<devPort>`.
- **Filesystem (`fs`) & Shell (`shell`) plugins:** Do **not** grant broad `fs:allow-read` / `fs:allow-write` or `shell:allow-execute` scopes to the WebView.
  - All persistence goes through domain-specific Rust IPC commands (`add_provider`, `enqueue_download`, `save_watch_progress`).
  - Opening the OS file picker (for importing local `.m3u` files) uses `tauri-plugin-dialog` scoped strictly to user-initiated selection, returning paths/contents to Rust.
  - Opening external URLs (Legal/Donate/GitHub links) uses an explicit Rust validator allowing only `https://` URLs.
- **Content Security Policy (CSP):**
  - Enforce strict CSP in `tauri.conf.json`:
    - `default-src 'self'`
    - `script-src 'self'` (no `'unsafe-eval'`, no remote `<script>`)
    - `img-src 'self' https: http: data: blob:` (for channel logos & VOD posters)
    - `media-src 'self' http://127.0.0.1:* blob:` (only local proxy/file playback when using WebView `<video>`)
    - `connect-src 'self' ipc: http://ipc.localhost`

---

## 3. Vega JS Extension Sandboxing (`iptvmine-vega`)

Running third-party Vega extensions (`catalog.ts`, `posts.ts`, `meta.ts`, `stream.ts`, `episodes.ts`) is the highest-risk surface in the application.

### 3.1 Isolated Runtime (QuickJS via `rquickjs`)
- Extensions **never** execute inside the main application WebView (no access to DOM, `window.__TAURI_INTERNALS__`, React state, or Tauri IPC).
- Each extension execution runs in a dedicated QuickJS `Runtime` + `Context` with:
  - **Memory cap:** `64 MB` heap limit (`runtime.set_memory_limit(...)`).
  - **Stack cap:** `1 MB` max stack size.
  - **Execution timeout / interrupt handler:** Hard `45s` timeout (and `15s` for episode/fallback probes), plus immediate cancellation via QuickJS interrupt callback when the caller cancels or switches screens—fixing `IPTVMine-Pro`'s leaked suspended coroutine issue documented in `BUGS.md`.
- **Disabled Globals:** No filesystem (`fs`), no process/env (`process`, `os`), no raw sockets, no `eval`/`Function` escalation beyond what the scraper module itself requires (or audited strictly within QuickJS).

### 3.2 Host Bridge Controls (`providerContext`)
| Bridge API | Access & Isolation Controls |
| :--- | :--- |
| `axios` (`httpFetch`) | Only `http://` and `https://` schemes allowed. Enforces 30s connect / 45s read timeouts, max response size (`16 MB` for HTML/JSON scraper responses), and blocks `file://`, `ftp://`, `javascript:`, `gopher://`. |
| **SSRF Protection** | Blocks requests from extensions to loopback (`127.0.0.0/8`, `::1`), link-local/cloud metadata (`169.254.0.0/16`), and RFC1918 private LAN IPs (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`) **unless** the user explicitly enables **Developer Test Mode** for a local Vega dev server (`http://<local-ip>:3001` per `VEGA_DOCS.md`). |
| `kvStore` | Namespaced in SQLite by `(provider_url, scraper_value, key)`. Extension A cannot read or overwrite Extension B's keys. Total storage per extension capped (e.g., `1 MB` / `500` keys). |
| `getBaseUrl` | Read-only lookup against discovered endpoints map; cannot mutate global app providers. |
| `openWebView` | Runs in a **separate, ephemeral, non-privileged** Tauri window with **Tauri IPC disabled** (`no-ipc` capability), returning only `{ data, cookies, userAgent, url }` with a strict timeout. |

---

## 4. Network, TLS & DoH Security (`iptvmine-net`)

- **TLS Verification:** Always enforce certificate validation (`danger_accept_invalid_certs(false)`). Never bypass TLS checks globally.
- **Bootstrap DoH Pinning:** Hardcoded bootstrap IP addresses for `cloudflare-dns.com`, `one.one.one.one`, `dns.google`, and `dns.adguard-dns.com` prevent ISP DNS hijacking from redirecting DoH queries.
- **Custom DoH Validation:** Custom resolver URLs must use `https://` (rejecting plaintext `http://` unless explicitly marked local/dev) and must validate JSON `Answer` records (`type == 1` or `type == 28`) as valid IPv4/IPv6 addresses before caching.
- **No Circular DNS Loops:** Bootstrap hosts and IP literals bypass DoH resolution (`is_bootstrap_or_ip`), mirroring `DnsPreferenceManager.kt`.
- **Credential & Header Redaction:** Custom `User-Agent`, `Cookie`, and `Referer` headers stored for streams or providers must be **redacted** (`[REDACTED]`) before writing logs or generating a Bug Report diagnostics bundle.

---

## 5. Filesystem, M3U Parsing & Download Safety

### 5.1 M3U & Playlist Guardrails (from `ChannelsProvider.kt`)
- **Size Cap:** Stream-read responses and abort immediately if size exceeds **100 MB** (`MAX_RESPONSE_SIZE`).
- **Content-Type Guard:** Reject binary `video/*`, `image/*`, `audio/*` responses (except `*mpegurl*` and `application/octet-stream`) so misconfigured video URLs cannot exhaust RAM.
- **Sanitization:** Strip Private Use Area (PUA) Unicode code points (`U+E000..U+F8FF`, `U+F0000..U+FFFFD`, `U+100000..U+10FFFD`) and ISO control characters from channel/movie titles (porting `WatchHistoryRepository.cleanTitle` and `TitleUtils`).

### 5.2 Download Path & Filename Sanitization (from `DownloadManager.kt`)
- Strip forbidden filename characters (`\ / : * ? " < > |`, control chars, `.` / `..` segments) from `title`.
- Restrict file extensions to an allowlisted media set (`mp4`, `mkv`, `webm`, `ts`, `m4v`, `avi`, `mov`, `mp3`, `aac`), defaulting to `mp4`.
- Canonicalize the destination path and verify `dest.starts_with(configured_download_dir)` before opening the file handle—preventing zip-slip / path-traversal writes.

---

## 6. Subprocess & External Player Safety

When launching an external player (`VLC`, `mpv`, or OS handler):
- **Never invoke a shell** (`cmd /c`, `powershell`, `sh -c`). Always spawn directly via `std::process::Command::new(executable_path)` with discrete `.arg(...)` entries.
- **Executable Validation:** Allow user-configured external player paths only if verified on disk, or resolve known binaries (`vlc`, `mpv`) via `which`.
- **URL & Header Validation:** Only pass URLs matching allowed schemes (`http://`, `https://`, `rtmp://`, `rtmps://`, or canonicalized local downloaded file paths). Reject option-injection strings starting with `-` as URL arguments (or pass `--` end-of-options marker before the URL).

---

## 7. Content Access & Privacy Controls

- **Content Filtering (`ContentFilter`):** Enabled by default to block adult/NSFW categories (`adult`, `xxx`, `18+`, `porn`, `erotic`, `nsfw`, etc.) and explicit channel keywords, with optional user-managed custom blocked categories.
- **Local-First Privacy:** Watch history, providers, extensions, and download logs remain 100% on the user's machine (`iptvmine.db`). No analytics SDKs, crash telemetry uploaders, or Firebase/external tracking are included.
- **Legal & Compliance:** Implement dedicated `Privacy Policy`, `Terms & Conditions`, and `Disclaimer` views (resolving the `TODO` in Android `SettingsScreen.kt`).
