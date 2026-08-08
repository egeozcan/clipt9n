#!/usr/bin/env bash
set -euo pipefail

readonly arm_target="aarch64-apple-darwin"
readonly intel_target="x86_64-apple-darwin"

cargo build --locked --release
CARGO_BUNDLE_SKIP_BUILD=1 cargo bundle --release --format osx
cargo build --locked --release --target "$arm_target"
cargo build --locked --release --target "$intel_target"

app_path="$(find target/release/bundle/osx -maxdepth 1 -name 'clipt9n.app' -type d | head -n 1)"
if [[ -z "$app_path" ]]; then
  echo "clipt9n.app not found under target/release/bundle/osx" >&2
  exit 1
fi

lipo -create \
  "target/$arm_target/release/clipt9n" \
  "target/$intel_target/release/clipt9n" \
  -output "$app_path/Contents/MacOS/clipt9n"
lipo -info "$app_path/Contents/MacOS/clipt9n"

plist="$app_path/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Delete :LSUIElement' "$plist" >/dev/null 2>&1 || true
/usr/libexec/PlistBuddy -c 'Add :LSUIElement bool true' "$plist"
/usr/libexec/PlistBuddy -c 'Delete :LSBackgroundOnly' "$plist" >/dev/null 2>&1 || true
/usr/libexec/PlistBuddy -c 'Add :LSBackgroundOnly bool false' "$plist"

# This is an ad-hoc signature for local execution, not a Developer ID signature
# and not evidence of notarization.
codesign --force --deep --sign - "$app_path"
echo "$app_path"
