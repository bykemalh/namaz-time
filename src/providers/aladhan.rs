use jiff::tz::TimeZone;
use jiff::Timestamp;
use reqwest::Client;
use serde::Deserialize;

use crate::i18n::{AppLang, active_lang, tr, tr_fmt};
use crate::models::{DayTimes, LocationProfile, ProviderKind};
use crate::providers::{ProviderError, check_response_status};

const BASE_URL: &str = "https://api.aladhan.com/v1";
const DIYANET_METHOD: &str = "13";

#[derive(Debug, Deserialize)]
struct AladhanEnvelope {
    code: u16,
    #[serde(default)]
    data: Vec<AladhanDay>,
}

#[derive(Debug, Deserialize)]
struct AladhanDay {
    timings: AladhanTimings,
    date: AladhanDate,
}

#[derive(Debug, Deserialize)]
struct AladhanTimings {
    #[serde(rename = "Fajr")]
    fajr: String,
    #[serde(rename = "Sunrise")]
    sunrise: String,
    #[serde(rename = "Dhuhr")]
    dhuhr: String,
    #[serde(rename = "Asr")]
    asr: String,
    #[serde(rename = "Maghrib")]
    maghrib: String,
    #[serde(rename = "Isha")]
    isha: String,
}

#[derive(Debug, Deserialize)]
struct AladhanDate {
    gregorian: AladhanGregorian,
    #[serde(default)]
    hijri: Option<AladhanHijri>,
}

#[derive(Debug, Deserialize)]
struct AladhanGregorian {
    date: String,
}

#[derive(Debug, Deserialize)]
struct AladhanHijri {
    #[serde(default)]
    day: Option<String>,
    #[serde(default)]
    month: Option<AladhanHijriMonth>,
    #[serde(default)]
    year: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AladhanHijriMonth {
    #[serde(default)]
    number: Option<u8>,
    #[serde(default)]
    en: Option<String>,
}

pub async fn fetch_prayer_times(
    client: &Client,
    profile: &LocationProfile,
    now: Timestamp,
) -> Result<Vec<DayTimes>, ProviderError> {
    let tz = TimeZone::get(&profile.timezone).unwrap_or_else(|_| TimeZone::UTC);
    let local_date = now.to_zoned(tz).date();
    let mut all_days = fetch_month(client, profile, local_date.year(), local_date.month(), now.as_second()).await?;

    // Fetch next month too near month-end so tomorrow's Fajr countdown always resolves.
    if local_date.day() >= 25 {
        let (next_year, next_month) = if local_date.month() == 12 {
            (local_date.year() + 1, 1)
        } else {
            (local_date.year(), local_date.month() + 1)
        };
        if let Ok(mut next_days) =
            fetch_month(client, profile, next_year, next_month, now.as_second()).await
        {
            all_days.append(&mut next_days);
        }
    }

    Ok(all_days)
}

async fn fetch_month(
    client: &Client,
    profile: &LocationProfile,
    year: i16,
    month: i8,
    now_unix: i64,
) -> Result<Vec<DayTimes>, ProviderError> {
    let resp = match (profile.lat, profile.lon) {
        (Some(lat), Some(lon)) => {
            let lat_s = lat.to_string();
            let lon_s = lon.to_string();
            client
                .get(format!("{BASE_URL}/calendar/{year}/{month}"))
                .query(&[
                    ("latitude", lat_s.as_str()),
                    ("longitude", lon_s.as_str()),
                    ("method", DIYANET_METHOD),
                ])
                .send()
                .await?
        }
        _ => {
            let city = if profile.district_name.is_empty() {
                profile.city_name.as_str()
            } else {
                profile.district_name.as_str()
            };
            let country = if !profile.country_code.is_empty() {
                profile.country_code.as_str()
            } else {
                profile.country_name.as_str()
            };
            client
                .get(format!("{BASE_URL}/calendarByCity/{year}/{month}"))
                .query(&[
                    ("city", city),
                    ("country", country),
                    ("method", DIYANET_METHOD),
                ])
                .send()
                .await?
        }
    };

    let resp = check_response_status(resp).await?;
    let raw = resp
        .text()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    parse_calendar_json(&raw, now_unix)
}

pub(crate) fn parse_calendar_json(raw: &str, now_unix: i64) -> Result<Vec<DayTimes>, ProviderError> {
    let env: AladhanEnvelope = serde_json::from_str(raw).map_err(|e| {
        ProviderError::InvalidResponse(tr_fmt("AlAdhan times JSON: %1", &[&e.to_string()]))
    })?;
    if env.code != 200 {
        return Err(ProviderError::InvalidResponse(tr(
            "AlAdhan returned non-200 status",
        )));
    }

    let mut out = Vec::with_capacity(env.data.len());
    for row in env.data {
        let iso_date = ddmmyyyy_dash_to_iso(&row.date.gregorian.date).ok_or_else(|| {
            ProviderError::InvalidResponse(tr_fmt(
                "AlAdhan invalid date format: %1",
                &[&row.date.gregorian.date],
            ))
        })?;
        let hijri = row.date.hijri.and_then(|h| format_hijri(&h, active_lang()));
        // For Diyanet method=13, Fajr corresponds to Imsak (AlAdhan's "Imsak" field is a -10m buffer).
        let day = DayTimes {
            date: iso_date,
            hijri,
            imsak: strip_tz_suffix(&row.timings.fajr),
            gunes: strip_tz_suffix(&row.timings.sunrise),
            ogle: strip_tz_suffix(&row.timings.dhuhr),
            ikindi: strip_tz_suffix(&row.timings.asr),
            aksam: strip_tz_suffix(&row.timings.maghrib),
            yatsi: strip_tz_suffix(&row.timings.isha),
            source: ProviderKind::Aladhan,
            fetched_at_unix: now_unix,
        };
        day.validate().map_err(ProviderError::InvalidResponse)?;
        out.push(day);
    }

    Ok(out)
}

fn strip_tz_suffix(raw: &str) -> String {
    raw.split_whitespace().next().unwrap_or(raw).trim().to_string()
}

fn ddmmyyyy_dash_to_iso(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw.trim().split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let (dd, mm, yyyy) = (parts[0], parts[1], parts[2]);
    if dd.len() != 2 || mm.len() != 2 || yyyy.len() != 4 {
        return None;
    }
    Some(format!("{yyyy}-{mm}-{dd}"))
}

fn format_hijri(h: &AladhanHijri, lang: AppLang) -> Option<String> {
    let day = h.day.as_deref()?.trim();
    let year = h.year.as_deref()?.trim();
    let month_info = h.month.as_ref()?;
    let month_name = month_info
        .number
        .and_then(|n| localized_hijri_month(n, lang))
        .or(month_info.en.as_deref())?;
    Some(format!("{day} {month_name} {year}"))
}

fn localized_hijri_month(month: u8, lang: AppLang) -> Option<&'static str> {
    match lang {
        AppLang::Tr => match month {
            1 => Some("Muharrem"),
            2 => Some("Safer"),
            3 => Some("Rebiülevvel"),
            4 => Some("Rebiülahir"),
            5 => Some("Cemaziyelevvel"),
            6 => Some("Cemaziyelahir"),
            7 => Some("Recep"),
            8 => Some("Şaban"),
            9 => Some("Ramazan"),
            10 => Some("Şevval"),
            11 => Some("Zilkade"),
            12 => Some("Zilhicce"),
            _ => None,
        },
        AppLang::Ru => match month {
            1 => Some("Мухаррам"),
            2 => Some("Сафар"),
            3 => Some("Раби аль-авваль"),
            4 => Some("Раби ас-сани"),
            5 => Some("Джумада аль-уля"),
            6 => Some("Джумада аль-ахира"),
            7 => Some("Раджаб"),
            8 => Some("Шаабан"),
            9 => Some("Рамадан"),
            10 => Some("Шавваль"),
            11 => Some("Зуль-каада"),
            12 => Some("Зуль-хиджа"),
            _ => None,
        },
        AppLang::En => match month {
            1 => Some("Muharram"),
            2 => Some("Safar"),
            3 => Some("Rabi al-Awwal"),
            4 => Some("Rabi al-Thani"),
            5 => Some("Jumada al-Ula"),
            6 => Some("Jumada al-Akhirah"),
            7 => Some("Rajab"),
            8 => Some("Shaban"),
            9 => Some("Ramadan"),
            10 => Some("Shawwal"),
            11 => Some("Dhu al-Qadah"),
            12 => Some("Dhu al-Hijjah"),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_aladhan_calendar_response() {
        let raw = r#"{"code":200,"status":"OK","data":[{"timings":{"Fajr":"06:20 (+03)","Sunrise":"07:49 (+03)","Dhuhr":"12:54 (+03)","Asr":"15:27 (+03)","Maghrib":"17:49 (+03)","Isha":"19:13 (+03)","Imsak":"06:10 (+03)"},"date":{"gregorian":{"date":"19-11-2026"},"hijri":{"day":"9","month":{"number":6,"en":"Jumādá al-ākhirah"},"year":"1448"}}}]}"#;
        let days = parse_calendar_json(raw, 1000).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-11-19");
        assert_eq!(days[0].imsak, "06:20");
        assert_eq!(days[0].yatsi, "19:13");
        assert_eq!(days[0].source, ProviderKind::Aladhan);
        assert!(days[0].hijri.as_deref().unwrap().contains("1448"));
    }
}
