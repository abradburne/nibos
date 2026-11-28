# Receiving games over the network (`nibos-drop`)

Status: **design.** A headless LocalSend v2.1 receiver for nibOS: drag ROMs from a computer or
phone onto the handheld in the LocalSend app, and the device identifies each ROM from its
contents, moves it into the right `Roms/<SYSTEM>/` folder, and rejects anything that is not a
supported ROM.

## In short

- **Standard LocalSend client, no custom sender app.** The handheld appears as a normal target in
  LocalSend on macOS, Windows, Linux, iOS and Android (`deviceType: "headless"`).
- **Two-stage rejection.** Obvious non-ROMs (photos, PDFs, installers, videos) are rejected in
  `prepare-upload` before a byte is transferred. Everything else is streamed to a hidden staging
  folder, verified by magic bytes and cartridge/disc headers (including inside `.zip`, `.chd`,
  `.pbp` and `.iso`), and deleted with an HTTP error if its contents are not a valid ROM.
- **No manual folder picking.** Each accepted ROM is moved atomically into the console folder
  DSlot expects (`Roms/NDS`, `Roms/GBA`, `Roms/SFC`, …). DSlot's inotify watcher (`rescan.rs`)
  puts it on the shelf about a second later.
- **Off by default, unprivileged.** `nibos-drop.service` is never enabled at boot (zero boot-time
  or idle cost). Toggled on from `nib drop on` or DSlot's **Network** menu, with an optional PIN,
  and automatically off again after a restart.
- **TF2 first, then TF1.** Dropped games go to `/mnt/ds/Roms` when a TF2 games card is mounted,
  otherwise `/mnt/roms/Roms` (`NIBOS-DATA`), never overwriting an existing file.
- **One backend in `nibos-hw`.** `nibos-drop` (Rust) + `nib drop` CLI + `/run/nibos/drop/status.json`
  for DSlot, following the same pattern as `nibos-update` ([updates.md](updates.md)).

## Protocol: LocalSend v2.1

LocalSend v2.1 is small, open, and requires no Flutter/Dart runtime. `nibos-drop` implements the
receiver subset natively in Rust.

### Identity and TLS

- **Port:** TCP `53317` (HTTPS) and UDP `53317` (multicast `224.0.0.167`).
- **Certificate:** a self-signed ECDSA P-256 certificate generated once on first start (`rcgen`)
  and saved under `/var/lib/nibos/drop/` (`cert.der`, `key.der`, mode `0600`). Because
  `/var/lib/nibos` is in `update-keep`, senders keep trusting the same fingerprint across A/B
  system updates.
- **Fingerprint:** SHA-256 hex digest of the DER-encoded certificate, as LocalSend v2 requires
  for HTTPS peers.
- **Alias and model:** `alias` defaults to `"DSlot"` (or a configured name via `nib drop alias`);
  `deviceModel` comes from `/usr/lib/nibos/board.conf` (e.g. `Anbernic RG DS+`); `deviceType` is
  `"headless"`; `version` is `"2.1"`; `protocol` is `"https"`; `download` is `false`
  (receive-only).

### Discovery

1. On startup (and after network wake), `nibos-drop` joins UDP multicast `224.0.0.167:53317` and
   broadcasts an announcement JSON packet with `"announce": true` (repeated a couple of times over
   the first few seconds to cover Wi-Fi multicast loss).
2. When it receives a peer's multicast packet with `"announce": true` (or legacy
   `"announcement": true`) and a different `fingerprint`, it responds both by
   `POST <peer.protocol>://<peer_ip>:<peer.port>/api/localsend/v2/register` (accepting the peer's
   self-signed cert) and with a UDP multicast reply (`"announce": false`) as fallback when the
   peer's firewall blocks inbound connections.
3. It answers `GET /api/localsend/v2/info` and `POST /api/localsend/v2/register` with its own
   `InfoDto` so direct-IP scans and favourites work even on networks that drop multicast.

### Transfer endpoints

| Endpoint | Role |
|---|---|
| `GET /api/localsend/v2/info` | Returns device `InfoDto` (`200`). |
| `POST /api/localsend/v2/register` | Accepts peer `InfoDto`, returns ours (`200`). |
| `POST /api/localsend/v2/prepare-upload[?pin=…]` | Checks PIN, session lock, disk space, and Stage 1 file filter. Returns `200` with `{ sessionId, files: { fileId: token } }` for candidate ROMs, `204` if all files are already present identically, `401` if PIN is missing/wrong, `403` if no files are accepted ROM candidates, `409` if another transfer session is active, `429` after repeated bad PINs, `507`/`500` if out of space. |
| `POST /api/localsend/v2/upload?sessionId=…&fileId=…&token=…` | Streams raw file bytes (supporting both `Content-Length` and `Transfer-Encoding: chunked`) into the hidden staging folder while hashing SHA-256 (checked if the sender supplied `sha256`). Runs Stage 2 verification and moves accepted files into place. Returns `200` on accept, `400`/`403` on non-ROM rejection, `409` on filename collision, `422` on checksum mismatch. |
| `POST /api/localsend/v2/cancel?sessionId=…` | Aborts the session and removes all staged files. |

### Implementation footprint

`nibos-hw` builds statically-linked small binaries (`opt-level = "s"`, `lto = true`) on a 1 GiB
handheld. Rather than pulling in `tokio` + `hyper` + OpenSSL, `nibos-drop` uses:

- `std::net::{TcpListener, UdpSocket}` with a bounded thread pool (up to 4 concurrent upload
  streams) and fixed 64 KiB copy buffers — streaming a 700 MiB CHD/ISO uses ~1 MiB of RAM.
- Pure-Rust TLS (`rustls` + `rcgen`; crypto provider chosen for clean offline aarch64 builds).
- Minimal HTTP/1.1 framing (`Content-Length`, `Transfer-Encoding: chunked`, `Expect: 100-continue`).
- `serde`, `serde_json`, `sha2` (already in `hw/daemons/Cargo.toml`).
- `miniz_oxide` (deflate peeking for `.zip` and `cdzl` CHDs); optional lightweight reader for
  `.7z` / `.chd` headers (see Stage 2).

## Two-stage ROM identification and rejection

### Stage 1: pre-transfer gate (`prepare-upload`)

Before accepting any bytes, `nibos-drop` inspects the `files` map from `prepare-upload`:

1. **Filename safety:** reject entries containing `..`, NUL bytes, leading `.`, or paths deeper
   than 1 subfolder (needed only for `PSP/<Game>/EBOOT.PBP` or a `.cue`/`.m3u` folder).
2. **Candidate extension check:** every file must match a supported ROM/disc/archive extension
   (case-insensitive):
   - Unambiguous ROMs: `.nds`, `.gba`, `.gb`, `.gbc`, `.sfc`, `.smc`, `.nes`, `.unf`, `.unif`,
     `.fds`, `.md`, `.gen`, `.smd`, `.32x`, `.sms`, `.sg`, `.gg`, `.pce`, `.lnx`, `.ngp`, `.ngc`,
     `.ws`, `.wsc`, `.mgw`, `.cso`, `.pbp`, `.chd`, `.iso`.
   - Archives: `.zip`, `.7z` (`.7z` only for Arcade / Game & Watch MAME sets; `.zip` for any
     cartridge system or MAME set).
   - Disc sets: `.cue`, `.m3u`, and `.bin`. A standalone `.bin` (no `.cue` in the same session) is
     allowed through Stage 1 only so Stage 2 can check for a Mega Drive / Sega Pico header; any
     other `.bin` is rejected in Stage 2.
   - Everything else (`.jpg`, `.png`, `.pdf`, `.txt`, `.mp4`, `.dmg`, `.exe`, `.sh`, `.apk`, …) is
     excluded immediately.
3. **Mixed batches:** if a user drags 5 ROMs and 1 `.DS_Store` or `readme.txt`, LocalSend v2 lets
   `prepare-upload` return tokens *only* for the accepted file IDs; the non-ROMs are skipped
   without transferring them and logged as rejected. If *zero* files pass Stage 1, return `403
   Forbidden` so LocalSend shows the transfer as rejected.
4. **Disk space check:** verify via `statvfs` that the target card has at least
   `total_accepted_bytes + 64 MiB` free before issuing tokens.

### Stage 2: content verification and routing (`upload`)

Every uploaded file is streamed to `<roms_root>/.drop-staging/<session>/<fileId>` and verified
from its contents before moving to `<roms_root>/<FOLDER>/<fileName>`. If verification fails, the
staged file is deleted immediately and `/upload` returns `403 Forbidden` (so LocalSend marks that
file failed/rejected).

#### Cartridge ROMs (loose or inside `.zip`)

For `.zip`, `nibos-drop` inspects the ZIP central directory without extracting to disk:

- Rejects ZIP bombs, encrypted entries, or archives containing no recognised ROM entry (or
  matching no known Arcade/G&W set).
- If the archive contains a single ROM file (ignoring benign metadata like `readme.txt` /
  `file_id.diz`? — stricter default: single ROM file, optionally alongside `.txt`/`.nfo`, or
  matches a known MAME set), it streams just enough bytes of that entry to run the header check
  below, and keeps the `.zip` intact when moving it to `Roms/<FOLDER>/`.

| Console | Folder | Extensions | Content check |
|---|---|---|---|
| Nintendo DS | `NDS` | `.nds`, `.zip` | Size ≥ `0x200`; standard 156-byte DS Nintendo logo at `0x0C0..0x15C` with logo CRC16 `0xCF56` at `0x15C` (or valid header CRC16 at `0x15E` and sane ARM9/ARM7 offsets within file size for homebrew). |
| Game Boy Advance | `GBA` | `.gba`, `.zip` | Size ≥ `0xC0`; fixed `0x96` at `0xB2`, 156-byte GBA Nintendo logo at `0x04..0xA0`, valid header complement byte at `0xBD`. *(GBA DS games live in `Roms/GBA` and are picked up by DSlot's `GBADS` page automatically.)* |
| Game Boy / Color | `GB`, `GBC` | `.gb`, `.gbc`, `.zip` | Size ≥ `0x150`; 48-byte Nintendo logo at `0x104..0x134` and valid header checksum at `0x14D`. Routed to `GBC` if `.gbc` or CGB-only (`head[0x143] == 0xC0`); otherwise `GB`. |
| Super Nintendo | `SFC` | `.sfc`, `.smc`, `.zip` | Same LoROM (`0x7FB0`), HiROM (`0xFFB0`), ExHiROM (`0x40FFB0`) + optional 512-byte copier header validator used by DSlot (`snes.rs`): valid map mode byte, printable JIS X 0201 title, plausible reset vector. |
| NES / Famicom | `FC` | `.nes`, `.unf`, `.unif`, `.zip` | Magic `NES\x1a` (iNES / NES 2.0) or `UNIF` at offset 0. |
| Famicom Disk System | `FDS` | `.fds`, `.zip` | Magic `FDS\x1a` at offset 0, or headerless disk with `\x01*NINTENDO-HVC*` at offset 0. |
| Sega Pico | `PICO` | `.md`, `.bin`, `.zip` | Checked before Mega Drive: `SEGA PICO` or `SEGA TOYS` at `0x100`. |
| Sega 32X | `32X` | `.32x`, `.zip` | `SEGA 32X` at `0x100` or `MARS` at `0x3C0`. |
| Mega Drive / Genesis | `MD` | `.md`, `.gen`, `.smd`, `.bin`, `.zip` | `SEGA MEGA DRIVE`, `SEGA GENESIS`, or `SEGA ` at `0x100`; or `.smd` copier header (`head[8..11] == [0xAA, 0xBB, 0x06]` and size ≥ 16 KiB + 512). Standalone `.bin` without a Sega header is rejected. |
| Master System | `SMS` | `.sms`, `.zip` | `TMR SEGA` at `0x7FF0`, `0x3FF0` or `0x1FF0` (plus optional 512-byte copier offset); falls back to `.sms` + size multiple of 8 KiB (≥ 16 KiB) for early JP titles without `TMR SEGA`, rejecting plain text/arbitrary files. |
| Game Gear | `GG` | `.gg`, `.zip` | `TMR SEGA` at `0x7FF0`/`0x3FF0`/`0x1FF0` (if ambiguous, region nibble `5..=7` indicates GG); fallback `.gg` + size multiple of 8 KiB (≥ 32 KiB) for headerless GG carts. |
| SG-1000 | `SG1000` | `.sg`, `.zip` | No standard header; requires `.sg` extension, size 8–64 KiB and multiple of 1 KiB, binary (rejects UTF-8 text). |
| PC Engine | `PCE` | `.pce`, `.zip` | No standard magic; requires `.pce` extension, size ≥ 32 KiB and `(size % 8192) in {0, 512}`, binary. |
| Atari Lynx | `LYNX` | `.lnx`, `.zip` | Magic `LYNX` at offset 0 (64-byte header) or `BS93` at offset 6. |
| Neo Geo Pocket | `NGP` | `.ngp`, `.ngc`, `.zip` | ` COPYRIGHT BY SNK CORPORATION` or ` LICENSED BY SNK CORPORATION` at offset 0. |
| WonderSwan | `WS` | `.ws`, `.wsc`, `.zip` | Footer in final 16 bytes: `0xEA` jump opcode at `len - 16`, valid ROM-size enum at `len - 6`, valid save-type byte at `len - 5`. |
| Game & Watch | `GW` | `.mgw`, `.zip`, `.7z` | `.mgw`: valid `gw-libretro` header (`"MGW"` / size bounds). `.zip`/`.7z`: stem matches an entry in an installed `GW` emulator's `[games]` list under `/usr/share/dslot/emulators/`. |
| Arcade | `ARCADE` | `.zip`, `.7z` | Valid ZIP/7z archive whose stem matches an installed `ARCADE` emulator's `[games]` list (or one of the 14 FBNeo BIOS sets such as `neogeo.zip`, which live in `Roms/ARCADE/` beside the games). |

#### Disc images and multi-file sets

| Format | How it is identified |
|---|---|
| `.pbp` | Magic `\0PBP`; parse embedded `PARAM.SFO` key `CATEGORY`: `ME` → `PS`, `UG`/`HG`/`EG` → `PSP`; anything else rejected. If the sender sent `<Game>/EBOOT.PBP`, preserve `<Game>/EBOOT.PBP` under `Roms/PSP/`. |
| `.cso` | Magic `CISO`; inflate sector 16 (2048 bytes) and verify ISO9660 `CD001` + `PSP_GAME` → `PSP`. |
| `.iso` | Check sector 0 for `SEGADISCSYSTEM  ` → `SEGACD`; else check sector 16 (`0x8000`) for ISO9660 `CD001` + `PSP_GAME` / `UMD_DATA.BIN` → `PSP`. Linux ISOs and arbitrary disk images are rejected. |
| `.chd` | Magic `MComprHD` (v4/v5); decompress hunk 0 (supporting `cdzl` deflate, `cdzs` zstd, `cdlz` lzma, raw) and inspect sector 0 & sector 16: `SEGADISCSYSTEM  ` → `SEGACD`; `PLAYSTATION` in PVD (or `BOOT = cdrom`) → `PS`; `PC Engine CD-ROM SYSTEM` → `PCECD`; `PSP_GAME` → `PSP`. |
| `.cue` + `.bin` (and `.m3u`) | Held in the session staging directory until all files referenced by the `.cue` (and `.m3u`, if present) have uploaded. Inspect track 1's data sector (raw 2352-byte or 2048-byte) with the same `SEGACD` / `PS` / `PCECD` signatures above. Once every referenced file is present and verified, move the whole set together into `Roms/<SYSTEM>/`. Missing tracks or unrecognised discs are deleted when the session closes or times out. |

#### Explicit non-goals (rejected)

- **Non-ROM files** (images, documents, music, firmware images, archives without a valid ROM).
- **Ports and Apps** (`Roms/PORTS`, `Roms/APPS`): PortMaster manages ports; apps are packaged
  separately.
- **Box art / saves / BIOS** (except Arcade BIOS sets in `Roms/ARCADE/`): out of scope for v1 to
  keep the "if not a ROM, rejected" contract strict and predictable.

## Destination, staging, and filesystem safety

### Target card selection

Controlled by `target` in `/var/lib/nibos/drop/config.toml` (default `auto`):

- **`auto` (default):** if `/mnt/ds` is mounted and writable (checked via `/proc/mounts`, matching
  `nibos-mount-cards`), write to `/mnt/ds/Roms`; otherwise `/mnt/roms/Roms`.
- **`tf2`:** always `/mnt/ds/Roms` (refuse transfers with `503` if TF2 is not mounted).
- **`tf1`:** always `/mnt/roms/Roms`.

### Staging and atomic placement

- Staging lives on the **same filesystem** as the destination: `<roms_root>/.drop-staging/<session>/`.
  - The leading `.` guarantees DSlot's `rom::scan` and `rescan.rs` ignore staging completely.
  - Because staging and `Roms/<SYSTEM>/` share the same mount, moving an accepted ROM into place is
    a single atomic `rename(2)` — DSlot's `inotify` watcher never sees a half-written file.
- On daemon start and stop (and after a session idle timeout of 60 s), any leftover
  `.drop-staging/` directories are wiped so a power cut or dropped Wi-Fi leaves no stray files.

### Filename sanitisation and collisions

- Strip any path traversal or unsafe FAT32/exFAT characters (`:`, `*`, `?`, `"`, `<`, `>`, `|`,
  control characters).
- **Never overwrite an existing file.** If `<roms_root>/<FOLDER>/<name>` already exists:
  - Reject with `409 Conflict` (and log `already exists` in `status.json`), deleting the staged
    copy. Existing ROMs, patches, and saves are never clobbered.

## On the device

### Daemon and systemd unit

- Binary: `/usr/bin/nibos-drop` (built in `hw/daemons/` as part of `nibos-hw`).
- Unit: `/usr/lib/systemd/system/nibos-drop.service` (installed **disabled**).
  - Runs as the nibOS user (same user drop-in pattern as `dslot.service.d/user.conf`, since
    `/mnt/roms` and `/mnt/ds` are mounted `uid=1000,gid=1000`), not as root.
  - Hardened: `NoNewPrivileges=yes`, `PrivateTmp=yes`, `ProtectSystem=strict`, `ProtectHome=yes`,
    `ReadWritePaths=/mnt/roms /mnt/ds /run/nibos/drop /var/lib/nibos/drop`.
  - Background priority: `Nice=19`, `IOSchedulingClass=idle`, `CPUWeight=1` (same as
    `nibos-update@.service`) so transfers do not hitch an active game.
  - Binds cleanly to Wi-Fi coming/going; does not pull in `nibos-network.target` on boot.
- Polkit rule: `/usr/share/polkit-1/rules.d/50-nibos-drop.rules` lets the `wheel` group start and
  stop `nibos-drop.service` without a password (exact same pattern as `50-nibos-update.rules`).
- `tmpfiles.d`: `/usr/lib/tmpfiles.d/nibos-drop.conf` creates `/run/nibos/drop` and
  `/var/lib/nibos/drop` with group `wheel` write access so `nib drop` and the daemon share config
  and status cleanly without root.

### Persistent config and runtime status

- Config: `/var/lib/nibos/drop/config.toml` (survives updates via `update-keep`):
  ```toml
  alias = "RG DS+"        # empty = system hostname
  target = "auto"         # auto | tf1 | tf2
  pin = ""                # empty = no PIN
  ```
- Runtime status: `/run/nibos/drop/status.json` (written atomically on state changes and every
  ~250 ms during an active upload):
  ```json
  {
    "active": true,
    "alias": "RG DS+",
    "port": 53317,
    "pin_required": false,
    "target": "/mnt/ds/Roms",
    "target_label": "TF2 card",
    "free_bytes": 48123456789,
    "transfer": {
      "file": "Chrono Trigger (USA).sfc",
      "received": 2097152,
      "total": 4194304
    },
    "counts": { "accepted": 3, "rejected": 1 },
    "recent": [
      {
        "time": "22:41:08",
        "file": "Chrono Trigger (USA).sfc",
        "outcome": "accepted",
        "console": "SFC",
        "reason": ""
      },
      {
        "time": "22:41:09",
        "file": "notes.pdf",
        "outcome": "rejected",
        "console": "",
        "reason": "not a recognised ROM"
      }
    ]
  }
  ```

### `nib drop` CLI

`/usr/lib/nibos/nib-drop`, run via `nib drop`:

```
nib drop                        # active/off, alias, IP, target card, recent transfers
nib drop on                     # start nibos-drop.service (turns Wi-Fi on if off)
nib drop off                    # stop nibos-drop.service
nib drop pin <CODE|off>         # require a PIN (or clear it); reloads if running
nib drop target auto|tf1|tf2    # where dropped ROMs go
nib drop alias <NAME>           # device name shown in LocalSend
```

For development and desktop testing, `nibos-drop` also supports standalone flags
(`--roms-dir DIR`, `--state-dir DIR`, `--status FILE`, `--emulators-dir DIR`, `--port N`,
`--pin CODE`, `--alias NAME`) so the full server and classifier can be run directly on macOS or
Linux against the real LocalSend app without systemd.

### DSlot integration

DSlot does not need to speak LocalSend or touch sockets:

1. **Shelf refresh (already works):** `rescan.rs` watches every `Roms/<SYSTEM>` folder with
   `inotify` (and watches `Roms/` for newly created system folders). Moving a verified ROM into
   `Roms/SFC/` automatically puts it on the SNES shelf ~1 s later. We deliberately do **not**
   write `/run/nibos/notice/` files for each ROM, since notices pop a modal dialog requiring a
   button press and would interrupt batch drops.
2. **Network menu (phase 2, in `dslot-public`):** add a **Receive games** section under the
   **Network** tab (shown only when `/usr/bin/nibos-drop` is installed):
   - **Receive games (LocalSend):** On / Off (runs `systemctl start/stop nibos-drop.service`).
   - **PIN:** Off or a 4-digit PIN (A generates/toggles a session PIN; displayed on screen to type
     into LocalSend).
   - **Save to:** Auto (TF2 → Internal) / TF2 card / Internal card, showing free space.
   - **Status & recent log:** read every 1 s from `/run/nibos/drop/status.json` while the section
     is open ("Waiting on 192.168.1.42…", live upload progress, and the last accepted/rejected
     files with console tags).

## Implementation plan and verification

### Phase 1: ROM classifier + LocalSend v2.1 daemon (`nibos-public/hw/daemons/`)

1. **`src/drop/rom.rs`**: pure-Rust two-stage classifier (`stage1_allow`, `classify_file`,
   `classify_cue_set`) with thorough unit tests covering every console's real header layout,
   `.zip` peeking, `.pbp`/`.iso`/`.cso`/`.chd` signatures, `.cue`+`.bin` sets, MAME set list
   matching, and rejection of fake/corrupt/non-ROM files (`.jpg`, `.pdf`, `.txt`, random `.bin`,
   non-ROM `.zip`, Linux `.iso`).
2. **`src/drop/protocol.rs` & `src/drop/server.rs`**: self-signed TLS identity (`rcgen` +
   `rustls`), UDP multicast discovery + `/register`, HTTP/1.1 server with streaming upload to
   `.drop-staging/`, PIN + rate limit, session lock, duplicate & free-space checks, `status.json`
   writer.
3. **`src/bin/nibos-drop.rs`**: CLI flags, config/state management, target card resolution,
   graceful shutdown and staging cleanup.
4. **Desktop live test on Mac**: run `nibos-drop --roms-dir /tmp/test-roms` on macOS, drag real
   ROMs (from the workspace: `.nes`, `.md`, `.lnx`, zipped ROMs) plus non-ROMs (`.png`, `.pdf`,
   random `.zip`) from the real **LocalSend** app on macOS/iPhone, and verify discovery, TLS,
   auto-sorting into `FC/`, `MD/`, `LYNX/`, and rejection of non-ROMs.

### Phase 2: nibOS packaging & `nib drop` CLI (`nibos-public/hw/`)

1. Add `nibos-drop.service`, `50-nibos-drop.rules`, `nibos-drop.conf` (`tmpfiles.d`),
   `/usr/lib/nibos/nib-drop`, `install.sh` and `PKGBUILD` wiring, and `make check` passing.
2. Deploy `nibos-hw` to the RG DS+, run `nib drop on`, drag ROMs from LocalSend on Mac, verify
   they land on TF2 (`Roms/<SYSTEM>/`) and pop straight onto DSlot's shelf.

### Phase 3: DSlot Network menu toggle (`dslot-public`)

1. Add the **Receive games** section under **Network** reading `/run/nibos/drop/status.json` and
   toggling `nibos-drop.service` + PIN + target.
2. Verify on the RG DS+ (including Wi-Fi off → on, TF2 removed → falls back to TF1, sleep/wake).
