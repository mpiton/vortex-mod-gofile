//! WASM-only entry points: `#[plugin_fn]` exports + `#[host_fn]` imports.
//!
//! Resolution flow used by `extract_links` and `resolve_stream_url`:
//!
//! ```text
//! url ─▶ classify ─▶ create guest token ─▶ getContent(folder, token)
//!         │                  │                      │
//!         ▼                  ▼                      ▼
//!      kind          token (one-shot,       folder + per-file
//!      (folder|file) cached per call)       direct CDN urls
//! ```
//!
//! The token is requested fresh on every call. Gofile guest tokens are
//! cheap to mint and the plugin host runs each WASM call in a sandbox
//! that drops state between invocations, so caching wouldn't survive
//! anyway.

use extism_pdk::*;

use crate::api_client::{
    build_content_request, build_create_account_request, parse_account_token, parse_folder_content,
    parse_http_response, GofileFolder,
};
use crate::error::PluginError;
use crate::url_matcher::{extract_ids, UrlKind};
use crate::{
    build_extract_links_response, ensure_known_url, handle_can_handle, handle_supports_playlist,
    pick_direct_url, pick_single_file_url,
};

#[host_fn]
extern "ExtismHost" {
    fn http_request(req: String) -> String;
}

#[plugin_fn]
pub fn can_handle(url: String) -> FnResult<String> {
    Ok(handle_can_handle(&url))
}

#[plugin_fn]
pub fn supports_playlist(url: String) -> FnResult<String> {
    Ok(handle_supports_playlist(&url))
}

#[plugin_fn]
pub fn extract_links(url: String) -> FnResult<String> {
    let kind = ensure_known_url(&url).map_err(error_to_fn_error)?;
    let (folder_id, file_id) =
        extract_ids(&url).ok_or_else(|| error_to_fn_error(PluginError::UnsupportedUrl(url)))?;
    let folder = fetch_folder(&folder_id)?;
    let target_file_id = match kind {
        UrlKind::File => file_id.as_deref(),
        _ => None,
    };
    let response =
        build_extract_links_response(folder, target_file_id).map_err(error_to_fn_error)?;
    Ok(serde_json::to_string(&response)?)
}

/// Resolve the direct CDN URL for a single Gofile file.
///
/// Input JSON: `{ "url": "..." }` — extra fields (`quality`, `format`,
/// `audio_only`) are accepted and ignored for API parity with crawler
/// plugins. The URL must be either:
/// - the synthesised per-file shape `https://gofile.io/d/<folder>/<file>`
///   (returned by `extract_links` for multi-file folders), or
/// - a folder URL `https://gofile.io/d/<folder>` whose folder happens to
///   carry exactly one file.
#[plugin_fn]
pub fn resolve_stream_url(input: String) -> FnResult<String> {
    #[derive(serde::Deserialize)]
    struct Input {
        url: String,
    }
    let params: Input =
        serde_json::from_str(&input).map_err(|e| error_to_fn_error(PluginError::SerdeJson(e)))?;
    let kind = ensure_known_url(&params.url).map_err(error_to_fn_error)?;
    let (folder_id, file_id) = extract_ids(&params.url)
        .ok_or_else(|| error_to_fn_error(PluginError::UnsupportedUrl(params.url.clone())))?;

    let folder = fetch_folder(&folder_id)?;
    let direct_url = match (kind, file_id) {
        (UrlKind::File, Some(id)) => pick_direct_url(&folder, &id).map_err(error_to_fn_error)?,
        (UrlKind::Folder, _) => pick_single_file_url(&folder).map_err(error_to_fn_error)?,
        _ => return Err(error_to_fn_error(PluginError::UnsupportedUrl(params.url))),
    };
    Ok(direct_url)
}

fn fetch_folder(folder_id: &str) -> FnResult<GofileFolder> {
    let token = fetch_guest_token()?;
    let req = build_content_request(folder_id, &token).map_err(error_to_fn_error)?;
    let raw = call_host_http(req)?;
    let resp = parse_http_response(&raw).map_err(error_to_fn_error)?;
    let body = resp.into_success_body().map_err(error_to_fn_error)?;
    parse_folder_content(&body).map_err(error_to_fn_error)
}

fn fetch_guest_token() -> FnResult<String> {
    let req = build_create_account_request().map_err(error_to_fn_error)?;
    let raw = call_host_http(req)?;
    let resp = parse_http_response(&raw).map_err(error_to_fn_error)?;
    let body = resp.into_success_body().map_err(error_to_fn_error)?;
    parse_account_token(&body).map_err(error_to_fn_error)
}

fn call_host_http(req: String) -> FnResult<String> {
    // SAFETY: `http_request` is resolved by the Vortex plugin host at load
    // time (see src-tauri/src/adapters/driven/plugin/host_functions.rs:
    // `make_http_request_function`). Invariants:
    //   1. The host registers `http_request` in the `ExtismHost` namespace
    //      before any `#[plugin_fn]` export is callable.
    //   2. The ABI is `(I64) -> I64`; the `#[host_fn]` macro marshals
    //      `String` in/out through Extism memory handles.
    //   3. The host gates the call on the `http` capability declared in
    //      `plugin.toml`; rejections return an error that `?` surfaces.
    //   4. Inputs/outputs are owned JSON strings — no aliasing concerns.
    Ok(unsafe { http_request(req)? })
}

fn error_to_fn_error(err: PluginError) -> WithReturnCode<extism_pdk::Error> {
    extism_pdk::Error::msg(err.to_string()).into()
}
