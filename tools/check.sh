#!/bin/bash
# The repository's checks, one by one or all together (what `make check` runs).
#
#   tools/check.sh [shell|boards|rust|python|links|all]...     (default: all)
#
#   shell    shellcheck on every shell script
#   boards   every board under boards/ with a board.sh sets the names the
#            scripts use, and so does its SoC's soc.sh; and it has the files
#            each layer in the tree reads from it (boards/README.md)
#   rust     rustfmt --check, clippy -D warnings and the unit tests of
#            hw/daemons, rootfs/splash, rootfs/safe-mode, sdk/template and portmaster-get; in the build machine when run on a
#            Mac (the daemons use Linux's evdev)
#   python   every Python file parses; kernel/mkboot.py's unit tests
#   links    every relative link in the Markdown files points at something
#
# Stops at the first check that fails. Runs with macOS's bash 3.2.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"
cd "$NIBOS_ROOT"

# Tracked shell scripts: *.sh, and files whose first line runs a shell.
shell_files() {
    git ls-files | while read -r f; do
        [ -f "$f" ] || continue
        case "$f" in
        *.sh) echo "$f" ;;
        *.md | *.patch | *.rs | *.c | *.py | *.dts | *.lock | *.toml | *.config | *.gcda) ;;
        *) head -n1 "$f" | grep -qE '^#!.*\b(ba|busybox )?sh\b' && echo "$f" ;;
        esac
    done
}

check_shell() {
    command -v shellcheck >/dev/null || die "needs shellcheck (brew install shellcheck, or pacman -S shellcheck)"
    local files=() f
    while read -r f; do files+=("$f"); done < <(shell_files)
    shellcheck -x "${files[@]}"
    echo "shell: ${#files[@]} scripts clean"
}

check_rust() {
    if [ "$(uname -s)" != Linux ]; then
        in_vm bash "$NIBOS_ROOT/tools/check.sh" rust
        return
    fi
    local crate
    for crate in hw/daemons rootfs/splash rootfs/safe-mode sdk/template rootfs/portmaster/get; do
        [ -d "$crate" ] || continue
        (
            cd "$crate"
            # Its own target directory: the Mac and the build machine share this tree.
            export CARGO_TARGET_DIR="${TMPDIR:-/tmp}/nibos-check/${crate//\//-}"
            cargo fmt --check
            cargo clippy --locked --all-targets --quiet -- -D warnings
            cargo test --locked --quiet
        )
        echo "rust: $crate clean"
    done
    # Every crate the GPL-3.0 parts build in has a licence compatible with them.
    python3 tools/check-licences.py hw/daemons rootfs/splash rootfs/safe-mode rootfs/portmaster/get | tail -1
}

check_python() {
    local files=() f
    while read -r f; do files+=("$f"); done < <(git ls-files '*.py')
    python3 - "${files[@]}" <<'PY'
import ast, sys
for f in sys.argv[1:]:
    ast.parse(open(f).read(), f)
PY
    echo "python: ${#files[@]} files parse"
    if [ -d kernel ]; then
        PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -q -s kernel -p 'test_*.py'
        echo "python: unit tests pass"
    fi
}

# The names every board.sh and soc.sh must set: what the build and install
# scripts read from them.
board_names="BOARD_NAME BOARD_SOC BOARD_HOSTNAME BOARD_STOCK_IMAGE BOARD_FIRMWARE BOARD_DTB
    BOARD_CONSOLE_ROWS BOARD_CONSOLE_COLUMNS BOARD_MEMORY BOARD_UBOOT_DEFCONFIG BOARD_RKBIN_DDR
    BOARD_UBOOT_MMC BOARD_BOOTLOADER_START BOARD_BOOTLOADER_END BOARD_FALLBACK_SECTOR
    BOARD_BOOT_PART BOARD_BOOT_PARTNAME BOARD_FALCON_PART BOARD_FALCON_PARTNAME
    BOARD_FALCON_SECTOR BOARD_ROMS_PART BOARD_ROOT_PART BOARD_ROOT_PARTLABEL"
soc_names="SOC_NAME SOC_KERNEL_PLATFORM SOC_DTS_DIR SOC_RKBIN_BL31"

check_boards() {
    local board name n=0
    for board in boards/*/board.sh; do
        name=${board#boards/}
        name=${name%/board.sh}
        # shellcheck disable=SC2016 # expanded by the inner bash
        NIBOS_BOARD=$name NAMES="$board_names $soc_names" bash -c '
            set -u
            . ./config.sh
            for v in $NAMES; do
                [ -n "${!v:-}" ] || { echo "boards/$NIBOS_BOARD: no $v" >&2; exit 1; }
            done
            # What each layer reads from the board, for the layers this tree has.
            b=boards/$NIBOS_BOARD s=socs/$BOARD_SOC
            need=()
            [ -d kernel ] && need+=("$b/dts/$BOARD_DTB.dts" "$b/kernel/$NIBOS_BOARD.config" \
                "$b/kernel/smoke.config" "$b/kernel/smoke-firmware" "$s/kernel/$BOARD_SOC.config")
            [ -d uboot ] && need+=("$b/uboot/$NIBOS_BOARD.config" "$b/uboot/falcon.config")
            [ -d hw ] && need+=("$b/hw/files/usr/lib/nibos/board.conf" "$s/hw/files/usr/lib/nibos/soc.conf")
            for f in ${need[@]+"${need[@]}"}; do
                [ -f "$f" ] || { echo "$b: no $f" >&2; exit 1; }
            done'
        n=$((n + 1))
    done
    echo "boards: $n described in full"
}

check_links() {
    python3 tools/check-links.py
}

[ $# -gt 0 ] || set -- all
for what in "$@"; do
    case "$what" in
    shell) check_shell ;;
    boards) check_boards ;;
    rust) check_rust ;;
    python) check_python ;;
    links) check_links ;;
    all) check_shell; check_boards; check_python; check_links; check_rust ;;
    *) usage ;;
    esac
done
