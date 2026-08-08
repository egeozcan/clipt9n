#!/usr/bin/env bash
set -euo pipefail

readonly target="${CLIPT9N_LINUX_TARGET:-x86_64-unknown-linux-gnu}"
readonly release_dir="target/$target/release"

cargo build --locked --release --target "$target"
stage="target/release/package-linux/clipt9n"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/icons/hicolor/256x256/apps" "$stage/share/doc/clipt9n"
cp "$release_dir/clipt9n" "$stage/bin/clipt9n"
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
cat > "$stage/share/doc/clipt9n/LINUX-RUNTIME.txt" <<'DOC'
Runtime dependencies:
- xdg-open: opens config and glossary files.
- xdotool: selected-text copy and inline replacement on X11.

Native Wayland selection automation is not supported. Use an X11 session for
selected-text and inline-replacement hotkeys until a portal/compositor adapter
is available. Clipboard-only translation remains available on Wayland.
DOC
echo "$stage"
