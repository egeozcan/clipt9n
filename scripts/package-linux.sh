#!/usr/bin/env bash
set -euo pipefail

cargo build --release
stage="target/release/package-linux/clipt9n"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/icons/hicolor/256x256/apps"
cp target/release/clipt9n "$stage/bin/clipt9n"
cp assets/icon-256.png "$stage/share/icons/hicolor/256x256/apps/clipt9n.png"
cat > "$stage/share/applications/dev.egecan.clipt9n.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=clipt9n
Comment=Keyboard-driven clipboard translator
Exec=clipt9n
Icon=clipt9n
Categories=Utility;
Terminal=false
DESKTOP
echo "$stage"
