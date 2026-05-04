# Contributing to vortex-mod-gofile

First off, thanks for considering contributing! Every contribution matters, whether it's a bug report, a feature request, or a pull request.

## How to Contribute

### Reporting Bugs

1. Check if the bug has already been reported in [Issues](https://github.com/mpiton/vortex-mod-gofile/issues)
2. If not, create a new issue using the **Bug Report** template
3. Include steps to reproduce, expected behavior, and actual behavior

### Suggesting Features

1. Check existing [Feature Requests](https://github.com/mpiton/vortex-mod-gofile/issues?q=label%3Aenhancement)
2. Open a new issue using the **Feature Request** template
3. Describe the problem and your proposed solution

### Pull Requests

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/your-feature`)
3. Make your changes following the project's coding standards
4. Write or update tests as needed
5. Commit using [Conventional Commits](https://www.conventionalcommits.org/) format
6. Push to your fork and open a Pull Request

### Commit Message Format

```
<type>(<scope>): <description>

[optional body]
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `chore`, `ci`

### Development Setup

```bash
git clone https://github.com/mpiton/vortex-mod-gofile.git
cd vortex-mod-gofile

rustup target add wasm32-wasip1
cargo test                                    # native unit + fixture tests
cargo build --target wasm32-wasip1 --release  # build WASM
cargo test --test wasm_smoke                  # WASM smoke (Extism-loaded)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

### Plugin contract

This plugin implements the Vortex plugin contract:

| Function             | Input        | Output                       |
|----------------------|--------------|------------------------------|
| `can_handle`         | URL string   | `"true"` / `"false"`         |
| `supports_playlist`  | URL string   | `"true"` for folder URLs     |
| `extract_links`      | URL string   | JSON `ExtractLinksResponse`  |
| `resolve_stream_url` | JSON `{url}` | direct CDN URL string        |

When changing JSON shapes or URL recognition rules, update `tests/fixtures/*.json` and the corresponding `tests/api_fixtures.rs` cases. The Extism WASM smoke test (`tests/wasm_smoke.rs`) covers the end-to-end `createAccount → getContent` flow with a stubbed `http_request`.

### Releasing a new version

1. Bump `version` in both `Cargo.toml` and `plugin.toml`
2. Run the full test/lint pipeline above
3. Compute checksums:
   ```bash
   sha256sum target/wasm32-wasip1/release/vortex_mod_gofile.wasm
   sha256sum plugin.toml
   ```
4. Update `CHANGELOG.md` (move `[Unreleased]` items to a new `[X.Y.Z]` section)
5. Tag and push: `git tag -a vX.Y.Z -m "Release vX.Y.Z" && git push --tags`
6. Open a PR against `mpiton/vortex` to update the matching entry in `registry/registry.toml` (version + both checksums)

## Questions?

Open a [Discussion](https://github.com/mpiton/vortex-mod-gofile/discussions) or file an issue using the **Question** template.
