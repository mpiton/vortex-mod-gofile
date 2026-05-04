//! Smoke test: load the compiled `.wasm` via Extism and call the pure
//! routing exports.
//!
//! `extract_links` and `resolve_stream_url` rely on two sequential
//! `http_request` calls (createAccount → getContent). The host stub
//! below dispatches on the request URL so a single load can cover the
//! happy path: it returns a valid token first, then a folder payload
//! with one file.
//!
//! Skipped unless the WASM artifact is present at
//! `target/wasm32-wasip1/release/vortex_mod_gofile.wasm`. To produce it:
//!
//! ```bash
//! cargo build --target wasm32-wasip1 --release
//! ```

use std::path::PathBuf;

use extism::{Function, UserData, Val, PTR};

const WASM_REL_PATH: &str = "target/wasm32-wasip1/release/vortex_mod_gofile.wasm";

fn wasm_path() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(WASM_REL_PATH);
    p.exists().then_some(p)
}

/// JSON envelope the host wraps each `http_request` response in.
fn http_envelope(body_json: &str) -> String {
    // We need to embed `body_json` (already JSON) as a JSON string. Use
    // serde to re-encode so quotes / escapes are correct.
    let body = serde_json::Value::String(body_json.to_string());
    format!(r#"{{"status":200,"headers":{{}},"body":{body}}}"#)
}

const TOKEN_RESPONSE: &str = r#"{"status":"ok","data":{"id":"u","token":"tok42","tier":"guest"}}"#;
const FOLDER_RESPONSE: &str = r#"{"status":"ok","data":{"id":"abc123","type":"folder","name":"Demo","contents":{"file777":{"id":"file777","type":"file","name":"hello.bin","size":17,"link":"https://store-eu1.gofile.io/download/file777/hello.bin","mimetype":"application/octet-stream"}}}}"#;

fn stub_http_request() -> Function {
    Function::new(
        "http_request",
        [PTR],
        [PTR],
        UserData::<()>::default(),
        |plugin, inputs, outputs, _user_data: UserData<()>| {
            let req_handle = inputs[0]
                .i64()
                .ok_or_else(|| extism::Error::msg("expected i64 input"))?;
            let req_str: String = plugin.memory_get_val(&Val::I64(req_handle))?;
            let body = if req_str.contains("/createAccount") {
                http_envelope(TOKEN_RESPONSE)
            } else if req_str.contains("/getContent") {
                http_envelope(FOLDER_RESPONSE)
            } else {
                http_envelope(r#"{"status":"error-notFound"}"#)
            };
            let handle = plugin.memory_new(&body)?;
            outputs[0] = Val::I64(handle.offset() as i64);
            Ok(())
        },
    )
}

fn load_plugin(path: &PathBuf) -> extism::Plugin {
    let manifest = extism::Manifest::new([extism::Wasm::file(path)]);
    extism::Plugin::new(&manifest, [stub_http_request()], true).expect("load wasm")
}

/// Resolve the WASM artefact path or skip the calling test with a build hint.
macro_rules! require_wasm {
    () => {
        match wasm_path() {
            Some(p) => p,
            None => {
                eprintln!(
                    "skipping: build with `cargo build --target wasm32-wasip1 --release` first"
                );
                return;
            }
        }
    };
}

#[test]
fn wasm_can_handle_recognises_folder_url() {
    let path = require_wasm!();
    let mut plugin = load_plugin(&path);
    let result: String = plugin
        .call("can_handle", "https://gofile.io/d/abc123def")
        .expect("can_handle call");
    assert_eq!(result.trim(), "true");
}

#[test]
fn wasm_can_handle_recognises_synthesised_file_url() {
    let path = require_wasm!();
    let mut plugin = load_plugin(&path);
    let result: String = plugin
        .call("can_handle", "https://gofile.io/d/abc123/file777")
        .expect("can_handle call");
    assert_eq!(result.trim(), "true");
}

#[test]
fn wasm_can_handle_rejects_unrelated_url() {
    let path = require_wasm!();
    let mut plugin = load_plugin(&path);
    let result: String = plugin
        .call("can_handle", "https://example.com/some/page")
        .expect("can_handle call");
    assert_eq!(result.trim(), "false");
}

#[test]
fn wasm_supports_playlist_true_for_folder() {
    let path = require_wasm!();
    let mut plugin = load_plugin(&path);
    let result: String = plugin
        .call("supports_playlist", "https://gofile.io/d/abc123def")
        .expect("supports_playlist call");
    assert_eq!(result.trim(), "true");
}

#[test]
fn wasm_supports_playlist_false_for_file_shape() {
    let path = require_wasm!();
    let mut plugin = load_plugin(&path);
    let result: String = plugin
        .call("supports_playlist", "https://gofile.io/d/abc123/file777")
        .expect("supports_playlist call");
    assert_eq!(result.trim(), "false");
}

#[test]
fn wasm_extract_links_returns_folder_metadata() {
    let path = require_wasm!();
    let mut plugin = load_plugin(&path);
    let result: String = plugin
        .call("extract_links", "https://gofile.io/d/abc123")
        .expect("extract_links call");
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("response is JSON");
    assert_eq!(parsed["kind"], "file"); // single-file folder collapses to file
    assert_eq!(parsed["folder_id"], "abc123");
    assert_eq!(parsed["files"][0]["filename"], "hello.bin");
    assert_eq!(parsed["files"][0]["size_bytes"], 17);
    assert_eq!(
        parsed["files"][0]["url"],
        "https://gofile.io/d/abc123/file777"
    );
    assert_eq!(
        parsed["files"][0]["direct_url"],
        "https://store-eu1.gofile.io/download/file777/hello.bin"
    );
    assert_eq!(parsed["files"][0]["resumable"], true);
}

#[test]
fn wasm_resolve_stream_url_returns_direct_cdn_url() {
    let path = require_wasm!();
    let mut plugin = load_plugin(&path);
    let input = r#"{"url":"https://gofile.io/d/abc123/file777"}"#;
    let result: String = plugin
        .call("resolve_stream_url", input)
        .expect("resolve_stream_url call");
    assert_eq!(
        result.trim(),
        "https://store-eu1.gofile.io/download/file777/hello.bin"
    );
}
