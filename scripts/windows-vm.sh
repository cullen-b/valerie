#!/usr/bin/env bash
# A throwaway Windows 11 ARM machine on an Apple Silicon Mac, for trying Valerie as a new user. It only uses CPU and memory while up.
#   scripts/windows-vm.sh install <win11-arm64.iso>  build the clean machine, hands-free (about 30 minutes, once)
#   scripts/windows-vm.sh up                         start a fresh copy of the clean machine; earlier changes are gone
#   scripts/windows-vm.sh try [valerie.zip]          put Valerie (default: latest edge build) in Downloads, as if from a browser, and open it
#   scripts/windows-vm.sh shot <name>                save the screen to target/windows-vm/<name>.png
#   scripts/windows-vm.sh click <x> <y>              click at a screen position
#   scripts/windows-vm.sh key <key>                  press a key (QEMU names: ret, tab, ctrl-d)
#   scripts/windows-vm.sh ps <command>               run PowerShell on the machine
#   scripts/windows-vm.sh down                       stop it and throw the copy away
# While it's up you can watch and use it yourself: open vnc://127.0.0.1:5905 in Screen Sharing or any VNC app.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
HERE="$REPO/scripts/windows-vm"
DIR="${VALERIE_VM_DIR:-$HOME/VMs/valerie-test}/windows"
OUT="$REPO/target/windows-vm"
QMP="$DIR/qmp.sock"
PIDFILE="$DIR/qemu.pid"
SSH_PORT=2222
QEMU_SHARE="$(brew --prefix qemu)/share/qemu"
VIRTIO="$DIR/../virtio/virtio-win.iso"

ssh_opts=(-q -i "$DIR/key" -P "$SSH_PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5)
ps_run() { ssh "${ssh_opts[@]/-P/-p}" tester@127.0.0.1 "powershell -NoProfile -EncodedCommand $(printf '%s' "$1" | iconv -t UTF-16LE | base64)"; }

qmp() { # one QMP command, as JSON; prints the answers
  printf '{"execute":"qmp_capabilities"}\n%s\n' "$1" | nc -U -w 2 "$QMP"
}
hmp() { qmp "{\"execute\":\"human-monitor-command\",\"arguments\":{\"command-line\":\"$1\"}}" >/dev/null; }

is_up() { [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; }

start_qemu() { # <disk> <vars> [extra qemu args]
  local disk="$1" vars="$2"; shift 2
  rm -f "$QMP"
  qemu-system-aarch64 -M virt,highmem=on -accel hvf -cpu host -smp 4 -m 6G \
    -drive if=pflash,format=raw,readonly=on,file="$QEMU_SHARE/edk2-aarch64-code.fd" \
    -drive if=pflash,format=qcow2,file="$vars" \
    -device ramfb -device qemu-xhci -device usb-kbd -device usb-tablet \
    -drive if=none,id=disk,format=qcow2,file="$disk" -device nvme,drive=disk,serial=valerie,bootindex=1 \
    -netdev "user,id=net,hostfwd=tcp:127.0.0.1:$SSH_PORT-:22" -device virtio-net-pci,netdev=net \
    -display none -vnc 127.0.0.1:5 -qmp "unix:$QMP,server=on,wait=off" \
    -daemonize -pidfile "$PIDFILE" "$@"
}

wait_ssh() { # up to <seconds>
  local until=$((SECONDS + $1))
  while [ $SECONDS -lt $until ]; do
    ps_run 'exit 0' 2>/dev/null && return 0
    sleep 5
  done
  return 1
}

case "${1:-}" in
  install)
    iso="${2:?give the path of the Windows 11 ARM64 ISO}"
    [ -f "$iso" ] && [ -f "$VIRTIO" ] || { echo "need $iso and $VIRTIO" >&2; exit 1; }
    is_up && { echo "the machine is up: run $0 down" >&2; exit 1; }
    mkdir -p "$DIR"
    [ -f "$DIR/key" ] || ssh-keygen -q -t ed25519 -N '' -C valerie-test-vm -f "$DIR/key"
    # Setup reads autounattend.xml from any attached disc; firstboot.ps1 and the SSH key sit beside it.
    cfg="$(mktemp -d)"
    cp "$HERE/autounattend.xml" "$HERE/firstboot.ps1" "$DIR/key.pub" "$cfg/"
    rm -f "$DIR/config.iso"
    hdiutil makehybrid -quiet -iso -joliet -default-volume-name CONFIG -o "$DIR/config.iso" "$cfg"
    rm -rf "$cfg"
    rm -f "$DIR/clean.qcow2" "$DIR/clean-vars.qcow2"
    qemu-img create -q -f qcow2 "$DIR/clean.qcow2" 64G
    qemu-img convert -q -f raw -O qcow2 "$QEMU_SHARE/edk2-arm-vars.fd" "$DIR/clean-vars.qcow2"
    start_qemu "$DIR/clean.qcow2" "$DIR/clean-vars.qcow2" \
      -drive if=none,id=win,media=cdrom,readonly=on,file="$iso" -device usb-storage,drive=win,bootindex=0 \
      -drive if=none,id=virtio,media=cdrom,readonly=on,file="$VIRTIO" -device usb-storage,drive=virtio \
      -drive if=none,id=config,media=cdrom,readonly=on,file="$DIR/config.iso" -device usb-storage,drive=config
    # "Press any key to boot from CD" only waits a few seconds.
    for _ in $(seq 1 30); do hmp "sendkey ret" 2>/dev/null || true; sleep 1; done
    echo "installing; watch on vnc://127.0.0.1:5905. Waiting for the first boot to finish (up to 90 minutes)."
    wait_ssh 5400 || { echo "the install didn't finish; see vnc://127.0.0.1:5905" >&2; exit 1; }
    ps_run 'Stop-Computer -Force' || true
    until ! is_up; do sleep 3; done
    rm -f "$PIDFILE" "$DIR/config.iso"
    du -h "$DIR/clean.qcow2"
    echo "clean machine ready: $0 up"
    ;;
  up)
    [ -f "$DIR/clean.qcow2" ] || { echo "no clean machine yet: run $0 install <iso>" >&2; exit 1; }
    is_up && { echo "already up"; exit 0; }
    # APFS clones: instant, and they take no disk space until the machine writes.
    rm -f "$DIR/run.qcow2" "$DIR/run-vars.qcow2"
    cp -c "$DIR/clean.qcow2" "$DIR/run.qcow2"
    cp -c "$DIR/clean-vars.qcow2" "$DIR/run-vars.qcow2"
    start=$SECONDS
    start_qemu "$DIR/run.qcow2" "$DIR/run-vars.qcow2"
    wait_ssh 300 || { echo "no answer from the machine; see vnc://127.0.0.1:5905" >&2; exit 1; }
    echo "up in $((SECONDS - start))s. Watch: vnc://127.0.0.1:5905"
    ;;
  try)
    is_up || { echo "not up: run $0 up" >&2; exit 1; }
    mkdir -p "$OUT"
    zip="${2:-}"
    if [ -z "$zip" ]; then
      zip="$OUT/valerie-windows-x86_64.zip"
      gh release download edge -R cullen-b/valerie -p valerie-windows-x86_64.zip -O "$zip" --clobber
    fi
    scp "${ssh_opts[@]}" "$zip" tester@127.0.0.1:Downloads/valerie.zip
    # Marked as from the internet like a browser download, so Windows treats it as a new user would see it.
    ps_run '
      cd $HOME\Downloads
      Set-Content valerie.zip -Stream Zone.Identifier "[ZoneTransfer]`r`nZoneId=3"
      Expand-Archive valerie.zip -DestinationPath Valerie -Force
      schtasks /create /tn open-valerie /sc once /st 00:00 /it /ru tester /f /tr "explorer.exe $HOME\Downloads\Valerie\valerie.exe" | Out-Null
      schtasks /run /tn open-valerie | Out-Null'
    echo "opened. Next: $0 shot first"
    ;;
  shot)
    is_up || { echo "not up" >&2; exit 1; }
    mkdir -p "$OUT"
    rm -f "$DIR/shot.png"
    qmp "{\"execute\":\"screendump\",\"arguments\":{\"filename\":\"$DIR/shot.png\",\"format\":\"png\"}}" >/dev/null
    sleep 1
    mv "$DIR/shot.png" "$OUT/$2.png"
    echo "$OUT/$2.png"
    ;;
  click)
    # The tablet's range covers the whole screen, whatever resolution Windows is using right now.
    rm -f "$DIR/size.png"
    qmp "{\"execute\":\"screendump\",\"arguments\":{\"filename\":\"$DIR/size.png\",\"format\":\"png\"}}" >/dev/null
    sleep 1
    w=$(sips -g pixelWidth "$DIR/size.png" | awk '/pixelWidth/{print $2}')
    h=$(sips -g pixelHeight "$DIR/size.png" | awk '/pixelHeight/{print $2}')
    rm -f "$DIR/size.png"
    x=$(($2 * 32767 / w)) y=$(($3 * 32767 / h))
    abs="{\"type\":\"abs\",\"data\":{\"axis\":\"x\",\"value\":$x}},{\"type\":\"abs\",\"data\":{\"axis\":\"y\",\"value\":$y}}"
    btn() { echo "{\"type\":\"btn\",\"data\":{\"button\":\"left\",\"down\":$1}}"; }
    qmp "{\"execute\":\"input-send-event\",\"arguments\":{\"events\":[$abs]}}" >/dev/null
    qmp "{\"execute\":\"input-send-event\",\"arguments\":{\"events\":[$(btn true)]}}" >/dev/null
    qmp "{\"execute\":\"input-send-event\",\"arguments\":{\"events\":[$(btn false)]}}" >/dev/null
    ;;
  key) hmp "sendkey $2" ;;
  ps) ps_run "$2" ;;
  down)
    if is_up; then
      hmp quit 2>/dev/null || kill "$(cat "$PIDFILE")"
      for _ in 1 2 3 4 5; do is_up || break; sleep 1; done
      is_up && kill -9 "$(cat "$PIDFILE")"
    fi
    rm -f "$PIDFILE" "$QMP" "$DIR/run.qcow2" "$DIR/run-vars.qcow2"
    echo down
    ;;
  *) sed -n 2,11p "$0"; exit 2 ;;
esac
