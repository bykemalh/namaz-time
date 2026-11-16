use adw::prelude::*;
use gtk::{gio, glib};

use crate::i18n::{request_restart, tr, tr_fmt};
use crate::models::TrayLabelFormat;
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
            if loc.provider_ids.idamedya.is_some() {
                matched.push(tr("IDA Medya"));
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
    let src_ida = tr("IDA Medya (Turkey Only)");
    let source_options = gtk::StringList::new(&[
        src_auto.as_str(),
        src_ims.as_str(),
        "mtopal v1.5",
        src_ida.as_str(),
    ]);
    let source_row = adw::ComboRow::builder()
        .title(tr("Preferred data source"))
        .subtitle(tr("Falls back to verified secondary providers if the primary source fails"))
        .model(&source_options)
        .build();

    let initial_source_idx = match settings.preferred_source_id().as_str() {
        "imsakiyem" => 1,
        "mtopal" => 2,
        "idamedya" => 3,
        _ => 0,
    };
    source_row.set_selected(initial_source_idx);
    location_group.add(&source_row);
    page.add(&location_group);

    // --- Group 2: GNOME Top Bar & Countdown ---
    let display_group = adw::PreferencesGroup::builder()
        .title(tr("GNOME Top Bar & Countdown"))
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
                3 => "idamedya",
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
