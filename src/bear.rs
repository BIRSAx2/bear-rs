use std::fmt::Write as _;
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn open_bear_action(action: &str, params: &[(String, String)]) -> Result<()> {
    let url = build_bear_url(action, params);
    let status = Command::new("open")
        .arg(&url)
        .status()
        .with_context(|| format!("failed to launch Bear URL for action {action}"))?;

    if !status.success() {
        bail!("Bear URL action failed: {action}");
    }

    Ok(())
}

pub fn build_bear_url(action: &str, params: &[(String, String)]) -> String {
    let mut url = format!("bear://x-callback-url/{}", percent_encode(action));
    if !params.is_empty() {
        url.push('?');
        for (index, (key, value)) in params.iter().enumerate() {
            if index > 0 {
                url.push('&');
            }
            let _ = write!(url, "{}={}", percent_encode(key), percent_encode(value));
        }
    }
    url
}

pub fn maybe_push(query: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        query.push((key.to_string(), value));
    }
}

pub fn maybe_push_bool(query: &mut Vec<(String, String)>, key: &str, enabled: bool) {
    if enabled {
        query.push((key.to_string(), "yes".to_string()));
    }
}

pub fn join_tags(tags: &[String]) -> Option<String> {
    if tags.is_empty() {
        None
    } else {
        Some(tags.join(","))
    }
}

fn percent_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte))
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{build_bear_url, join_tags};

    #[test]
    fn encodes_bear_urls() {
        let url = build_bear_url(
            "add-text",
            &[
                ("id".into(), "ABC 123".into()),
                ("text".into(), "hello/world".into()),
            ],
        );
        assert_eq!(
            url,
            "bear://x-callback-url/add-text?id=ABC%20123&text=hello%2Fworld"
        );
    }

    #[test]
    fn joins_tags() {
        assert_eq!(join_tags(&["a".into(), "b".into()]), Some("a,b".into()));
        assert_eq!(join_tags(&[]), None);
    }
}
