//! The files the package ships (D-Bus, polkit, systemd) say what the code
//! does, for both identities.

use telamon_update_engine::helper::Op;
use telamon_update_engine::identity::Identity;

fn data(path: &str) -> String {
    std::fs::read_to_string(format!("{}/data/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn ops() -> Vec<Op> {
    vec![
        Op::Status,
        Op::CheckForUpdate,
        Op::Upgrade,
        Op::Rollback,
        Op::SwitchChannel("stable".into()),
    ]
}

/// The `<action id=...>` blocks of the policy, by id.
fn actions(policy: &str) -> Vec<(String, String)> {
    policy
        .split("<action id=\"")
        .skip(1)
        .map(|a| {
            let (id, rest) = a.split_once('"').unwrap();
            let body = rest.split("</action>").next().unwrap();
            (id.to_string(), body.to_string())
        })
        .collect()
}

#[test]
fn the_policy_has_both_action_sets_with_the_same_defaults() {
    let all = actions(&data("polkit-1/actions/net.eterneon.telamon.system.policy"));
    assert_eq!(all.len(), 10);
    let of = |id: Identity| -> Vec<_> {
        all.iter()
            .filter(|(i, _)| i.starts_with(id.action_prefix()))
            .cloned()
            .collect()
    };
    let (new, old) = (of(Identity::Telamon), of(Identity::Legacy));
    assert_eq!((new.len(), old.len()), (5, 5));
    for op in ops() {
        for id in Identity::ALL {
            let want = op.action_id_for(id);
            assert!(
                all.iter().any(|(i, _)| i == want),
                "{want} is not in the policy"
            );
        }
    }
    // identical but for the id
    for ((ni, nb), (oi, ob)) in new.iter().zip(&old) {
        assert_eq!(
            ni.strip_prefix("net.eterneon.telamon.system."),
            oi.strip_prefix("net.eterneon.atlas.system.")
        );
        assert_eq!(nb, ob, "{ni} and {oi} have different defaults");
    }
}

#[test]
fn the_rules_cover_check_and_upgrade_under_both_prefixes() {
    let rules = data("polkit-1/rules.d/50-telamon-system.rules");
    for id in Identity::ALL {
        for suffix in ["check", "upgrade"] {
            assert!(rules.contains(&format!("\"{}.{suffix}\"", id.action_prefix())));
        }
        for suffix in ["status", "rollback", "switch-channel"] {
            assert!(!rules.contains(&format!("{}.{suffix}", id.action_prefix())));
        }
    }
}

#[test]
fn each_identity_has_its_dbus_policy_and_activation_file() {
    for (id, file) in [
        (Identity::Telamon, "net.eterneon.telamon.SystemHelper"),
        (Identity::Legacy, "net.eterneon.atlas.SystemHelper"),
    ] {
        assert_eq!(id.bus_name(), file);
        let conf = data(&format!("dbus-1/system.d/{file}.conf"));
        assert!(conf.contains(&format!("<allow own=\"{}\"/>", id.bus_name())));
        assert!(conf.contains(&format!("send_interface=\"{}\"", id.interface())));
        // and nothing of the other identity
        let other = Identity::ALL.into_iter().find(|o| *o != id).unwrap();
        assert!(!conf.contains(other.bus_name()));
        let service = data(&format!("dbus-1/system-services/{file}.service"));
        assert!(service.contains(&format!("\nName={}\n", id.bus_name())));
        // both start the one unit, which the helper's name is the new one of
        assert!(service.contains("\nSystemdService=telamon-system-helper.service\n"));
    }
    let unit = data("systemd/telamon-system-helper.service");
    assert!(unit.contains(&format!("\nBusName={}\n", Identity::Telamon.bus_name())));
    assert!(unit.contains("\nExecStart=/usr/libexec/telamon-system-helper\n"));
}

#[test]
fn the_units_and_the_preset_use_the_new_names() {
    let preset = data("systemd/50-telamon-system-helper.preset");
    assert_eq!(
        preset,
        "enable telamon-record-boot.service\nenable telamon-drivers.timer\n"
    );
    for unit in ["telamon-drivers.service", "telamon-record-boot.service"] {
        let text = data(&format!("systemd/{unit}"));
        assert!(
            text.contains("ExecStart=/usr/libexec/telamon-system-helper "),
            "{unit}"
        );
        // the state directory keeps its name on purpose
        assert!(text.contains("StateDirectory=atlas-core"), "{unit}");
    }
    assert!(data("systemd/telamon-drivers.timer").contains("[Timer]"));
    assert_eq!(
        data("dnf/protected.d/telamon-updater.conf"),
        "telamon-system-helper\ntelamon-updater\n"
    );
}
