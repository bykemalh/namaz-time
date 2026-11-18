pub mod aladhan;
pub mod diyanet_mirror;
pub mod imsakiyem;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jiff::tz::TimeZone;
use jiff::Timestamp;
use reqwest::{Client, Response, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use tracing::{info, warn};

use crate::i18n::{tr, tr_fmt};
use crate::models::location::{
    expand_name_aliases, fallback_timezone_for_country, match_unique_by_name, normalize_name,
    system_timezone_iana,
};
use crate::models::{LocationCandidate, LocationProfile, ProviderIds, ProviderKind};
use crate::providers::diyanet_mirror::{
    DiyanetMirror, EZANVAKTI_MIRROR, MTOPAL_MIRROR, MirrorCity, MirrorCountry, MirrorDistrict,
};
use crate::services::cache::{CachedDays, DiskCache};

#[derive(Debug, Clone)]
pub enum ProviderError {
    RateLimited { retry_after_secs: Option<u64> },
    NotFound,
    ServerError(u16),
    Network(String),
    InvalidResponse(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited {
                retry_after_secs: Some(s),
            } => write!(f, "{}", tr_fmt("Rate limit exceeded (wait %1s)", &[&s.to_string()])),
            Self::RateLimited {
                retry_after_secs: None,
            } => write!(f, "{}", tr("Rate limit exceeded (429)")),
            Self::NotFound => write!(f, "{}", tr("Location not found (404)")),
            Self::ServerError(code) => write!(f, "{}", tr_fmt("Server error (HTTP %1)", &[&code.to_string()])),
            Self::Network(msg) => write!(f, "{}", tr_fmt("Connection error: %1", &[msg])),
            Self::InvalidResponse(msg) => write!(f, "{}", tr_fmt("Invalid response: %1", &[msg])),
        }
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(err: reqwest::Error) -> Self {
        Self::Network(err.to_string())
    }
}

pub(crate) async fn check_response_status(resp: Response) -> Result<Response, ProviderError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        return Err(ProviderError::RateLimited {
            retry_after_secs: retry,
        });
    }
    if status == StatusCode::NOT_FOUND {
        return Err(ProviderError::NotFound);
    }
    if status.is_server_error() {
        return Err(ProviderError::ServerError(status.as_u16()));
    }
    Err(ProviderError::InvalidResponse(format!(
        "HTTP {}",
        status.as_u16()
    )))
}

#[derive(Debug, Clone)]
pub struct CountryOption {
    pub imsakiyem_id: Option<String>,
    pub mirror: Option<(DiyanetMirror, String)>,
    pub iso2: String,
    pub name: String,
    pub name_en: String,
}

#[derive(Debug, Clone)]
pub struct StateOption {
    pub imsakiyem_id: Option<String>,
    pub mirror: Option<(DiyanetMirror, String)>,
    pub name: String,
    pub name_en: String,
}

#[derive(Debug, Clone)]
pub struct FetchTimesResult {
    pub cached: CachedDays,
    pub stale: bool,
    pub stale_reason: Option<String>,
}

pub struct ProviderCoordinator {
    client: Client,
    cooldowns: Mutex<HashMap<ProviderKind, Instant>>,
}

impl ProviderCoordinator {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("namaz-time/0.1.0 (+https://github.com/bykemalh/namaz-time)")
            .timeout(Duration::from_secs(12))
            .build()
            .expect("HTTP istemcisi başlatılamadı");

        Self {
            client,
            cooldowns: Mutex::new(HashMap::new()),
        }
    }

    fn is_cooling_down(&self, provider: ProviderKind) -> bool {
        let guard = self.cooldowns.lock().unwrap();
        guard
            .get(&provider)
            .is_some_and(|until| Instant::now() < *until)
    }

    fn record_rate_limit(&self, provider: ProviderKind, retry_after_secs: Option<u64>) {
        let secs = retry_after_secs.unwrap_or(60).clamp(15, 600);
        let mut guard = self.cooldowns.lock().unwrap();
        guard.insert(provider, Instant::now() + Duration::from_secs(secs));
    }

    /// Loads global country list (Imsakiyem v2 primary, mtopal / EzanVakti mirrors fallback) with 7-day cache.
    pub async fn list_countries(&self, cache: &DiskCache) -> Result<Vec<CountryOption>, String> {
        let now_unix = Timestamp::now().as_second();
        if let Some(cached) =
            cache.load_location_list::<Vec<imsakiyem::ImsakiyemCountry>>("imsakiyem_countries", now_unix, false)
        {
            return Ok(sort_countries(
                cached
                    .into_iter()
                    .map(|c| CountryOption {
                        imsakiyem_id: Some(c.id),
                        mirror: None,
                        iso2: c.iso2,
                        name: c.names.as_ref().and_then(|n| n.tr.clone()).unwrap_or(c.name),
                        name_en: c.names.and_then(|n| n.en).unwrap_or_default(),
                    })
                    .collect(),
            ));
        }

        if !self.is_cooling_down(ProviderKind::Imsakiyem) {
            match imsakiyem::fetch_countries(&self.client).await {
                Ok(list) if !list.is_empty() => {
                    cache.save_location_list("imsakiyem_countries", &list, now_unix);
                    return Ok(sort_countries(
                        list.into_iter()
                            .map(|c| CountryOption {
                                imsakiyem_id: Some(c.id),
                                mirror: None,
                                iso2: c.iso2,
                                name: c.names.as_ref().and_then(|n| n.tr.clone()).unwrap_or(c.name),
                                name_en: c.names.and_then(|n| n.en).unwrap_or_default(),
                            })
                            .collect(),
                    ));
                }
                Ok(_) => {}
                Err(ProviderError::RateLimited { retry_after_secs }) => {
                    self.record_rate_limit(ProviderKind::Imsakiyem, retry_after_secs);
                }
                Err(err) => warn!("İmsakiyem ülke listesi hatası: {err}"),
            }
        }

        let mut last_err = None;
        for mirror in [MTOPAL_MIRROR, EZANVAKTI_MIRROR] {
            match self.get_mirror_countries(mirror, cache).await {
                Ok(list) => {
                    return Ok(sort_countries(
                        list.into_iter()
                            .map(|c| CountryOption {
                                imsakiyem_id: None,
                                mirror: Some((mirror, c.ulke_id)),
                                iso2: String::new(),
                                name: c.ulke_adi,
                                name_en: c.ulke_adi_en,
                            })
                            .collect(),
                    ));
                }
                Err(err) => last_err = Some(err),
            }
        }

        Err(last_err.unwrap_or_else(|| tr("Unable to load country list")))
    }

    pub async fn list_states(
        &self,
        country: &CountryOption,
        cache: &DiskCache,
    ) -> Result<Vec<StateOption>, String> {
        let now_unix = Timestamp::now().as_second();
        if let Some(cid) = &country.imsakiyem_id {
            let key = format!("imsakiyem_states_{cid}");
            if let Some(cached) =
                cache.load_location_list::<Vec<imsakiyem::ImsakiyemState>>(&key, now_unix, false)
            {
                return Ok(cached
                    .into_iter()
                    .map(|s| StateOption {
                        imsakiyem_id: Some(s.id),
                        mirror: None,
                        name: s.name,
                        name_en: s.name_en.unwrap_or_default(),
                    })
                    .collect());
            }
            if !self.is_cooling_down(ProviderKind::Imsakiyem) {
                match imsakiyem::fetch_states(&self.client, cid).await {
                    Ok(list) => {
                        cache.save_location_list(&key, &list, now_unix);
                        return Ok(list
                            .into_iter()
                            .map(|s| StateOption {
                                imsakiyem_id: Some(s.id),
                                mirror: None,
                                name: s.name,
                                name_en: s.name_en.unwrap_or_default(),
                            })
                            .collect());
                    }
                    Err(ProviderError::RateLimited { retry_after_secs }) => {
                        self.record_rate_limit(ProviderKind::Imsakiyem, retry_after_secs);
                    }
                    Err(err) => warn!("İmsakiyem şehir listesi hatası: {err}"),
                }
            }
        }

        if let Some((mirror, country_id)) = &country.mirror {
            let cities = self.get_mirror_cities(*mirror, country_id, cache).await?;
            return Ok(cities
                .into_iter()
                .map(|c| StateOption {
                    imsakiyem_id: None,
                    mirror: Some((*mirror, c.sehir_id)),
                    name: c.sehir_adi,
                    name_en: c.sehir_adi_en,
                })
                .collect());
        }

        Err(tr("Unable to load city list"))
    }

    pub async fn list_districts(
        &self,
        country: &CountryOption,
        state: &StateOption,
        cache: &DiskCache,
    ) -> Result<Vec<LocationCandidate>, String> {
        let now_unix = Timestamp::now().as_second();
        if let Some(sid) = &state.imsakiyem_id {
            let key = format!("imsakiyem_districts_state_{sid}");
            if let Some(cached) = cache.load_location_list::<Vec<LocationCandidate>>(&key, now_unix, false) {
                if !cached.is_empty() {
                    return Ok(cached);
                }
            }
            if !self.is_cooling_down(ProviderKind::Imsakiyem) {
                match imsakiyem::fetch_districts(&self.client, Some(sid), None).await {
                    Ok(list) if !list.is_empty() => {
                        cache.save_location_list(&key, &list, now_unix);
                        return Ok(list);
                    }
                    Ok(_) => {
                        // Small countries like Monaco may omit state.id on districts: fallback to countryId
                        if let Some(cid) = &country.imsakiyem_id {
                            if let Ok(by_country) =
                                imsakiyem::fetch_districts(&self.client, None, Some(cid)).await
                            {
                                if !by_country.is_empty() {
                                    cache.save_location_list(&key, &by_country, now_unix);
                                    return Ok(by_country);
                                }
                            }
                        }
                    }
                    Err(ProviderError::RateLimited { retry_after_secs }) => {
                        self.record_rate_limit(ProviderKind::Imsakiyem, retry_after_secs);
                    }
                    Err(err) => warn!("İmsakiyem ilçe listesi hatası: {err}"),
                }
            }
        }

        if let Some((mirror, city_id)) = &state.mirror {
            let districts = self.get_mirror_districts(*mirror, city_id, cache).await?;
            let tz = fallback_timezone_for_country(&country.iso2, &country.name)
                .map(str::to_string);
            return Ok(districts
                .into_iter()
                .map(|d| {
                    let mut ids = ProviderIds::default();
                    match mirror.kind {
                        ProviderKind::Mtopal => ids.mtopal = Some(d.ilce_id.clone()),
                        ProviderKind::Ezanvakti => ids.ezanvakti = Some(d.ilce_id.clone()),
                        _ => {}
                    }
                    LocationCandidate {
                        country_code: country.iso2.clone(),
                        country_name: country.name.clone(),
                        city_name: state.name.clone(),
                        district_name: d.ilce_adi,
                        display_label: String::new(),
                        timezone: tz.clone(),
                        lat: None,
                        lon: None,
                        provider_ids: ids,
                    }
                })
                .map(|mut cand| {
                    cand.display_label = format!(
                        "{}, {}, {}",
                        cand.district_name, cand.city_name, cand.country_name
                    );
                    cand
                })
                .collect());
        }

        Err(tr("Unable to load district list"))
    }

    pub async fn search_locations(
        &self,
        query: &str,
        cache: &DiskCache,
    ) -> Result<Vec<LocationCandidate>, String> {
        let trimmed = query.trim();
        if trimmed.chars().count() < 2 {
            return Ok(Vec::new());
        }

        if !self.is_cooling_down(ProviderKind::Imsakiyem) {
            match imsakiyem::search_districts(&self.client, trimmed).await {
                Ok(results) => return Ok(results),
                Err(ProviderError::RateLimited { retry_after_secs }) => {
                    self.record_rate_limit(ProviderKind::Imsakiyem, retry_after_secs);
                }
                Err(err) => warn!("İmsakiyem arama hatası: {err}"),
            }
        }

        // Fallback search across Turkish provinces from reliable Diyanet mirrors (country ID = 2)
        let q_norm = normalize_name(trimmed);
        for mirror in [EZANVAKTI_MIRROR, MTOPAL_MIRROR] {
            if let Ok(cities) = self.get_mirror_cities(mirror, "2", cache).await {
                let tr_name = tr("Turkey");
                let matched: Vec<LocationCandidate> = cities
                    .into_iter()
                    .filter(|c| normalize_name(&c.sehir_adi).contains(&q_norm))
                    .map(|c| LocationCandidate {
                        country_code: "TR".to_string(),
                        country_name: tr_name.clone(),
                        city_name: c.sehir_adi.clone(),
                        district_name: c.sehir_adi.clone(),
                        display_label: format!("{}, {}", c.sehir_adi, tr_name),
                        timezone: Some("Europe/Istanbul".to_string()),
                        lat: None,
                        lon: None,
                        provider_ids: ProviderIds::default(),
                    })
                    .collect();
                if !matched.is_empty() {
                    return Ok(matched);
                }
            }
        }

        Err(tr("Location search is currently unavailable"))
    }

    /// Ensures the LocationProfile has accurate IANA timezone and resolves verified matching
    /// IDs across providers without blindly assuming identical numeric IDs.
    pub async fn resolve_location_profile(
        &self,
        candidate: LocationCandidate,
        cache: &DiskCache,
    ) -> LocationProfile {
        let mut ids = candidate.provider_ids.clone();
        let mut timezone = candidate.timezone.clone();
        let mut country_code = candidate.country_code.clone();
        let mut lat = candidate.lat;
        let mut lon = candidate.lon;

        // Enrich with Imsakiyem detail if we have imsakiyem_id but missing metadata
        if timezone.is_none() || lat.is_none() || lon.is_none() {
            if let Some(ref ims_id) = ids.imsakiyem {
                if let Ok(detail) = imsakiyem::fetch_district_detail(&self.client, ims_id).await {
                    if timezone.is_none() {
                        timezone = detail.timezone;
                    }
                    if country_code.is_empty() {
                        country_code = detail.country_code;
                    }
                    if lat.is_none() {
                        lat = detail.lat;
                    }
                    if lon.is_none() {
                        lon = detail.lon;
                    }
                }
            }
        }

        // If selected from a mirror while Imsakiyem was down, try matching in Imsakiyem search
        if ids.imsakiyem.is_none() && !self.is_cooling_down(ProviderKind::Imsakiyem) {
            if let Ok(results) =
                imsakiyem::search_districts(&self.client, &candidate.district_name).await
            {
                let d_targets = expand_name_aliases(&candidate.district_name, Some(&candidate.city_name));
                let c_targets = expand_name_aliases(&candidate.country_name, None);
                let matched = results
                    .into_iter()
                    .filter(|r| {
                        let r_c = expand_name_aliases(&r.country_name, None);
                        let r_d = expand_name_aliases(&r.district_name, Some(&r.city_name));
                        c_targets.iter().any(|x| r_c.contains(x))
                            && d_targets.iter().any(|x| r_d.contains(x))
                    })
                    .collect::<Vec<_>>();
                if matched.len() == 1 {
                    ids.imsakiyem = matched[0].provider_ids.imsakiyem.clone();
                    if timezone.is_none() {
                        timezone = matched[0].timezone.clone();
                    }
                    if country_code.is_empty() {
                        country_code = matched[0].country_code.clone();
                    }
                    if lat.is_none() {
                        lat = matched[0].lat;
                    }
                    if lon.is_none() {
                        lon = matched[0].lon;
                    }
                }
            }
        }

        let resolved_tz = timezone
            .or_else(|| {
                fallback_timezone_for_country(&country_code, &candidate.country_name)
                    .map(str::to_string)
            })
            .unwrap_or_else(system_timezone_iana);

        let mut profile = LocationProfile {
            country_code,
            country_name: candidate.country_name,
            city_name: candidate.city_name,
            district_name: candidate.district_name,
            display_label: candidate.display_label,
            timezone: resolved_tz,
            lat,
            lon,
            provider_ids: ids,
        };

        if profile.provider_ids.mtopal.is_none() {
            profile.provider_ids.mtopal = self.match_mirror_id(MTOPAL_MIRROR, &profile, cache).await;
        }
        if profile.provider_ids.ezanvakti.is_none() {
            profile.provider_ids.ezanvakti =
                self.match_mirror_id(EZANVAKTI_MIRROR, &profile, cache).await;
        }

        info!(
            "Konum eşleştirildi: {} -> tz={}, imsakiyem={:?}, mtopal={:?}, ezanvakti={:?}",
            profile.display_label,
            profile.timezone,
            profile.provider_ids.imsakiyem,
            profile.provider_ids.mtopal,
            profile.provider_ids.ezanvakti
        );

        profile
    }

    async fn match_mirror_id(
        &self,
        mirror: DiyanetMirror,
        profile: &LocationProfile,
        cache: &DiskCache,
    ) -> Option<String> {
        let countries = self.get_mirror_countries(mirror, cache).await.ok()?;
        let c_targets = expand_name_aliases(&profile.country_name, None);
        let country = match_unique_by_name(&c_targets, &countries, |c| {
            let mut v = expand_name_aliases(&c.ulke_adi, None);
            if !c.ulke_adi_en.is_empty() {
                v.extend(expand_name_aliases(&c.ulke_adi_en, None));
            }
            v
        })?;

        let cities = self.get_mirror_cities(mirror, &country.ulke_id, cache).await.ok()?;
        let s_targets = expand_name_aliases(&profile.city_name, None);
        let city = match_unique_by_name(&s_targets, &cities, |s| {
            let mut v = expand_name_aliases(&s.sehir_adi, None);
            if !s.sehir_adi_en.is_empty() {
                v.extend(expand_name_aliases(&s.sehir_adi_en, None));
            }
            v
        })?;

        let districts = self.get_mirror_districts(mirror, &city.sehir_id, cache).await.ok()?;
        let d_targets = expand_name_aliases(&profile.district_name, Some(&profile.city_name));
        let district = match_unique_by_name(&d_targets, &districts, |d| {
            let mut v = expand_name_aliases(&d.ilce_adi, Some(&city.sehir_adi));
            if !d.ilce_adi_en.is_empty() {
                v.extend(expand_name_aliases(&d.ilce_adi_en, Some(&city.sehir_adi_en)));
            }
            v
        })?;

        Some(district.ilce_id.clone())
    }

    async fn get_cached_mirror_list<T, Fut>(
        &self,
        mirror: DiyanetMirror,
        cache_key: &str,
        cache: &DiskCache,
        fetch: impl FnOnce() -> Fut,
    ) -> Result<Vec<T>, String>
    where
        T: Serialize + DeserializeOwned,
        Fut: Future<Output = Result<Vec<T>, ProviderError>>,
    {
        let now_unix = Timestamp::now().as_second();
        if let Some(cached) = cache.load_location_list::<Vec<T>>(cache_key, now_unix, false) {
            return Ok(cached);
        }
        if self.is_cooling_down(mirror.kind) {
            return cache
                .load_location_list(cache_key, now_unix, true)
                .ok_or_else(|| tr_fmt("%1 rate limit pending", &[mirror.label]));
        }
        match fetch().await {
            Ok(list) => {
                cache.save_location_list(cache_key, &list, now_unix);
                Ok(list)
            }
            Err(ProviderError::RateLimited { retry_after_secs }) => {
                self.record_rate_limit(mirror.kind, retry_after_secs);
                cache
                    .load_location_list(cache_key, now_unix, true)
                    .ok_or_else(|| tr_fmt("%1 rate limit exceeded", &[mirror.label]))
            }
            Err(err) => cache
                .load_location_list(cache_key, now_unix, true)
                .ok_or_else(|| err.to_string()),
        }
    }

    async fn get_mirror_countries(
        &self,
        mirror: DiyanetMirror,
        cache: &DiskCache,
    ) -> Result<Vec<MirrorCountry>, String> {
        let key = format!("{}_countries", mirror.cache_prefix);
        self.get_cached_mirror_list(mirror, &key, cache, || {
            diyanet_mirror::fetch_countries(&self.client, mirror)
        })
        .await
    }

    async fn get_mirror_cities(
        &self,
        mirror: DiyanetMirror,
        country_id: &str,
        cache: &DiskCache,
    ) -> Result<Vec<MirrorCity>, String> {
        let key = format!("{}_cities_{country_id}", mirror.cache_prefix);
        self.get_cached_mirror_list(mirror, &key, cache, || {
            diyanet_mirror::fetch_cities(&self.client, mirror, country_id)
        })
        .await
    }

    async fn get_mirror_districts(
        &self,
        mirror: DiyanetMirror,
        city_id: &str,
        cache: &DiskCache,
    ) -> Result<Vec<MirrorDistrict>, String> {
        let key = format!("{}_districts_{city_id}", mirror.cache_prefix);
        self.get_cached_mirror_list(mirror, &key, cache, || {
            diyanet_mirror::fetch_districts(&self.client, mirror, city_id)
        })
        .await
    }

    /// Loads prayer days from disk cache when fresh, otherwise queries providers in fallback order
    /// and validates that today's local date is covered. Never fabricates missing data.
    pub async fn get_prayer_days(
        &self,
        profile: &LocationProfile,
        preferred: Option<ProviderKind>,
        cache: &DiskCache,
        force_refresh: bool,
    ) -> Result<FetchTimesResult, String> {
        let now = Timestamp::now();
        let location_key = profile.cache_key();
        let existing = cache.load_days(&location_key);

        if !force_refresh && !DiskCache::should_refresh(&existing, &profile.timezone, now) {
            return Ok(FetchTimesResult {
                cached: existing,
                stale: false,
                stale_reason: None,
            });
        }

        let tz = TimeZone::get(&profile.timezone).unwrap_or_else(|_| TimeZone::UTC);
        let local_today = now.to_zoned(tz).date().to_string();

        let order = provider_order(preferred);
        let mut errors = Vec::new();

        for provider in order {
            if self.is_cooling_down(provider) {
                errors.push(tr_fmt("%1: cooldown active", &[&provider.display_name()]));
                continue;
            }

            let res = match provider {
                ProviderKind::Imsakiyem => {
                    let Some(id) = profile.provider_ids.imsakiyem.as_deref() else {
                        continue;
                    };
                    let (from, to) = imsakiyem::default_range_for(now);
                    imsakiyem::fetch_prayer_times(&self.client, id, from, to, now.as_second()).await
                }
                ProviderKind::Mtopal => {
                    let Some(id) = profile.provider_ids.mtopal.as_deref() else {
                        continue;
                    };
                    diyanet_mirror::fetch_prayer_times(
                        &self.client,
                        MTOPAL_MIRROR,
                        id,
                        now.as_second(),
                    )
                    .await
                }
                ProviderKind::Ezanvakti => {
                    let resolved_id = match profile.provider_ids.ezanvakti.clone() {
                        Some(id) => Some(id),
                        None => self.match_mirror_id(EZANVAKTI_MIRROR, profile, cache).await,
                    };
                    let Some(id) = resolved_id else {
                        continue;
                    };
                    diyanet_mirror::fetch_prayer_times(
                        &self.client,
                        EZANVAKTI_MIRROR,
                        &id,
                        now.as_second(),
                    )
                    .await
                }
                ProviderKind::Aladhan => {
                    aladhan::fetch_prayer_times(&self.client, profile, now).await
                }
            };

            match res {
                Ok(days) if days.iter().any(|d| d.date == local_today) => {
                    let merged = cache.merge_days(
                        &location_key,
                        days,
                        provider,
                        now,
                        &profile.timezone,
                    );
                    return Ok(FetchTimesResult {
                        cached: merged,
                        stale: false,
                        stale_reason: None,
                    });
                }
                Ok(_) => {
                    errors.push(tr_fmt(
                        "%1: missing today's (%2) prayer schedule",
                        &[&provider.display_name(), &local_today],
                    ));
                }
                Err(ProviderError::RateLimited { retry_after_secs }) => {
                    self.record_rate_limit(provider, retry_after_secs);
                    errors.push(tr_fmt("%1: rate limit (429)", &[&provider.display_name()]));
                }
                Err(err) => {
                    errors.push(format!("{}: {err}", provider.display_name()));
                }
            }
        }

        // Fallback to cached days only if today's date actually exists in cache.
        if existing.days.contains_key(&local_today) {
            let reason = if errors.is_empty() {
                tr("Showing offline cache")
            } else {
                tr_fmt(
                    "Update failed (%1); showing cached schedule",
                    &[&errors.join("; ")],
                )
            };
            return Ok(FetchTimesResult {
                cached: existing,
                stale: true,
                stale_reason: Some(reason),
            });
        }

        if errors.is_empty() {
            Err(tr("No verified provider is available for the selected location."))
        } else {
            Err(tr_fmt("All sources failed: %1", &[&errors.join(" | ")]))
        }
    }
}

pub fn provider_order(preferred: Option<ProviderKind>) -> Vec<ProviderKind> {
    let mut order = match preferred {
        Some(p) => vec![p],
        None => Vec::new(),
    };
    for p in ProviderKind::ALL {
        if !order.contains(&p) {
            order.push(p);
        }
    }
    order
}

fn sort_countries(mut list: Vec<CountryOption>) -> Vec<CountryOption> {
    list.sort_by(|a, b| {
        let a_tr = a.iso2.eq_ignore_ascii_case("TR") || normalize_name(&a.name) == "turkiye";
        let b_tr = b.iso2.eq_ignore_ascii_case("TR") || normalize_name(&b.name) == "turkiye";
        b_tr.cmp(&a_tr).then_with(|| a.name.cmp(&b.name))
    });
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_order_respects_preferred() {
        assert_eq!(
            provider_order(Some(ProviderKind::Mtopal)),
            vec![
                ProviderKind::Mtopal,
                ProviderKind::Imsakiyem,
                ProviderKind::Ezanvakti,
                ProviderKind::Aladhan,
            ]
        );
        assert_eq!(
            provider_order(None),
            vec![
                ProviderKind::Imsakiyem,
                ProviderKind::Mtopal,
                ProviderKind::Ezanvakti,
                ProviderKind::Aladhan,
            ]
        );
    }

    #[test]
    fn sort_countries_pins_turkey_first() {
        let countries = vec![
            CountryOption {
                imsakiyem_id: Some("13".into()),
                mirror: None,
                iso2: "DE".into(),
                name: "Almanya".into(),
                name_en: "Germany".into(),
            },
            CountryOption {
                imsakiyem_id: Some("2".into()),
                mirror: None,
                iso2: "TR".into(),
                name: "Türkiye".into(),
                name_en: "Turkey".into(),
            },
        ];
        let sorted = sort_countries(countries);
        assert_eq!(sorted[0].iso2, "TR");
    }
}
