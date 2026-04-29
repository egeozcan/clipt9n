#!/usr/bin/env bash
set -euo pipefail

fail=0

check_pattern() {
  local pattern="$1"
  local label="$2"
  local output

  if output=$(grep -rn "$pattern" src/ | grep -v '^src/platform/'); then
    printf 'Platform discipline violation: %s outside src/platform/\n' "$label" >&2
    printf '%s\n' "$output" >&2
    fail=1
  fi
}

check_pattern '^[[:space:]]*#\[cfg(target_os' '#[cfg(target_os = ...)]'
check_pattern '^[[:space:]]*#\[cfg(unix' '#[cfg(unix)]'
check_pattern '^[[:space:]]*#\[cfg(not(unix' '#[cfg(not(unix))]'

exit "$fail"
