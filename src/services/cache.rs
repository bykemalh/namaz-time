use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use jiff::tz::TimeZone;
use jiff::Timestamp;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tracing::warn;

use crate::models::{DayTimes, ProviderKind};

const LOCATION_LIST_TTL_SECS: i64 = 7 * 86_400;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedPayload<T> {
    fetched_at_unix: i64,
    payload: T,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedDays {
    pub days: BTreeMap<String, DayTimes>,
    pub last_sync_unix: i64,
    pub last_source: Option<ProviderKind>,
}

#[derive(Debug, Clone)]
pub struct DiskCache {
    root: PathBuf,
}

impl DiskCache {
    pub fn default_xdg() -> Self {
        let base = gtk::glib::user_cache_dir();
        Self::with_root(base.join("me.bykemah.namaztime"))
    }

    pub fn with_root(root: PathBuf) -> Self {
        let _ = fs::create_dir_all(root.join("locations"));
        let _ = fs::create_dir_all(root.join("times"));
        Self { root }
    }

    fn location_path(&self, key: &str) -> PathBuf {
        let safe: String = key
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        self.root.join("locations").join(format!("{safe}.json"))
    }

    fn times_path(&self, location_key: &str) -> PathBuf {
        let safe: String = location_key
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        self.root.join("times").join(format!("{safe}.json"))
    }

    pub fn load_location_list<T: DeserializeOwned>(
        &self,
        key: &str,
        now_unix: i64,
        allow_expired: bool,
    ) -> Option<T> {
        let path = self.location_path(key);
        let raw = fs::read_to_string(&path).ok()?;
        let cached: CachedPayload<T> = serde_json::from_str(&raw).ok()?;
        let age = now_unix.saturating_sub(cached.fetched_at_unix);
        if allow_expired || age <= LOCATION_LIST_TTL_SECS {
            Some(cached.payload)
        } else {
            None
        }
    }

    pub fn save_location_list<T: Serialize>(&self, key: &str, payload: &T, now_unix: i64) {
        let path = self.location_path(key);
        let wrapper = CachedPayload {
            fetched_at_unix: now_unix,
            payload,
        };
        if let Ok(json) = serde_json::to_string(&wrapper) {
            if let Err(err) = atomic_write(&path, json.as_bytes()) {
                warn!("Konum önbelleği yazılamadı ({path:?}): {err}");
            }
        }
    }

    pub fn load_days(&self, location_key: &str) -> CachedDays {
        let path = self.times_path(location_key);
        let Ok(raw) = fs::read_to_string(&path) else {
            return CachedDays::default();
        };
        match serde_json::from_str::<CachedDays>(&raw) {
            Ok(mut cached) => {
                cached.days.retain(|_, d| d.validate().is_ok());
                cached
            }
            Err(err) => {
                warn!("Vakit önbelleği okunamadı ({path:?}): {err}");
                CachedDays::default()
            }
        }
    }

    pub fn merge_days(
        &self,
        location_key: &str,
        new_days: Vec<DayTimes>,
        source: ProviderKind,
        now: Timestamp,
        timezone_iana: &str,
    ) -> CachedDays {
        let mut current = self.load_days(location_key);
        let tz = TimeZone::get(timezone_iana).unwrap_or_else(|_| TimeZone::UTC);
        let today = now.to_zoned(tz).date();
        let min_keep = today
            .checked_sub(jiff::Span::new().days(7))
            .map(|d| d.to_string())
            .unwrap_or_default();

        for day in new_days {
            if day.validate().is_ok() {
                current.days.insert(day.date.clone(), day);
            }
        }

        if !min_keep.is_empty() {
            current.days.retain(|date, _| date.as_str() >= min_keep.as_str());
        }

        current.last_sync_unix = now.as_second();
        current.last_source = Some(source);

        let path = self.times_path(location_key);
        if let Ok(json) = serde_json::to_string(&current) {
            if let Err(err) = atomic_write(&path, json.as_bytes()) {
                warn!("Vakit önbelleği yazılamadı ({path:?}): {err}");
            }
        }
        current
    }

    pub fn should_refresh(cached: &CachedDays, timezone_iana: &str, now: Timestamp) -> bool {
        let Ok(tz) = TimeZone::get(timezone_iana) else {
            return true;
        };
        let today_date = now.to_zoned(tz).date();
        let today_key = today_date.to_string();
        let tomorrow_key = today_date
            .tomorrow()
            .map(|d| d.to_string())
            .unwrap_or_default();

        let Some(today_entry) = cached.days.get(&today_key) else {
            return true;
        };
        if !tomorrow_key.is_empty() && !cached.days.contains_key(&tomorrow_key) {
            return true;
        }

        let max_age_secs = match today_entry.source {
            ProviderKind::Idamedya => 6 * 3600,
            ProviderKind::Imsakiyem | ProviderKind::Mtopal => 24 * 3600,
        };
        now.as_second().saturating_sub(cached.last_sync_unix) > max_age_secs
    }
}

fn atomic_write(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_day(date: &str, source: ProviderKind, fetched: i64) -> DayTimes {
        DayTimes {
            date: date.to_string(),
            hijri: None,
            imsak: "06:16".to_string(),
            gunes: "07:44".to_string(),
            ogle: "12:54".to_string(),
            ikindi: "15:29".to_string(),
            aksam: "17:53".to_string(),
            yatsi: "19:16".to_string(),
            source,
            fetched_at_unix: fetched,
        }
    }

    #[test]
    fn merges_and_evaluates_refresh_ttl() {
        let dir = std::env::temp_dir().join(format!(
            "namaz_time_cache_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let cache = DiskCache::with_root(dir.clone());

        let now: Timestamp = "2026-11-15T12:00:00Z".parse().unwrap();
        let days = vec![
            make_day("2026-11-15", ProviderKind::Mtopal, now.as_second()),
            make_day("2026-11-16", ProviderKind::Mtopal, now.as_second()),
        ];

        let cached = cache.merge_days("tr_test", days, ProviderKind::Mtopal, now, "Europe/Istanbul");
        assert_eq!(cached.days.len(), 2);
        assert!(!DiskCache::should_refresh(&cached, "Europe/Istanbul", now));

        let later: Timestamp = "2026-11-16T12:00:00Z".parse().unwrap();
        // Tomorrow (2026-11-17) is missing from cache -> must refresh
        assert!(DiskCache::should_refresh(&cached, "Europe/Istanbul", later));

        let _ = fs::remove_dir_all(&dir);
    }
}
