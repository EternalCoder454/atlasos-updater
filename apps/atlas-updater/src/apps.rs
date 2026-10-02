//! Flatpak app updates (through atlas-core's flatpak module).

use std::path::Path;

use atlas_core::flatpak::{self, AppUpdate, InstallationKind};
use serde::{Deserialize, Serialize};

use crate::config;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub name: String,
    pub id: String,
    pub branch: String,
    pub system: bool,
    pub runtime: bool,
    pub size: u64,
    #[serde(default)]
    pub size_text: String,
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    if bytes == 0 {
        return String::new();
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn row(u: AppUpdate) -> Row {
    Row {
        size_text: format_size(u.download_size),
        name: u.name,
        id: u.id,
        branch: u.branch,
        system: u.installation == InstallationKind::System,
        runtime: u.is_runtime,
        size: u.download_size,
    }
}

/// Blocking. `refresh` updates the appstream and summary caches first.
pub fn list(refresh: bool, fixtures: Option<&Path>) -> Result<Vec<Row>, String> {
    if let Some(dir) = fixtures {
        let text = config::read_fixture(dir, "flatpak.json").unwrap_or_else(|| "[]".into());
        let mut rows: Vec<Row> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for r in &mut rows {
            r.size_text = format_size(r.size);
        }
        return Ok(rows);
    }
    let mut rows: Vec<Row> = flatpak::list_updates(refresh)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(row)
        .collect();
    // Apps first, then runtimes; alphabetical within each.
    rows.sort_by(|a, b| {
        a.runtime
            .cmp(&b.runtime)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(rows)
}

/// Blocking. `progress` gets a short status line.
pub fn update_all(
    fixtures: Option<&Path>,
    mut progress: impl FnMut(String) + 'static,
) -> Result<(), String> {
    if fixtures.is_some() {
        progress("Updating…".into());
        std::thread::sleep(std::time::Duration::from_millis(300));
        return Ok(());
    }
    flatpak::update_all(move |p| {
        let what = p
            .reference
            .split('/')
            .nth(1)
            .unwrap_or(&p.reference)
            .to_string();
        progress(format!("{} {what}… {}%", p.status, p.percent));
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(12_300_000), "12.3 MB");
    }
}
