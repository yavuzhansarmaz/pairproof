#!/bin/sh
set -eu

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

if [ $# -ne 1 ]; then
  printf 'usage: %s <dir>\n' "$0" >&2
  exit 2
fi

DIR="$1"
[ -d "$DIR" ] || fail "not a directory: $DIR"

EXPECTED_ARCHIVES="pairproof-linux-x86_64-unknown-linux-musl.tar.gz pairproof-linux-aarch64-unknown-linux-musl.tar.gz pairproof-darwin-x86_64-apple-darwin.tar.gz pairproof-darwin-aarch64-apple-darwin.tar.gz"
EXPECTED_FILES="pairproof-linux-x86_64-unknown-linux-musl.tar.gz pairproof-linux-x86_64-unknown-linux-musl.tar.gz.sha256 pairproof-linux-aarch64-unknown-linux-musl.tar.gz pairproof-linux-aarch64-unknown-linux-musl.tar.gz.sha256 pairproof-darwin-x86_64-apple-darwin.tar.gz pairproof-darwin-x86_64-apple-darwin.tar.gz.sha256 pairproof-darwin-aarch64-apple-darwin.tar.gz pairproof-darwin-aarch64-apple-darwin.tar.gz.sha256"

COUNT=0
for ENTRY in "$DIR"/*; do
  [ -e "$ENTRY" ] || continue
  [ -f "$ENTRY" ] || fail "unexpected non-regular entry: $ENTRY"
  COUNT=$((COUNT + 1))
  BASE="${ENTRY##*/}"
  case " $EXPECTED_FILES " in
    *" $BASE "*) ;;
    *) fail "unexpected file: $BASE" ;;
  esac
done
[ "$COUNT" -eq 8 ] || fail "expected exactly 8 files in $DIR, found $COUNT"

for NAME in $EXPECTED_FILES; do
  [ -f "$DIR/$NAME" ] || fail "missing expected file: $NAME"
done

compute_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{ print $1 }'
  else
    fail "neither sha256sum nor shasum is available"
  fi
}

for ARCHIVE in $EXPECTED_ARCHIVES; do
  ARCHIVE_PATH="$DIR/$ARCHIVE"
  CHECKSUM_PATH="$DIR/$ARCHIVE.sha256"
  [ -f "$ARCHIVE_PATH" ] || fail "missing archive: $ARCHIVE"
  [ -f "$CHECKSUM_PATH" ] || fail "missing checksum file: $ARCHIVE.sha256"

  LINES="$(wc -l < "$CHECKSUM_PATH" | tr -d '[:space:]')"
  [ "$LINES" -eq 1 ] || fail "checksum file $ARCHIVE.sha256 must contain exactly one line"

  if ! grep -Eq '^[0-9a-f]{64}  [^ ]+$' "$CHECKSUM_PATH"; then
    fail "checksum file $ARCHIVE.sha256 does not match format '<64 lowercase hex><two spaces><archive filename>'"
  fi

  RECORDED_HEX="$(cut -c 1-64 "$CHECKSUM_PATH")"
  RECORDED_NAME="$(cut -c 67- "$CHECKSUM_PATH")"
  [ "$RECORDED_NAME" = "$ARCHIVE" ] || fail "checksum file $ARCHIVE.sha256 names '$RECORDED_NAME' instead of '$ARCHIVE'"

  ACTUAL_HEX="$(compute_sha256 "$ARCHIVE_PATH")"
  [ "$RECORDED_HEX" = "$ACTUAL_HEX" ] || fail "checksum mismatch for $ARCHIVE: recorded $RECORDED_HEX, recomputed $ACTUAL_HEX"

  LISTING="$(tar -tzf "$ARCHIVE_PATH" 2>/dev/null)" || fail "could not list archive: $ARCHIVE"
  [ "$LISTING" = "pairproof" ] || fail "archive $ARCHIVE must contain exactly 'pairproof' at archive root, found: $LISTING"
done

printf 'ok: verified 8 release assets in %s\n' "$DIR"
