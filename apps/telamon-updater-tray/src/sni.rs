//! The panel icon: a StatusNotifierItem (what KStatusNotifierItem exported
//! before, same Id, so Plasma keeps the user's tray settings for it), its
//! menu over com.canonical.dbusmenu, and the tray's own small interface
//! (`Reload`, `SetWorking`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc::UnboundedSender;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, StructureBuilder, Type, Value};

use crate::Msg;

pub const ITEM_PATH: &str = "/StatusNotifierItem";
pub const MENU_PATH: &str = "/MenuBar";
pub const ID: &str = "net.eterneon.telamon.updater";
pub const TITLE: &str = "Telamon Updater";

type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

/// Everything the panel shows that changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    pub status: &'static str,
    pub icon: String,
    pub attention: String,
    /// The tooltip's text under the title.
    pub tip: String,
    /// Menu: "Restart to Update" and "Cancel Scheduled Restart" shown.
    pub restart: bool,
    pub cancel: bool,
}

pub struct Item {
    pub look: Look,
    pub tx: UnboundedSender<Msg>,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "SystemServices"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        ID
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        TITLE
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        self.look.status
    }

    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        &self.look.icon
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmaps {
        Vec::new()
    }

    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Pixmaps {
        Vec::new()
    }

    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        &self.look.attention
    }

    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Pixmaps {
        Vec::new()
    }

    #[zbus(property)]
    fn attention_movie_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, Pixmaps, String, String) {
        (
            crate::APP_ICON.to_string(),
            Vec::new(),
            TITLE.to_string(),
            self.look.tip.clone(),
        )
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        ObjectPath::from_static_str_unchecked(MENU_PATH).into()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }

    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send(Msg::Activate);
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {}

    /// Plasma shows the menu from `Menu`; nothing to do here.
    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: &str) {}

    /// Sent by Plasma just before `Activate`: lets Settings take focus.
    fn provide_xdg_activation_token(&self, token: String) {
        let _ = self.tx.send(Msg::Token(token));
    }

    #[zbus(signal)]
    pub async fn new_icon(e: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_attention_icon(e: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_status(e: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_tool_tip(e: &SignalEmitter<'_>) -> zbus::Result<()>;
}

// ---- the menu ----

pub const OPEN: i32 = 1;
pub const CHECK: i32 = 2;
pub const RESTART: i32 = 3;
pub const CANCEL: i32 = 4;
const SEPARATOR: i32 = 5;
pub const QUIT: i32 = 6;
const ITEMS: [i32; 6] = [OPEN, CHECK, RESTART, CANCEL, SEPARATOR, QUIT];

type Props = HashMap<String, Value<'static>>;

pub struct Menu {
    pub restart: bool,
    pub cancel: bool,
    pub revision: u32,
    pub tx: UnboundedSender<Msg>,
}

/// `(ia{sv}av)`: an item, its properties and its children.
#[derive(Debug, serde::Serialize, Type)]
pub struct Layout {
    id: i32,
    props: Props,
    children: Vec<Value<'static>>,
}

fn filtered(mut props: Props, names: &[String]) -> Props {
    if !names.is_empty() {
        props.retain(|k, _| names.iter().any(|n| n == k));
    }
    props
}

impl Menu {
    /// An item's properties, all of them; `None` for an id that isn't one.
    fn props(&self, id: i32) -> Option<Props> {
        let item = |label: &str, icon: &str, visible: bool| {
            Props::from([
                ("label".to_string(), Value::from(label.to_string())),
                ("icon-name".to_string(), Value::from(icon.to_string())),
                ("visible".to_string(), Value::from(visible)),
                ("enabled".to_string(), Value::from(true)),
            ])
        };
        Some(match id {
            0 => Props::from([(
                "children-display".to_string(),
                Value::from("submenu".to_string()),
            )]),
            OPEN => item("Open Updates", crate::APP_ICON, true),
            CHECK => item("Check for Updates", "view-refresh", true),
            RESTART => item("Restart to Update", "system-reboot", self.restart),
            CANCEL => item("Cancel Scheduled Restart", "dialog-cancel", self.cancel),
            SEPARATOR => Props::from([("type".to_string(), Value::from("separator".to_string()))]),
            QUIT => item("Quit", "application-exit", true),
            _ => return None,
        })
    }

    /// A leaf item as a variant, for a parent's children.
    fn child(&self, id: i32, names: &[String]) -> Value<'static> {
        let props = filtered(self.props(id).unwrap_or_default(), names);
        let structure = StructureBuilder::new()
            .add_field(id)
            .add_field(props)
            .add_field(Vec::<Value<'static>>::new())
            .build()
            .expect("a menu item has fields");
        Value::Structure(structure)
    }

    /// The items whose visibility changes, for `ItemsPropertiesUpdated`.
    pub fn changed(&self) -> Vec<(i32, Props)> {
        [RESTART, CANCEL]
            .iter()
            .filter_map(|id| {
                let p = self.props(*id)?;
                Some((*id, filtered(p, &["visible".to_string()])))
            })
            .collect()
    }
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let props = self
            .props(parent_id)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}")))?;
        let children = if parent_id == 0 && recursion_depth != 0 {
            ITEMS
                .iter()
                .map(|id| self.child(*id, &property_names))
                .collect()
        } else {
            Vec::new()
        };
        Ok((
            self.revision,
            Layout {
                id: parent_id,
                props: filtered(props, &property_names),
                children,
            },
        ))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, Props)> {
        let ids = if ids.is_empty() { ITEMS.to_vec() } else { ids };
        ids.into_iter()
            .filter_map(|id| Some((id, filtered(self.props(id)?, &property_names))))
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        let v = self
            .props(id)
            .and_then(|mut p| p.remove(&name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on {id}")))?;
        v.try_to_owned()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        if event_id == "clicked" && ITEMS.contains(&id) {
            let _ = self.tx.send(Msg::Menu(id));
        }
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        let mut errors = Vec::new();
        for (id, event_id, _, _) in events {
            if !ITEMS.contains(&id) {
                errors.push(id);
            } else if event_id == "clicked" {
                let _ = self.tx.send(Msg::Menu(id));
            }
        }
        errors
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let errors = ids
            .into_iter()
            .filter(|id| *id != 0 && !ITEMS.contains(id))
            .collect();
        (Vec::new(), errors)
    }

    #[zbus(signal)]
    pub async fn items_properties_updated(
        e: &SignalEmitter<'_>,
        updated: Vec<(i32, Props)>,
        removed: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn layout_updated(
        e: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

// ---- the way in for Settings ----

#[derive(Clone)]
pub struct Control {
    pub tx: UnboundedSender<Msg>,
    /// Shared with the tray: a Reload waits in the queue already.
    pub pending: Arc<AtomicBool>,
}

#[interface(name = "net.eterneon.telamon.updater.Tray")]
impl Control {
    /// Read the settings again: Settings changed one (telamon_updater_base::tray).
    fn reload(&self) {
        if !self.pending.swap(true, Ordering::SeqCst) {
            let _ = self.tx.send(Msg::Reload);
        }
    }

    /// The caller says whether it is changing the system (an update, a
    /// switch or a go back being staged, apps updating, firmware installing):
    /// the screen-edge glow shows while anybody does. The claim is the
    /// caller's connection's: `false` releases it, so does the connection
    /// closing, and it runs out 3 hours after the last `true`. One boolean:
    /// nothing else is accepted (the bus refuses any other signature).
    fn set_working(
        &self,
        on: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no sender".into()))?;
        self.working(sender.as_str(), on)
    }
}

impl Control {
    fn working(&self, sender: &str, on: bool) -> zbus::fdo::Result<()> {
        // A unique name (":1.23"), whatever the caller says: it is the
        // key of the claim.
        if !sender.starts_with(':') {
            return Err(zbus::fdo::Error::InvalidArgs("not a unique name".into()));
        }
        self.tx
            .send(Msg::SetWorking(sender.to_string(), on))
            .map_err(|_| zbus::fdo::Error::Failed("the tray is ending".into()))
    }
}

/// The same, under the name it had until 0.3.0
/// (`net.eterneon.atlas.updater.Tray` at `/net/eterneon/atlas/updater/Tray`),
/// for programs that have not moved yet. Remove in the release after the next.
#[derive(Clone)]
pub struct LegacyControl(pub Control);

#[interface(name = "net.eterneon.atlas.updater.Tray")]
impl LegacyControl {
    fn reload(&self) {
        self.0.reload();
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use telamon_updater_base::tray;
    use zbus::object_server::Interface;

    #[test]
    fn the_tray_answers_to_both_interfaces() {
        assert_eq!(Control::name().as_str(), tray::INTERFACE);
        assert_eq!(LegacyControl::name().as_str(), tray::LEGACY_INTERFACE);
        assert_ne!(tray::INTERFACE, tray::LEGACY_INTERFACE);
    }

    #[test]
    fn a_reload_through_either_name_reaches_the_tray_once() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let pending = Arc::new(AtomicBool::new(false));
        let control = Control {
            tx,
            pending: pending.clone(),
        };
        let legacy = LegacyControl(control.clone());
        legacy.reload();
        assert!(matches!(rx.try_recv(), Ok(Msg::Reload)));
        assert!(pending.load(Ordering::SeqCst));
        // one is waiting in the queue already: the second is not queued again
        control.reload();
        legacy.reload();
        assert!(rx.try_recv().is_err());
        pending.store(false, Ordering::SeqCst);
        control.reload();
        assert!(matches!(rx.try_recv(), Ok(Msg::Reload)));
    }

    #[test]
    fn a_claim_reaches_the_tray_with_the_callers_unique_name() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let control = Control {
            tx,
            pending: Arc::new(AtomicBool::new(false)),
        };
        control.working(":1.42", true).unwrap();
        control.working(":1.42", false).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Msg::SetWorking(s, true)) if s == ":1.42"));
        assert!(matches!(rx.try_recv(), Ok(Msg::SetWorking(s, false)) if s == ":1.42"));
        // never a well-known name or anything else as the key
        assert!(control.working("org.example.Name", true).is_err());
        assert!(control.working("", true).is_err());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_panel_item_has_the_new_id() {
        assert_eq!(ID, "net.eterneon.telamon.updater");
        assert_eq!(TITLE, "Telamon Updater");
    }
}
