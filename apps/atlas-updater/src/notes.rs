//! Release notes: GitHub release JSON (`.body`) from https, http or file://.

use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Notes {
    Found(String),
    /// No release for this version (404, missing file, empty body).
    Missing,
}

pub fn url_for(template: &str, version: &str) -> String {
    template.replace("{version}", version)
}

/// Pulls `.body` out of a GitHub release JSON document.
pub fn parse_body(json: &str) -> Notes {
    let body = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("body").and_then(|b| b.as_str()).map(str::to_string))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match body {
        Some(b) => Notes::Found(b),
        None => Notes::Missing,
    }
}

/// Blocking. Call from a worker thread.
pub fn fetch(template: &str, version: &str) -> Result<Notes, String> {
    let url = url_for(template, version);
    if let Some(path) = url.strip_prefix("file://") {
        return match std::fs::read_to_string(path) {
            Ok(text) => Ok(parse_body(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Notes::Missing),
            Err(e) => Err(e.to_string()),
        };
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .user_agent(concat!("atlas-updater/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut resp = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    if status == 404 || status == 410 {
        return Ok(Notes::Missing);
    }
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}"));
    }
    let text = resp
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| e.to_string())?;
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
