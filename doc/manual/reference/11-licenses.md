# Licenses & Acknowledgements

This appendix summarises the licensing of MAKI itself, the Rust crates compiled into the binary, the AI models downloaded on demand, and the external tools MAKI calls.

## MAKI

MAKI is released under the **Apache License 2.0**. The full license text is in the `LICENSE` file at the root of the source repository and is available at <https://www.apache.org/licenses/LICENSE-2.0>.

```
Copyright 2024–2026 Thomas Herrmann
Licensed under the Apache License, Version 2.0
```

You may use MAKI for any purpose, modify it, and redistribute it (in source or compiled form), subject to the conditions of the Apache License 2.0 — primarily that you preserve the copyright notice and provide a copy of the license with any redistribution.

## Third-party Rust crates

The MAKI binary statically links many open-source Rust crates. All bundled dependencies use **permissive open-source licenses**:

| License | Examples | Notes |
|---------|----------|-------|
| Apache-2.0 | tokio, axum, serde, image, ort | Permissive; requires copyright preservation |
| MIT | clap, sha2, anyhow, rusqlite | Permissive; requires copyright preservation |
| BSD-2-Clause / BSD-3-Clause | various | Permissive; no advertising clause |
| ISC | various | Functionally equivalent to MIT |
| MPL-2.0 | a few crates | Weak copyleft, file-level only |
| NCSA | rav1e (transitive) | University of Illinois — permissive |
| Unicode-3.0 | text/Unicode crates | Unicode consortium license |
| Zlib, BSL-1.0, CC0, 0BSD | misc. | All permissive |

The full text of every dependency's license — including copyright notices and the dependency tree showing which crates use which license — is in the file:

```
THIRD_PARTY_LICENSES.md
```

This file is shipped alongside the `maki` binary in every release archive (`maki-<version>-<platform>.tar.gz` / `.zip`). It is generated automatically with [`cargo-about`] during the release build.

You can also see a summary on the command line:

```bash
maki licenses              # full overview
maki licenses --summary    # short version
maki licenses --json       # machine-readable
```

[`cargo-about`]: https://github.com/EmbarkStudios/cargo-about

### License compatibility validation

The MAKI build pipeline runs [`cargo-deny`] on every release to validate that:

- All dependencies use only the licenses listed above (no copyleft licenses like GPL or LGPL slip in via transitive dependencies)
- No security advisories affect any compiled-in crate
- All dependencies come from `crates.io` (no untrusted git or alternate registries)

The configuration files are `about.toml` (for `cargo-about`) and `deny.toml` (for `cargo-deny`) at the repository root.

[`cargo-deny`]: https://github.com/EmbarkStudios/cargo-deny

## AI models *(Pro)*

MAKI Pro uses image-text encoder models (SigLIP, SigLIP 2) and face detection/recognition models. These are **downloaded on demand** from Hugging Face the first time you run `maki auto-tag --download` or trigger a feature that needs them; they are not bundled in the binary.

| Model | Source | License | Credit |
|-------|--------|---------|--------|
| `siglip-vit-b16-256` | `Xenova/siglip-base-patch16-256` | Apache-2.0 | Google Research |
| `siglip-vit-l16-256` | `Xenova/siglip-large-patch16-256` | Apache-2.0 | Google Research |
| `siglip2-base-256-multi` | `onnx-community/siglip2-base-patch16-256-ONNX` | Apache-2.0 | Google Research (SigLIP 2) |
| `siglip2-large-256-multi` | `onnx-community/siglip2-large-patch16-256-ONNX` | Apache-2.0 | Google Research (SigLIP 2) |
| Face detection (RetinaFace) | Hugging Face | Apache-2.0 | InsightFace project |
| Face recognition (ArcFace) | Hugging Face | Apache-2.0 | InsightFace project |

By downloading these models, you agree to the terms of the Apache-2.0 license under which they are distributed. Because MAKI does not bundle the model weights, MAKI itself has no redistribution obligation for them; the obligation, if any, is on you when you copy a downloaded model elsewhere.

## External tools

MAKI calls several external command-line tools as separate processes when they are present on the system. These tools are **not bundled** with MAKI — they are installed by the user, governed by their own licenses, and run as independent processes.

| Tool | Used for | License |
|------|----------|---------|
| `dcraw` / `libraw` | RAW image preview generation | LGPL-2.1 (libraw), GPL-2 (dcraw) |
| `ffmpeg` | Video thumbnail and proxy generation | LGPL-2.1 / GPL-2 (build-dependent) |
| `ffprobe` | Video metadata extraction (duration, codec, resolution) | Same as ffmpeg |
| `curl` | AI model download and VLM HTTP calls | MIT/X-style |

Because MAKI invokes these tools via `std::process::Command` (a separate-process boundary), there is no linking and no derived-work relationship — MAKI's Apache-2.0 license is unaffected by their licenses, and vice versa.

If you redistribute MAKI bundled together with these tools (for example, in an installer that ships ffmpeg alongside the `maki` binary), you become the redistributor of those tools and must comply with their respective licenses. The default MAKI release archives only contain the `maki` binary, the manual PDF, and `THIRD_PARTY_LICENSES.md` — no external tools are bundled.

## Reporting license concerns

If you believe a dependency listed in `THIRD_PARTY_LICENSES.md` is incorrectly attributed, or if you spot a license-compatibility issue we should address, please open an issue at <https://github.com/thoherr/maki/issues>.

---

This appendix is updated whenever the dependency tree changes. The authoritative, version-pinned license text for the binary you are running is always in the `THIRD_PARTY_LICENSES.md` file shipped in your release archive.
