use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use jiff::Timestamp;
use tracing::{debug, info, warn};

use crate::config::VERSION;
use crate::i18n::{tr, tr_fmt};
use crate::models::{LocationProfile, PrayerName, TrayLabelFormat};
use crate::services::{
    AppSettings, CachedDays, DataEvent, DiskCache, PrayerController, TrayCommand, shared_runtime,
    spawn_tray_service,
};
use crate::ui;
use crate::window::NamazTimeWindow;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayUpdateBucket {
    location_key: Option<String>,
    today_date: Option<String>,
    current_prayer: Option<PrayerName>,
    next_prayer: Option<PrayerName>,
    minute_bucket: Option<(bool, bool, i64)>,
    show_panel_label: bool,
    panel_format: TrayLabelFormat,
    error_msg: Option<String>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct NamazTimeApplication {
        pub settings: OnceCell<AppSettings>,
        pub controller: OnceCell<Rc<PrayerController>>,
        pub hold_guard: RefCell<Option<gio::ApplicationHoldGuard>>,
        pub window: RefCell<Option<glib::WeakRef<NamazTimeWindow>>>,

        pub current_profile: RefCell<Option<LocationProfile>>,
        pub cached_days: RefCell<Option<CachedDays>>,
        pub is_stale: RefCell<bool>,
        pub stale_reason: RefCell<Option<String>>,
        pub last_error: RefCell<Option<String>>,
        pub is_fetching: RefCell<bool>,
        pub manual_refresh_pending: RefCell<bool>,
        pub last_fetch_attempt_unix: RefCell<i64>,
        pub last_preferred_source: RefCell<String>,
        pub(super) last_tray_bucket: RefCell<Option<TrayUpdateBucket>>,
        pub tray_watcher_online: RefCell<Option<bool>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NamazTimeApplication {
        const NAME: &'static str = "NamazTimeApplication";
        type Type = super::NamazTimeApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for NamazTimeApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_gactions();
            obj.set_accels_for_action("app.choose-location", &["<control>l"]);
            obj.set_accels_for_action("app.refresh", &["<control>r", "F5"]);
            obj.set_accels_for_action("app.preferences", &["<control>comma"]);
            obj.set_accels_for_action("app.shortcuts", &["<control>question"]);
            obj.set_accels_for_action("app.quit", &["<control>q"]);
        }
    }

    impl ApplicationImpl for NamazTimeApplication {
        fn startup(&self) {
            self.parent_startup();
            self.obj().initialize_app();
        }

        fn activate(&self) {
            let obj = self.obj();
            let window = obj.get_or_create_window();
            obj.sync_window_view(&window);
            window.present();
        }
    }

    impl GtkApplicationImpl for NamazTimeApplication {}
    impl AdwApplicationImpl for NamazTimeApplication {}
}

glib::wrapper! {
    pub struct NamazTimeApplication(ObjectSubclass<imp::NamazTimeApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl NamazTimeApplication {
    pub fn new(application_id: &str, flags: &gio::ApplicationFlags) -> Self {
        glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", flags)
            .property("resource-base-path", "/io/github/bykemalh/namaz_time/NamazTime")
            .build()
    }

    fn setup_gactions(&self) {
        let choose_location = gio::ActionEntry::builder("choose-location")
            .activate(move |app: &Self, _, _| app.show_location_picker())
            .build();
        let refresh = gio::ActionEntry::builder("refresh")
            .activate(move |app: &Self, _, _| app.trigger_manual_refresh())
            .build();
        let preferences = gio::ActionEntry::builder("preferences")
            .activate(move |app: &Self, _, _| app.show_preferences())
            .build();
        let shortcuts = gio::ActionEntry::builder("shortcuts")
            .activate(move |app: &Self, _, _| app.show_shortcuts())
            .build();
        let about = gio::ActionEntry::builder("about")
            .activate(move |app: &Self, _, _| app.show_about())
            .build();
        let quit = gio::ActionEntry::builder("quit")
            .activate(move |app: &Self, _, _| app.quit_gracefully())
            .build();

        self.add_action_entries([
            choose_location,
            refresh,
            preferences,
            shortcuts,
            about,
            quit,
        ]);
    }

    fn initialize_app(&self) {
        self.load_css();

        let imp = self.imp();
        let settings = AppSettings::new();
        *imp.last_preferred_source.borrow_mut() = settings.preferred_source_id();
        let _ = imp.settings.set(settings.clone());

        self.sync_hold_guard();

        let rt = shared_runtime();
        let (tray_cmd_tx, tray_cmd_rx) = async_channel::unbounded::<TrayCommand>();
        let (watcher_tx, watcher_rx) = async_channel::unbounded::<bool>();
        let (data_tx, data_rx) = async_channel::unbounded::<DataEvent>();

        let tray_handle = spawn_tray_service(rt, tray_cmd_tx, watcher_tx);
        let controller = Rc::new(PrayerController::new(rt, tray_handle, data_tx));
        let _ = imp.controller.set(controller);

        if let Some(profile) = settings.location_profile() {
            self.load_profile(profile, false);
        } else {
            self.sync_tray_and_views(true);
        }

        // Listen for GNOME tray icon & DBusMenu commands.
        {
            let app = self.clone();
            glib::spawn_future_local(async move {
                while let Ok(cmd) = tray_cmd_rx.recv().await {
                    match cmd {
                        TrayCommand::ShowWindow => {
                            app.activate();
                        }
                        TrayCommand::ToggleWindow => {
                            app.toggle_window_visibility();
                        }
                        TrayCommand::Refresh => {
                            app.trigger_manual_refresh();
                        }
                        TrayCommand::OpenPreferences => {
                            app.activate();
                            app.show_preferences();
                        }
                        TrayCommand::Quit => {
                            app.quit_gracefully();
                        }
                    }
                }
            });
        }

        // Listen for StatusNotifierWatcher availability updates.
        {
            let app = self.clone();
            glib::spawn_future_local(async move {
                while let Ok(online) = watcher_rx.recv().await {
                    *app.imp().tray_watcher_online.borrow_mut() = Some(online);
                    if let Some(win) = app.existing_window() {
                        win.set_tray_warning(!online);
                    }
                }
            });
        }

        // Listen for background provider fetch results.
        {
            let app = self.clone();
            glib::spawn_future_local(async move {
                while let Ok(event) = data_rx.recv().await {
                    app.handle_data_event(event);
                }
            });
        }

        // 1-second live countdown + day-change & cache refresh tick.
        {
            let app = self.clone();
            glib::timeout_add_seconds_local(1, move || {
                app.on_tick();
                glib::ControlFlow::Continue
            });
        }

        // React to external or dialog GSettings changes immediately.
        {
            let app = self.clone();
            settings.raw().connect_changed(None, move |_, key| {
                match key {
                    "run-in-background" => app.sync_hold_guard(),
                    "show-tray-label" | "tray-label-format" | "show-seconds" => {
                        app.sync_tray_and_views(false);
                    }
                    _ => {}
                }
            });
        }
    }

    fn load_css(&self) {
        if let Some(display) = gdk::Display::default() {
            let icon_theme = gtk::IconTheme::for_display(&display);
            icon_theme.add_resource_path("/io/github/bykemalh/namaz_time/NamazTime/icons");

            let provider = gtk::CssProvider::new();
            provider.load_from_resource("/io/github/bykemalh/namaz_time/NamazTime/style.css");
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    fn settings(&self) -> &AppSettings {
        self.imp()
            .settings
            .get()
            .expect("AppSettings başlatılmadı")
    }

    fn controller(&self) -> &Rc<PrayerController> {
        self.imp()
            .controller
            .get()
            .expect("PrayerController başlatılmadı")
    }

    fn sync_hold_guard(&self) {
        let imp = self.imp();
        let should_hold = self.settings().run_in_background();
        let mut guard = imp.hold_guard.borrow_mut();
        if should_hold && guard.is_none() {
            *guard = Some(self.hold());
        } else if !should_hold && guard.is_some() {
            *guard = None;
        }
    }

    fn existing_window(&self) -> Option<NamazTimeWindow> {
        self.imp()
            .window
            .borrow()
            .as_ref()
            .and_then(|weak| weak.upgrade())
    }

    fn get_or_create_window(&self) -> NamazTimeWindow {
        if let Some(win) = self.existing_window() {
            return win;
        }

        let win = NamazTimeWindow::new(self);
        if let Some(online) = *self.imp().tray_watcher_online.borrow() {
            win.set_tray_warning(!online);
        }

        let settings = self.settings().clone();
        win.connect_close_request(move |w| {
            if settings.run_in_background() {
                w.set_visible(false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });

        *self.imp().window.borrow_mut() = Some(win.downgrade());
        win
    }

    fn toggle_window_visibility(&self) {
        if let Some(win) = self.existing_window() {
            if win.is_visible() && self.settings().run_in_background() {
                win.set_visible(false);
            } else {
                self.sync_window_view(&win);
                win.present();
            }
        } else {
            self.activate();
        }
    }

    fn quit_gracefully(&self) {
        info!("Namaz Vakti kapatılıyor");
        *self.imp().hold_guard.borrow_mut() = None;
        if let Some(win) = self.existing_window() {
            win.destroy();
        }
        self.quit();
    }

    fn load_profile(&self, profile: LocationProfile, force_fetch: bool) {
        let imp = self.imp();
        let controller = self.controller();

        let cached = controller.load_cached(&profile);
        let has_days = !cached.days.is_empty();
        let needs_fetch = force_fetch
            || !has_days
            || DiskCache::should_refresh(&cached, &profile.timezone, Timestamp::now())
            || PrayerController::compute_current_snapshot(&profile, &cached, false, None).is_err();

        *imp.current_profile.borrow_mut() = Some(profile.clone());
        *imp.cached_days.borrow_mut() = if has_days { Some(cached) } else { None };
        *imp.is_stale.borrow_mut() = false;
        *imp.stale_reason.borrow_mut() = None;
        *imp.last_error.borrow_mut() = None;

        if needs_fetch {
            self.start_fetch(profile, force_fetch, false);
        }

        self.sync_tray_and_views(true);
    }

    fn trigger_manual_refresh(&self) {
        let Some(profile) = self.imp().current_profile.borrow().clone() else {
            if let Some(win) = self.existing_window() {
                win.show_toast(&tr("Select a location first"));
            }
            return;
        };
        if *self.imp().is_fetching.borrow() {
            return;
        }
        self.start_fetch(profile, true, true);
        if let Some(win) = self.existing_window() {
            self.sync_window_view(&win);
        }
    }

    fn start_fetch(&self, profile: LocationProfile, force_refresh: bool, is_manual: bool) {
        let imp = self.imp();
        *imp.is_fetching.borrow_mut() = true;
        *imp.manual_refresh_pending.borrow_mut() = is_manual;
        *imp.last_fetch_attempt_unix.borrow_mut() = Timestamp::now().as_second();

        if let Some(win) = self.existing_window() {
            win.set_refreshing(true);
        }

        self.controller().trigger_fetch(
            profile,
            self.settings().preferred_source(),
            force_refresh,
        );
    }

    fn handle_data_event(&self, event: DataEvent) {
        let imp = self.imp();
        let active_key = imp
            .current_profile
            .borrow()
            .as_ref()
            .map(|p| p.cache_key());

        match event {
            DataEvent::TimesLoaded {
                location_key,
                result,
            } => {
                if active_key.as_deref() != Some(location_key.as_str()) {
                    debug!("Eski konuma ait veri yoksayıldı: {location_key}");
                    return;
                }

                let was_manual = imp.manual_refresh_pending.replace(false);
                *imp.is_fetching.borrow_mut() = false;
                *imp.cached_days.borrow_mut() = Some(result.cached);
                *imp.is_stale.borrow_mut() = result.stale;
                *imp.stale_reason.borrow_mut() = result.stale_reason.clone();
                *imp.last_error.borrow_mut() = None;

                if let Some(win) = self.existing_window() {
                    win.set_refreshing(false);
                    if result.stale {
                        if let Some(ref notice) = result.stale_reason {
                            if was_manual {
                                win.show_toast(notice);
                            }
                        }
                    } else if was_manual {
                        win.show_toast(&tr("Prayer times updated"));
                    }
                }

                self.sync_tray_and_views(true);
            }
            DataEvent::TimesError {
                location_key,
                message,
            } => {
                if active_key.as_deref() != Some(location_key.as_str()) {
                    return;
                }

                warn!("Vakit yükleme hatası: {message}");
                let was_manual = imp.manual_refresh_pending.replace(false);
                *imp.is_fetching.borrow_mut() = false;

                let has_valid_today = self.compute_active_snapshot().is_some();
                if has_valid_today {
                    *imp.is_stale.borrow_mut() = true;
                    *imp.stale_reason.borrow_mut() = Some(tr_fmt(
                        "Update failed; showing cached schedule (%1)",
                        &[&message],
                    ));
                    *imp.last_error.borrow_mut() = None;
                } else {
                    *imp.last_error.borrow_mut() = Some(message.clone());
                }

                if let Some(win) = self.existing_window() {
                    win.set_refreshing(false);
                    if was_manual {
                        win.show_toast(&tr_fmt("Refresh failed: %1", &[&message]));
                    }
                }

                self.sync_tray_and_views(true);
            }
        }
    }

    fn on_tick(&self) {
        let imp = self.imp();
        let profile_opt = imp.current_profile.borrow().clone();

        if let Some(profile) = profile_opt {
            let should_auto_refresh = match imp.cached_days.borrow().as_ref() {
                Some(cached) => {
                    DiskCache::should_refresh(cached, &profile.timezone, Timestamp::now())
                        || PrayerController::compute_current_snapshot(&profile, cached, false, None)
                            .is_err()
                }
                None => true,
            };

            if should_auto_refresh && !*imp.is_fetching.borrow() {
                let now = Timestamp::now().as_second();
                let last_attempt = *imp.last_fetch_attempt_unix.borrow();
                if now - last_attempt >= 300 {
                    self.start_fetch(profile, false, false);
                }
            }
        }

        self.sync_tray_and_views(false);
    }

    fn compute_active_snapshot(&self) -> Option<crate::models::ScheduleSnapshot> {
        let imp = self.imp();
        let profile = imp.current_profile.borrow();
        let cached = imp.cached_days.borrow();
        let stale = *imp.is_stale.borrow();
        let stale_reason = imp.stale_reason.borrow().clone();

        match (profile.as_ref(), cached.as_ref()) {
            (Some(loc), Some(days)) => {
                PrayerController::compute_current_snapshot(loc, days, stale, stale_reason).ok()
            }
            _ => None,
        }
    }

    fn sync_tray_and_views(&self, force_tray: bool) {
        let imp = self.imp();
        let settings = self.settings();
        let profile = imp.current_profile.borrow().clone();
        let snapshot = self.compute_active_snapshot();
        let error_msg = if snapshot.is_none() {
            imp.last_error.borrow().clone()
        } else {
            None
        };

        if let Some(win) = self.existing_window() {
            if win.is_visible() || force_tray {
                self.render_to_window(
                    &win,
                    profile.as_ref(),
                    snapshot.as_ref(),
                    error_msg.as_deref(),
                );
            }
        }

        let bucket = match (profile.as_ref(), snapshot.as_ref(), error_msg.as_ref()) {
            (Some(loc), Some(snap), _) => TrayUpdateBucket {
                location_key: Some(loc.cache_key()),
                today_date: Some(snap.today.date.clone()),
                current_prayer: Some(snap.current_prayer),
                next_prayer: snap.next_prayer.as_ref().map(|n| n.name),
                minute_bucket: snap.next_prayer.as_ref().map(|n| {
                    let s = n.remaining_seconds.max(0);
                    (s == 0, s < 60, s / 60)
                }),
                show_panel_label: settings.show_tray_label(),
                panel_format: settings.tray_label_format(),
                error_msg: None,
            },
            (Some(loc), None, Some(err)) => TrayUpdateBucket {
                location_key: Some(loc.cache_key()),
                today_date: None,
                current_prayer: None,
                next_prayer: None,
                minute_bucket: None,
                show_panel_label: settings.show_tray_label(),
                panel_format: settings.tray_label_format(),
                error_msg: Some(err.clone()),
            },
            (Some(loc), None, None) => TrayUpdateBucket {
                location_key: Some(loc.cache_key()),
                today_date: None,
                current_prayer: None,
                next_prayer: None,
                minute_bucket: None,
                show_panel_label: settings.show_tray_label(),
                panel_format: settings.tray_label_format(),
                error_msg: Some("__loading__".to_string()),
            },
            (None, _, _) => TrayUpdateBucket {
                location_key: None,
                today_date: None,
                current_prayer: None,
                next_prayer: None,
                minute_bucket: None,
                show_panel_label: settings.show_tray_label(),
                panel_format: settings.tray_label_format(),
                error_msg: None,
            },
        };

        let should_update_tray =
            force_tray || imp.last_tray_bucket.borrow().as_ref() != Some(&bucket);
        if should_update_tray {
            *imp.last_tray_bucket.borrow_mut() = Some(bucket);
            self.controller().update_tray_and_portal(
                profile.as_ref(),
                snapshot.as_ref(),
                error_msg.as_deref(),
                settings,
            );
        }
    }

    fn sync_window_view(&self, win: &NamazTimeWindow) {
        let imp = self.imp();
        let profile = imp.current_profile.borrow().clone();
        let snapshot = self.compute_active_snapshot();
        let error_msg = if snapshot.is_none() {
            imp.last_error.borrow().clone()
        } else {
            None
        };
        self.render_to_window(
            win,
            profile.as_ref(),
            snapshot.as_ref(),
            error_msg.as_deref(),
        );
    }

    fn render_to_window(
        &self,
        win: &NamazTimeWindow,
        profile: Option<&LocationProfile>,
        snapshot: Option<&crate::models::ScheduleSnapshot>,
        error_msg: Option<&str>,
    ) {
        let show_seconds = self.settings().show_seconds();
        match (profile, snapshot, error_msg) {
            (None, _, _) => win.show_empty(),
            (Some(loc), Some(snap), _) => win.render_snapshot(loc, snap, show_seconds),
            (Some(loc), None, Some(err)) => win.show_error(loc, err),
            (Some(loc), None, None) => win.show_loading(loc),
        }
    }

    fn show_location_picker(&self) {
        let win = self.get_or_create_window();
        if !win.is_visible() {
            win.present();
        }
        let controller = self.controller();
        let app = self.clone();
        let settings = self.settings().clone();

        ui::show_location_dialog(
            &win,
            controller.coordinator(),
            controller.cache(),
            controller.runtime(),
            move |profile| {
                settings.set_location_profile(&profile);
                app.load_profile(profile, true);
            },
        );
    }

    fn show_preferences(&self) {
        let win = self.get_or_create_window();
        if !win.is_visible() {
            win.present();
        }
        let settings = self.settings().clone();
        let rt = self.controller().runtime();

        ui::show_preferences_dialog(
            &win,
            &settings,
            rt,
            {
                let app = self.clone();
                move || app.show_location_picker()
            },
            {
                let app = self.clone();
                move || {
                    app.sync_hold_guard();
                    let new_source = app.settings().preferred_source_id();
                    let source_changed = {
                        let mut prev = app.imp().last_preferred_source.borrow_mut();
                        if *prev != new_source {
                            *prev = new_source;
                            true
                        } else {
                            false
                        }
                    };
                    if source_changed {
                        if let Some(profile) = app.imp().current_profile.borrow().clone() {
                            app.start_fetch(profile, true, true);
                        }
                    }
                    app.sync_tray_and_views(true);
                }
            },
        );
    }

    fn show_shortcuts(&self) {
        let win = self.get_or_create_window();
        let builder = gtk::Builder::from_resource("/io/github/bykemalh/namaz_time/NamazTime/shortcuts-dialog.ui");
        if let Some(dialog) = builder.object::<adw::Dialog>("shortcuts_dialog") {
            dialog.present(Some(&win));
        }
    }

    fn show_about(&self) {
        let win = self.get_or_create_window();
        let credit_imsakiyem = tr("Imsakiyem API (Diyanet data)");
        let credit_mtopal = tr("mtopal Prayer Times v1.5 (Diyanet data)");
        let credit_idamedya = tr("IDA Medya Prayer Times API (Turkey / Diyanet data)");
        let about = adw::AboutDialog::builder()
            .application_name(tr("Prayer Times"))
            .application_icon("io.github.bykemalh.namaz_time.NamazTime")
            .developer_name("Kemal (@bykemalh)")
            .version(VERSION)
            .website("https://github.com/bykemalh/namaz-time")
            .issue_url("https://github.com/bykemalh/namaz-time/issues")
            .developers(vec!["Kemal https://github.com/bykemalh"])
            .comments(tr(
                "Native GNOME prayer times application with top-bar countdown, system tray menu, and three-stage Diyanet API fallback.",
            ))
            .license_type(gtk::License::MitX11)
            .translator_credits("Kemal (@bykemalh)")
            .copyright("© 2026 Kemal (@bykemalh)")
            .build();
        about.add_link(&tr("GitHub Profile"), "https://github.com/bykemalh");
        about.add_credit_section(
            Some(&tr("Data Sources")),
            &[
                credit_imsakiyem.as_str(),
                credit_mtopal.as_str(),
                credit_idamedya.as_str(),
            ],
        );

        about.present(Some(&win));
    }
}
