# vortex-mod-gofile

Gofile WASM plugin for [Vortex](https://github.com/mpiton/vortex). Resolves
public Gofile share pages (`https://gofile.io/d/<folder>`) into one or more
direct CDN URLs via the public JSON API and a guest token.

## Features

- Two HTTP calls per resolution — `GET /createAccount` (guest token) then
  `GET /getContent?contentId=<id>&token=<token>`
- Folder share URLs return one `FileLink` per direct file child; nested
  sub-folders are skipped (flat list semantics)
- Multi-file folders synthesise per-file URLs of the shape
  `https://gofile.io/d/<folder>/<file>` so the host's `resolve_stream_url`
  contract — "url string in, single CDN url out" — has something to
  disambiguate against
- Maps Gofile's status envelope onto the Vortex plugin error vocabulary:
  - `"error-notFound"` / `"error-passwordRequired"` → `PluginError::Offline`
    (engine surfaces the link as offline rather than hammering retry)
  - any other `"error-..."` → `PluginError::ApiError`
- Forward-compatible parsers — fields the API may add later (md5, modTime,
  ads flags…) are ignored without rejecting the response

## URL shapes recognised

- `https://gofile.io/d/<folder>` (canonical share page)
- `https://www.gofile.io/d/<folder>` (alias host)
- `https://gofile.io/d/<folder>/<file>` (synthesised per-file shape — only
  produced by this plugin's `extract_links` and consumed by
  `resolve_stream_url`; not a public Gofile URL)

`<folder>` is an alphanumeric token of 6+ characters; `<file>` is the
gofile content id of the child (alphanumeric, dash, or underscore).

## Plugin contract

| Function                 | Input        | Output                        |
|--------------------------|--------------|-------------------------------|
| `can_handle`             | URL string   | `"true"` / `"false"`          |
| `supports_playlist`      | URL string   | `"true"` for folder URLs      |
| `extract_links`          | URL string   | JSON `ExtractLinksResponse`   |
| `resolve_stream_url`     | JSON `{url}` | direct CDN URL string         |

`ExtractLinksResponse` mirrors the `LinkStatus::Online` shape used by the
host's link-check pipeline. Each `files[]` entry carries `filename`,
`size_bytes`, `mime_type`, `direct_url`, and `resumable: true`.

## Build

```bash
rustup target add wasm32-wasip1
cargo build --target wasm32-wasip1 --release
```

Resulting WASM: `target/wasm32-wasip1/release/vortex_mod_gofile.wasm`.

## Install (development)

```bash
PLUGIN_DIR="$HOME/.local/share/dev.vortex.app/plugins/vortex-mod-gofile"
mkdir -p "$PLUGIN_DIR"
cp plugin.toml "$PLUGIN_DIR/plugin.toml"
cp target/wasm32-wasip1/release/vortex_mod_gofile.wasm \
   "$PLUGIN_DIR/vortex-mod-gofile.wasm"
```

Vortex picks up the new plugin via the file watcher; no restart needed.

## Tests

```bash
cargo test                                  # native unit + fixture tests
cargo build --target wasm32-wasip1 --release  # build for WASM smoke
cargo test --test wasm_smoke                # WASM smoke (auto-skip if wasm missing)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The JSON API fixtures live in `tests/fixtures/*.json` — nine variants
covering the createAccount happy path / error envelope, single- and
multi-file folders, nested sub-folder skipping, unicode filenames,
not-found and password-protected errors, and an empty folder.

## License

GPL-3.0 — same as Vortex core.
