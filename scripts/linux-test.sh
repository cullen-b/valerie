#!/usr/bin/env bash
# Try Valerie's first launch on a clean Linux desktop in a throwaway container. Nothing stays behind but the build cache.
#   scripts/linux-test.sh build                  build the Linux version (ARM on an ARM Mac; cached, later builds are quick)
#   scripts/linux-test.sh up [ubuntu|fedora]     start a clean machine and open Valerie as a new user (KEYRING=off: no keyring)
#   scripts/linux-test.sh shot <name>            save the screen to target/linux-test/<name>.png
#   scripts/linux-test.sh click <x> <y>          click at a screen position
#   scripts/linux-test.sh key <keys>             press keys (xdotool names: Return, ctrl+d)
#   scripts/linux-test.sh type <text>            type text
#   scripts/linux-test.sh sh <command>           run a command on the machine as the user
#   scripts/linux-test.sh down                   throw the machine away
set -euo pipefail
cd "$(dirname "$0")/.."

NAME=valerie-linux-test
OUT=target/linux-test
SCREEN=1280x800
BUILD_IMAGE=valerie-linux-build
LIMITS=(--cpus=6 --memory=6g)

in_machine() { docker exec -u tester -e DISPLAY=:1 -e HOME=/home/tester "$NAME" "$@"; }

machine_image() {
  case "$1" in
    ubuntu) docker build -q -t valerie-linux-ubuntu - <<'EOF'
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends libwebkit2gtk-4.1-0 libgtk-3-0 libxdo3 libsoup-3.0-0 \
    ca-certificates curl xdg-utils xvfb openbox xdotool imagemagick x11-utils fonts-dejavu dbus-x11 xterm gnome-keyring \
  && useradd -m tester
EOF
      ;;
    fedora) docker build -q -t valerie-linux-fedora - <<'EOF'
FROM fedora:42
RUN dnf install -y --setopt=install_weak_deps=False webkit2gtk4.1 gtk3 libsoup3 \
    ca-certificates curl xdg-utils xorg-x11-server-Xvfb openbox xdotool ImageMagick dejavu-sans-fonts dbus-x11 xterm gnome-keyring \
  && useradd -m tester
EOF
      ;;
    *) echo "unknown machine: $1 (ubuntu or fedora)" >&2; exit 2 ;;
  esac
}

case "${1:-}" in
  build)
    docker build -q -t "$BUILD_IMAGE" - <<'EOF'
FROM rust:1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev
EOF
    # The checkout is read-only here; the cache lives in Docker volumes, apart from the Mac's own target folder.
    docker run --rm "${LIMITS[@]}" -v "$PWD:/src:ro" -w /src -e CARGO_TARGET_DIR=/target \
      -v valerie-linux-target:/target -v valerie-linux-cargo:/usr/local/cargo/registry \
      "$BUILD_IMAGE" cargo build --locked
    mkdir -p "$OUT"
    docker run --rm -v valerie-linux-target:/target -v "$PWD/$OUT:/out" "$BUILD_IMAGE" cp /target/debug/valerie /out/valerie
    ls -lh "$OUT/valerie"
    ;;
  up)
    distro="${2:-ubuntu}"
    [ -x "$OUT/valerie" ] || { echo "no build yet: run $0 build" >&2; exit 1; }
    image="$(machine_image "$distro")"
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --rm --name "$NAME" "${LIMITS[@]}" --shm-size=1g --init "$image" sleep infinity >/dev/null
    docker cp "$OUT/valerie" "$NAME:/usr/local/bin/valerie"
    docker exec -d "$NAME" Xvfb :1 -screen 0 "${SCREEN}x24"
    sleep 1
    docker exec -d -u tester -e DISPLAY=:1 -e HOME=/home/tester "$NAME" openbox
    # No VALERIE_* folders: this machine is empty, and the guide only opens for a real first launch.
    docker exec -d -u tester -e DISPLAY=:1 -e HOME=/home/tester "$NAME" \
      dbus-launch --exit-with-session sh -c '[ "'"${KEYRING:-on}"'" = off ] || printf test | gnome-keyring-daemon --unlock --components=secrets >/dev/null 2>&1; valerie >"$HOME/valerie.out" 2>&1'
    echo "up: $distro. Next: $0 shot first"
    ;;
  shot)
    mkdir -p "$OUT"
    in_machine import -window root "/tmp/shot.png"
    docker cp -q "$NAME:/tmp/shot.png" "$OUT/$2.png"
    echo "$OUT/$2.png"
    ;;
  click) in_machine xdotool mousemove "$2" "$3" click 1 ;;
  key) shift; in_machine xdotool key "$@" ;;
  type) in_machine xdotool type --delay 20 "$2" ;;
  sh) in_machine sh -c "$2" ;;
  down) docker rm -f "$NAME" >/dev/null 2>&1 || true; echo "down" ;;
  *) sed -n 2,10p "$0"; exit 2 ;;
esac
