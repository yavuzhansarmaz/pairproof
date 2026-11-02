#!/usr/bin/env bash
# pairproof installer
#
# Downloads the latest stable pairproof release from GitHub, verifies its
# SHA-256 checksum and archive contents, and installs the single `pairproof`
# binary into the install directory. It never escalates privileges and
# never modifies any shell configuration file.
#
# Production defaults (used when no override is set):
#   API:            https://api.github.com/repos/yavuzhansarmaz/pairproof/releases/latest
#   Download base:  https://github.com/yavuzhansarmaz/pairproof/releases/download
#   Download URL:   <download base>/<tag>/<asset>
#   Install dir:    $HOME/.local/bin
#
# Testability hooks (environment overrides; production defaults when unset):
#   PAIRPROOF_API_URL           URL queried for the latest release JSON.
#                               Default: https://api.github.com/repos/yavuzhansarmaz/pairproof/releases/latest
#   PAIRPROOF_RELEASE_BASE_URL  Prefix before /<tag>/<asset>.
#                               Default: https://github.com/yavuzhansarmaz/pairproof/releases/download
#   PAIRPROOF_INSTALL_DIR       Directory the `pairproof` binary is installed to.
#                               Default: $HOME/.local/bin
#   PAIRPROOF_CURL              curl executable (name or path) used for downloads.
#                               Default: curl
#                               All downloads are invoked as:
#                                 "$PAIRPROOF_CURL" -fsSL -o <file> <url>
# Platform is detected with `uname -s` / `uname -m` (tests may stub `uname`
# via PATH). Unsupported platforms abort; they are never guessed.

set -eu
# pipefail is bash-specific; this script always runs under bash.
set -o pipefail

API_URL="${PAIRPROOF_API_URL:-https://api.github.com/repos/yavuzhansarmaz/pairproof/releases/latest}"
RELEASE_BASE_URL="${PAIRPROOF_RELEASE_BASE_URL:-https://github.com/yavuzhansarmaz/pairproof/releases/download}"
INSTALL_DIR="${PAIRPROOF_INSTALL_DIR:-$HOME/.local/bin}"
CURL_BIN="${PAIRPROOF_CURL:-curl}"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

warn() {
  printf 'warning: %s\n' "$*" >&2
}

WORK_DIR=""
INSTALL_TMP=""

cleanup() {
  if [ -n "$INSTALL_TMP" ] && [ -e "$INSTALL_TMP" ]; then
    rm -f "$INSTALL_TMP"
  fi
  if [ -n "$WORK_DIR" ] && [ -d "$WORK_DIR" ]; then
    rm -rf "$WORK_DIR"
  fi
}
trap cleanup EXIT
trap 'exit 1' INT TERM

# --- Platform detection -----------------------------------------------------

OS_RAW="$(uname -s)" || fail "failed to detect operating system with 'uname -s'"
ARCH_RAW="$(uname -m)" || fail "failed to detect machine architecture with 'uname -m'"

OS_PART=""
case "$OS_RAW" in
  Linux) OS_PART="linux" ;;
  Darwin) OS_PART="darwin" ;;
  *) fail "unsupported operating system '$OS_RAW': supported systems are Linux and Darwin (macOS)" ;;
esac

ARCH_PART=""
case "$ARCH_RAW" in
  x86_64) ARCH_PART="x86_64" ;;
  aarch64|arm64) ARCH_PART="aarch64" ;;
  *) fail "unsupported architecture '$ARCH_RAW' on $OS_RAW: supported architectures are x86_64 and aarch64/arm64" ;;
esac

ASSET=""
case "$OS_PART-$ARCH_PART" in
  linux-x86_64) ASSET="pairproof-linux-x86_64-unknown-linux-musl.tar.gz" ;;
  linux-aarch64) ASSET="pairproof-linux-aarch64-unknown-linux-musl.tar.gz" ;;
  darwin-x86_64) ASSET="pairproof-darwin-x86_64-apple-darwin.tar.gz" ;;
  darwin-aarch64) ASSET="pairproof-darwin-aarch64-apple-darwin.tar.gz" ;;
  *) fail "unsupported platform '$OS_RAW/$ARCH_RAW'" ;;
esac

# --- Prerequisites ------------------------------------------------------------

command -v "$CURL_BIN" >/dev/null 2>&1 || fail "curl not found ('$CURL_BIN'): please install curl and re-run this installer"
command -v tar >/dev/null 2>&1 || fail "tar not found: please install tar and re-run this installer"
if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
  fail "neither sha256sum nor shasum is available: please install one of them and re-run this installer"
fi

compute_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  else
    shasum -a 256 "$1" | awk '{ print $1 }'
  fi
}

# --- Resolve latest stable tag -------------------------------------------------

WORK_DIR="$(mktemp -d)" || fail "failed to create a temporary directory"
API_JSON="$WORK_DIR/release.json"

printf 'Fetching latest pairproof release information...\n'
"$CURL_BIN" -fsSL -o "$API_JSON" "$API_URL" \
  || fail "failed to fetch latest release information from $API_URL"

# Extract tag_name with sed only (no jq dependency).
TAG="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$API_JSON" | head -n 1)"
[ -n "$TAG" ] || fail "could not find tag_name in release information returned by $API_URL"
printf '%s' "$TAG" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' \
  || fail "invalid release tag '$TAG': expected a tag matching ^v[0-9]+\.[0-9]+\.[0-9]+$"

printf 'Latest stable release: %s\n' "$TAG"
printf 'Platform: %s/%s -> %s\n' "$OS_RAW" "$ARCH_RAW" "$ASSET"

# --- Download archive and checksum (before touching the install directory) -----

ARCHIVE_PATH="$WORK_DIR/$ASSET"
CHECKSUM_PATH="$WORK_DIR/$ASSET.sha256"
ARCHIVE_URL="$RELEASE_BASE_URL/$TAG/$ASSET"
CHECKSUM_URL="$ARCHIVE_URL.sha256"

printf 'Downloading %s\n' "$ARCHIVE_URL"
"$CURL_BIN" -fsSL -o "$ARCHIVE_PATH" "$ARCHIVE_URL" \
  || fail "failed to download archive from $ARCHIVE_URL"

printf 'Downloading %s\n' "$CHECKSUM_URL"
"$CURL_BIN" -fsSL -o "$CHECKSUM_PATH" "$CHECKSUM_URL" \
  || fail "failed to download checksum from $CHECKSUM_URL"

# --- Verify checksum (strict format, before touching the install directory) ---

CHECKSUM_LINES="$(wc -l < "$CHECKSUM_PATH" | tr -d '[:space:]')"
[ "$CHECKSUM_LINES" -eq 1 ] \
  || fail "checksum file $ASSET.sha256 must contain exactly one line, found $CHECKSUM_LINES"

grep -Eq '^[0-9a-f]{64}  [^ ]+$' "$CHECKSUM_PATH" \
  || fail "checksum file $ASSET.sha256 does not match format '<64 hex><two spaces><filename>'"

RECORDED_HEX="$(cut -c 1-64 "$CHECKSUM_PATH")"
RECORDED_NAME="$(cut -c 67- "$CHECKSUM_PATH")"
# Tolerate a trailing carriage return in the recorded filename only.
RECORDED_NAME="$(printf '%s' "$RECORDED_NAME" | tr -d '\r')"
[ "$RECORDED_NAME" = "$ASSET" ] \
  || fail "checksum file $ASSET.sha256 names '$RECORDED_NAME' instead of '$ASSET'"

ACTUAL_HEX="$(compute_sha256 "$ARCHIVE_PATH")"
RECORDED_HEX_LOWER="$(printf '%s' "$RECORDED_HEX" | tr '[:upper:]' '[:lower:]')"
ACTUAL_HEX_LOWER="$(printf '%s' "$ACTUAL_HEX" | tr '[:upper:]' '[:lower:]')"
[ "$RECORDED_HEX_LOWER" = "$ACTUAL_HEX_LOWER" ] \
  || fail "checksum mismatch for $ASSET: recorded $RECORDED_HEX_LOWER, computed $ACTUAL_HEX_LOWER (aborting; any existing install is untouched)"

printf 'Checksum verified: %s\n' "$ASSET"

# --- Validate and extract the archive (in the temporary directory only) -------

if ! LISTING="$(tar -tzf "$ARCHIVE_PATH" 2>/dev/null)"; then
  fail "could not list archive $ASSET: the downloaded file is not a valid gzip tar archive"
fi
[ "$LISTING" = "pairproof" ] \
  || fail "archive $ASSET must contain exactly 'pairproof' at the archive root, found: $LISTING"

EXTRACT_DIR="$WORK_DIR/extract"
mkdir -p "$EXTRACT_DIR" || fail "failed to create extraction directory"
tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR" \
  || fail "failed to extract archive $ASSET"
[ -f "$EXTRACT_DIR/pairproof" ] \
  || fail "archive $ASSET did not contain a regular file named 'pairproof'"
chmod +x "$EXTRACT_DIR/pairproof" || fail "failed to make the extracted pairproof binary executable"

# --- Install (atomic-ish: same-directory temp file + mv) -----------------------

mkdir -p "$INSTALL_DIR" || fail "failed to create install directory $INSTALL_DIR"
INSTALL_TMP="$INSTALL_DIR/.pairproof.tmp.$$"
cp "$EXTRACT_DIR/pairproof" "$INSTALL_TMP" \
  || fail "failed to copy pairproof into install directory $INSTALL_DIR"
chmod 0755 "$INSTALL_TMP" || fail "failed to set permissions on the staged pairproof binary"
mv -f "$INSTALL_TMP" "$INSTALL_DIR/pairproof" \
  || fail "failed to install pairproof into $INSTALL_DIR"
INSTALL_TMP=""

# --- Verify the installed binary ------------------------------------------------

if ! VERSION_OUTPUT="$("$INSTALL_DIR/pairproof" --version)"; then
  fail "installed binary '$INSTALL_DIR/pairproof --version' failed to run successfully"
fi
printf 'Installed pairproof %s to %s\n' "$TAG" "$INSTALL_DIR/pairproof"
printf '%s\n' "$VERSION_OUTPUT"

# --- PATH guidance (never modifies shell configuration files) -------------------

case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    printf '%s is already in your PATH.\n' "$INSTALL_DIR"
    ;;
  *)
    case "$INSTALL_DIR" in
      "$HOME/.local/bin") PATH_DISPLAY='$HOME/.local/bin' ;;
      *) PATH_DISPLAY="$INSTALL_DIR" ;;
    esac
    printf '%s is not in your PATH.\n' "$INSTALL_DIR"
    printf 'Add it to your PATH (the installer did NOT modify any shell configuration files):\n'
    printf '  bash/zsh: add this line to ~/.bashrc or ~/.zshrc, then start a new shell:\n'
    printf '    export PATH="%s:$PATH"\n' "$PATH_DISPLAY"
    printf '  fish: run this once:\n'
    printf '    set -gx PATH "%s" $PATH\n' "$PATH_DISPLAY"
    printf 'Or run pairproof by its full path: %s\n' "$INSTALL_DIR/pairproof"
    ;;
esac

# --- Optional companion tools (warn only, never install anything) ----------------

if ! command -v docker >/dev/null 2>&1; then
  warn "docker was not found in PATH. pairproof sandboxes use Docker; install Docker separately if you need sandbox features. This installer will not install Docker."
fi
if ! command -v mutagen >/dev/null 2>&1; then
  warn "mutagen was not found in PATH. Install Mutagen separately if you need file synchronization features. This installer will not install Mutagen."
fi

printf 'Done. pairproof %s is installed at %s\n' "$TAG" "$INSTALL_DIR/pairproof"
