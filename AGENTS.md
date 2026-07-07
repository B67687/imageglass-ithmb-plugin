# AGENTS.md — AI Agent Guide for Imageglass-Ithmb-Plugin

This file tells AI coding agents how to work with this repository. Read this first before editing any code.

## Repository Purpose

C ABI shared library plugin for [ImageGlass v10](https://imageglass.org) that enables decoding Apple `.ithmb` thumbnail files. This is a **thin FFI wrapper** around the core decoding library — all decode logic lives in the ithmb-core crate, not here.

## Architecture

```
ImageGlass (Windows)
    ↓ ig_plugin_get_api()
Imageglass-Ithmb-Plugin (C ABI cdylib)
    ↓ FFI
ithmb-core (Rust library, loaded as shared lib)
    ↔ 7 decoders, 54 profiles, PhotoDB
```

This repo contains:
- `src/` — C ABI entry point (implements ImageGlass v10 native plugin ABI)
- `Cargo.toml` — declares dependency on `ithmb-core` via git

## Key Facts

- **Language**: Rust (cdylib)
- **ABI**: ImageGlass v10 native codec plugin (v1.0.0.0)
- **Platform**: Windows-only (ImageGlass requirement)
- **Build**: `cargo build --release` with MinGW or MSVC toolchain
- **Dependency**: [`ithmb-core`](https://github.com/B67687/Ithmb-Codec) (the actual codec)

## Relationship to Ithmb-Codec

All decoding logic is in the main [`Ithmb-Codec`](https://github.com/B67687/Ithmb-Codec) repo. This plugin is just the C ABI glue layer. Changes to decoding behavior belong in the upstream crate, not here.

## For Agents

- This is a thin wrapper — almost all relevant code is in the `ithmb-core` dependency.
- See the main repo's `AGENTS.md` and `ARCHITECTURE.md` for the full picture.
- Build requires either `mingw-w64` (cross-compile from Linux) or MSVC (native Windows).
- Only modify this repo if the ImageGlass plugin ABI changes.
