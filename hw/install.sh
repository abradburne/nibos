#!/bin/sh
# Install the hardware layer into a root file system: the files every board
# shares (files/), then its SoC's and its own (socs/<soc>/hw/files/,
# boards/<board>/hw/files/), and the daemons. Used by the Arch package
# (PKGBUILD). The board's firmware from its stock image isn't part of it: it
# isn't ours to redistribute (install-firmware.sh).
#
#   install.sh DESTDIR BINDIR SOCFILES BOARDFILES
#
# BINDIR holds the built nibos-input, nibos-audio, nibos-backlight, nibos-provision,
# nibos-charge-boot, nibos-bootctl, nibos-keys and nibos-update
# (daemons/, cargo build --release).
set -eu
[ $# -eq 4 ] || { echo "usage: install.sh DESTDIR BINDIR SOCFILES BOARDFILES" >&2; exit 2; }
dest=$1 bin=$2
here=$(cd "$(dirname "$0")" && pwd)
shift 2

# The overlays may not overlap: a board file replacing a shared one would hide
# the change from anyone reading files/.
for files in "$here/files" "$1" "$2"; do
    # (Not the Finder's .DS_Store and ._ files, should the tree have been browsed on a Mac.)
    (cd "$files" && find . -type f ! -name .DS_Store ! -name "._*") | while read -r f; do
        [ ! -e "$dest/${f#./}" ] || { echo "install.sh: ${f#./} is in more than one overlay" >&2; exit 1; }
        install -Dm644 "$files/$f" "$dest/${f#./}"
    done
done
chmod 755 "$dest/usr/bin/nib" "$dest/usr/lib/nibos/nib-net" "$dest/usr/lib/nibos/nib-brightness" \
    "$dest/usr/lib/nibos/nib-lid" "$dest/usr/lib/nibos/nib-battery" "$dest/usr/lib/nibos/nib-wifi" "$dest/usr/lib/nibos/nib-audio" \
    "$dest/usr/lib/nibos/nib-bluetooth" "$dest/usr/lib/nibos/radios-restore" "$dest/usr/lib/nibos/nib-update" \
    "$dest/usr/lib/nibos/nib-bug-report" "$dest/usr/lib/nibos/nib-clock" \
    "$dest/usr/lib/systemd/system-sleep/nibos-unbind" "$dest/usr/lib/systemd/system-sleep/nibos-resumed" \
    "$dest/usr/lib/systemd/system-sleep/nibos-wifi-wake" "$dest/usr/lib/nibos/wifi-after-wake"
# sudo ignores a rule file anyone but root may write.
chmod 440 "$dest/etc/sudoers.d/nibos-audio" "$dest/etc/sudoers.d/nibos-clock"
install -Dm755 "$bin/nibos-input" "$dest/usr/bin/nibos-input"
install -Dm755 "$bin/nibos-audio" "$dest/usr/bin/nibos-audio"
install -Dm755 "$bin/nibos-backlight" "$dest/usr/bin/nibos-backlight"
install -Dm755 "$bin/nibos-provision" "$dest/usr/bin/nibos-provision"
install -Dm755 "$bin/nibos-charge-boot" "$dest/usr/bin/nibos-charge-boot"
install -Dm755 "$bin/nibos-bootctl" "$dest/usr/bin/nibos-bootctl"
install -Dm755 "$bin/nibos-update" "$dest/usr/bin/nibos-update"
install -Dm755 "$bin/nibos-keys" "$dest/usr/bin/nibos-keys"
