# Building & Testing

## Building

### Debug Build

```bash
cargo build
```

Produces an unoptimized binary at `target/debug/dam` with debug symbols and runtime checks enabled. Fast compilation, suitable for development.

### Release Build

```bash
cargo build --release
```

Produces an optimized binary at `target/release/dam`. Significantly faster runtime performance. Use this for production deployment.

### Requirements

- **Rust edition**: 2021 (stable toolchain)
- **Platforms**: macOS, Linux
- **SQLite**: Bundled via `rusqlite` with the `bundled` feature (no system SQLite required)

## Testing

### Run All Tests

```bash
cargo test
```

Runs approximately 693 tests total: ~465 unit tests and ~228 integration tests.

### Unit Tests Only

```bash
cargo test --lib
```

Runs the ~465 unit tests embedded in library source files (`#[cfg(test)]` modules within `src/`).

### Integration Tests Only

```bash
cargo test --test integration
```

Runs the ~228 integration tests defined in `tests/cli.rs`. These tests exercise the full system through the CLI binary and library API, using temporary catalogs and volumes.

### Run a Specific Test

```bash
cargo test test_name_pattern
```

Runs only tests whose names match the given pattern.

### Test Helpers

The integration test suite provides helper functions for setting up test catalogs:

- **`setup_search_catalog()`** -- Creates a catalog with pre-populated assets for search testing. Requires `asset.variants` to be populated before calling `catalog.insert_asset()` (because denormalized columns `best_variant_hash`, `primary_variant_format`, and `variant_count` are computed at insert time).

- **`setup_metadata_catalog()`** -- Creates a catalog with assets that have metadata (tags, ratings, descriptions, recipes) for metadata operation testing. Same requirement: variants must be populated before insert.

## Documentation

### Rust API Docs

```bash
cargo doc --no-deps --open
```

Generates HTML documentation from doc comments and opens it in your browser. The `--no-deps` flag skips building docs for third-party dependencies, which speeds up the build considerably. Output is at `target/doc/dam/`.

### PDF Manual

```bash
bash doc/manual/build-pdf.sh
```

Generates `doc/manual/dam-manual.pdf` from the 21 Markdown source files. The script concatenates all sections in order, renders mermaid diagrams to PNG, and produces a PDF with table of contents, headers/footers, and syntax-highlighted code blocks. The version number is read from `Cargo.toml`.

**Prerequisites** (not required for building or running dam itself):

- **pandoc** -- Document conversion. `brew install pandoc`
- **XeLaTeX** -- PDF typesetting with Unicode support. `brew install --cask mactex-no-gui`
- **mermaid-cli** (`mmdc`) -- Diagram rendering. `brew install mermaid-cli`

## Release Process

1. **Update documentation**: User manual, README, CHANGELOG, and any other docs affected by the release.

2. **Bump version** in `Cargo.toml`:
   ```toml
   [package]
   version = "X.Y.Z"
   ```

3. **Update lockfile**:
   ```bash
   cargo build
   ```
   This regenerates `Cargo.lock` with the new version.

4. **Run all tests**:
   ```bash
   cargo test
   ```
   All tests must pass before releasing.

5. **Commit**:
   ```bash
   git add -A
   git commit -m "Release vX.Y.Z -- brief description"
   ```

6. **Tag**:
   ```bash
   git tag vX.Y.Z
   ```

7. **Push**:
   ```bash
   git push && git push --tags
   ```

8. **Create GitHub release**:
   ```bash
   gh release create vX.Y.Z --title "vX.Y.Z" --notes "changelog content here"
   ```

## Dependencies

### Rust Crates

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing with derive macros |
| `sha2` | SHA-256 content hashing |
| `serde` / `serde_json` / `serde_yaml` | Serialization for JSON output, YAML sidecars |
| `rusqlite` | SQLite database access (bundled) |
| `kamadak-exif` | EXIF metadata extraction from images |
| `quick-xml` | XMP/XML parsing and manipulation |
| `regex` | Pattern matching in query parsing and XMP processing |
| `image` | Image decoding, resizing, and encoding for previews |
| `imageproc` | Text rendering on info card previews |
| `ab_glyph` | Font loading for info card text (embedded DejaVu Sans) |
| `lofty` | Audio metadata extraction (duration, bitrate) for info cards |
| `uuid` | UUID v4 generation (asset IDs) and v5 (deterministic IDs) |
| `axum` | HTTP web framework for the `serve` command |
| `askama` | Compile-time HTML template engine |
| `tokio` | Async runtime for the web server |
| `tower-http` | Static file serving middleware (`ServeDir` for previews) |
| `toml` | Configuration file parsing (`dam.toml`, `searches.toml`) |
| `glob-match` | Filename glob matching for import exclusion patterns |
| `chrono` | Date/time handling with serde support |
| `anyhow` / `thiserror` | Error handling |

### Dev Dependencies

| Crate | Purpose |
|-------|---------|
| `assert_cmd` | CLI binary testing (running `dam` as a subprocess) |
| `predicates` | Assertion helpers for CLI output matching |
| `tempfile` | Temporary directories for test isolation |

### External Tools (Optional)

These tools are not Rust dependencies but are invoked as subprocesses for specific preview generation tasks. Their absence does not prevent the application from running; missing tools result in info card fallback previews.

- **dcraw** or **LibRaw** (`dcraw_emu`) -- RAW image preview extraction. Used to decode camera-native formats (NEF, ARW, CR2, CR3, etc.) into RGB data for thumbnail generation. LibRaw's `dcraw_emu` is preferred when available.

- **ffmpeg** -- Video thumbnail extraction. Used to capture a frame from video files (MP4, MOV, AVI, etc.) for preview generation.

To check if these tools are available:

```bash
which dcraw_emu || which dcraw
which ffmpeg
```

Preview generation silently falls back to info cards (metadata display images) when these tools are missing. Use `dam generate-previews --debug` to see external tool invocations and errors.
