//! The tiny slice of `~/.config/atlas-updaterrc` (KConfig INI format) we need:
//! the scheduled restart and the staged update we already told the user about.

use std::path::PathBuf;

pub fn path() -> PathBuf {
    // XDG: relative values are ignored.
    let abs = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let base = abs("XDG_CONFIG_HOME")
        .or_else(|| abs("HOME").map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    base.join("atlas-updaterrc")
}

pub fn get(group: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path()).ok()?;
    get_in(&text, group, key)
}

/// `None` removes the key.
pub fn set(group: &str, key: &str, value: Option<&str>) {
    let p = path();
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let out = set_in(&text, group, key, value);
    if out != text {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Write a temp file and rename it: a crash or a second instance
        // writing at the same time never leaves a half-written file.
        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, &p).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn header(line: &str) -> Option<&str> {
    line.trim().strip_prefix('[')?.strip_suffix(']')
}

pub fn get_in(text: &str, group: &str, key: &str) -> Option<String> {
    let mut in_group = false;
    for line in text.lines() {
        if let Some(g) = header(line) {
            in_group = g == group;
        } else if in_group
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

pub fn set_in(text: &str, group: &str, key: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_group = false;
    let mut group_seen = false;
    let mut done = false;
    let new_line = value.map(|v| format!("{key}={v}"));
    let flush = |out: &mut Vec<String>, done: &mut bool| {
        if !*done {
            if let Some(l) = &new_line {
                // Insert before trailing blank lines of the group.
                let mut at = out.len();
                while at > 0 && out[at - 1].trim().is_empty() {
                    at -= 1;
                }
                out.insert(at, l.clone());
            }
            *done = true;
        }
    };
    for line in text.lines() {
        if let Some(g) = header(line) {
            if in_group {
                flush(&mut out, &mut done);
            }
            in_group = g == group;
            group_seen |= in_group;
            out.push(line.to_string());
            continue;
        }
        if in_group
            && let Some((k, _)) = line.split_once('=')
            && k.trim() == key
        {
            if !done {
                if let Some(l) = &new_line {
                    out.push(l.clone());
                }
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if in_group {
        flush(&mut out, &mut done);
    }
    if !group_seen
        && !done
        && let Some(l) = &new_line
    {
        if !out.is_empty() && !out.last().is_some_and(|l| l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push(format!("[{group}]"));
        out.push(l.clone());
    }
    let mut s = out.join("\n");
    if !s.is_empty() {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_remove() {
        let t = set_in("", "Restart", "ScheduledAt", Some("100"));
        assert_eq!(t, "[Restart]\nScheduledAt=100\n");
        assert_eq!(get_in(&t, "Restart", "ScheduledAt").as_deref(), Some("100"));
        let t = set_in(&t, "Restart", "ScheduledAt", Some("200"));
        assert_eq!(get_in(&t, "Restart", "ScheduledAt").as_deref(), Some("200"));
        let t = set_in(&t, "Notified", "Digest", Some("sha256:x"));
        assert_eq!(get_in(&t, "Restart", "ScheduledAt").as_deref(), Some("200"));
        assert_eq!(
            get_in(&t, "Notified", "Digest").as_deref(),
            Some("sha256:x")
        );
        let t = set_in(&t, "Restart", "ScheduledAt", None);
        assert_eq!(get_in(&t, "Restart", "ScheduledAt"), None);
        assert_eq!(
            get_in(&t, "Notified", "Digest").as_deref(),
            Some("sha256:x")
        );
    }

    #[test]
    fn keeps_other_groups() {
        let t = "[General]\nA=1\n\n[Restart]\nB=2\n";
        let t = set_in(t, "Restart", "ScheduledAt", Some("5"));
        assert_eq!(t, "[General]\nA=1\n\n[Restart]\nB=2\nScheduledAt=5\n");
    }
}
