use gtk::gio;
use gtk::prelude::*;
use tracing::warn;

use crate::models::{
    AppearanceConfig, HeroTheme, LocationProfile, ProviderKind, TrayLabelFormat, UiStyle,
};

pub const APP_ID: &str = "io.github.bykemalh.namaz_time.NamazTime";

#[derive(Debug, Clone)]
pub struct AppSettings {
    inner: gio::Settings,
}

impl AppSettings {
    pub fn new() -> Self {
        Self {
            inner: gio::Settings::new(APP_ID),
        }
    }

    pub fn raw(&self) -> &gio::Settings {
        &self.inner
    }

    pub fn location_profile(&self) -> Option<LocationProfile> {
        let json = self.inner.string("location-profile-json");
        let trimmed = json.trim();
        if trimmed.is_empty() {
            return None;
        }
        match serde_json::from_str(trimmed) {
            Ok(profile) => Some(profile),
            Err(err) => {
                warn!("Kayıtlı konum okunamadı: {err}");
                None
            }
        }
    }

    pub fn set_location_profile(&self, profile: &LocationProfile) {
        if let Ok(json) = serde_json::to_string(profile) {
            if let Err(err) = self.inner.set_string("location-profile-json", &json) {
                warn!("Konum kaydedilemedi: {err}");
            }
        }
    }

    pub fn preferred_source(&self) -> Option<ProviderKind> {
        let val = self.inner.string("preferred-source");
        if val == "auto" {
            None
        } else {
            ProviderKind::from_id(&val)
        }
    }

    pub fn preferred_source_id(&self) -> String {
        self.inner.string("preferred-source").to_string()
    }

    pub fn set_preferred_source_id(&self, id: &str) {
        let _ = self.inner.set_string("preferred-source", id);
    }

    pub fn run_in_background(&self) -> bool {
        self.inner.boolean("run-in-background")
    }

    pub fn set_run_in_background(&self, enabled: bool) {
        let _ = self.inner.set_boolean("run-in-background", enabled);
    }

    pub fn autostart(&self) -> bool {
        self.inner.boolean("autostart")
    }

    pub fn set_autostart(&self, enabled: bool) {
        let _ = self.inner.set_boolean("autostart", enabled);
    }

    pub fn show_tray_label(&self) -> bool {
        self.inner.boolean("show-tray-label")
    }

    pub fn set_show_tray_label(&self, enabled: bool) {
        let _ = self.inner.set_boolean("show-tray-label", enabled);
    }

    pub fn tray_label_format(&self) -> TrayLabelFormat {
        TrayLabelFormat::from_id(&self.inner.string("tray-label-format"))
    }

    pub fn set_tray_label_format(&self, fmt: TrayLabelFormat) {
        let _ = self.inner.set_string("tray-label-format", fmt.as_str());
    }

    pub fn show_seconds(&self) -> bool {
        self.inner.boolean("show-seconds")
    }

    pub fn set_show_seconds(&self, enabled: bool) {
        let _ = self.inner.set_boolean("show-seconds", enabled);
    }

    pub fn app_language(&self) -> String {
        self.inner.string("app-language").to_string()
    }

    pub fn set_app_language(&self, lang: &str) {
        let _ = self.inner.set_string("app-language", lang);
    }

    pub fn hero_theme(&self) -> HeroTheme {
        HeroTheme::from_id(&self.inner.string("hero-theme"))
    }

    pub fn set_hero_theme(&self, theme: HeroTheme) {
        let _ = self.inner.set_string("hero-theme", theme.id());
    }

    pub fn ui_style(&self) -> UiStyle {
        UiStyle::from_id(&self.inner.string("ui-style"))
    }

    pub fn set_ui_style(&self, style: UiStyle) {
        let _ = self.inner.set_string("ui-style", style.id());
    }

    pub fn colorful_icons(&self) -> bool {
        self.inner.boolean("colorful-icons")
    }

    pub fn set_colorful_icons(&self, enabled: bool) {
        let _ = self.inner.set_boolean("colorful-icons", enabled);
    }

    pub fn appearance(&self) -> AppearanceConfig {
        AppearanceConfig {
            ui_style: self.ui_style(),
            hero_theme: self.hero_theme(),
            colorful_icons: self.colorful_icons(),
        }
    }
}
