//! Picks the tray's icon names from the user's icon theme, as
//! `QIcon::hasThemeIcon` did: the theme's own system-update panel icons when
//! it has them (Papirus on AtlasOS), else ours. Plasma draws the icon by name;
//! this only decides which name exists.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use telamon_framework_core::settings::{Settings, config_dir};

pub struct Icons {
    pub base: String,
    pub ready: String,
    pub urgent: String,
}

impl Icons {
    pub fn load() -> Icons {
        let theme = Theme::current();
        let base = theme.pick(
            &["system-software-update-panel", "update-none"],
            "net.eterneon.atlas.updater-symbolic",
        );
        let ready = theme.pick(
            &[
                "software-update-available",
                "update-low",
                "software-update-available-symbolic",
            ],
            "net.eterneon.atlas.updater-ready-symbolic",
        );
        let urgent = theme.pick(
            &[
                "software-update-urgent",
                "update-high",
                "software-update-urgent-symbolic",
            ],
            &ready,
        );
        Icons {
            base,
            ready,
            urgent,
        }
    }
}

struct Theme {
    /// Every directory an icon of the theme (or a theme it inherits) can be
    /// in, in lookup order.
    dirs: Vec<PathBuf>,
}

const EXTENSIONS: [&str; 3] = ["svg", "png", "svgz"];

impl Theme {
    fn current() -> Theme {
        Theme::named(&theme_name(), &base_dirs())
    }

    fn named(name: &str, bases: &[PathBuf]) -> Theme {
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = vec![name.to_string()];
        // hicolor is every theme's last fallback, after everything inherited.
        let mut hicolor_queued = name == "hicolor";
        loop {
            let next = queue.pop().or_else(|| {
                (!hicolor_queued).then(|| {
                    hicolor_queued = true;
                    "hicolor".to_string()
                })
            });
            let Some(t) = next else { break };
            if !seen.insert(t.clone()) || seen.len() > 16 {
                continue;
            }
            let mut inherits = Vec::new();
            for base in bases {
                let root = base.join(&t);
                let Some(index) = read_index(&root.join("index.theme")) else {
                    continue;
                };
                for sub in index.dirs {
                    dirs.push(root.join(sub));
                }
                if inherits.is_empty() {
                    inherits = index.inherits;
                }
            }
            // The first inherited theme is looked up first.
            for i in inherits.into_iter().rev() {
                queue.push(i);
            }
        }
        Theme { dirs }
    }

    fn has(&self, name: &str) -> bool {
        self.dirs.iter().any(|d| {
            EXTENSIONS
                .iter()
                .any(|ext| d.join(format!("{name}.{ext}")).is_file())
        })
    }

    fn pick(&self, names: &[&str], fallback: &str) -> String {
        names
            .iter()
            .find(|n| self.has(n))
            .map_or_else(|| fallback.to_string(), |n| n.to_string())
    }
}

struct Index {
    dirs: Vec<String>,
    inherits: Vec<String>,
}

fn list(v: Option<String>) -> Vec<String> {
    v.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.contains('/') && *s != "." && *s != "..")
        .map(String::from)
        .collect()
}

fn read_index(path: &Path) -> Option<Index> {
    if !path.is_file() {
        return None;
    }
    let s = Settings::at(path);
    // Subdirectories nest ("22x22/panel"), but never lead out of the theme.
    let dirs = ["Directories", "ScaledDirectories"]
        .iter()
        .filter_map(|k| s.get("Icon Theme", k))
        .flat_map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|p| {
                    !p.is_empty() && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
                })
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect();
    Some(Index {
        dirs,
        inherits: list(s.get("Icon Theme", "Inherits")),
    })
}

/// The icon theme Plasma uses: kdeglobals `[Icons] Theme`, the user's file
/// first, then the system's; Breeze when none says.
fn theme_name() -> String {
    let mut files = vec![config_dir().join("kdeglobals")];
    let sys = std::env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/etc/xdg".into());
    files.extend(
        sys.split(':')
            .filter(|p| p.starts_with('/'))
            .map(|p| Path::new(p).join("kdeglobals")),
    );
    files
        .iter()
        // Only regular files: a FIFO there would block the tray.
        .filter(|f| f.is_file())
        .find_map(|f| Settings::at(f).get("Icons", "Theme"))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && !t.contains('/') && t != "." && t != "..")
        .unwrap_or_else(|| "breeze".into())
}

/// Where icon themes live (the icon theme spec's order).
fn base_dirs() -> Vec<PathBuf> {
    let abs = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let mut out = Vec::new();
    if let Some(h) = abs("HOME") {
        out.push(h.join(".icons"));
    }
    match abs("XDG_DATA_HOME") {
        Some(d) => out.push(d.join("icons")),
        None => {
            if let Some(h) = abs("HOME") {
                out.push(h.join(".local/share/icons"));
            }
        }
    }
    let data = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    out.extend(
        data.split(':')
            .filter(|p| p.starts_with('/'))
            .map(|p| Path::new(p).join("icons")),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(root: &Path, name: &str, dirs: &str, inherits: &str) {
        let t = root.join(name);
        std::fs::create_dir_all(&t).unwrap();
        std::fs::write(
            t.join("index.theme"),
            format!("[Icon Theme]\nName={name}\nDirectories={dirs}\nInherits={inherits}\n"),
        )
        .unwrap();
    }

    fn icon(root: &Path, theme: &str, dir: &str, file: &str) {
        let d = root.join(theme).join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(file), b"<svg/>").unwrap();
    }

    #[test]
    fn follows_inherits_then_hicolor() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        theme(r, "Papirus-Dark", "22x22/panel", "Papirus,breeze");
        theme(r, "Papirus", "22x22/panel,symbolic/status", "breeze");
        theme(r, "hicolor", "scalable/status", "");
        icon(r, "Papirus", "22x22/panel", "update-none.svg");
        icon(
            r,
            "hicolor",
            "scalable/status",
            "net.eterneon.atlas.updater-ready-symbolic.svg",
        );
        let t = Theme::named("Papirus-Dark", &[r.to_path_buf()]);
        assert!(t.has("update-none"));
        assert!(t.has("net.eterneon.atlas.updater-ready-symbolic"));
        assert!(!t.has("software-update-urgent"));
        assert_eq!(
            t.pick(&["system-software-update-panel", "update-none"], "x"),
            "update-none"
        );
        assert_eq!(t.pick(&["nope"], "fallback"), "fallback");
    }

    #[test]
    fn a_missing_theme_or_a_loop_is_harmless() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        theme(r, "A", "x", "B");
        theme(r, "B", "x", "A");
        let t = Theme::named("A", &[r.to_path_buf()]);
        assert!(!t.has("anything"));
        let t = Theme::named("Gone", &[r.to_path_buf()]);
        assert!(t.dirs.is_empty());
    }

    #[test]
    fn index_paths_cannot_leave_the_theme() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        theme(r, "T", "ok/22,../../etc,/abs,a//b", "../x");
        let i = read_index(&r.join("T/index.theme")).unwrap();
        assert_eq!(i.dirs, ["ok/22"]);
        assert!(i.inherits.is_empty());
    }
}
