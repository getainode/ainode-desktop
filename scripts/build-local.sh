#!/usr/bin/env bash
# Build the AINode macOS app on this Mac and print where the .app landed.
#
# Default: a universal (arm64 + x86_64) build, the same shape CI ships. That
# needs rustup with both Apple targets installed; the script adds them if
# rustup is present. Pass --native to build for this machine's architecture
# only (works with a Homebrew rustc, no rustup needed).
#
# Signing is optional here exactly as in CI: export APPLE_SIGNING_IDENTITY
# (and, for notarization, APPLE_API_KEY, APPLE_API_ISSUER and
# APPLE_API_KEY_PATH, or the Apple ID trio APPLE_ID, APPLE_PASSWORD,
# APPLE_TEAM_ID) before running and Tauri signs; leave them unset and the app
# builds unsigned. See docs/RELEASING.md for where the material lives.
#
# Usage:
#   scripts/build-local.sh            # universal
#   scripts/build-local.sh --native   # host arch only
#   scripts/build-local.sh --debug    # faster, unoptimised build (either mode)

set -euo pipefail

usage() {
  sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

mode="universal"
profile_args=()
for arg in "$@"; do
  case "$arg" in
    --native) mode="native" ;;
    --universal) mode="universal" ;;
    --debug) profile_args+=(--debug) ;;
    -h|--help) usage 0 ;;
    *) echo "unknown option: $arg" >&2; usage 2 ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This script builds a macOS bundle and has to run on a Mac." >&2
  exit 1
fi

for tool in npm cargo rustc; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing $tool. Install Node 22 and Rust stable first." >&2
    exit 1
  fi
done

if [ ! -f src-tauri/tauri.conf.json ]; then
  echo "src-tauri/tauri.conf.json not found; run this from the ainode-desktop checkout." >&2
  exit 1
fi

target_args=()
bundle_dir="src-tauri/target/release/bundle"
if [ "$mode" = "universal" ]; then
  if ! command -v rustup >/dev/null 2>&1; then
    cat >&2 <<'EOF'
A universal build needs rustup with both Apple targets, and rustup is not on
this machine (a Homebrew rustc only carries the host target).

  Install rustup:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  Then re-run this script, or build for this Mac only:  scripts/build-local.sh --native
EOF
    exit 1
  fi
  echo "==> Ensuring Apple targets are installed"
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  target_args=(--target universal-apple-darwin)
  bundle_dir="src-tauri/target/universal-apple-darwin/release/bundle"
fi
if [ "${#profile_args[@]}" -gt 0 ]; then
  bundle_dir="${bundle_dir/\/release\//\/debug\/}"
fi

if [ ! -d node_modules ]; then
  echo "==> Installing npm dependencies"
  npm ci
fi

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  echo "==> Signing with: ${APPLE_SIGNING_IDENTITY}"
  if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    echo "==> Notarization: on (Apple ID)"
  elif [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ] && [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    echo "==> Notarization: on (App Store Connect API key)"
  else
    echo "==> Notarization: off (set APPLE_API_KEY, APPLE_API_ISSUER and APPLE_API_KEY_PATH, or APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID)"
  fi
else
  echo "==> Signing: off (APPLE_SIGNING_IDENTITY not set), the app will be unsigned"
fi

echo "==> Building (${mode})"
npm run tauri build -- ${target_args[@]+"${target_args[@]}"} ${profile_args[@]+"${profile_args[@]}"}

app="$(find "${bundle_dir}/macos" -maxdepth 1 -name '*.app' 2>/dev/null | head -n 1 || true)"
if [ -z "$app" ]; then
  echo "Build finished but no .app was found under ${bundle_dir}/macos" >&2
  exit 1
fi
dmg="$(find "${bundle_dir}/dmg" -maxdepth 1 -name '*.dmg' 2>/dev/null | head -n 1 || true)"

echo
echo "App:  ${repo_root}/${app}"
if [ -n "$dmg" ]; then
  echo "DMG:  ${repo_root}/${dmg}"
fi
exe="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "${app}/Contents/Info.plist")"
echo "Arch: $(lipo -archs "${app}/Contents/MacOS/${exe}")"
echo
echo "Check it with: scripts/verify-bundle.sh \"${repo_root}/${app}\""
