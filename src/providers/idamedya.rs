use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::models::{DayTimes, ProviderKind};
use crate::providers::{ProviderError, check_response_status};

const BASE_URL: &str = "https://idamedya.org/api/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdamedyaProvince {
    pub ad: String,
    pub ilce: u64,
    pub sehir: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdamedyaDistrict {
    pub ad: String,
    pub ilce: u64,
}

#[derive(Debug, Deserialize)]
struct IdamedyaEnvelope<T> {
    ok: bool,
    data: Option<T>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IdamedyaTimesPayload {
    gunler: Vec<IdamedyaDayRow>,
}

#[derive(Debug, Deserialize)]
struct IdamedyaDayRow {
    tarih: String,
    #[serde(default)]
    hicri: Option<String>,
    imsak: String,
    gunes: String,
    ogle: String,
    ikindi: String,
    aksam: String,
    yatsi: String,
}

pub async fn fetch_provinces(client: &Client) -> Result<Vec<IdamedyaProvince>, ProviderError> {
    let resp = client.get(format!("{BASE_URL}/namaz/iller")).send().await?;
    let resp = check_response_status(resp).await?;
    let body: IdamedyaEnvelope<Vec<IdamedyaProvince>> = resp
        .json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("İDA Medya iller JSON: {e}")))?;
    if !body.ok {
        return Err(ProviderError::InvalidResponse(
            body.error
                .unwrap_or_else(|| "İDA Medya ok=false".to_string()),
        ));
    }
    Ok(body.data.unwrap_or_default())
}

pub async fn fetch_districts(
    client: &Client,
    sehir_id: u64,
) -> Result<Vec<IdamedyaDistrict>, ProviderError> {
    let resp = client
        .get(format!("{BASE_URL}/namaz/ilceler/{sehir_id}"))
        .send()
        .await?;
    let resp = check_response_status(resp).await?;
    let body: IdamedyaEnvelope<Vec<IdamedyaDistrict>> = resp
        .json()
        .await
        .map_err(|e| ProviderError::InvalidResponse(format!("İDA Medya ilçeler JSON: {e}")))?;
    if !body.ok {
        return Err(ProviderError::InvalidResponse(
            body.error
                .unwrap_or_else(|| "İDA Medya ok=false".to_string()),
        ));
    }
    Ok(body.data.unwrap_or_default())
}

pub async fn fetch_prayer_times(
    client: &Client,
    ilce_id: &str,
    now_unix: i64,
) -> Result<Vec<DayTimes>, ProviderError> {
    let resp = client
        .get(format!("{BASE_URL}/namaz/{ilce_id}"))
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
    let env: IdamedyaEnvelope<IdamedyaTimesPayload> = serde_json::from_str(raw)
        .map_err(|e| ProviderError::InvalidResponse(format!("İDA Medya vakit JSON: {e}")))?;
    if !env.ok {
        return Err(ProviderError::InvalidResponse(
            env.error
                .unwrap_or_else(|| "İDA Medya ok=false".to_string()),
        ));
    }
    let payload = env
        .data
        .ok_or_else(|| ProviderError::InvalidResponse("İDA Medya data boş".into()))?;

    let mut out = Vec::with_capacity(payload.gunler.len());
    for row in payload.gunler {
        let day = DayTimes {
            date: row.tarih,
            hijri: row.hicri.filter(|s| !s.trim().is_empty()),
            imsak: row.imsak,
            gunes: row.gunes,
            ogle: row.ogle,
            ikindi: row.ikindi,
            aksam: row.aksam,
            yatsi: row.yatsi,
            source: ProviderKind::Idamedya,
            fetched_at_unix: now_unix,
        };
        day.validate().map_err(ProviderError::InvalidResponse)?;
        out.push(day);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_idamedya_response() {
        let raw = r#"{"ok":true,"data":{"yer":"İstanbul","ilce":9541,"kaynak":"Diyanet İşleri Başkanlığı","gunler":[{"tarih":"2026-11-15","hicri":"6 Cemaziyelahir 1448","imsak":"06:16","gunes":"07:44","ogle":"12:54","ikindi":"15:29","aksam":"17:53","yatsi":"19:16"}]}}"#;
        let days = parse_times_json(raw, 1000).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-11-15");
        assert_eq!(days[0].ogle, "12:54");
        assert_eq!(days[0].source, ProviderKind::Idamedya);
    }
}
