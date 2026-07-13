# AGENTS.md — AI Agent Guide for Imageglass-Ithmb-Plugin

This file tells AI coding agents how to work with this repository. Read this first before editing any code.

## Repository Purpose

C ABI shared library plugin for [ImageGlass v10](https://imageglass.org) that enables decoding Apple `.ithmb` thumbnail files. This is a **thin FFI wrapper** around the core decoding library in `ithmb-core`.

## Architecture

```
ImageGlass (cross-platform Avalonia UI)
    ↓ ig_plugin_get_api()
Imageglass-Ithmb-Plugin (C ABI cdylib)
    ↓ FFI
ithmb-core (Rust library, loaded as shared lib)
    ↔ 7 encoders, 8 decoders, 54 profiles, PhotoDB
```

## Key Facts

- **Language**: Rust (cdylib)
- **ABI**: ImageGlass v10 native codec plugin (v1.0.0.0)
- **Platforms**: Linux, macOS, Windows (CI builds all 3)
- **Memory rule**: Plugin allocates pixel buffers via its own allocator (`libc::malloc`), not the host allocator. Whoever allocates, frees.
- **Buffer tracking**: `BufferRegistry` in `buffer_registry.rs` tracks live pixel buffers to prevent double-free and use-after-free.
- **Build**: `cargo build --release`, then `./scripts/package.sh [linux|macos|windows]` produces `.igplugin.zip`
- **CI**: GitHub Actions builds + clippy + deny for all 3 platforms on every push, automatically creates releases on `v*` tags
- **Dependency**: [`ithmb-core`](https://github.com/B67687/Ithmb-Codec)

## Plugin Files

| Path | Purpose |
|------|---------|
| `src/lib.rs` | C ABI entry point, codec API, metadata loading, decode dispatch |
| `src/allocator.rs` | `pixel_buffer_alloc`/`pixel_buffer_free` wrappers (own allocator, not host) |
| `src/buffer_registry.rs` | Thread-safe HashMap tracking live pixel allocations |
| `src/types.rs` | `#[repr(C)]` ABI type definitions mirroring ImageGlass C# SDK structs |
| `src/logging.rs` | Thin wrapper around host logging callback |
| `igplugin.json` | Plugin manifest (id, name, executable, kind) |
| `scripts/package.sh` | Build + package into `.igplugin.zip` per platform |

## load_metadata Flow

1. Read 4-byte format prefix from file
2. Try `device_profiles::find_formats_by_id()` (fast path, ~41 of 54 profiles)
3. If not found, fall back to `ProfileDb::load_builtin() + get(prefix)` (all 54 profiles)
4. Return correct dimensions or `NotImplemented`

## free_pixel_buffer Safety

- Always clears buffer struct fields first (prevents ImageGlass from accessing stale pointers)
- Checks `BufferRegistry` before freeing
- Uses own allocator (`allocator::pixel_buffer_free`) — NOT the host allocator
- Safe during shutdown: host allocator may have been torn down, but our allocator is always available

## Relationship to Ithmb-Codec

All decoding logic is in the main [`Ithmb-Codec`](https://github.com/B67687/Ithmb-Codec) repo. This plugin is the C ABI glue layer. Changes to decoding behavior belong in the upstream crate, not here.

## For Agents

- This is a thin wrapper — all decode logic is in the `ithmb-core` dependency.
- The `#[repr(C)]` ABI types in `types.rs` must match ImageGlass's C# SDK structs exactly.
- `git commit` uses `-S` (GPG sign). Author date is preserved via `GIT_COMMITTER_DATE`.
- CI enforces `#[deny(clippy::pedantic)]` — run `cargo clippy --fix` before pushing.
- Releases are created by pushing a `v*` tag — CI builds all 3 platforms and publishes.
