//! Small file helpers shared by the history and events writers.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::Path;

/// Append `line` (a newline is added) to `path`: refuses to follow a symlink
/// at the final component, starts with a newline if the file does not end in
/// one (a torn earlier write), creates it with `mode`, and syncs.
pub fn append_line(path: &Path, line: &str, mode: u32) -> io::Result<()> {
    let f = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let len = f.metadata()?.len();
    let mut data = String::with_capacity(line.len() + 2);
    if len > 0 {
        let mut last = [0u8; 1];
        f.read_exact_at(&mut last, len - 1)?;
        if last[0] != b'\n' {
            data.push('\n');
        }
    }
    data.push_str(line);
    data.push('\n');
    (&f).write_all(data.as_bytes())?;
    f.sync_all()
}

/// The lines of `bytes`, split on `\n`, each decoded lossily, so one torn
/// multibyte character spoils only its own line. Empty lines are dropped.
pub fn lossy_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_missing_newline_and_refuses_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, "torn").unwrap();
        append_line(&p, "{}", 0o644).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "torn\n{}\n");
        let l = d.path().join("link");
        std::os::unix::fs::symlink(&p, &l).unwrap();
        assert!(append_line(&l, "x", 0o644).is_err());
    }
}
