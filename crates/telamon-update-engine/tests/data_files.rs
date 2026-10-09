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

fn strip_xml_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        rest = match rest[i..].find("-->") {
            Some(j) => &rest[i + j + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The `<action id=...>` blocks of the policy, by id.
fn actions(policy: &str) -> Vec<(String, String)> {
    let policy = strip_xml_comments(policy);
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

// ---- Secure phase: what the shipped files may and may not grant (docs/SECURITY.md)

/// `(allow_any, allow_inactive, allow_active)` of an action's `<defaults>`.
fn defaults(body: &str) -> (String, String, String) {
    let get = |tag: &str| {
        body.split(&format!("<{tag}>"))
            .nth(1)
            .and_then(|r| r.split(&format!("</{tag}>")).next())
            .unwrap_or_else(|| panic!("no <{tag}>"))
            .trim()
            .to_string()
    };
    (get("allow_any"), get("allow_inactive"), get("allow_active"))
}

#[test]
fn only_status_and_check_are_open_to_users_everything_else_asks_an_administrator() {
    let all = actions(&data("polkit-1/actions/net.eterneon.telamon.system.policy"));
    for (id, body) in &all {
        let (any, inactive, active) = defaults(body);
        let name = id.rsplit('.').next().unwrap();
        match name {
            // reads what `bootc status` says; harmless for anyone
            "status" => assert_eq!(
                (any.as_str(), inactive.as_str(), active.as_str()),
                ("yes", "yes", "yes"),
                "{id}"
            ),
            // may use the network: only a user at the machine, never remote
            "check" => assert_eq!(
                (any.as_str(), inactive.as_str(), active.as_str()),
                ("no", "no", "yes"),
                "{id}"
            ),
            // changes what boots next: an administrator, from any session
            "upgrade" | "rollback" | "switch-channel" => assert_eq!(
                (any.as_str(), inactive.as_str(), active.as_str()),
                ("auth_admin", "auth_admin", "auth_admin_keep"),
                "{id}"
            ),
            other => panic!("{id}: an action ({other}) the tests do not know"),
        }
        // a prompt must say what it is for
        assert!(
            body.contains("<message>") && body.contains("<description>"),
            "{id}"
        );
        // never lets a program be run with chosen arguments, and implies
        // nothing else: no annotation at all
        assert!(!body.contains("<annotate"), "{id}");
    }
}

#[test]
fn the_rule_grants_only_check_and_upgrade_to_local_active_wheel() {
    // Every word of the rule, comments aside: a widened condition or id list
    // is a change to review, not one a substring test lets through.
    let rules = data("polkit-1/rules.d/50-telamon-system.rules");
    let code: String = rules
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join(" ");
    let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(
        code,
        "polkit.addRule(function(action, subject) { var ids = [ \
         \"net.eterneon.telamon.system.check\", \"net.eterneon.telamon.system.upgrade\", \
         \"net.eterneon.atlas.system.check\", \"net.eterneon.atlas.system.upgrade\" ]; \
         if (ids.indexOf(action.id) >= 0 && subject.local && subject.active && \
         subject.isInGroup(\"wheel\")) { return polkit.Result.YES; } });"
            .replace("\\\n", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    );
    // and it is the only rules file the package ships
    let dir = format!("{}/data/polkit-1/rules.d", env!("CARGO_MANIFEST_DIR"));
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1);
}

#[test]
fn the_dbus_policy_lets_only_root_own_the_names_and_allows_only_named_interfaces() {
    for file in [
        "dbus-1/system.d/net.eterneon.telamon.SystemHelper.conf",
        "dbus-1/system.d/net.eterneon.atlas.SystemHelper.conf",
    ] {
        let conf = strip_xml_comments(&data(file));
        let conf = conf.split_whitespace().collect::<Vec<_>>().join(" ");
        let bus = file
            .rsplit('/')
            .next()
            .unwrap()
            .trim_end_matches(".conf")
            .to_string();
        // every allowance concerns the helper's own name, and nothing else
        for part in conf.split("send_destination=\"").skip(1) {
            assert_eq!(part.split('"').next().unwrap(), bus, "{file}");
        }
        for part in conf.split("own=\"").skip(1) {
            assert_eq!(part.split('"').next().unwrap(), bus, "{file}");
        }
        // owning: root's policy only
        let owns: Vec<_> = conf.match_indices("<allow own=").map(|(i, _)| i).collect();
        assert_eq!(owns.len(), 1, "{file}");
        let root = conf.find("<policy user=\"root\">").unwrap();
        let default = conf.find("<policy context=\"default\">").unwrap();
        assert!(
            root < owns[0] && owns[0] < default,
            "{file}: own is not in root's policy"
        );
        // sending: every allow names an interface, and there is no allow_*
        // of a user, group or the whole bus
        for line in conf.split("<allow").skip(1) {
            assert!(
                line.contains("own=") || line.contains("send_destination"),
                "{file}: {line}"
            );
        }
        let after_default = &conf[default..];
        assert_eq!(
            after_default.matches("<allow send_destination").count(),
            after_default.matches("send_interface").count(),
            "{file}: a send without an interface"
        );
        for bad in [
            "<allow user=",
            "<allow group=",
            "<deny",
            "eavesdrop",
            "receive_",
        ] {
            assert!(!conf.contains(bad), "{file}: {bad}");
        }
    }
}

/// `(name, parameters)` of every `fn` in `body`, parameters split at the
/// top-level commas, whatever the layout of the signature.
fn functions(body: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = body[at..].find("fn ") {
        let start = at + i;
        at = start + 3;
        // a whole word: not the tail of another identifier
        if body[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let rest = &body[start + 3..];
        let Some(open) = rest.find('(') else { continue };
        let name = rest[..open].trim().to_string();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let (mut depth, mut cur, mut params) = (0, String::new(), Vec::new());
        for c in rest[open..].chars() {
            match c {
                '(' | '[' | '<' => {
                    depth += 1;
                    if depth > 1 {
                        cur.push(c);
                    }
                }
                ')' | ']' | '>' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    cur.push(c);
                }
                ',' if depth == 1 => params.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
        params.push(cur);
        let params = params
            .iter()
            .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|p| !p.is_empty())
            .collect();
        out.push((name, params));
    }
    out
}

#[test]
fn the_helper_exposes_six_methods_and_only_the_channel_takes_an_argument() {
    // every interface attribute anywhere in the engine
    let mut sources = Vec::new();
    let mut dirs = vec![std::path::PathBuf::from(format!(
        "{}/src",
        env!("CARGO_MANIFEST_DIR")
    ))];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                sources.push(std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    let all = sources.join("\n");
    let interfaces = all.matches("zbus::interface").count() + all.matches("#[interface").count();
    assert_eq!(interfaces, 2, "a D-Bus interface was added or removed");

    let service = std::fs::read_to_string(format!(
        "{}/src/helper/service.rs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    // the production part of the file, before its tests
    let service = service.split("#[cfg(test)]").next().unwrap();
    let blocks: Vec<&str> = service.split("#[zbus::interface(").skip(1).collect();
    assert_eq!(blocks.len(), 2);
    for iface in blocks {
        let body = iface.split("\n}\n").next().unwrap();
        let mut methods = Vec::new();
        for (name, params) in functions(body) {
            // `&self`, the message header and the connection come from zbus,
            // not from the caller
            let args: Vec<_> = params
                .iter()
                .filter(|p| {
                    *p != "&self"
                        && !p.starts_with("#[zbus(header)]")
                        && !p.starts_with("#[zbus(connection)]")
                })
                .cloned()
                .collect();
            if name == "progress" {
                assert!(args.is_empty(), "the property takes nothing");
                continue;
            }
            let want: &[&str] = if name == "switch_channel" {
                &["channel: String"]
            } else {
                &[]
            };
            assert_eq!(args, want, "{name} takes an argument from the caller");
            methods.push(name);
        }
        methods.sort();
        assert_eq!(
            methods,
            [
                "cancel_rollback",
                "check_for_update",
                "rollback",
                "status",
                "switch_channel",
                "upgrade"
            ]
        );
    }
}

#[test]
fn the_helper_units_are_sandboxed_as_far_as_bootc_allows() {
    // what every unit that runs the helper must keep (docs/SECURITY.md)
    let all = [
        "ProtectHome=yes",
        "ProtectKernelModules=yes",
        "ProtectKernelLogs=yes",
        "ProtectControlGroups=yes",
        "ProtectClock=yes",
        "ProtectHostname=yes",
        "LockPersonality=yes",
        "RestrictRealtime=yes",
        "SystemCallArchitectures=native",
        "PrivateTmp=yes",
    ];
    for unit in [
        "telamon-system-helper.service",
        "telamon-drivers.service",
        "telamon-record-boot.service",
    ] {
        let text = data(&format!("systemd/{unit}"));
        for want in all {
            assert!(text.lines().any(|l| l == want), "{unit} lost {want}");
        }
        // the capabilities that must never come back
        let caps = text
            .lines()
            .find(|l| l.starts_with("CapabilityBoundingSet="))
            .unwrap_or_else(|| panic!("{unit}: no CapabilityBoundingSet"));
        for cap in [
            "CAP_SYS_MODULE",
            "CAP_SYS_BOOT",
            "CAP_SYS_RAWIO",
            "CAP_SYS_TIME",
            "CAP_NET_ADMIN",
            "CAP_NET_RAW",
            "CAP_BPF",
            "CAP_PERFMON",
        ] {
            assert!(caps.contains(cap), "{unit}: {cap} is no longer removed");
        }
        assert!(
            caps.starts_with("CapabilityBoundingSet=~"),
            "{unit}: must be a deny list"
        );
        // none of them lets a user run it with arguments
        assert!(!text.contains("User="), "{unit}");
    }
    // no network at all for the one that only reads bootc status
    let boot = data("systemd/telamon-record-boot.service");
    assert!(boot.contains("\nPrivateNetwork=yes\n"));
    assert!(boot.contains("\nRestrictAddressFamilies=AF_UNIX\n"));
    // the helper and the drivers run reach registries, and only over IP
    for unit in ["telamon-system-helper.service", "telamon-drivers.service"] {
        assert!(
            data(&format!("systemd/{unit}"))
                .contains("\nRestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK\n")
        );
    }
}
