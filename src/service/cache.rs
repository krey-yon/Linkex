//! Bounded TTL-aware LRU cache with optional one-file-per-entry disk
//! persistence. Every miss costs a request against a rate-limited upstream
//! account, so caching matters more here than in a typical service.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use lru::LruCache;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::profile::Profile;

pub const SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Clone)]
pub struct CachedProfile {
    pub profile: Arc<Profile>,
    /// Monotonic insertion instant for memory-side TTL.
    pub inserted_at: Instant,
    /// Wall-clock epoch (seconds) for restart survival / age reporting.
    pub stored_at: f64,
}

impl CachedProfile {
    pub fn age_seconds(&self) -> f64 {
        // Monotonic where available; wall clock when freshly loaded from disk.
        let elapsed = self.inserted_at.elapsed().as_secs_f64();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if elapsed > 0.0 && self.stored_at > 0.0 {
            (now - self.stored_at).max(0.0)
        } else {
            elapsed
        }
    }
}

#[derive(Serialize, Deserialize)]
struct DiskEntry {
    schema_version: u64,
    key: String,
    stored_at: f64,
    profile: Profile,
}

pub struct ProfileCache {
    ttl: std::time::Duration,
    directory: Option<PathBuf>,
    entries: std::sync::Mutex<LruCache<String, CachedProfile>>,
    hits: std::sync::atomic::AtomicU64,
    misses: std::sync::atomic::AtomicU64,
}

impl ProfileCache {
    pub fn new(
        ttl_seconds: u64,
        max_entries: usize,
        directory: Option<&str>,
        persist: bool,
    ) -> Self {
        let directory = directory
            .filter(|d| !d.is_empty())
            .filter(|_| persist)
            .map(PathBuf::from);
        let max_entries = max_entries.max(1);
        ProfileCache {
            ttl: std::time::Duration::from_secs(ttl_seconds),
            directory,
            entries: std::sync::Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(max_entries).expect(">= 1"),
            )),
            hits: std::sync::atomic::AtomicU64::new(0),
            misses: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn ttl(&self) -> u64 {
        self.ttl.as_secs()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Memory lookup. Disk fallback is a separate method so the caller can
    /// run it off the lock (e.g. via `spawn_blocking`).
    pub fn get(&self, key: &str) -> Option<CachedProfile> {
        let mut entries = self.entries.lock().unwrap();
        let Some(entry) = entries.get(key) else {
            self.misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return None;
        };
        if entry.inserted_at.elapsed() > self.ttl {
            self.misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            entries.pop(key);
            return None;
        }
        self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(entry.clone())
    }

    /// Read the persisted copy for `key` (blocking; run off the request path
    /// where possible). Validates the stored key and enforces TTL.
    pub fn load_disk(&self, key: &str) -> Option<CachedProfile> {
        let path = self.path_for(key)?;
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => return None,
        };
        let entry: DiskEntry = match serde_json::from_str(&raw) {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!(key = key, error = %err, "cache.disk_read_failed");
                let _ = std::fs::remove_file(&path);
                return None;
            }
        };
        if entry.schema_version != SCHEMA_VERSION || entry.key != key {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if now - entry.stored_at > self.ttl.as_secs_f64() {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        Some(CachedProfile {
            profile: Arc::new(entry.profile),
            inserted_at: Instant::now(),
            stored_at: entry.stored_at,
        })
    }

    pub fn insert_memory(&self, key: String, entry: CachedProfile) {
        let mut entries = self.entries.lock().unwrap();
        entries.put(key, entry);
    }

    /// Persist (blocking; run off the request path where possible).
    pub fn save_disk(&self, key: &str, entry: &CachedProfile) {
        let Some(path) = self.path_for(key) else {
            return;
        };
        let payload = DiskEntry {
            schema_version: SCHEMA_VERSION,
            key: key.to_string(),
            stored_at: entry.stored_at,
            profile: (*entry.profile).clone(),
        };
        let json = match serde_json::to_vec(&payload) {
            Ok(json) => json,
            Err(err) => {
                tracing::warn!(key = key, error = %err, "cache.disk_write_failed");
                return;
            }
        };
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }
        let temp = path.with_extension("json.tmp");
        if let Err(err) = std::fs::write(&temp, &json) {
            tracing::warn!(key = key, error = %err, "cache.disk_write_failed");
            return;
        }
        if let Err(err) = std::fs::rename(&temp, &path) {
            tracing::warn!(key = key, error = %err, "cache.disk_write_failed");
            let _ = std::fs::remove_file(&temp);
        }
    }

    pub fn invalidate(&self, key: &str) {
        self.entries.lock().unwrap().pop(key);
        if let Some(path) = self.path_for(key) {
            let _ = std::fs::remove_file(path);
        }
    }

    pub fn clear_memory(&self) {
        self.entries.lock().unwrap().clear();
    }

    fn path_for(&self, key: &str) -> Option<PathBuf> {
        let directory = self.directory.as_ref()?;
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        Some(directory.join(format!("{digest}.json")))
    }

    /// Bounded diagnostics: never label anything by profile identifier.
    pub fn stats(&self) -> serde_json::Value {
        let hits = self.hits.load(std::sync::atomic::Ordering::Relaxed);
        let misses = self.misses.load(std::sync::atomic::Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate = if total > 0 {
            serde_json::Value::from((hits as f64 / total as f64 * 1000.0).round() / 1000.0)
        } else {
            serde_json::Value::Null
        };
        serde_json::json!({
            "entries": self.len(),
            "ttl_seconds": self.ttl(),
            "hits": hits,
            "misses": misses,
            "hit_rate": hit_rate,
            "persistent": self.directory.is_some(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::profile::{Profile, ProfileMeta};
    use chrono::{TimeZone, Utc};

    fn sample(name: &str) -> Profile {
        Profile {
            profile_url: format!("https://www.linkedin.com/in/{name}/"),
            public_identifier: Some(name.to_string()),
            member_urn: None,
            profile_id: None,
            first_name: Some("Ada".to_string()),
            last_name: None,
            full_name: Some("Ada Lovelace".to_string()),
            headline: Some("Engineer".to_string()),
            about: None,
            industry: None,
            pronouns: None,
            location: None,
            profile_picture: None,
            background_picture: None,
            network: None,
            contact: None,
            experience: Vec::new(),
            education: Vec::new(),
            skills: Vec::new(),
            certifications: Vec::new(),
            languages: Vec::new(),
            projects: Vec::new(),
            publications: Vec::new(),
            honors: Vec::new(),
            volunteering: Vec::new(),
            courses: Vec::new(),
            patents: Vec::new(),
            test_scores: Vec::new(),
            organizations: Vec::new(),
            meta: ProfileMeta {
                fetched_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                sources: Vec::new(),
                warnings: Vec::new(),
                sections_populated: Vec::new(),
                completeness: 0.0,
            },
        }
    }

    #[test]
    fn hit_miss_and_ttl() {
        let cache = ProfileCache::new(900, 16, None, false);
        let key = "profile:ada".to_string();
        assert!(cache.get(&key).is_none());
        cache.insert_memory(
            key.clone(),
            CachedProfile {
                profile: Arc::new(sample("ada")),
                inserted_at: Instant::now(),
                stored_at: 1.0,
            },
        );
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn lru_evicts_oldest() {
        let cache = ProfileCache::new(900, 2, None, false);
        cache.insert_memory(
            "a".to_string(),
            CachedProfile {
                profile: Arc::new(sample("a")),
                inserted_at: Instant::now(),
                stored_at: 0.0,
            },
        );
        cache.insert_memory(
            "b".to_string(),
            CachedProfile {
                profile: Arc::new(sample("b")),
                inserted_at: Instant::now(),
                stored_at: 0.0,
            },
        );
        assert!(cache.get("a").is_some());
        cache.insert_memory(
            "c".to_string(),
            CachedProfile {
                profile: Arc::new(sample("c")),
                inserted_at: Instant::now(),
                stored_at: 0.0,
            },
        );
        // "b" was evicted as the least recently used.
        assert!(cache.get("b").is_none());
        assert!(cache.get("a").is_some());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn disk_round_trip_corruption_and_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ProfileCache::new(900, 16, Some(dir.path().to_str().unwrap_or("")), true);
        let key = "profile:ada".to_string();
        let entry = CachedProfile {
            profile: Arc::new(sample("ada")),
            inserted_at: Instant::now(),
            stored_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
        };
        cache.save_disk(&key, &entry);
        let loaded = cache.load_disk(&key).expect("disk hit");
        assert_eq!(loaded.profile.public_identifier.as_deref(), Some("ada"));

        // Corrupt: invalid JSON
        let path = dir
            .path()
            .read_dir()
            .unwrap()
            .filter_map(|e| e.ok())
            .next()
            .unwrap()
            .path();
        std::fs::write(&path, "{oops").unwrap();
        assert!(cache.load_disk(&key).is_none());
        assert!(!path.exists(), "corrupt file removed");

        // Wrong key: rewrite with another key.
        cache.save_disk(&key, &entry);
        let other = cache.load_disk("profile:bob");
        assert!(other.is_none());
    }

    #[test]
    fn schema_version_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ProfileCache::new(900, 16, Some(dir.path().to_str().unwrap_or("")), true);
        // sha256("x") hex digest — the name path_for() derives from the key.
        let path = dir
            .path()
            .join("2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881.json");
        std::fs::write(
            &path,
            r#"{"schema_version": 99, "key": "x", "stored_at": 1.0, "profile": {}}"#,
        )
        .unwrap();
        assert!(cache.load_disk("x").is_none());
        assert!(!path.exists());
    }

    #[test]
    fn stats_bounded() {
        let cache = ProfileCache::new(900, 16, None, false);
        cache.insert_memory(
            "k".to_string(),
            CachedProfile {
                profile: Arc::new(sample("k")),
                inserted_at: Instant::now(),
                stored_at: 0.0,
            },
        );
        let _ = cache.get("k");
        let _ = cache.get("nope");
        let stats = cache.stats();
        assert_eq!(stats["hits"], 1);
        assert_eq!(stats["misses"], 1);
        assert_eq!(stats["entries"], 1);
    }
}
