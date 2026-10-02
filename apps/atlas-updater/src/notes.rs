//! Release notes: GitHub release JSON (`.body`) from https, http or file://.

use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Notes {
    /// An HTML fragment from [`render`].
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

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
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
    // Open links: whether each one was written as <a>.
    let mut links: Vec<bool> = Vec::new();
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
                Tag::Link { dest_url, .. } => {
                    let ok = is_safe_link(&dest_url) && stack.len() < MAX_DEPTH;
                    links.push(ok);
                    if ok {
                        out.push_str("<a href=\"");
                        escape(dest_url.trim(), &mut out);
                        out.push_str("\">");
                    }
                }
                // an image is its alt text (the events inside); raw HTML
                // blocks get no wrapper, their text is escaped below
                _ => stack.push(""),
            },
            Event::End(end) => match end {
                TagEnd::Link => {
                    if links.pop() == Some(true) {
                        out.push_str("</a>");
                    }
                }
                _ => {
                    if let Some(c) = stack.pop() {
                        out.push_str(c);
                    }
                }
            },
            Event::Code(t) => {
                out.push_str("<code>");
                escape(&t, &mut out);
                out.push_str("</code>");
            }
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => escape(&t, &mut out),
            Event::SoftBreak => out.push('\n'),
            Event::HardBreak => out.push_str("<br>\n"),
            Event::Rule => out.push_str("<hr>\n"),
            Event::FootnoteReference(l) => escape(&l, &mut out),
            _ => {}
        }
    }
    out
}

/// Pulls `.body` out of a GitHub release JSON document.
pub fn parse_body(json: &str) -> Notes {
    let body = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("body").and_then(|b| b.as_str()).map(str::to_string))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match body.map(|b| render(&b)).filter(|h| !h.trim().is_empty()) {
        Some(h) => Notes::Found(h),
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
            Notes::Found(h) => assert!(
                h.contains("<h4>Changes</h4>") && h.contains("<li>a</li>"),
                "{h}"
            ),
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
            Notes::Found("<p>hello</p>\n".into())
        );
        assert_eq!(fetch(&t, "44.3").unwrap(), Notes::Missing);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
