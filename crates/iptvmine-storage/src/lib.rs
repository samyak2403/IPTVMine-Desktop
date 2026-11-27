use iptvmine_core::{
    clean_title, AppSettings, DownloadStatus, DownloadTask, Provider, VegaCatalog, VegaPost,
    VegaProvider, WatchHistoryEntry,
};
use lru::LruCache;
use md5::{Digest, Md5};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("storage lock poisoned")]
    Lock,
}

pub type Result<T> = std::result::Result<T, StorageError>;

pub struct Storage {
    conn: Mutex<Connection>,
    mem_posts: Mutex<LruCache<String, Vec<VegaPost>>>,
    mem_catalogs: Mutex<LruCache<String, Vec<VegaCatalog>>>,
    mem_scrapers: Mutex<LruCache<String, Vec<VegaProvider>>>,
}

impl Storage {
    pub fn open<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        if let Some(parent) = db_path.as_ref().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS providers (
                url TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                user_agent TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                channel_count INTEGER NOT NULL DEFAULT 0,
                provider_type TEXT NOT NULL DEFAULT 'IPTV',
                sort_order INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS installed_extensions (
                value TEXT PRIMARY KEY
            );
            CREATE TABLE IF NOT EXISTS watch_history (
                link TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                stream_url TEXT NOT NULL,
                image_url TEXT NOT NULL,
                provider_url TEXT NOT NULL,
                scraper_value TEXT NOT NULL,
                position_ms INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                last_watched_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS downloads (
                id TEXT PRIMARY KEY,
                payload_json TEXT NOT NULL,
                added_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS extension_kv (
                provider_url TEXT NOT NULL,
                scraper_value TEXT NOT NULL,
                k TEXT NOT NULL,
                v TEXT NOT NULL,
                PRIMARY KEY (provider_url, scraper_value, k)
            );
            CREATE TABLE IF NOT EXISTS cache_entries (
                cache_key TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                k TEXT PRIMARY KEY,
                v TEXT NOT NULL
            );
            ",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            mem_posts: Mutex::new(LruCache::new(NonZeroUsize::new(200).unwrap())),
            mem_catalogs: Mutex::new(LruCache::new(NonZeroUsize::new(50).unwrap())),
            mem_scrapers: Mutex::new(LruCache::new(NonZeroUsize::new(20).unwrap())),
        })
    }

    // ── Providers ─────────────────────────────────────────────────────────────

    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT title, url, user_agent, is_active, channel_count, provider_type
             FROM providers ORDER BY sort_order ASC, rowid ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let type_str: String = row.get(5)?;
            let r#type = serde_json::from_str(&format!("\"{}\"", type_str)).unwrap_or_default();
            Ok(Provider {
                title: row.get(0)?,
                url: row.get(1)?,
                user_agent: row.get(2)?,
                is_active: row.get::<_, i64>(3)? != 0,
                channel_count: row.get::<_, i64>(4)?.max(0) as usize,
                r#type,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn upsert_provider(&self, provider: &Provider) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let type_str = serde_json::to_string(&provider.r#type)?
            .trim_matches('"')
            .to_string();
        conn.execute(
            "INSERT INTO providers (url, title, user_agent, is_active, channel_count, provider_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(url) DO UPDATE SET
                title = excluded.title,
                user_agent = excluded.user_agent,
                is_active = excluded.is_active,
                channel_count = excluded.channel_count,
                provider_type = excluded.provider_type",
            params![
                provider.url,
                provider.title,
                provider.user_agent,
                if provider.is_active { 1 } else { 0 },
                provider.channel_count as i64,
                type_str,
            ],
        )?;
        Ok(())
    }

    pub fn replace_provider_url(&self, old_url: &str, new_provider: &Provider) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM providers WHERE url = ?1", params![old_url])?;
        let type_str = serde_json::to_string(&new_provider.r#type)?
            .trim_matches('"')
            .to_string();
        tx.execute(
            "INSERT OR REPLACE INTO providers (url, title, user_agent, is_active, channel_count, provider_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                new_provider.url,
                new_provider.title,
                new_provider.user_agent,
                if new_provider.is_active { 1 } else { 0 },
                new_provider.channel_count as i64,
                type_str,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_provider(&self, url: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute("DELETE FROM providers WHERE url = ?1", params![url])?;
        Ok(())
    }

    // ── Extensions ────────────────────────────────────────────────────────────

    pub fn get_installed_extensions(&self) -> Result<HashSet<String>> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let mut stmt = conn.prepare("SELECT value FROM installed_extensions")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = HashSet::new();
        for row in rows {
            out.insert(row?);
        }
        Ok(out)
    }

    pub fn set_extension_installed(&self, value: &str, installed: bool) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        if installed {
            conn.execute(
                "INSERT OR IGNORE INTO installed_extensions (value) VALUES (?1)",
                params![value],
            )?;
        } else {
            conn.execute(
                "DELETE FROM installed_extensions WHERE value = ?1",
                params![value],
            )?;
        }
        Ok(())
    }

    // ── Watch History (100-entry cap + >90% / <15s reset rule) ────────────────

    pub fn get_watch_history(&self) -> Result<Vec<WatchHistoryEntry>> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT title, link, stream_url, image_url, provider_url, scraper_value,
                    position_ms, duration_ms, last_watched_ms
             FROM watch_history ORDER BY last_watched_ms DESC LIMIT 100",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WatchHistoryEntry {
                title: clean_title(&row.get::<_, String>(0)?),
                link: row.get(1)?,
                stream_url: row.get(2)?,
                image_url: row.get(3)?,
                provider_url: row.get(4)?,
                scraper_value: row.get(5)?,
                position_ms: row.get(6)?,
                duration_ms: row.get(7)?,
                last_watched_ms: row.get(8)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn save_watch_progress(&self, entry: &WatchHistoryEntry) -> Result<()> {
        if entry.link.trim().is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO watch_history
             (link, title, stream_url, image_url, provider_url, scraper_value, position_ms, duration_ms, last_watched_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                entry.link,
                clean_title(&entry.title),
                entry.stream_url,
                entry.image_url,
                entry.provider_url,
                entry.scraper_value,
                entry.position_ms,
                entry.duration_ms,
                entry.last_watched_ms,
            ],
        )?;
        tx.execute(
            "DELETE FROM watch_history WHERE link NOT IN (
                SELECT link FROM watch_history ORDER BY last_watched_ms DESC LIMIT 100
             )",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_resume_position_ms(&self, link: &str) -> Result<i64> {
        if link.trim().is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT position_ms, duration_ms FROM watch_history WHERE link = ?1",
                params![link],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((pos, dur)) = row else {
            return Ok(0);
        };
        if dur > 0 {
            let ratio = pos as f64 / dur as f64;
            if ratio > 0.90 || (dur - pos) < 15_000 {
                return Ok(0);
            }
        }
        Ok(pos.max(0))
    }

    pub fn delete_watch_history_entry(&self, link: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute("DELETE FROM watch_history WHERE link = ?1", params![link])?;
        Ok(())
    }

    pub fn clear_watch_history(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute("DELETE FROM watch_history", [])?;
        Ok(())
    }

    // ── Downloads ─────────────────────────────────────────────────────────────

    pub fn list_downloads(&self) -> Result<Vec<DownloadTask>> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let mut stmt =
            conn.prepare("SELECT payload_json FROM downloads ORDER BY added_at_ms DESC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(task) = serde_json::from_str::<DownloadTask>(&row?) {
                out.push(task);
            }
        }
        Ok(out)
    }

    pub fn upsert_download(&self, task: &DownloadTask) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let json = serde_json::to_string(task)?;
        conn.execute(
            "INSERT OR REPLACE INTO downloads (id, payload_json, added_at_ms) VALUES (?1, ?2, ?3)",
            params![task.id, json, task.added_at_ms],
        )?;
        Ok(())
    }

    pub fn reset_unfinished_downloads_to_pending(&self) -> Result<()> {
        let tasks = self.list_downloads()?;
        for mut t in tasks {
            if matches!(
                t.status,
                DownloadStatus::Downloading | DownloadStatus::Pending
            ) {
                t.status = DownloadStatus::Pending;
                t.speed.clear();
                self.upsert_download(&t)?;
            }
        }
        Ok(())
    }

    pub fn delete_download(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute("DELETE FROM downloads WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ── Extension KV (Scoped per provider_url + scraper_value) ────────────────

    pub fn ext_kv_get(
        &self,
        provider_url: &str,
        scraper_value: &str,
        key: &str,
    ) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let val = conn
            .query_row(
                "SELECT v FROM extension_kv WHERE provider_url = ?1 AND scraper_value = ?2 AND k = ?3",
                params![provider_url, scraper_value, key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(val)
    }

    pub fn ext_kv_set(
        &self,
        provider_url: &str,
        scraper_value: &str,
        key: &str,
        val: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute(
            "INSERT OR REPLACE INTO extension_kv (provider_url, scraper_value, k, v)
             VALUES (?1, ?2, ?3, ?4)",
            params![provider_url, scraper_value, key, val],
        )?;
        Ok(())
    }

    pub fn ext_kv_delete(
        &self,
        provider_url: &str,
        scraper_value: &str,
        key: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute(
            "DELETE FROM extension_kv WHERE provider_url = ?1 AND scraper_value = ?2 AND k = ?3",
            params![provider_url, scraper_value, key],
        )?;
        Ok(())
    }

    pub fn ext_kv_clear(&self, provider_url: &str, scraper_value: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute(
            "DELETE FROM extension_kv WHERE provider_url = ?1 AND scraper_value = ?2",
            params![provider_url, scraper_value],
        )?;
        Ok(())
    }

    // ── Two-Tier VOD Cache (MD5-keyed L1 LRU + L2 SQLite) ─────────────────────

    fn md5_hex(input: &str) -> String {
        let mut hasher = Md5::new();
        hasher.update(input.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn post_key(provider_url: &str, scraper_value: &str, filter_or_query: &str) -> String {
        Self::md5_hex(&format!(
            "{}\0{}\0{}",
            provider_url,
            scraper_value,
            filter_or_query.trim().to_ascii_lowercase()
        ))
    }

    fn catalog_key(provider_url: &str, scraper_value: &str) -> String {
        Self::md5_hex(&format!("{}\0{}", provider_url, scraper_value))
    }

    pub fn get_cached_posts(
        &self,
        provider_url: &str,
        scraper_value: &str,
        filter_or_query: &str,
    ) -> Result<Option<Vec<VegaPost>>> {
        let key = Self::post_key(provider_url, scraper_value, filter_or_query);
        if let Ok(mut cache) = self.mem_posts.lock() {
            if let Some(hit) = cache.get(&key) {
                return Ok(Some(hit.clone()));
            }
        }
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let json: Option<String> = conn
            .query_row(
                "SELECT payload_json FROM cache_entries WHERE cache_key = ?1 AND kind = 'posts'",
                params![format!("posts_{key}")],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(raw) = json {
            if let Ok(list) = serde_json::from_str::<Vec<VegaPost>>(&raw) {
                if !list.is_empty() {
                    if let Ok(mut cache) = self.mem_posts.lock() {
                        cache.put(key, list.clone());
                    }
                }
                return Ok(Some(list));
            }
        }
        Ok(None)
    }

    pub fn put_cached_posts(
        &self,
        provider_url: &str,
        scraper_value: &str,
        filter_or_query: &str,
        posts: &[VegaPost],
        now_ms: i64,
    ) -> Result<()> {
        if posts.is_empty() {
            return Ok(());
        }
        let key = Self::post_key(provider_url, scraper_value, filter_or_query);
        // Truncate BEFORE putting into L1 so L1 and L2 stay consistent (BUG-014 fix)
        let truncated: Vec<VegaPost> = posts.iter().take(80).cloned().collect();
        if let Ok(mut cache) = self.mem_posts.lock() {
            cache.put(key.clone(), truncated.clone());
        }
        let json = serde_json::to_string(&truncated)?;
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute(
            "INSERT OR REPLACE INTO cache_entries (cache_key, kind, payload_json, updated_at_ms)
             VALUES (?1, 'posts', ?2, ?3)",
            params![format!("posts_{key}"), json, now_ms],
        )?;
        Ok(())
    }

    pub fn get_cached_catalog(
        &self,
        provider_url: &str,
        scraper_value: &str,
    ) -> Result<Option<Vec<VegaCatalog>>> {
        let key = Self::catalog_key(provider_url, scraper_value);
        if let Ok(mut cache) = self.mem_catalogs.lock() {
            if let Some(hit) = cache.get(&key) {
                return Ok(Some(hit.clone()));
            }
        }
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let json: Option<String> = conn
            .query_row(
                "SELECT payload_json FROM cache_entries WHERE cache_key = ?1 AND kind = 'catalog'",
                params![format!("cats_{key}")],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(raw) = json {
            if let Ok(list) = serde_json::from_str::<Vec<VegaCatalog>>(&raw) {
                if !list.is_empty() {
                    if let Ok(mut cache) = self.mem_catalogs.lock() {
                        cache.put(key, list.clone());
                    }
                }
                return Ok(Some(list));
            }
        }
        Ok(None)
    }

    pub fn put_cached_catalog(
        &self,
        provider_url: &str,
        scraper_value: &str,
        catalog: &[VegaCatalog],
        now_ms: i64,
    ) -> Result<()> {
        if catalog.is_empty() {
            return Ok(());
        }
        let key = Self::catalog_key(provider_url, scraper_value);
        if let Ok(mut cache) = self.mem_catalogs.lock() {
            cache.put(key.clone(), catalog.to_vec());
        }
        let json = serde_json::to_string(catalog)?;
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute(
            "INSERT OR REPLACE INTO cache_entries (cache_key, kind, payload_json, updated_at_ms)
             VALUES (?1, 'catalog', ?2, ?3)",
            params![format!("cats_{key}"), json, now_ms],
        )?;
        Ok(())
    }

    pub fn get_cached_scrapers(&self, provider_url: &str) -> Result<Option<Vec<VegaProvider>>> {
        let key = Self::md5_hex(provider_url.trim());
        if let Ok(mut cache) = self.mem_scrapers.lock() {
            if let Some(hit) = cache.get(&key) {
                return Ok(Some(hit.clone()));
            }
        }
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let json: Option<String> = conn
            .query_row(
                "SELECT payload_json FROM cache_entries WHERE cache_key = ?1 AND kind = 'scrapers'",
                params![format!("scrapers_{key}")],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(raw) = json {
            if let Ok(list) = serde_json::from_str::<Vec<VegaProvider>>(&raw) {
                if !list.is_empty() {
                    if let Ok(mut cache) = self.mem_scrapers.lock() {
                        cache.put(key, list.clone());
                    }
                }
                return Ok(Some(list));
            }
        }
        Ok(None)
    }

    pub fn put_cached_scrapers(
        &self,
        provider_url: &str,
        scrapers: &[VegaProvider],
        now_ms: i64,
    ) -> Result<()> {
        if scrapers.is_empty() {
            return Ok(());
        }
        let key = Self::md5_hex(provider_url.trim());
        if let Ok(mut cache) = self.mem_scrapers.lock() {
            cache.put(key.clone(), scrapers.to_vec());
        }
        let json = serde_json::to_string(scrapers)?;
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute(
            "INSERT OR REPLACE INTO cache_entries (cache_key, kind, payload_json, updated_at_ms)
             VALUES (?1, 'scrapers', ?2, ?3)",
            params![format!("scrapers_{key}"), json, now_ms],
        )?;
        Ok(())
    }

    pub fn clear_vod_cache(&self) -> Result<()> {
        if let Ok(mut c) = self.mem_posts.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.mem_catalogs.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.mem_scrapers.lock() {
            c.clear();
        }
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        conn.execute("DELETE FROM cache_entries", [])?;
        Ok(())
    }

    // ── Settings ──────────────────────────────────────────────────────────────

    pub fn get_app_settings(&self) -> Result<AppSettings> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let json: Option<String> = conn
            .query_row(
                "SELECT v FROM settings WHERE k = 'app_settings'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match json {
            Some(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
            None => Ok(AppSettings::default()),
        }
    }

    pub fn save_app_settings(&self, settings: &AppSettings) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| StorageError::Lock)?;
        let json = serde_json::to_string(settings)?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (k, v) VALUES ('app_settings', ?1)",
            params![json],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iptvmine_core::ProviderType;

    #[test]
    fn provider_crud_and_settings_roundtrip() {
        let store = Storage::open_in_memory().unwrap();
        let p = Provider {
            title: "My IPTV".into(),
            url: "https://example.com/list.m3u".into(),
            user_agent: Some("CustomUA/1.0".into()),
            is_active: true,
            channel_count: 120,
            r#type: ProviderType::Iptv,
        };
        store.upsert_provider(&p).unwrap();
        assert_eq!(store.list_providers().unwrap().len(), 1);
        store.remove_provider(&p.url).unwrap();
        assert!(store.list_providers().unwrap().is_empty());

        let mut settings = store.get_app_settings().unwrap();
        settings.black_theme = true;
        store.save_app_settings(&settings).unwrap();
        assert!(store.get_app_settings().unwrap().black_theme);
    }

    #[test]
    fn watch_history_caps_at_100_and_resets_near_end() {
        let store = Storage::open_in_memory().unwrap();
        for i in 0..105 {
            store
                .save_watch_progress(&WatchHistoryEntry {
                    title: format!("Item {i}"),
                    link: format!("https://example.com/v/{i}"),
                    stream_url: "https://cdn/s.m3u8".into(),
                    image_url: String::new(),
                    provider_url: String::new(),
                    scraper_value: String::new(),
                    position_ms: 30_000,
                    duration_ms: 100_000,
                    last_watched_ms: i as i64,
                })
                .unwrap();
        }
        assert_eq!(store.get_watch_history().unwrap().len(), 100);
        assert_eq!(
            store
                .get_resume_position_ms("https://example.com/v/104")
                .unwrap(),
            30_000
        );

        store
            .save_watch_progress(&WatchHistoryEntry {
                title: "Finished".into(),
                link: "https://example.com/done".into(),
                stream_url: "https://cdn/done.m3u8".into(),
                image_url: String::new(),
                provider_url: String::new(),
                scraper_value: String::new(),
                position_ms: 95_000,
                duration_ms: 100_000,
                last_watched_ms: 999,
            })
            .unwrap();
        assert_eq!(
            store
                .get_resume_position_ms("https://example.com/done")
                .unwrap(),
            0
        );
    }

    #[test]
    fn vod_cache_truncates_at_80_consistently() {
        let store = Storage::open_in_memory().unwrap();
        let posts: Vec<VegaPost> = (0..120)
            .map(|i| VegaPost {
                title: format!("Post {i}"),
                link: format!("https://example.com/{i}"),
                image: String::new(),
            })
            .collect();
        store
            .put_cached_posts("https://repo", "vega", "/cat", &posts, 1)
            .unwrap();
        let cached = store
            .get_cached_posts("https://repo", "vega", "/cat")
            .unwrap()
            .unwrap();
        assert_eq!(cached.len(), 80);
    }
}
