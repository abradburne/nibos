#!/bin/bash
# Build the safe mode in the build machine and put it on the device: /usr/lib/nibos/safe-mode,
# nibos-safe-mode.service, and dslot.service's drop-in that restarts DSlot when it fails and
# starts the safe mode when it keeps failing.
#
#   rootfs/safe-mode/install.sh [user@host]
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../../tools/lib.sh
. "$here/../../tools/lib.sh"
target=${1:-$DEVICE}
ssh_opts+=(-o ConnectTimeout=5)

in_vm bash -lc "cd '$here' && cargo build --release"
scp -q "${ssh_opts[@]}" "$here/target/release/nibos-safe-mode" "$here/nibos-safe-mode.service" \
    "$here/nibos-recovery.conf" "$target:/tmp/"
ssh "${ssh_opts[@]}" "$target" 'sh -s' <<'REMOTE'
set -e
sudo install -D -m 755 /tmp/nibos-safe-mode /usr/lib/nibos/safe-mode
sudo install -D -m 644 /tmp/nibos-safe-mode.service /usr/lib/systemd/system/nibos-safe-mode.service
sudo install -D -m 644 /tmp/nibos-recovery.conf /usr/lib/systemd/system/dslot.service.d/nibos-recovery.conf
rm /tmp/nibos-safe-mode /tmp/nibos-safe-mode.service /tmp/nibos-recovery.conf
sudo systemctl daemon-reload
systemctl show dslot.service -p Restart -p OnFailure -p StartLimitBurst
REMOTE
