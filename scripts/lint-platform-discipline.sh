#!/usr/bin/env bash
set -euo pipefail

fail=0

check_pattern() {
  local pattern="$1"
  local label="$2"
  local output

  if output=$(grep -rnF "$pattern" src/ | grep -v '^src/platform/'); then
    printf 'Platform discipline violation: %s outside src/platform/\n' "$label" >&2
    printf '%s\n' "$output" >&2
    fail=1
  fi
}

check_pattern '#[cfg(target_os' '#[cfg(target_os = ...)]'
check_pattern '#[cfg(unix' '#[cfg(unix)]'
check_pattern '#[cfg(not(unix' '#[cfg(not(unix))]'

exit "$fail"
