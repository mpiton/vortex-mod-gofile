//! Gofile JSON API wrapper.
//!
//! Two endpoints are exercised, in this order:
//! 1. `GET https://api.gofile.io/createAccount` — returns a guest token
//!    used to authenticate the subsequent content lookup.
//! 2. `GET https://api.gofile.io/getContent?contentId={id}&token={token}` —
//!    returns folder metadata + an indexed map of children. Each child of
//!    `type == "file"` carries a `link` field with the direct CDN URL.
//!
//! All HTTP I/O is delegated to the host via `http_request`. Pure parsing
//! lives here so it can be exercised natively without WASM or the host.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::PluginError;

const USER_AGENT: &str = "Mozilla/5.0 (Vortex/1.0; +https://vortex-app.com) GofilePlugin/1.0";
const API_BASE: &str = "https://api.gofile.io";

// ── HTTP envelope ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: String,
}

/// Reject responses larger than this so a malicious server can't make
/// every JSON parse a multi-megabyte buffer. Real Gofile payloads for
/// reasonably-sized folders stay under 64 KB; 1 MB is a generous ceiling
/// for big folders without leaving the door open to memory abuse.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

impl HttpResponse {
    pub fn into_success_body(self) -> Result<String, PluginError> {
        if (200..300).contains(&self.status) {
            if self.body.len() > MAX_BODY_BYTES {
                return Err(PluginError::HttpStatus {
                    status: self.status,
                    message: format!("body exceeds {MAX_BODY_BYTES} bytes"),
                });
            }
            Ok(self.body)
        } else if self.status == 404 || self.status == 410 {
            Err(PluginError::Offline(format!("status {}", self.status)))
        } else {
            Err(PluginError::HttpStatus {
                status: self.status,
                message: truncate(&self.body, 256),
            })
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut cut = max;
        while !s.is_char_boundary(cut) && cut > 0 {
            cut -= 1;
        }
        format!("{}…", &s[..cut])
    }
}

pub fn parse_http_response(raw: &str) -> Result<HttpResponse, PluginError> {
    serde_json::from_str(raw).map_err(|e| PluginError::HostResponse(e.to_string()))
}

// ── Request builders ─────────────────────────────────────────────────────────

pub fn build_create_account_request() -> Result<String, PluginError> {
    serialise_get(&format!("{API_BASE}/createAccount"))
}

pub fn build_content_request(content_id: &str, token: &str) -> Result<String, PluginError> {
    let url = format!(
        "{API_BASE}/getContent?contentId={}&token={}",
        url_encode(content_id),
        url_encode(token),
    );
    serialise_get(&url)
}

fn serialise_get(url: &str) -> Result<String, PluginError> {
    let mut headers = HashMap::new();
    headers.insert("User-Agent".to_string(), USER_AGENT.to_string());
    headers.insert("Accept".to_string(), "application/json".to_string());
    let req = HttpRequest {
        method: "GET".into(),
        url: url.to_string(),
        headers,
        body: None,
    };
    serde_json::to_string(&req).map_err(PluginError::SerdeJson)
}

/// Minimal percent-encoder for the characters that actually appear in
/// Gofile content ids and guest tokens. The API only emits `[A-Za-z0-9_-]`
/// for those, so we percent-encode everything outside that set defensively
/// to keep the query string well-formed if a future token ever carries a
/// `+` or `/`.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// ── Account creation ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct StatusEnvelope<T> {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    data: Option<T>,
}

#[derive(Debug, Default, Deserialize)]
struct AccountData {
    #[serde(default)]
    token: Option<String>,
}

/// Parse the body of `GET /createAccount` and return the guest token.
pub fn parse_account_token(body: &str) -> Result<String, PluginError> {
    let envelope: StatusEnvelope<AccountData> = serde_json::from_str(body)?;
    classify_status(envelope.status.as_deref())?;
    let data = envelope
        .data
        .ok_or_else(|| PluginError::ApiError("createAccount: missing data field".into()))?;
    let token = data.token.unwrap_or_default();
    if token.is_empty() {
        return Err(PluginError::ApiError(
            "createAccount: empty or missing token".into(),
        ));
    }
    Ok(token)
}

// ── Content (folder) lookup ──────────────────────────────────────────────────

/// A single resolved file inside a Gofile folder.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GofileFile {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub direct_url: String,
    pub mimetype: Option<String>,
}

/// Resolved content of a `getContent` call: folder metadata + flat file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GofileFolder {
    pub id: String,
    pub name: Option<String>,
    pub files: Vec<GofileFile>,
}

#[derive(Debug, Default, Deserialize)]
struct ContentRoot {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    contents: HashMap<String, ContentChild>,
}

#[derive(Debug, Deserialize)]
struct ContentChild {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    mimetype: Option<String>,
}

/// Parse the body of `GET /getContent?...` into a [`GofileFolder`].
///
/// Only direct file children of the root folder are returned. Nested
/// sub-folders are intentionally ignored — the plugin host treats each
/// folder URL as a flat list, and recursion is the responsibility of a
/// higher-level crawler if it ever lands.
pub fn parse_folder_content(body: &str) -> Result<GofileFolder, PluginError> {
    let envelope: StatusEnvelope<ContentRoot> = serde_json::from_str(body)?;
    classify_status(envelope.status.as_deref())?;
    let root = envelope
        .data
        .ok_or_else(|| PluginError::ApiError("getContent: missing data field".into()))?;
    let folder_id = root
        .id
        .clone()
        .ok_or_else(|| PluginError::ApiError("getContent: missing id".into()))?;
    if let Some(kind) = root.kind.as_deref() {
        if kind != "folder" {
            return Err(PluginError::ApiError(format!(
                "getContent: expected folder, got '{kind}'"
            )));
        }
    }
    let mut files = collect_files(&root.contents);
    // Stable ordering: gofile returns children as a JSON object (no order
    // guarantee), so sort by id to keep ExtractLinksResponse deterministic
    // across runs.
    files.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(GofileFolder {
        id: folder_id,
        name: root.name,
        files,
    })
}

fn collect_files(contents: &HashMap<String, ContentChild>) -> Vec<GofileFile> {
    contents
        .values()
        .filter(|c| matches!(c.kind.as_deref(), Some("file")))
        .filter_map(|c| {
            let id = c.id.clone()?;
            let name = c.name.clone()?;
            let size = c.size.unwrap_or(0);
            let direct_url = c.link.clone()?;
            if direct_url.is_empty() {
                return None;
            }
            Some(GofileFile {
                id,
                name,
                size,
                direct_url,
                mimetype: c.mimetype.clone(),
            })
        })
        .collect()
}

/// Map Gofile's status string onto the plugin error vocabulary.
///
/// Recognised shapes:
/// - `"ok"` → success (returns `Ok`)
/// - `"error-notFound"` / `"error-not-found"` → [`PluginError::Offline`]
/// - any other `"error-..."` → [`PluginError::ApiError`]
/// - missing / empty → [`PluginError::ApiError`]
fn classify_status(status: Option<&str>) -> Result<(), PluginError> {
    match status {
        Some("ok") => Ok(()),
        Some(s) if is_not_found(s) => Err(PluginError::Offline(s.to_string())),
        Some(s) if s.starts_with("error-") => Err(PluginError::ApiError(s.to_string())),
        Some(other) => Err(PluginError::ApiError(format!(
            "unexpected status '{other}'"
        ))),
        None => Err(PluginError::ApiError(
            "response missing status field".into(),
        )),
    }
}

fn is_not_found(status: &str) -> bool {
    matches!(
        status,
        "error-notFound"
            | "error-not-found"
            | "error-noContent"
            | "error-notPublic"
            | "error-passwordRequired"
            | "error-passwordWrong"
    ) || status.contains("notFound")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── HTTP envelope ───────────────────────────────────────────────────────

    #[test]
    fn parse_http_response_round_trips_success() {
        let raw = r#"{"status":200,"headers":{},"body":"{}"}"#;
        let resp = parse_http_response(raw).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, "{}");
    }

    #[test]
    fn into_success_body_passes_2xx() {
        let resp = HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: "{}".into(),
        };
        assert_eq!(resp.into_success_body().unwrap(), "{}");
    }

    #[test]
    fn into_success_body_maps_404_to_offline() {
        let resp = HttpResponse {
            status: 404,
            headers: HashMap::new(),
            body: String::new(),
        };
        assert!(matches!(
            resp.into_success_body().unwrap_err(),
            PluginError::Offline(_)
        ));
    }

    #[test]
    fn into_success_body_rejects_oversized_payload() {
        let resp = HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: "x".repeat(MAX_BODY_BYTES + 1),
        };
        assert!(matches!(
            resp.into_success_body().unwrap_err(),
            PluginError::HttpStatus { status: 200, .. }
        ));
    }

    #[test]
    fn into_success_body_maps_500_to_http_status() {
        let resp = HttpResponse {
            status: 500,
            headers: HashMap::new(),
            body: "boom".into(),
        };
        assert!(matches!(
            resp.into_success_body().unwrap_err(),
            PluginError::HttpStatus { status: 500, .. }
        ));
    }

    // ── Request builders ────────────────────────────────────────────────────

    #[test]
    fn build_create_account_request_targets_endpoint() {
        let json = build_create_account_request().unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["method"], "GET");
        assert_eq!(v["url"], "https://api.gofile.io/createAccount");
        assert_eq!(v["headers"]["Accept"], "application/json");
        assert!(v["headers"]["User-Agent"].is_string());
    }

    #[test]
    fn build_content_request_includes_id_and_token() {
        let json = build_content_request("abc123", "tok42").unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["method"], "GET");
        let url = v["url"].as_str().unwrap();
        assert!(url.starts_with("https://api.gofile.io/getContent?"));
        assert!(url.contains("contentId=abc123"));
        assert!(url.contains("token=tok42"));
    }

    #[test]
    fn build_content_request_percent_encodes_special_chars() {
        let json = build_content_request("abc", "tok+/=").unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let url = v["url"].as_str().unwrap();
        assert!(url.contains("token=tok%2B%2F%3D"), "got: {url}");
    }

    // ── Account token parser ────────────────────────────────────────────────

    #[test]
    fn parse_account_token_extracts_token() {
        let body = r#"{"status":"ok","data":{"id":"u1","token":"abcDEF123","tier":"guest"}}"#;
        assert_eq!(parse_account_token(body).unwrap(), "abcDEF123");
    }

    #[test]
    fn parse_account_token_rejects_missing_data() {
        let body = r#"{"status":"ok"}"#;
        assert!(matches!(
            parse_account_token(body).unwrap_err(),
            PluginError::ApiError(_)
        ));
    }

    #[test]
    fn parse_account_token_rejects_empty_token() {
        let body = r#"{"status":"ok","data":{"token":""}}"#;
        assert!(matches!(
            parse_account_token(body).unwrap_err(),
            PluginError::ApiError(_)
        ));
    }

    #[test]
    fn parse_account_token_maps_error_status_to_api_error() {
        let body = r#"{"status":"error-rateLimit","data":null}"#;
        assert!(matches!(
            parse_account_token(body).unwrap_err(),
            PluginError::ApiError(_)
        ));
    }

    // ── Folder content parser ───────────────────────────────────────────────

    #[test]
    fn parse_folder_content_extracts_files_only() {
        let body = r#"{
            "status": "ok",
            "data": {
                "id": "folder1",
                "name": "Holiday",
                "type": "folder",
                "contents": {
                    "f1": {"id":"f1","name":"a.zip","type":"file","size":42,"link":"https://store-eu1.gofile.io/download/f1/a.zip","mimetype":"application/zip"},
                    "f2": {"id":"f2","name":"b.txt","type":"file","size":7,"link":"https://store-eu1.gofile.io/download/f2/b.txt","mimetype":"text/plain"},
                    "sub": {"id":"sub","name":"nested","type":"folder"}
                }
            }
        }"#;
        let folder = parse_folder_content(body).unwrap();
        assert_eq!(folder.id, "folder1");
        assert_eq!(folder.name.as_deref(), Some("Holiday"));
        assert_eq!(folder.files.len(), 2, "sub-folder must be filtered out");
        // Stable order by id
        assert_eq!(folder.files[0].id, "f1");
        assert_eq!(folder.files[1].id, "f2");
        assert_eq!(folder.files[0].size, 42);
        assert_eq!(
            folder.files[0].direct_url,
            "https://store-eu1.gofile.io/download/f1/a.zip"
        );
        assert_eq!(folder.files[1].mimetype.as_deref(), Some("text/plain"));
    }

    #[test]
    fn parse_folder_content_rejects_non_folder_root() {
        let body = r#"{"status":"ok","data":{"id":"x","type":"file","contents":{}}}"#;
        assert!(matches!(
            parse_folder_content(body).unwrap_err(),
            PluginError::ApiError(_)
        ));
    }

    #[test]
    fn parse_folder_content_maps_not_found_to_offline() {
        let body = r#"{"status":"error-notFound","data":null}"#;
        assert!(matches!(
            parse_folder_content(body).unwrap_err(),
            PluginError::Offline(_)
        ));
    }

    #[test]
    fn parse_folder_content_maps_password_required_to_offline() {
        // Treat password-protected folders as Offline so the host doesn't
        // get stuck retrying — the user has to provide the password through
        // a future config dialog before the link is usable.
        let body = r#"{"status":"error-passwordRequired","data":null}"#;
        assert!(matches!(
            parse_folder_content(body).unwrap_err(),
            PluginError::Offline(_)
        ));
    }

    #[test]
    fn parse_folder_content_skips_files_with_missing_link() {
        let body = r#"{
            "status":"ok",
            "data":{
                "id":"folder1",
                "type":"folder",
                "contents":{
                    "f1":{"id":"f1","name":"a.zip","type":"file","size":42,"link":""},
                    "f2":{"id":"f2","name":"b.txt","type":"file","size":7,"link":"https://x/f2"}
                }
            }
        }"#;
        let folder = parse_folder_content(body).unwrap();
        assert_eq!(folder.files.len(), 1);
        assert_eq!(folder.files[0].id, "f2");
    }

    #[test]
    fn parse_folder_content_rejects_garbage() {
        assert!(matches!(
            parse_folder_content("not json").unwrap_err(),
            PluginError::SerdeJson(_)
        ));
    }
}
