pub mod idamedya;
pub mod imsakiyem;
pub mod mtopal;

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jiff::tz::TimeZone;
use jiff::Timestamp;
use reqwest::{Client, Response, StatusCode};
use tracing::{info, warn};

use crate::models::location::{
    expand_name_aliases, fallback_timezone_for_country, match_unique_by_name, normalize_name,
    system_timezone_iana,
};
use crate::models::{LocationCandidate, LocationProfile, ProviderIds, ProviderKind};
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
            } => write!(f, "İstek limiti aşıldı ({s} sn bekleme)"),
            Self::RateLimited {
                retry_after_secs: None,
            } => write!(f, "İstek limiti aşıldı (429)"),
            Self::NotFound => write!(f, "Konum bulunamadı (404)"),
            Self::ServerError(code) => write!(f, "Sunucu hatası (HTTP {code})"),
            Self::Network(msg) => write!(f, "Bağlantı hatası: {msg}"),
            Self::InvalidResponse(msg) => write!(f, "Geçersiz yanıt: {msg}"),
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
    pub mtopal_id: Option<String>,
    pub iso2: String,
    pub name: String,
    pub name_en: String,
}

#[derive(Debug, Clone)]
pub struct StateOption {
    pub imsakiyem_id: Option<String>,
    pub mtopal_id: Option<String>,
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

    /// Loads global country list (Imsakiyem v2 primary, mtopal fallback) with 7-day disk cache.
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
                        mtopal_id: None,
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
                                mtopal_id: None,
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

        // Fallback to mtopal countries
        let mtopal_list = self.get_mtopal_countries(cache).await?;
        Ok(sort_countries(
            mtopal_list
                .into_iter()
                .map(|c| CountryOption {
                    imsakiyem_id: None,
                    mtopal_id: Some(c.ulke_id),
                    iso2: String::new(),
                    name: c.ulke_adi,
                    name_en: c.ulke_adi_en,
                })
                .collect(),
        ))
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
                        mtopal_id: None,
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
                                mtopal_id: None,
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

        if let Some(mid) = &country.mtopal_id {
            let cities = self.get_mtopal_cities(mid, cache).await?;
            return Ok(cities
                .into_iter()
                .map(|c| StateOption {
                    imsakiyem_id: None,
                    mtopal_id: Some(c.sehir_id),
                    name: c.sehir_adi,
                    name_en: c.sehir_adi_en,
                })
                .collect());
        }

        Err("Şehir listesi alınamadı".to_string())
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

        if let Some(mid) = &state.mtopal_id {
            let districts = self.get_mtopal_districts(mid, cache).await?;
            let tz = fallback_timezone_for_country(&country.iso2, &country.name)
                .map(str::to_string);
            return Ok(districts
                .into_iter()
                .map(|d| LocationCandidate {
                    country_code: country.iso2.clone(),
                    country_name: country.name.clone(),
                    city_name: state.name.clone(),
                    district_name: d.ilce_adi.clone(),
                    display_label: format!("{}, {}, {}", d.ilce_adi, state.name, country.name),
                    timezone: tz.clone(),
                    lat: None,
                    lon: None,
                    provider_ids: ProviderIds {
                        imsakiyem: None,
                        mtopal: Some(d.ilce_id),
                        idamedya: None,
                    },
                })
                .collect());
        }

        Err("İlçe listesi alınamadı".to_string())
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

        // Offline/fallback search across Turkish provinces from Idamedya if Turkey query
        let q_norm = normalize_name(trimmed);
        if let Ok(provinces) = self.get_idamedya_provinces(cache).await {
            let matched: Vec<LocationCandidate> = provinces
                .into_iter()
                .filter(|p| normalize_name(&p.ad).contains(&q_norm))
                .map(|p| LocationCandidate {
                    country_code: "TR".to_string(),
                    country_name: "Türkiye".to_string(),
                    city_name: p.ad.clone(),
                    district_name: p.ad.clone(),
                    display_label: format!("{}, Türkiye", p.ad),
                    timezone: Some("Europe/Istanbul".to_string()),
                    lat: None,
                    lon: None,
                    provider_ids: ProviderIds {
                        imsakiyem: None,
                        mtopal: None,
                        idamedya: Some(p.ilce.to_string()),
                    },
                })
                .collect();
            if !matched.is_empty() {
                return Ok(matched);
            }
        }

        Err("Konum araması şu an yapılamadı".to_string())
    }

    /// Ensures the LocationProfile has accurate IANA timezone and resolves verified matching
    /// IDs across the other providers without blindly assuming identical numeric IDs.
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

        // Enrich with Imsakiyem detail if we have imsakiyem_id but missing timezone
        if timezone.is_none() {
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

        // If selected from mtopal while Imsakiyem was down, try matching in Imsakiyem search
        if ids.imsakiyem.is_none() && !self.is_cooling_down(ProviderKind::Imsakiyem) {
            if let Ok(results) =
                imsakiyem::search_districts(&self.client, &candidate.district_name).await
            {
                let d_targets = expand_name_aliases(&candidate.district_name, Some(&candidate.city_name));
                let c_targets = expand_name_aliases(&candidate.country_name, None);
                let matched = results.into_iter().filter(|r| {
                    let r_c = expand_name_aliases(&r.country_name, None);
                    let r_d = expand_name_aliases(&r.district_name, Some(&r.city_name));
                    c_targets.iter().any(|x| r_c.contains(x))
                        && d_targets.iter().any(|x| r_d.contains(x))
                }).collect::<Vec<_>>();
                if matched.len() == 1 {
                    ids.imsakiyem = matched[0].provider_ids.imsakiyem.clone();
                    if timezone.is_none() {
                        timezone = matched[0].timezone.clone();
                    }
                    if country_code.is_empty() {
                        country_code = matched[0].country_code.clone();
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

        // Resolve mtopal ID by verified name matching
        if profile.provider_ids.mtopal.is_none() {
            profile.provider_ids.mtopal = self.match_mtopal_id(&profile, cache).await;
        }

        // Resolve Idamedya ID only for Turkey
        if profile.is_turkey() && profile.provider_ids.idamedya.is_none() {
            profile.provider_ids.idamedya = self.match_idamedya_id(&profile, cache).await;
        }

        info!(
            "Konum eşleştirildi: {} -> tz={}, imsakiyem={:?}, mtopal={:?}, idamedya={:?}",
            profile.display_label,
            profile.timezone,
            profile.provider_ids.imsakiyem,
            profile.provider_ids.mtopal,
            profile.provider_ids.idamedya
        );

        profile
    }

    async fn match_mtopal_id(&self, profile: &LocationProfile, cache: &DiskCache) -> Option<String> {
        let countries = self.get_mtopal_countries(cache).await.ok()?;
        let c_targets = expand_name_aliases(&profile.country_name, None);
        let country = match_unique_by_name(&c_targets, &countries, |c| {
            let mut v = expand_name_aliases(&c.ulke_adi, None);
            if !c.ulke_adi_en.is_empty() {
                v.extend(expand_name_aliases(&c.ulke_adi_en, None));
            }
            v
        })?;

        let cities = self.get_mtopal_cities(&country.ulke_id, cache).await.ok()?;
        let s_targets = expand_name_aliases(&profile.city_name, None);
        let city = match_unique_by_name(&s_targets, &cities, |s| {
            let mut v = expand_name_aliases(&s.sehir_adi, None);
            if !s.sehir_adi_en.is_empty() {
                v.extend(expand_name_aliases(&s.sehir_adi_en, None));
            }
            v
        })?;

        let districts = self.get_mtopal_districts(&city.sehir_id, cache).await.ok()?;
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

    async fn match_idamedya_id(
        &self,
        profile: &LocationProfile,
        cache: &DiskCache,
    ) -> Option<String> {
        let provinces = self.get_idamedya_provinces(cache).await.ok()?;
        let s_targets = expand_name_aliases(&profile.city_name, None);
        let province = match_unique_by_name(&s_targets, &provinces, |p| {
            expand_name_aliases(&p.ad, None)
        })?;

        let districts = self
            .get_idamedya_districts(province.sehir, cache)
            .await
            .ok()?;
        let d_targets = expand_name_aliases(&profile.district_name, Some(&profile.city_name));
        let district = match_unique_by_name(&d_targets, &districts, |d| {
            expand_name_aliases(&d.ad, Some(&province.ad))
        })?;

        Some(district.ilce.to_string())
    }

    async fn get_mtopal_countries(
        &self,
        cache: &DiskCache,
    ) -> Result<Vec<mtopal::MtopalCountry>, String> {
        let now_unix = Timestamp::now().as_second();
        if let Some(cached) =
            cache.load_location_list::<Vec<mtopal::MtopalCountry>>("mtopal_countries", now_unix, false)
        {
            return Ok(cached);
        }
        if self.is_cooling_down(ProviderKind::Mtopal) {
            return cache
                .load_location_list("mtopal_countries", now_unix, true)
                .ok_or_else(|| "mtopal limit bekliyor".to_string());
        }
        match mtopal::fetch_countries(&self.client).await {
            Ok(list) => {
                cache.save_location_list("mtopal_countries", &list, now_unix);
                Ok(list)
            }
            Err(ProviderError::RateLimited { retry_after_secs }) => {
                self.record_rate_limit(ProviderKind::Mtopal, retry_after_secs);
                cache
                    .load_location_list("mtopal_countries", now_unix, true)
                    .ok_or_else(|| "mtopal limit aşıldı".to_string())
            }
            Err(err) => cache
                .load_location_list("mtopal_countries", now_unix, true)
                .ok_or_else(|| err.to_string()),
        }
    }

    async fn get_mtopal_cities(
        &self,
        country_id: &str,
        cache: &DiskCache,
    ) -> Result<Vec<mtopal::MtopalCity>, String> {
        let now_unix = Timestamp::now().as_second();
        let key = format!("mtopal_cities_{country_id}");
        if let Some(cached) = cache.load_location_list::<Vec<mtopal::MtopalCity>>(&key, now_unix, false) {
            return Ok(cached);
        }
        if self.is_cooling_down(ProviderKind::Mtopal) {
            return cache
                .load_location_list(&key, now_unix, true)
                .ok_or_else(|| "mtopal limit bekliyor".to_string());
        }
        match mtopal::fetch_cities(&self.client, country_id).await {
            Ok(list) => {
                cache.save_location_list(&key, &list, now_unix);
                Ok(list)
            }
            Err(ProviderError::RateLimited { retry_after_secs }) => {
                self.record_rate_limit(ProviderKind::Mtopal, retry_after_secs);
                cache
                    .load_location_list(&key, now_unix, true)
                    .ok_or_else(|| "mtopal limit aşıldı".to_string())
            }
            Err(err) => cache
                .load_location_list(&key, now_unix, true)
                .ok_or_else(|| err.to_string()),
        }
    }

    async fn get_mtopal_districts(
        &self,
        city_id: &str,
        cache: &DiskCache,
    ) -> Result<Vec<mtopal::MtopalDistrict>, String> {
        let now_unix = Timestamp::now().as_second();
        let key = format!("mtopal_districts_{city_id}");
        if let Some(cached) =
            cache.load_location_list::<Vec<mtopal::MtopalDistrict>>(&key, now_unix, false)
        {
            return Ok(cached);
        }
        if self.is_cooling_down(ProviderKind::Mtopal) {
            return cache
                .load_location_list(&key, now_unix, true)
                .ok_or_else(|| "mtopal limit bekliyor".to_string());
        }
        match mtopal::fetch_districts(&self.client, city_id).await {
            Ok(list) => {
                cache.save_location_list(&key, &list, now_unix);
                Ok(list)
            }
            Err(ProviderError::RateLimited { retry_after_secs }) => {
                self.record_rate_limit(ProviderKind::Mtopal, retry_after_secs);
                cache
                    .load_location_list(&key, now_unix, true)
                    .ok_or_else(|| "mtopal limit aşıldı".to_string())
            }
            Err(err) => cache
                .load_location_list(&key, now_unix, true)
                .ok_or_else(|| err.to_string()),
        }
    }

    async fn get_idamedya_provinces(
        &self,
        cache: &DiskCache,
    ) -> Result<Vec<idamedya::IdamedyaProvince>, String> {
        let now_unix = Timestamp::now().as_second();
        if let Some(cached) = cache
            .load_location_list::<Vec<idamedya::IdamedyaProvince>>("idamedya_iller", now_unix, false)
        {
            return Ok(cached);
        }
        match idamedya::fetch_provinces(&self.client).await {
            Ok(list) => {
                cache.save_location_list("idamedya_iller", &list, now_unix);
                Ok(list)
            }
            Err(err) => cache
                .load_location_list("idamedya_iller", now_unix, true)
                .ok_or_else(|| err.to_string()),
        }
    }

    async fn get_idamedya_districts(
        &self,
        sehir_id: u64,
        cache: &DiskCache,
    ) -> Result<Vec<idamedya::IdamedyaDistrict>, String> {
        let now_unix = Timestamp::now().as_second();
        let key = format!("idamedya_ilceler_{sehir_id}");
        if let Some(cached) =
            cache.load_location_list::<Vec<idamedya::IdamedyaDistrict>>(&key, now_unix, false)
        {
            return Ok(cached);
        }
        match idamedya::fetch_districts(&self.client, sehir_id).await {
            Ok(list) => {
                cache.save_location_list(&key, &list, now_unix);
                Ok(list)
            }
            Err(err) => cache
                .load_location_list(&key, now_unix, true)
                .ok_or_else(|| err.to_string()),
        }
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
            let id_opt = match provider {
                ProviderKind::Imsakiyem => profile.provider_ids.imsakiyem.as_deref(),
                ProviderKind::Mtopal => profile.provider_ids.mtopal.as_deref(),
                ProviderKind::Idamedya => {
                    if profile.is_turkey() {
                        profile.provider_ids.idamedya.as_deref()
                    } else {
                        None
                    }
                }
            };

            let Some(id) = id_opt else { continue };
            if self.is_cooling_down(provider) {
                errors.push(format!("{}: bekleme süresinde", provider.display_name()));
                continue;
            }

            let res = match provider {
                ProviderKind::Imsakiyem => {
                    let (from, to) = imsakiyem::default_range_for(now);
                    imsakiyem::fetch_prayer_times(&self.client, id, from, to, now.as_second()).await
                }
                ProviderKind::Mtopal => {
                    mtopal::fetch_prayer_times(&self.client, id, now.as_second()).await
                }
                ProviderKind::Idamedya => {
                    idamedya::fetch_prayer_times(&self.client, id, now.as_second()).await
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
                    errors.push(format!(
                        "{}: bugünün ({local_today}) vakti yanıtta yok",
                        provider.display_name()
                    ));
                }
                Err(ProviderError::RateLimited { retry_after_secs }) => {
                    self.record_rate_limit(provider, retry_after_secs);
                    errors.push(format!("{}: 429 limit", provider.display_name()));
                }
                Err(err) => {
                    errors.push(format!("{}: {err}", provider.display_name()));
                }
            }
        }

        // Fallback to cached days only if today's date actually exists in cache.
        if existing.days.contains_key(&local_today) {
            let reason = if errors.is_empty() {
                "Çevrimdışı önbellek kullanılıyor".to_string()
            } else {
                format!("Güncellenemedi ({}), önbellek gösteriliyor", errors.join("; "))
            };
            return Ok(FetchTimesResult {
                cached: existing,
                stale: true,
                stale_reason: Some(reason),
            });
        }

        if errors.is_empty() {
            Err("Seçili konum için doğrulanmış bir veri sağlayıcısı bulunamadı.".to_string())
        } else {
            Err(format!("Tüm kaynaklar başarısız: {}", errors.join(" | ")))
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
            vec![ProviderKind::Mtopal, ProviderKind::Imsakiyem, ProviderKind::Idamedya]
        );
        assert_eq!(
            provider_order(None),
            vec![ProviderKind::Imsakiyem, ProviderKind::Mtopal, ProviderKind::Idamedya]
        );
    }

    #[test]
    fn sort_countries_pins_turkey_first() {
        let countries = vec![
            CountryOption {
                imsakiyem_id: Some("13".into()),
                mtopal_id: None,
                iso2: "DE".into(),
                name: "Almanya".into(),
                name_en: "Germany".into(),
            },
            CountryOption {
                imsakiyem_id: Some("2".into()),
                mtopal_id: None,
                iso2: "TR".into(),
                name: "Türkiye".into(),
                name_en: "Turkey".into(),
            },
        ];
        let sorted = sort_countries(countries);
        assert_eq!(sorted[0].iso2, "TR");
    }
}
