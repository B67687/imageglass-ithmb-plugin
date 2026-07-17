<div align="center">

# ImageGlass Ithmb Plugin

<a href="./docs/CREDITS.md"><img src="https://cdn.jsdelivr.net/gh/B67687/Imageglass-Ithmb-Plugin@main/docs/badges/deepseek.svg" alt="DeepSeek"></a>
<a href="./docs/CREDITS.md"><img src="https://cdn.jsdelivr.net/gh/B67687/Imageglass-Ithmb-Plugin@main/docs/badges/opencode.svg" alt="OpenCode"></a>
<a href="./docs/CREDITS.md"><img src="https://cdn.jsdelivr.net/gh/B67687/Imageglass-Ithmb-Plugin@main/docs/badges/omo.svg" alt="Oh My OpenAgent"></a>

</div>

C ABI plugin for [ImageGlass](https://imageglass.org) v10 to decode `.ithmb` thumbnail files using [ithmb-core](https://crates.io/crates/ithmb-core) from the parent repo [Ithmb-Codec](https://github.com/B67687/Ithmb-Codec)

## Build & Package

```bash
# Build the cdylib
cargo build --release

# Package as .igplugin.zip (auto-detects host platform)
./scripts/package.sh

# Or specify a target:
# ./scripts/package.sh linux
# ./scripts/package.sh macos
# ./scripts/package.sh windows
```

Output: `dist/ithmb-codec-<platform>.igplugin.zip` (binary + manifest).

## ImageGlass Integration (v10+)

1. Build and package: `./scripts/package.sh`
2. Open ImageGlass v10 -> **Settings -> Plugins -> Add**
3. Select the `.igplugin.zip` file from `dist/`
4. ImageGlass installs and registers the codec automatically

`.ithmb` and `.ipm` files now open natively in ImageGlass.

## Files

| Path | Purpose |
|------|---------|
| `src/` | Rust cdylib (721 lines, ImageGlass native plugin ABI) |
| `igplugin.json` | Plugin manifest (id, name, executable, kind) |
| `scripts/package.sh` | Build + package into `.igplugin.zip` |
| `.github/workflows/ci.yml` | CI: build + clippy + deny + package artifacts |

## FFI from Other Languages

The library exposes a single C entry point:

```c
const IGPluginApi* ig_plugin_get_api(i32 abi_version);
```

See the [ImageGlass plugin SDK](https://github.com/ImageGlass/SDK) for details.

## License

MIT
