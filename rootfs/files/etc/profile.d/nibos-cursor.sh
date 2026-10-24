# nibos (rootfs/make-arch.sh): the console's cursor is off from boot when the splash is on
# shellcheck shell=sh
# (vt.global_cursor_default=0, kernel/deploy.sh), so it doesn't flash between the pictures and
# DSlot. A login shell on the console shows it, and hides it again on the way out.
case "$(tty 2>/dev/null)" in
/dev/tty[0-9]*)
    printf '\033[?25h'
    trap 'printf "\033[?25l"' EXIT
    ;;
esac
