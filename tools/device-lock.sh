#!/usr/bin/env bash
# A lock for several programs or agents sharing one device: one at a time changes it.
#
#   tools/device-lock.sh take "WHO: WHAT"   take it, or exit 1 and say who holds it
#   tools/device-lock.sh drop               give it back
#   tools/device-lock.sh show               who holds it, if anyone
#   tools/device-lock.sh game               exit 0 and print it if a game is running
#
# Take it before anything that changes the device or needs it quiet: installing,
# stopping or starting DSlot, games, benchmarks, deploys, reboots, suspend. Reading
# (journalctl, /proc, cat) needs no lock. The lock is a directory on the device,
# ~/.cache/nibos/device.lock of the login user, with an `owner` file inside;
# mkdir is atomic, so whoever creates it holds it. It is on the card, not a tmpfs,
# so a test that reboots the device keeps it. A reboot or a flat battery doesn't
# clear it either: `show` gives its age; whoever holds a stale one decides when it
# goes. Never drop someone else's lock.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"

# Relative: the remote shell starts in the login user's home.
lock=.cache/nibos/device.lock

ssh_opts+=(-o ConnectTimeout=8)

case "${1:-}" in
take)
    [ -n "${2:-}" ] || usage
    owner="$2, $(date '+%a %d %b %H:%M:%S %Z')"
    # The owner goes in as an argument, quoted for the remote shell.
    if ! on_device "mkdir -p ${lock%/*} && mkdir $lock 2>/dev/null && printf '%s\n' $(printf '%q' "$owner") > $lock/owner && sync"; then
        echo "device lock held by: $(on_device "cat $lock/owner 2>/dev/null" || echo '(unreadable)')" >&2
        exit 1
    fi
    # Read it back: a take that didn't land would let a second holder in.
    [ "$(on_device "cat $lock/owner")" = "$owner" ] || die "device lock: wrote it but can't read it back"
    echo "device lock taken: $owner"
    ;;
drop)
    on_device "rm -rf $lock"
    echo "device lock dropped"
    ;;
show)
    on_device "if [ -d $lock ]; then cat $lock/owner; echo \"(held for \$(( \$(date +%s) - \$(stat -c %Y $lock) ))s)\"; else echo free; fi"
    ;;
game)
    on_device "pgrep -a -x 'dsperate|inkdot|nibcase'"
    ;;
-h) usage 0 ;;
*) usage ;;
esac
