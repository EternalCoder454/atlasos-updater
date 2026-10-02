//! Release notes: GitHub release JSON (`.body`) from https, http or file://.

use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Notes {
    Found(String),
    /// No release for this version (404, missing file, empty body).
    Missing,
}

/// The version goes into a URL (or a file name), so every byte outside
/// `A-Za-z0-9._-` is percent-encoded and `.`/`..` cannot stand alone.
fn encode_version(version: &str) -> String {
    let enc: String = version
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    match enc.as_str() {
        "." | ".." | "" => "%2E".to_string() + &enc[1.min(enc.len())..],
        _ => enc,
    }
}

pub fn url_for(template: &str, version: &str) -> String {
    template.replace("{version}", &encode_version(version))
}

/// Why notes could not be loaded, in words for the user.
#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    RateLimited,
    Other(String),
}

impl FetchError {
    pub fn text(&self) -> String {
        match self {
            FetchError::RateLimited => "GitHub is limiting requests from this network right now. The release notes will load later.".into(),
            FetchError::Other(_) => "Could not load the release notes. Check your internet connection.".into(),
        }
    }
}

/// Only `https://` links are safe to open from release notes.
pub fn is_safe_link(link: &str) -> bool {
    let l = link.trim();
    l.len() > 8 && l[..8].eq_ignore_ascii_case("https://") && !l.chars().any(char::is_control)
}

/// Release notes are untrusted text. Drop images (Qt would fetch them when
/// the notes are shown) and HTML tags, and turn every link that is not
/// `https://` into its plain text.
pub fn sanitize(md: &str) -> String {
    let chars: Vec<char> = md.chars().collect();
    let mut out = String::with_capacity(md.len());
    let mut i = 0;
    // `]` index matching the `[` at `open`, if any (one line, nesting counted).
    let close_of = |open: usize| -> Option<usize> {
        let mut depth = 0;
        for (j, &c) in chars.iter().enumerate().skip(open) {
            match c {
                '\n' => return None,
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        None
    };
    // `(...)` right after `at`: the index of its `)` and the target text.
    let target_after = |at: usize| -> Option<(usize, String)> {
        if chars.get(at + 1) != Some(&'(') {
            return None;
        }
        let mut depth = 0;
        for j in at + 1..chars.len() {
            match chars[j] {
                '\n' => return None,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner: String = chars[at + 2..j].iter().collect();
                        let url = inner.split_whitespace().next().unwrap_or("").to_string();
                        return Some((j, url));
                    }
                }
                _ => {}
            }
        }
        None
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '!'
            && chars.get(i + 1) == Some(&'[')
            && let Some(close) = close_of(i + 1)
        {
            i = match target_after(close) {
                Some((end, _)) => end + 1,
                // `![alt][ref]`: drop the reference part too
                None if chars.get(close + 1) == Some(&'[') => {
                    close_of(close + 1).map_or(close + 1, |e| e + 1)
                }
                None => close + 1,
            };
            continue;
        }
        if c == '['
            && let Some(close) = close_of(i)
        {
            let text: String = sanitize(&chars[i + 1..close].iter().collect::<String>());
            if let Some((end, url)) = target_after(close) {
                if is_safe_link(&url) {
                    out.push_str(&format!("[{text}]({url})"));
                } else {
                    out.push_str(&text);
                }
                i = end + 1;
                continue;
            }
            // `[text][ref]` and `[ref]: url` forms: text only
            if chars.get(close + 1) == Some(&'[') {
                out.push_str(&text);
                i = close_of(close + 1).map_or(close + 1, |e| e + 1);
                continue;
            }
        }
        if c == '<'
            && chars
                .get(i + 1)
                .is_some_and(|n| n.is_ascii_alphabetic() || *n == '/' || *n == '!')
            && let Some(rel) = chars[i..].iter().position(|&x| x == '>')
        {
            let tag: String = chars[i + 1..i + rel].iter().collect();
            // `<https://...>` autolinks stay; any other tag goes
            if is_safe_link(&tag) && !tag.contains(char::is_whitespace) {
                out.push_str(&format!("<{tag}>"));
            }
            i += rel + 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    // reference definitions `[x]: http://...` would still resolve; drop them
    out.lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with('[') && t.contains("]:"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pulls `.body` out of a GitHub release JSON document.
pub fn parse_body(json: &str) -> Notes {
    let body = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("body").and_then(|b| b.as_str()).map(str::to_string))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match body {
        Some(b) => Notes::Found(sanitize(&b)),
        None => Notes::Missing,
    }
}

/// Blocking. Call from a worker thread.
pub fn fetch(template: &str, version: &str) -> Result<Notes, FetchError> {
    let url = url_for(template, version);
    if let Some(path) = url.strip_prefix("file://") {
        return match std::fs::read_to_string(path) {
            Ok(text) => Ok(parse_body(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Notes::Missing),
            Err(e) => Err(FetchError::Other(e.to_string())),
        };
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .user_agent(concat!("atlas-updater/", env!("CARGO_PKG_VERSION")))
        .max_redirects(3)
        // an https template never follows a redirect down to http
        .https_only(url.starts_with("https://"))
        .build()
        .into();
    let mut resp = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| FetchError::Other(e.to_string()))?;
    let status = resp.status().as_u16();
    if status == 404 || status == 410 {
        return Ok(Notes::Missing);
    }
    if status == 403 || status == 429 {
        return Err(FetchError::RateLimited);
    }
    if !(200..300).contains(&status) {
        return Err(FetchError::Other(format!("HTTP {status}")));
    }
    let text = resp
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| FetchError::Other(e.to_string()))?;
    Ok(parse_body(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_fills_version() {
        assert_eq!(
            url_for("file:///n/{version}.json", "44.1"),
            "file:///n/44.1.json"
        );
    }

    #[test]
    fn version_is_encoded_in_urls() {
        assert_eq!(
            url_for("https://h/{version}", "44.1/../x?y#z"),
            "https://h/44.1%2F..%2Fx%3Fy%23z"
        );
        assert_eq!(url_for("file:///n/{version}", ".."), "file:///n/%2E.");
        assert_eq!(
            url_for("https://h/{version}", "44.1-rc_2"),
            "https://h/44.1-rc_2"
        );
    }

    #[test]
    fn images_html_and_unsafe_links_are_removed() {
        let md = "Hi ![pixel](http://t.example/p.png) there\n![x][r]\n<img src=\"http://t/p\"> ok\n[good](https://example.org/a \"t\") [bad](file:///etc/passwd) [worse](javascript:x)\n<https://example.org> <http://plain.example>\n[r]: http://t.example/p.png";
        let out = sanitize(md);
        assert_eq!(
            out,
            "Hi  there\n\n ok\n[good](https://example.org/a) bad worse\n<https://example.org> "
        );
        assert!(!out.contains("http://") && !out.contains("img"));
        assert!(
            is_safe_link("https://x.example/a")
                && !is_safe_link("http://x")
                && !is_safe_link("file:///x")
                && !is_safe_link("HTTPS:// ")
        );
    }

    #[test]
    fn body() {
        assert_eq!(
            parse_body("{\"tag_name\":\"44.1\",\"body\":\"## Changes\\n- a\\n\"}"),
            Notes::Found("## Changes\n- a".into())
        );
        assert_eq!(parse_body(r#"{"body":null}"#), Notes::Missing);
        assert_eq!(parse_body(r#"{"message":"Not Found"}"#), Notes::Missing);
        assert_eq!(parse_body("nonsense"), Notes::Missing);
    }

    #[test]
    fn file_url() {
        let dir = std::env::temp_dir().join(format!("atlas-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("44.2.json"), r#"{"body":"hello"}"#).unwrap();
        let t = format!("file://{}/{{version}}.json", dir.display());
        assert_eq!(fetch(&t, "44.2").unwrap(), Notes::Found("hello".into()));
        assert_eq!(fetch(&t, "44.3").unwrap(), Notes::Missing);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
