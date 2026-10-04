//! Whether background work may use the network and the battery right now:
//! NetworkManager's metered and connectivity state, UPower's battery. Read
//! over the system bus each time (nothing stays connected while idle).

use std::time::Duration;

/// On battery below this, nothing downloads by itself. The OS update stager
/// (AtlasOS `update-stage-condition`) uses the same number.
pub const LOW_BATTERY_PERCENT: f64 = 30.0;

const BUS_TIMEOUT: Duration = Duration::from_secs(3);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(8);

/// What the system reported. `None`: the service isn't there or didn't answer.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct State {
    /// NetworkManager `Metered`: 1 yes, 2 no, 3 guessed yes, 4 guessed no.
    pub metered: Option<u32>,
    /// NetworkManager `Connectivity`: 1 none, 2 portal, 3 limited, 4 full.
    pub connectivity: Option<u32>,
    pub on_battery: Option<bool>,
    /// The combined battery level, when there is a battery.
    pub battery_percent: Option<f64>,
}

/// Why nothing may use the network now (looking for updates included).
pub fn network_why_not(s: &State) -> Option<&'static str> {
    // 0: NetworkManager doesn't know, which it says when there is no
    // connection to go by. Not knowing counts as metered.
    if matches!(s.metered, Some(0 | 1 | 3)) {
        return Some("the connection is metered");
    }
    // 0 (unknown) and a missing NetworkManager don't stop anything: the
    // download itself then says whether there is a network.
    if matches!(s.connectivity, Some(1..=3)) {
        return Some("the computer is offline");
    }
    None
}

/// Why background work waits, in words for the log and the UI.
pub fn why_not(s: &State) -> Option<&'static str> {
    if let Some(why) = network_why_not(s) {
        return Some(why);
    }
    if s.on_battery == Some(true) && s.battery_percent.is_some_and(|p| p < LOW_BATTERY_PERCENT) {
        return Some("the battery is low");
    }
    None
}

/// What is assumed when the network state couldn't be read: metered (3,
/// "guessed yes"), so nothing spends someone's data on a guess.
fn unsure() -> State {
    State {
        metered: Some(3),
        ..Default::default()
    }
}

/// Blocking: reads the state (at most a few seconds).
pub fn read() -> State {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return unsure();
    };
    // One limit for the whole read, however many services hang.
    rt.block_on(async {
        tokio::time::timeout(TOTAL_TIMEOUT, read_all())
            .await
            .unwrap_or_else(|_| unsure())
    })
}

/// Whether NetworkManager runs: `None` when the bus couldn't say, or when
/// it is installed but not running right now (restarting, say).
async fn nm_running(conn: &zbus::Connection) -> Option<bool> {
    const NM: &str = "org.freedesktop.NetworkManager";
    let ask = async {
        let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
        let name = zbus::names::BusName::try_from(NM).ok()?;
        if dbus.name_has_owner(name).await.ok()? {
            return Some(true);
        }
        let installed = dbus.list_activatable_names().await.ok()?;
        if installed.iter().any(|n| n.as_str() == NM) {
            None
        } else {
            Some(false)
        }
    };
    tokio::time::timeout(BUS_TIMEOUT, ask).await.ok().flatten()
}

async fn read_all() -> State {
    {
        let Ok(Ok(conn)) = tokio::time::timeout(BUS_TIMEOUT, zbus::Connection::system()).await
        else {
            return unsure();
        };
        let nm = (
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        );
        let up = (
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower",
            "org.freedesktop.UPower",
        );
        let dev = (
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.UPower.Device",
        );
        let present: Option<bool> = prop(&conn, dev, "IsPresent").await;
        let (metered, connectivity) = match nm_running(&conn).await {
            // No NetworkManager (a desktop set up some other way): nothing
            // to go by, and the download itself says whether there is a
            // network.
            Some(false) => (None, None),
            // There, or unknown, but it didn't say: assume metered.
            _ => (
                Some(prop(&conn, nm, "Metered").await.unwrap_or(3)),
                prop(&conn, nm, "Connectivity").await,
            ),
        };
        State {
            metered,
            connectivity,
            on_battery: prop(&conn, up, "OnBattery").await,
            battery_percent: if present == Some(true) {
                prop(&conn, dev, "Percentage").await
            } else {
                None
            },
        }
    }
}

async fn prop<T>(conn: &zbus::Connection, at: (&str, &str, &str), name: &str) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    let get = async {
        let p = zbus::Proxy::new(conn, at.0, at.1, at.2).await.ok()?;
        p.get_property::<T>(name).await.ok()
    };
    tokio::time::timeout(BUS_TIMEOUT, get).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugged_in_and_online_is_fine() {
        let s = State {
            metered: Some(4),
            connectivity: Some(4),
            on_battery: Some(false),
            battery_percent: Some(5.0),
        };
        assert_eq!(why_not(&s), None);
        // A desktop without UPower or NetworkManager answers.
        assert_eq!(why_not(&State::default()), None);
        // A state that couldn't be read waits.
        assert_eq!(why_not(&unsure()), Some("the connection is metered"));
    }

    #[test]
    fn low_battery_doesnt_stop_looking() {
        let s = State {
            on_battery: Some(true),
            battery_percent: Some(5.0),
            ..Default::default()
        };
        assert_eq!(network_why_not(&s), None);
        assert_eq!(why_not(&s), Some("the battery is low"));
    }

    #[test]
    fn metered_waits() {
        for m in [1, 3] {
            let s = State {
                metered: Some(m),
                ..Default::default()
            };
            assert_eq!(why_not(&s), Some("the connection is metered"));
        }
        let s = State {
            metered: Some(2),
            ..Default::default()
        };
        assert_eq!(why_not(&s), None);
    }

    #[test]
    fn offline_waits_unknown_does_not() {
        let s = |c| State {
            connectivity: Some(c),
            ..Default::default()
        };
        assert_eq!(why_not(&s(1)), Some("the computer is offline"));
        assert_eq!(why_not(&s(3)), Some("the computer is offline"));
        assert_eq!(why_not(&s(0)), None);
        // NetworkManager not knowing whether it is metered waits.
        let unknown = State {
            metered: Some(0),
            ..Default::default()
        };
        assert_eq!(why_not(&unknown), Some("the connection is metered"));
    }

    #[test]
    fn low_battery_only_when_unplugged() {
        let s = |on, p| State {
            on_battery: Some(on),
            battery_percent: Some(p),
            ..Default::default()
        };
        assert_eq!(why_not(&s(true, 29.0)), Some("the battery is low"));
        assert_eq!(why_not(&s(true, 30.0)), None);
        assert_eq!(why_not(&s(false, 10.0)), None);
        // On battery with no level known: go ahead.
        let unknown = State {
            on_battery: Some(true),
            ..Default::default()
        };
        assert_eq!(why_not(&unknown), None);
    }
}
