# nibOS as a PortMaster platform

`0001-nibOS-...patch` adds nibOS to [PortMaster-GUI](https://github.com/PortsMaster/PortMaster-GUI)
in its own layout, on top of `47af913` (release 2026.10.21-0058):

- `PortMaster/nibos/`: `control.txt`, `launch_hooks.txt`, `PortMaster.txt` (the launcher stub),
  `gamecontrollerdb.txt`; `PortMaster/mod_nibOS.txt` and `PortMaster/libgl_nibOS.txt`. The
  same files as this package's `files/usr/lib/portmaster/nibos/`.
- `device_info.txt` and `harbourmaster/hardware.py`: nibOS is recognised by
  `/usr/lib/nibos/board.conf`.
- `harbourmaster/config.py`, `detect_paths()`: PortMaster in `$XDG_DATA_HOME`, ports in
  `Roms/PORTS` on `/mnt/ds`, else `/mnt/roms`; two tests in `tests/test_config.py`.
- `harbourmaster/platform/nibos.py`: puts `nibos/control.txt` in place after every update, as
  the JELOS/ROCKNIX platform does.

Tested on a Mac: PortMaster's suite (`pytest -m "not sdl"`) gives the same 8 failures with and
without the patch (all in `test_installer.py` and `test_dialog_shell.py`, from macOS's tools),
plus the 3 new tests passing; `test_config.py`, `test_launchers.py` and
`test_release_layout.py` pass (67). With it upstream, nibOS's package would only need the launchers,
`bin/` and the udev rule; `env.sh`'s `device_info.txt` edit and copying `nibos/` would go.

What's not in it: the `mount`/`umount` wrappers for squashfs runtimes (nibOS-side, in `bin/`),
and `tools/installer.sh` (nibOS installs PortMaster with its package).

It hasn't been submitted upstream.
