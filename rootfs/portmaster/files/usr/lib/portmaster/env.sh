# shellcheck shell=bash
# SPDX-License-Identifier: GPL-3.0-or-later
# PortMaster on nibOS: where it and the ports live, sourced by portmaster and run-port.
#
# nibOS doesn't ship PortMaster: portmaster-get downloads the release nibOS pins
# (release.conf) into ~/.local/share/PortMaster, a place every port's script looks for it, and
# PortMaster updates itself there and fetches runtimes as the user. nibOS's own files (nibos/)
# are put back in that copy before each use: PortMaster's self-update replaces control.txt.

PORTMASTER_LIB=/usr/lib/portmaster

export XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
export controlfolder="$XDG_DATA_HOME/PortMaster"

## Ports go in Roms/PORTS on the games card when it is in, else on the ROMS partition.
## NIBOS_PORTS_DIR set beforehand wins. The card in TF2 is in when /mnt/ds is mounted (udev
## mounts it when the card appears, whatever its name).
if [ -z "${NIBOS_PORTS_DIR:-}" ]; then
  if mountpoint -q /mnt/ds; then
    NIBOS_PORTS_DIR=/mnt/ds/Roms/PORTS
  else
    NIBOS_PORTS_DIR=/mnt/roms/Roms/PORTS
  fi
fi
export NIBOS_PORTS_DIR

## harbourmaster's directories (config.py): PortMaster in tools/PortMaster, ports and their
## scripts together in Roms/PORTS.
export HM_TOOLS_DIR="$XDG_DATA_HOME"
export HM_PORTS_DIR="$NIBOS_PORTS_DIR"
export HM_SCRIPTS_DIR="$NIBOS_PORTS_DIR"

## Ports run as the user: a sudo that refuses, ahead of the real one.
export PATH="$PORTMASTER_LIB/bin:$PATH"

## The release nibOS pins: PORTMASTER_VERSION, _URL, _SHA256, _SIZE.
# shellcheck source=/dev/null
. "$PORTMASTER_LIB/release.conf"

## The version in the user's folder, or nothing.
portmaster_installed() {
  cat "$controlfolder/version" 2>/dev/null
}

## Runs portmaster-get so that a SIGTERM to this script (DSlot's power menu, shutdown) reaches
## it: bash acts on a signal only once its foreground child is done, and DSlot kills the script
## 10 s later, which would leave portmaster-get holding the screens.
portmaster_get_run() {
  local pid status
  "$PORTMASTER_LIB/portmaster-get" "$@" &
  pid=$!
  trap 'kill -TERM "$pid" 2>/dev/null' TERM INT HUP
  wait "$pid"
  status=$?
  ## wait returns when the signal is caught: wait again for portmaster-get's own end.
  while kill -0 "$pid" 2>/dev/null; do
    wait "$pid"
    status=$?
  done
  trap - TERM INT HUP
  return "$status"
}

## Gets PortMaster when it isn't there, and offers the pinned release over an older copy
## (versions are dates, YYYY.MM.DD-HHMM, so they sort as text). Fails when there is none to use.
portmaster_get() {
  local installed status
  installed="$(portmaster_installed)"
  if [ -z "$installed" ]; then
    portmaster_get_run
  elif [[ "$PORTMASTER_VERSION" > "$installed" ]]; then
    portmaster_get_run --update "$installed"
    status=$?
    ## 3: "not now", keep the copy there is.
    [ "$status" = 3 ] && return 0
    return "$status"
  fi
}

## Puts nibOS's files in the user's copy of PortMaster.
portmaster_sync() {
  [ -n "$(portmaster_installed)" ] || return 1

  ## The release zip keeps no file modes; PortMaster's launcher makes everything executable.
  chmod -R u+x "$controlfolder"

  mkdir -p "$controlfolder/nibos" "$NIBOS_PORTS_DIR"
  cp -f "$PORTMASTER_LIB"/nibos/* "$controlfolder/nibos/"
  cp -f "$controlfolder/nibos/control.txt" "$controlfolder/control.txt"
  cp -f "$controlfolder/nibos/mod_nibOS.txt" "$controlfolder/mod_nibOS.txt"
  cp -f "$controlfolder/nibos/libgl_nibOS.txt" "$controlfolder/libgl_nibOS.txt"

  ## device_info.txt names the firmware for every port, some of which source it again after
  ## control.txt. Until PortMaster knows nibOS, teach this copy the one test it needs.
  if ! grep -q '/usr/lib/nibos/board.conf' "$controlfolder/device_info.txt"; then
    sed -i '0,/^if \[ -f "\/app\/bin\/retrodeck" \]/s||if [ -f "/usr/lib/nibos/board.conf" ]; then\n    export CFW_NAME="nibOS"\nelif [ -f "/app/bin/retrodeck" ]|' \
      "$controlfolder/device_info.txt"
  fi
}
