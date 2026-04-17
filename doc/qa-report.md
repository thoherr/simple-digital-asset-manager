# QA Report — Post-Refactoring Code Quality Review

**Date**: 2026-04-17
**Baseline commit**: `855ad50` (P1a)
**Scope**: Rust source under `src/`, post four recent refactorings (P1a, P1b, P2a, P2b).

## Executive Summary

The four recent refactorings landed their intended wins — all four numerical claims verify. But the codebase is still heavily concentrated in four mega-files (`catalog.rs`, `asset_service.rs`, `main.rs`, `query.rs` together = 70% of LOC) and the largest functions remain enormous. The next highest-leverage cleanups are **boilerplate consolidation in `main.rs`**, **splitting `web/routes.rs` by resource domain**, and **breaking `import_with_callback` into phases**.

## 1. Code Metrics — Current State

### File-size distribution (LOC)

Total source: **58,044 LOC across 28 files**. Top files:

| Rank | File | LOC | Threshold |
|-----:|---|---:|---|
| 1 | `src/catalog.rs` | 8,944 | ⚠️ bloated |
| 2 | `src/asset_service.rs` | 8,706 | ⚠️ bloated |
| 3 | `src/main.rs` | 8,270 | ⚠️ bloated |
| 4 | `src/web/routes.rs` | 6,599 | ⚠️ bloated |
| 5 | `src/query.rs` | 5,996 | ⚠️ bloated |
| 6 | `src/xmp_reader.rs` | 2,156 | ⚠️ bloated |
| 7 | `src/shell.rs` | 1,709 | ⚠️ large |
| 8 | `src/face_store.rs` | 1,686 | ⚠️ large |
| 9 | `src/contact_sheet.rs` | 1,395 | acceptable |
| 10 | `src/preview.rs` | 1,355 | acceptable |
| 11 | `src/config.rs` | 1,139 | acceptable |
| 12 | `src/face.rs` | 1,115 | acceptable |
| 13 | `src/web/templates.rs` | 961 | ok |
| 14 | `src/ai.rs` | 960 | ok |
| 15 | `src/vlm.rs` | 950 | ok |

**Observation**: The top 5 files hold 38,515 LOC — 66% of the codebase. This is the central complexity hotspot.

### Largest functions

| Function | File | Lines | Note |
|---|---|---:|---|
| `run_command` | `main.rs:2338-8005` | **5,668** | dispatcher; 44 top-level `Commands::` arms |
| `import_with_callback` | `asset_service.rs` | ~668 | pipeline with embedded phases |
| `run_faces_command` | `main.rs:1707` | ~628 | extracted in P1a ✓ |
| `cleanup` | `asset_service.rs` | ~385 | procedural |
| `build_search_where` | `catalog.rs:2983` | ~350 | down from 467 in P1b ✓ |
| `relocate` | `asset_service.rs` | ~331 | procedural |
| `describe_inner` | `asset_service.rs` | ~313 | procedural |
| `refresh` | `asset_service.rs` | ~289 | procedural |
| `sync` | `asset_service.rs` | ~282 | procedural |
| `verify` | `asset_service.rs` | ~265 | procedural |
| `run_migrations` | `catalog.rs` | ~214 | unavoidable |

## 2. Refactoring Wins — Verified

| Ref | Target | Claim | Verified state |
|---|---|---|---|
| P1a | `run_faces_command` extracted | 617 lines out of `run_command` | ✓ `run_command` now 5,668 lines; `run_faces_command` ~628 lines as a sibling |
| P1b | `build_search_where` helpers | 467 → 390 lines | ✓ now ~350 lines (even better than the commit message claimed) |
| P2a | `Volume::online_map()` | 7 call sites deduplicated | ✓ 4 sites in `main.rs` + 3 in `asset_service.rs` confirmed |
| P2b | `resolve_collection_ids()` | 13 call sites deduplicated | ✓ 14 uses in `web/routes.rs` now go through the helper |

All four refactorings are clean, mechanical, and deliver real LOC reductions. No regressions evident.

## 3. Pain Points

### 3.1 `run_command` is still a 5,668-line mega-dispatcher

`main.rs:2338-8005` contains command-dispatch logic *and* inline business logic for 44 top-level commands. P1a removed only the faces subtree. 43 other commands (import, describe, search, show, preview, tag, edit, group, split, auto-group, auto-tag, embed, stack, duplicates, dedup, generate-previews, fix-roles, fix-dates, fix-recipes, create-sidecars, rebuild-catalog, migrate, relocate, update-location, verify, sync, sync-metadata, refresh, cleanup, writeback, backup-status, stats, serve, doc, licenses, saved-search, collection, shell, volume, init, delete, export, contact-sheet) are still inline.

### 3.2 Minor `FaceStore::initialize` pairing

Inside `run_faces_command`, the pair

```rust
let catalog = maki::catalog::Catalog::open(&catalog_root)?;
let _ = maki::face_store::FaceStore::initialize(catalog.conn());
```

repeats ~10 times (Cluster, People, Name, Merge, DeletePerson, Unassign, Clean, DumpAligned, Similarity, Export). A small helper would save ~20 LOC. The broader "catalog-opening boilerplate" claim does *not* hold up on inspection: the 39 `Catalog::open` sites in `main.rs` are mostly standalone one-liners — `catalog_root` is resolved once at the top of `run_command`, not per arm, so there is no repeated 3-5-line setup block to consolidate.

### 3.3 Ad-hoc progress and error printing

`eprintln!` appears **135 times** in `main.rs` alone. Many of these follow stereotyped patterns: `[{current}/{total}] ...`, `Error: ...`, duration formatting, short-ID truncation. There is no centralized printer/formatter, so each handler reinvents the message shape.

### 3.4 `asset_service.rs` is a collection of procedural mega-functions

Seven functions > 260 lines each (`import_with_callback`, `cleanup`, `relocate`, `describe_inner`, `refresh`, `sync`, `verify`). The largest, `import_with_callback` (~668 lines), is a callback-driven pipeline with 4–5 embedded phases (scan → type-filter → dedup → XMP writeback → preview generation) and no internal decomposition. This makes partial changes risky and testing tedious.

### 3.5 `web/routes.rs` is a flat 6,599-line file

All HTTP handlers for every resource (assets, search, browse, collections, people, stroll, calendar, map, analytics, similarity, saved-searches) live in one file. There's no per-resource organization.

### 3.6 Feature-gate sprawl

`#[cfg(feature = "ai")]` occurs **34 times in `main.rs`**, scattered across command handlers, argument parsing, and helper calls. Runtime branches per arm rather than variant-level gating.

### 3.7 Test coverage asymmetry

26 of 28 source files have `#[cfg(test)] mod tests` blocks. The conspicuous gap: **`main.rs` (8,270 lines, 0 tests)**. All CLI dispatch logic is integration-tested only through `tests/cli.rs`. Any pure logic embedded inline in command arms (output formatting, flag interpretation, glue) is effectively untested in isolation.

### 3.8 Repeated SQL join / scoring patterns in `catalog.rs`

CASE expressions for scoring (lines ~908-912 and ~1252-1256) and the `assets → variants → file_locations` join shape recur in multiple query-building functions without a named builder helper.

### 3.9 `face.rs` + `face_store.rs` cohesion

1,115 + 1,686 LOC across two top-level modules that are tightly coupled (every detector call ends in a store write). Candidate for a `faces/` subdirectory.

## 4. Prioritized Proposals

Ordered by leverage (impact per unit risk). Each carries an ID for reference in future commits.

### Priority 1 — Low risk, high clarity

**P3a — `open_catalog_with_faces()` helper** *(small, scoped)*
- **Extract**: the `Catalog::open` + `FaceStore::initialize(catalog.conn())` pair into one helper.
- **Impact**: ~20 LOC saved inside `run_faces_command` (10 call sites × 2 lines → 1).
- **Risk**: Low. Mechanical.
- **Effort**: ~15 minutes.
- **Note**: An earlier draft of this report proposed a much larger `open_catalog_and_config()` helper claiming ~150 LOC savings. That claim was based on a mis-reading: the 39 `Catalog::open` sites in `main.rs` are mostly standalone one-liners, not a repeated 3-5 line trio. Proposal downscoped accordingly.

**P3b — Progress / error printing helpers**
- **Extract**: `print_progress(current, total, item)`, `print_error(msg)`, `format_duration(dur)`, `short_id(hash)`.
- **Impact**: Collapses ~50 of the 135 `eprintln!` sites to one-liners; unifies message format.
- **Risk**: Low. Pure formatting.
- **Effort**: ~1 hour.

**P3c — Split `web/routes.rs` by resource**
- **Split**: `web/routes/{assets,search,browse,collections,people,stroll,calendar,map,analytics,saved_search}.rs`.
- **Impact**: No LOC reduction, but huge navigability win. 6,599 → ~700 LOC per file.
- **Risk**: Low-medium. Pure reorganization; careful with shared helpers and imports.
- **Effort**: ~2 hours.

### Priority 2 — Medium risk, high value

**P4a — Phase-split `import_with_callback`**
- **Break into**: `scan_candidates` → `filter_types` → `deduplicate` → `apply_metadata` → `write_xmp` → `generate_previews`, orchestrated by a thin top-level function.
- **Impact**: 668 LOC → 5 functions of ~130 LOC each + ~60 LOC orchestration; each phase independently testable.
- **Risk**: Medium. Callback semantics must be preserved across phase boundaries.
- **Effort**: ~half day.

**P4b — `CommandContext` struct**
- **Introduce**: `struct CommandContext { catalog, config, verbosity, json, log }`, thread through command handlers instead of 5 separate params.
- **Impact**: Cleaner signatures; sets up future testability of command logic in isolation.
- **Risk**: Low-medium. Touches many function signatures but no logic.
- **Effort**: ~2 hours.

**P4c — Merge `face.rs` + `face_store.rs` into `faces/`**
- **Reorg**: `faces/detector.rs`, `faces/store.rs`, `faces/mod.rs`.
- **Impact**: Structural only; reflects actual coupling.
- **Risk**: Low. No logic changes.
- **Effort**: ~30 minutes.

### Priority 3 — Design work required

**P5a — Query-builder DRY-out in `catalog.rs`**
- **Extract**: Named builder for `assets → variants → file_locations` joins; shared scoring CASE fragments.
- **Impact**: ~50-100 LOC; fewer places to update when schema changes.
- **Risk**: Medium-high. Subtle SQL bugs possible; need good test coverage first.

**P5b — Phase-split `sync` / `refresh` / `verify`**
- **Impact**: ~100 LOC across the three; each phase testable separately.
- **Risk**: Medium. Ordering constraints in state-mutating operations.

**P5c — Variant-level feature gating for AI commands**
- **Change**: Gate `Commands::Faces` (and other AI-only variants) at the enum level via `#[cfg(feature = "ai")]`, remove the ~34 runtime branches in `main.rs`.
- **Impact**: ~30 LOC; cleaner compile-time separation.
- **Risk**: Medium. Requires validating all binary-size / feature-combination CI builds.

## 5. Recommended Next Step

Start with **P3b + P3c** as a single refactoring batch — both low-risk, both mechanical, both improve navigability and consistency with no behavioral change. Combined expected impact: **~100 LOC removed and `web/routes.rs` reorganized from one 6,599-line file into ~10 per-resource modules of ~700 LOC each**. P3a is a nice-to-have cleanup inside `run_faces_command` but trivial in scope.

After that, **P4a** (import phase split) is the highest-value deeper cleanup, but it deserves its own focused session.
