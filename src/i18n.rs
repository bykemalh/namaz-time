use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use gettextrs::{bind_textdomain_codeset, bindtextdomain, gettext, setlocale, textdomain, LocaleCategory};
use gtk::gio;
use gtk::prelude::*;

use crate::config::{GETTEXT_PACKAGE, LOCALEDIR};
use crate::services::settings::APP_ID;

static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn take_restart_requested() -> bool {
    RESTART_REQUESTED.swap(false, Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppLang {
    En,
    Tr,
    Ru,
}

impl AppLang {
    pub fn from_code(code: &str) -> Option<Self> {
        let lower = code.trim().to_ascii_lowercase();
        let prefix = lower.split(['_', '-', '.']).next().unwrap_or("");
        match prefix {
            "tr" => Some(Self::Tr),
            "ru" => Some(Self::Ru),
            "en" => Some(Self::En),
            _ => None,
        }
    }
}

static ACTIVE_LANG: OnceLock<AppLang> = OnceLock::new();

pub fn active_lang() -> AppLang {
    *ACTIVE_LANG.get().unwrap_or(&AppLang::En)
}

/// Configures process locale, GSettings language override, and gettext text domain
/// before GTK or any worker threads are initialized.
pub fn init() {
    let configured = read_configured_language();
    let effective = match configured.as_deref() {
        Some("tr") => {
            apply_language_env("tr:en", &["tr_TR.UTF-8", "tr_TR.utf8", "en_US.UTF-8", "en_US.utf8"]);
            AppLang::Tr
        }
        Some("ru") => {
            apply_language_env("ru:en", &["ru_RU.UTF-8", "ru_RU.utf8", "en_US.UTF-8", "en_US.utf8"]);
            AppLang::Ru
        }
        Some("en") => {
            apply_language_env("en:C", &["en_US.UTF-8", "en_US.utf8"]);
            AppLang::En
        }
        _ => {
            if setlocale(LocaleCategory::LcAll, "").is_none() {
                let _ = setlocale(LocaleCategory::LcAll, "C.UTF-8");
            }
            detect_system_lang()
        }
    };
    let _ = ACTIVE_LANG.set(effective);

    let locale_dir = resolve_locale_dir();
    if let Err(err) = bindtextdomain(GETTEXT_PACKAGE, &locale_dir) {
        tracing::warn!("Unable to bind text domain ({locale_dir}): {err}");
    }
    let _ = bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8");
    let _ = textdomain(GETTEXT_PACKAGE);
}

fn read_configured_language() -> Option<String> {
    let source = gio::SettingsSchemaSource::default()?;
    source.lookup(APP_ID, true)?;
    let settings = gio::Settings::new(APP_ID);
    Some(settings.string("app-language").to_string())
}

fn is_c_locale(val: &str) -> bool {
    let trimmed = val.trim();
    trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("C")
        || trimmed.eq_ignore_ascii_case("POSIX")
        || trimmed.eq_ignore_ascii_case("C.UTF-8")
        || trimmed.eq_ignore_ascii_case("C.utf8")
}

fn apply_language_env(language_list: &str, preferred_locales: &[&str]) {
    // Initialize LC_ALL from current environment, falling back to a working non-C UTF-8
    // locale when host LC_ALL/LC_MESSAGES/LANG is C/C.UTF-8/POSIX (because glibc disables
    // LANGUAGE lookup completely under any C/C.UTF-8/POSIX locale).
    if let Ok(val) = std::env::var("LC_ALL") {
        if is_c_locale(&val) {
            std::env::remove_var("LC_ALL");
        }
    }

    if setlocale(LocaleCategory::LcAll, "").is_none() {
        let _ = setlocale(LocaleCategory::LcAll, "C.UTF-8");
    }

    let need_non_c_messages = std::env::var("LC_MESSAGES")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("LANG").ok())
        .map_or(true, |v| is_c_locale(&v));

    if need_non_c_messages {
        for candidate in preferred_locales {
            if setlocale(LocaleCategory::LcMessages, *candidate).is_some() {
                std::env::set_var("LC_MESSAGES", candidate);
                if std::env::var("LANG").map_or(true, |v| is_c_locale(&v)) {
                    std::env::set_var("LANG", candidate);
                }
                break;
            }
        }
    } else if let Some(&first) = preferred_locales.first() {
        // Prefer matching LC_MESSAGES if installed so GTK stock widgets follow the choice too.
        if setlocale(LocaleCategory::LcMessages, first).is_some() {
            std::env::set_var("LC_MESSAGES", first);
        }
    }

    std::env::set_var("LANGUAGE", language_list);
}

fn detect_system_lang() -> AppLang {
    for key in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = std::env::var(key) {
            for item in val.split(':') {
                if let Some(lang) = AppLang::from_code(item) {
                    return lang;
                }
            }
        }
    }
    AppLang::En
}

fn resolve_locale_dir() -> String {
    if let Ok(custom) = std::env::var("NAMAZ_TIME_LOCALEDIR") {
        if !custom.trim().is_empty() && has_compiled_catalogs(Path::new(&custom)) {
            return custom;
        }
    }
    if has_compiled_catalogs(Path::new(LOCALEDIR)) {
        return LOCALEDIR.to_string();
    }
    for dev in ["_stage/app/share/locale", "_build/locale"] {
        if has_compiled_catalogs(Path::new(dev)) {
            return dev.to_string();
        }
    }
    LOCALEDIR.to_string()
}

fn has_compiled_catalogs(dir: &Path) -> bool {
    ["tr", "ru", "en"]
        .iter()
        .any(|lang| dir.join(lang).join("LC_MESSAGES").join(format!("{GETTEXT_PACKAGE}.mo")).exists())
}

pub fn tr(msgid: &str) -> String {
    gettext(msgid)
}

/// Translates `template` and replaces positional placeholders `%1`, `%2`, ...
/// in a single pass so word order can differ naturally across languages.
pub fn tr_fmt(template: &str, args: &[&str]) -> String {
    let translated = gettext(template);
    apply_positional_args(&translated, args)
}

fn apply_positional_args(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.peek().copied() {
                Some(d @ '1'..='9') => {
                    chars.next();
                    let idx = (d as usize) - ('1' as usize);
                    if let Some(val) = args.get(idx) {
                        out.push_str(val);
                    } else {
                        out.push('%');
                        out.push(d);
                    }
                }
                Some('%') => {
                    chars.next();
                    out.push('%');
                }
                _ => out.push('%'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Converts provider-supplied Hijri date strings (commonly Turkish month names
/// from Diyanet/İmsakiyem feeds or DD.MM.YYYY numeric values) into the active UI language.
pub fn localize_hijri(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Handle "6 Cemaziyelahir 1448" or similar whitespace-separated strings.
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() >= 3 {
        let day = parts[0];
        let year = parts[parts.len() - 1];
        let month_raw = parts[1..parts.len() - 1].join(" ");
        if let Some(month_num) = parse_hijri_month(&month_raw) {
            return format!("{day} {} {year}", hijri_month_label(month_num));
        }
    }

    // Handle "06.06.1448" numeric format if any provider sends it.
    let dots: Vec<&str> = trimmed.split('.').collect();
    if dots.len() == 3 {
        if let (Ok(day), Ok(month), Ok(year)) = (
            dots[0].trim().parse::<u32>(),
            dots[1].trim().parse::<u32>(),
            dots[2].trim().parse::<u32>(),
        ) {
            if (1..=12).contains(&month) {
                return format!("{day} {} {year}", hijri_month_label(month as u8));
            }
        }
    }

    trimmed.to_string()
}

fn parse_hijri_month(name: &str) -> Option<u8> {
    let norm: String = name
        .replace('İ', "i")
        .to_lowercase()
        .replace('\u{307}', "")
        .replace('ı', "i")
        .replace('ş', "s")
        .replace('ğ', "g")
        .replace('ü', "u")
        .replace('ö', "o")
        .replace('ç', "c")
        .replace(['\'', '’', '-', ' '], "");

    match norm.as_str() {
        "muharrem" | "muharram" => Some(1),
        "safer" | "safar" => Some(2),
        "rebiulevvel" | "rabialawwal" | "rabiulawwal" | "rebiulevel" => Some(3),
        "rebiulahir" | "rabialthani" | "rabiulakhir" | "rabialakhir" => Some(4),
        "cemaziyelevvel" | "jumadaalawwal" | "jumadalawwal" | "cemaziyelevel" => Some(5),
        "cemaziyelahir" | "jumadaalthani" | "jumadalthani" | "jumadalakhir" => Some(6),
        "recep" | "receb" | "rajab" => Some(7),
        "saban" | "shaban" => Some(8),
        "ramazan" | "ramadan" => Some(9),
        "sevval" | "shawwal" => Some(10),
        "zilkade" | "dhualqidah" | "zulqadah" => Some(11),
        "zilhicce" | "dhualhijjah" | "zulhijjah" => Some(12),
        _ => None,
    }
}

fn hijri_month_label(month: u8) -> String {
    match month {
        1 => tr("Muharram"),
        2 => tr("Safar"),
        3 => tr("Rabi' al-Awwal"),
        4 => tr("Rabi' al-Thani"),
        5 => tr("Jumada al-Awwal"),
        6 => tr("Jumada al-Thani"),
        7 => tr("Rajab"),
        8 => tr("Sha'ban"),
        9 => tr("Ramadan"),
        10 => tr("Shawwal"),
        11 => tr("Dhu al-Qi'dah"),
        12 => tr("Dhu al-Hijjah"),
        _ => String::new(),
    }
}
