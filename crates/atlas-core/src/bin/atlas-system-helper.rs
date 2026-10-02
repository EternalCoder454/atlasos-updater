//! `atlas-system-helper`: D-Bus activated root helper that runs bootc for Atlas
//! apps. With the argument `record-boot` it instead appends the booted image
//! to the boot history and exits (run at boot by `atlas-record-boot.service`).

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use atlas_core::helper::service::{IDLE_TIMEOUT, serve};
use atlas_core::helper::{Core, SystemBootc, events};
use atlas_core::history;

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
            match serve(
                builder,
                Arc::new(SystemBootc),
                IDLE_TIMEOUT,
                Some(events::DEFAULT_PATH.into()),
            )
            .await
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("atlas-system-helper: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!(
                "usage: atlas-system-helper [record-boot | record-event health-check-failed|health-check-passed]"
            );
            ExitCode::from(2)
        }
    }
}
