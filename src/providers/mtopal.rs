use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::models::{DayTimes, ProviderKind};
use crate::providers::{ProviderError, check_response_status};

const BASE_URL: &str = "https://namazvakti.mtopal.dev";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtopalCountry {
    #[serde(rename = "UlkeAdi")]
    pub ulke_adi: String,
    #[serde(rename = "UlkeAdiEn", default)]
    pub ulke_adi_en: String,
    #[serde(rename = "UlkeID")]
    pub ulke_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtopalCity {
    #[serde(rename = "SehirAdi")]
    pub sehir_adi: String,
    #[serde(rename = "SehirAdiEn", default)]
    pub sehir_adi_en: String,
    #[serde(rename = "SehirID")]
    pub sehir_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtopalDistrict {
    #[serde(rename = "IlceAdi")]
    pub ilce_adi: String,
    #[serde(rename = "IlceAdiEn", default)]
    pub ilce_adi_en: String,
    #[serde(rename = "IlceID")]
    pub ilce_id: String,
}

#[derive(Debug, Deserialize)]
struct MtopalPrayerRow {
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
    #[serde(rename = "HicriTarihKisa", default)]
    hicri_tarih_kisa: Option<String>,
}

pub async fn fetch_countries(client: &Client) -> Result<Vec<MtopalCountry>, ProviderError> {
    let resp = client.get(format!("{BASE_URL}/ulkeler")).send().await?;
    let resp = check_response_status(resp).await?;
    resp.json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("mtopal ülkeler JSON: {e}")))
}

pub async fn fetch_cities(
    client: &Client,
    country_id: &str,
) -> Result<Vec<MtopalCity>, ProviderError> {
    let resp = client
        .get(format!("{BASE_URL}/sehirler/{country_id}"))
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    resp.json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("mtopal şehirler JSON: {e}")))
}

pub async fn fetch_districts(
    client: &Client,
    city_id: &str,
) -> Result<Vec<MtopalDistrict>, ProviderError> {
    let resp = client
        .get(format!("{BASE_URL}/ilceler/{city_id}"))
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    resp.json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("mtopal ilçeler JSON: {e}")))
}

pub async fn fetch_prayer_times(
    client: &Client,
    district_id: &str,
    now_unix: i64,
) -> Result<Vec<DayTimes>, ProviderError> {
    let resp = client
        .get(format!("{BASE_URL}/vakitler/{district_id}"))
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    let raw = resp
        .text()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    parse_times_json(&raw, now_unix)
}

pub(crate) fn parse_times_json(raw: &str, now_unix: i64) -> Result<Vec<DayTimes>, ProviderError> {
    let rows: Vec<MtopalPrayerRow> = serde_json::from_str(raw)
        .map_err(|e| ProviderError::InvalidResponse(format!("mtopal vakit JSON: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let iso_date = ddmmyyyy_to_iso(&row.miladi_tarih_kisa).ok_or_else(|| {
            ProviderError::InvalidResponse(format!(
                "mtopal geçersiz tarih formatı: {}",
                row.miladi_tarih_kisa
            ))
        })?;
        // Note: Intentionally ignore MiladiTarihUzunIso8601 (+03:00 hardcoded by upstream for all countries).
        let day = DayTimes {
            date: iso_date,
            hijri: row.hicri_tarih_kisa.filter(|s| !s.trim().is_empty()),
            imsak: row.imsak,
            gunes: row.gunes,
            ogle: row.ogle,
            ikindi: row.ikindi,
            aksam: row.aksam,
            yatsi: row.yatsi,
            source: ProviderKind::Mtopal,
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
        let days = parse_times_json(raw, 1000).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-11-15");
        assert_eq!(days[0].aksam, "17:53");
        assert_eq!(days[0].hijri.as_deref(), Some("6 Cemaziyelahir 1448"));
        assert_eq!(days[0].source, ProviderKind::Mtopal);
    }
}
