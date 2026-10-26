#!/usr/bin/env bash
# Assemble the LOCAL macOS development app bundle for ax-shell.
#
# Honest scope (docs/PRD.md, docs/USING-AX-SHELL.md): this is a local
# development bundle, not a beta/release package — the PRD gates beta/release
# distribution on a proven signing/notarization plan, and the owner's host is
# non-Apple hardware (distribution claims require legal review, see
# docs/USER-DEVICE-PROFILE.md). The bundle is ad-hoc signed, carries no icon,
# and accessibility (AX) trust under the bundled identity is NOT
# runtime-tested yet — the proven AX evidence is the CLI binary identity
# (docs/ARENA-VERIFICATION.md). LSMinimumSystemVersion mirrors the pinned
# build convention (MACOSX_DEPLOYMENT_TARGET=26.0); the PRD floor of
# Ventura 13 is not met by the current build and is honestly not claimed here.
#
# Usage (from the repo root, or it cd's itself there):
#   scripts/package-macos.sh
# Env overrides: AX_SHELL_APP_NAME, AX_SHELL_BUNDLE_ID
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="${AX_SHELL_APP_NAME:-ax-shell}"
BUNDLE_ID="${AX_SHELL_BUNDLE_ID:-io.github.niumination.ax-shell}"
BIN_NAME="${AX_SHELL_BIN_NAME:-ax-shell}"
MANIFEST="crates/platform-macos/Cargo.toml"
CRATE_TARGET="crates/platform-macos/target"

if [ "$(uname)" != "Darwin" ]; then
    echo "error: this script assembles a macOS bundle; run it on macOS" >&2
    exit 2
fi

version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$MANIFEST" | head -n 1)"
if [ -z "$version" ]; then
    echo "error: could not read the crate version from $MANIFEST" >&2
    exit 2
fi

MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-26.0}"
export MACOSX_DEPLOYMENT_TARGET
cargo build --manifest-path "$MANIFEST" --release --bin "$BIN_NAME"

bundle="$CRATE_TARGET/dist/${APP_NAME}.app"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS"
install -m 0755 "$CRATE_TARGET/release/$BIN_NAME" "$bundle/Contents/MacOS/$BIN_NAME"

# The accessory key (LSUIElement) matches the menu-bar shell's design: no
# Dock ticket, menu-clock-level presence only.
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>$BIN_NAME</string>
	<key>CFBundleIdentifier</key>
	<string>$BUNDLE_ID</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>$APP_NAME</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>LSMinimumSystemVersion</key>
	<string>$MACOSX_DEPLOYMENT_TARGET</string>
	<key>LSUIElement</key>
	<true/>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
PLIST

plutil -lint "$bundle/Contents/Info.plist"

# Ad-hoc signature only (no identity, no notarization, no ticket): keeps the
# bundle self-consistent on this host, does NOT make it distributable.
if command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - "$bundle"
fi

echo "packaged: $PWD/$bundle"
find "$bundle" -print
