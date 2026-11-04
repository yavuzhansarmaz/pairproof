#!/bin/sh
# Decide the version a push to main should release.
#
# Usage: plan_release_version.sh <cargo-version> <latest-release-version>
# The latest release may be empty when the repository has no release yet.
# Prints "<version> keep" when Cargo.toml already names the version to ship,
# or "<version> bump" when the patch number must increase.
set -eu

CURRENT="${1:?cargo version required}"
LATEST="${2:-}"

semver() {
  printf '%s' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
}

semver "$CURRENT" || {
  printf 'error: cargo version %s is not major.minor.patch\n' "$CURRENT" >&2
  exit 1
}
if [ -n "$LATEST" ]; then
  semver "$LATEST" || {
    printf 'error: latest release %s is not major.minor.patch\n' "$LATEST" >&2
    exit 1
  }
fi

# No release yet, or Cargo.toml was set to something other than the latest
# release: ship the version already written in Cargo.toml.
if [ -z "$LATEST" ] || [ "$CURRENT" != "$LATEST" ]; then
  printf '%s keep\n' "$CURRENT"
  exit 0
fi

MAJOR="${CURRENT%%.*}"
REST="${CURRENT#*.}"
MINOR="${REST%%.*}"
PATCH="${CURRENT##*.}"
printf '%s bump\n' "$MAJOR.$MINOR.$((PATCH + 1))"
