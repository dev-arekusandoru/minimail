#!/bin/bash
# Build a macOS .app and a drag-to-install .dmg of the app.
#
# Runs the same locally and in CI. Output: dist/Minimail-<version>-arm64.dmg
#
# Required env:
#   MAIL_CLASSIFIER_GOOGLE_CLIENT_ID / _SECRET   baked into the binary (as
#       MAIL_CLASSIFIER_BUNDLED_GOOGLE_CLIENT_*) so users can connect Gmail.
#       Your .envrc already exports these.
#
# Optional env:
#   CHANNEL                       "nightly" (default) or "release". Nightly gets
#                                 version <cargo version>-nightly.<YYYYMMDD>.
#   VERSION                       override the computed version outright.
#   MAIL_CLASSIFIER_SIGN_IDENTITY codesign identity (name or SHA-1). Default "-"
#                                 (ad-hoc: runs locally, Gatekeeper blocks it elsewhere).
#                                 A real identity also enables the hardened runtime.
#   Notarization (only with a "Developer ID Application" identity). Either:
#     NOTARY_PROFILE              a `xcrun notarytool store-credentials` profile, or
#     APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD
#   With neither, the dmg is signed but not notarized.
set -euo pipefail

cd "$(dirname "$0")/.."

[ "$(uname)" = Darwin ] || { echo "macOS only" >&2; exit 1; }
[ "$(uname -m)" = arm64 ] || { echo "arm64 only" >&2; exit 1; }

: "${MAIL_CLASSIFIER_GOOGLE_CLIENT_ID:?not set (see .envrc)}"
: "${MAIL_CLASSIFIER_GOOGLE_CLIENT_SECRET:?not set (see .envrc)}"

APP_NAME=Minimail
BIN_NAME=mail-classifier
BUNDLE_ID=com.dev-arekusandoru.minimail
CHANNEL="${CHANNEL:-nightly}"
IDENTITY="${MAIL_CLASSIFIER_SIGN_IDENTITY:--}"

base=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | sed -n 1p)
stamp=$(date -u +%Y%m%d)
case "$CHANNEL" in
    nightly) default_version="$base-nightly.$stamp" ;;
    release) default_version="$base" ;;
    *) echo "CHANNEL must be nightly or release" >&2; exit 1 ;;
esac
VERSION="${VERSION:-$default_version}"
# CFBundleVersion must be dotted integers; a date is monotonic for nightlies.
BUILD_NUMBER="${BUILD_NUMBER:-$stamp}"

dist=dist
app="$dist/$APP_NAME.app"
dmg="$dist/$APP_NAME-$VERSION-arm64.dmg"
rm -rf "$dist"
mkdir -p "$app/Contents/MacOS"

echo "==> Building $VERSION"
MAIL_CLASSIFIER_BUNDLED_GOOGLE_CLIENT_ID="$MAIL_CLASSIFIER_GOOGLE_CLIENT_ID" \
MAIL_CLASSIFIER_BUNDLED_GOOGLE_CLIENT_SECRET="$MAIL_CLASSIFIER_GOOGLE_CLIENT_SECRET" \
    cargo build --release --locked
cp "target/release/$BIN_NAME" "$app/Contents/MacOS/$BIN_NAME"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key><string>$BIN_NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
    <key>LSMinimumSystemVersion</key><string>10.15</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

sign_args=(--force --timestamp=none -s "$IDENTITY")
if [ "$IDENTITY" != - ]; then
    sign_args=(--force --timestamp --options runtime -s "$IDENTITY")
fi

echo "==> Signing app ($IDENTITY)"
codesign "${sign_args[@]}" "$app"
codesign --verify --strict --verbose=2 "$app"

echo "==> Creating dmg"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "$APP_NAME" -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"
codesign "${sign_args[@]}" "$dmg"

notary_args=()
if [ -n "${NOTARY_PROFILE:-}" ]; then
    notary_args=(--keychain-profile "$NOTARY_PROFILE")
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_APP_PASSWORD:-}" ]; then
    notary_args=(--apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD")
fi

if [ "$IDENTITY" != - ] && [ ${#notary_args[@]} -gt 0 ]; then
    echo "==> Notarizing"
    xcrun notarytool submit "$dmg" "${notary_args[@]}" --wait
    xcrun stapler staple "$dmg"
else
    echo "==> Skipping notarization (needs a Developer ID identity and notary credentials)"
fi

echo "==> $dmg"
[ -z "${GITHUB_OUTPUT:-}" ] || {
    echo "version=$VERSION" >> "$GITHUB_OUTPUT"
    echo "dmg=$dmg" >> "$GITHUB_OUTPUT"
}
