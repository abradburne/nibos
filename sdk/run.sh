#!/bin/bash
# The dev loop for a nibOS app: build it in the build machine, copy it to the device, and run it
# on the screens in the foreground, its output here; when it exits (or on Ctrl-C), the screens
# go back to DSlot. With --install, put it on DSlot's Apps tab instead.
#
#   sdk/run.sh [APP_DIR] [-- ARGS...]       (default: sdk/template)
#   sdk/run.sh --install [APP_DIR]          the program, app.toml and icon.png to the card's
#                                           Roms/APPS/<name>/, and DSlot restarted to list it
#
# APP_DIR is a Cargo project whose binary has the package's name. DSlot owns the screens while it
# runs, so it is stopped for the run and started again after; the script refuses while a game is
# being played (pgrep dsperate). The binary goes to sdk/out/ here and /tmp on the device;
# nothing is installed.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../tools/lib.sh
. "$here/../tools/lib.sh"
install=0
[ "${1:-}" = --install ] && { install=1; shift; }
app="$(cd "${1:-$here/template}" && pwd)"
[ $# -gt 0 ] && shift
[ "${1:-}" = -- ] && shift
name=$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$app/Cargo.toml" | head -1)
[ -n "$name" ] || die "no package name in $app/Cargo.toml"
target="\$HOME/nibos-app-target/$name"

# Built in the machine's own file system, copied out to here, and from here to the device (a pipe
# from `orb ... cat` into ssh can arrive cut short).
out="$here/out"
mkdir -p "$out"
in_vm bash -lc "cd '$app' && CARGO_TARGET_DIR=$target cargo build --release --quiet && cp $target/release/$name '$out/$name'"

if [ "$install" = 1 ]; then
    # DSlot's Apps tab: a folder in Roms/APPS with app.toml (DSlot's docs/apps.md). The ROMS
    # partition is FAT, mounted with every file executable.
    [ -f "$app/app.toml" ] || die "no $app/app.toml: DSlot needs one to list the app"
    dir=/mnt/roms/Roms/APPS/$name
    files=("$out/$name" "$app/app.toml")
    [ -f "$app/icon.png" ] && files+=("$app/icon.png")
    on_device "mkdir -p '$dir'"
    scp -q "${ssh_opts[@]}" "${files[@]}" "$DEVICE:$dir/"
    on_device 'pgrep -x dsperate >/dev/null' ||
        on_device 'systemctl -q is-active dslot.service && sudo systemctl restart dslot.service || true'
    echo "installed in $dir; DSlot lists it on its Apps tab"
    exit
fi
scp -q "${ssh_opts[@]}" "$out/$name" "$DEVICE:/tmp/$name"

on_device 'pgrep -x dsperate >/dev/null' && die "a game is running on the device: try again when it isn't"
dslot=$(on_device 'systemctl is-active dslot.service || true')
restore() {
    [ "$dslot" = active ] && on_device 'sudo systemctl start dslot.service' >/dev/null 2>&1
}
trap restore EXIT
[ "$dslot" = active ] && on_device 'sudo systemctl stop dslot.service'
# As DSlot starts games: KMSDRM straight on the panels, the legacy (non-atomic) path, which
# drives both; PipeWire found through the user's runtime directory.
args=
[ $# -eq 0 ] || args=$(printf ' %q' "$@")
ssh "${ssh_opts[@]}" -t "$DEVICE" \
    "SDL_VIDEODRIVER=kmsdrm SDL_KMSDRM_ATOMIC=0 XDG_RUNTIME_DIR=/run/user/\$(id -u) /tmp/$name$args"
