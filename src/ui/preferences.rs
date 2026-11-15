use adw::prelude::*;
use gtk::glib;

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
        .title("Tercihler")
        .build();

    let page = adw::PreferencesPage::new();

    // --- Group 1: Location & Data Source ---
    let location_group = adw::PreferencesGroup::builder()
        .title("Konum ve Veri Kaynağı")
        .build();

    let (loc_title, loc_subtitle) = match settings.location_profile() {
        Some(loc) => {
            let mut matched = Vec::new();
            if loc.provider_ids.imsakiyem.is_some() {
                matched.push("İmsakiyem");
            }
            if loc.provider_ids.mtopal.is_some() {
                matched.push("mtopal");
            }
            if loc.provider_ids.idamedya.is_some() {
                matched.push("İDA Medya");
            }
            let sources = if matched.is_empty() {
                "Yok".to_string()
            } else {
                matched.join(", ")
            };
            (
                loc.display_label,
                format!("Saat dilimi: {} • Sağlayıcılar: {sources}", loc.timezone),
            )
        }
        None => (
            "Konum seçilmedi".to_string(),
            "Lütfen bir şehir veya ilçe seçin".to_string(),
        ),
    };

    let loc_row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&loc_title))
        .subtitle(glib::markup_escape_text(&loc_subtitle))
        .build();
    let change_btn = gtk::Button::builder()
        .label("Değiştir")
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

    let source_options = gtk::StringList::new(&[
        "Otomatik (Önerilen)",
        "İmsakiyem",
        "mtopal v1.5",
        "İDA Medya (Yalnızca Türkiye)",
    ]);
    let source_row = adw::ComboRow::builder()
        .title("Tercih edilen veri kaynağı")
        .subtitle("Birincil kaynak başarısız olursa doğrulanan diğer kaynaklar yedeklenir")
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
        .title("GNOME Üst Çubuk ve Sayaç")
        .build();

    let show_tray_label_row = adw::SwitchRow::builder()
        .title("Üst çubukta kalan süreyi göster")
        .subtitle("GNOME üst panelindeki tepsi ikonunun yanında kalan süreyi gösterir")
        .active(settings.show_tray_label())
        .build();

    let format_options = gtk::StringList::new(&[
        "Vakit + Süre (ör. İkindi 42dk)",
        "Yalnızca Süre (ör. 42dk)",
    ]);
    let tray_format_row = adw::ComboRow::builder()
        .title("Panel metin biçimi")
        .model(&format_options)
        .sensitive(settings.show_tray_label())
        .build();
    tray_format_row.set_selected(match settings.tray_label_format() {
        TrayLabelFormat::PrayerAndTime => 0,
        TrayLabelFormat::TimeOnly => 1,
    });

    let show_seconds_row = adw::SwitchRow::builder()
        .title("Ana pencerede saniyeleri göster")
        .subtitle("Geri sayımı SS:DD:SN veya SS:DD biçiminde gösterir")
        .active(settings.show_seconds())
        .build();

    display_group.add(&show_tray_label_row);
    display_group.add(&tray_format_row);
    display_group.add(&show_seconds_row);
    page.add(&display_group);

    // --- Group 3: Background & Autostart ---
    let bg_group = adw::PreferencesGroup::builder()
        .title("Arka Plan Çalışması")
        .build();

    let run_bg_row = adw::SwitchRow::builder()
        .title("Pencere kapatılınca arka planda çalış")
        .subtitle("Tepsi ikonu ve geri sayım pencere gizlendiğinde de çalışmaya devam eder")
        .active(settings.run_in_background())
        .build();

    let autostart_row = adw::SwitchRow::builder()
        .title("Oturum açılışında otomatik başlat")
        .subtitle("Flatpak XDG Background Portalı ile girişte arka planda başlatır")
        .active(settings.autostart())
        .build();

    bg_group.add(&run_bg_row);
    bg_group.add(&autostart_row);
    page.add(&bg_group);

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

    dialog.present(Some(parent));
}
