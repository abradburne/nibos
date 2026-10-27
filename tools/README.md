# Tools

| Path | What |
|---|---|
| [`lib.sh`](lib.sh) | The start every host-side script shares: loads [`config.sh`](../config.sh), `config.local.sh` and the board's and SoC's descriptions, and gives `die`, `usage`, `in_vm`, `on_device`, `ssh_opts`, `device_tool`, `spl_time`, `card_disk` |
| [`check.sh`](check.sh), [`check-links.py`](check-links.py) | The repository's checks, run by `make check` |
| [`check-licences.py`](check-licences.py) | In `make check`: every crate the GPL-3.0 parts (nibos-hw, the splash, the safe mode) build in has a licence compatible with GPL-3.0, from `cargo metadata` |
| [`image-notice.py`](image-notice.py) | A downloadable image's NOTICE, its package part: every package with its licence, the custom licences named, notes where they restrict passing the image on |
| [`device-lock.sh`](device-lock.sh) | A lock for several programs or agents sharing one device, kept as a directory on it: `take`, `drop`, `show`, and `game` (is someone playing?) ([AGENTS.md](../AGENTS.md#building-and-shipping)) |
| [`release-build.sh`](release-build.sh) | A release's build half, the same every time ([docs/releasing.md](../docs/releasing.md)): nibos-hw, both images from the public trees, `test-image.sh` and `test-update.sh` one at a time, the source offer collected and packed; then says what needs a person |
| [`card-image.sh`](card-image.sh) | Save a whole card to one compressed image, or write one back (Mac) |
| [`hwtest.sh`](hwtest.sh), [`hwtest/device.sh`](hwtest/device.sh) | The hardware test suite over SSH: boot, display, GPU, audio, input, storage, radios, power, suspend, charging; PASS/FAIL/SKIP and the counts, from the board's `hwtest.conf`. Takes the device lock, won't run during a game; `--sound`, `--suspend N` and `--wifi N` (Wi-Fi off and on, then asleep while it connects) only when asked; it runs detached on the device, so a check that drops SSH carries on; the charging tests need hands and are listed as skipped |
| [`device/`](device/) | Tiny C tools run on the device: `counter` (the arch counter's time), `memdump` and `memwrite` (physical memory through `/dev/mem`), built static on first use by `device_tool` |
| [`stock-probe/`](stock-probe/README.md) | An app for the RG DS+'s stock firmware that reports the hardware: RAM, amps, touch, lid, backlights, USB-C host, suspend |
