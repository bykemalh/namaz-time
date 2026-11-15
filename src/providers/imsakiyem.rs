use jiff::civil::Date;
use jiff::Timestamp;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::models::{DayTimes, LocationCandidate, ProviderIds, ProviderKind};
use crate::providers::{ProviderError, check_response_status};

const BASE_URL: &str = "https://ezanvakti.imsakiyem.com";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImsakiyemCountry {
    pub id: String,
    #[serde(default)]
    pub iso2: String,
    pub name: String,
    #[serde(default)]
    pub diyanet_name: Option<String>,
    #[serde(default)]
    pub names: Option<LocalizedNames>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalizedNames {
    #[serde(default)]
    pub tr: Option<String>,
    #[serde(default)]
    pub en: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImsakiyemState {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub name_en: Option<String>,
}

#[derive(Debug, Deserialize)]
struct V2Envelope<T> {
    success: bool,
    data: Option<T>,
    meta: Option<V2Meta>,
}

#[derive(Debug, Deserialize)]
struct V2Meta {
    pagination: Option<V2Pagination>,
}

#[derive(Debug, Deserialize)]
struct V2Pagination {
    pages: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct V1Envelope<T> {
    success: bool,
    data: Option<T>,
}

#[derive(Debug, Clone, Deserialize)]
struct V2DistrictDto {
    district_id: String,
    name: String,
    #[serde(default)]
    diyanet_name: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    names: Option<LocalizedNames>,
    #[serde(default)]
    state: Option<V2StateRef>,
    #[serde(default)]
    country: Option<V2CountryRef>,
    #[serde(default)]
    coordinates: Option<V2Coords>,
    #[serde(default)]
    timezone: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct V2StateRef {
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    names: Option<LocalizedNames>,
}

#[derive(Debug, Clone, Deserialize)]
struct V2CountryRef {
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(default)]
    iso2: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    names: Option<LocalizedNames>,
}

#[derive(Debug, Clone, Deserialize)]
struct V2Coords {
    lat: Option<f64>,
    lon: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct V2TimesPayload {
    #[serde(default)]
    times: Vec<V2TimeRow>,
}

#[derive(Debug, Deserialize)]
struct V2TimeRow {
    date: String,
    #[serde(default)]
    hijri: Option<V2Hijri>,
    imsak: String,
    gunes: String,
    ogle: String,
    ikindi: String,
    aksam: String,
    yatsi: String,
}

#[derive(Debug, Deserialize)]
struct V2Hijri {
    day: Option<u32>,
    month_name: Option<String>,
    year: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct V1TimeRow {
    date: String,
    #[serde(default)]
    hijri_date: Option<serde_json::Value>,
    times: V1TimesObj,
}

#[derive(Debug, Deserialize)]
struct V1TimesObj {
    imsak: String,
    gunes: String,
    ogle: String,
    ikindi: String,
    aksam: String,
    yatsi: String,
}

pub async fn fetch_countries(client: &Client) -> Result<Vec<ImsakiyemCountry>, ProviderError> {
    let url = format!("{BASE_URL}/api/v2/locations/countries");
    let resp = client.get(url).send().await?;
    let resp = check_response_status(resp).await?;
    let body: V2Envelope<Vec<ImsakiyemCountry>> = resp
        .json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("İmsakiyem ülke JSON: {e}")))?;
    if !body.success {
        return Err(ProviderError::InvalidResponse(
            "İmsakiyem ülkeler success=false".into(),
        ));
    }
    Ok(body.data.unwrap_or_default())
}

pub async fn fetch_states(
    client: &Client,
    country_id: &str,
) -> Result<Vec<ImsakiyemState>, ProviderError> {
    let url = format!("{BASE_URL}/api/locations/states");
    let resp = client
        .get(url)
        .query(&[("countryId", country_id)])
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    let body: V1Envelope<Vec<ImsakiyemState>> = resp
        .json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("İmsakiyem şehir JSON: {e}")))?;
    if !body.success {
        return Err(ProviderError::InvalidResponse(
            "İmsakiyem şehirler success=false".into(),
        ));
    }
    Ok(body.data.unwrap_or_default())
}

pub async fn fetch_districts(
    client: &Client,
    state_id: Option<&str>,
    country_id: Option<&str>,
) -> Result<Vec<LocationCandidate>, ProviderError> {
    let mut all = Vec::new();
    let mut page = 1u32;

    loop {
        let mut req = client.get(format!("{BASE_URL}/api/v2/locations/districts"));
        if let Some(sid) = state_id {
            req = req.query(&[("stateId", sid)]);
        } else if let Some(cid) = country_id {
            req = req.query(&[("countryId", cid)]);
        } else {
            return Ok(Vec::new());
        }
        let page_str = page.to_string();
        req = req.query(&[("page", page_str.as_str()), ("limit", "500")]);

        let resp = req.send().await?;
        let resp = check_response_status(resp).await?;
        let body: V2Envelope<Vec<V2DistrictDto>> = resp
            .json()
            .await
            .map_err(|e| ProviderError::InvalidResponse(format!("İmsakiyem ilçe JSON: {e}")))?;

        if !body.success {
            return Err(ProviderError::InvalidResponse(
                "İmsakiyem ilçeler success=false".into(),
            ));
        }

        let items = body.data.unwrap_or_default();
        all.extend(items.into_iter().map(dto_to_candidate));

        let total_pages = body
            .meta
            .and_then(|m| m.pagination)
            .and_then(|p| p.pages)
            .unwrap_or(1);
        if page >= total_pages || page >= 10 {
            break;
        }
        page += 1;
    }

    Ok(all)
}

pub async fn search_districts(
    client: &Client,
    query: &str,
) -> Result<Vec<LocationCandidate>, ProviderError> {
    let resp = client
        .get(format!("{BASE_URL}/api/v2/locations/search"))
        .query(&[("q", query), ("limit", "30")])
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    let body: V2Envelope<Vec<V2DistrictDto>> = resp
        .json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("İmsakiyem arama JSON: {e}")))?;

    if !body.success {
        return Err(ProviderError::InvalidResponse(
            "İmsakiyem arama success=false".into(),
        ));
    }

    Ok(body
        .data
        .unwrap_or_default()
        .into_iter()
        .map(dto_to_candidate)
        .collect())
}

pub async fn fetch_district_detail(
    client: &Client,
    district_id: &str,
) -> Result<LocationCandidate, ProviderError> {
    let url = format!("{BASE_URL}/api/v2/locations/districts/{district_id}");
    let resp = client.get(url).send().await?;
    let resp = check_response_status(resp).await?;
    let body: V2Envelope<V2DistrictDto> = resp
        .json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("İmsakiyem ilçe detay JSON: {e}")))?;
    if !body.success {
        return Err(ProviderError::InvalidResponse(
            "İmsakiyem ilçe detay success=false".into(),
        ));
    }
    body.data
        .map(dto_to_candidate)
        .ok_or_else(|| ProviderError::NotFound)
}

pub async fn fetch_prayer_times(
    client: &Client,
    district_id: &str,
    from_date: Date,
    to_date: Date,
    now_unix: i64,
) -> Result<Vec<DayTimes>, ProviderError> {
    let from_str = from_date.to_string();
    let to_str = to_date.to_string();

    let v2_resp = client
        .get(format!("{BASE_URL}/api/v2/times"))
        .query(&[
            ("district", district_id),
            ("from", from_str.as_str()),
            ("to", to_str.as_str()),
        ])
        .send()
        .await;

    match v2_resp {
        Ok(resp) => {
            let resp = check_response_status(resp).await?;
            let text = resp
                .text()
                .await
                .map_err(|e| ProviderError::Network(e.to_string()))?;
            if let Ok(days) = parse_v2_times_json(&text, now_unix) {
                if !days.is_empty() {
                    return Ok(days);
                }
            }
            // Fallback to v1 monthly if v2 shape fails unexpectedly
            fetch_v1_monthly(client, district_id, now_unix).await
        }
        Err(_) => fetch_v1_monthly(client, district_id, now_unix).await,
    }
}

async fn fetch_v1_monthly(
    client: &Client,
    district_id: &str,
    now_unix: i64,
) -> Result<Vec<DayTimes>, ProviderError> {
    let url = format!("{BASE_URL}/api/prayer-times/{district_id}/monthly");
    let resp = client.get(url).send().await?;
    let resp = check_response_status(resp).await?;
    let text = resp
        .text()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    parse_v1_times_json(&text, now_unix)
}

pub(crate) fn parse_v2_times_json(raw: &str, now_unix: i64) -> Result<Vec<DayTimes>, ProviderError> {
    let env: V2Envelope<V2TimesPayload> = serde_json::from_str(raw)
        .map_err(|e| ProviderError::InvalidResponse(format!("İmsakiyem v2 vakit JSON: {e}")))?;
    if !env.success {
        return Err(ProviderError::InvalidResponse(
            "İmsakiyem v2 success=false".into(),
        ));
    }
    let payload = env
        .data
        .ok_or_else(|| ProviderError::InvalidResponse("İmsakiyem v2 data boş".into()))?;

    let mut out = Vec::with_capacity(payload.times.len());
    for row in payload.times {
        let hijri = row.hijri.and_then(|h| match (h.day, h.month_name, h.year) {
            (Some(d), Some(m), Some(y)) => Some(format!("{d} {m} {y}")),
            _ => None,
        });
        let day = DayTimes {
            date: row.date.chars().take(10).collect(),
            hijri,
            imsak: row.imsak,
            gunes: row.gunes,
            ogle: row.ogle,
            ikindi: row.ikindi,
            aksam: row.aksam,
            yatsi: row.yatsi,
            source: ProviderKind::Imsakiyem,
            fetched_at_unix: now_unix,
        };
        day.validate().map_err(ProviderError::InvalidResponse)?;
        out.push(day);
    }
    Ok(out)
}

pub(crate) fn parse_v1_times_json(raw: &str, now_unix: i64) -> Result<Vec<DayTimes>, ProviderError> {
    let env: V1Envelope<Vec<V1TimeRow>> = serde_json::from_str(raw)
        .map_err(|e| ProviderError::InvalidResponse(format!("İmsakiyem v1 vakit JSON: {e}")))?;
    if !env.success {
        return Err(ProviderError::InvalidResponse(
            "İmsakiyem v1 success=false".into(),
        ));
    }
    let rows = env
        .data
        .ok_or_else(|| ProviderError::InvalidResponse("İmsakiyem v1 data boş".into()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let hijri = row.hijri_date.and_then(|v| match v {
            serde_json::Value::String(s) => Some(s),
            serde_json::Value::Object(map) => {
                map.get("full_date")
                    .or_else(|| map.get("hijri"))
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            }
            _ => None,
        });
        let day = DayTimes {
            date: row.date.chars().take(10).collect(),
            hijri,
            imsak: row.times.imsak,
            gunes: row.times.gunes,
            ogle: row.times.ogle,
            ikindi: row.times.ikindi,
            aksam: row.times.aksam,
            yatsi: row.times.yatsi,
            source: ProviderKind::Imsakiyem,
            fetched_at_unix: now_unix,
        };
        day.validate().map_err(ProviderError::InvalidResponse)?;
        out.push(day);
    }
    Ok(out)
}

fn dto_to_candidate(dto: V2DistrictDto) -> LocationCandidate {
    let district_name = dto
        .names
        .as_ref()
        .and_then(|n| n.tr.clone())
        .unwrap_or_else(|| dto.name.clone());

    let city_name = dto
        .state
        .as_ref()
        .map(|s| {
            s.names
                .as_ref()
                .and_then(|n| n.tr.clone())
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| s.name.clone())
        })
        .unwrap_or_else(|| district_name.clone());

    let (country_code, country_name) = dto
        .country
        .as_ref()
        .map(|c| {
            let cname = c
                .names
                .as_ref()
                .and_then(|n| n.tr.clone())
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| c.name.clone());
            (c.iso2.clone(), cname)
        })
        .unwrap_or_default();

    let display_label = dto
        .label
        .unwrap_or_else(|| format!("{district_name}, {city_name}, {country_name}"));

    let _ = dto.diyanet_name;

    LocationCandidate {
        country_code,
        country_name,
        city_name,
        district_name,
        display_label,
        timezone: dto.timezone,
        lat: dto.coordinates.as_ref().and_then(|c| c.lat),
        lon: dto.coordinates.as_ref().and_then(|c| c.lon),
        provider_ids: ProviderIds {
            imsakiyem: Some(dto.district_id),
            mtopal: None,
            idamedya: None,
        },
    }
}

pub fn default_range_for(now: Timestamp) -> (Date, Date) {
    let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
    let from = today
        .yesterday()
        .unwrap_or(today);
    let to = today
        .checked_add(jiff::Span::new().days(35))
        .unwrap_or(today);
    (from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_v2_times_response() {
        let raw = r#"{"success":true,"data":{"location":{"district_id":"9220","name":"Polatlı","timezone":"Europe/Istanbul"},"times":[{"date":"2026-11-15","hijri":{"day":6,"month":6,"month_name":"Cemaziyelahir","year":1448},"imsak":"06:02","gunes":"07:28","ogle":"12:41","ikindi":"15:20","aksam":"17:44","yatsi":"19:05","flags":[]}]},"meta":{"version":"2"}}"#;
        let days = parse_v2_times_json(raw, 1000).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-11-15");
        assert_eq!(days[0].imsak, "06:02");
        assert_eq!(days[0].hijri.as_deref(), Some("6 Cemaziyelahir 1448"));
        assert_eq!(days[0].source, ProviderKind::Imsakiyem);
    }

    #[test]
    fn parses_v1_monthly_iso_date() {
        let raw = r#"{"success":true,"code":200,"data":[{"date":"2026-11-02T00:00:00.000Z","times":{"imsak":"05:47","gunes":"07:13","ogle":"12:38","ikindi":"15:27","aksam":"17:52","yatsi":"19:12"}}]}"#;
        let days = parse_v1_times_json(raw, 1000).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-11-02");
        assert_eq!(days[0].yatsi, "19:12");
    }
}
