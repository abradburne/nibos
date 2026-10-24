#!/bin/bash
# Build the splash in the build machine and put it on the device: /usr/lib/nibos/splash (init=,
# and the goodbye at power-off), its pictures in /usr/lib/nibos/pictures (boot.pspl and
# goodbye.pspl, made from pictures/ by nibos-pspl), and nibos-goodbye.service, enabled.
#
#   rootfs/splash/install.sh [user@host]
#
# Leaves the command line as it is: rootfs/splash/boot.sh on|off switches the boot picture.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../../tools/lib.sh
. "$here/../../tools/lib.sh"
target=${1:-$DEVICE}
ssh_opts+=(-o ConnectTimeout=5)

in_vm bash -lc "cd '$here' && cargo build --release && mkdir -p out &&
    target/release/nibos-pspl out/boot.pspl pictures/logo.png &&
    target/release/nibos-pspl out/goodbye.pspl pictures/goodbye.png"
scp -q "${ssh_opts[@]}" "$here/target/release/nibos-splash" "$here/out/boot.pspl" \
    "$here/out/goodbye.pspl" "$here/nibos-goodbye.service" "$target:/tmp/"
ssh "${ssh_opts[@]}" "$target" 'sh -s' <<'REMOTE'
set -e
sudo install -D -m 755 /tmp/nibos-splash /usr/lib/nibos/splash
sudo install -D -m 644 -t /usr/lib/nibos/pictures /tmp/boot.pspl /tmp/goodbye.pspl
sudo install -D -m 644 /tmp/nibos-goodbye.service /usr/lib/systemd/system/nibos-goodbye.service
rm /tmp/nibos-splash /tmp/boot.pspl /tmp/goodbye.pspl /tmp/nibos-goodbye.service
sudo systemctl daemon-reload
sudo systemctl enable --now nibos-goodbye.service
ls -l /usr/lib/nibos/splash /usr/lib/nibos/pictures
REMOTE
