#!/bin/sh
# Decide the version a push to main should release.
#
# Usage: plan_release_version.sh <cargo-version> <latest-release-version>
# The latest release may be empty when the repository has no release yet.
# Prints "<version> keep" when Cargo.toml already names the version to ship,
# or "<version> bump" when the patch number must increase (from latest, so a
# Cargo.toml that lagged a published tag still advances past that tag).
set -eu

CURRENT="${1:?cargo version required}"
LATEST="${2:-}"

semver() {
  printf '%s' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
}

# True when $1 is strictly greater than $2 under semver ordering.
version_gt() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]
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

# No release yet: ship the version already written in Cargo.toml.
if [ -z "$LATEST" ]; then
  printf '%s keep\n' "$CURRENT"
  exit 0
fi

# Cargo.toml is ahead of the latest release (manual bump on main): ship it.
if version_gt "$CURRENT" "$LATEST"; then
  printf '%s keep\n' "$CURRENT"
  exit 0
fi

# Equal to latest, or behind a tag that never landed in Cargo.toml: patch-bump
# from the published latest so the next release is always a free version.
MAJOR="${LATEST%%.*}"
REST="${LATEST#*.}"
MINOR="${REST%%.*}"
PATCH="${LATEST##*.}"
printf '%s bump\n' "$MAJOR.$MINOR.$((PATCH + 1))"
