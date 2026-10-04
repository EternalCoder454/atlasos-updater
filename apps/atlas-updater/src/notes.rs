//! Release notes: GitHub release JSON (`.body`) from https, http or file://.

use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Notes {
    /// The HTML fragment from [`render`] and the plain text from [`render_plain`].
    Found(String, String),
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

/// Characters that never belong in a link: controls, any Unicode whitespace
/// (U+00A0, U+2028 ...) and the invisible format characters (zero-width,
/// bidi controls U+202A-202E and U+2066-2069, BOM, tags). U+200C and U+200D
/// (joiners) stay: emoji sequences and Persian or Indic text need them.
fn is_hidden(c: char) -> bool {
    c.is_control()
        || c.is_whitespace()
        || matches!(c,
            '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}' | '\u{180e}'
            | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}' | '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')
}

/// The longest link we accept, in characters.
const MAX_LINK: usize = 4096;

/// A safe https link: the trimmed URL and its lower-case host. The host must be
/// real (letters, digits, `-` and non-empty dot-separated labels, an optional
/// numeric port), and the authority holds no `@` (user info hides the real
/// host) and no `\` (browsers read it as `/`).
fn parse_https(link: &str) -> Option<(&str, String)> {
    let t = link.trim_matches(|c: char| c.is_ascii_whitespace());
    if t.chars().count() > MAX_LINK || t.chars().any(is_hidden) {
        return None;
    }
    let scheme = t.get(..8)?;
    if !scheme.eq_ignore_ascii_case("https://") {
        return None;
    }
    let rest = &t[8..];
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.contains(['@', '\\']) {
        return None;
    }
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if port.is_some_and(|p| p.is_empty() || p.len() > 5 || !p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let host_ok = !host.is_empty()
        && host
            .split('.')
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    host_ok.then(|| (t, host.to_ascii_lowercase()))
}

/// Only `https://` links are safe to open from release notes. Never panics
/// (it runs on the GUI thread).
pub fn is_safe_link(link: &str) -> bool {
    parse_https(link).is_some()
}

/// Last labels that are file extensions or names of tools and are not top-level
/// domains: `Node.js`, `notes.txt`. Real TLDs (`md`, `rs`, `py`, `sh`, `zip`,
/// `pl`, `cc`, `so`, `java`, ...) must never be listed: `microsoft.zip` is a site.
/// Checked against IANA tlds-alpha-by-domain.txt version 2026100200; new gTLDs
/// can appear, so keep this list short.
const FILE_TAILS: &[&str] = &[
    "js", "ts", "rb", "go", "cpp", "hpp", "txt", "json", "toml", "yaml", "yml", "xml", "html",
    "htm", "css", "csv", "log", "conf", "cfg", "ini", "lock", "rpm", "deb", "gz", "xz", "tar",
    "tgz", "iso", "img", "png", "jpg", "jpeg", "gif", "svg", "pdf", "doc", "docx", "odt", "rlib",
    "service", "desktop", "patch", "diff", "php", "kt", "lua", "mjs", "cjs", "jsx", "tsx",
];

/// Drops a leading `www.` when at least two labels remain.
fn strip_www(host: &str) -> &str {
    match host.strip_prefix("www.") {
        Some(rest) if rest.contains('.') => rest,
        _ => host,
    }
}

/// The host a link's text claims, when the text looks like a URL or a domain
/// (`https://bank.com/login`, `www.bank.com`, `bank.com`). Hidden characters
/// and trailing dots are dropped first, fullwidth and ideographic dots count as
/// dots, and a leading `www.` is dropped when two or more labels remain.
fn text_host(text: &str) -> Option<String> {
    let cleaned: String = text
        .chars()
        .filter(|c| !is_hidden(*c) || c.is_whitespace())
        .map(|c| {
            if matches!(c, '\u{3002}' | '\u{FF0E}' | '\u{FF61}') {
                '.'
            } else {
                c
            }
        })
        .collect();
    let t = cleaned.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    let scheme = ["https://", "http://", "ftp://"].iter().find_map(|p| {
        t.get(..p.len())
            .filter(|h| h.eq_ignore_ascii_case(p))
            .map(|_| &t[p.len()..])
    });
    let t = scheme.unwrap_or(t);
    let auth = &t[..t.find(['/', '?', '#']).unwrap_or(t.len())];
    let auth = auth.rsplit('@').next().unwrap_or(auth);
    let host = auth.split(':').next().unwrap_or(auth).to_lowercase();
    let host = host.trim_end_matches('.');
    let host = strip_www(host);
    let labels: Vec<&str> = host.split('.').collect();
    let last = labels.last()?;
    let shaped = labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
        && (2..=24).contains(&last.chars().count())
        && last.chars().all(char::is_alphabetic);
    // a bare name needs a TLD-like tail; with a scheme or `www.` it is a site
    let claims_site = scheme.is_some() || t.to_lowercase().starts_with("www.");
    (shaped && (claims_site || !FILE_TAILS.contains(last))).then(|| host.to_string())
}

/// `s` as HTML text. Invisible and bidi characters are dropped, as in
/// [`render_plain`] (they would let a note reorder or hide what is shown).
fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            c if is_hidden(c) && !c.is_whitespace() => {}
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

/// Containers nested deeper than this are shown as plain text blocks.
const MAX_DEPTH: usize = 24;

/// Release notes are untrusted. This parses the Markdown and writes the HTML
/// itself from an allow-list, so nothing the author wrote can reach the output
/// as markup: every text is escaped, raw HTML is shown as text, an image is
/// shown as its alt text, and a link keeps its target only when it is https.
/// No attributes other than a link's `href`; the view adds the styling.
pub fn render(md: &str) -> String {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
    let mut out = String::with_capacity(md.len() + md.len() / 2);
    // For every open container: the closing text to write for it.
    let mut stack: Vec<&'static str> = Vec::new();
    // Open links: the host of each one written as <a> (None: written as text)
    // and the text shown for it so far.
    let mut links: Vec<(Option<String>, String)> = Vec::new();
    let open =
        |out: &mut String, stack: &mut Vec<&'static str>, o: &'static str, c: &'static str| {
            if stack.len() < MAX_DEPTH {
                out.push_str(o);
                stack.push(c);
            } else {
                stack.push("");
            }
        };
    let opts = Options::ENABLE_STRIKETHROUGH;
    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => open(&mut out, &mut stack, "<p>", "</p>\n"),
                Tag::Heading { level, .. } => match level {
                    HeadingLevel::H1 => open(&mut out, &mut stack, "<h3>", "</h3>\n"),
                    HeadingLevel::H2 => open(&mut out, &mut stack, "<h4>", "</h4>\n"),
                    _ => open(&mut out, &mut stack, "<h5>", "</h5>\n"),
                },
                Tag::BlockQuote(_) => open(&mut out, &mut stack, "<blockquote>", "</blockquote>\n"),
                Tag::CodeBlock(_) => open(&mut out, &mut stack, "<pre>", "</pre>\n"),
                Tag::List(None) => open(&mut out, &mut stack, "<ul>\n", "</ul>\n"),
                Tag::List(Some(_)) => open(&mut out, &mut stack, "<ol>\n", "</ol>\n"),
                Tag::Item => open(&mut out, &mut stack, "<li>", "</li>\n"),
                Tag::Emphasis => open(&mut out, &mut stack, "<em>", "</em>"),
                Tag::Strong => open(&mut out, &mut stack, "<strong>", "</strong>"),
                Tag::Strikethrough => open(&mut out, &mut stack, "<s>", "</s>"),
                Tag::Link { dest_url, .. } => match parse_https(&dest_url) {
                    Some((url, host)) if stack.len() < MAX_DEPTH => {
                        links.push((Some(host), String::new()));
                        out.push_str("<a href=\"");
                        escape(url, &mut out);
                        out.push_str("\">");
                    }
                    _ => links.push((None, String::new())),
                },
                // an image is its alt text (the events inside); raw HTML
                // blocks get no wrapper, their text is escaped below
                _ => stack.push(""),
            },
            Event::End(end) => match end {
                TagEnd::Link => {
                    if let Some((Some(host), text)) = links.pop() {
                        out.push_str("</a>");
                        // Text that names another site than the link goes to:
                        // show where it really goes.
                        if text_host(&text)
                            // (parse_https already rejects a host with a trailing dot)
                            .is_some_and(|t| t != strip_www(host.trim_end_matches('.')))
                        {
                            out.push_str(" (");
                            escape(host.trim_end_matches('.'), &mut out);
                            out.push(')');
                        }
                    }
                }
                _ => {
                    if let Some(c) = stack.pop() {
                        out.push_str(c);
                    }
                }
            },
            Event::Code(t) => {
                if let Some(l) = links.last_mut() {
                    l.1.push_str(&t);
                }
                out.push_str("<code>");
                escape(&t, &mut out);
                out.push_str("</code>");
            }
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                if let Some(l) = links.last_mut() {
                    l.1.push_str(&t);
                }
                escape(&t, &mut out)
            }
            Event::SoftBreak => out.push('\n'),
            Event::HardBreak => out.push_str("<br>\n"),
            Event::Rule => out.push_str("<hr>\n"),
            Event::FootnoteReference(l) => escape(&l, &mut out),
            _ => {}
        }
    }
    out
}

/// The notes as plain text for assistive technology: no markup, a line per
/// paragraph, heading, list item and code line.
pub fn render_plain(md: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    fn newline(out: &mut String) {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    let mut out = String::with_capacity(md.len());
    // open links: the real host (None when the link is not safe) and the text
    let mut links: Vec<(Option<String>, String)> = Vec::new();
    for ev in Parser::new_ext(md, Options::ENABLE_STRIKETHROUGH) {
        match ev {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => {
                let t: String = t
                    .chars()
                    .filter(|c| !is_hidden(*c) || c.is_whitespace())
                    .collect();
                out.push_str(&t);
                if let Some(l) = links.last_mut() {
                    l.1.push_str(&t);
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                links.push((parse_https(&dest_url).map(|(_, h)| h), String::new()))
            }
            Event::End(TagEnd::Link) => {
                if let Some((Some(host), text)) = links.pop()
                    // (parse_https already rejects a host with a trailing dot)
                    && text_host(&text).is_some_and(|t| t != strip_www(host.trim_end_matches('.')))
                {
                    out.push_str(" (");
                    out.push_str(host.trim_end_matches('.'));
                    out.push(')');
                }
            }
            Event::SoftBreak => out.push(' '),
            Event::HardBreak | Event::Rule => newline(&mut out),
            Event::Start(Tag::Item) => {
                newline(&mut out);
                out.push_str("- ");
            }
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::CodeBlock
                | TagEnd::Item
                | TagEnd::BlockQuote(_),
            ) => newline(&mut out),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Pulls `.body` out of a GitHub release JSON document.
pub fn parse_body(json: &str) -> Notes {
    let body = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("body").and_then(|b| b.as_str()).map(str::to_string))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match body
        .map(|b| (render(&b), b))
        .filter(|(h, _)| !h.trim().is_empty())
    {
        Some((h, b)) => Notes::Found(h, render_plain(&b)),
        None => Notes::Missing,
    }
}

/// Blocking. Call from a worker thread.
pub fn fetch(template: &str, version: &str) -> Result<Notes, FetchError> {
    Ok(get(&url_for(template, version))?.map_or(Notes::Missing, |t| parse_body(&t)))
}

/// One release from GitHub's release list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag, which is the version (44.YYYYMMDD-N; older ones 44.YYYYMMDD).
    pub version: String,
    /// When it was published, RFC 3339 (may be empty).
    pub date: String,
    /// [`render`] and [`render_plain`] of the body; empty when it has none.
    pub html: String,
    pub plain: String,
}

/// The release list's URL for the notes `template`: GitHub's
/// `.../releases/tags/{version}` becomes `.../releases?per_page=100`; any
/// other template (`file:///dir/{version}.json`) gets `releases` for the
/// version (`file:///dir/releases.json`).
pub fn releases_url(template: &str) -> String {
    match template.strip_suffix("/tags/{version}") {
        Some(base) => format!("{base}?per_page=100"),
        None => template.replace("{version}", "releases"),
    }
}

/// GitHub's release list (a JSON array), drafts left out.
pub fn parse_releases(json: &str) -> Vec<Release> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(json) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|r| r["draft"] != serde_json::Value::Bool(true))
        .filter_map(|r| {
            let version = r["tag_name"].as_str()?.trim();
            if version.is_empty() {
                return None;
            }
            let body = r["body"].as_str().unwrap_or("").trim();
            let html = if body.is_empty() {
                String::new()
            } else {
                render(body)
            };
            Some(Release {
                version: version.to_string(),
                date: r["published_at"].as_str().unwrap_or("").to_string(),
                plain: if html.trim().is_empty() {
                    String::new()
                } else {
                    render_plain(body)
                },
                html: if html.trim().is_empty() {
                    String::new()
                } else {
                    html
                },
            })
        })
        .collect()
}

/// The raw release list from the URL for `template` (see [`releases_url`]);
/// `None` when there is none. Blocking.
pub fn fetch_releases(template: &str) -> Result<Option<String>, FetchError> {
    get(&releases_url(template))
}

/// The text at `url` (https, http or file://); `None` for a missing one.
fn get(url: &str) -> Result<Option<String>, FetchError> {
    if let Some(path) = url.strip_prefix("file://") {
        return match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
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
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| FetchError::Other(e.to_string()))?;
    let status = resp.status().as_u16();
    if status == 404 || status == 410 {
        return Ok(None);
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
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_drops_bidi_and_zero_width_characters() {
        let html = render("fix\u{202e}gnp.exe \u{200b}safe\u{2066}\u{feff}\u{e0041} text\n\nnext");
        for c in ['\u{202e}', '\u{200b}', '\u{2066}', '\u{feff}', '\u{e0041}'] {
            assert!(!html.contains(c), "{c:?} in {html:?}");
        }
        assert!(html.contains("fixgnp.exe safe text"), "{html}");
        assert!(html.contains("<p>next</p>"), "{html}");
    }

    #[test]
    fn joiners_stay_for_emoji_and_persian_text() {
        let html = render("family \u{1f468}\u{200d}\u{1f469} می\u{200c}خواهم \u{200f}x");
        assert!(html.contains("\u{1f468}\u{200d}\u{1f469}"), "{html}");
        assert!(html.contains("می\u{200c}خواهم"), "{html}");
        assert!(!html.contains('\u{200f}'), "{html}");
    }

    #[test]
    fn release_list_url() {
        assert_eq!(
            releases_url(crate::config::DEFAULT_NOTES_URL),
            "https://api.github.com/repos/EternalCoder454/AtlasOS/releases?per_page=100"
        );
        assert_eq!(
            releases_url("file:///var/notes/{version}.json"),
            "file:///var/notes/releases.json"
        );
    }

    #[test]
    fn release_list_parses_and_skips_drafts() {
        let json = r#"[
            {"tag_name": "44.20261002", "published_at": "2026-10-02T20:00:00Z", "body": "- Plasma 6.7.5"},
            {"tag_name": "44.20260925", "draft": true, "body": "secret"},
            {"tag_name": " ", "body": "x"},
            {"tag_name": "44.20260924", "body": ""}
        ]"#;
        let r = parse_releases(json);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].version, "44.20261002");
        assert_eq!(r[0].date, "2026-10-02T20:00:00Z");
        assert!(r[0].html.contains("Plasma 6.7.5"));
        assert!(r[0].plain.contains("Plasma 6.7.5"));
        assert_eq!(r[1].version, "44.20260924");
        assert!(r[1].html.is_empty() && r[1].plain.is_empty());
        assert!(parse_releases("{}").is_empty());
        assert!(parse_releases("not json").is_empty());
    }

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

    /// Every `<` in `out` must start one of the tags the renderer writes, and
    /// every link must be https; no image, ever.
    fn assert_safe(out: &str, input: &str) {
        const OK: &[&str] = &[
            "p",
            "h3",
            "h4",
            "h5",
            "blockquote",
            "pre",
            "ul",
            "ol",
            "li",
            "em",
            "strong",
            "s",
            "code",
            "br",
            "hr",
            "a",
        ];
        assert!(
            !out.to_ascii_lowercase().contains("<img"),
            "{input:?} -> {out:?}"
        );
        let mut rest = out;
        while let Some(i) = rest.find('<') {
            let tail = &rest[i + 1..];
            let end = tail
                .find('>')
                .unwrap_or_else(|| panic!("{input:?} -> {out:?}"));
            let tag = &tail[..end];
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            assert!(
                OK.contains(&name.as_str()),
                "tag {tag:?}: {input:?} -> {out:?}"
            );
            if name == "a" && !tag.starts_with('/') {
                assert!(
                    tag.starts_with("a href=\"https://")
                        && tag.ends_with('"')
                        && !tag[8..tag.len() - 1].contains('"'),
                    "{tag:?}: {input:?} -> {out:?}"
                );
            } else {
                assert!(
                    !tag.contains(' ') && !tag.contains('"'),
                    "{tag:?}: {input:?} -> {out:?}"
                );
            }
            rest = &tail[end + 1..];
        }
        assert!(!out.contains("href=\"http:") && !out.contains("href=\"file:"));
    }

    const BYPASSES: &[&str] = &[
        "`a\nx` ![i](http://e/i.png) `y",
        "- a\n\n  ```\n  x\n- b ![i](http://e/i.png)",
        "<pre>\n\n```\n<img src=\"http://e/i.png\">\n```\n",
        "<p>\n<img src=\"http://e/i.png\">",
        "[a]<!---->(file:///etc/passwd)",
        "[a\nb]: http://evil\n\n[a b]",
        "[a](x)(http://evil.example/p)",
        "![i](http://e/i.png)",
        "![![a](b)](c)",
        "<img src=\"http://e/i.png\">",
        "<IMG SRC=http://e/i.png>",
        "<https://a`b> ![i](http://e/i.png) `",
        "\\`x ![i](http://e/i.png) `",
        "x\r[r]: http://e/p\r\r[r]",
        "[a\\]b]: http://x\n\n[a\\]b]",
        "Fixed![#12](https://x.y/1)",
        "[x](javascript:alert(1)) [y](file:///etc/passwd) [z](HTTP://e)",
        "<a href=\"http://e\">x</a> <script>alert(1)</script>",
        "> ![i](http://e/i.png)\n> - ![j](http://e/j.png)",
        "[a][r]\n\n[r]: http://e/p \"t\"",
        "<div>\n\n![i](http://e/i.png)\n\n</div>",
        "~~~\n```\n![i](http://e/i.png)\n~~~\n![j](http://e/j.png)",
        "[a](<http://e/p>) [b](<https://ok.example/p>)",
        "<!-- ![i](http://e/i.png) --> ![j](http://e/j.png)",
        "[![i](http://e/i.png)](https://ok.example)",
    ];

    #[test]
    fn link_hosts_are_checked() {
        for bad in [
            "https:///evil",
            "https://.",
            "https://bank.com@evil.example/",
            "https://user:pw@host/",
            "https://x\\@evil",
            "https://good.example\\.evil.example/",
            "https://host:/x",
            "https://host:80a/",
            "https://a..b/",
            "https://exa mple.com/",
            "https://example.com/\u{a0}x",
            "https://example.com/\u{2028}x",
            "https://example.com/\u{202e}gpj.exe",
            "https://example.com/\u{2066}x",
            "https://example.com/\u{200b}x",
            "https://[::1]/",
            "https://exämple.com/",
        ] {
            assert!(!is_safe_link(bad), "{bad:?}");
        }
        let long = format!("https://example.com/{}\u{7f}", "a".repeat(5000));
        assert!(!is_safe_link(&long));
        let long_ok = format!("https://example.com/{}", "a".repeat(5000));
        assert!(!is_safe_link(&long_ok), "over 4096 characters");
        for good in [
            "https://example.com",
            "https://sub.example.com:8443/a/b?c=d#e",
            "HTTPS://Example.COM/x@y",
            "https://example.com/a\\b",
            "https://1.2.3.4/",
        ] {
            assert!(is_safe_link(good), "{good:?}");
        }
    }

    #[test]
    fn a_link_text_that_names_another_site_shows_the_real_host() {
        let o = render("[https://bank.com/login](https://evil.example/x)");
        assert!(o.contains("</a> (evil.example)"), "{o}");
        let o = render("[www.bank.com](https://evil.example/)");
        assert!(o.contains("(evil.example)"), "{o}");
        // the same site, or ordinary words: nothing added
        for md in [
            "[https://example.com/docs](https://example.com/other)",
            "[example.com](https://EXAMPLE.com/x)",
            "[the docs](https://example.com/)",
            "[v1.2](https://example.com/)",
            "[a b.com c](https://example.com/)",
        ] {
            assert!(!render(md).contains("</a> ("), "{md}");
        }
        // evasions: a trailing dot and hidden characters in the text
        for md in [
            "[bank.com.](https://evil.example/)",
            "[bank\u{200b}.com](https://evil.example/)",
            "[bank\u{ad}.com](https://evil.example/)",
            "[b\u{202e}ank.com](https://evil.example/)",
        ] {
            assert!(render(md).contains("</a> (evil.example)"), "{md}");
        }
        // file and tool names are not sites; www. is not a different site
        for md in [
            "[Node.js](https://nodejs.org)",
            "[notes.txt](https://example.com/)",
            "[www.bank.com](https://bank.com/)",
            "[bank.com](https://www.bank.com/)",
        ] {
            assert!(!render(md).contains("</a> ("), "{md}");
        }
        // but a scheme or www. makes even a file-like tail a site name
        assert!(render("[www.evil.md](https://x.example/)").contains("(x.example)"));
        // real TLDs are never treated as file names
        for md in [
            "[microsoft.zip](https://evil.example)",
            "[github.sh](https://evil.example)",
            "[bank.md](https://evil.example)",
            "[README.md](https://evil.example)",
            "[bank.rs](https://evil.example)",
            "[oracle.java](https://evil.example)",
        ] {
            assert!(render(md).contains("</a> (evil.example)"), "{md}");
            assert!(render_plain(md).ends_with("(evil.example)"), "{md}");
        }
        // trailing dots, ideographic dots, and www.com
        for md in [
            "[bank.com..](https://evil.example/)",
            "[bank\u{3002}com](https://evil.example/)",
            "[bank\u{ff0e}com](https://evil.example/)",
            "[bank\u{ff61}com](https://evil.example/)",
        ] {
            assert!(render(md).contains("</a> (evil.example)"), "{md}");
        }
        assert!(!render("[bank.com](https://bank.com./)").contains("</a> ("));
        assert!(!render_plain("[bank.com](https://bank.com./)").contains('('));
        assert!(!render("[www.com](https://www.com/)").contains("</a> ("));
        assert!(render("[www.com](https://evil.example/)").contains("</a> (evil.example)"));
        // the plain text carries the same warning and drops hidden characters
        assert_eq!(
            render_plain("[bank\u{200b}.com](https://evil.example/)"),
            "bank.com (evil.example)"
        );
        // text of a link that is not a link stays plain, no host added
        assert_eq!(
            render("[bank.com](http://evil.example)").trim(),
            "<p>bank.com</p>"
        );
    }

    #[test]
    fn text_host_drops_trailing_dots() {
        assert_eq!(text_host("bank.com.").as_deref(), Some("bank.com"));
        assert_eq!(text_host("www.bank.com..").as_deref(), Some("bank.com"));
    }

    #[test]
    fn bypass_inputs_stay_safe() {
        for md in BYPASSES {
            assert_safe(&render(md), md);
        }
        // the specific ones: an image is its alt text, a bad link its text
        assert_eq!(
            render("![alt text](http://e/i.png)").trim(),
            "<p>alt text</p>"
        );
        assert_eq!(render("[a](http://e/p)").trim(), "<p>a</p>");
        assert!(render("[a](https://e/p?a=1&b=2)").contains("href=\"https://e/p?a=1&amp;b=2\""));
        // raw HTML is shown as text
        let o = render("<b onclick=x>hi</b>");
        assert!(
            o.contains("&lt;b onclick=x&gt;") && !o.contains("<b"),
            "{o}"
        );
    }

    #[test]
    fn plain_text_has_lines_and_no_markup() {
        let md = "# Title\n\nSome **bold** and [a link](https://e.example/x) with `code`.\nSoft wrapped.\n\n- one\n- two *x*\n\n1. first\n\n```\nlet v: Vec<String>;\n```\n\n![alt text](http://e/i.png) <b>raw</b>";
        assert_eq!(
            render_plain(md),
            "Title\nSome bold and a link with code. Soft wrapped.\n- one\n- two x\n- first\nlet v: Vec<String>;\nalt text <b>raw</b>"
        );
        assert_eq!(render_plain(""), "");
    }

    #[test]
    fn formatting_survives() {
        let o = render(
            "# T\n\n**b** *i* ~~s~~ `c`\n\n- one\n- two\n\n1. x\n\n> q\n\n---\n\n```\nlet v: Vec<String>;\n```\n\n[ok](https://e.example/a) a  \nb",
        );
        for want in [
            "<h3>T</h3>",
            "<strong>b</strong>",
            "<em>i</em>",
            "<s>s</s>",
            "<code>c</code>",
            "<ul>",
            "<li>one</li>",
            "<ol>",
            "<blockquote>",
            "<hr>",
            "<pre>let v: Vec&lt;String&gt;;\n</pre>",
            "<a href=\"https://e.example/a\">ok</a>",
            "<br>",
        ] {
            assert!(o.contains(want), "{want} not in {o}");
        }
    }

    #[test]
    fn random_mixes_stay_safe() {
        let frags: Vec<&str> = BYPASSES
            .iter()
            .copied()
            .chain([
                "`",
                "``",
                "```",
                "~~~",
                "\n",
                "\n\n",
                "\r",
                "  ",
                "- ",
                "> ",
                "1. ",
                "<",
                ">",
                "![",
                "](",
                ")",
                "[",
                "]",
                "]:",
                "(",
                "<!--",
                "-->",
                "<pre>",
                "<p>",
                "http://e/i.png",
                "https://ok.example",
                "\\",
                "&",
                "\"",
                "'",
                "<https://a>",
                "<b>",
                "*",
                "_",
                "#",
                "| a | b |\n|---|---|\n| ![i](http://e/i.png) | x |",
            ])
            .collect();
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..4000 {
            let n = 1 + (next() % 12) as usize;
            let md: String = (0..n)
                .map(|_| frags[(next() % frags.len() as u64) as usize])
                .collect();
            assert_safe(&render(&md), &md);
        }
    }

    #[test]
    fn hostile_input_is_fast_and_does_not_panic() {
        let t = std::time::Instant::now();
        for md in [
            "[".repeat(200_000),
            "> ".repeat(100_000),
            "- ".repeat(100_000),
            "`a".repeat(100_000),
            "<a ".repeat(100_000),
            "![a](".repeat(50_000),
            "*a ".repeat(100_000),
        ] {
            assert_safe(&render(&md), "(large)");
        }
        assert!(
            t.elapsed() < std::time::Duration::from_secs(20),
            "{:?}",
            t.elapsed()
        );
    }

    #[test]
    fn body() {
        match parse_body("{\"tag_name\":\"44.1\",\"body\":\"## Changes\\n- a\\n\"}") {
            Notes::Found(h, p) => {
                assert!(
                    h.contains("<h4>Changes</h4>") && h.contains("<li>a</li>"),
                    "{h}"
                );
                assert_eq!(p, "Changes\n- a");
            }
            n => panic!("{n:?}"),
        }
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
        assert_eq!(
            fetch(&t, "44.2").unwrap(),
            Notes::Found("<p>hello</p>\n".into(), "hello".into())
        );
        assert_eq!(fetch(&t, "44.3").unwrap(), Notes::Missing);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
