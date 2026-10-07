//! A private D-Bus daemon for the tests that need a real bus (the client's
//! choice of identity, both identities answering). Without a `dbus-daemon`
//! binary these tests say so and return, unless `TELAMON_REQUIRE_DBUS_TESTS`
//! is set (the checks in the build container set it): then they fail.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

pub struct TestBus {
    child: Child,
    pub address: String,
    _dir: tempfile::TempDir,
}

impl TestBus {
    /// A bus whose D-Bus activation files name `activatable` (nothing is
    /// ever started: they only show in `ListActivatableNames`).
    pub fn start(activatable: &[&str]) -> Option<TestBus> {
        let dir = tempfile::tempdir().unwrap();
        let services = dir.path().join("services");
        std::fs::create_dir(&services).unwrap();
        for name in activatable {
            std::fs::write(
                services.join(format!("{name}.service")),
                format!("[D-BUS Service]\nName={name}\nExec=/bin/false\n"),
            )
            .unwrap();
        }
        let config = dir.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={sock}</listen>
  <servicedir>{services}</servicedir>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                sock = dir.path().join("bus").display(),
                services = services.display(),
            ),
        )
        .unwrap();
        let spawned = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                assert!(
                    std::env::var_os("TELAMON_REQUIRE_DBUS_TESTS").is_none(),
                    "TELAMON_REQUIRE_DBUS_TESTS is set but dbus-daemon cannot start: {e}"
                );
                eprintln!("skipped: no dbus-daemon ({e})");
                return None;
            }
        };
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_string();
        assert!(address.starts_with("unix:"), "dbus-daemon said {address:?}");
        Some(TestBus {
            child,
            address,
            _dir: dir,
        })
    }

    pub async fn connect(&self) -> zbus::Connection {
        self.builder().build().await.unwrap()
    }

    /// A builder for a helper to serve on this bus.
    pub fn builder(&self) -> zbus::connection::Builder<'static> {
        let address: zbus::Address = self.address.parse().unwrap();
        zbus::connection::Builder::address(address).unwrap()
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Waits (at most 10 s) until `name` has an owner on `conn`'s bus.
pub async fn wait_for_owner(conn: &zbus::Connection, name: &str) {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.unwrap();
    for _ in 0..200 {
        if dbus
            .name_has_owner(zbus::names::BusName::try_from(name).unwrap())
            .await
            .unwrap()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("nobody took {name}");
}
