#!/usr/bin/env bash
set -euo pipefail

cargo bundle --release --format osx

app_path="$(find target/release/bundle/osx -maxdepth 1 -name 'clipt9n.app' -type d | head -n 1)"
if [[ -z "$app_path" ]]; then
  echo "clipt9n.app not found under target/release/bundle/osx" >&2
  exit 1
fi

plist="$app_path/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Delete :LSUIElement' "$plist" >/dev/null 2>&1 || true
/usr/libexec/PlistBuddy -c 'Add :LSUIElement bool true' "$plist"
/usr/libexec/PlistBuddy -c 'Delete :LSBackgroundOnly' "$plist" >/dev/null 2>&1 || true
/usr/libexec/PlistBuddy -c 'Add :LSBackgroundOnly bool false' "$plist"

codesign --force --deep --sign - "$app_path"
echo "$app_path"
