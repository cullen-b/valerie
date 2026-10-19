#!/bin/bash
# Install or update Valerie on a Mac:
#   curl -fsSL https://cullen-b.github.io/valerie/install.sh | bash
# Downloads the latest release, checks its checksum and that it's signed with Valerie's release key, and puts it in
# /Applications (or ~/Applications). After that Valerie keeps itself up to date.
set -euo pipefail
REPO="cullen-b/valerie"
# Must match RELEASE_REQUIREMENTS in src/update.rs.
REQUIREMENT='identifier "dev.valerie.browser" and certificate leaf = H"C69D28F44851570281196E2FC1C24BDCB9B9B6D1"'

if [ "$(uname)" != "Darwin" ]; then
  echo "This installs the Mac app. For Linux and Windows, download Valerie from https://github.com/${REPO}/releases/latest"
  exit 1
fi
TMP=$(mktemp -d)
trap 'rm -rf "${TMP}"' EXIT

echo "Looking up the latest Valerie…"
curl -fsSL "https://github.com/${REPO}/releases/latest/download/latest.json" -o "${TMP}/latest.json"
VERSION=$(plutil -extract version raw -o - "${TMP}/latest.json")
URL=$(plutil -extract assets.macos.url raw -o - "${TMP}/latest.json")
SHA=$(plutil -extract assets.macos.sha256 raw -o - "${TMP}/latest.json")

echo "Downloading Valerie ${VERSION}…"
curl -fL --progress-bar "${URL}" -o "${TMP}/Valerie.zip"
if [ "$(shasum -a 256 "${TMP}/Valerie.zip" | cut -d' ' -f1)" != "${SHA}" ]; then
  echo "The download was damaged (its checksum doesn't match). Nothing was installed."
  exit 1
fi
ditto -x -k "${TMP}/Valerie.zip" "${TMP}/unpacked"
if ! codesign --verify --deep --strict "${TMP}/unpacked/Valerie.app" 2>/dev/null ||
   ! codesign --verify --test-requirement="=${REQUIREMENT}" "${TMP}/unpacked/Valerie.app" 2>/dev/null; then
  echo "This download isn't signed with Valerie's release key. Nothing was installed."
  exit 1
fi

# VALERIE_INSTALL_DIR puts it somewhere else (tests use it).
DEST="${VALERIE_INSTALL_DIR:-/Applications}"
[ -w "${DEST}" ] || { DEST="${HOME}/Applications"; mkdir -p "${DEST}"; }
if [ -d "${DEST}/Valerie.app" ]; then
  rm -rf "${DEST}/Valerie.app.old"
  mv "${DEST}/Valerie.app" "${DEST}/Valerie.app.old"
fi
mv "${TMP}/unpacked/Valerie.app" "${DEST}/Valerie.app"
rm -rf "${DEST}/Valerie.app.old"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "${DEST}/Valerie.app" >/dev/null 2>&1 || true

echo "Installed Valerie ${VERSION} in ${DEST}."

# The valerie command for terminals (valerie screenshot, valerie call…): a link in the first folder on PATH it may
# write to, never over another program called valerie. ~/.local/bin is added to the shell's PATH if it isn't there.
APP_BIN="${DEST}/Valerie.app/Contents/MacOS/valerie"
LINKED=""
for dir in /opt/homebrew/bin /usr/local/bin "${HOME}/.local/bin"; do
  [ "${dir}" = "${HOME}/.local/bin" ] && mkdir -p "${dir}"
  { [ -d "${dir}" ] && [ -w "${dir}" ]; } || continue
  if [ -e "${dir}/valerie" ] && [ ! -L "${dir}/valerie" ]; then continue; fi
  ln -sfn "${APP_BIN}" "${dir}/valerie" && LINKED="${dir}" && break
done
if [ -n "${LINKED}" ]; then
  case ":${PATH}:" in
    *":${LINKED}:"*) ;;
    *)
      PROFILE="${HOME}/.zprofile"
      [ "${SHELL##*/}" = "bash" ] && PROFILE="${HOME}/.bash_profile"
      LINE="export PATH=\"${LINKED/#${HOME}/\$HOME}:\$PATH\""
      grep -qsF "${LINE}" "${PROFILE}" || printf '\n# Added by the Valerie installer, for the valerie command\n%s\n' "${LINE}" >> "${PROFILE}"
      ;;
  esac
  echo "The valerie command works in new terminal windows (try: valerie screenshot --full page.png)."
fi
if pgrep -xq valerie; then
  echo "Valerie is open: the new version starts next time you open it (or run Restart Valerie from the command bar)."
else
  open "${DEST}/Valerie.app"
fi
