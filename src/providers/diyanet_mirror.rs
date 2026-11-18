use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::i18n::tr_fmt;
use crate::models::{DayTimes, ProviderKind};
use crate::providers::{ProviderError, check_response_status};

#[derive(Debug, Clone, Copy)]
pub struct DiyanetMirror {
    pub kind: ProviderKind,
    pub base_url: &'static str,
    pub label: &'static str,
    pub cache_prefix: &'static str,
}

pub const MTOPAL_MIRROR: DiyanetMirror = DiyanetMirror {
    kind: ProviderKind::Mtopal,
    base_url: "https://namazvakti.mtopal.dev",
    label: "mtopal",
    cache_prefix: "mtopal",
};

pub const EZANVAKTI_MIRROR: DiyanetMirror = DiyanetMirror {
    kind: ProviderKind::Ezanvakti,
    base_url: "https://ezanvakti.emushaf.net",
    label: "EzanVakti",
    cache_prefix: "ezanvakti",
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorCountry {
    #[serde(rename = "UlkeAdi")]
    pub ulke_adi: String,
    #[serde(rename = "UlkeAdiEn", default)]
    pub ulke_adi_en: String,
    #[serde(rename = "UlkeID")]
    pub ulke_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorCity {
    #[serde(rename = "SehirAdi")]
    pub sehir_adi: String,
    #[serde(rename = "SehirAdiEn", default)]
    pub sehir_adi_en: String,
    #[serde(rename = "SehirID")]
    pub sehir_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorDistrict {
    #[serde(rename = "IlceAdi")]
    pub ilce_adi: String,
    #[serde(rename = "IlceAdiEn", default)]
    pub ilce_adi_en: String,
    #[serde(rename = "IlceID")]
    pub ilce_id: String,
}

#[derive(Debug, Deserialize)]
struct MirrorPrayerRow {
    #[serde(rename = "Imsak")]
    imsak: String,
    #[serde(rename = "Gunes")]
    gunes: String,
    #[serde(rename = "Ogle")]
    ogle: String,
    #[serde(rename = "Ikindi")]
    ikindi: String,
    #[serde(rename = "Aksam")]
    aksam: String,
    #[serde(rename = "Yatsi")]
    yatsi: String,
    #[serde(rename = "MiladiTarihKisa")]
    miladi_tarih_kisa: String,
    #[serde(rename = "HicriTarihUzun", default)]
    hicri_tarih_uzun: Option<String>,
    #[serde(rename = "HicriTarihKisa", default)]
    hicri_tarih_kisa: Option<String>,
}

pub async fn fetch_countries(
    client: &Client,
    mirror: DiyanetMirror,
) -> Result<Vec<MirrorCountry>, ProviderError> {
    let resp = client.get(format!("{}/ulkeler", mirror.base_url)).send().await?;
    let resp = check_response_status(resp).await?;
    resp.json()
        .await
        .map_err(|e| {
            ProviderError::InvalidResponse(tr_fmt(
                "%1 countries JSON: %2",
                &[mirror.label, &e.to_string()],
            ))
        })
}

pub async fn fetch_cities(
    client: &Client,
    mirror: DiyanetMirror,
    country_id: &str,
) -> Result<Vec<MirrorCity>, ProviderError> {
    let resp = client
        .get(format!("{}/sehirler/{country_id}", mirror.base_url))
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    resp.json()
        .await
        .map_err(|e| {
            ProviderError::InvalidResponse(tr_fmt(
                "%1 cities JSON: %2",
                &[mirror.label, &e.to_string()],
            ))
        })
}

pub async fn fetch_districts(
    client: &Client,
    mirror: DiyanetMirror,
    city_id: &str,
) -> Result<Vec<MirrorDistrict>, ProviderError> {
    let resp = client
        .get(format!("{}/ilceler/{city_id}", mirror.base_url))
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    resp.json()
        .await
        .map_err(|e| {
            ProviderError::InvalidResponse(tr_fmt(
                "%1 districts JSON: %2",
                &[mirror.label, &e.to_string()],
            ))
        })
}

pub async fn fetch_prayer_times(
    client: &Client,
    mirror: DiyanetMirror,
    district_id: &str,
    now_unix: i64,
) -> Result<Vec<DayTimes>, ProviderError> {
    let resp = client
        .get(format!("{}/vakitler/{district_id}", mirror.base_url))
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    let raw = resp
        .text()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    parse_times_json(&raw, mirror, now_unix)
}

pub(crate) fn parse_times_json(
    raw: &str,
    mirror: DiyanetMirror,
    now_unix: i64,
) -> Result<Vec<DayTimes>, ProviderError> {
    let rows: Vec<MirrorPrayerRow> = serde_json::from_str(raw).map_err(|e| {
        ProviderError::InvalidResponse(tr_fmt(
            "%1 times JSON: %2",
            &[mirror.label, &e.to_string()],
        ))
    })?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let iso_date = ddmmyyyy_to_iso(&row.miladi_tarih_kisa).ok_or_else(|| {
            ProviderError::InvalidResponse(tr_fmt(
                "%1 invalid date format: %2",
                &[mirror.label, &row.miladi_tarih_kisa],
            ))
        })?;
        // Note: Prefer HicriTarihUzun (readable month name on emushaf.net) over numeric HicriTarihKisa,
        // and intentionally ignore MiladiTarihUzunIso8601 (+03:00 hardcoded upstream for all countries).
        let hijri = row
            .hicri_tarih_uzun
            .filter(|s| !s.trim().is_empty())
            .or_else(|| row.hicri_tarih_kisa.filter(|s| !s.trim().is_empty()));
        let day = DayTimes {
            date: iso_date,
            hijri,
            imsak: row.imsak,
            gunes: row.gunes,
            ogle: row.ogle,
            ikindi: row.ikindi,
            aksam: row.aksam,
            yatsi: row.yatsi,
            source: mirror.kind,
            fetched_at_unix: now_unix,
        };
        day.validate().map_err(ProviderError::InvalidResponse)?;
        out.push(day);
    }

    Ok(out)
}

fn ddmmyyyy_to_iso(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw.trim().split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let dd = parts[0];
    let mm = parts[1];
    let yyyy = parts[2];
    if dd.len() != 2 || mm.len() != 2 || yyyy.len() != 4 {
        return None;
    }
    Some(format!("{yyyy}-{mm}-{dd}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_mtopal_array_and_ignores_offset() {
        let raw = r#"[{"Imsak":"06:16","Gunes":"07:44","Ogle":"12:54","Ikindi":"15:29","Aksam":"17:53","Yatsi":"19:16","MiladiTarihKisa":"15.11.2026","MiladiTarihUzunIso8601":"2026-11-15T00:00:00.0000000+03:00","HicriTarihKisa":"6 Cemaziyelahir 1448"}]"#;
        let days = parse_times_json(raw, MTOPAL_MIRROR, 1000).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-11-15");
        assert_eq!(days[0].aksam, "17:53");
        assert_eq!(days[0].hijri.as_deref(), Some("6 Cemaziyelahir 1448"));
        assert_eq!(days[0].source, ProviderKind::Mtopal);
    }

    #[test]
    fn parses_live_ezanvakti_response_with_long_hijri() {
        let raw = r#"[{"HicriTarihKisa":"10.6.1448","HicriTarihUzun":"10 Cemaziyelahir 1448","MiladiTarihKisa":"19.11.2026","Imsak":"06:20","Gunes":"07:49","Ogle":"12:54","Ikindi":"15:27","Aksam":"17:50","Yatsi":"19:13"}]"#;
        let days = parse_times_json(raw, EZANVAKTI_MIRROR, 1000).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-11-19");
        assert_eq!(days[0].imsak, "06:20");
        assert_eq!(days[0].hijri.as_deref(), Some("10 Cemaziyelahir 1448"));
        assert_eq!(days[0].source, ProviderKind::Ezanvakti);
    }
}
