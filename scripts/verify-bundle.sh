#!/usr/bin/env bash
# Check an AINode.app bundle before it ships.
#
# Reports:
#   - code signature: unsigned, ad-hoc, or signed (with the authority chain)
#   - Gatekeeper: the spctl --assess verdict and its source
#   - notarization ticket: xcrun stapler validate
#   - Info.plist: bundle id must be ai.ainode.desktop; version and archs shown
#
# Exit status is 0 when the bundle exists and the bundle id is right. Unsigned
# and Gatekeeper-rejected builds are reported, not failed: an unsigned build is
# a valid release shape here. Pass --require-signed to fail on those too.
#
# Usage:
#   scripts/verify-bundle.sh [path/to/AINode.app] [--require-signed]
# With no path it looks under src-tauri/target for the universal bundle first,
# then the native one.

set -euo pipefail

EXPECTED_BUNDLE_ID="ai.ainode.desktop"

app=""
require_signed=0
for arg in "$@"; do
  case "$arg" in
    --require-signed) require_signed=1 ;;
    -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) app="$arg" ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -z "$app" ]; then
  for dir in \
    "$repo_root/src-tauri/target/universal-apple-darwin/release/bundle/macos" \
    "$repo_root/src-tauri/target/release/bundle/macos" \
    "$repo_root/src-tauri/target/universal-apple-darwin/debug/bundle/macos" \
    "$repo_root/src-tauri/target/debug/bundle/macos"; do
    found="$(find "$dir" -maxdepth 1 -name '*.app' 2>/dev/null | head -n 1 || true)"
    if [ -n "$found" ]; then app="$found"; break; fi
  done
fi
if [ -z "$app" ] || [ ! -d "$app" ]; then
  echo "No .app bundle found. Pass a path: scripts/verify-bundle.sh path/to/AINode.app" >&2
  exit 1
fi
app="${app%/}"
plist="$app/Contents/Info.plist"
if [ ! -f "$plist" ]; then
  echo "FAIL  $app has no Contents/Info.plist; not an app bundle" >&2
  exit 1
fi

failures=0
pass() { printf 'PASS  %s\n' "$1"; }
info() { printf 'INFO  %s\n' "$1"; }
warn() { printf 'WARN  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; failures=$((failures + 1)); }

echo "Bundle: $app"
echo

# --- Info.plist -------------------------------------------------------------
bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist" 2>/dev/null || true)"
version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist" 2>/dev/null || true)"
exe_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist" 2>/dev/null || true)"
min_os="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$plist" 2>/dev/null || true)"

if [ "$bundle_id" = "$EXPECTED_BUNDLE_ID" ]; then
  pass "bundle id is $bundle_id"
else
  fail "bundle id is '${bundle_id:-<missing>}', expected $EXPECTED_BUNDLE_ID"
fi
info "version ${version:-<missing>}, minimum macOS ${min_os:-<unset>}"

exe="$app/Contents/MacOS/$exe_name"
if [ -n "$exe_name" ] && [ -x "$exe" ]; then
  archs="$(lipo -archs "$exe" 2>/dev/null || echo unknown)"
  case "$archs" in
    *arm64*x86_64*|*x86_64*arm64*) pass "universal binary ($archs)" ;;
    *) warn "binary is not universal ($archs)" ;;
  esac
else
  fail "main executable missing: Contents/MacOS/${exe_name:-<missing>}"
fi

# --- Code signature ---------------------------------------------------------
echo
sig_out="$(codesign -dv --verbose=2 "$app" 2>&1 || true)"
if echo "$sig_out" | grep -q "code object is not signed at all"; then
  sig_state="unsigned"
  if [ "$require_signed" -eq 1 ]; then
    fail "unsigned (codesign: code object is not signed at all)"
  else
    warn "unsigned (codesign: code object is not signed at all)"
  fi
elif echo "$sig_out" | grep -q "^Signature=adhoc"; then
  sig_state="adhoc"
  if [ "$require_signed" -eq 1 ]; then
    fail "ad-hoc signature only (no Developer ID)"
  else
    warn "ad-hoc signature only (no Developer ID)"
  fi
elif echo "$sig_out" | grep -q "^Authority="; then
  sig_state="signed"
  authority="$(echo "$sig_out" | grep '^Authority=' | head -n 1 | cut -d= -f2-)"
  team="$(echo "$sig_out" | grep '^TeamIdentifier=' | head -n 1 | cut -d= -f2-)"
  pass "signed by: ${authority}${team:+ (team ${team})}"
  if codesign --verify --deep --strict "$app" >/dev/null 2>&1; then
    pass "codesign --verify --deep --strict"
  else
    fail "codesign --verify --deep --strict failed"
  fi
else
  sig_state="unknown"
  warn "could not read the signature state; codesign said: $(echo "$sig_out" | head -n 1)"
fi
if [ "$sig_state" != "unsigned" ]; then
  runtime="$(echo "$sig_out" | grep '^CodeDirectory' | grep -o 'flags=[^ ]*' || true)"
  [ -n "$runtime" ] && info "codesign $runtime"
fi

# --- Gatekeeper -------------------------------------------------------------
spctl_out="$(spctl --assess --type execute -vv "$app" 2>&1 || true)"
if echo "$spctl_out" | grep -q ": accepted"; then
  source="$(echo "$spctl_out" | grep '^source=' | cut -d= -f2- || true)"
  pass "Gatekeeper accepted${source:+ (source: $source)}"
else
  reason="$(echo "$spctl_out" | head -n 1 | sed "s|^$app: ||")"
  if [ "$require_signed" -eq 1 ]; then
    fail "Gatekeeper rejected: ${reason:-no output}"
  else
    warn "Gatekeeper rejected: ${reason:-no output}. Users will need right-click Open (macOS 14 and earlier) or Privacy and Security, Open Anyway (macOS 15 and later)."
  fi
fi

# --- Notarization ticket ----------------------------------------------------
if [ "$sig_state" = "signed" ]; then
  if xcrun stapler validate "$app" >/dev/null 2>&1; then
    pass "notarization ticket stapled"
  else
    warn "no notarization ticket stapled (stapler validate failed); Gatekeeper will need to reach Apple to check the notarization, or the build was not notarized"
  fi
else
  info "notarization check skipped (bundle is $sig_state)"
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "OK: $failures failures"
  exit 0
fi
echo "FAILED: $failures failure(s)"
exit 1
