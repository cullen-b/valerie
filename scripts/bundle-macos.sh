#!/usr/bin/env bash
# Build Valerie.app so macOS can use it as the default browser (http/https handler).
set -euo pipefail
cd "$(dirname "$0")/.."
# Each build carries the commit it came from (shown in "Valerie updated" and `valerie --version`).
export VALERIE_BUILD="$(git rev-parse --short HEAD 2>/dev/null || echo dev)$(git diff --quiet 2>/dev/null || echo +)"
cargo build --release
APP=target/Valerie.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/valerie "$APP/Contents/MacOS/valerie"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
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
  </array>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>HTML document</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSItemContentTypes</key><array><string>public.html</string><string>public.xhtml</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
# ---- Signing ---------------------------------------------------------------------------------------------------
# Picks the best certificate in your keychain (override with VALERIE_SIGN_IDENTITY):
#   Developer ID Application (paid account; can be notarized and shared) > Apple Development (free account; runs on
#   your Mac) > ad-hoc (no account).
IDENTITY="${VALERIE_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
  IDENTITIES=$(security find-identity -v -p codesigning 2>/dev/null || true)
  IDENTITY=$(echo "$IDENTITIES" | grep -o '"Developer ID Application: [^"]*"' | head -1 | tr -d '"' || true)
  [ -z "$IDENTITY" ] && IDENTITY=$(echo "$IDENTITIES" | grep -o '"Apple Development: [^"]*"' | head -1 | tr -d '"' || true)
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
  codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$APP/Contents/MacOS/valerie"
  codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$APP"
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
