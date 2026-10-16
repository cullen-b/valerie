#!/usr/bin/env bash
# Build Valerie, sign it, and install it in /Applications. A running Valerie is told an update is ready (it keeps
# your tabs; "Restart Valerie" from the command bar switches to the new version).
set -euo pipefail
cd "$(dirname "$0")/.."
./scripts/bundle-macos.sh
DEST=/Applications/Valerie.app
# ditto copies the signed bundle as-is; the running app keeps using the old files until it restarts.
rm -rf "$DEST.new"
ditto target/Valerie.app "$DEST.new"
rm -rf "$DEST"
mv "$DEST.new" "$DEST"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST" >/dev/null 2>&1 || true
BUILD=$("$DEST/Contents/MacOS/valerie" --version 2>/dev/null || echo "")
echo "Installed $DEST ($BUILD)"
# Tell a running Valerie (if any) that a new version is waiting.
"$DEST/Contents/MacOS/valerie" call run_command '{"command":{"cmd":"update_installed"}}' >/dev/null 2>&1 && echo "Told the running Valerie: restart to update" || true
