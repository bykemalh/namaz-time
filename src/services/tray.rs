use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;
use serde::Serialize;
use std::sync::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use zbus::names::InterfaceName;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, ObjectPath, OwnedVariant, Str, Structure, Type, Variant, fdo};

use crate::i18n::{tr, tr_fmt};
use crate::models::{
    PrayerName, ScheduleSnapshot, TrayLabelFormat, format_hm_words, format_tray_label,
};

const SNI_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";

#[derive(Debug, Clone)]
pub enum TrayCommand {
    ShowWindow,
    ToggleWindow,
    Refresh,
    OpenPreferences,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayPrayerItem {
    pub name: PrayerName,
    pub time: String,
    pub is_current: bool,
    pub is_next: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayViewState {
    pub location_label: String,
    pub countdown_label: String,
    pub panel_label: String,
    pub tooltip_title: String,
    pub tooltip_desc: String,
    pub prayers: Vec<TrayPrayerItem>,
}

impl Default for TrayViewState {
    fn default() -> Self {
        Self {
            location_label: tr("No location set"),
            countdown_label: tr("Choose a location to begin"),
            panel_label: String::new(),
            tooltip_title: tr("Prayer Times"),
            tooltip_desc: tr("No location set"),
            prayers: Vec::new(),
        }
    }
}

impl TrayViewState {
    pub fn from_snapshot(
        location_title: &str,
        snap: &ScheduleSnapshot,
        show_panel_label: bool,
        panel_format: TrayLabelFormat,
    ) -> Self {
        let (countdown_label, panel_label, tooltip_desc) = if let Some(ref next) = snap.next_prayer
        {
            let words = format_hm_words(next.remaining_seconds);
            let cd = if next.is_tomorrow {
                tr_fmt(
                    "Tomorrow's %1 in %2 (%3)",
                    &[&next.name.label(), &words, &next.time_str],
                )
            } else {
                tr_fmt(
                    "%1 in %2 (%3)",
                    &[&next.name.label(), &words, &next.time_str],
                )
            };
            let pl = if show_panel_label {
                format_tray_label(next, panel_format)
            } else {
                String::new()
            };
            let td = tr_fmt(
                "%1\nNow: %2 • %3",
                &[location_title, &snap.current_prayer.label(), &cd],
            );
            (cd, pl, td)
        } else {
            let cd = tr_fmt("Now: %1", &[&snap.current_prayer.label()]);
            (cd.clone(), String::new(), format!("{location_title}\n{cd}"))
        };

        let next_name = snap.next_prayer.as_ref().map(|n| n.name);
        let prayers = PrayerName::ALL
            .iter()
            .map(|&name| TrayPrayerItem {
                name,
                time: snap.today.time_for(name).to_string(),
                is_current: name == snap.current_prayer,
                is_next: next_name == Some(name) && !snap.next_prayer.as_ref().is_some_and(|n| n.is_tomorrow),
            })
            .collect();

        Self {
            location_label: location_title.to_string(),
            countdown_label,
            panel_label,
            tooltip_title: tr_fmt("Prayer Times — %1", &[location_title]),
            tooltip_desc,
            prayers,
        }
    }

    pub fn loading(location_title: &str) -> Self {
        let loading_msg = tr("Fetching prayer times…");
        Self {
            location_label: location_title.to_string(),
            countdown_label: loading_msg.clone(),
            panel_label: String::new(),
            tooltip_title: tr_fmt("Prayer Times — %1", &[location_title]),
            tooltip_desc: loading_msg,
            prayers: Vec::new(),
        }
    }

    pub fn error(location_title: &str, message: &str) -> Self {
        Self {
            location_label: location_title.to_string(),
            countdown_label: message.to_string(),
            panel_label: String::new(),
            tooltip_title: tr_fmt("Prayer Times — %1", &[location_title]),
            tooltip_desc: message.to_string(),
            prayers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Type, Serialize)]
pub struct SniPixmap {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, Type, Serialize)]
pub struct SniToolTip {
    pub icon_name: String,
    pub icon_pixmap: Vec<SniPixmap>,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Default, Type, Serialize)]
pub struct MenuLayout {
    pub id: i32,
    pub properties: HashMap<Cow<'static, str>, OwnedVariant>,
    pub children: Vec<Variant<'static>>,
}

impl<'a> From<MenuLayout> for Variant<'a> {
    fn from(s: MenuLayout) -> Self {
        Variant::from(
            Structure::builder()
                .add_field(s.id)
                .add_field(s.properties)
                .add_field(s.children)
                .build()
                .unwrap(),
        )
    }
}

struct SharedTrayState {
    view: TrayViewState,
    revision: u32,
    pixmaps: Vec<SniPixmap>,
}

#[derive(Clone)]
pub struct TrayHandle {
    state: Arc<RwLock<SharedTrayState>>,
    update_tx: mpsc::UnboundedSender<()>,
}

impl TrayHandle {
    pub fn update_state(&self, new_view: TrayViewState) {
        let mut guard = self.state.write().expect("tray lock poisoned");
        if guard.view != new_view {
            guard.view = new_view;
            guard.revision = guard.revision.wrapping_add(1);
            let _ = self.update_tx.send(());
        }
    }
}

pub fn spawn_tray_service(
    rt: &tokio::runtime::Runtime,
    cmd_tx: async_channel::Sender<TrayCommand>,
    watcher_tx: async_channel::Sender<bool>,
) -> TrayHandle {
    let pixmaps = vec![
        render_crescent_pixmap(22),
        render_crescent_pixmap(32),
        render_crescent_pixmap(48),
    ];

    let state = Arc::new(RwLock::new(SharedTrayState {
        view: TrayViewState::default(),
        revision: 1,
        pixmaps,
    }));

    let (update_tx, update_rx) = mpsc::unbounded_channel();
    let handle = TrayHandle {
        state: state.clone(),
        update_tx,
    };

    rt.spawn(async move {
        if let Err(err) = run_tray_loop(state, update_rx, cmd_tx, watcher_tx).await {
            warn!("Sistem tepsisi D-Bus döngüsü durdu: {err}");
        }
    });

    handle
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    async fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

struct SniInterface {
    state: Arc<RwLock<SharedTrayState>>,
    cmd_tx: async_channel::Sender<TrayCommand>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl SniInterface {
    fn context_menu(&self, _x: i32, _y: i32) -> fdo::Result<()> {
        Err(fdo::Error::UnknownMethod("Use DBusMenu".into()))
    }

    fn activate(&self, _x: i32, _y: i32) -> fdo::Result<()> {
        let _ = self.cmd_tx.send_blocking(TrayCommand::ShowWindow);
        Ok(())
    }

    fn secondary_activate(&self, _x: i32, _y: i32) -> fdo::Result<()> {
        let _ = self.cmd_tx.send_blocking(TrayCommand::ToggleWindow);
        Ok(())
    }

    #[zbus(name = "XAyatanaSecondaryActivate")]
    fn x_ayatana_secondary_activate(&self, _timestamp: u32) -> fdo::Result<()> {
        let _ = self.cmd_tx.send_blocking(TrayCommand::ToggleWindow);
        Ok(())
    }

    fn scroll(&self, _delta: i32, _orientation: &str) -> fdo::Result<()> {
        Ok(())
    }

    fn provide_xdg_activation_token(&self, _token: String) -> fdo::Result<()> {
        Ok(())
    }

    #[zbus(property)]
    fn category(&self) -> fdo::Result<String> {
        Ok("ApplicationStatus".to_string())
    }

    #[zbus(property)]
    fn id(&self) -> fdo::Result<String> {
        Ok("io.github.bykemalh.namaz_time.NamazTime".to_string())
    }

    #[zbus(property)]
    fn title(&self) -> fdo::Result<String> {
        Ok(tr("Prayer Times"))
    }

    #[zbus(property)]
    fn status(&self) -> fdo::Result<String> {
        Ok("Active".to_string())
    }

    #[zbus(property)]
    fn window_id(&self) -> fdo::Result<i32> {
        Ok(0)
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> fdo::Result<String> {
        Ok(String::new())
    }

    #[zbus(property)]
    fn menu(&self) -> fdo::Result<ObjectPath<'static>> {
        Ok(ObjectPath::from_static_str_unchecked(MENU_PATH))
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    #[zbus(property)]
    fn icon_name(&self) -> fdo::Result<String> {
        Ok(String::new())
    }

    #[zbus(property)]
    async fn icon_pixmap(&self) -> fdo::Result<Vec<SniPixmap>> {
        Ok(self.state.read().expect("tray lock poisoned").pixmaps.clone())
    }

    #[zbus(property)]
    async fn tool_tip(&self) -> fdo::Result<SniToolTip> {
        let guard = self.state.read().expect("tray lock poisoned");
        Ok(SniToolTip {
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
            title: guard.view.tooltip_title.clone(),
            description: guard.view.tooltip_desc.clone(),
        })
    }

    #[zbus(property, name = "XAyatanaLabel")]
    async fn x_ayatana_label(&self) -> fdo::Result<String> {
        Ok(self.state.read().expect("tray lock poisoned").view.panel_label.clone())
    }

    #[zbus(property, name = "XAyatanaLabelGuide")]
    fn x_ayatana_label_guide(&self) -> fdo::Result<String> {
        Ok(tr("Maghrib 12h 59m"))
    }

    #[zbus(signal)]
    async fn new_tool_tip(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal, name = "XAyatanaNewLabel")]
    async fn x_ayatana_new_label(
        ctxt: &SignalEmitter<'_>,
        label: &str,
        guide: &str,
    ) -> zbus::Result<()>;
}

const ID_LOCATION: i32 = 1;
const ID_COUNTDOWN: i32 = 2;
const ID_SEP_TOP: i32 = 3;
const ID_PRAYER_BASE: i32 = 10; // 10..=15
const ID_SEP_BOTTOM: i32 = 20;
const ID_SHOW_WINDOW: i32 = 21;
const ID_REFRESH: i32 = 22;
const ID_PREFS: i32 = 23;
const ID_QUIT: i32 = 24;

struct DbusMenuInterface {
    state: Arc<RwLock<SharedTrayState>>,
    cmd_tx: async_channel::Sender<TrayCommand>,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenuInterface {
    #[zbus(property)]
    fn version(&self) -> fdo::Result<u32> {
        Ok(3)
    }

    #[zbus(property)]
    fn text_direction(&self) -> fdo::Result<String> {
        Ok("ltr".to_string())
    }

    #[zbus(property)]
    fn status(&self) -> fdo::Result<String> {
        Ok("normal".to_string())
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> fdo::Result<(u32, MenuLayout)> {
        if parent_id != 0 {
            return Err(fdo::Error::InvalidArgs("Unknown parentId".into()));
        }
        let guard = self.state.read().expect("tray lock poisoned");
        let children = if recursion_depth == 0 {
            Vec::new()
        } else {
            build_menu_items(&guard.view)
                .into_iter()
                .map(|(id, props)| {
                    let filtered = filter_props(props, &property_names);
                    Variant::from(MenuLayout {
                        id,
                        properties: filtered,
                        children: Vec::new(),
                    })
                })
                .collect()
        };

        let mut root_props = HashMap::new();
        root_props.insert(
            Cow::Borrowed("children-display"),
            OwnedVariant::from(Str::from("submenu")),
        );

        Ok((
            guard.revision,
            MenuLayout {
                id: 0,
                properties: filter_props(root_props, &property_names),
                children,
            },
        ))
    }

    async fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> fdo::Result<Vec<(i32, HashMap<Cow<'static, str>, OwnedVariant>)>> {
        let guard = self.state.read().expect("tray lock poisoned");
        let items = build_menu_items(&guard.view);
        let result = items
            .into_iter()
            .filter(|(id, _)| ids.is_empty() || ids.contains(id))
            .map(|(id, props)| (id, filter_props(props, &property_names)))
            .collect();
        Ok(result)
    }

    async fn get_property(&self, id: i32, name: String) -> fdo::Result<OwnedVariant> {
        let guard = self.state.read().expect("tray lock poisoned");
        let items = build_menu_items(&guard.view);
        let (_, props) = items
            .into_iter()
            .find(|(item_id, _)| *item_id == id)
            .ok_or_else(|| fdo::Error::InvalidArgs("id not found".into()))?;
        props
            .into_iter()
            .find(|(k, _)| k.as_ref() == name.as_str())
            .map(|(_, v)| v)
            .ok_or_else(|| fdo::Error::InvalidArgs("property not found".into()))
    }

    async fn event(
        &self,
        id: i32,
        event_id: String,
        _data: OwnedVariant,
        _timestamp: u32,
    ) -> fdo::Result<()> {
        if event_id == "clicked" {
            let cmd = match id {
                ID_LOCATION | ID_COUNTDOWN | ID_SHOW_WINDOW => Some(TrayCommand::ShowWindow),
                i if (ID_PRAYER_BASE..ID_PRAYER_BASE + 6).contains(&i) => {
                    Some(TrayCommand::ShowWindow)
                }
                ID_REFRESH => Some(TrayCommand::Refresh),
                ID_PREFS => Some(TrayCommand::OpenPreferences),
                ID_QUIT => Some(TrayCommand::Quit),
                _ => None,
            };
            if let Some(cmd) = cmd {
                let _ = self.cmd_tx.send(cmd).await;
            }
        }
        Ok(())
    }

    async fn event_group(
        &self,
        events: Vec<(i32, String, OwnedVariant, u32)>,
    ) -> fdo::Result<Vec<i32>> {
        for (id, event_id, data, ts) in events {
            let _ = self.event(id, event_id, data, ts).await;
        }
        Ok(Vec::new())
    }

    fn about_to_show(&self, _id: i32) -> fdo::Result<bool> {
        Ok(false)
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> fdo::Result<(Vec<i32>, Vec<i32>)> {
        Ok((Vec::new(), Vec::new()))
    }

    #[zbus(signal)]
    async fn layout_updated(
        ctxt: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn items_properties_updated(
        ctxt: &SignalEmitter<'_>,
        updated_props: Vec<(i32, HashMap<Cow<'static, str>, OwnedVariant>)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;
}

fn str_prop(s: &str) -> OwnedVariant {
    OwnedVariant::from(Str::from(s.to_owned()))
}

fn bool_prop(b: bool) -> OwnedVariant {
    OwnedVariant::from(b)
}

fn i32_prop(v: i32) -> OwnedVariant {
    OwnedVariant::from(v)
}

fn standard_item(label: &str, enabled: bool) -> HashMap<Cow<'static, str>, OwnedVariant> {
    let mut props = HashMap::new();
    props.insert(Cow::Borrowed("type"), str_prop("standard"));
    props.insert(Cow::Borrowed("label"), str_prop(label));
    props.insert(Cow::Borrowed("enabled"), bool_prop(enabled));
    props.insert(Cow::Borrowed("visible"), bool_prop(true));
    props
}

fn separator_item() -> HashMap<Cow<'static, str>, OwnedVariant> {
    let mut props = HashMap::new();
    props.insert(Cow::Borrowed("type"), str_prop("separator"));
    props.insert(Cow::Borrowed("visible"), bool_prop(true));
    props
}

fn build_menu_items(view: &TrayViewState) -> Vec<(i32, HashMap<Cow<'static, str>, OwnedVariant>)> {
    let mut items = Vec::new();
    items.push((ID_LOCATION, standard_item(&format!("📍 {}", view.location_label), true)));
    items.push((ID_COUNTDOWN, standard_item(&format!("⏳ {}", view.countdown_label), true)));
    items.push((ID_SEP_TOP, separator_item()));

    for (idx, prayer) in view.prayers.iter().enumerate() {
        let marker = if prayer.is_next {
            "▸ "
        } else if prayer.is_current {
            "• "
        } else {
            "  "
        };
        let suffix = if prayer.is_next {
            tr("  (Next)")
        } else if prayer.is_current {
            tr("  (Now)")
        } else {
            String::new()
        };
        let label = format!("{marker}{:<8}  {}{suffix}", prayer.name.label(), prayer.time);
        let mut props = standard_item(&label, true);
        if prayer.is_current {
            props.insert(Cow::Borrowed("toggle-type"), str_prop("radio"));
            props.insert(Cow::Borrowed("toggle-state"), i32_prop(1));
        }
        items.push((ID_PRAYER_BASE + idx as i32, props));
    }

    if !view.prayers.is_empty() {
        items.push((ID_SEP_BOTTOM, separator_item()));
    }

    items.push((ID_SHOW_WINDOW, standard_item(&tr("Show Window"), true)));
    items.push((ID_REFRESH, standard_item(&tr("Refresh Times"), true)));
    items.push((ID_PREFS, standard_item(&tr("Preferences"), true)));
    items.push((ID_QUIT, standard_item(&tr("Quit"), true)));

    items
}

fn filter_props(
    mut props: HashMap<Cow<'static, str>, OwnedVariant>,
    requested: &[String],
) -> HashMap<Cow<'static, str>, OwnedVariant> {
    if requested.is_empty() {
        return props;
    }
    props.retain(|k, _| requested.iter().any(|r| r == k.as_ref()));
    props
}

async fn run_tray_loop(
    state: Arc<RwLock<SharedTrayState>>,
    mut update_rx: mpsc::UnboundedReceiver<()>,
    cmd_tx: async_channel::Sender<TrayCommand>,
    watcher_tx: async_channel::Sender<bool>,
) -> zbus::Result<()> {
    let sni = SniInterface {
        state: state.clone(),
        cmd_tx: cmd_tx.clone(),
    };
    let menu = DbusMenuInterface {
        state: state.clone(),
        cmd_tx,
    };

    let conn = zbus::connection::Builder::session()
        .serve_at(SNI_PATH, sni)
        .serve_at(MENU_PATH, menu)
        .build()
        .await?;

    let unique_name = conn
        .unique_name()
        .expect("D-Bus unique name missing")
        .to_string();

    let watcher = StatusNotifierWatcherProxy::builder(&conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;

    let dbus = fdo::DBusProxy::new(&conn).await?;
    let mut name_changes = dbus
        .receive_name_owner_changed_with_args(&[(0, "org.kde.StatusNotifierWatcher")])
        .await?;

    match watcher.register_status_notifier_item(&unique_name).await {
        Ok(()) => {
            info!("StatusNotifierWatcher kaydı başarılı: {unique_name}");
            let _ = watcher_tx.send(true).await;
        }
        Err(err) => {
            debug!("StatusNotifierWatcher şu an bulunamadı: {err}");
            let _ = watcher_tx.send(false).await;
        }
    }

    let sni_iface = conn
        .object_server()
        .interface::<_, SniInterface>(SNI_PATH)
        .await?;
    let menu_iface = conn
        .object_server()
        .interface::<_, DbusMenuInterface>(MENU_PATH)
        .await?;

    let mut last_panel_label = String::new();
    let mut last_tooltip = (String::new(), String::new());

    loop {
        tokio::select! {
            Some(()) = update_rx.recv() => {
                let (revision, panel_label, tooltip, menu_items) = {
                    let guard = state.read().expect("tray lock poisoned");
                    (
                        guard.revision,
                        guard.view.panel_label.clone(),
                        (guard.view.tooltip_title.clone(), guard.view.tooltip_desc.clone()),
                        build_menu_items(&guard.view),
                    )
                };

                if panel_label != last_panel_label {
                    last_panel_label = panel_label.clone();
                    let guide = tr("Maghrib 12h 59m");
                    let _ = SniInterface::x_ayatana_new_label(
                        sni_iface.signal_emitter(),
                        &panel_label,
                        &guide,
                    )
                    .await;
                    let _ = emit_sni_properties_changed(&conn, &panel_label).await;
                }

                if tooltip != last_tooltip {
                    last_tooltip = tooltip;
                    let _ = SniInterface::new_tool_tip(sni_iface.signal_emitter()).await;
                }

                let _ = DbusMenuInterface::items_properties_updated(
                    menu_iface.signal_emitter(),
                    menu_items,
                    Vec::new(),
                )
                .await;
                let _ = DbusMenuInterface::layout_updated(
                    menu_iface.signal_emitter(),
                    revision,
                    0,
                )
                .await;
            }
            Some(signal) = name_changes.next() => {
                if let Ok(args) = signal.args() {
                    if args.new_owner.is_some() {
                        if watcher.register_status_notifier_item(&unique_name).await.is_ok() {
                            info!("StatusNotifierWatcher yeniden bağlandı.");
                            let _ = watcher_tx.send(true).await;
                        }
                    } else {
                        debug!("StatusNotifierWatcher çevrimdışı oldu.");
                        let _ = watcher_tx.send(false).await;
                    }
                }
            }
        }
    }
}

async fn emit_sni_properties_changed(conn: &Connection, panel_label: &str) -> zbus::Result<()> {
    let iface_name = InterfaceName::from_static_str_unchecked("org.kde.StatusNotifierItem");
    let mut changed: HashMap<&str, Variant<'_>> = HashMap::new();
    changed.insert("XAyatanaLabel", Variant::from(panel_label));
    conn.emit_signal(
        Option::<&str>::None,
        SNI_PATH,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        &(iface_name, changed, Vec::<&str>::new()),
    )
    .await
}

/// Generates an anti-aliased symbolic crescent & star icon in ARGB32 network byte order.
fn render_crescent_pixmap(size: i32) -> SniPixmap {
    let mut data = vec![0u8; (size * size * 4) as usize];
    let sub = 4;
    let inv_total = 1.0 / (sub * sub) as f32;

    // Bright soft white matching GNOME symbolic top-bar icons (#f2f2f2)
    let r = 0xf2u8;
    let g = 0xf2u8;
    let b = 0xf2u8;

    for py in 0..size {
        for px in 0..size {
            let mut coverage = 0.0f32;

            for sy in 0..sub {
                for sx in 0..sub {
                    let fx = (px as f32 + (sx as f32 + 0.5) / sub as f32) / size as f32 * 2.0 - 1.0;
                    let fy = (py as f32 + (sy as f32 + 0.5) / sub as f32) / size as f32 * 2.0 - 1.0;

                    // Outer circle & cutout circle forming crescent
                    let d_outer = ((fx + 0.12).powi(2) + fy.powi(2)).sqrt();
                    let d_cut = ((fx - 0.22).powi(2) + (fy + 0.14).powi(2)).sqrt();
                    let in_crescent = d_outer <= 0.74 && d_cut >= 0.56;

                    // Small crisp 4-point star inside the crescent opening
                    let sx_c = fx - 0.38;
                    let sy_c = fy + 0.20;
                    let in_star = (sx_c.abs().powf(0.65) + sy_c.abs().powf(0.65)) <= 0.34;

                    if in_crescent || in_star {
                        coverage += inv_total;
                    }
                }
            }

            let alpha = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
            let idx = ((py * size + px) * 4) as usize;
            data[idx] = alpha;
            data[idx + 1] = r;
            data[idx + 2] = g;
            data[idx + 3] = b;
        }
    }

    SniPixmap {
        width: size,
        height: size,
        data,
    }
}
