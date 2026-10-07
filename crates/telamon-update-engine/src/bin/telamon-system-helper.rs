//! `telamon-system-helper`: D-Bus activated root helper that runs bootc for Telamon
//! apps. It is also installed as `/usr/libexec/atlas-system-helper` (a link),
//! which the OS image's own scripts call (`atlas-system-helper record-event
//! ...`): nothing here looks at argv[0], so both names do the same.
//!
//! With the argument `record-boot` it instead appends the booted image to
//! the boot history and exits (run at boot by `telamon-record-boot.service`).
//! With `drivers` it switches to the image the hardware needs, if it isn't
//! on it (run by `telamon-drivers.timer`).

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use telamon_framework_system::{bootc, history};
use telamon_update_engine::helper::service::{IDLE_TIMEOUT, Service, serve};
use telamon_update_engine::helper::{
    Core, DriverRun, DriversCfg, OP_LOCK_FILES, SystemBootc, events, layered,
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
                Ok(true) => eprintln!("telamon-system-helper: recorded the booted image"),
                Ok(false) => eprintln!("telamon-system-helper: booted image already recorded"),
                Err(e) => {
                    eprintln!("telamon-system-helper: record-boot failed: {e}");
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
                .with_lock_files(OP_LOCK_FILES.iter().map(Into::into).collect())
                .with_drivers(DriversCfg::default())
                .with_retry_wait(|_| false);
            match core.auto_drivers() {
                DriverRun::Skipped(why) => eprintln!("telamon-system-helper: drivers: {why}"),
                DriverRun::Busy => {
                    eprintln!("telamon-system-helper: drivers: busy, next round");
                }
                DriverRun::Staged { driver, action } => {
                    eprintln!("telamon-system-helper: drivers: {driver} {action} staged");
                }
                DriverRun::Failed(e) => {
                    // exit 0: the state file holds the backoff; a failed unit would nag
                    eprintln!("telamon-system-helper: drivers: failed, will retry later: {e}");
                }
            }
            ExitCode::SUCCESS
        }
        ["record-event", name] => {
            let core = Core::new(Arc::new(SystemBootc)).with_events(events::DEFAULT_PATH.into());
            match core.record_event(name) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("telamon-system-helper: {e}");
                    ExitCode::from(2)
                }
            }
        }
        [] => {
            let builder = match zbus::connection::Builder::system() {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("telamon-system-helper: cannot reach the system bus: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let core = Core::new(Arc::new(SystemBootc))
                .with_events(events::DEFAULT_PATH.into())
                .with_update_file(layered::UPDATE_FILE.into())
                .with_policy(bootc::CONTAINERS_POLICY.into())
                .with_lock_files(OP_LOCK_FILES.iter().map(Into::into).collect())
                .with_drivers(DriversCfg::default());
            let service = Service::from_core(core);
            match serve(builder, service, IDLE_TIMEOUT).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("telamon-system-helper: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!(
                "usage: telamon-system-helper [record-boot | drivers | record-event health-check-failed|health-check-passed]"
            );
            ExitCode::from(2)
        }
    }
}
