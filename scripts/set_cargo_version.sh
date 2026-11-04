#!/bin/sh
# Rewrite the pairproof package version in Cargo.toml, Cargo.lock, the
# release-workflow version pin test, and the README status line.
set -eu

VERSION="${1:?version required}"
printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || {
  printf 'error: invalid version %s\n' "$VERSION" >&2
  exit 1
}

ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"

awk -v ver="$VERSION" '
  BEGIN { done = 0 }
  !done && /^version[[:space:]]*=/ {
    print "version = \"" ver "\""
    done = 1
    next
  }
  { print }
' "$ROOT/Cargo.toml" > "$ROOT/Cargo.toml.tmp"
mv "$ROOT/Cargo.toml.tmp" "$ROOT/Cargo.toml"

awk -v ver="$VERSION" '
  $0 == "name = \"pairproof\"" {
    print
    if (getline <= 0) exit 1
    print "version = \"" ver "\""
    next
  }
  { print }
' "$ROOT/Cargo.lock" > "$ROOT/Cargo.lock.tmp"
mv "$ROOT/Cargo.lock.tmp" "$ROOT/Cargo.lock"

PIN_TEST="$ROOT/tests/release_workflow.rs"
awk -v ver="$VERSION" '
  BEGIN { updated = 0 }
  /^[[:space:]]*cargo_version, "[0-9]+\.[0-9]+\.[0-9]+",[[:space:]]*$/ {
    indent = $0
    sub(/cargo_version.*/, "", indent)
    print indent "cargo_version, \"" ver "\","
    updated = 1
    next
  }
  { print }
  END {
    if (!updated) {
      print "error: did not find cargo_version pin assertion in tests/release_workflow.rs" > "/dev/stderr"
      exit 1
    }
  }
' "$PIN_TEST" > "$PIN_TEST.tmp"
mv "$PIN_TEST.tmp" "$PIN_TEST"

README="$ROOT/README.md"
awk -v ver="$VERSION" '
  BEGIN { updated = 0 }
  {
    line = $0
    if (match(line, /Early open-source \(`[0-9]+\.[0-9]+\.[0-9]+`\)/)) {
      line = substr(line, 1, RSTART - 1) "Early open-source (`" ver "`)" substr(line, RSTART + RLENGTH)
      updated = 1
    }
    print line
  }
  END {
    if (!updated) {
      print "error: did not find Early open-source version in README.md" > "/dev/stderr"
      exit 1
    }
  }
' "$README" > "$README.tmp"
mv "$README.tmp" "$README"
