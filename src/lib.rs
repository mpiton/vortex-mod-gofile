//! Vortex Gofile WASM plugin.
//!
//! Implements the plugin contract used by the Vortex plugin host:
//! - `can_handle(url)` → `"true"` / `"false"`
//! - `supports_playlist(url)` → `"true"` for folder share URLs
//! - `extract_links(url)` → JSON metadata for every file inside the folder
//! - `resolve_stream_url(input)` → direct CDN URL for a single file
//!
//! Network access is delegated to the host via `http_request`. Resolution
//! takes two HTTP round-trips: `GET /createAccount` to obtain a guest
//! token, then `GET /getContent?contentId=<id>&token=<token>` to read
//! folder metadata + per-file CDN links. Pure JSON parsing
//! (`api_client.rs`) is exercised natively without WASM.

pub mod api_client;
pub mod error;
pub mod url_matcher;

#[cfg(target_family = "wasm")]
mod plugin_api;

use serde::Serialize;

use crate::api_client::{GofileFile, GofileFolder};
use crate::error::PluginError;
use crate::url_matcher::{synthesise_file_url, UrlKind};

// ── IPC DTOs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ExtractLinksResponse {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
    pub files: Vec<FileLink>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FileLink {
    pub id: String,
    pub url: String,
    pub filename: Option<String>,
    pub size_bytes: Option<u64>,
    pub mime_type: Option<String>,
    pub direct_url: String,
    pub resumable: bool,
}

// ── Routing helpers ──────────────────────────────────────────────────────────

pub fn handle_can_handle(url: &str) -> String {
    let kind = url_matcher::classify_url(url);
    bool_to_string(matches!(kind, UrlKind::Folder | UrlKind::File))
}

/// Folder share URLs are flagged as playlist-capable so the host knows it
/// may receive multiple `FileLink` results from `extract_links`. Per-file
/// synthetic URLs always resolve to a single file → `"false"`.
pub fn handle_supports_playlist(url: &str) -> String {
    bool_to_string(matches!(url_matcher::classify_url(url), UrlKind::Folder))
}

fn bool_to_string(b: bool) -> String {
    if b {
        "true".into()
    } else {
        "false".into()
    }
}

pub fn ensure_known_url(url: &str) -> Result<UrlKind, PluginError> {
    match url_matcher::classify_url(url) {
        UrlKind::Folder => Ok(UrlKind::Folder),
        UrlKind::File => Ok(UrlKind::File),
        UrlKind::Unknown => Err(PluginError::UnsupportedUrl(url.to_string())),
    }
}

// ── Response builders ────────────────────────────────────────────────────────

/// Build the `extract_links` response for a fully resolved Gofile folder.
///
/// `requested_file_id` is `Some` only when the input URL was the synthetic
/// per-file shape — in that case the response is filtered down to a single
/// `FileLink` so the host doesn't re-process unrelated siblings.
pub fn build_extract_links_response(
    folder: GofileFolder,
    requested_file_id: Option<&str>,
) -> Result<ExtractLinksResponse, PluginError> {
    let folder_id = folder.id.clone();
    let folder_name = folder.name.clone();

    let mut files: Vec<FileLink> = folder
        .files
        .into_iter()
        .map(|f| build_file_link(&folder_id, f))
        .collect();

    if let Some(target_id) = requested_file_id {
        files.retain(|f| f.id == target_id);
        if files.is_empty() {
            return Err(PluginError::FileNotInFolder(target_id.to_string()));
        }
    } else if files.is_empty() {
        return Err(PluginError::EmptyFolder);
    }

    Ok(ExtractLinksResponse {
        kind: if files.len() == 1 { "file" } else { "folder" },
        folder_id: Some(folder_id),
        folder_name,
        files,
    })
}

fn build_file_link(folder_id: &str, file: GofileFile) -> FileLink {
    FileLink {
        url: synthesise_file_url(folder_id, &file.id),
        id: file.id,
        filename: Some(file.name),
        size_bytes: Some(file.size),
        mime_type: file.mimetype,
        direct_url: file.direct_url,
        resumable: true,
    }
}

/// Pick the direct CDN URL for a specific file id inside an already
/// resolved folder. Surfaces [`PluginError::FileNotInFolder`] if the id
/// has disappeared between two calls (e.g. uploader removed it after the
/// link-check pass).
pub fn pick_direct_url(folder: &GofileFolder, file_id: &str) -> Result<String, PluginError> {
    folder
        .files
        .iter()
        .find(|f| f.id == file_id)
        .map(|f| f.direct_url.clone())
        .ok_or_else(|| PluginError::FileNotInFolder(file_id.to_string()))
}

/// When the input URL is a folder URL, pick a single CDN URL — only valid
/// when the folder has exactly one file. For multi-file folders the host
/// must call `extract_links` and dispatch each `FileLink.url` separately.
pub fn pick_single_file_url(folder: &GofileFolder) -> Result<String, PluginError> {
    match folder.files.len() {
        0 => Err(PluginError::EmptyFolder),
        1 => Ok(folder.files[0].direct_url.clone()),
        n => Err(PluginError::ApiError(format!(
            "folder has {n} files; resolve_stream_url needs the per-file URL shape \
             (https://gofile.io/d/<folder>/<file>) to disambiguate"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_folder() -> GofileFolder {
        GofileFolder {
            id: "folder1".into(),
            name: Some("Holiday".into()),
            files: vec![
                GofileFile {
                    id: "f1".into(),
                    name: "a.zip".into(),
                    size: 42,
                    direct_url: "https://store-eu1.gofile.io/download/f1/a.zip".into(),
                    mimetype: Some("application/zip".into()),
                },
                GofileFile {
                    id: "f2".into(),
                    name: "b.txt".into(),
                    size: 7,
                    direct_url: "https://store-eu1.gofile.io/download/f2/b.txt".into(),
                    mimetype: Some("text/plain".into()),
                },
            ],
        }
    }

    fn single_file_folder() -> GofileFolder {
        GofileFolder {
            id: "folderS".into(),
            name: None,
            files: vec![GofileFile {
                id: "only".into(),
                name: "only.bin".into(),
                size: 1024,
                direct_url: "https://store-eu1.gofile.io/download/only/only.bin".into(),
                mimetype: None,
            }],
        }
    }

    // ── Routing ─────────────────────────────────────────────────────────────

    #[test]
    fn can_handle_recognises_folder_url() {
        assert_eq!(handle_can_handle("https://gofile.io/d/abc123"), "true");
    }

    #[test]
    fn can_handle_recognises_synthesised_file_url() {
        assert_eq!(
            handle_can_handle("https://gofile.io/d/abc123/file42"),
            "true"
        );
    }

    #[test]
    fn can_handle_rejects_unrelated() {
        assert_eq!(handle_can_handle("https://example.com/d/abc123"), "false");
    }

    #[test]
    fn supports_playlist_true_for_folder_only() {
        assert_eq!(
            handle_supports_playlist("https://gofile.io/d/abc123"),
            "true"
        );
        assert_eq!(
            handle_supports_playlist("https://gofile.io/d/abc123/file42"),
            "false"
        );
    }

    #[test]
    fn ensure_known_url_classifies_folder_and_file() {
        assert_eq!(
            ensure_known_url("https://gofile.io/d/abc123").unwrap(),
            UrlKind::Folder
        );
        assert_eq!(
            ensure_known_url("https://gofile.io/d/abc123/file42").unwrap(),
            UrlKind::File
        );
    }

    #[test]
    fn ensure_known_url_rejects_unrelated() {
        assert!(matches!(
            ensure_known_url("https://example.com/d/abc123"),
            Err(PluginError::UnsupportedUrl(_))
        ));
    }

    // ── Response builder (folder shape) ─────────────────────────────────────

    #[test]
    fn build_extract_links_response_returns_all_files_for_folder_url() {
        let r = build_extract_links_response(sample_folder(), None).unwrap();
        assert_eq!(r.kind, "folder");
        assert_eq!(r.folder_id.as_deref(), Some("folder1"));
        assert_eq!(r.folder_name.as_deref(), Some("Holiday"));
        assert_eq!(r.files.len(), 2);

        let f1 = &r.files[0];
        assert_eq!(f1.id, "f1");
        assert_eq!(f1.url, "https://gofile.io/d/folder1/f1");
        assert_eq!(f1.filename.as_deref(), Some("a.zip"));
        assert_eq!(f1.size_bytes, Some(42));
        assert_eq!(f1.mime_type.as_deref(), Some("application/zip"));
        assert_eq!(
            f1.direct_url,
            "https://store-eu1.gofile.io/download/f1/a.zip"
        );
        assert!(f1.resumable);
    }

    #[test]
    fn build_extract_links_response_filters_to_single_file_when_id_given() {
        let r = build_extract_links_response(sample_folder(), Some("f2")).unwrap();
        assert_eq!(r.kind, "file", "single-result responses must use kind=file");
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].id, "f2");
    }

    #[test]
    fn build_extract_links_response_uses_kind_file_for_single_file_folder() {
        let r = build_extract_links_response(single_file_folder(), None).unwrap();
        assert_eq!(r.kind, "file");
        assert_eq!(r.files.len(), 1);
    }

    #[test]
    fn build_extract_links_response_errors_on_unknown_file_id() {
        assert!(matches!(
            build_extract_links_response(sample_folder(), Some("missing")),
            Err(PluginError::FileNotInFolder(_))
        ));
    }

    #[test]
    fn build_extract_links_response_errors_on_empty_folder() {
        let empty = GofileFolder {
            id: "e".into(),
            name: None,
            files: vec![],
        };
        assert!(matches!(
            build_extract_links_response(empty, None),
            Err(PluginError::EmptyFolder)
        ));
    }

    #[test]
    fn extract_links_response_serialises_kind_and_files() {
        let r = build_extract_links_response(sample_folder(), None).unwrap();
        let json = serde_json::to_string(&r).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["kind"], "folder");
        assert_eq!(parsed["files"][0]["resumable"], true);
        assert_eq!(parsed["files"][0]["url"], "https://gofile.io/d/folder1/f1");
    }

    // ── pick_* helpers ──────────────────────────────────────────────────────

    #[test]
    fn pick_direct_url_returns_matching_file() {
        assert_eq!(
            pick_direct_url(&sample_folder(), "f2").unwrap(),
            "https://store-eu1.gofile.io/download/f2/b.txt"
        );
    }

    #[test]
    fn pick_direct_url_errors_on_unknown_id() {
        assert!(matches!(
            pick_direct_url(&sample_folder(), "ghost"),
            Err(PluginError::FileNotInFolder(_))
        ));
    }

    #[test]
    fn pick_single_file_url_works_for_one_file_folder() {
        assert_eq!(
            pick_single_file_url(&single_file_folder()).unwrap(),
            "https://store-eu1.gofile.io/download/only/only.bin"
        );
    }

    #[test]
    fn pick_single_file_url_errors_for_multi_file_folder() {
        assert!(matches!(
            pick_single_file_url(&sample_folder()),
            Err(PluginError::ApiError(_))
        ));
    }

    #[test]
    fn pick_single_file_url_errors_for_empty_folder() {
        let empty = GofileFolder {
            id: "e".into(),
            name: None,
            files: vec![],
        };
        assert!(matches!(
            pick_single_file_url(&empty),
            Err(PluginError::EmptyFolder)
        ));
    }
}
