#!/usr/bin/env bash
# Hermetic tests for ../install.sh
#
# Uses a fake `curl` stub (selected via PAIRPROOF_CURL) that serves a local
# API JSON document and release files from a fixture directory, a stub
# `uname` (via PATH) for platform mapping, and a fake HOME / install dir
# inside a temporary directory. No network access is used.
#
# Stub behaviour is controlled by environment:
#   STUB_FIXTURE_DIR  directory containing api.json + assets + .sha256 files
#   STUB_LOG          file that receives one downloaded URL per line
#   STUB_MODE         success (default) | api_fail | download_fail
#   STUB_UNAME_S      value returned by the stub `uname -s`
#   STUB_UNAME_M      value returned by the stub `uname -m`

set -u

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPO_ROOT="$(dirname -- "$SCRIPT_DIR")"
INSTALL_SH="$REPO_ROOT/install.sh"

PASS_COUNT=0
FAIL_COUNT=0
ROOT_TMP="$(mktemp -d)"
trap 'rm -rf "$ROOT_TMP"' EXIT

ASSET_LINUX_X64="pairproof-linux-x86_64-unknown-linux-musl.tar.gz"
ASSET_LINUX_A64="pairproof-linux-aarch64-unknown-linux-musl.tar.gz"
ASSET_DARWIN_X64="pairproof-darwin-x86_64-apple-darwin.tar.gz"
ASSET_DARWIN_A64="pairproof-darwin-aarch64-apple-darwin.tar.gz"
ALL_ASSETS="$ASSET_LINUX_X64 $ASSET_LINUX_A64 $ASSET_DARWIN_X64 $ASSET_DARWIN_A64"

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  else
    shasum -a 256 "$1" | awk '{ print $1 }'
  fi
}

# build_fixture <dir> <variant: good|bad_checksum|invalid_archive|bad_tag>
build_fixture() {
  _dir="$1"; _variant="$2"
  mkdir -p "$_dir/staging"
  cat > "$_dir/staging/pairproof" <<'EOF'
#!/bin/sh
if [ "${1:-}" = "--version" ]; then
  printf 'pairproof 1.2.3\n'
  exit 0
fi
printf 'pairproof fixture\n'
EOF
  chmod +x "$_dir/staging/pairproof"
  for _asset in $ALL_ASSETS; do
    tar -czf "$_dir/$_asset" -C "$_dir/staging" pairproof
    printf '%s  %s\n' "$(sha256_of "$_dir/$_asset")" "$_asset" > "$_dir/$_asset.sha256"
  done
  if [ "$_variant" = "bad_tag" ]; then
    printf '{"tag_name":"not-a-version"}\n' > "$_dir/api.json"
  else
    printf '{"tag_name":"v1.2.3","name":"pairproof v1.2.3"}\n' > "$_dir/api.json"
  fi
  if [ "$_variant" = "bad_checksum" ]; then
    printf '%s  %s\n' "0000000000000000000000000000000000000000000000000000000000000000" \
      "$ASSET_LINUX_X64" > "$_dir/$ASSET_LINUX_X64.sha256"
  fi
  if [ "$_variant" = "invalid_archive" ]; then
    printf 'this is not a tar archive\n' > "$_dir/$ASSET_LINUX_X64"
    printf '%s  %s\n' "$(sha256_of "$_dir/$ASSET_LINUX_X64")" "$ASSET_LINUX_X64" \
      > "$_dir/$ASSET_LINUX_X64.sha256"
  fi
  rm -rf "$_dir/staging"
}

# --- fake curl + fake uname -----------------------------------------------------

FAKEBIN="$ROOT_TMP/fakebin"
mkdir -p "$FAKEBIN"

cat > "$FAKEBIN/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) printf '%s\n' "${STUB_UNAME_S:-Linux}" ;;
  -m) printf '%s\n' "${STUB_UNAME_M:-x86_64}" ;;
  *) printf '%s\n' "${STUB_UNAME_S:-Linux}" ;;
esac
EOF
chmod +x "$FAKEBIN/uname"

STUB_CURL="$ROOT_TMP/fake-curl"
cat > "$STUB_CURL" <<'EOF'
#!/bin/sh
# Fake curl: supports the exact form used by install.sh: curl -fsSL -o <file> <url>
outfile=""
url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) shift; outfile="${1:-}" ;;
    -*) ;;
    *) url="$1" ;;
  esac
  shift
done
if [ -n "${STUB_LOG:-}" ]; then
  printf '%s\n' "$url" >> "$STUB_LOG"
fi
mode="${STUB_MODE:-success}"
fixture="${STUB_FIXTURE_DIR:?STUB_FIXTURE_DIR is not set}"
case "$url" in
  *releases/latest*|*fixture.invalid/api*)
    [ "$mode" = "api_fail" ] && exit 22
    src="$fixture/api.json"
    ;;
  *.tar.gz.sha256)
    src="$fixture/${url##*/}"
    ;;
  *.tar.gz)
    [ "$mode" = "download_fail" ] && exit 22
    src="$fixture/${url##*/}"
    ;;
  *)
    src="$fixture/${url##*/}"
    ;;
esac
[ -f "$src" ] || exit 22
[ -n "$outfile" ] || { cat "$src"; exit 0; }
cp "$src" "$outfile" || exit 1
exit 0
EOF
chmod +x "$STUB_CURL"

# --- per-case harness -------------------------------------------------------------

CASE_DIR=""
FAKE_HOME=""
INSTALL_DIR_CASE=""
OUT_FILE=""
ERR_FILE=""
LOG_FILE=""
STATUS=0
UNAME_S="Linux"
UNAME_M="x86_64"
STUB_MODE_CASE="success"
FIXTURE_DIR_CASE=""
EXTRA_PATH=""

setup_case() {
  CASE_DIR="$ROOT_TMP/case-$1"
  FAKE_HOME="$CASE_DIR/home"
  INSTALL_DIR_CASE="$CASE_DIR/install-bin"
  OUT_FILE="$CASE_DIR/out.txt"
  ERR_FILE="$CASE_DIR/err.txt"
  LOG_FILE="$CASE_DIR/urls.log"
  FIXTURE_DIR_CASE="$CASE_DIR/fixture"
  mkdir -p "$FAKE_HOME"
  : > "$LOG_FILE"
  UNAME_S="Linux"; UNAME_M="x86_64"; STUB_MODE_CASE="success"; EXTRA_PATH=""
}

run_install() {
  if [ -n "$EXTRA_PATH" ]; then
    _path="$FAKEBIN:$EXTRA_PATH:/usr/local/bin:/usr/bin:/bin"
  else
    _path="$FAKEBIN:/usr/local/bin:/usr/bin:/bin"
  fi
  STATUS=0
  env -i \
    HOME="$FAKE_HOME" \
    PATH="$_path" \
    PAIRPROOF_API_URL="https://fixture.invalid/releases/latest" \
    PAIRPROOF_RELEASE_BASE_URL="https://fixture.invalid/download" \
    PAIRPROOF_INSTALL_DIR="$INSTALL_DIR_CASE" \
    PAIRPROOF_CURL="$STUB_CURL" \
    STUB_FIXTURE_DIR="$FIXTURE_DIR_CASE" \
    STUB_LOG="$LOG_FILE" \
    STUB_MODE="$STUB_MODE_CASE" \
    STUB_UNAME_S="$UNAME_S" \
    STUB_UNAME_M="$UNAME_M" \
    bash "$INSTALL_SH" > "$OUT_FILE" 2> "$ERR_FILE" || STATUS=$?
}

place_old_binary() {
  mkdir -p "$INSTALL_DIR_CASE"
  cat > "$INSTALL_DIR_CASE/pairproof" <<'EOF'
#!/bin/sh
printf 'OLD_SENTINEL_BINARY\n'
EOF
  chmod +x "$INSTALL_DIR_CASE/pairproof"
}

old_binary_unchanged() {
  [ -f "$INSTALL_DIR_CASE/pairproof" ] \
    && grep -Fq 'OLD_SENTINEL_BINARY' "$INSTALL_DIR_CASE/pairproof"
}

contains() { grep -Fq "$2" "$1"; }

run_case() {
  _name="$1"; shift
  if "$@"; then
    PASS_COUNT=$((PASS_COUNT + 1))
    printf 'PASS %s\n' "$_name"
  else
    FAIL_COUNT=$((FAIL_COUNT + 1))
    printf 'FAIL %s\n' "$_name"
    if [ -n "${OUT_FILE:-}" ] && [ -f "$OUT_FILE" ]; then
      printf '%s\n' '--- stdout ---'; cat "$OUT_FILE"
      printf '%s\n' '--- stderr ---'; cat "$ERR_FILE"
    fi
  fi
}

# --- cases ------------------------------------------------------------------------

case_linux_x86_64_success() {
  setup_case linux-x64; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] && contains "$LOG_FILE" "$ASSET_LINUX_X64" \
    && [ -x "$INSTALL_DIR_CASE/pairproof" ]
}

case_linux_aarch64_mapping() {
  setup_case linux-a64; UNAME_M="aarch64"; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] && contains "$LOG_FILE" "$ASSET_LINUX_A64" \
    && [ -x "$INSTALL_DIR_CASE/pairproof" ]
}

case_linux_arm64_mapping() {
  setup_case linux-arm64; UNAME_M="arm64"; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] && contains "$LOG_FILE" "$ASSET_LINUX_A64"
}

case_darwin_x86_64_mapping() {
  setup_case darwin-x64; UNAME_S="Darwin"; UNAME_M="x86_64"
  build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] && contains "$LOG_FILE" "$ASSET_DARWIN_X64"
}

case_darwin_arm64_mapping() {
  setup_case darwin-a64; UNAME_S="Darwin"; UNAME_M="arm64"
  build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] && contains "$LOG_FILE" "$ASSET_DARWIN_A64" \
    && [ -x "$INSTALL_DIR_CASE/pairproof" ]
}

case_unsupported_platform() {
  setup_case unsupported; UNAME_S="FreeBSD"; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -ne 0 ] && contains "$ERR_FILE" "unsupported" \
    && [ ! -e "$INSTALL_DIR_CASE/pairproof" ]
}

case_unsupported_arch() {
  setup_case unsupported-arch; UNAME_M="riscv64"; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -ne 0 ] && contains "$ERR_FILE" "unsupported" \
    && [ ! -e "$INSTALL_DIR_CASE/pairproof" ]
}

case_checksum_success() {
  setup_case checksum-ok; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] && contains "$OUT_FILE" "Checksum verified"
}

case_checksum_mismatch_old_preserved() {
  setup_case checksum-bad; build_fixture "$FIXTURE_DIR_CASE" bad_checksum
  place_old_binary; run_install
  [ "$STATUS" -ne 0 ] && contains "$ERR_FILE" "checksum mismatch" && old_binary_unchanged
}

case_download_failure() {
  setup_case download-fail; STUB_MODE_CASE="download_fail"
  build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -ne 0 ] && contains "$ERR_FILE" "failed to download" \
    && [ ! -e "$INSTALL_DIR_CASE/pairproof" ]
}

case_api_failure() {
  setup_case api-fail; STUB_MODE_CASE="api_fail"
  build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -ne 0 ] && [ ! -e "$INSTALL_DIR_CASE/pairproof" ]
}

case_invalid_tag() {
  setup_case bad-tag; build_fixture "$FIXTURE_DIR_CASE" bad_tag; run_install
  [ "$STATUS" -ne 0 ] && contains "$ERR_FILE" "invalid release tag" \
    && [ ! -e "$INSTALL_DIR_CASE/pairproof" ]
}

case_invalid_archive() {
  setup_case invalid-archive; build_fixture "$FIXTURE_DIR_CASE" invalid_archive; run_install
  [ "$STATUS" -ne 0 ] && [ ! -e "$INSTALL_DIR_CASE/pairproof" ]
}

case_successful_install_and_version() {
  setup_case version; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] && contains "$OUT_FILE" "pairproof 1.2.3" \
    && [ "$("$INSTALL_DIR_CASE/pairproof" --version)" = "pairproof 1.2.3" ]
}

case_existing_preserved_on_failure() {
  setup_case preserve-download; STUB_MODE_CASE="download_fail"
  build_fixture "$FIXTURE_DIR_CASE" good; place_old_binary; run_install
  [ "$STATUS" -ne 0 ] && old_binary_unchanged
}

case_path_guidance_when_missing() {
  setup_case path-missing; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] && contains "$OUT_FILE" "not in your PATH" \
    && contains "$OUT_FILE" 'export PATH=' && contains "$OUT_FILE" "fish" \
    && contains "$OUT_FILE" "did NOT modify"
}

case_path_present_no_guidance_needed() {
  setup_case path-present; build_fixture "$FIXTURE_DIR_CASE" good
  EXTRA_PATH="$INSTALL_DIR_CASE"; run_install
  [ "$STATUS" -eq 0 ] && contains "$OUT_FILE" "already in your PATH"
}

case_reinstall_idempotent() {
  setup_case reinstall; build_fixture "$FIXTURE_DIR_CASE" good; run_install
  [ "$STATUS" -eq 0 ] || return 1
  run_install
  [ "$STATUS" -eq 0 ] && [ "$("$INSTALL_DIR_CASE/pairproof" --version)" = "pairproof 1.2.3" ]
}

run_case "platform_linux_x86_64_success" case_linux_x86_64_success
run_case "platform_linux_aarch64_mapping" case_linux_aarch64_mapping
run_case "platform_linux_arm64_mapping" case_linux_arm64_mapping
run_case "platform_darwin_x86_64_mapping" case_darwin_x86_64_mapping
run_case "platform_darwin_arm64_mapping" case_darwin_arm64_mapping
run_case "platform_unsupported_rejected" case_unsupported_platform
run_case "platform_unsupported_arch_rejected" case_unsupported_arch
run_case "checksum_success" case_checksum_success
run_case "checksum_mismatch_old_binary_preserved" case_checksum_mismatch_old_preserved
run_case "download_failure" case_download_failure
run_case "api_failure" case_api_failure
run_case "invalid_tag_rejected" case_invalid_tag
run_case "invalid_archive" case_invalid_archive
run_case "successful_install_and_version" case_successful_install_and_version
run_case "existing_install_preserved_on_failure" case_existing_preserved_on_failure
run_case "path_guidance_when_missing" case_path_guidance_when_missing
run_case "path_present_recognised" case_path_present_no_guidance_needed
run_case "reinstall_idempotent" case_reinstall_idempotent

printf 'summary: %d passed, %d failed\n' "$PASS_COUNT" "$FAIL_COUNT"
[ "$FAIL_COUNT" -eq 0 ]
