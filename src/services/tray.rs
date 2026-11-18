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
    prayer_icons: HashMap<PrayerName, Vec<u8>>,
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
    let prayer_icons = render_prayer_menu_icons();

    let state = Arc::new(RwLock::new(SharedTrayState {
        view: TrayViewState::default(),
        revision: 1,
        pixmaps,
        prayer_icons,
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
            build_menu_items(&guard.view, &guard.prayer_icons)
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
        let items = build_menu_items(&guard.view, &guard.prayer_icons);
        let result = items
            .into_iter()
            .filter(|(id, _)| ids.is_empty() || ids.contains(id))
            .map(|(id, props)| (id, filter_props(props, &property_names)))
            .collect();
        Ok(result)
    }

    async fn get_property(&self, id: i32, name: String) -> fdo::Result<OwnedVariant> {
        let guard = self.state.read().expect("tray lock poisoned");
        let items = build_menu_items(&guard.view, &guard.prayer_icons);
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

fn bytes_prop(bytes: &[u8]) -> OwnedVariant {
    let arr = zbus::Array::from(bytes);
    OwnedVariant::try_from(Variant::Array(arr)).expect("byte array OwnedVariant")
}

fn standard_item(label: &str, enabled: bool) -> HashMap<Cow<'static, str>, OwnedVariant> {
    let mut props = HashMap::new();
    props.insert(Cow::Borrowed("type"), str_prop("standard"));
    props.insert(Cow::Borrowed("label"), str_prop(label));
    props.insert(Cow::Borrowed("enabled"), bool_prop(enabled));
    props.insert(Cow::Borrowed("visible"), bool_prop(true));
    props
}

fn icon_item(
    label: &str,
    icon_name: &str,
    enabled: bool,
) -> HashMap<Cow<'static, str>, OwnedVariant> {
    let mut props = standard_item(label, enabled);
    props.insert(Cow::Borrowed("icon-name"), str_prop(icon_name));
    props
}

fn separator_item() -> HashMap<Cow<'static, str>, OwnedVariant> {
    let mut props = HashMap::new();
    props.insert(Cow::Borrowed("type"), str_prop("separator"));
    props.insert(Cow::Borrowed("visible"), bool_prop(true));
    props
}

fn build_menu_items(
    view: &TrayViewState,
    prayer_icons: &HashMap<PrayerName, Vec<u8>>,
) -> Vec<(i32, HashMap<Cow<'static, str>, OwnedVariant>)> {
    let mut items = Vec::new();
    items.push((
        ID_LOCATION,
        icon_item(&view.location_label, "mark-location-symbolic", true),
    ));
    items.push((
        ID_COUNTDOWN,
        icon_item(&view.countdown_label, "alarm-symbolic", true),
    ));
    items.push((ID_SEP_TOP, separator_item()));

    for (idx, prayer) in view.prayers.iter().enumerate() {
        let suffix = if prayer.is_next {
            tr("  (Next)")
        } else if prayer.is_current {
            tr("  (Now)")
        } else {
            String::new()
        };
        let label = format!("{}  {}{suffix}", prayer.name.label(), prayer.time);
        let mut props = standard_item(&label, true);
        props.insert(Cow::Borrowed("toggle-type"), str_prop("radio"));
        props.insert(
            Cow::Borrowed("toggle-state"),
            i32_prop(i32::from(prayer.is_current)),
        );
        if let Some(png) = prayer_icons.get(&prayer.name) {
            props.insert(Cow::Borrowed("icon-data"), bytes_prop(png));
        }
        items.push((ID_PRAYER_BASE + idx as i32, props));
    }

    if !view.prayers.is_empty() {
        items.push((ID_SEP_BOTTOM, separator_item()));
    }

    items.push((
        ID_SHOW_WINDOW,
        icon_item(&tr("Show Window"), "focus-windows-symbolic", true),
    ));
    items.push((
        ID_REFRESH,
        icon_item(&tr("Refresh Times"), "view-refresh-symbolic", true),
    ));
    items.push((
        ID_PREFS,
        icon_item(&tr("Preferences"), "preferences-system-symbolic", true),
    ));
    items.push((
        ID_QUIT,
        icon_item(&tr("Quit"), "application-exit-symbolic", true),
    ));

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
                        build_menu_items(&guard.view, &guard.prayer_icons),
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

fn prayer_menu_color(prayer: PrayerName) -> (u8, u8, u8) {
    match prayer {
        PrayerName::Imsak => (0x26, 0xd0, 0xbe),  // Dawn Teal
        PrayerName::Gunes => (0xf9, 0x8a, 0x36),  // Sunrise Orange
        PrayerName::Ogle => (0xf5, 0xb9, 0x20),   // Midday Gold
        PrayerName::Ikindi => (0x30, 0xb2, 0xf0), // Afternoon Sky Blue
        PrayerName::Aksam => (0xf4, 0x62, 0x7c),  // Sunset Rose
        PrayerName::Yatsi => (0x9c, 0x80, 0xf8),  // Night Violet
    }
}

fn render_prayer_menu_icons() -> HashMap<PrayerName, Vec<u8>> {
    let mut map = HashMap::with_capacity(PrayerName::ALL.len());
    for prayer in PrayerName::ALL {
        if let Some(png) = render_prayer_png(prayer, 32) {
            map.insert(prayer, png);
        }
    }
    map
}

fn render_prayer_png(prayer: PrayerName, size: i32) -> Option<Vec<u8>> {
    let mut surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, size, size).ok()?;
    {
        let cr = gtk::cairo::Context::new(&surface).ok()?;
        cr.scale(size as f64 / 16.0, size as f64 / 16.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);

        match prayer {
            PrayerName::Imsak => {
                draw_crescent(&cr, 6.8, 6.0, 4.2, 2.0, -0.8, 3.6);
                draw_four_point_star(&cr, 12.1, 4.3, 1.8, 0.52);
                draw_rounded_rect(&cr, 1.5, 12.2, 13.0, 1.7, 0.85);
            }
            PrayerName::Gunes => {
                draw_vertical_arrow(&cr, true);
                draw_horizon_sun(&cr);
            }
            PrayerName::Ogle => {
                cr.arc(8.0, 8.0, 3.4, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                for step in 0..8 {
                    let _ = cr.save();
                    cr.translate(8.0, 8.0);
                    cr.rotate(step as f64 * std::f64::consts::FRAC_PI_4);
                    draw_rounded_rect(&cr, -0.8, -7.0, 1.6, 2.4, 0.8);
                    let _ = cr.restore();
                }
            }
            PrayerName::Ikindi => {
                cr.arc(5.2, 5.4, 2.4, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                for deg in [-90.0f64, -45.0, 0.0, 45.0] {
                    let _ = cr.save();
                    cr.translate(5.2, 5.4);
                    cr.rotate(deg.to_radians());
                    draw_rounded_rect(&cr, -0.7, -4.4, 1.4, 1.6, 0.7);
                    let _ = cr.restore();
                }
                for (cx, cy, r) in [(6.2, 11.6, 2.2), (9.3, 10.4, 2.9), (12.4, 11.7, 2.1)] {
                    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                }
                cr.rectangle(6.2, 11.3, 6.2, 2.5);
                let _ = cr.fill();
            }
            PrayerName::Aksam => {
                draw_vertical_arrow(&cr, false);
                draw_horizon_sun(&cr);
            }
            PrayerName::Yatsi => {
                draw_crescent(&cr, 7.6, 8.0, 5.8, 2.4, -0.9, 4.8);
                draw_four_point_star(&cr, 11.8, 4.2, 1.8, 0.52);
                draw_four_point_star(&cr, 13.2, 8.8, 1.3, 0.40);
            }
        }
    }

    surface.flush();
    let data = surface.data().ok()?;
    let (tr, tg, tb) = prayer_menu_color(prayer);
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for px in data.chunks_exact(4) {
        let argb = u32::from_ne_bytes([px[0], px[1], px[2], px[3]]);
        let alpha = (argb >> 24) as u8;
        rgba.push(tr);
        rgba.push(tg);
        rgba.push(tb);
        rgba.push(alpha);
    }

    Some(encode_rgba_png(size as u32, size as u32, &rgba))
}

fn draw_crescent(
    cr: &gtk::cairo::Context,
    cx: f64,
    cy: f64,
    outer_r: f64,
    cut_dx: f64,
    cut_dy: f64,
    cut_r: f64,
) {
    cr.arc(cx, cy, outer_r, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    cr.set_operator(gtk::cairo::Operator::Clear);
    cr.arc(cx + cut_dx, cy + cut_dy, cut_r, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    cr.set_operator(gtk::cairo::Operator::Over);
    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
}

fn draw_four_point_star(
    cr: &gtk::cairo::Context,
    cx: f64,
    cy: f64,
    outer_r: f64,
    inner_r: f64,
) {
    cr.move_to(cx, cy - outer_r);
    cr.line_to(cx + inner_r, cy - inner_r);
    cr.line_to(cx + outer_r, cy);
    cr.line_to(cx + inner_r, cy + inner_r);
    cr.line_to(cx, cy + outer_r);
    cr.line_to(cx - inner_r, cy + inner_r);
    cr.line_to(cx - outer_r, cy);
    cr.line_to(cx - inner_r, cy - inner_r);
    cr.close_path();
    let _ = cr.fill();
}

fn draw_rounded_rect(
    cr: &gtk::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
) {
    let r = r.min(w * 0.5).min(h * 0.5);
    let right = x + w;
    let bottom = y + h;
    cr.new_sub_path();
    cr.arc(right - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(right - r, bottom - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, bottom - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
    let _ = cr.fill();
}

fn draw_vertical_arrow(cr: &gtk::cairo::Context, up: bool) {
    let pts: &[(f64, f64)] = if up {
        &[
            (8.0, 1.2),
            (5.2, 4.2),
            (6.3, 5.2),
            (7.2, 4.2),
            (7.2, 6.8),
            (8.8, 6.8),
            (8.8, 4.2),
            (9.7, 5.2),
            (10.8, 4.2),
        ]
    } else {
        &[
            (8.0, 6.8),
            (10.8, 3.8),
            (9.7, 2.8),
            (8.8, 3.8),
            (8.8, 1.2),
            (7.2, 1.2),
            (7.2, 3.8),
            (6.3, 2.8),
            (5.2, 3.8),
        ]
    };
    cr.move_to(pts[0].0, pts[0].1);
    for &(x, y) in &pts[1..] {
        cr.line_to(x, y);
    }
    cr.close_path();
    let _ = cr.fill();
}

fn draw_horizon_sun(cr: &gtk::cairo::Context) {
    cr.arc(8.0, 11.2, 3.6, std::f64::consts::PI, std::f64::consts::TAU);
    cr.close_path();
    let _ = cr.fill();
    draw_rounded_rect(cr, 1.2, 10.0, 2.2, 1.5, 0.75);
    draw_rounded_rect(cr, 12.6, 10.0, 2.2, 1.5, 0.75);
    draw_rounded_rect(cr, 1.5, 12.6, 13.0, 1.7, 0.85);
}

/// Encodes straight RGBA pixels into a spec-compliant PNG using an uncompressed zlib stream.
fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row_bytes = (width * 4) as usize;
    let raw_len = height as usize * (1 + row_bytes);
    let mut raw = Vec::with_capacity(raw_len);
    for row in rgba.chunks_exact(row_bytes) {
        raw.push(0); // Filter type 0 (None)
        raw.extend_from_slice(row);
    }

    // Zlib wrapper around uncompressed deflate stored blocks (max 65535 bytes per block).
    let mut zlib = Vec::with_capacity(raw.len() + 16);
    zlib.push(0x78);
    zlib.push(0x01);
    let mut offset = 0usize;
    while offset < raw.len() {
        let chunk_len = (raw.len() - offset).min(65535);
        let is_final = offset + chunk_len == raw.len();
        zlib.push(u8::from(is_final));
        let len_u16 = chunk_len as u16;
        zlib.extend_from_slice(&len_u16.to_le_bytes());
        zlib.extend_from_slice(&(!len_u16).to_le_bytes());
        zlib.extend_from_slice(&raw[offset..offset + chunk_len]);
        offset += chunk_len;
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = Vec::with_capacity(64 + zlib.len());
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // Bit depth
    ihdr[9] = 6; // Color type: RGBA
    write_png_chunk(&mut png, b"IHDR", &ihdr);
    write_png_chunk(&mut png, b"IDAT", &zlib);
    write_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_png_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(tag);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for &b in data {
        s1 = (s1 + b as u32) % MOD;
        s2 = (s2 + s1) % MOD;
    }
    (s2 << 16) | s1
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_menu_uses_icons_instead_of_emojis() {
        let icons = render_prayer_menu_icons();
        assert_eq!(icons.len(), PrayerName::ALL.len());
        for prayer in PrayerName::ALL {
            let png = icons.get(&prayer).expect("missing prayer png");
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(png.len() > 100);
        }

        let view = TrayViewState {
            location_label: "İnegöl, Bursa".to_string(),
            countdown_label: "Fajr in 4 hr 50 min (06:14)".to_string(),
            panel_label: "Fajr 4h 50m".to_string(),
            tooltip_title: "Prayer Times — İnegöl".to_string(),
            tooltip_desc: "Now: Isha".to_string(),
            prayers: vec![
                TrayPrayerItem {
                    name: PrayerName::Imsak,
                    time: "06:14".to_string(),
                    is_current: false,
                    is_next: true,
                },
                TrayPrayerItem {
                    name: PrayerName::Yatsi,
                    time: "19:14".to_string(),
                    is_current: true,
                    is_next: false,
                },
            ],
        };

        let items = build_menu_items(&view, &icons);
        for (id, props) in &items {
            if let Some(label_var) = props.get("label") {
                let label_str: &str = label_var.downcast_ref().expect("label must be string");
                assert!(!label_str.contains('📍'), "emoji in item {id}");
                assert!(!label_str.contains('⏳'), "emoji in item {id}");
                assert!(
                    props.contains_key("icon-name") || props.contains_key("icon-data"),
                    "item {id} missing icon"
                );
            }
            if let Some(icon_data) = props.get("icon-data") {
                assert_eq!(icon_data.value_signature().to_string(), "ay");
            }
        }
    }
}
