use adw::prelude::*;
use gtk::{gio, glib};

use crate::i18n::{request_restart, tr, tr_fmt};
use crate::models::{HeroTheme, TrayLabelFormat, UiStyle};
use crate::services::{AppSettings, portal};

pub fn show_preferences_dialog(
    parent: &impl IsA<gtk::Widget>,
    settings: &AppSettings,
    rt: &'static tokio::runtime::Runtime,
    on_change_location: impl Fn() + 'static,
    on_settings_changed: impl Fn() + 'static,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title(tr("Preferences"))
        .build();

    let page = adw::PreferencesPage::new();

    // --- Group 1: Location & Data Source ---
    let location_group = adw::PreferencesGroup::builder()
        .title(tr("Location and Data Source"))
        .build();

    let (loc_title, loc_subtitle) = match settings.location_profile() {
        Some(loc) => {
            let mut matched = Vec::new();
            if loc.provider_ids.imsakiyem.is_some() {
                matched.push(tr("Imsakiyem"));
            }
            if loc.provider_ids.mtopal.is_some() {
                matched.push("mtopal".to_string());
            }
            if loc.provider_ids.ezanvakti.is_some() {
                matched.push("EzanVakti".to_string());
            }
            if loc.lat.zip(loc.lon).is_some() || !loc.city_name.is_empty() {
                matched.push("AlAdhan".to_string());
            }
            let sources = if matched.is_empty() {
                tr("None")
            } else {
                matched.join(", ")
            };
            (
                loc.display_label,
                tr_fmt("Time zone: %1 • Providers: %2", &[&loc.timezone, &sources]),
            )
        }
        None => (
            tr("No location selected"),
            tr("Please choose a city or district"),
        ),
    };

    let loc_row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&loc_title))
        .subtitle(glib::markup_escape_text(&loc_subtitle))
        .build();
    let change_btn = gtk::Button::builder()
        .label(tr("Change"))
        .valign(gtk::Align::Center)
        .build();
    {
        let dialog = dialog.clone();
        change_btn.connect_clicked(move |_| {
            dialog.close();
            on_change_location();
        });
    }
    loc_row.add_suffix(&change_btn);
    location_group.add(&loc_row);

    let src_auto = tr("Automatic (Recommended)");
    let src_ims = tr("Imsakiyem");
    let src_ezan = tr("EzanVakti (emushaf.net)");
    let src_ala = tr("AlAdhan (Diyanet Method)");
    let source_options = gtk::StringList::new(&[
        src_auto.as_str(),
        src_ims.as_str(),
        "mtopal v1.5",
        src_ezan.as_str(),
        src_ala.as_str(),
    ]);
    let source_row = adw::ComboRow::builder()
        .title(tr("Preferred data source"))
        .subtitle(tr("Falls back to verified secondary providers if the primary source fails"))
        .model(&source_options)
        .build();

    let initial_source_idx = match settings.preferred_source_id().as_str() {
        "imsakiyem" => 1,
        "mtopal" => 2,
        "ezanvakti" => 3,
        "aladhan" => 4,
        _ => 0,
    };
    source_row.set_selected(initial_source_idx);
    location_group.add(&source_row);
    page.add(&location_group);

    // --- Group 2: Appearance ---
    let appearance_group = adw::PreferencesGroup::builder()
        .title(tr("Appearance"))
        .build();

    let style_labels: Vec<String> = UiStyle::ALL.iter().map(|s| s.label()).collect();
    let style_strs: Vec<&str> = style_labels.iter().map(String::as_str).collect();
    let style_options = gtk::StringList::new(&style_strs);
    let current_ui_style = settings.ui_style();
    let ui_style_row = adw::ComboRow::builder()
        .title(tr("Interface theme"))
        .subtitle(tr("Choose between visual scenes, clean colorful icons, or monochrome grey"))
        .model(&style_options)
        .build();
    let initial_style_idx = UiStyle::ALL
        .iter()
        .position(|&s| s == current_ui_style)
        .unwrap_or(0) as u32;
    ui_style_row.set_selected(initial_style_idx);

    let theme_labels: Vec<String> = HeroTheme::ALL.iter().map(|t| t.label()).collect();
    let theme_strs: Vec<&str> = theme_labels.iter().map(String::as_str).collect();
    let theme_options = gtk::StringList::new(&theme_strs);
    let hero_theme_row = adw::ComboRow::builder()
        .title(tr("Hero Card Scene"))
        .subtitle(tr("Choose the 3D mosque scene or dynamic sky for the countdown card"))
        .model(&theme_options)
        .sensitive(current_ui_style == UiStyle::Illustrated)
        .build();
    let initial_theme_idx = HeroTheme::ALL
        .iter()
        .position(|&t| t == settings.hero_theme())
        .unwrap_or(0) as u32;
    hero_theme_row.set_selected(initial_theme_idx);

    let colorful_icons_row = adw::SwitchRow::builder()
        .title(tr("Colorful prayer icons"))
        .subtitle(tr("Color each prayer icon to match its time of day"))
        .active(settings.colorful_icons())
        .sensitive(current_ui_style != UiStyle::Minimal)
        .build();

    appearance_group.add(&ui_style_row);
    appearance_group.add(&hero_theme_row);
    appearance_group.add(&colorful_icons_row);
    page.add(&appearance_group);

    // --- Group 3: GNOME Top Bar & Countdown ---
    let display_group = adw::PreferencesGroup::builder()
        .title(glib::markup_escape_text(&tr("GNOME Top Bar & Countdown")))
        .build();

    let show_tray_label_row = adw::SwitchRow::builder()
        .title(tr("Show remaining time in top bar"))
        .subtitle(tr("Display remaining time beside the tray icon in the GNOME top panel"))
        .active(settings.show_tray_label())
        .build();

    let fmt_both = tr("Prayer + Time (e.g. Asr 42m)");
    let fmt_time = tr("Time Only (e.g. 42m)");
    let format_options = gtk::StringList::new(&[fmt_both.as_str(), fmt_time.as_str()]);
    let tray_format_row = adw::ComboRow::builder()
        .title(tr("Panel label format"))
        .model(&format_options)
        .sensitive(settings.show_tray_label())
        .build();
    tray_format_row.set_selected(match settings.tray_label_format() {
        TrayLabelFormat::PrayerAndTime => 0,
        TrayLabelFormat::TimeOnly => 1,
    });

    let show_seconds_row = adw::SwitchRow::builder()
        .title(tr("Show seconds in main window"))
        .subtitle(tr("Display the countdown in HH:MM:SS or HH:MM format"))
        .active(settings.show_seconds())
        .build();

    display_group.add(&show_tray_label_row);
    display_group.add(&tray_format_row);
    display_group.add(&show_seconds_row);
    page.add(&display_group);

    // --- Group 3: Background & Autostart ---
    let bg_group = adw::PreferencesGroup::builder()
        .title(tr("Background Behavior"))
        .build();

    let run_bg_row = adw::SwitchRow::builder()
        .title(tr("Keep running in background on close"))
        .subtitle(tr("Keep the tray icon and countdown active when the window is hidden"))
        .active(settings.run_in_background())
        .build();

    let autostart_row = adw::SwitchRow::builder()
        .title(tr("Start automatically on login"))
        .subtitle(tr("Launch in the background at login using the Flatpak XDG Background Portal"))
        .active(settings.autostart())
        .build();

    bg_group.add(&run_bg_row);
    bg_group.add(&autostart_row);
    page.add(&bg_group);

    // --- Group 4: Language ---
    let lang_group = adw::PreferencesGroup::builder()
        .title(tr("Language"))
        .build();

    let lang_system = tr("System Default");
    let lang_options = gtk::StringList::new(&[
        lang_system.as_str(),
        "Türkçe",
        "English",
        "Русский",
    ]);
    let lang_row = adw::ComboRow::builder()
        .title(tr("Application language"))
        .subtitle(tr("Restart required to apply language changes"))
        .model(&lang_options)
        .build();

    let initial_lang_idx = match settings.app_language().as_str() {
        "tr" => 1,
        "en" => 2,
        "ru" => 3,
        _ => 0,
    };
    lang_row.set_selected(initial_lang_idx);
    lang_group.add(&lang_row);
    page.add(&lang_group);

    dialog.add(&page);

    // Wire settings signals cleanly using shared Rc callback
    let on_changed = std::rc::Rc::new(on_settings_changed);

    source_row.connect_selected_notify({
        let settings = settings.clone();
        let on_changed = on_changed.clone();
        move |row| {
            let id = match row.selected() {
                1 => "imsakiyem",
                2 => "mtopal",
                3 => "ezanvakti",
                4 => "aladhan",
                _ => "auto",
            };
            settings.set_preferred_source_id(id);
            on_changed();
        }
    });

    show_tray_label_row.connect_active_notify({
        let settings = settings.clone();
        let tray_format_row = tray_format_row.clone();
        let on_changed = on_changed.clone();
        move |row| {
            let active = row.is_active();
            settings.set_show_tray_label(active);
            tray_format_row.set_sensitive(active);
            on_changed();
        }
    });

    tray_format_row.connect_selected_notify({
        let settings = settings.clone();
        let on_changed = on_changed.clone();
        move |row| {
            let fmt = match row.selected() {
                1 => TrayLabelFormat::TimeOnly,
                _ => TrayLabelFormat::PrayerAndTime,
            };
            settings.set_tray_label_format(fmt);
            on_changed();
        }
    });

    show_seconds_row.connect_active_notify({
        let settings = settings.clone();
        let on_changed = on_changed.clone();
        move |row| {
            settings.set_show_seconds(row.is_active());
            on_changed();
        }
    });

    ui_style_row.connect_selected_notify({
        let settings = settings.clone();
        let hero_theme_row = hero_theme_row.clone();
        let colorful_icons_row = colorful_icons_row.clone();
        let on_changed = on_changed.clone();
        move |row| {
            let idx = row.selected() as usize;
            if let Some(&style) = UiStyle::ALL.get(idx) {
                settings.set_ui_style(style);
                hero_theme_row.set_sensitive(style == UiStyle::Illustrated);
                colorful_icons_row.set_sensitive(style != UiStyle::Minimal);
                on_changed();
            }
        }
    });

    hero_theme_row.connect_selected_notify({
        let settings = settings.clone();
        let on_changed = on_changed.clone();
        move |row| {
            let idx = row.selected() as usize;
            if let Some(&theme) = HeroTheme::ALL.get(idx) {
                settings.set_hero_theme(theme);
                on_changed();
            }
        }
    });

    colorful_icons_row.connect_active_notify({
        let settings = settings.clone();
        let on_changed = on_changed.clone();
        move |row| {
            settings.set_colorful_icons(row.is_active());
            on_changed();
        }
    });

    run_bg_row.connect_active_notify({
        let settings = settings.clone();
        let on_changed = on_changed.clone();
        move |row| {
            settings.set_run_in_background(row.is_active());
            on_changed();
        }
    });

    autostart_row.connect_active_notify({
        let settings = settings.clone();
        let on_changed = on_changed.clone();
        let dialog = dialog.clone();
        move |row| {
            let desired = row.is_active();
            settings.set_autostart(desired);

            let (tx, rx) = async_channel::bounded::<Result<(bool, bool), String>>(1);
            rt.spawn(async move {
                let res = portal::request_background(desired).await;
                let _ = tx.send(res).await;
            });

            let settings = settings.clone();
            let row = row.clone();
            let dialog = dialog.clone();
            let on_changed = on_changed.clone();
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok((_bg, auto))) => {
                        if auto != desired {
                            settings.set_autostart(auto);
                            row.set_active(auto);
                        }
                    }
                    Ok(Err(err)) => {
                        dialog.add_toast(adw::Toast::new(&err));
                    }
                    Err(_) => {}
                }
                on_changed();
            });
        }
    });

    lang_row.connect_selected_notify({
        let settings = settings.clone();
        let dialog = dialog.clone();
        move |row| {
            let code = match row.selected() {
                1 => "tr",
                2 => "en",
                3 => "ru",
                _ => "system",
            };
            if settings.app_language() == code {
                return;
            }
            settings.set_app_language(code);
            let toast = adw::Toast::builder()
                .title(tr("Restart the app to apply the language change"))
                .button_label(tr("Restart"))
                .timeout(10)
                .build();
            toast.connect_button_clicked(|_| {
                request_restart();
                if let Some(app) = gio::Application::default() {
                    app.quit();
                }
            });
            dialog.add_toast(toast);
        }
    });

    dialog.present(Some(parent));
}
