//! Gofile URL detection and parsing.
//!
//! Gofile exposes a single user-facing shape: `https://gofile.io/d/<folder>`
//! where `<folder>` is a 6+ alphanumeric content id. The folder may carry
//! one or many files; the plugin synthesises a per-file URL of the shape
//! `https://gofile.io/d/<folder>/<file>` so that the host's
//! `resolve_stream_url` contract — "url string in, single CDN url out" —
//! can disambiguate which file inside the folder to resolve.
//!
//! Allowed hosts: `gofile.io`, `www.gofile.io`. Anything else falls through
//! to [`UrlKind::Unknown`].

use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UrlKind {
    /// Folder share page: `gofile.io/d/<folder>` — may contain N files.
    Folder,
    /// Synthesised per-file shape: `gofile.io/d/<folder>/<file>`.
    File,
    /// Anything else.
    Unknown,
}

pub fn classify_url(url: &str) -> UrlKind {
    let Some(path) = gofile_path(url) else {
        return UrlKind::Unknown;
    };
    if file_regex().is_match(path) {
        UrlKind::File
    } else if folder_regex().is_match(path) {
        UrlKind::Folder
    } else {
        UrlKind::Unknown
    }
}

/// Extract `(folder_id, file_id?)` from a recognised gofile URL.
///
/// `file_id` is `Some` only for the synthesised per-file shape produced by
/// `extract_links` for multi-file folders; the bare `gofile.io/d/<folder>`
/// share URL returns `None` for `file_id`.
pub fn extract_ids(url: &str) -> Option<(String, Option<String>)> {
    let path = gofile_path(url)?;
    if let Some(caps) = file_regex().captures(path) {
        return Some((caps[1].to_string(), Some(caps[2].to_string())));
    }
    if let Some(caps) = folder_regex().captures(path) {
        return Some((caps[1].to_string(), None));
    }
    None
}

fn gofile_path(url: &str) -> Option<&str> {
    let (host, path) = validate_and_split(url)?;
    if !is_gofile_host(host) {
        return None;
    }
    Some(normalize_path(path))
}

fn is_gofile_host(host: &str) -> bool {
    ["gofile.io", "www.gofile.io"]
        .iter()
        .any(|h| host.eq_ignore_ascii_case(h))
}

fn normalize_path(path: &str) -> &str {
    let no_frag = path.split('#').next().unwrap_or("");
    let no_query = no_frag.split('?').next().unwrap_or("");
    no_query.trim_end_matches('/')
}

fn folder_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^/d/([A-Za-z0-9]{6,})$")
            .expect("folder_regex: compile-time constant regex must compile")
    })
}

fn file_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^/d/([A-Za-z0-9]{6,})/([A-Za-z0-9_-]{6,})$")
            .expect("file_regex: compile-time constant regex must compile")
    })
}

fn validate_and_split(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let (authority, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    let authority_no_user = authority.rsplit('@').next().unwrap_or(authority);
    let host = extract_host(authority_no_user)?;
    if host.is_empty() {
        return None;
    }
    Some((host, path_and_query))
}

/// Extract the host portion (without port) from an authority string.
fn extract_host(authority: &str) -> Option<&str> {
    if authority.is_empty() {
        return None;
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let close = rest.find(']')?;
        return Some(&authority[..=close + 1]);
    }
    let host = authority.split(':').next().unwrap_or(authority);
    (!host.is_empty()).then_some(host)
}

/// Build the synthetic per-file URL exposed in `extract_links` results.
pub fn synthesise_file_url(folder_id: &str, file_id: &str) -> String {
    format!("https://gofile.io/d/{folder_id}/{file_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("https://gofile.io/d/abc123", UrlKind::Folder)]
    #[case("https://www.gofile.io/d/abc123def", UrlKind::Folder)]
    #[case("http://gofile.io/d/abc123", UrlKind::Folder)]
    #[case("https://gofile.io/d/Abc123XYZ/", UrlKind::Folder)]
    #[case(
        "https://gofile.io/d/abc123/xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
        UrlKind::File
    )]
    #[case("https://gofile.io/d/abc123/file_id_42", UrlKind::File)]
    #[case("https://gofile.io/", UrlKind::Unknown)]
    #[case("https://gofile.io/api/getContent", UrlKind::Unknown)]
    #[case("https://example.com/d/abc123", UrlKind::Unknown)]
    #[case("ftp://gofile.io/d/abc123", UrlKind::Unknown)]
    #[case("not a url", UrlKind::Unknown)]
    fn classify_url_recognises_shapes(#[case] url: &str, #[case] expected: UrlKind) {
        assert_eq!(classify_url(url), expected);
    }

    #[test]
    fn classify_handles_query_and_fragment() {
        assert_eq!(
            classify_url("https://gofile.io/d/abc123?foo=bar#x"),
            UrlKind::Folder
        );
    }

    #[test]
    fn classify_rejects_short_folder_id() {
        assert_eq!(
            classify_url("https://gofile.io/d/abc"),
            UrlKind::Unknown,
            "Gofile content ids are 6+ alphanumerics; shorter shapes must not match"
        );
    }

    #[test]
    fn classify_rejects_special_chars_in_folder_id() {
        assert_eq!(
            classify_url("https://gofile.io/d/abc-123"),
            UrlKind::Unknown
        );
    }

    #[test]
    fn extract_ids_from_folder_returns_none_for_file() {
        let (folder, file) = extract_ids("https://gofile.io/d/abc123def").unwrap();
        assert_eq!(folder, "abc123def");
        assert!(file.is_none());
    }

    #[test]
    fn extract_ids_from_synthesised_file_returns_both() {
        let (folder, file) = extract_ids("https://gofile.io/d/abc123/uuid_42").unwrap();
        assert_eq!(folder, "abc123");
        assert_eq!(file.as_deref(), Some("uuid_42"));
    }

    #[test]
    fn extract_ids_strips_trailing_slash() {
        let (folder, file) = extract_ids("https://gofile.io/d/abc123/").unwrap();
        assert_eq!(folder, "abc123");
        assert!(file.is_none());
    }

    #[test]
    fn extract_ids_handles_query() {
        let (folder, file) = extract_ids("https://gofile.io/d/abc123?download").unwrap();
        assert_eq!(folder, "abc123");
        assert!(file.is_none());
    }

    #[test]
    fn extract_ids_other_host_returns_none() {
        assert!(extract_ids("https://example.com/d/abc123").is_none());
    }

    #[test]
    fn synthesise_file_url_round_trips() {
        let url = synthesise_file_url("folder42", "file_uuid");
        let (folder, file) = extract_ids(&url).unwrap();
        assert_eq!(folder, "folder42");
        assert_eq!(file.as_deref(), Some("file_uuid"));
    }
}
