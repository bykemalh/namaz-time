use std::cell::{OnceCell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::i18n::{localize_hijri, tr, tr_fmt};
use crate::models::{
    AppearanceConfig, HeroTheme, LocationProfile, PrayerName, ScheduleSnapshot, UiStyle, format_hm,
    format_hm_words, format_hms,
};
use crate::ui::set_css_class;

#[derive(Debug, Clone)]
struct PrayerRowWidgets {
    prayer: PrayerName,
    row: gtk::ListBoxRow,
    bg_pic: gtk::Picture,
    badge: gtk::Label,
    time_label: gtk::Label,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/bykemalh/namaz_time/NamazTime/window.ui")]
    pub struct NamazTimeWindow {
        #[template_child]
        pub location_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub location_button_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub refresh_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub status_banner: TemplateChild<adw::Banner>,
        #[template_child]
        pub tray_warning_banner: TemplateChild<adw::Banner>,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub empty_select_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub loading_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub error_retry_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub error_location_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub hero_card_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub hero_content_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub hero_media_overlay: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub hero_bg_picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub gregorian_date_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub hijri_date_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub current_prayer_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub current_prayer_badge: TemplateChild<gtk::Label>,
        #[template_child]
        pub next_prayer_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub countdown_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub countdown_words_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub interval_progress: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub timezone_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub prayers_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub source_label: TemplateChild<gtk::Label>,

        pub(super) prayer_rows: OnceCell<Vec<PrayerRowWidgets>>,
        pub(super) active_hero_resource: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NamazTimeWindow {
        const NAME: &'static str = "NamazTimeWindow";
        type Type = super::NamazTimeWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for NamazTimeWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_widgets();
        }
    }

    impl WidgetImpl for NamazTimeWindow {}
    impl WindowImpl for NamazTimeWindow {}
    impl ApplicationWindowImpl for NamazTimeWindow {}
    impl AdwApplicationWindowImpl for NamazTimeWindow {}
}

glib::wrapper! {
    pub struct NamazTimeWindow(ObjectSubclass<imp::NamazTimeWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl NamazTimeWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn setup_widgets(&self) {
        let imp = self.imp();

        let mut rows = Vec::with_capacity(PrayerName::ALL.len());
        for prayer in PrayerName::ALL {
            let row = gtk::ListBoxRow::builder()
                .activatable(false)
                .selectable(false)
                .build();
            row.add_css_class("prayer-row");

            let overlay = gtk::Overlay::builder()
                .overflow(gtk::Overflow::Hidden)
                .build();
            overlay.add_css_class("prayer-row-overlay");

            let bg_path = format!(
                "/io/github/bykemalh/namaz_time/NamazTime/backgrounds/prayer-{}.svg",
                prayer.css_key()
            );
            let bg_pic = gtk::Picture::for_resource(&bg_path);
            bg_pic.set_can_shrink(true);
            bg_pic.set_content_fit(gtk::ContentFit::Cover);
            overlay.set_child(Some(&bg_pic));

            let content = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .hexpand(true)
                .vexpand(true)
                .build();
            content.add_css_class("prayer-row-scrim");

            let icon = gtk::Image::builder()
                .icon_name(prayer.icon_name())
                .pixel_size(18)
                .valign(gtk::Align::Center)
                .build();
            icon.add_css_class("prayer-row-icon");
            icon.add_css_class(&format!("prayer-icon-{}", prayer.css_key()));

            let title = gtk::Label::builder()
                .label(prayer.label())
                .hexpand(true)
                .xalign(0.0)
                .valign(gtk::Align::Center)
                .build();
            title.add_css_class("prayer-row-title");

            let badge = gtk::Label::builder()
                .valign(gtk::Align::Center)
                .visible(false)
                .build();
            badge.add_css_class("pill-badge");

            let time_label = gtk::Label::builder()
                .label("--:--")
                .valign(gtk::Align::Center)
                .build();
            time_label.add_css_class("prayer-time-digits");

            content.append(&icon);
            content.append(&title);
            content.append(&badge);
            content.append(&time_label);

            overlay.add_overlay(&content);
            overlay.set_measure_overlay(&content, true);
            row.set_child(Some(&overlay));
            imp.prayers_list.append(&row);

            rows.push(PrayerRowWidgets {
                prayer,
                row,
                bg_pic,
                badge,
                time_label,
            });
        }
        let _ = imp.prayer_rows.set(rows);

        imp.location_button.connect_clicked(|btn| {
            let _ = btn.activate_action("app.choose-location", None);
        });
        imp.empty_select_button.connect_clicked(|btn| {
            let _ = btn.activate_action("app.choose-location", None);
        });
        imp.error_location_button.connect_clicked(|btn| {
            let _ = btn.activate_action("app.choose-location", None);
        });

        imp.refresh_button.connect_clicked(|btn| {
            let _ = btn.activate_action("app.refresh", None);
        });
        imp.error_retry_button.connect_clicked(|btn| {
            let _ = btn.activate_action("app.refresh", None);
        });

        imp.tray_warning_banner.connect_button_clicked(|banner| {
            banner.set_revealed(false);
        });
    }

    pub fn apply_appearance(&self, appearance: AppearanceConfig) {
        let imp = self.imp();
        for style in UiStyle::ALL {
            let cls = format!("ui-style-{}", style.id());
            set_css_class(self, &cls, style == appearance.ui_style);
        }
        let show_colorful_icons =
            appearance.colorful_icons && appearance.ui_style != UiStyle::Minimal;
        set_css_class(self, "icons-colorful", show_colorful_icons);

        let is_illustrated = appearance.ui_style == UiStyle::Illustrated;
        imp.hero_media_overlay.set_visible(is_illustrated);
        imp.hero_content_box.set_hexpand(!is_illustrated);

        for theme in HeroTheme::ALL {
            let cls = format!("hero-theme-{}", theme.id());
            set_css_class(&*imp.hero_card_box, &cls, is_illustrated && theme == appearance.hero_theme);
        }

        if let Some(rows) = imp.prayer_rows.get() {
            for item in rows {
                item.bg_pic.set_visible(is_illustrated);
            }
        }
    }

    pub fn set_location_title(&self, title: Option<&str>) {
        let fallback = tr("Choose Location");
        let label = title.unwrap_or(&fallback);
        self.imp().location_button_label.set_label(label);
    }

    pub fn set_refreshing(&self, refreshing: bool) {
        self.imp().refresh_button.set_sensitive(!refreshing);
    }

    pub fn show_empty(&self) {
        let imp = self.imp();
        self.set_location_title(None);
        imp.loading_spinner.stop();
        imp.status_banner.set_revealed(false);
        imp.main_stack.set_visible_child_name("empty");
    }

    pub fn show_loading(&self, profile: &LocationProfile) {
        let imp = self.imp();
        self.set_location_title(Some(&profile.short_title()));
        imp.status_banner.set_revealed(false);
        imp.loading_spinner.start();
        imp.main_stack.set_visible_child_name("loading");
    }

    pub fn show_error(&self, profile: &LocationProfile, message: &str) {
        let imp = self.imp();
        self.set_location_title(Some(&profile.short_title()));
        imp.loading_spinner.stop();
        imp.status_banner.set_revealed(false);
        imp.error_page.set_description(Some(message));
        imp.main_stack.set_visible_child_name("error");
    }

    pub fn set_tray_warning(&self, visible: bool) {
        self.imp().tray_warning_banner.set_revealed(visible);
    }

    pub fn set_stale_banner(&self, reason: Option<&str>) {
        let banner = &self.imp().status_banner;
        if let Some(msg) = reason {
            banner.set_title(msg);
            banner.set_revealed(true);
        } else {
            banner.set_revealed(false);
        }
    }

    pub fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.imp().toast_overlay.add_toast(toast);
    }

    pub fn render_snapshot(
        &self,
        profile: &LocationProfile,
        snap: &ScheduleSnapshot,
        show_seconds: bool,
        appearance: AppearanceConfig,
    ) {
        let imp = self.imp();
        self.apply_appearance(appearance);
        if appearance.ui_style == UiStyle::Illustrated {
            let hero_path = appearance.hero_theme.hero_resource(snap.current_prayer);
            if imp.active_hero_resource.borrow().as_deref() != Some(hero_path.as_str()) {
                *imp.active_hero_resource.borrow_mut() = Some(hero_path.clone());
                imp.hero_bg_picture.set_resource(Some(&hero_path));
            }
        }
        self.set_location_title(Some(&profile.short_title()));
        imp.loading_spinner.stop();
        if imp.main_stack.visible_child_name().as_deref() != Some("content") {
            imp.main_stack.set_visible_child_name("content");
        }

        let offline_fallback = tr("Showing offline cache");
        self.set_stale_banner(if snap.stale {
            snap.stale_reason
                .as_deref()
                .or(Some(offline_fallback.as_str()))
        } else {
            None
        });

        imp.gregorian_date_label.set_label(&snap.local_date_str);
        if let Some(ref hijri) = snap.today.hijri {
            let localized_hijri = localize_hijri(hijri);
            imp.hijri_date_label.set_label(&localized_hijri);
            imp.hijri_date_label.set_visible(true);
        } else {
            imp.hijri_date_label.set_visible(false);
        }

        imp.current_prayer_icon
            .set_icon_name(Some(snap.current_prayer.icon_name()));
        imp.current_prayer_badge
            .set_label(&tr_fmt("Now: %1", &[&snap.current_prayer.label()]));

        if let Some(ref next) = snap.next_prayer {
            let next_title = if next.is_tomorrow {
                tr_fmt(
                    "Time until tomorrow's %1 (%2)",
                    &[&next.name.label(), &next.time_str],
                )
            } else {
                tr_fmt(
                    "Time until %1 (%2)",
                    &[&next.name.label(), &next.time_str],
                )
            };
            imp.next_prayer_label.set_label(&next_title);

            let digits = if show_seconds {
                format_hms(next.remaining_seconds)
            } else {
                format_hm(next.remaining_seconds)
            };
            imp.countdown_label.set_label(&digits);
            imp.countdown_words_label
                .set_label(&format_hm_words(next.remaining_seconds));

            if let Some(fraction) = next.progress {
                imp.interval_progress.set_fraction(fraction);
                imp.interval_progress.set_visible(true);
            } else {
                imp.interval_progress.set_visible(false);
            }
        } else {
            imp.next_prayer_label
                .set_label(&tr("Waiting for tomorrow's Fajr time"));
            imp.countdown_label.set_label("--:--");
            imp.countdown_words_label
                .set_label(&tr("Countdown resumes once tomorrow's schedule is refreshed"));
            imp.interval_progress.set_visible(false);
        }

        imp.timezone_label
            .set_label(&tr_fmt("Time zone: %1", &[&snap.timezone]));

        let next_today = snap
            .next_prayer
            .as_ref()
            .filter(|n| !n.is_tomorrow)
            .map(|n| n.name);

        if let Some(rows) = imp.prayer_rows.get() {
            for item in rows {
                item.time_label.set_label(snap.today.time_for(item.prayer));

                let is_current = item.prayer == snap.current_prayer;
                let is_next = !is_current && next_today == Some(item.prayer);

                set_css_class(&item.row, "prayer-row-current", is_current);
                set_css_class(&item.row, "prayer-row-next", is_next);

                if is_current {
                    item.badge.set_label(&tr("Now"));
                    set_css_class(&item.badge, "pill-badge-current", true);
                    set_css_class(&item.badge, "pill-badge-next", false);
                    item.badge.set_visible(true);
                } else if is_next {
                    item.badge.set_label(&tr("Next"));
                    set_css_class(&item.badge, "pill-badge-current", false);
                    set_css_class(&item.badge, "pill-badge-next", true);
                    item.badge.set_visible(true);
                } else {
                    item.badge.set_visible(false);
                }
            }
        }

        imp.source_label
            .set_label(&tr_fmt("Source: %1", &[&snap.today.source.attribution_text()]));
    }
}
