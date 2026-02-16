# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A digital asset manager designed for large collections of images and videos (terabytes across multiple offline storage devices). Key design goals:

- Content-addressable storage for originals (SHA-based, since originals like RAW files are immutable)
- Text-based metadata in sidecar files
- Grouping of asset variants (RAW/JPEG, different processing versions)
- Deduplication of identical files
- Transparent file relocation across storage devices
- Management of processing recipes (CaptureOne, Photoshop, etc.)
- Location-independent navigation and retrieval

## Technology

- **Language**: Rust
- **Platforms**: macOS, Linux
- **Interface**: CLI-first (`dam` command), optional web GUI
- **Catalog**: SQLite (cache/index), YAML sidecar files (source of truth)
- **Key crates**: clap, sha2, serde, rusqlite, axum, kamadak-exif, image
- **External tools**: dcraw/libraw (RAW previews), ffmpeg (video thumbnails)

## Architecture

See `doc/architecture-overview.md` for the high-level system design and `doc/component-specification.md` for detailed component specs.

Core layers: CLI → Core Library (Asset Service, Content Store, Metadata Store, Device Registry, Query Engine, Preview Generator) → Storage (Local Catalog + Media Volumes).

## Status

Core CLI is functional. See `specification.md` for full requirements.

**Implemented commands**: `init`, `volume add/list`, `import`, `search`, `show`, `tag`, `group`, `rebuild-catalog`

**Not yet implemented**: `relocate`, `verify`, `duplicates`
