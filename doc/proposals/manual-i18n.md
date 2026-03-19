# Proposal: Multi-Language User Manual (i18n)

## Goal

Produce the MAKI user manual in English and German from a single source. All technical content (commands, config keys, data model, code examples, scripting) stays in English. Only prose descriptions, headings, and explanatory text are translated.

## Approach: Inline Language Markers

Each markdown file contains both languages, wrapped in HTML comment markers:

```markdown
## Getting Started

<!--EN-->
maki is a command-line digital asset manager designed for photographers
who manage large collections across multiple storage devices.
<!--/EN-->
<!--DE-->
maki ist ein kommandozeilenbasierter Digital-Asset-Manager für Fotografen,
die große Sammlungen über mehrere Speichergeräte hinweg verwalten.
<!--/DE-->

```bash
maki init
maki import ~/Photos
```
```

Code blocks, config examples, command output, and tables with technical content appear once (shared by both languages). Only prose is duplicated with markers.

### Why inline markers over parallel files

- **No drift**: when editing a paragraph, the other language is right there — impossible to forget
- **Shared code**: commands, config, output examples appear once
- **Incremental**: translate one section at a time; untranslated sections can fall back to English
- **Reviewable**: `git diff` shows both languages side by side

### Trade-offs

- Files become ~1.5-2x larger (prose is duplicated, code is not)
- Reading raw markdown is noisier (but the built PDF is clean)
- Merge conflicts in translated sections are slightly more complex

## Build Process

The existing `build-pdf.sh` gets a language parameter:

```bash
bash doc/manual/build-pdf.sh          # default: English
bash doc/manual/build-pdf.sh de       # German
```

The build script:
1. Copies the concatenated markdown to a temp file
2. Strips the non-selected language blocks (e.g. for German: remove `<!--EN-->...<!--/EN-->` blocks)
3. Strips the selected language markers themselves (leaving the content)
4. If a section has no `<!--DE-->` block, the English content remains as fallback
5. Generates the PDF with a language-appropriate title page ("User Manual" vs "Benutzerhandbuch")

The stripping logic is a simple `sed` or `awk` pass — roughly 10 lines of shell.

### Output files

- `maki-manual-en.pdf` — English manual (default, same as current `maki-manual.pdf`)
- `maki-manual-de.pdf` — German manual

## What Gets Translated

| Content | Translated? | Notes |
|---------|-------------|-------|
| Section headings | Yes | "Getting Started" → "Erste Schritte" |
| Prose paragraphs | Yes | Explanatory text, workflow descriptions |
| Command examples | No | `maki import`, `maki search tag:sunset` stay as-is |
| Config examples | No | TOML keys, values, comments stay in English |
| CLI output examples | No | Error messages, status output stay in English |
| Table headers (technical) | No | Column names for data model, config reference |
| Table headers (descriptive) | Yes | "What to do" → "Was zu tun ist" |
| Tips, warnings | Yes | "Note: ..." → "Hinweis: ..." |
| Cover page | Yes | "User Manual" → "Benutzerhandbuch" |
| Headers/footers | Yes | "MAKI User Manual" → "MAKI Benutzerhandbuch" |

## Effort Estimate

### Initial implementation (tooling)

- Build script: language parameter, stripping logic, dual title page — **3-4 hours**
- Test with one chapter, verify PDF output — **1 hour**

### Initial translation

- ~25 files, ~2000 lines of translatable prose (excluding shared code/config blocks)
- At ~200 lines/hour for technical translation by a native speaker: **10-15 hours**
- AI-assisted first draft + human review could reduce this to **6-10 hours**

### Ongoing maintenance

- Each feature/doc update: **15-30 min extra** to translate new/changed prose
- Compounding cost: ~1-2 hours/week during active development
- Risk: German falls behind during development sprints (mitigated by fallback to English)

## Suggested Implementation Steps

1. **Proof of concept** — implement the build tooling and translate one chapter (e.g. `01-overview.md` or `02-setup.md`) to validate the workflow and marker syntax
2. **Evaluate** — is the editing experience acceptable? Does the PDF look right?
3. **Translate remaining chapters** — working through the manual incrementally
4. **Release** — ship dual-language PDFs starting with the next minor version

## Open Questions

- Should the web UI also be translated? (Separate effort, involves Askama templates and JS strings — significantly more work and ongoing maintenance)
- Should the CLI help text (`--help`) be translated? (Unusual for CLI tools, not recommended)
- Should we include language selection in the GitHub release? (Two PDF attachments, or one combined PDF with both languages?)
