# Contributing

nibOS is meant to be read as much as run, so the bar is that someone new can follow any
change: what it does, why, and what it was measured to do. This page is how the repository is
laid out and kept that way.

## Before you send a change

```bash
make check
```

It runs shellcheck on every shell script, a check that every board's description is complete,
rustfmt, clippy and the unit tests of the Rust code (in the build machine when you're on a
Mac), a parse of every Python file and `mkboot.py`'s tests, and a check that every relative
link in the Markdown files points at something. The same runs on every push once the
repository has a remote ([`.github/workflows/`](.github/workflows/)). Install shellcheck with
`brew install shellcheck` (Mac) or `pacman -S shellcheck` (build machine).

Anything that changes what runs on the device is tested on the device too, and says how in its
commit message. Boot-time changes are measured from reset with the benchmarks
([bench/](bench/README.md)), three boots or more, before and after, and the result goes in
[bench/results/](bench/results/README.md) with its date, hardware and build. Check that
suspend and Wi-Fi still work after anything that touches drivers, the kernel or the sleep path.
Don't run anything on the device while someone is playing on it.

## Layout

Each top-level directory is one layer ([docs/architecture.md](docs/architecture.md)) with a
README that is its reference: what is in it, how to build and use it, how it works and why.
`boards/` and `socs/` hold what only one board or one SoC family needs, for every layer.
`docs/` has the documents that span layers: architecture, building, using, recovery, boot
time, and engineering notes on each part of the hardware.

- Build outputs go in an `out/` next to what makes them, never in git.
- Host-side scripts are bash and start the same way:

  ```bash
  #!/bin/bash
  # What it does, in a sentence or two.
  #
  #   path/to/script.sh ARGS            how to call it
  #
  # What it needs, what it changes, how to undo it.
  set -euo pipefail

  here="$(cd "$(dirname "$0")" && pwd)"
  # shellcheck source=../tools/lib.sh
  . "$here/../tools/lib.sh"
  ```

  [`tools/lib.sh`](tools/lib.sh) loads the settings and gives `die`, `usage` (the header
  comment, also on `-h`), `in_vm`, `on_device`, `ssh_opts` and a few more.
- Nothing names a person or a machine. Who and where come from [`config.sh`](config.sh)
  (`NIBOS_USER`, `DEVICE`, `SSH_KEY`, `BUILD_MACHINE`, source paths and pinned versions),
  overridden in a gitignored `config.local.sh`. Files installed on the device carry
  `@NIBOS_USER@`-style placeholders that their install script fills in.
- Nothing in the shared directories names a board's hardware. What a board or a SoC has comes
  from its `board.sh` / `soc.sh` (`BOARD_*`, `SOC_*`) on the host and from
  `/usr/lib/nibos/board.conf` / `soc.conf` on the device; files only one board needs go in
  `boards/<board>/`, files one SoC family shares in `socs/<soc>/`
  ([boards/README.md](boards/README.md)).
- Scripts that run on the device are POSIX `sh` unless they need more, and use `set -eu`, or
  say why not (a sleep hook must never fail the sleep).
- Names: nibOS in prose (with a lower-case n, also at the start of a sentence), `nibos` in
  every identifier: packages (`nibos-hw`), daemons (`nibos-input`), units and files
  (`nibos-*`), directories (`/usr/lib/nibos`, `/etc/nibos`, `ROMS/nibos/`), variables
  (`NIBOS_*`). The one command on the device is `nib`; a new subcommand is a program
  `/usr/lib/nibos/nib-<name>` whose second line is `# nib <name>: what it does`.
- Units, rules and config files the hardware layer installs start with `# nibos-hw:` and
  what they are for; the root image's and the boot UI's with `# nibos (<where>):`.

## Style

- **Comments say why**, in plain sentences: the constraint, the measurement, the thing that
  went wrong without it. What the code does should be readable from the code. History ("we
  first tried...", dates, rounds) goes in commit messages, not in comments.
- **Rust**: `cargo fmt` (100 columns, `rustfmt.toml`) and `cargo clippy -- -D warnings` clean.
  Keep logic that can be tested away from device I/O, in the crate's library, with unit tests;
  the binaries open devices and run loops. Dependencies are pinned in `Cargo.lock` and built
  `--locked`.
- **Shell**: shellcheck clean; where a check is wrong for a line, disable it on that line with
  the reason. Quote everything; arrays, not word splitting.
- **Documentation**: reference documents describe the present. Measurements carry what was
  measured and how ("3 boots, seconds from reset"). Link to files and sections rather than
  naming them in backticks, so `make links` can check them.

## Commits

One logical change per commit. The subject says what changed, after the area it changes, in the
style of the existing history (`hw: ...`, `kernel: ...`, `uboot: ...`, `docs: ...`); the body says
why, what was measured, and what was tested on the device. Don't rewrite published history.

A contribution is under the licence of the part it changes ([README.md](README.md#license)):
GPL-3.0-or-later for nibOS's own code, MIT for the template, GPL-2.0 for kernel
patches, GPL-2.0-or-later for U-Boot's. nibkit (MIT) takes changes in
[its own repository](https://github.com/abradburne/nibkit). A new dependency must be compatible with that licence:
`make check` lists the hardware layer's crates and fails on one that isn't
(`tools/check-licences.py`).

Every commit in a pull request carries a `Signed-off-by:` line from the human who submits it
(`git commit -s`): their statement under the [Developer Certificate of Origin](https://developercertificate.org/)
that they may contribute it under the project's licence. A check on every pull request
([`.github/workflows/dco.yml`](.github/workflows/dco.yml)) refuses commits without one. It
checks only the commits a pull request adds. **An AI agent never adds
`Signed-off-by:`**: the human does, when submitting; the agent's part goes in a
`Co-Authored-By:` trailer.

## Kernel and U-Boot patches

The patches in [`kernel/patches/`](kernel/patches/README.md),
[`uboot/patches/`](uboot/patches/README.md) and the SoCs' and boards' `patches/` directories
are `git format-patch` output, one set per directory, applied generic, SoC, board. To change
one, apply the sets to a tree at the pinned version (`KERNEL_COMMIT`, `UBOOT_TAG`), change or
add a commit, and regenerate each set
([docs/engineering/patches.md](docs/engineering/patches.md#changing-a-patch)). A patch goes in
the most general set it is true for. Every patch's message ends with its upstream status: a
candidate, or board-specific (and then why). Check that the built output is what you expect:
the kernel's `out/config` and U-Boot's binaries are reproducible enough to diff.

The patch files themselves have no `Signed-off-by:` (this repository's commits that carry them
do): for the kernel and U-Boot that line is the patch author's own certification under their
[Developer's Certificate of Origin](https://docs.kernel.org/process/submitting-patches.html#sign-your-work-the-developer-s-certificate-of-origin),
added when a patch is sent upstream; don't add one on someone else's behalf.

## AI-assisted contributions

**AI contributions are accepted.** Much of nibOS is written with AI coding agents and tested on
the device by a person, and the same way of working is open to everyone. The rules are the same
for every change, whoever or whatever wrote it, and are there to keep contributions easy to
review.

**Accountability.** A human submits every pull request and answers for it: they have read the
change, run it, and stand behind it. Disclose AI help (the box in the pull request template, and
a `Co-Authored-By:` trailer on the commits). Never claim a test
or a device run that didn't happen; say plainly what wasn't checked.

**Small and precise.** One logical change per pull request: no unrelated refactors, reformatting
or drive-by renames mixed in. A few hundred changed lines at most, unless agreed first. **Open an
issue first** for new features, new dependencies, and anything touching the bootloader, the
partitions or the boot chain.

**Every pull request says** what it changes; why (the problem, with evidence: logs,
measurements); how; the alternatives considered; the risk; and how it was tested: `make check`
green, plus device testing with the board, the build, the commands and the results. Performance
claims come with before/after numbers from the repository's benchmarks
([bench/](bench/README.md)); anything visible, with a photo or screenshot.

**Tests.** New behaviour comes with tests where it can be tested; a bug fix with a regression
test where possible.

**Code.** Match the surrounding style ([Style](#style)); comments say why; no dead or
commented-out code, debug leftovers or stray environment switches; no personal paths, host
names or secrets; licence headers where the part needs them; no vendored binaries or firmware.
Docs and [CHANGELOG.md](CHANGELOG.md) change in the same pull request.

**Hardware safety.** A change that could stop a device booting says so at the top of the pull
request, with the way back ([docs/recovery.md](docs/recovery.md)).

**Triage.** Labels: `ai-assisted`, `needs-device-test`, `bootloader`, `breaking`. Maintainers may
close without a detailed review pull requests that ignore these rules: mass rewrites, unverified
hardware claims, speculative "improvements", invented APIs. Pull requests with no activity for
60 days are closed; reopening is welcome.

The short version for agents is in [AGENTS.md](AGENTS.md#contributing).
