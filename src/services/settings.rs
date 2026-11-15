use gtk::gio;
use gtk::prelude::*;
use tracing::warn;

use crate::models::{LocationProfile, ProviderKind, TrayLabelFormat};

pub const APP_ID: &str = "me.bykemah.namaztime";

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
}
