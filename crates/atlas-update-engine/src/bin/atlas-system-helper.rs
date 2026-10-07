//! `atlas-system-helper`: D-Bus activated root helper that runs bootc for Atlas
//! apps. With the argument `record-boot` it instead appends the booted image
//! to the boot history and exits (run at boot by `atlas-record-boot.service`).
//! With `drivers` it switches to the image the hardware needs, if it isn't
//! on it (run by `atlas-drivers.timer`).

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use telamon_framework_system::{bootc, history};
use atlas_update_engine::helper::service::{IDLE_TIMEOUT, Service, serve};
use atlas_update_engine::helper::{
    Core, DriverRun, DriversCfg, OP_LOCK_FILE, SystemBootc, events, layered,
};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["record-boot"] => {
            let core = Core::new(Arc::new(SystemBootc)).with_events(events::DEFAULT_PATH.into());
            match core.record_boot(Path::new(history::DEFAULT_PATH)) {
                Ok(true) => eprintln!("atlas-system-helper: recorded the booted image"),
                Ok(false) => eprintln!("atlas-system-helper: booted image already recorded"),
                Err(e) => {
                    eprintln!("atlas-system-helper: record-boot failed: {e}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        ["drivers"] => {
            // One try, never a retry in the same run: the timer comes back.
            let core = Core::new(Arc::new(SystemBootc))
                .with_events(events::DEFAULT_PATH.into())
                .with_update_file(layered::UPDATE_FILE.into())
                .with_policy(bootc::CONTAINERS_POLICY.into())
                .with_lock_file(OP_LOCK_FILE.into())
                .with_drivers(DriversCfg::default())
                .with_retry_wait(|_| false);
            match core.auto_drivers() {
                DriverRun::Skipped(why) => eprintln!("atlas-system-helper: drivers: {why}"),
                DriverRun::Busy => {
                    eprintln!("atlas-system-helper: drivers: busy, next round");
                }
                DriverRun::Staged { driver, action } => {
                    eprintln!("atlas-system-helper: drivers: {driver} {action} staged");
                }
                DriverRun::Failed(e) => {
                    // exit 0: the state file holds the backoff; a failed unit would nag
                    eprintln!("atlas-system-helper: drivers: failed, will retry later: {e}");
                }
            }
            ExitCode::SUCCESS
        }
        ["record-event", name] => {
            let core = Core::new(Arc::new(SystemBootc)).with_events(events::DEFAULT_PATH.into());
            match core.record_event(name) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("atlas-system-helper: {e}");
                    ExitCode::from(2)
                }
            }
        }
        [] => {
            let builder = match zbus::connection::Builder::system() {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("atlas-system-helper: cannot reach the system bus: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let core = Core::new(Arc::new(SystemBootc))
                .with_events(events::DEFAULT_PATH.into())
                .with_update_file(layered::UPDATE_FILE.into())
                .with_policy(bootc::CONTAINERS_POLICY.into())
                .with_lock_file(OP_LOCK_FILE.into())
                .with_drivers(DriversCfg::default());
            let service = Service::from_core(core);
            match serve(builder, service, IDLE_TIMEOUT).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("atlas-system-helper: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!(
                "usage: atlas-system-helper [record-boot | drivers | record-event health-check-failed|health-check-passed]"
            );
            ExitCode::from(2)
        }
    }
}
