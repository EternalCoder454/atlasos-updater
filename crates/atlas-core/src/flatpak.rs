//! Flatpak updates through libflatpak (cargo feature `flatpak`).
//!
//! System installations go through flatpak's own system helper, which asks
//! polkit itself; the Atlas system helper is not involved. The calls block, so
//! run them on a worker thread, not on a UI thread.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use libflatpak::gio::Cancellable;
use libflatpak::prelude::*;
use libflatpak::{Installation, InstalledRef, RefKind, Transaction};

/// Which installation an update belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstallationKind {
    System,
    User,
}

/// An app or runtime with an update available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppUpdate {
    /// Flatpak ID, e.g. `org.kde.kate`.
    pub id: String,
    /// Display name from the app's metadata; the ID when there is none.
    pub name: String,
    pub branch: String,
    pub installation: InstallationKind,
    /// Bytes to download; 0 when flatpak cannot tell.
    pub download_size: u64,
    pub current_version: Option<String>,
    /// Not known without downloading metadata; always `None` for now.
    pub new_version: Option<String>,
    /// True for runtimes, false for apps.
    pub is_runtime: bool,
}

/// Progress of the running transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub installation: InstallationKind,
    /// The ref being updated, e.g. `app/org.kde.kate/x86_64/stable`.
    pub reference: String,
    /// 0 to 100 for the current operation.
    pub percent: u32,
    /// Flatpak's status line, e.g. "Downloading".
    pub status: String,
}

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<libflatpak::glib::Error> for Error {
    fn from(e: libflatpak::glib::Error) -> Self {
        Error(e.message().to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn installations() -> Result<Vec<(InstallationKind, Installation)>> {
    let mut v = Vec::new();
    for i in libflatpak::functions::system_installations(None::<&Cancellable>)? {
        v.push((InstallationKind::System, i));
    }
    v.push((
        InstallationKind::User,
        Installation::new_user(None::<&Cancellable>)?,
    ));
    Ok(v)
}

/// Updates available in the system and user installations, apps and
/// runtimes. With `refresh`, appstream data and remote summaries are updated
/// first (network); without it only cached metadata is read. A remote that
/// fails to refresh is skipped.
pub fn list_updates(refresh: bool) -> Result<Vec<AppUpdate>> {
    let none = None::<&Cancellable>;
    let arch = libflatpak::functions::default_arch();
    let mut out = Vec::new();
    for (kind, inst) in installations()? {
        if refresh {
            for remote in inst.list_remotes(none)? {
                let Some(name) = remote.name() else { continue };
                let _ = inst.update_remote_sync(&name, none);
                let _ = inst.update_appstream_sync(&name, arch.as_deref(), none);
            }
        }
        for r in inst.list_installed_refs_for_update(none)? {
            out.push(to_update(&inst, kind, &r));
        }
    }
    Ok(out)
}

fn to_update(inst: &Installation, kind: InstallationKind, r: &InstalledRef) -> AppUpdate {
    let id = r.name().map(|s| s.to_string()).unwrap_or_default();
    let name = r
        .appdata_name()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| id.clone());
    let download_size = r
        .origin()
        .and_then(|o| {
            inst.fetch_remote_size_sync(&o, r, None::<&Cancellable>)
                .ok()
        })
        .map_or(0, |(download, _installed)| download);
    AppUpdate {
        name,
        branch: r.branch().map(|s| s.to_string()).unwrap_or_default(),
        installation: kind,
        download_size,
        current_version: r
            .appdata_version()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
        new_version: None,
        is_runtime: r.kind() == RefKind::Runtime,
        id,
    }
}

/// Update everything that has an update: one transaction per installation.
/// `progress` is called as operations advance (flatpak calls it from inside
/// `update_all`, on the calling thread). Stops at the first failing
/// installation.
pub fn update_all(progress: impl FnMut(Progress) + 'static) -> Result<()> {
    let none = None::<&Cancellable>;
    let progress: Rc<RefCell<dyn FnMut(Progress)>> = Rc::new(RefCell::new(progress));
    for (kind, inst) in installations()? {
        let refs = inst.list_installed_refs_for_update(none)?;
        if refs.is_empty() {
            continue;
        }
        let tx = Transaction::for_installation(&inst, none)?;
        for r in &refs {
            let spec = r
                .format_ref()
                .ok_or_else(|| Error("ref without a name".into()))?;
            tx.add_update(&spec, &[], None)?;
        }
        let cb = progress.clone();
        tx.connect_new_operation(move |_tx, op, prog| {
            let reference = op.get_ref().map(|s| s.to_string()).unwrap_or_default();
            let cb = cb.clone();
            prog.connect_changed(move |p| {
                (cb.borrow_mut())(Progress {
                    installation: kind,
                    reference: reference.clone(),
                    percent: p.progress().clamp(0, 100) as u32,
                    status: p.status().map(|s| s.to_string()).unwrap_or_default(),
                });
            });
        });
        tx.run(none)?;
    }
    Ok(())
}
