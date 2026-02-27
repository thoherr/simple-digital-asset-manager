# Configuration Reference (dam.toml)

The `dam.toml` file stores catalog-level configuration. It lives at the root of your catalog directory (the same directory that contains the `metadata/`, `previews/`, and `catalog.db` files).

---

## File Location

`dam.toml` is created automatically by `dam init`. dam locates it by searching the current directory and walking up through parent directories until it finds a directory containing `dam.toml`.

```
my-catalog/
  dam.toml              <-- configuration file
  catalog.db
  metadata/
  previews/
  searches.toml
  collections.yaml
```

All sections and fields are optional. A missing file or an empty file is equivalent to all-defaults. A comment-only file is also valid:

```toml
# dam catalog configuration
```

---

## Top-Level Fields

### default_volume

- **Type:** UUID string (optional)
- **Default:** none

Fallback volume for `dam import` when auto-detection from the file path is ambiguous or fails. When set, import uses this volume if it cannot determine the correct volume from the first path argument.

```toml
default_volume = "550e8400-e29b-41d4-a716-446655440000"
```

Find your volume UUIDs with `dam volume list --json`.

---

## [preview] Section

Controls how preview thumbnails are generated during import and by `dam generate-previews`.

### max_edge

- **Type:** unsigned integer
- **Default:** `800`
- **Validation:** must be greater than 0

Maximum pixel size of the longest edge for generated thumbnails. The shorter edge is scaled proportionally. Applies to all preview types (standard images, RAW conversions, video frames, and info cards).

### format

- **Type:** string (`"jpeg"` or `"webp"`)
- **Default:** `"jpeg"`

Output format for preview files. JPEG produces lossy compressed thumbnails controlled by the `quality` setting. WebP produces lossless thumbnails via the `image` crate (the `quality` setting is ignored for WebP).

### quality

- **Type:** unsigned integer (1--100)
- **Default:** `85`
- **Validation:** must be between 1 and 100

JPEG compression quality. Higher values produce larger files with better visual quality. Only applies when `format = "jpeg"`. Ignored for WebP.

### Notes

Changing `max_edge` or `format` affects only newly generated previews. Existing previews are not automatically regenerated. Use `dam generate-previews --force` to regenerate all previews with the new settings.

```toml
[preview]
max_edge = 1200
format = "jpeg"
quality = 90
```

---

## [serve] Section

Controls the built-in web UI server started by `dam serve`.

### port

- **Type:** unsigned 16-bit integer
- **Default:** `8080`

TCP port for the web server.

### bind

- **Type:** string (IP address)
- **Default:** `"127.0.0.1"`

Bind address for the web server. Use `"127.0.0.1"` to restrict access to the local machine. Use `"0.0.0.0"` to allow connections from other devices on the network.

### CLI Override

The `--port` and `--bind` flags on `dam serve` override the values from `dam.toml`:

```bash
dam serve --port 9090 --bind 0.0.0.0
```

```toml
[serve]
port = 8080
bind = "127.0.0.1"
```

---

## [import] Section

Controls import behavior for `dam import`.

### exclude

- **Type:** array of glob pattern strings
- **Default:** `[]` (empty, nothing excluded)

Glob patterns matched against filenames (not full paths). Files matching any pattern are skipped during import. Useful for excluding OS-generated files, temporary files, and other non-media clutter.

Pattern matching uses the `glob-match` crate with standard glob syntax: `*` matches any sequence of characters, `?` matches a single character.

```toml
[import]
exclude = [
    "Thumbs.db",
    ".DS_Store",
    "*.tmp",
    "*.bak",
    "desktop.ini",
]
```

### auto_tags

- **Type:** array of tag strings
- **Default:** `[]` (empty, no auto-tags)

Tags automatically applied to every newly imported asset. These are merged with any tags extracted from XMP metadata and deduplicated (no duplicate tags are created).

Useful for marking import batches or applying a default workflow status:

```toml
[import]
auto_tags = ["inbox", "unreviewed"]
```

---

## [dedup] Section

Controls dedup behavior for `dam dedup` and the web UI's auto-resolve action.

### prefer

- **Type:** string (optional)
- **Default:** none

Default path substring for the `--prefer` flag. When set, dedup prefers keeping file locations whose relative path contains this string. Useful for always keeping files in a curated directory (e.g. `Selects`) while removing copies elsewhere.

The CLI `--prefer` flag overrides this value. The web UI duplicates page pre-populates its "Prefer keeping" input from this setting.

```toml
[dedup]
prefer = "Selects"
```

---

## Full Example

A complete `dam.toml` with all options set and annotated:

```toml
# Default volume for import when auto-detection is ambiguous.
# Find volume UUIDs with: dam volume list --json
default_volume = "550e8400-e29b-41d4-a716-446655440000"

[preview]
# Maximum pixel size of the longest edge for thumbnails.
max_edge = 1200
# Output format: "jpeg" (lossy) or "webp" (lossless).
format = "jpeg"
# JPEG quality (1-100). Ignored for WebP.
quality = 90

[serve]
# Web UI port. Override with: dam serve --port 9090
port = 8080
# Bind address. Use "0.0.0.0" to allow network access.
bind = "127.0.0.1"

[import]
# Glob patterns to exclude during import (matched against filenames).
exclude = [
    "Thumbs.db",
    ".DS_Store",
    "*.tmp",
    "*.bak",
    "desktop.ini",
]
# Tags automatically applied to every new asset.
auto_tags = ["inbox", "unreviewed"]

[dedup]
# Default path substring for --prefer (keep files whose path contains this).
prefer = "Selects"
```

---

## Minimal Examples

### Preview-only: larger thumbnails in WebP

```toml
[preview]
max_edge = 1600
format = "webp"
```

### Import-only: exclude system files and auto-tag

```toml
[import]
exclude = [".DS_Store", "Thumbs.db", "*.tmp"]
auto_tags = ["2026-import"]
```

### Serve-only: custom port for LAN access

```toml
[serve]
port = 9090
bind = "0.0.0.0"
```

---

## Defaults Summary

When a field is absent from `dam.toml`, these defaults apply:

| Field | Default |
|-------|---------|
| `default_volume` | none |
| `preview.max_edge` | `800` |
| `preview.format` | `"jpeg"` |
| `preview.quality` | `85` |
| `serve.port` | `8080` |
| `serve.bind` | `"127.0.0.1"` |
| `import.exclude` | `[]` |
| `import.auto_tags` | `[]` |
| `dedup.prefer` | none |

---

## Related Topics

- [Setup (User Guide)](../user-guide/02-setup.md) -- creating a catalog with `dam init`
- [CLI Conventions](00-cli-conventions.md) -- global flags and catalog discovery
- [Search Filters Reference](06-search-filters.md) -- search query syntax
