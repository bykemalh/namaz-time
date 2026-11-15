use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::models::{LocationCandidate, LocationProfile, normalize_name};
use crate::providers::{CountryOption, ProviderCoordinator, StateOption};
use crate::services::DiskCache;

pub fn show_location_dialog(
    parent: &impl IsA<gtk::Widget>,
    coordinator: Arc<ProviderCoordinator>,
    cache: Arc<DiskCache>,
    rt: &'static tokio::runtime::Runtime,
    on_selected: impl Fn(LocationProfile) + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title("Konum Seç")
        .content_width(430)
        .content_height(580)
        .build();

    let toast_overlay = adw::ToastOverlay::new();
    let nav_view = adw::NavigationView::new();
    toast_overlay.set_child(Some(&nav_view));
    dialog.set_child(Some(&toast_overlay));

    let on_selected = Rc::new(on_selected);
    let resolving_flag = Rc::new(RefCell::new(false));

    let pick_candidate = {
        let dialog = dialog.clone();
        let toast_overlay = toast_overlay.clone();
        let coordinator = coordinator.clone();
        let cache = cache.clone();
        let on_selected = on_selected.clone();
        let resolving_flag = resolving_flag.clone();

        Rc::new(move |candidate: LocationCandidate| {
            if *resolving_flag.borrow() {
                return;
            }
            *resolving_flag.borrow_mut() = true;
            toast_overlay.add_toast(adw::Toast::new("Konum eşleştiriliyor…"));

            let (tx, rx) = async_channel::bounded::<LocationProfile>(1);
            let coord = coordinator.clone();
            let c = cache.clone();
            rt.spawn(async move {
                let profile = coord.resolve_location_profile(candidate, &c).await;
                let _ = tx.send(profile).await;
            });

            let dialog = dialog.clone();
            let on_selected = on_selected.clone();
            let resolving_flag = resolving_flag.clone();
            glib::spawn_future_local(async move {
                if let Ok(profile) = rx.recv().await {
                    *resolving_flag.borrow_mut() = false;
                    on_selected(profile);
                    dialog.close();
                } else {
                    *resolving_flag.borrow_mut() = false;
                }
            });
        })
    };

    // --- Page 1: Global Search + Countries ---
    let root_toolbar = adw::ToolbarView::new();
    root_toolbar.add_top_bar(&adw::HeaderBar::new());

    let root_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root_box.set_margin_top(12);
    root_box.set_margin_bottom(12);
    root_box.set_margin_start(14);
    root_box.set_margin_end(14);

    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some("İlçe veya şehir ara (ör. Kadıköy, Berlin)…"));
    root_box.append(&search_entry);

    let root_stack = gtk::Stack::new();
    root_stack.set_vexpand(true);
    root_box.append(&root_stack);

    let country_list = gtk::ListBox::new();
    country_list.add_css_class("boxed-list");
    country_list.set_selection_mode(gtk::SelectionMode::None);

    let countries_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&wrap_clamp(&country_list))
        .build();

    let search_list = gtk::ListBox::new();
    search_list.add_css_class("boxed-list");
    search_list.set_selection_mode(gtk::SelectionMode::None);

    let search_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&wrap_clamp(&search_list))
        .build();

    let loading_box = centered_spinner("Konumlar yükleniyor…");
    let empty_search = adw::StatusPage::builder()
        .icon_name("system-search-symbolic")
        .title("Sonuç Bulunamadı")
        .description("Farklı bir yazım deneyin veya aşağıdan ülke seçerek ilerleyin.")
        .build();

    root_stack.add_named(&loading_box, Some("loading"));
    root_stack.add_named(&countries_scroll, Some("countries"));
    root_stack.add_named(&search_scroll, Some("search"));
    root_stack.add_named(&empty_search, Some("empty"));
    root_stack.set_visible_child_name("loading");

    root_toolbar.set_content(Some(&root_box));
    let root_page = adw::NavigationPage::builder()
        .title("Konum Seç")
        .child(&root_toolbar)
        .build();
    nav_view.add(&root_page);

    let countries_cache: Rc<RefCell<Vec<CountryOption>>> = Rc::new(RefCell::new(Vec::new()));

    // Load initial countries list
    {
        let (tx, rx) = async_channel::bounded::<Result<Vec<CountryOption>, String>>(1);
        let coord = coordinator.clone();
        let c = cache.clone();
        rt.spawn(async move {
            let res = coord.list_countries(&c).await;
            let _ = tx.send(res).await;
        });

        let root_stack = root_stack.clone();
        let country_list = country_list.clone();
        let countries_cache = countries_cache.clone();
        let nav_view = nav_view.clone();
        let coordinator = coordinator.clone();
        let cache = cache.clone();
        let pick_candidate = pick_candidate.clone();
        let toast_overlay = toast_overlay.clone();
        let search_entry = search_entry.clone();

        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(countries)) => {
                    *countries_cache.borrow_mut() = countries.clone();
                    populate_countries(
                        &country_list,
                        &countries,
                        &nav_view,
                        coordinator,
                        cache,
                        rt,
                        pick_candidate,
                        toast_overlay,
                    );
                    if search_entry.text().trim().chars().count() < 2 {
                        root_stack.set_visible_child_name("countries");
                    }
                }
                Ok(Err(err)) => {
                    toast_overlay.add_toast(adw::Toast::new(&err));
                    root_stack.set_visible_child_name("empty");
                }
                Err(_) => {}
            }
        });
    }

    // Live debounced search handler
    let search_generation = Rc::new(RefCell::new(0u64));
    let pending_source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

    search_entry.connect_search_changed({
        let root_stack = root_stack.clone();
        let search_list = search_list.clone();
        let countries_cache = countries_cache.clone();
        let country_list = country_list.clone();
        let coordinator = coordinator.clone();
        let cache = cache.clone();
        let pick_candidate = pick_candidate.clone();
        let search_generation = search_generation.clone();
        let pending_source = pending_source.clone();

        move |entry| {
            if let Some(source) = pending_source.borrow_mut().take() {
                source.remove();
            }

            let q = entry.text().trim().to_string();
            *search_generation.borrow_mut() += 1;
            let current_gen = *search_generation.borrow();

            if q.chars().count() < 2 {
                if countries_cache.borrow().is_empty() {
                    root_stack.set_visible_child_name("loading");
                } else {
                    root_stack.set_visible_child_name("countries");
                }
                let _ = &country_list;
                return;
            }

            root_stack.set_visible_child_name("loading");

            let root_stack = root_stack.clone();
            let search_list = search_list.clone();
            let coordinator = coordinator.clone();
            let cache = cache.clone();
            let pick_candidate = pick_candidate.clone();
            let search_generation = search_generation.clone();
            let pending_source_inner = pending_source.clone();

            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(280),
                move || {
                    *pending_source_inner.borrow_mut() = None;
                    let (tx, rx) =
                        async_channel::bounded::<Result<Vec<LocationCandidate>, String>>(1);
                    let coord = coordinator.clone();
                    let c = cache.clone();
                    let q_clone = q.clone();
                    rt.spawn(async move {
                        let res = coord.search_locations(&q_clone, &c).await;
                        let _ = tx.send(res).await;
                    });

                    glib::spawn_future_local(async move {
                        if *search_generation.borrow() != current_gen {
                            return;
                        }
                        match rx.recv().await {
                            Ok(Ok(results)) if !results.is_empty() => {
                                clear_listbox(&search_list);
                                for cand in results {
                                    let row = adw::ActionRow::builder()
                                        .title(glib::markup_escape_text(&cand.district_name))
                                        .subtitle(glib::markup_escape_text(&cand.display_label))
                                        .activatable(true)
                                        .build();
                                    let pick = pick_candidate.clone();
                                    row.connect_activated(move |_| pick(cand.clone()));
                                    search_list.append(&row);
                                }
                                root_stack.set_visible_child_name("search");
                            }
                            _ => {
                                root_stack.set_visible_child_name("empty");
                            }
                        }
                    });
                },
            );
            *pending_source.borrow_mut() = Some(id);
        }
    });

    dialog.present(Some(parent));
}

#[allow(clippy::too_many_arguments)]
fn populate_countries(
    listbox: &gtk::ListBox,
    countries: &[CountryOption],
    nav_view: &adw::NavigationView,
    coordinator: Arc<ProviderCoordinator>,
    cache: Arc<DiskCache>,
    rt: &'static tokio::runtime::Runtime,
    pick_candidate: Rc<dyn Fn(LocationCandidate)>,
    toast_overlay: adw::ToastOverlay,
) {
    clear_listbox(listbox);
    for country in countries {
        let subtitle = if !country.name_en.is_empty() && country.name_en != country.name {
            country.name_en.as_str()
        } else {
            ""
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&country.name))
            .subtitle(glib::markup_escape_text(subtitle))
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

        let country = country.clone();
        let nav_view = nav_view.clone();
        let coordinator = coordinator.clone();
        let cache = cache.clone();
        let pick_candidate = pick_candidate.clone();
        let toast_overlay = toast_overlay.clone();

        row.connect_activated(move |_| {
            push_states_page(
                &nav_view,
                country.clone(),
                coordinator.clone(),
                cache.clone(),
                rt,
                pick_candidate.clone(),
                toast_overlay.clone(),
            );
        });
        listbox.append(&row);
    }
}

fn push_states_page(
    nav_view: &adw::NavigationView,
    country: CountryOption,
    coordinator: Arc<ProviderCoordinator>,
    cache: Arc<DiskCache>,
    rt: &'static tokio::runtime::Runtime,
    pick_candidate: Rc<dyn Fn(LocationCandidate)>,
    toast_overlay: adw::ToastOverlay,
) {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(14);
    content.set_margin_end(14);

    let filter_entry = gtk::SearchEntry::new();
    filter_entry.set_placeholder_text(Some("İl veya bölge filtrele…"));
    content.append(&filter_entry);

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    content.append(&stack);

    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&wrap_clamp(&list))
        .build();
    let loading = centered_spinner("Şehirler yükleniyor…");
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&scroll, Some("list"));
    stack.set_visible_child_name("loading");

    toolbar.set_content(Some(&content));
    let page = adw::NavigationPage::builder()
        .title(country.name.as_str())
        .child(&toolbar)
        .build();
    nav_view.push(&page);

    let (tx, rx) = async_channel::bounded::<Result<Vec<StateOption>, String>>(1);
    let coord = coordinator.clone();
    let c = cache.clone();
    let country_req = country.clone();
    rt.spawn(async move {
        let res = coord.list_states(&country_req, &c).await;
        let _ = tx.send(res).await;
    });

    let all_states: Rc<RefCell<Vec<StateOption>>> = Rc::new(RefCell::new(Vec::new()));
    let nav_view_c = nav_view.clone();

    let render_states = {
        let list = list.clone();
        let country = country.clone();
        let nav_view = nav_view_c.clone();
        let coordinator = coordinator.clone();
        let cache = cache.clone();
        let pick_candidate = pick_candidate.clone();
        let toast_overlay = toast_overlay.clone();

        Rc::new(move |states: &[StateOption], query: &str| {
            clear_listbox(&list);
            let q = normalize_name(query);
            for st in states {
                if !q.is_empty() && !normalize_name(&st.name).contains(&q) {
                    continue;
                }
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&st.name))
                    .activatable(true)
                    .build();
                row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

                let st_c = st.clone();
                let country_c = country.clone();
                let nav_view_c = nav_view.clone();
                let coord_c = coordinator.clone();
                let cache_c = cache.clone();
                let pick_c = pick_candidate.clone();
                let toast_c = toast_overlay.clone();

                row.connect_activated(move |_| {
                    push_districts_page(
                        &nav_view_c,
                        country_c.clone(),
                        st_c.clone(),
                        coord_c.clone(),
                        cache_c.clone(),
                        rt,
                        pick_c.clone(),
                        toast_c.clone(),
                    );
                });
                list.append(&row);
            }
        })
    };

    filter_entry.connect_search_changed({
        let all_states = all_states.clone();
        let render_states = render_states.clone();
        move |entry| {
            render_states(&all_states.borrow(), &entry.text());
        }
    });

    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Ok(states)) => {
                *all_states.borrow_mut() = states.clone();
                render_states(&states, "");
                stack.set_visible_child_name("list");
            }
            Ok(Err(err)) => {
                toast_overlay.add_toast(adw::Toast::new(&err));
            }
            Err(_) => {}
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn push_districts_page(
    nav_view: &adw::NavigationView,
    country: CountryOption,
    state: StateOption,
    coordinator: Arc<ProviderCoordinator>,
    cache: Arc<DiskCache>,
    rt: &'static tokio::runtime::Runtime,
    pick_candidate: Rc<dyn Fn(LocationCandidate)>,
    toast_overlay: adw::ToastOverlay,
) {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(14);
    content.set_margin_end(14);

    let filter_entry = gtk::SearchEntry::new();
    filter_entry.set_placeholder_text(Some("İlçe filtrele…"));
    content.append(&filter_entry);

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    content.append(&stack);

    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&wrap_clamp(&list))
        .build();
    let loading = centered_spinner("İlçeler yükleniyor…");
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&scroll, Some("list"));
    stack.set_visible_child_name("loading");

    toolbar.set_content(Some(&content));
    let page = adw::NavigationPage::builder()
        .title(state.name.as_str())
        .child(&toolbar)
        .build();
    nav_view.push(&page);

    let (tx, rx) = async_channel::bounded::<Result<Vec<LocationCandidate>, String>>(1);
    rt.spawn(async move {
        let res = coordinator.list_districts(&country, &state, &cache).await;
        let _ = tx.send(res).await;
    });

    let all_districts: Rc<RefCell<Vec<LocationCandidate>>> = Rc::new(RefCell::new(Vec::new()));

    let render_districts = {
        let list = list.clone();
        let pick_candidate = pick_candidate.clone();
        Rc::new(move |items: &[LocationCandidate], query: &str| {
            clear_listbox(&list);
            let q = normalize_name(query);
            for item in items {
                if !q.is_empty() && !normalize_name(&item.district_name).contains(&q) {
                    continue;
                }
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&item.district_name))
                    .subtitle(glib::markup_escape_text(
                        item.timezone.as_deref().unwrap_or(""),
                    ))
                    .activatable(true)
                    .build();
                let cand = item.clone();
                let pick = pick_candidate.clone();
                row.connect_activated(move |_| pick(cand.clone()));
                list.append(&row);
            }
        })
    };

    filter_entry.connect_search_changed({
        let all_districts = all_districts.clone();
        let render_districts = render_districts.clone();
        move |entry| {
            render_districts(&all_districts.borrow(), &entry.text());
        }
    });

    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Ok(items)) => {
                *all_districts.borrow_mut() = items.clone();
                render_districts(&items, "");
                stack.set_visible_child_name("list");
            }
            Ok(Err(err)) => {
                toast_overlay.add_toast(adw::Toast::new(&err));
            }
            Err(_) => {}
        }
    });
}

fn wrap_clamp(child: &impl IsA<gtk::Widget>) -> adw::Clamp {
    adw::Clamp::builder()
        .maximum_size(460)
        .child(child)
        .build()
}

fn centered_spinner(text: &str) -> gtk::Box {
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 12);
    bx.set_valign(gtk::Align::Center);
    bx.set_halign(gtk::Align::Center);
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    spinner.set_size_request(28, 28);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    bx.append(&spinner);
    bx.append(&label);
    bx
}

fn clear_listbox(listbox: &gtk::ListBox) {
    while let Some(child) = listbox.first_child() {
        listbox.remove(&child);
    }
}

#[allow(dead_code)]
fn _unused_gio_check(_: &gio::Application) {}
