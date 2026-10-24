#!/bin/bash
# The hardware test suite: checks every area of the board over SSH and says what passed.
#
#   tools/hwtest.sh [--sound] [--suspend N] [--wifi N] [AREA...]
#
# AREAs: boot display gpu audio input storage radios power suspend charging (all by default).
# It takes the device lock (tools/device-lock.sh) and won't run while a game does. Nothing
# plays a sound unless --sound (the speakers, out loud), nothing sleeps the device unless
# --suspend N (N RTC-woken suspends of 8 s), nothing turns Wi-Fi off unless --wifi N (N off-and-on
# toggles, then one sleep while it connects; the run carries on without SSH and is collected
# when it's back), and the charging tests, which need hands, are
# listed as skipped with where they're written down. What the board should have is its
# boards/<board>/hwtest.conf; every check is bounded (10 s; a suspend, 40 s).
#
# Output: one line per check, PASS / FAIL / SKIP, then the counts; exit 1 if anything failed.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"

sound=0 suspend=0 wifi=0 areas=()
while [ $# -gt 0 ]; do
    case "$1" in
    --sound) sound=1 ;;
    --suspend) suspend=${2:?--suspend N}; shift ;;
    --wifi) wifi=${2:?--wifi N}; shift ;;
    -h | --help) usage ;;
    *) areas+=("$1") ;;
    esac
    shift
done
profile="$NIBOS_ROOT/boards/$NIBOS_BOARD/hwtest.conf"
[ -f "$profile" ] || die "no $profile"

"$here/device-lock.sh" take "hwtest: ${areas[*]:-all}" >/dev/null || exit 1
trap '"$here/device-lock.sh" drop >/dev/null' EXIT
if "$here/device-lock.sh" game >/dev/null; then
    die "a game is running: not now"
fi

scp -q "${ssh_opts[@]}" "$here/hwtest/device.sh" "$profile" "$DEVICE:/tmp/"
# Detached on the device, so a check that turns Wi-Fi off (and SSH with it) or sleeps carries on;
# the results are collected once it ends ("END"), retrying while the device can't be reached.
ssh "${ssh_opts[@]}" "$DEVICE" "sudo rm -f /tmp/hwtest.out; sudo systemd-run --quiet --collect --unit=hwtest-run \
    --setenv=HWTEST_PROFILE=/tmp/hwtest.conf --setenv=HWTEST_AREAS='${areas[*]:-}' --setenv=HWTEST_SOUND=$sound \
    --setenv=HWTEST_SUSPEND=$suspend --setenv=HWTEST_WIFI_TOGGLE=$wifi \
    sh -c 'timeout 900 bash /tmp/device.sh > /tmp/hwtest.out 2>&1; echo END >> /tmp/hwtest.out'"
results=
for _ in $(seq 200); do
    sleep 5
    out=$(ssh "${ssh_opts[@]}" -o ConnectTimeout=5 "$DEVICE" "cat /tmp/hwtest.out 2>/dev/null" 2>/dev/null) || continue
    if [ "$(printf '%s\n' "$out" | tail -1)" = END ]; then
        results=$(printf '%s\n' "$out" | sed '$d')
        break
    fi
done
[ -n "$results" ] || die "no results from the device within 1000 s"
ssh "${ssh_opts[@]}" "$DEVICE" "sudo rm -f /tmp/device.sh /tmp/hwtest.conf /tmp/hwtest.out"

pass=0 fail=0 skip=0
while IFS='|' read -r area name verdict detail; do
    [ -n "$area" ] || continue
    printf '%-9s %-32s %-4s %s\n' "$area" "$name" "$verdict" "$detail"
    case "$verdict" in PASS) pass=$((pass + 1)) ;; FAIL) fail=$((fail + 1)) ;; *) skip=$((skip + 1)) ;; esac
done <<<"$results"
echo "$pass passed, $fail failed, $skip skipped"
[ "$fail" = 0 ]
