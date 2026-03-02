# dam User Manual

**dam** is a command-line digital asset manager built in Rust, designed for photographers and media professionals who manage large collections across multiple storage devices.

This manual is organized into three sections:

## User Guide

Workflow-oriented guides that walk you through common tasks.

1. [Overview & Concepts](user-guide/01-overview.md) — Data model, architecture, and the round-trip workflow
2. [Setup](user-guide/02-setup.md) — Installation, initialization, volumes, and configuration
3. [Ingesting Assets](user-guide/03-ingest.md) — Importing files, auto-grouping, metadata extraction, and previews
4. [Organizing Assets](user-guide/04-organize.md) — Tags, editing, grouping, collections, and saved searches
5. [Browsing & Searching](user-guide/05-browse-and-search.md) — CLI search, filters, output formats, and statistics
6. [Web UI](user-guide/06-web-ui.md) — Browser interface, batch operations, and keyboard navigation
7. [Maintenance](user-guide/07-maintenance.md) — Verification, sync, refresh, cleanup, and relocation
8. [Scripting](user-guide/08-scripting.md) — Shell and Python scripting patterns, workflow automation

## Reference Guide

Man-page style documentation for every command, filter, and configuration option.

- [CLI Conventions](reference/00-cli-conventions.md) — Global flags, scripting patterns, exit codes
- [Setup Commands](reference/01-setup-commands.md) — `init`, `volume add`, `volume list`, `volume combine`, `volume remove`
- [Ingest Commands](reference/02-ingest-commands.md) — `import`, `tag`, `edit`, `group`, `auto-group`
- [Organize Commands](reference/03-organize-commands.md) — `collection`, `saved-search`
- [Retrieve Commands](reference/04-retrieve-commands.md) — `search`, `show`, `export`, `duplicates`, `stats`, `serve`
- [Maintain Commands](reference/05-maintain-commands.md) — `verify`, `sync`, `refresh`, `cleanup`, `relocate`, `update-location`, `generate-previews`, `fix-roles`, `fix-dates`, `rebuild-catalog`
- [Search Filters](reference/06-search-filters.md) — Complete filter syntax reference
- [Format Templates](reference/07-format-templates.md) — Output format presets, custom templates, placeholders
- [Configuration](reference/08-configuration.md) — `dam.toml` reference
- [Data Model](reference/09-data-model.md) — Asset, Variant, Recipe, Volume, and FileLocation entities

## Developer Guide

Technical documentation for integrators and contributors.

1. [REST API](developer/01-rest-api.md) — Complete web API documentation
2. [Module Reference](developer/02-module-reference.md) — Rust module overview and dependency graph
3. [Building & Testing](developer/03-building-and-testing.md) — Build commands, tests, and release process

---

**Version**: v1.8.9 | **Source**: [GitHub](https://github.com/tblck/dam) | **License**: See repository
