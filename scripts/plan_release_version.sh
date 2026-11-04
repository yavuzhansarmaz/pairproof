#!/bin/sh
# Decide the version a push to main should release.
#
# Usage: plan_release_version.sh <cargo-version> <latest-release-version>
# The latest release may be empty when the repository has no release yet.
# Prints "<version> keep". A version equal to the latest release is not
# incremented; a release is published only when Cargo.toml names a new one.
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

# Ship the version written in Cargo.toml. If it already matches the latest
# release, the workflow sees the existing tag and does not publish again.
printf '%s keep\n' "$CURRENT"
