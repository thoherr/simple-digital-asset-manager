# Format Templates Reference

The `--format` flag controls how `maki search` and `maki duplicates` display their results. It accepts preset names or custom template strings with placeholder substitution.

---

## Presets

### `ids`

One full UUID per line, with no header or decoration. Ideal for scripting and piping into other commands.

**Shorthand:** `-q` (equivalent to `--format=ids`)

```
$ maki search -q "tag:landscape"
a1b2c3d4-5678-9abc-def0-123456789abc
e5f6a7b8-1234-5678-9abc-def012345678
```

The result count line is suppressed.

### `short`

Default compact view. One line per result showing short ID, filename, type, format, and date. A result count is printed at the end.

```
$ maki search "sunset"
a1b2c3d4  IMG_1234.jpg [image] (JPEG) -- 2026-01-15T10:30:00
e5f6a7b8  DSC_5678.nef [image] (NEF) -- 2026-01-14T16:45:00

2 result(s)
```

This is the default when no `--format` is specified.

### `full`

Detailed view including tags and description on each line.

```
$ maki search "sunset" --format full
a1b2c3d4  IMG_1234.jpg [image] (JPEG) -- 2026-01-15T10:30:00 tags:sunset,landscape A golden sunset over the mountains
e5f6a7b8  DSC_5678.nef [image] (NEF) -- 2026-01-14T16:45:00 tags:sunset,nature
```

### `json`

JSON array containing all result objects. Equivalent to using the global `--json` flag.

```
$ maki search "sunset" --format json
[
  {
    "asset_id": "a1b2c3d4-5678-9abc-def0-123456789abc",
    "original_filename": "IMG_1234.jpg",
    "asset_type": "image",
    ...
  }
]
```

---

## Custom Templates

Build your own output format by passing a string containing `{placeholder}` tokens.

**Syntax:** `--format '{placeholder}\t{another}'`

### Rules

- Placeholders are delimited by `{` and `}`.
- Known placeholders are replaced with the corresponding value for each result row.
- Unknown placeholders are left as-is in the output (e.g., `{bogus}` prints literally as `{bogus}`).
- An unclosed brace (no matching `}`) is emitted literally: `{world` prints as `{world`.
- The result count line is suppressed when a custom template (or any explicit `--format`) is used.

---

## Placeholders (search)

Available in `maki search --format`:

| Placeholder | Description |
|-------------|-------------|
| `{id}` | Full asset UUID |
| `{short_id}` | First 8 characters of the UUID |
| `{name}` | Asset name if set, otherwise falls back to the original filename |
| `{filename}` | Original filename (always the file's actual name, ignoring any custom asset name) |
| `{type}` | Asset type: `image`, `video`, `audio`, `document`, or `other` |
| `{format}` | Primary variant format/extension (e.g., `JPEG`, `NEF`, `MOV`) |
| `{date}` | Asset creation timestamp (ISO 8601 format) |
| `{tags}` | Comma-separated list of tags (empty string if no tags) |
| `{description}` | Description text (empty string if no description) |
| `{hash}` | Content hash of the best display variant |
| `{label}` | Color label name (e.g., `Red`, `Blue`; empty string if no label) |

## Placeholders (duplicates)

Available in `maki duplicates --format`. Includes all search placeholders above, plus:

| Placeholder | Description |
|-------------|-------------|
| `{locations}` | Comma-separated list of all file locations where the duplicate exists (format: `volume_label -> relative/path`) |

---

## Escape Sequences

Three escape sequences are recognized in template strings:

| Sequence | Output |
|----------|--------|
| `\t` | Tab character |
| `\n` | Newline character |
| `\\` | Literal backslash |

A backslash followed by any other character emits a literal backslash (the backslash is not consumed).

---

## Examples

### Tab-separated values for a spreadsheet

```bash
maki search "type:image" --format '{name}\t{format}\t{date}\t{tags}'
```

Output:

```
IMG_1234.jpg	JPEG	2026-01-15T10:30:00	sunset,landscape
DSC_5678.nef	NEF	2026-01-14T16:45:00	sunset,nature
```

### One name per line with rating label

```bash
maki search "rating:4+" --format '{name} [{label}]'
```

Output:

```
Golden Sunset [Red]
Mountain Lake [Blue]
```

### Multi-line detail view

```bash
maki search "tag:portfolio" --format '{name}\n  Format: {format}\n  Tags: {tags}\n  {description}\n'
```

Output:

```
Golden Sunset
  Format: NEF
  Tags: sunset,landscape,portfolio
  A golden sunset over the mountain range

Mountain Lake
  Format: JPEG
  Tags: nature,lake,portfolio
  Crystal clear reflections at dawn
```

### Short IDs with filenames (compact scripting format)

```bash
maki search "format:nef" --format '{short_id} {filename}'
```

Output:

```
a1b2c3d4 DSC_1234.NEF
e5f6a7b8 DSC_5678.NEF
```

### Hash-based file listing

```bash
maki search "type:image" --format '{hash}\t{filename}'
```

Output:

```
sha256:abc123def456...	IMG_1234.jpg
sha256:def789abc012...	DSC_5678.NEF
```

### Duplicate locations as tab-separated

```bash
maki duplicates --format '{filename}\t{format}\t{locations}'
```

Output:

```
DSC_1234.NEF	NEF	Photos -> Capture/2026-01-15/DSC_1234.NEF, Backup -> Capture/2026-01-15/DSC_1234.NEF
```

---

## Format Flag Behavior

| Flag | Equivalent | Result count shown |
|------|------------|:---:|
| (none) | `--format short` | yes |
| `--format short` | default | yes |
| `--format ids` | `-q` | no |
| `-q` | `--format ids` | no |
| `--format full` | -- | no |
| `--format json` | `--json` | no |
| `--format '{...}'` | custom template | no |

When `--format` is explicitly provided (any value including `short`), the result count line is suppressed, keeping output clean for piping and parsing. The count is only shown when no `--format` flag is given at all.

---

## Related Topics

- [Search Filters Reference](06-search-filters.md) -- all available search filters
- [Browse & Search (User Guide)](../user-guide/05-browse-and-search.md) -- practical search and output examples
- [CLI Conventions](00-cli-conventions.md) -- global flags, scripting patterns, exit codes
