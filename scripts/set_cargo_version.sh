#!/bin/sh
# Rewrite the pairproof package version in Cargo.toml and Cargo.lock.
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
