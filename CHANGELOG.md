# Changelog

## v1.0.0 (2026-07-12)

Initial ImageGlass v10 native codec plugin for decoding Apple `.ithmb` thumbnail files.

### Features
- Decodes `.ithmb` and `.ipm` files natively in ImageGlass v10
- 54 SIMD-optimized processing profiles via ithmb-core
- Supports cancellation, multi-frame, and thread-safe decoding
- Cross-platform: Windows, macOS, Linux

### Packaging
- `.igplugin.zip` format for ImageGlass v10 plugin manager
- Install via Settings -> Plugins -> Add
- Pre-compiled binaries for all 3 platforms in GitHub Releases
