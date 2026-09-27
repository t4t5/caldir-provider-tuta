#!/usr/bin/env bash
# Checks that rencal-plugin.toml matches the caldir-core version the binary
# is built with.
set -euo pipefail

cd "$(dirname "$0")/.."

toml_string() {
  grep -m1 "^$1 *=" "$2" | sed 's/.*"\(.*\)".*/\1/'
}

manifest_caldir_core=$(toml_string caldir_core rencal-plugin.toml)
locked_caldir_core=$(awk '
  /^name = "caldir-core"$/ { found = 1; next }
  found && /^version = / { gsub(/"/, "", $3); print $3; exit }
' Cargo.lock)

if [[ "$manifest_caldir_core" != "$locked_caldir_core" ]]; then
  echo "rencal-plugin.toml caldir_core ${manifest_caldir_core} does not match Cargo.lock caldir-core ${locked_caldir_core}" >&2
  exit 1
fi
