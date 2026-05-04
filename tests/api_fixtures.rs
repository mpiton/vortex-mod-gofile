//! Fixture-driven integration tests for the Gofile JSON API parsers.
//!
//! Each fixture in `tests/fixtures/*.json` mirrors a shape returned by
//! the real Gofile public API. The tests validate that:
//!
//! 1. The `createAccount` envelope yields the guest token.
//! 2. The `getContent` envelope decodes into a [`GofileFolder`] with file
//!    children flattened, sub-folders dropped, and unicode names preserved.
//! 3. Error envelopes (`status` starting with `error-...`) map onto the
//!    expected [`PluginError`] variant — `Offline` for not-found /
//!    password-required, `ApiError` for everything else.
//! 4. The folder→FileLink transformation produced by `lib::build_extract_links_response`
//!    matches the on-the-wire JSON the host expects.

use std::fs;
use std::path::Path;

use rstest::rstest;
use vortex_mod_gofile::api_client::{parse_account_token, parse_folder_content};
use vortex_mod_gofile::error::PluginError;
use vortex_mod_gofile::{build_extract_links_response, ExtractLinksResponse};

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn load_fixture(name: &str) -> String {
    let path = Path::new(FIXTURES_DIR).join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

// ── createAccount fixtures ────────────────────────────────────────────────────

#[test]
fn account_token_fixture_yields_guest_token() {
    let body = load_fixture("01_account_token.json");
    let token = parse_account_token(&body).expect("ok envelope must yield token");
    assert_eq!(token, "abcDEF123456");
}

#[test]
fn account_error_fixture_maps_to_api_error() {
    let body = load_fixture("02_account_error.json");
    let err = parse_account_token(&body).unwrap_err();
    assert!(
        matches!(err, PluginError::ApiError(_)),
        "createAccount error envelope must surface ApiError, got: {err:?}"
    );
}

// ── getContent fixtures: success shapes ───────────────────────────────────────

/// Bundle of expectations checked against a single folder fixture.
///
/// Grouping the fields into one parameter keeps the parametric test under
/// the clippy `too_many_arguments` ceiling without losing the per-case
/// readability that `#[rstest]` gives us.
struct FolderCase {
    fixture: &'static str,
    expected_id: &'static str,
    expected_name: Option<&'static str>,
    expected_count: usize,
    /// One file entry the case asserts on — id, name, size, direct CDN url.
    file: (&'static str, &'static str, u64, &'static str),
}

#[rstest]
#[case(FolderCase {
    fixture: "03_folder_single_file.json",
    expected_id: "abcDEF",
    expected_name: Some("Solo"),
    expected_count: 1,
    file: (
        "11111111-aaaa-bbbb-cccc-222222222222",
        "report.pdf",
        4096,
        "https://store-eu1.gofile.io/download/11111111-aaaa-bbbb-cccc-222222222222/report.pdf",
    ),
})]
#[case(FolderCase {
    fixture: "04_folder_multi_files.json",
    expected_id: "FOLDER1",
    expected_name: Some("Holiday Photos"),
    expected_count: 3,
    file: (
        "aaaa1111",
        "beach.jpg",
        2_048_000,
        "https://store-eu1.gofile.io/download/aaaa1111/beach.jpg",
    ),
})]
#[case(FolderCase {
    fixture: "06_folder_unicode_filename.json",
    expected_id: "uniFLD",
    expected_name: Some("Résumés Été 2026"),
    expected_count: 1,
    file: (
        "u1",
        "Résumé clé 2026.pdf",
        65_536,
        "https://store-eu1.gofile.io/download/u1/r%C3%A9sum%C3%A9.pdf",
    ),
})]
fn parses_recognised_folder_fixture(#[case] case: FolderCase) {
    let body = load_fixture(case.fixture);
    let folder = parse_folder_content(&body).expect("fixture must parse");
    assert_eq!(folder.id, case.expected_id, "fixture: {}", case.fixture);
    assert_eq!(
        folder.name.as_deref(),
        case.expected_name,
        "fixture: {}",
        case.fixture
    );
    assert_eq!(
        folder.files.len(),
        case.expected_count,
        "fixture: {}",
        case.fixture
    );
    let (file_id, file_name, file_size, file_link) = case.file;
    let entry = folder
        .files
        .iter()
        .find(|f| f.id == file_id)
        .unwrap_or_else(|| panic!("fixture {} missing file id {file_id}", case.fixture));
    assert_eq!(entry.name, file_name);
    assert_eq!(entry.size, file_size);
    assert_eq!(entry.direct_url, file_link);
}

#[test]
fn subfolder_fixture_drops_nested_folders() {
    let body = load_fixture("05_folder_with_subfolder.json");
    let folder = parse_folder_content(&body).unwrap();
    assert_eq!(
        folder.files.len(),
        1,
        "subfolder children must be filtered out — only direct files belong in the flat list"
    );
    assert_eq!(folder.files[0].id, "file111");
}

#[test]
fn unicode_fixture_preserves_filename() {
    let body = load_fixture("06_folder_unicode_filename.json");
    let folder = parse_folder_content(&body).unwrap();
    assert_eq!(folder.files[0].name, "Résumé clé 2026.pdf");
    assert_eq!(folder.name.as_deref(), Some("Résumés Été 2026"));
}

// ── getContent fixtures: error shapes ─────────────────────────────────────────

#[test]
fn not_found_fixture_maps_to_offline() {
    let body = load_fixture("07_folder_not_found.json");
    let err = parse_folder_content(&body).unwrap_err();
    assert!(
        matches!(err, PluginError::Offline(_)),
        "not-found responses must surface PluginError::Offline, got: {err:?}"
    );
}

#[test]
fn password_required_fixture_maps_to_offline() {
    let body = load_fixture("08_folder_password_required.json");
    let err = parse_folder_content(&body).unwrap_err();
    assert!(
        matches!(err, PluginError::Offline(_)),
        "password-required responses surface as Offline so the engine doesn't infinitely retry"
    );
}

#[test]
fn empty_folder_parses_then_fails_at_link_build() {
    let body = load_fixture("09_folder_empty.json");
    let folder = parse_folder_content(&body).expect("parser must accept empty folders");
    assert!(folder.files.is_empty());
    let err = build_extract_links_response(folder, None).unwrap_err();
    assert!(
        matches!(err, PluginError::EmptyFolder),
        "empty-folder response must fail at the link-build step, got: {err:?}"
    );
}

// ── End-to-end folder → ExtractLinksResponse ──────────────────────────────────

#[test]
fn multi_file_fixture_builds_full_extract_links_response() {
    let body = load_fixture("04_folder_multi_files.json");
    let folder = parse_folder_content(&body).unwrap();
    let response: ExtractLinksResponse = build_extract_links_response(folder, None).unwrap();
    assert_eq!(response.kind, "folder");
    assert_eq!(response.folder_id.as_deref(), Some("FOLDER1"));
    assert_eq!(response.files.len(), 3);

    let video = response
        .files
        .iter()
        .find(|f| f.id == "cccc3333")
        .expect("video file present");
    assert_eq!(video.filename.as_deref(), Some("video.mp4"));
    assert_eq!(video.size_bytes, Some(52_428_800));
    assert_eq!(video.url, "https://gofile.io/d/FOLDER1/cccc3333");
    assert_eq!(
        video.direct_url,
        "https://store-eu1.gofile.io/download/cccc3333/video.mp4"
    );
    assert_eq!(video.mime_type.as_deref(), Some("video/mp4"));
    assert!(video.resumable);
}

#[test]
fn single_file_fixture_uses_kind_file() {
    let body = load_fixture("03_folder_single_file.json");
    let folder = parse_folder_content(&body).unwrap();
    let response = build_extract_links_response(folder, None).unwrap();
    assert_eq!(
        response.kind, "file",
        "single-file folders must collapse to kind=file"
    );
    assert_eq!(response.files.len(), 1);
}

// ── Coverage sentinel ─────────────────────────────────────────────────────────

#[test]
fn fixture_count_covers_main_shapes() {
    let entries = fs::read_dir(FIXTURES_DIR)
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .count();
    assert!(
        entries >= 8,
        "expected at least 8 JSON fixtures (account + folder shapes + errors), found {entries}"
    );
}
