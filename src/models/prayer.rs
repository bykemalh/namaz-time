use std::collections::BTreeMap;
use std::fmt;

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Imsakiyem,
    Mtopal,
    Idamedya,
}

impl ProviderKind {
    pub const ALL: [Self; 3] = [Self::Imsakiyem, Self::Mtopal, Self::Idamedya];

    pub fn id(self) -> &'static str {
        match self {
            Self::Imsakiyem => "imsakiyem",
            Self::Mtopal => "mtopal",
            Self::Idamedya => "idamedya",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Imsakiyem => "İmsakiyem",
            Self::Mtopal => "mtopal v1.5",
            Self::Idamedya => "İDA Medya",
        }
    }

    pub fn attribution_text(self) -> String {
        format!("{} (Diyanet verisi)", self.display_name())
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "imsakiyem" => Some(Self::Imsakiyem),
            "mtopal" => Some(Self::Mtopal),
            "idamedya" => Some(Self::Idamedya),
            _ => None,
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrayerName {
    Imsak,
    Gunes,
    Ogle,
    Ikindi,
    Aksam,
    Yatsi,
}

impl PrayerName {
    pub const ALL: [Self; 6] = [
        Self::Imsak,
        Self::Gunes,
        Self::Ogle,
        Self::Ikindi,
        Self::Aksam,
        Self::Yatsi,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Imsak => "İmsak",
            Self::Gunes => "Güneş",
            Self::Ogle => "Öğle",
            Self::Ikindi => "İkindi",
            Self::Aksam => "Akşam",
            Self::Yatsi => "Yatsı",
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Imsak => "namaz-imsak-symbolic",
            Self::Gunes => "namaz-gunes-symbolic",
            Self::Ogle => "namaz-ogle-symbolic",
            Self::Ikindi => "namaz-ikindi-symbolic",
            Self::Aksam => "namaz-aksam-symbolic",
            Self::Yatsi => "namaz-yatsi-symbolic",
        }
    }
}

impl fmt::Display for PrayerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTimes {
    pub date: String,
    pub hijri: Option<String>,
    pub imsak: String,
    pub gunes: String,
    pub ogle: String,
    pub ikindi: String,
    pub aksam: String,
    pub yatsi: String,
    pub source: ProviderKind,
    pub fetched_at_unix: i64,
}

impl DayTimes {
    pub fn time_for(&self, prayer: PrayerName) -> &str {
        match prayer {
            PrayerName::Imsak => &self.imsak,
            PrayerName::Gunes => &self.gunes,
            PrayerName::Ogle => &self.ogle,
            PrayerName::Ikindi => &self.ikindi,
            PrayerName::Aksam => &self.aksam,
            PrayerName::Yatsi => &self.yatsi,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let _: Date = self
            .date
            .parse()
            .map_err(|_| format!("Geçersiz tarih: {}", self.date))?;

        for prayer in PrayerName::ALL {
            let val = self.time_for(prayer);
            parse_hhmm(val).ok_or_else(|| format!("Geçersiz {} vakti: {}", prayer.label(), val))?;
        }
        Ok(())
    }

    pub fn to_timestamp(&self, prayer: PrayerName, tz: &TimeZone) -> Result<Timestamp, String> {
        let date: Date = self
            .date
            .parse()
            .map_err(|_| format!("Geçersiz tarih: {}", self.date))?;
        let (hour, minute) = parse_hhmm(self.time_for(prayer))
            .ok_or_else(|| format!("Geçersiz {} vakti", prayer.label()))?;
        let zoned = date
            .at(hour as i8, minute as i8, 0, 0)
            .to_zoned(tz.clone())
            .map_err(|e| format!("Saat dilimi hatası: {e}"))?;
        Ok(zoned.timestamp())
    }
}

pub fn parse_hhmm(raw: &str) -> Option<(u8, u8)> {
    let trimmed = raw.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None;
    }
    let hour: u8 = trimmed[0..2].parse().ok()?;
    let minute: u8 = trimmed[3..5].parse().ok()?;
    if hour <= 23 && minute <= 59 {
        Some((hour, minute))
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrayLabelFormat {
    PrayerAndTime,
    TimeOnly,
}

impl TrayLabelFormat {
    pub fn from_id(s: &str) -> Self {
        match s.trim() {
            "time-only" => Self::TimeOnly,
            _ => Self::PrayerAndTime,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrayerAndTime => "prayer-and-time",
            Self::TimeOnly => "time-only",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NextPrayerInfo {
    pub name: PrayerName,
    pub time_str: String,
    pub target_timestamp: i64,
    pub is_tomorrow: bool,
    pub remaining_seconds: i64,
    pub progress: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleSnapshot {
    pub today: DayTimes,
    pub tomorrow: Option<DayTimes>,
    pub current_prayer: PrayerName,
    pub next_prayer: Option<NextPrayerInfo>,
    pub local_date_str: String,
    pub timezone: String,
    pub stale: bool,
    pub stale_reason: Option<String>,
}

/// Computes the active prayer, next prayer, remaining seconds, and interval progress
/// for `now` inside `timezone_iana` using cached `yesterday + today + tomorrow` entries.
pub fn compute_snapshot(
    days: &BTreeMap<String, DayTimes>,
    timezone_iana: &str,
    now: Timestamp,
    stale: bool,
    stale_reason: Option<String>,
) -> Result<ScheduleSnapshot, String> {
    let tz = TimeZone::get(timezone_iana)
        .map_err(|e| format!("Geçersiz saat dilimi ({timezone_iana}): {e}"))?;
    let zoned_now = now.to_zoned(tz.clone());
    let today_date = zoned_now.date();
    let today_key = today_date.to_string();

    let today = days
        .get(&today_key)
        .cloned()
        .ok_or_else(|| format!("Bugünün ({today_key}) namaz vakti verisi bulunamadı"))?;

    let yesterday_key = today_date
        .yesterday()
        .map(|d| d.to_string())
        .unwrap_or_default();
    let tomorrow_key = today_date
        .tomorrow()
        .map(|d| d.to_string())
        .unwrap_or_default();

    let yesterday = days.get(&yesterday_key).cloned();
    let tomorrow = days.get(&tomorrow_key).cloned();

    let today_entries: Vec<(PrayerName, Timestamp)> = PrayerName::ALL
        .iter()
        .map(|&p| today.to_timestamp(p, &tz).map(|ts| (p, ts)))
        .collect::<Result<Vec<_>, _>>()?;

    let now_sec = now.as_second();

    let (current_prayer, current_start_ts, next_prayer) =
        if now_sec < today_entries[0].1.as_second() {
            // Before today's Imsak: active prayer is yesterday's Yatsi.
            let prev_start = yesterday
                .as_ref()
                .and_then(|y| y.to_timestamp(PrayerName::Yatsi, &tz).ok());
            let next_ts = today_entries[0].1;
            let remaining = (next_ts.as_second() - now_sec).max(0);
            let progress = prev_start.map(|start| compute_progress(start.as_second(), now_sec, next_ts.as_second()));
            (
                PrayerName::Yatsi,
                prev_start,
                Some(NextPrayerInfo {
                    name: PrayerName::Imsak,
                    time_str: today.imsak.clone(),
                    target_timestamp: next_ts.as_second(),
                    is_tomorrow: false,
                    remaining_seconds: remaining,
                    progress,
                }),
            )
        } else if let Some(idx) = today_entries.iter().position(|(_, ts)| now_sec < ts.as_second()) {
            let (curr_p, curr_ts) = today_entries[idx - 1];
            let (next_p, next_ts) = today_entries[idx];
            let remaining = (next_ts.as_second() - now_sec).max(0);
            let progress = Some(compute_progress(
                curr_ts.as_second(),
                now_sec,
                next_ts.as_second(),
            ));
            (
                curr_p,
                Some(curr_ts),
                Some(NextPrayerInfo {
                    name: next_p,
                    time_str: today.time_for(next_p).to_string(),
                    target_timestamp: next_ts.as_second(),
                    is_tomorrow: false,
                    remaining_seconds: remaining,
                    progress,
                }),
            )
        } else {
            // After today's Yatsi: next prayer is tomorrow's Imsak (if cached).
            let curr_ts = today_entries[5].1;
            let next = if let Some(ref tom) = tomorrow {
                let next_ts = tom.to_timestamp(PrayerName::Imsak, &tz)?;
                let remaining = (next_ts.as_second() - now_sec).max(0);
                let progress = Some(compute_progress(
                    curr_ts.as_second(),
                    now_sec,
                    next_ts.as_second(),
                ));
                Some(NextPrayerInfo {
                    name: PrayerName::Imsak,
                    time_str: tom.imsak.clone(),
                    target_timestamp: next_ts.as_second(),
                    is_tomorrow: true,
                    remaining_seconds: remaining,
                    progress,
                })
            } else {
                None
            };
            (PrayerName::Yatsi, Some(curr_ts), next)
        };

    let _ = current_start_ts;

    Ok(ScheduleSnapshot {
        today,
        tomorrow,
        current_prayer,
        next_prayer,
        local_date_str: format_turkish_date(today_date),
        timezone: timezone_iana.to_string(),
        stale,
        stale_reason,
    })
}

fn compute_progress(start_sec: i64, now_sec: i64, end_sec: i64) -> f64 {
    let total = end_sec - start_sec;
    if total <= 0 {
        return 1.0;
    }
    let elapsed = now_sec - start_sec;
    (elapsed as f64 / total as f64).clamp(0.0, 1.0)
}

pub fn format_hms(remaining_seconds: i64) -> String {
    let secs = remaining_seconds.max(0);
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub fn format_hm(remaining_seconds: i64) -> String {
    let secs = remaining_seconds.max(0);
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    format!("{hours:02}:{minutes:02}")
}

pub fn format_hm_words(remaining_seconds: i64) -> String {
    let secs = remaining_seconds.max(0);
    if secs == 0 {
        return "Vakit girdi".to_string();
    }
    if secs < 60 {
        return "<1 dk".to_string();
    }
    let total_minutes = secs / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    match (hours, minutes) {
        (0, m) => format!("{m} dk"),
        (h, 0) => format!("{h} sa"),
        (h, m) => format!("{h} sa {m} dk"),
    }
}

pub fn format_tray_label(next: &NextPrayerInfo, mode: TrayLabelFormat) -> String {
    let secs = next.remaining_seconds.max(0);
    let duration_part = if secs == 0 {
        "0dk".to_string()
    } else if secs < 60 {
        "<1dk".to_string()
    } else {
        let total_minutes = secs / 60;
        let hours = total_minutes / 60;
        let minutes = total_minutes % 60;
        match (hours, minutes) {
            (0, m) => format!("{m}dk"),
            (h, 0) => format!("{h}s"),
            (h, m) => format!("{h}s {m:02}dk"),
        }
    };

    match mode {
        TrayLabelFormat::PrayerAndTime => format!("{} {duration_part}", next.name.label()),
        TrayLabelFormat::TimeOnly => duration_part,
    }
}

pub fn format_turkish_date(date: Date) -> String {
    let month_name = match date.month() {
        1 => "Ocak",
        2 => "Şubat",
        3 => "Mart",
        4 => "Nisan",
        5 => "Mayıs",
        6 => "Haziran",
        7 => "Temmuz",
        8 => "Ağustos",
        9 => "Eylül",
        10 => "Ekim",
        11 => "Kasım",
        12 => "Aralık",
        _ => "",
    };
    let weekday_name = match date.weekday() {
        Weekday::Monday => "Pazartesi",
        Weekday::Tuesday => "Salı",
        Weekday::Wednesday => "Çarşamba",
        Weekday::Thursday => "Perşembe",
        Weekday::Friday => "Cuma",
        Weekday::Saturday => "Cumartesi",
        Weekday::Sunday => "Pazar",
    };
    format!("{} {} {}, {}", date.day(), month_name, date.year(), weekday_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_day(date: &str) -> DayTimes {
        DayTimes {
            date: date.to_string(),
            hijri: Some("6 Cemaziyelahir 1448".to_string()),
            imsak: "06:16".to_string(),
            gunes: "07:44".to_string(),
            ogle: "12:54".to_string(),
            ikindi: "15:29".to_string(),
            aksam: "17:53".to_string(),
            yatsi: "19:16".to_string(),
            source: ProviderKind::Mtopal,
            fetched_at_unix: 0,
        }
    }

    fn ts_istanbul(y: i16, m: i8, d: i8, h: i8, min: i8, sec: i8) -> Timestamp {
        let tz = TimeZone::get("Europe/Istanbul").unwrap();
        Date::new(y, m, d)
            .unwrap()
            .at(h, min, sec, 0)
            .to_zoned(tz)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn validates_day_times() {
        let valid = sample_day("2026-11-15");
        assert!(valid.validate().is_ok());

        let mut bad_time = valid.clone();
        bad_time.ikindi = "25:00".to_string();
        assert!(bad_time.validate().is_err());

        let mut bad_date = valid;
        bad_date.date = "15.11.2026".to_string();
        assert!(bad_date.validate().is_err());
    }

    #[test]
    fn countdown_midday_before_ikindi() {
        let mut days = BTreeMap::new();
        days.insert("2026-11-15".to_string(), sample_day("2026-11-15"));

        let now = ts_istanbul(2026, 11, 15, 14, 47, 0);
        let snap = compute_snapshot(&days, "Europe/Istanbul", now, false, None).unwrap();

        assert_eq!(snap.current_prayer, PrayerName::Ogle);
        let next = snap.next_prayer.unwrap();
        assert_eq!(next.name, PrayerName::Ikindi);
        assert_eq!(next.time_str, "15:29");
        assert_eq!(next.remaining_seconds, 42 * 60);
        assert!(!next.is_tomorrow);
        assert_eq!(format_tray_label(&next, TrayLabelFormat::PrayerAndTime), "İkindi 42dk");
        assert_eq!(format_tray_label(&next, TrayLabelFormat::TimeOnly), "42dk");
    }

    #[test]
    fn countdown_after_yatsi_rolls_to_tomorrow_imsak() {
        let mut days = BTreeMap::new();
        days.insert("2026-11-15".to_string(), sample_day("2026-11-15"));
        let mut tomorrow = sample_day("2026-11-16");
        tomorrow.imsak = "06:17".to_string();
        days.insert("2026-11-16".to_string(), tomorrow);

        let now = ts_istanbul(2026, 11, 15, 23, 17, 0);
        let snap = compute_snapshot(&days, "Europe/Istanbul", now, false, None).unwrap();

        assert_eq!(snap.current_prayer, PrayerName::Yatsi);
        let next = snap.next_prayer.unwrap();
        assert_eq!(next.name, PrayerName::Imsak);
        assert!(next.is_tomorrow);
        assert_eq!(next.time_str, "06:17");
        assert_eq!(next.remaining_seconds, 7 * 3600);
    }

    #[test]
    fn countdown_before_imsak_uses_yesterday_yatsi_for_progress() {
        let mut days = BTreeMap::new();
        days.insert("2026-11-14".to_string(), sample_day("2026-11-14"));
        days.insert("2026-11-15".to_string(), sample_day("2026-11-15"));

        let now = ts_istanbul(2026, 11, 15, 3, 16, 0);
        let snap = compute_snapshot(&days, "Europe/Istanbul", now, false, None).unwrap();

        assert_eq!(snap.current_prayer, PrayerName::Yatsi);
        let next = snap.next_prayer.unwrap();
        assert_eq!(next.name, PrayerName::Imsak);
        assert!(!next.is_tomorrow);
        assert_eq!(next.remaining_seconds, 3 * 3600);
        assert!(next.progress.is_some());
    }

    #[test]
    fn foreign_timezone_berlin_computes_local_date() {
        // 2026-11-15 23:30 UTC is 2026-11-16 00:30 in Europe/Berlin (UTC+1)
        let mut days = BTreeMap::new();
        days.insert("2026-11-15".to_string(), sample_day("2026-11-15"));
        let mut nov16 = sample_day("2026-11-16");
        nov16.imsak = "05:40".to_string();
        days.insert("2026-11-16".to_string(), nov16);

        let now: Timestamp = "2026-11-15T23:30:00Z".parse().unwrap();
        let snap = compute_snapshot(&days, "Europe/Berlin", now, false, None).unwrap();

        assert_eq!(snap.today.date, "2026-11-16");
        assert_eq!(snap.current_prayer, PrayerName::Yatsi);
        let next = snap.next_prayer.unwrap();
        assert_eq!(next.name, PrayerName::Imsak);
        assert_eq!(next.remaining_seconds, (5 * 60 + 10) * 60);
    }
}
