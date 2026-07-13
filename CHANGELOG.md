# Changelog

## v1.0.0 (2026-07-13) — Fixed plugin manifest & ABI

### Fixed
- **igplugin.json**: Set correct executable name (was `unset`, now per-platform so ImageGlass can load the native codec)
- **Critical ABI fix**: Rewrote FFI layer to match C# SDK struct layouts exactly
  (IGCodecApi had phantom struct_size/abi_version fields, wrong function signatures,
  entry point signature was missing host API parameter)
- `ig_plugin_get_api` now takes `(hostAbiVersion, hostApi)` per C# signature
- GetCodec returns IGStatus via output pointer instead of direct pointer return
- All codec callbacks now use IGStringRef by value (not by pointer)
- Initialize/Shutdown take no arguments
- Added null animation function pointers to match struct layout
- FreePixelBuffer returns void (was returning IGStatus)
- Capability flags: SupportsMetadata=1, SupportsStaticRaster=1,
  SupportsColorProfiles=0, SupportsAnimation=0
- Fixed codec priority to 200 (was 0, losing selection to Magick.NET)
- Removed broken magic signature check (ithmb has no header magic)
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
