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

fn starts_https(s: &[char]) -> bool {
    s.len() > 8
        && s[..8]
            .iter()
            .zip("https://".chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

/// Only `https://` links are safe to open from release notes. Never panics
/// (it runs on the GUI thread).
pub fn is_safe_link(link: &str) -> bool {
    let l: Vec<char> = link.trim().chars().take(4096).collect();
    starts_https(&l) && !l.iter().any(|c| c.is_control())
}

/// Longest line we look at; the rest of a longer line is cut.
const MAX_LINE: usize = 4000;
/// Longest tag we recognize.
const MAX_TAG: usize = 300;
/// HTML Qt may render without loading anything or hiding the text.
const PLAIN_TAGS: &[&str] = &[
    "b",
    "i",
    "em",
    "strong",
    "code",
    "pre",
    "br",
    "p",
    "ul",
    "ol",
    "li",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "kbd",
    "sub",
    "sup",
    "s",
    "del",
    "hr",
    "details",
    "summary",
    "tt",
    "u",
    "strike",
];

struct Out {
    s: String,
    /// Backslashes right before the end (an odd count escapes the next char).
    bs: usize,
    last: Option<char>,
}

impl Out {
    fn push(&mut self, c: char) {
        self.bs = if c == '\\' { self.bs + 1 } else { 0 };
        self.last = Some(c);
        self.s.push(c);
    }
}

/// For each index, the next position at or after it holding `c` (`n` if none).
fn next_of(chars: &[char], c: char) -> Vec<usize> {
    let n = chars.len();
    let mut v = vec![n; n + 1];
    for i in (0..n).rev() {
        v[i] = if chars[i] == c { i } else { v[i + 1] };
    }
    v
}

fn tag_is_plain(inner: &[char]) -> bool {
    let inner = inner.strip_prefix(&['/']).unwrap_or(inner);
    let name: String = inner
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    let rest: String = inner[name.len()..].iter().collect();
    PLAIN_TAGS.contains(&name.as_str()) && matches!(rest.trim(), "" | "/")
}

/// Index of the `[` of a link reference definition (`[label]: dest`) at the
/// start of the line (after quote and list markers) whose destination is not
/// https. Such a definition could point a later `[text]` at any URL.
fn unsafe_definition(chars: &[char]) -> Option<usize> {
    let n = chars.len();
    let mut k = 0;
    loop {
        while k < n && chars[k] == ' ' {
            k += 1;
        }
        match chars.get(k) {
            Some('>') => k += 1,
            Some('-' | '*' | '+') if chars.get(k + 1) == Some(&' ') => k += 2,
            Some(d) if d.is_ascii_digit() => {
                let mut j = k;
                while j < n && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if matches!(chars.get(j), Some('.' | ')')) && chars.get(j + 1) == Some(&' ') {
                    k = j + 2;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    if chars.get(k) != Some(&'[') {
        return None;
    }
    // the first `]` that is not escaped (`[a\]b]: dest`)
    let close = (k + 1..n).find(|&i| chars[i] == ']' && chars[i - 1] != '\\')?;
    if chars.get(close + 1) != Some(&':') {
        return None;
    }
    let mut j = close + 2;
    while j < n && chars[j] == ' ' {
        j += 1;
    }
    let dest: Vec<char> = chars[j..]
        .iter()
        .take_while(|c| !c.is_whitespace())
        .copied()
        .collect();
    (!starts_https(&dest)).then_some(k)
}

/// For each backtick run (maximal, so a longer run is never split) the start
/// of the next run of exactly the same length, indexed by its start.
fn code_closers(chars: &[char]) -> Vec<Option<(usize, usize)>> {
    let n = chars.len();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < n {
        if chars[i] == '`' {
            let st = i;
            while i < n && chars[i] == '`' {
                i += 1;
            }
            runs.push((st, i - st));
        } else {
            i += 1;
        }
    }
    let mut out = vec![None; n];
    let mut seen: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for &(st, len) in runs.iter().rev() {
        out[st] = seen.get(&len).map(|&c| (len, c));
        seen.insert(len, st);
    }
    out
}

fn sanitize_line(line: &str) -> String {
    let mut chars: Vec<char> = line.chars().take(MAX_LINE + 1).collect();
    let cut = chars.len() > MAX_LINE;
    chars.truncate(MAX_LINE);
    let n = chars.len();
    let next_gt = next_of(&chars, '>');
    let next_paren = next_of(&chars, ')');
    let escape_def = unsafe_definition(&chars);
    let closers = code_closers(&chars);
    let mut o = Out {
        s: String::with_capacity(n),
        bs: 0,
        last: None,
    };
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if escape_def == Some(i) {
            o.push('\\');
        }
        match c {
            // No `![` ever survives, so no Markdown image can be formed
            // (however the brackets are nested or split over lines).
            // (An escaped bracket is literal text.)
            '[' if o.last == Some('!') => {
                o.push('\\');
                o.push('[');
                i += 1;
                continue;
            }
            // A code span is literal in Markdown, so it passes through as is
            // (`Vec<String>`). Only a span that really closes on this line; an
            // escaped or unclosed run is plain text and is handled as such.
            '`' if o.bs.is_multiple_of(2) && (i == 0 || chars[i - 1] != '`') => {
                if let Some((len, close)) = closers[i] {
                    for &ch in &chars[i..close + len] {
                        o.push(ch);
                    }
                    i = close + len;
                } else {
                    while i < n && chars[i] == '`' {
                        o.push('`');
                        i += 1;
                    }
                }
                continue;
            }
            '`' => {
                // an escaped run: all of it is text (never an opener here)
                while i < n && chars[i] == '`' {
                    o.push('`');
                    i += 1;
                }
                continue;
            }
            // A link whose target is not https loses its target.
            ']' if chars.get(i + 1) == Some(&'(') => {
                let mut j = i + 2;
                while j < n && chars[j] == ' ' {
                    j += 1;
                }
                if chars.get(j) == Some(&'<') {
                    j += 1;
                }
                if !starts_https(&chars[j.min(n)..]) {
                    o.push(']');
                    if next_paren[i + 2] < n {
                        i = next_paren[i + 2] + 1;
                        // `](x)(http://...)` must not join into a new target
                        if chars.get(i) == Some(&'(') {
                            o.push(' ');
                        }
                    } else {
                        o.push(' ');
                        i += 1;
                    }
                    continue;
                }
            }
            '<' if o.bs.is_multiple_of(2) => {
                let gt = next_gt[i];
                if gt < n && gt - i <= MAX_TAG {
                    let inner = &chars[i + 1..gt];
                    let comment = inner.len() >= 5
                        && inner.starts_with(&['!', '-', '-'])
                        && inner.ends_with(&['-', '-']);
                    if comment {
                        i = gt + 1;
                        continue;
                    }
                    let autolink = starts_https(inner)
                        && !inner.iter().any(|c| c.is_whitespace() || *c == '<');
                    if autolink || tag_is_plain(inner) {
                        // the whole tag at once: a backtick inside an autolink
                        // is not the start of a code span
                        for &ch in &chars[i..=gt] {
                            o.push(ch);
                        }
                        i = gt + 1;
                        continue;
                    }
                }
                // Not a tag we trust: show it as text.
                o.push('\\');
            }
            _ => {}
        }
        o.push(c);
        i += 1;
    }
    if cut {
        o.s.push('…');
    }
    o.s
}

/// A fence line: up to 3 spaces, then 3 or more backticks (none in the rest
/// of the line) or tildes. Returns the fence character and its length.
fn fence_of(line: &str) -> Option<(char, usize)> {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return None;
    }
    let c = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = t.chars().take_while(|&x| x == c).count();
    let rest = &t[len..];
    (len >= 3 && !(c == '`' && rest.contains('`'))).then_some((c, len))
}

/// Release notes are untrusted text. Nothing in the result can make Qt load
/// a resource: no Markdown image survives, no HTML except plain formatting
/// tags, and links keep only `https://` targets. Code spans and fenced code
/// blocks are literal text and pass through. Linear time, no recursion.
pub fn sanitize(md: &str) -> String {
    // MD4C ends a line at \r too
    let md = md.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    // the fence that is open, and whether the previous line was blank (a
    // fence only counts after a blank line, so it is never inside an HTML
    // block or a paragraph that Markdown may read differently)
    let mut open: Option<(char, usize)> = None;
    let mut prev_blank = true;
    for line in md.lines() {
        let blank = line.trim().is_empty();
        match (open, fence_of(line)) {
            (Some((c, len)), f) => {
                let closes = f.is_some_and(|(fc, fl)| fc == c && fl >= len)
                    && line.trim().chars().all(|x| x == c);
                if closes {
                    open = None;
                }
                out.push(line.to_string());
            }
            (None, Some(f)) if prev_blank => {
                open = Some(f);
                out.push(line.to_string());
            }
            _ => out.push(sanitize_line(line)),
        }
        prev_blank = blank;
    }
    out.join("\n")
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
    fn images_never_survive() {
        for md in [
            "Hi ![pixel](http://t.example/p.png) there",
            "![x][r]\n\n[r]: http://t.example/p.png",
            "![alt\nmore](http://t.example/p.png)",
            "![a`[`](http://t.example/p.png)",
            "!![[a](http://t.example/p.png)",
            "![![a](b)](c)",
        ] {
            let out = sanitize(md);
            assert!(!out.contains("!["), "{md:?} -> {out:?}");
            assert!(
                !out.contains("](http://") && !out.contains("\n[r]:"),
                "{md:?} -> {out:?}"
            );
        }
        assert_eq!(
            sanitize("Hi ![pixel](http://t.example/p.png) there"),
            "Hi !\\[pixel] there"
        );
    }

    #[test]
    fn unsafe_link_targets_are_dropped_and_https_kept() {
        assert_eq!(
            sanitize(
                "[good](https://example.org/a) [bad](file:///etc/passwd) [worse](javascript:x)"
            ),
            "[good](https://example.org/a) [bad] [worse]"
        );
        assert_eq!(sanitize("[a\nb](javascript:x)"), "[a\nb]");
        assert_eq!(
            sanitize("[open](  <HTTPS://e.org>)"),
            "[open](  <HTTPS://e.org>)"
        );
        assert_eq!(sanitize("[x](nope"), "[x] (nope");
    }

    #[test]
    fn html_is_neutralized_without_eating_text() {
        assert_eq!(
            sanitize("<img src=\"http://t/p\"> ok"),
            "\\<img src=\"http://t/p\"> ok"
        );
        assert_eq!(sanitize("a <b>bold</b> <br/>"), "a <b>bold</b> <br/>");
        assert_eq!(
            sanitize("Vec<String> and width <height later -> x"),
            "Vec\\<String> and width \\<height later -> x"
        );
        assert_eq!(sanitize("x <!-- hidden --> y"), "x  y");
        assert_eq!(
            sanitize("<https://example.org> <javascript:x>"),
            "<https://example.org> \\<javascript:x>"
        );
        // already escaped by the author: no second backslash
        assert_eq!(sanitize("\\<img src=x>"), "\\<img src=x>");
        assert_eq!(
            sanitize("<a href=\"http://x\">t</a>"),
            "\\<a href=\"http://x\">t\\</a>"
        );
    }

    #[test]
    fn reference_definitions() {
        assert_eq!(sanitize("[Security]: fixed X"), "\\[Security]: fixed X");
        assert_eq!(
            sanitize("> [r]: http://t.example/p"),
            "> \\[r]: http://t.example/p"
        );
        assert_eq!(
            sanitize("- 1. [r]:\nhttp://t.example"),
            "- 1. [r]:\nhttp://t.example".replacen("[r]", "\\[r]", 1)
        );
        assert_eq!(
            sanitize("[r]: https://ok.example/x"),
            "[r]: https://ok.example/x"
        );
    }

    #[test]
    fn hostile_input_is_linear_and_does_not_panic() {
        let t = std::time::Instant::now();
        for md in [
            "[".repeat(2_000_000),
            "![".repeat(1_000_000),
            "<a ".repeat(600_000),
            "](".repeat(1_000_000),
            "[[[[]]]](x)".repeat(100_000),
            "é".repeat(2_000_000),
        ] {
            let _ = sanitize(&md);
        }
        assert!(t.elapsed() < std::time::Duration::from_secs(10));
        let long = format!("{}\nnext", "a".repeat(10_000));
        let out = sanitize(&long);
        assert!(out.ends_with("…\nnext"));
    }

    #[test]
    fn safe_link_check() {
        assert!(is_safe_link("https://x.example/a") && is_safe_link(" HTTPS://x.example "));
        assert!(
            !is_safe_link("http://x") && !is_safe_link("file:///x") && !is_safe_link("HTTPS:// ")
        );
        // byte 8 inside a multibyte char must not panic
        assert!(!is_safe_link("docs/été") && !is_safe_link("[x](docs/été)"));
        assert!(!is_safe_link("https://x\u{0}y"));
        assert!(!is_safe_link(""));
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

    #[test]
    fn round5_sanitizer_cases() {
        // a dropped target cannot be joined with a following one
        let o = sanitize("[a](x)(http://evil.example/p)");
        assert!(!o.contains("](http://"), "{o}");
        // code spans and fences are literal text
        assert_eq!(sanitize("use `Vec<String>` here"), "use `Vec<String>` here");
        assert_eq!(
            sanitize("text\n\n```\nlet v: Vec<String>;\n![x](http://e/i.png)\n```\nafter <b>"),
            "text\n\n```\nlet v: Vec<String>;\n![x](http://e/i.png)\n```\nafter <b>"
        );
        // ... but not a fence glued to a paragraph, nor an unclosed span
        assert!(!sanitize("para\n```\n![x](http://e/i.png)").contains("![x]"));
        let o = sanitize("`unclosed ![x](http://e/i.png)");
        assert!(!o.contains("![x]"), "{o}");
        // a backtick inside an autolink does not open a span
        let o = sanitize("<https://a`b> ![i](http://e/i.png) `");
        assert!(!o.contains("![i]"), "{o}");
        // an escaped backtick does not open one either
        let o = sanitize("\\`x ![i](http://e/i.png) `");
        assert!(!o.contains("![i]"), "{o}");
        // a bare CR ends a line
        let o = sanitize("x\r[r]: http://e/p");
        assert!(o.contains("\n\\[r]:"), "{o:?}");
        // an escaped bracket in a definition label
        let o = sanitize("[a\\]b]: http://x");
        assert!(o.starts_with("\\["), "{o}");
        // `!` before a bracket stays visible
        assert_eq!(
            sanitize("Fixed![#12](https://x.y/1)"),
            "Fixed!\\[#12](https://x.y/1)"
        );
    }
}
