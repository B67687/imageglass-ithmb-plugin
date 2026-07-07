#!/usr/bin/env bash
set -euo pipefail

# Package the ImageGlass-ITHMB plugin as a .igplugin.zip
#
# Usage: ./scripts/package.sh [linux|macos|windows]
#
# Produces: dist/ithmb-codec-<target>.igplugin.zip
#
# The zip contains the compiled cdylib binary and igplugin.json manifest.
# Install in ImageGlass v10: Settings -> Plugins -> Add -> select the .igplugin.zip

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$REPO_ROOT/dist"

# ---- Platform ----
TARGET="${1:-$(rustc -vV | grep 'host:' | awk '{print $2}')}"

case "$TARGET" in
    *linux*)
        BINARY="libithmb_core_cabi.so"
        PKG_TAG="linux"
        ;;
    *apple*|*macos*|*darwin*)
        BINARY="libithmb_core_cabi.dylib"
        PKG_TAG="macos"
        ;;
    *windows*|*msvc*|*mingw*)
        BINARY="ithmb_core_cabi.dll"
        PKG_TAG="windows"
        ;;
    *)
        echo "Unknown target: $TARGET"
        echo "Usage: $0 [linux|macos|windows]"
        exit 1
        ;;
esac

ARCHIVE_NAME="ithmb-codec-${PKG_TAG}.igplugin.zip"

echo "Building for $TARGET..."
cargo build --release

echo "Packaging $ARCHIVE_NAME..."
mkdir -p "$DIST_DIR"

# Use a fixed relative temp dir (avoids mktemp Unix/Win path issues)
PKG_DIR="$DIST_DIR/_pkg"
rm -rf "$PKG_DIR" && mkdir -p "$PKG_DIR"

# Generate igplugin.json with the correct executable name inline
cat > "$PKG_DIR/igplugin.json" << IGMANIFEST
{
  "id": "Plugin_IthmbCodec",
  "name": "iThmb Codec",
  "description": "Native codec plugin that decodes Apple .ithmb thumbnail files (iPod/iPhone thumbnail databases) into viewable images in ImageGlass v10.",
  "version": "1.0.0",
  "author": "Nacai",
  "website": "https://github.com/B67687/Imageglass-Ithmb-Plugin",
  "kind": "Codec",
  "executable": "${BINARY}"
}
IGMANIFEST

# Copy binary
cp "$REPO_ROOT/target/release/$BINARY" "$PKG_DIR/"

# Create .igplugin.zip
cd "$PKG_DIR"
case "$PKG_TAG" in
    windows)
        # Use PowerShell Compress-Archive with relative paths
        pwsh.exe -NoProfile -Command "Compress-Archive -Path '*' -DestinationPath '../$ARCHIVE_NAME'" 2>/dev/null || \
        powershell.exe -NoProfile -Command "Compress-Archive -Path '*' -DestinationPath '../$ARCHIVE_NAME'"
        ;;
    *)
        zip -r "../$ARCHIVE_NAME" . -x ".*" > /dev/null 2>&1
        ;;
esac

cd "$REPO_ROOT"
rm -rf "$PKG_DIR"

echo ""
echo "Done -> $DIST_DIR/$ARCHIVE_NAME"
echo "Install: Settings -> Plugins -> Add -> select ithmb-codec-${PKG_TAG}.igplugin.zip"
