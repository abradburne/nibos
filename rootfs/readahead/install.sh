#!/bin/bash
# Build nibos-readahead in the build machine and install it on the DS+ as a systemd generator,
# with its list. The shader cache's files are listed on the device (they change), after Mesa.
#
#   rootfs/readahead/install.sh [user@host]      install
#   rootfs/readahead/install.sh [user@host] off  remove
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../../tools/lib.sh
. "$here/../../tools/lib.sh"
target=${1:-$DEVICE}
ssh_opts+=(-o ConnectTimeout=5)

if [ "${2:-}" = off ]; then
    ssh "${ssh_opts[@]}" "$target" 'sudo rm -f /etc/systemd/system-generators/nibos-readahead /etc/nibos/readahead.list'
    exit
fi
in_vm bash -c "cd '$here' && mkdir -p out && gcc -O2 -static -o out/nibos-readahead readahead.c"
# The list names files in the DSlot user's home as @HOME@.
sed "s|@HOME@|/home/$NIBOS_USER|g" "$here/readahead.list" > "$here/out/readahead.list"
scp -q "${ssh_opts[@]}" "$here/out/nibos-readahead" "$here/out/readahead.list" "$target:/tmp/"
ssh "${ssh_opts[@]}" "$target" "U=$NIBOS_USER sh -s" <<'EOF'
set -e
sudo install -D -m 755 /tmp/nibos-readahead /etc/systemd/system-generators/nibos-readahead
cache=$(find "/home/$U/.cache/mesa_shader_cache" -type f 2>/dev/null | sort)
awk -v cache="$cache" "{ print } /mesa_shader_cache\/index\$/ && cache != \"\" { print cache }" /tmp/readahead.list |
    sudo install -D -m 644 /dev/stdin /etc/nibos/readahead.list
echo "$(grep -c ^/ /etc/nibos/readahead.list) files listed"
EOF
