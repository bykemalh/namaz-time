use std::sync::{Arc, OnceLock};

use jiff::Timestamp;
use tracing::{debug, info};

use crate::i18n::tr_fmt;
use crate::models::{LocationProfile, ProviderKind, ScheduleSnapshot, compute_snapshot, format_hm_words};
use crate::providers::{FetchTimesResult, ProviderCoordinator};
use crate::services::cache::{CachedDays, DiskCache};
use crate::services::portal;
use crate::services::settings::AppSettings;
use crate::services::tray::{TrayHandle, TrayViewState};

static TOKIO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub fn shared_runtime() -> &'static tokio::runtime::Runtime {
    TOKIO_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Tokio runtime başlatılamadı")
    })
}

#[derive(Debug, Clone)]
pub enum DataEvent {
    TimesLoaded {
        location_key: String,
        result: FetchTimesResult,
    },
    TimesError {
        location_key: String,
        message: String,
    },
}

/// Manages shared Tokio runtime, disk cache, HTTP provider fetching, and periodic sync.
pub struct PrayerController {
    rt: &'static tokio::runtime::Runtime,
    coordinator: Arc<ProviderCoordinator>,
    cache: Arc<DiskCache>,
    tray: TrayHandle,
    data_tx: async_channel::Sender<DataEvent>,
}

impl PrayerController {
    pub fn new(
        rt: &'static tokio::runtime::Runtime,
        tray: TrayHandle,
        data_tx: async_channel::Sender<DataEvent>,
    ) -> Self {
        Self {
            rt,
            coordinator: Arc::new(ProviderCoordinator::new()),
            cache: Arc::new(DiskCache::default_xdg()),
            tray,
            data_tx,
        }
    }

    pub fn runtime(&self) -> &'static tokio::runtime::Runtime {
        self.rt
    }

    pub fn coordinator(&self) -> Arc<ProviderCoordinator> {
        self.coordinator.clone()
    }

    pub fn cache(&self) -> Arc<DiskCache> {
        self.cache.clone()
    }

    pub fn tray(&self) -> &TrayHandle {
        &self.tray
    }

    pub fn load_cached(&self, profile: &LocationProfile) -> CachedDays {
        self.cache.load_days(&profile.cache_key())
    }

    pub fn trigger_fetch(
        &self,
        profile: LocationProfile,
        preferred: Option<ProviderKind>,
        force_refresh: bool,
    ) {
        let coordinator = self.coordinator.clone();
        let cache = self.cache.clone();
        let tx = self.data_tx.clone();
        let location_key = profile.cache_key();

        self.rt.spawn(async move {
            debug!(
                "Vakit getirme başlatıldı: {} (force={force_refresh})",
                profile.display_label
            );
            match coordinator
                .get_prayer_days(&profile, preferred, &cache, force_refresh)
                .await
            {
                Ok(result) => {
                    info!(
                        "Vakitler hazır: {} ({} gün, stale={})",
                        profile.display_label,
                        result.cached.days.len(),
                        result.stale
                    );
                    let _ = tx
                        .send(DataEvent::TimesLoaded {
                            location_key,
                            result,
                        })
                        .await;
                }
                Err(message) => {
                    let _ = tx
                        .send(DataEvent::TimesError {
                            location_key,
                            message,
                        })
                        .await;
                }
            }
        });
    }

    pub fn update_tray_and_portal(
        &self,
        profile: Option<&LocationProfile>,
        snapshot: Option<&ScheduleSnapshot>,
        error_msg: Option<&str>,
        settings: &AppSettings,
    ) {
        let show_panel_label = settings.show_tray_label();
        let panel_format = settings.tray_label_format();

        let tray_state = match (profile, snapshot, error_msg) {
            (Some(loc), Some(snap), _) => {
                let title = loc.short_title();
                let view = TrayViewState::from_snapshot(&title, snap, show_panel_label, panel_format);

                if let Some(ref next) = snap.next_prayer {
                    let words = format_hm_words(next.remaining_seconds);
                    let status = if next.is_tomorrow {
                        tr_fmt(
                            "%1 • Tomorrow's %2 in %3",
                            &[&title, &next.name.label(), &words],
                        )
                    } else {
                        tr_fmt("%1 • %2 in %3", &[&title, &next.name.label(), &words])
                    };
                    self.rt.spawn(async move {
                        portal::set_background_status(&status).await;
                    });
                }
                view
            }
            (Some(loc), None, Some(err)) => TrayViewState::error(&loc.short_title(), err),
            (Some(loc), None, None) => TrayViewState::loading(&loc.short_title()),
            (None, _, _) => TrayViewState::default(),
        };

        self.tray.update_state(tray_state);
    }

    pub fn compute_current_snapshot(
        profile: &LocationProfile,
        cached: &CachedDays,
        stale: bool,
        stale_reason: Option<String>,
    ) -> Result<ScheduleSnapshot, String> {
        compute_snapshot(
            &cached.days,
            &profile.timezone,
            Timestamp::now(),
            stale,
            stale_reason,
        )
    }
}
