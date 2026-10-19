#!/usr/bin/env bash
# Build Valerie.app so macOS can use it as the default browser (http/https handler).
set -euo pipefail
cd "$(dirname "$0")/.."
# Each build carries the commit it came from (shown in "Valerie updated" and `valerie --version`).
export VALERIE_BUILD="$(git rev-parse --short HEAD 2>/dev/null || echo dev)$(git diff --quiet 2>/dev/null || echo +)"
# The version's last number counts commits, so every build from main is newer than the one before (the updater
# compares these). CI checks out the whole history so the count matches a local build of the same commit.
BASE=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2 | cut -d. -f1-2)
export VALERIE_VERSION="${VALERIE_VERSION:-$BASE.$(git rev-list --count HEAD 2>/dev/null || echo 0)}"
VERSION="$VALERIE_VERSION"
APP=target/Valerie.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
if [ -n "${VALERIE_UNIVERSAL:-}" ]; then
  # One app for Apple silicon and Intel Macs (releases).
  cargo build --release --locked --target aarch64-apple-darwin
  cargo build --release --locked --target x86_64-apple-darwin
  lipo -create -output "$APP/Contents/MacOS/valerie" target/aarch64-apple-darwin/release/valerie target/x86_64-apple-darwin/release/valerie
else
  cargo build --release
  cp target/release/valerie "$APP/Contents/MacOS/valerie"
fi
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Valerie</string>
  <key>CFBundleDisplayName</key><string>Valerie</string>
  <key>CFBundleIdentifier</key><string>dev.valerie.browser</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>valerie</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSCameraUsageDescription</key><string>Websites you allow can use the camera, for example for video calls.</string>
  <key>NSMicrophoneUsageDescription</key><string>Websites you allow can use the microphone, for example for video calls.</string>
  <key>NSLocationUsageDescription</key><string>Websites you allow can use your location, for example for maps.</string>
  <key>NSLocationWhenInUseUsageDescription</key><string>Websites you allow can use your location, for example for maps.</string>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>Web site URL</string>
      <key>CFBundleURLSchemes</key><array><string>http</string><string>https</string></array>
    </dict>
    <dict>
      <key>CFBundleURLName</key><string>Valerie</string>
      <key>CFBundleURLSchemes</key><array><string>valerie</string></array>
    </dict>
  </array>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>HTML document</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSItemContentTypes</key><array><string>public.html</string><string>public.xhtml</string></array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Markdown document</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key><array><string>net.daringfireball.markdown</string></array>
    </dict>
  </array>
  <key>UTImportedTypeDeclarations</key>
  <array>
    <dict>
      <key>UTTypeIdentifier</key><string>net.daringfireball.markdown</string>
      <key>UTTypeDescription</key><string>Markdown document</string>
      <key>UTTypeConformsTo</key><array><string>public.plain-text</string></array>
      <key>UTTypeTagSpecification</key>
      <dict>
        <key>public.filename-extension</key><array><string>md</string><string>markdown</string><string>mdown</string><string>mkd</string><string>mkdn</string></array>
        <key>public.mime-type</key><array><string>text/markdown</string></array>
      </dict>
    </dict>
  </array>
</dict>
</plist>
PLIST
# ---- Signing ---------------------------------------------------------------------------------------------------
# Picks the best certificate in your keychain (override with VALERIE_SIGN_IDENTITY):
#   Developer ID Application (paid account; can be notarized and shared) > Valerie Release (the key releases are
#   signed with, so updates keep camera, microphone and keychain permissions; trusted for code signing once) >
#   Apple Development (free account; runs on your Mac) > ad-hoc (no account).
IDENTITY="${VALERIE_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
  IDENTITIES=$(security find-identity -v -p codesigning 2>/dev/null || true)
  IDENTITY=$(echo "$IDENTITIES" | grep -o '"Developer ID Application: [^"]*"' | head -1 | tr -d '"' || true)
  [ -z "$IDENTITY" ] && IDENTITY=$(echo "$IDENTITIES" | grep -o '"Valerie Release"' | head -1 | tr -d '"' || true)
  [ -z "$IDENTITY" ] && IDENTITY=$(echo "$IDENTITIES" | grep -o '"Apple Development: [^"]*"' | head -1 | tr -d '"' || true)
fi
# Apple's certificates get a secure timestamp; Valerie's own release key pins its requirement to that one certificate
# (the updater checks updates against it, see src/update.rs).
SIGN_ARGS=(--timestamp)
if [ -n "$IDENTITY" ] && [[ "$IDENTITY" != Developer\ ID* ]] && [[ "$IDENTITY" != Apple\ Development* ]]; then
  HASH=$(security find-certificate -c "$IDENTITY" -Z 2>/dev/null | awk '/SHA-1 hash:/ {print $3; exit}')
  SIGN_ARGS=(--timestamp=none --requirements="=designated => identifier \"dev.valerie.browser\" and certificate leaf = H\"$HASH\"")
fi

# Passkeys need Apple's web-browser public-key-credential entitlement, granted to paid accounts on request, plus the
# provisioning profile that carries it. Without both, macOS refuses to launch an app claiming the entitlement, so it
# is only added when VALERIE_PROVISIONING_PROFILE points at that profile.
ENTITLEMENTS=$(mktemp -t valerie-entitlements).plist
cp assets/Valerie.entitlements "$ENTITLEMENTS"
if [ -n "${VALERIE_PROVISIONING_PROFILE:-}" ]; then
  cp "$VALERIE_PROVISIONING_PROFILE" "$APP/Contents/embedded.provisionprofile"
  /usr/libexec/PlistBuddy -c "Add :com.apple.developer.web-browser.public-key-credential bool true" "$ENTITLEMENTS"
  echo "Passkeys: entitlement added (profile $VALERIE_PROVISIONING_PROFILE)"
fi

if [ -n "$IDENTITY" ]; then
  codesign --force --options runtime "${SIGN_ARGS[@]}" --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$APP/Contents/MacOS/valerie"
  codesign --force --options runtime "${SIGN_ARGS[@]}" --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$APP"
  echo "Signed with: $IDENTITY"
else
  codesign --force --sign - "$APP" >/dev/null 2>&1 || true
  echo "Signed ad-hoc (no Apple certificate found; see README > Signing)"
  if security find-identity -p codesigning 2>/dev/null | grep -q '"Apple Development\|"Developer ID'; then
    echo "  You have a certificate macOS doesn't trust yet: install Apple's WWDR G3 intermediate,"
    echo "  https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer (double-click it), then run this again."
  fi
fi
codesign --verify --deep --strict "$APP" && echo "Signature verified"

# ---- Notarization (Developer ID only) ----------------------------------------------------------------------------
# Store credentials once with: xcrun notarytool store-credentials valerie --apple-id you@example.com --team-id TEAMID
if [ -n "${VALERIE_NOTARY_PROFILE:-}" ] && [[ "$IDENTITY" == Developer\ ID* ]]; then
  ZIP=target/Valerie-notarize.zip
  ditto -c -k --keepParent "$APP" "$ZIP"
  xcrun notarytool submit "$ZIP" --keychain-profile "$VALERIE_NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
  rm -f "$ZIP"
fi
echo "Built $APP. Move it to /Applications, then pick Valerie in System Settings > Desktop & Dock > Default web browser."
