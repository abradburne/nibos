//! Two-stage ROM filter and content verifier for `nibos-drop` (docs/design/rom-drop.md).
//!
//! Stage 1 (`stage1_plan`) runs on `prepare-upload` metadata before any bytes are transferred:
//! it rejects obvious non-ROMs and groups `.cue`/`.bin`/`.m3u` disc sets.
//!
//! Stage 2 (`classify_file` and `classify_cue_set`) runs on the staged file(s) after upload:
//! it inspects actual magic bytes, cartridge/disc headers, ZIP entries, and installed emulator
//! `[games]` lists, returning either the exact DSlot console folder (`NDS`, `GBA`, `SFC`, ...)
//! or a human-readable rejection reason.

use super::sanitize_relpath;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Every ROM console folder DSlot recognises (matching `dslot::rom::Console::folder`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Console {
    Ds,
    Gba,
    Gb,
    Gbc,
    Snes,
    Nes,
    Fds,
    MegaDrive,
    MegaCd,
    Sega32x,
    Pico,
    MasterSystem,
    Sg1000,
    GameGear,
    PcEngine,
    PcEngineCd,
    PlayStation,
    Psp,
    Lynx,
    NeoGeoPocket,
    WonderSwan,
    GameWatch,
    Arcade,
}

impl Console {
    /// Target folder under `Roms/` (identical to DSlot's `Console::folder`).
    pub fn folder(self) -> &'static str {
        match self {
            Console::Ds => "NDS",
            Console::Gba => "GBA",
            Console::Gb => "GB",
            Console::Gbc => "GBC",
            Console::Snes => "SFC",
            Console::Nes => "FC",
            Console::Fds => "FDS",
            Console::MegaDrive => "MD",
            Console::MegaCd => "SEGACD",
            Console::Sega32x => "32X",
            Console::Pico => "PICO",
            Console::MasterSystem => "SMS",
            Console::Sg1000 => "SG1000",
            Console::GameGear => "GG",
            Console::PcEngine => "PCE",
            Console::PcEngineCd => "PCECD",
            Console::PlayStation => "PS",
            Console::Psp => "PSP",
            Console::Lynx => "LYNX",
            Console::NeoGeoPocket => "NGP",
            Console::WonderSwan => "WS",
            Console::GameWatch => "GW",
            Console::Arcade => "ARCADE",
        }
    }
}

/// Known Arcade and Game & Watch MAME set stems loaded from `/usr/share/dslot/emulators/*.toml`.
#[derive(Clone, Debug, Default)]
pub struct EmulatorIndex {
    pub arcade: HashSet<String>,
    pub game_watch: HashSet<String>,
}

/// FinalBurn Neo's board BIOS sets that live alongside games in `Roms/ARCADE/`.
const ARCADE_BIOS_SETS: [&str; 14] = [
    "atarisy1", "bubsys", "cchip", "decocass", "isgsm", "midssio", "namcoc69", "namcoc70",
    "namcoc75", "neogeo", "nmk004", "pgm", "skns", "ym2608",
];

impl EmulatorIndex {
    /// Reads `[games]` tables from every `.toml` file in `dir`. Missing directory is fine (empty).
    pub fn load(dir: &Path) -> Self {
        let mut idx = Self::default();
        let Ok(rd) = fs::read_dir(dir) else {
            return idx;
        };
        #[derive(serde::Deserialize)]
        struct EmuDecl {
            #[serde(default)]
            games: BTreeMap<String, PathBuf>,
        }
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(decl) = toml::from_str::<EmuDecl>(&text) else {
                continue;
            };
            let base = path.parent().unwrap_or(dir);
            for (sys, list_path) in decl.games {
                let target = match sys.as_str() {
                    "ARCADE" => &mut idx.arcade,
                    "GW" => &mut idx.game_watch,
                    _ => continue,
                };
                let full = if list_path.is_absolute() {
                    list_path
                } else {
                    base.join(list_path)
                };
                let Ok(list) = fs::read_to_string(full) else {
                    continue;
                };
                for line in list.lines() {
                    if line.trim_start().starts_with('#') {
                        continue;
                    }
                    let mut cols = line.split('\t').map(str::trim);
                    let Some(stem) = cols.next().filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    if cols.next().is_some_and(|t| !t.is_empty()) {
                        target.insert(stem.to_ascii_lowercase());
                    }
                }
            }
        }
        idx
    }

    pub fn is_arcade_set(&self, stem: &str) -> bool {
        let lower = stem.to_ascii_lowercase();
        self.arcade.contains(&lower) || ARCADE_BIOS_SETS.contains(&lower.as_str())
    }

    pub fn is_gw_set(&self, stem: &str) -> bool {
        self.game_watch.contains(&stem.to_ascii_lowercase())
    }
}

/// Candidate file in `prepare-upload`.
#[derive(Clone, Debug)]
pub struct Candidate<'a> {
    pub id: &'a str,
    pub file_name: &'a str,
    pub size: u64,
}

/// How an accepted Stage 1 file will be handled once uploaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileRole {
    /// Standalone ROM or single-file disc/archive.
    Single,
    /// Part of a `.cue` / `.bin` / `.m3u` multi-file disc set (committed together).
    DiscSetMember,
}

#[derive(Clone, Debug)]
pub struct AcceptedCandidate {
    pub id: String,
    pub subdir: Option<String>,
    pub file_name: String,
    pub size: u64,
    pub role: FileRole,
}

#[derive(Clone, Debug)]
pub struct RejectedCandidate {
    pub id: String,
    pub file_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct Stage1Plan {
    pub accepted: Vec<AcceptedCandidate>,
    pub rejected: Vec<RejectedCandidate>,
}

impl Stage1Plan {
    pub fn total_bytes(&self) -> u64 {
        self.accepted.iter().map(|c| c.size).sum()
    }
}

/// Extensions allowed past Stage 1 as standalone ROM/disc/archive candidates.
const STAGE1_SINGLE_EXTS: &[&str] = &[
    "nds", "gba", "gb", "gbc", "sfc", "smc", "nes", "unf", "unif", "fds", "md", "gen", "smd",
    "32x", "sms", "sg", "gg", "pce", "lnx", "ngp", "ngc", "ws", "wsc", "mgw", "cso", "pbp", "chd",
    "iso", "zip", "7z", "bin",
];

const DISC_SET_EXTS: &[&str] = &["cue", "m3u"];

/// Stage 1: filters `prepare-upload` metadata before any bytes are transferred.
pub fn stage1_plan(candidates: &[Candidate<'_>]) -> Stage1Plan {
    let mut plan = Stage1Plan::default();
    // Pre-pass: check whether each (optional) subdir has a `.cue` or `.m3u` in this session so
    // accompanying `.bin` tracks are marked as disc-set members instead of standalone Genesis ROMs.
    let mut disc_subdirs: HashSet<Option<String>> = HashSet::new();
    for c in candidates {
        if let Some((sub, name)) = sanitize_relpath(c.file_name) {
            if let Some(ext) = ext_lower(&name) {
                if DISC_SET_EXTS.contains(&ext.as_str()) {
                    disc_subdirs.insert(sub);
                }
            }
        }
    }

    for c in candidates {
        let Some((subdir, name)) = sanitize_relpath(c.file_name) else {
            plan.rejected.push(RejectedCandidate {
                id: c.id.to_owned(),
                file_name: c.file_name.to_owned(),
                reason: "unsafe or hidden filename".into(),
            });
            continue;
        };
        if c.size == 0 {
            plan.rejected.push(RejectedCandidate {
                id: c.id.to_owned(),
                file_name: name,
                reason: "empty file".into(),
            });
            continue;
        }
        let Some(ext) = ext_lower(&name) else {
            plan.rejected.push(RejectedCandidate {
                id: c.id.to_owned(),
                file_name: name,
                reason: "no file extension".into(),
            });
            continue;
        };
        let in_disc_set = disc_subdirs.contains(&subdir)
            && (DISC_SET_EXTS.contains(&ext.as_str())
                || ext == "bin"
                || ext == "iso"
                || ext == "chd");
        // Subfolders are only meaningful for PSP `<Game>/EBOOT.PBP` or a disc set folder.
        let is_eboot = name.eq_ignore_ascii_case("EBOOT.PBP");
        if subdir.is_some() && !is_eboot && !in_disc_set {
            // Flatten harmless single-level folders of ROMs to the filename itself, keeping
            // `<Game>/EBOOT.PBP` and disc-set folders intact.
        }
        if in_disc_set {
            plan.accepted.push(AcceptedCandidate {
                id: c.id.to_owned(),
                subdir,
                file_name: name,
                size: c.size,
                role: FileRole::DiscSetMember,
            });
        } else if STAGE1_SINGLE_EXTS.contains(&ext.as_str()) {
            let kept_subdir = if is_eboot { subdir } else { None };
            plan.accepted.push(AcceptedCandidate {
                id: c.id.to_owned(),
                subdir: kept_subdir,
                file_name: name,
                size: c.size,
                role: FileRole::Single,
            });
        } else {
            plan.rejected.push(RejectedCandidate {
                id: c.id.to_owned(),
                file_name: name,
                reason: format!("unsupported file type (.{ext})"),
            });
        }
    }
    plan
}

/// Successful Stage 2 classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    pub console: Console,
    /// Optional subdirectory inside `Roms/<CONSOLE>/` (used for PSP `<Game>/EBOOT.PBP`).
    pub preserve_subdir: bool,
}

/// Stage 2: verifies a staged standalone file by its contents and picks its target console.
pub fn classify_file(
    path: &Path,
    file_name: &str,
    index: &EmulatorIndex,
) -> Result<Classification, String> {
    let ext = ext_lower(file_name).ok_or_else(|| "missing extension".to_owned())?;
    let meta = fs::metadata(path).map_err(|e| format!("cannot read staged file: {e}"))?;
    let size = meta.len();
    if size == 0 {
        return Err("empty file".into());
    }

    match ext.as_str() {
        "zip" => classify_zip(path, file_name, index),
        "7z" => classify_7z(path, file_name, index),
        "pbp" => classify_pbp(path, file_name),
        "cso" => classify_cso(path),
        "iso" => classify_iso(path),
        "chd" => classify_chd(path),
        _ => {
            let mut f = File::open(path).map_err(|e| format!("cannot open file: {e}"))?;
            let console = classify_raw_rom(&ext, size, &mut f)?;
            Ok(Classification {
                console,
                preserve_subdir: false,
            })
        }
    }
}

/// Parses `.cue` `FILE "..."` references (returning the referenced base filenames in order).
pub fn parse_cue_files(cue_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in cue_text.lines() {
        let trimmed = line.trim();
        let rest = if let Some(r) = trimmed.strip_prefix("FILE ") {
            r
        } else if trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("FILE ") {
            &trimmed[5..]
        } else {
            continue;
        };
        let rest = rest.trim_start();
        let name = if let Some(after_q) = rest.strip_prefix('"') {
            after_q.split('"').next().unwrap_or("")
        } else {
            rest.split_whitespace().next().unwrap_or("")
        };
        if !name.is_empty() {
            // Take the filename component if the cue had a relative path.
            let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
            if !base.is_empty() {
                out.push(base.to_owned());
            }
        }
    }
    out
}

/// Parses `.m3u` playlist entries (non-empty, non-`#` lines).
pub fn parse_m3u_files(m3u_text: &str) -> Vec<String> {
    m3u_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.rsplit(['/', '\\']).next().unwrap_or(l).to_owned())
        .collect()
}

/// Classifies a `.cue` + track `.bin` (and optional `.m3u`) disc set staged together.
/// `staged` maps lowercase filename -> (original filename, staged path).
pub fn classify_disc_set(staged: &HashMap<String, (String, PathBuf)>) -> Result<Console, String> {
    let cues: Vec<&PathBuf> = staged
        .iter()
        .filter(|(k, _)| k.ends_with(".cue"))
        .map(|(_, (_, p))| p)
        .collect();
    let m3us: Vec<&PathBuf> = staged
        .iter()
        .filter(|(k, _)| k.ends_with(".m3u"))
        .map(|(_, (_, p))| p)
        .collect();

    if cues.is_empty() && m3us.is_empty() {
        return Err("disc set has no .cue or .m3u".into());
    }

    // Verify all `.m3u` references exist in the staged set.
    let mut m3u_consoles = Vec::new();
    for m3u in &m3us {
        let text = fs::read_to_string(m3u).map_err(|e| format!("cannot read .m3u: {e}"))?;
        let refs = parse_m3u_files(&text);
        if refs.is_empty() {
            return Err("empty .m3u playlist".into());
        }
        for r in refs {
            let Some((_, path)) = staged.get(&r.to_ascii_lowercase()) else {
                return Err(format!("missing disc referenced by .m3u: {r}"));
            };
            if r.to_ascii_lowercase().ends_with(".chd") {
                m3u_consoles.push(classify_chd(path)?.console);
            } else if r.to_ascii_lowercase().ends_with(".iso") {
                m3u_consoles.push(classify_iso(path)?.console);
            }
        }
    }

    // Verify all `.cue` references exist and inspect their first data track.
    let mut detected: Option<Console> = None;
    for cue in &cues {
        let text = fs::read_to_string(cue).map_err(|e| format!("cannot read .cue: {e}"))?;
        let refs = parse_cue_files(&text);
        if refs.is_empty() {
            return Err("empty .cue sheet".into());
        }
        let mut cue_console = None;
        for r in &refs {
            let Some((_, track_path)) = staged.get(&r.to_ascii_lowercase()) else {
                return Err(format!("missing track referenced by .cue: {r}"));
            };
            if cue_console.is_none() {
                cue_console = identify_disc_track_file(track_path);
            }
        }
        let console =
            cue_console.ok_or_else(|| "unrecognised disc image in .cue set".to_owned())?;
        if let Some(prev) = detected {
            if prev != console {
                return Err("mixed console discs in one .cue set".into());
            }
        } else {
            detected = Some(console);
        }
    }

    for c in m3u_consoles {
        if let Some(prev) = detected {
            if prev != c {
                return Err("mixed console discs in .m3u set".into());
            }
        } else {
            detected = Some(c);
        }
    }

    detected.ok_or_else(|| "unrecognised disc set".into())
}

// ---------------------------------------------------------------------------
// Raw cartridge ROM identification
// ---------------------------------------------------------------------------

/// SHA-256 of the 48-byte Game Boy Nintendo logo (`0x104..0x134`).
const GB_LOGO_SHA256: [u8; 32] = [
    0xda, 0xf4, 0xca, 0xbd, 0xc8, 0x52, 0xba, 0xa0, 0x29, 0x18, 0x49, 0x20, 0x3f, 0x0b, 0x41, 0xfd,
    0x0b, 0x4e, 0xcd, 0x58, 0xe0, 0xd7, 0xaf, 0xf4, 0xa5, 0x09, 0xf5, 0xde, 0x4d, 0x7f, 0x9a, 0x2e,
];

/// First 8 bytes of the 156-byte GBA / NDS compressed Nintendo logo.
const GBA_DS_LOGO_PREFIX: [u8; 8] = [0x24, 0xFF, 0xAE, 0x51, 0x69, 0x9A, 0xA2, 0x21];
/// CRC-16/MODBUS of the 156-byte GBA / NDS compressed Nintendo logo.
const GBA_DS_LOGO_CRC16: u16 = 0xCF56;

fn classify_raw_rom<R: Read + Seek>(ext: &str, size: u64, r: &mut R) -> Result<Console, String> {
    match ext {
        "nds" => verify_nds(size, r).map(|()| Console::Ds),
        "gba" => verify_gba(size, r).map(|()| Console::Gba),
        "gb" | "gbc" => verify_gb(ext, size, r),
        "sfc" | "smc" => verify_snes(size, r).map(|()| Console::Snes),
        "nes" | "unf" | "unif" => verify_nes(ext, size, r).map(|()| Console::Nes),
        "fds" => verify_fds(size, r).map(|()| Console::Fds),
        "32x" => verify_32x(size, r).map(|()| Console::Sega32x),
        "md" | "gen" | "smd" | "bin" => verify_sega_16bit(ext, size, r),
        "sms" => verify_sms_gg(Console::MasterSystem, size, r),
        "gg" => verify_sms_gg(Console::GameGear, size, r),
        "sg" => verify_sg1000(size, r).map(|()| Console::Sg1000),
        "pce" => verify_pce(size, r).map(|()| Console::PcEngine),
        "lnx" => verify_lynx(size, r).map(|()| Console::Lynx),
        "ngp" | "ngc" => verify_ngp(size, r).map(|()| Console::NeoGeoPocket),
        "ws" | "wsc" => verify_wonderswan(size, r).map(|()| Console::WonderSwan),
        "mgw" => verify_mgw(size, r).map(|()| Console::GameWatch),
        _ => Err(format!("not a recognised ROM (.{ext})")),
    }
}

fn read_at<R: Read + Seek>(r: &mut R, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    r.seek(SeekFrom::Start(offset))?;
    r.read_exact(buf)
}

fn crc16_modbus(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn verify_nds<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if size < 0x200 {
        return Err("too small for a Nintendo DS ROM".into());
    }
    let mut head = [0u8; 0x160];
    read_at(r, 0, &mut head).map_err(|_| "cannot read NDS header".to_owned())?;
    let logo = &head[0x0C0..0x15C];
    let logo_crc_field = u16::from_le_bytes([head[0x15C], head[0x15D]]);
    let header_crc_field = u16::from_le_bytes([head[0x15E], head[0x15F]]);
    let logo_ok = logo.starts_with(&GBA_DS_LOGO_PREFIX)
        && crc16_modbus(logo) == GBA_DS_LOGO_CRC16
        && logo_crc_field == GBA_DS_LOGO_CRC16;
    if logo_ok {
        return Ok(());
    }
    // Homebrew NDS ROMs may omit the Nintendo logo but still have a valid header CRC16 and
    // ARM9/ARM7 offsets within the file.
    let header_crc_ok = crc16_modbus(&head[..0x15E]) == header_crc_field;
    let arm9_off = u64::from(u32::from_le_bytes(head[0x20..0x24].try_into().unwrap()));
    let arm9_len = u64::from(u32::from_le_bytes(head[0x2C..0x30].try_into().unwrap()));
    let arm7_off = u64::from(u32::from_le_bytes(head[0x30..0x34].try_into().unwrap()));
    let arm7_len = u64::from(u32::from_le_bytes(head[0x3C..0x40].try_into().unwrap()));
    if header_crc_ok
        && arm9_off >= 0x200
        && arm9_len > 0
        && arm9_off.saturating_add(arm9_len) <= size
        && arm7_off >= 0x200
        && arm7_len > 0
        && arm7_off.saturating_add(arm7_len) <= size
    {
        return Ok(());
    }
    Err("not a valid Nintendo DS ROM header".into())
}

fn verify_gba<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if size < 0xC0 {
        return Err("too small for a GBA ROM".into());
    }
    let mut head = [0u8; 0xC0];
    read_at(r, 0, &mut head).map_err(|_| "cannot read GBA header".to_owned())?;
    let fixed = head[0xB2] == 0x96;
    let complement = head[0xA0..0xBD]
        .iter()
        .fold(0u8, |acc, &b| acc.wrapping_sub(b))
        .wrapping_sub(0x19);
    let logo = &head[0x04..0xA0];
    let logo_ok = logo.starts_with(&GBA_DS_LOGO_PREFIX) && crc16_modbus(logo) == GBA_DS_LOGO_CRC16;
    if fixed && complement == head[0xBD] && logo_ok {
        return Ok(());
    }
    Err("not a valid GBA ROM header".into())
}

fn verify_gb<R: Read + Seek>(ext: &str, size: u64, r: &mut R) -> Result<Console, String> {
    if size < 0x150 {
        return Err("too small for a Game Boy ROM".into());
    }
    let mut head = [0u8; 0x150];
    read_at(r, 0, &mut head).map_err(|_| "cannot read GB header".to_owned())?;
    let logo_hash: [u8; 32] = Sha256::digest(&head[0x104..0x134]).into();
    let checksum = head[0x134..0x14D]
        .iter()
        .fold(0u8, |acc, &b| acc.wrapping_sub(b).wrapping_sub(1));
    if logo_hash != GB_LOGO_SHA256 || checksum != head[0x14D] {
        return Err("not a valid Game Boy ROM header".into());
    }
    let cgb_flag = head[0x143];
    if ext == "gbc" || cgb_flag == 0xC0 {
        Ok(Console::Gbc)
    } else {
        Ok(Console::Gb)
    }
}

/// SNES LoROM / HiROM / ExHiROM validation (mirrors DSlot's `rom/snes.rs`).
fn verify_snes<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    const OFFSETS: [u64; 3] = [0x7FB0, 0xFFB0, 0x40FFB0];
    const HEADER_LEN: usize = 0x50;
    let copier = if size % 1024 == 512 { 512 } else { 0 };
    if size < 0x8000 {
        return Err("too small for a SNES ROM".into());
    }
    for offset in OFFSETS {
        if offset + copier + HEADER_LEN as u64 > size {
            continue;
        }
        let mut bytes = [0u8; HEADER_LEN];
        if read_at(r, offset + copier, &mut bytes).is_err() {
            continue;
        }
        let mode = bytes[0x25] & !0x10;
        let mode_ok = match offset {
            0x7FB0 => matches!(mode, 0x20 | 0x22 | 0x23),
            0xFFB0 => mode == 0x21,
            _ => mode == 0x25,
        };
        let title = &bytes[0x10..0x25];
        let title_ok = title.iter().any(|&b| b != b' ')
            && title.iter().all(|b| matches!(b, 0x20..=0x7E | 0xA1..=0xDF));
        let le16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let checksum_ok = le16(0x2C) ^ le16(0x2E) == 0xFFFF && le16(0x2E) != 0;
        let reset_ok = le16(0x4C) >= 0x8000;
        if mode_ok && title_ok && (checksum_ok || reset_ok) {
            return Ok(());
        }
    }
    Err("not a valid SNES ROM header".into())
}

fn verify_nes<R: Read + Seek>(ext: &str, size: u64, r: &mut R) -> Result<(), String> {
    if size < 16 {
        return Err("too small for a NES ROM".into());
    }
    let mut magic = [0u8; 4];
    read_at(r, 0, &mut magic).map_err(|_| "cannot read NES header".to_owned())?;
    if magic == *b"NES\x1a" && size >= 16 + 16384 {
        return Ok(());
    }
    if matches!(ext, "unf" | "unif") && magic == *b"UNIF" && size >= 32 {
        return Ok(());
    }
    Err("not a valid iNES/UNIF ROM header".into())
}

fn verify_fds<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if size < 65500 {
        return Err("too small for a Famicom Disk System image".into());
    }
    let mut head = [0u8; 16];
    read_at(r, 0, &mut head).map_err(|_| "cannot read FDS header".to_owned())?;
    if head[..4] == *b"FDS\x1a" || head[..15] == *b"\x01*NINTENDO-HVC*" {
        return Ok(());
    }
    Err("not a valid FDS disk image".into())
}

fn verify_32x<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if size < 0x400 {
        return Err("too small for a Sega 32X ROM".into());
    }
    let mut head = [0u8; 0x400];
    read_at(r, 0, &mut head).map_err(|_| "cannot read 32X header".to_owned())?;
    if head[0x100..0x110].starts_with(b"SEGA 32X") || &head[0x3C0..0x3C4] == b"MARS" {
        return Ok(());
    }
    Err("not a valid Sega 32X ROM header".into())
}

fn verify_sega_16bit<R: Read + Seek>(ext: &str, size: u64, r: &mut R) -> Result<Console, String> {
    if size < 0x200 {
        return Err("too small for a Mega Drive / Pico ROM".into());
    }
    let mut head = [0u8; 0x400];
    let n = head.len().min(size as usize);
    read_at(r, 0, &mut head[..n]).map_err(|_| "cannot read Sega ROM header".to_owned())?;
    let sig = &head[0x100..0x110];
    if sig.starts_with(b"SEGA PICO") || sig.starts_with(b"SEGA TOYS") {
        return Ok(Console::Pico);
    }
    if sig.starts_with(b"SEGA 32X") || (n >= 0x3C4 && &head[0x3C0..0x3C4] == b"MARS") {
        return Ok(Console::Sega32x);
    }
    if sig.starts_with(b"SEGA MEGA DRIVE")
        || sig.starts_with(b"SEGA GENESIS")
        || sig.starts_with(b"SEGA ")
    {
        return Ok(Console::MegaDrive);
    }
    // Super Magic Drive (.smd) interleaved copier format: 512-byte header with 0xAA, 0xBB, 0x06.
    if ext == "smd" && size >= 512 + 16384 && head[8] == 0xAA && head[9] == 0xBB && head[10] == 0x06
    {
        return Ok(Console::MegaDrive);
    }
    Err("not a valid Mega Drive or Sega Pico ROM header".into())
}

fn verify_sms_gg<R: Read + Seek>(hint: Console, size: u64, r: &mut R) -> Result<Console, String> {
    let copier = if size % 1024 == 512 { 512 } else { 0 };
    for offset in [0x7FF0u64, 0x3FF0, 0x1FF0] {
        if offset + copier + 16 > size {
            continue;
        }
        let mut hdr = [0u8; 16];
        if read_at(r, offset + copier, &mut hdr).is_ok() && &hdr[..8] == b"TMR SEGA" {
            let region = hdr[15] >> 4;
            let console = match region {
                5..=7 => Console::GameGear,
                3..=4 => Console::MasterSystem,
                _ => hint,
            };
            return Ok(console);
        }
    }
    // Early Japanese SMS and some GG carts omit `TMR SEGA`. Require standard cart size & binary.
    let raw_size = size.saturating_sub(copier);
    let min_size = if hint == Console::GameGear {
        32 * 1024
    } else {
        16 * 1024
    };
    if raw_size >= min_size
        && raw_size <= 4 * 1024 * 1024
        && raw_size.is_multiple_of(8192)
        && looks_binary(r)
    {
        return Ok(hint);
    }
    Err("not a valid Master System / Game Gear ROM".into())
}

fn verify_sg1000<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if (8 * 1024..=64 * 1024).contains(&size) && size.is_multiple_of(1024) && looks_binary(r) {
        return Ok(());
    }
    Err("not a valid SG-1000 ROM".into())
}

fn verify_pce<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    let rem = size % 8192;
    if (32 * 1024..=8 * 1024 * 1024).contains(&size) && (rem == 0 || rem == 512) && looks_binary(r)
    {
        return Ok(());
    }
    Err("not a valid PC Engine ROM".into())
}

fn verify_lynx<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if size < 64 {
        return Err("too small for an Atari Lynx ROM".into());
    }
    let mut head = [0u8; 16];
    read_at(r, 0, &mut head).map_err(|_| "cannot read Lynx header".to_owned())?;
    if &head[..4] == b"LYNX" && size > 64 {
        return Ok(());
    }
    if &head[6..10] == b"BS93" {
        return Ok(());
    }
    // Headerless No-Intro `.lnx` dumps are exact cart bank sizes (64K..1M).
    const RAW_SIZES: [u64; 5] = [65536, 131072, 262144, 524288, 1048576];
    if RAW_SIZES.contains(&size) && looks_binary(r) {
        return Ok(());
    }
    Err("not a valid Atari Lynx ROM".into())
}

fn verify_ngp<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if size < 0x100 {
        return Err("too small for a Neo Geo Pocket ROM".into());
    }
    let mut head = [0u8; 28];
    read_at(r, 0, &mut head).map_err(|_| "cannot read NGP header".to_owned())?;
    if &head == b"COPYRIGHT BY SNK CORPORATION" || &head == b" LICENSED BY SNK CORPORATION" {
        return Ok(());
    }
    Err("not a valid Neo Geo Pocket ROM header".into())
}

fn verify_wonderswan<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if size < 64 * 1024 || !size.is_multiple_of(1024) {
        return Err("invalid WonderSwan ROM size".into());
    }
    let mut foot = [0u8; 16];
    read_at(r, size - 16, &mut foot).map_err(|_| "cannot read WonderSwan footer".to_owned())?;
    // Boot jump opcode 0xEA, valid ROM size code (0x01..=0x09), valid save type (<= 0x05 or 0x10..=0x50).
    let rom_size_ok = matches!(foot[10], 0x01..=0x09);
    let save_ok = foot[11] <= 0x05 || matches!(foot[11], 0x10 | 0x20 | 0x50);
    if foot[0] == 0xEA && rom_size_ok && save_ok {
        return Ok(());
    }
    Err("not a valid WonderSwan ROM footer".into())
}

fn verify_mgw<R: Read + Seek>(size: u64, r: &mut R) -> Result<(), String> {
    if size < 16 {
        return Err("too small for a Game & Watch ROM".into());
    }
    let mut head = [0u8; 8];
    read_at(r, 0, &mut head).map_err(|_| "cannot read MGW header".to_owned())?;
    if &head[..4] == b"MGW\0" || &head[..3] == b"MGW" {
        return Ok(());
    }
    Err("not a valid Game & Watch (.mgw) file".into())
}

/// Heuristic rejecting plain UTF-8 text or common non-ROM magic headers (PNG, JPEG, PDF, ELF, ZIP)
/// for the few headerless 8-bit formats (SG-1000, PCE, headerless Lynx/SMS/GG).
fn looks_binary<R: Read + Seek>(r: &mut R) -> bool {
    let mut sample = [0u8; 512];
    if r.seek(SeekFrom::Start(0)).is_err() {
        return false;
    }
    let Ok(n) = r.read(&mut sample) else {
        return false;
    };
    if n < 64 {
        return false;
    }
    let s = &sample[..n];
    // Reject well-known non-ROM containers.
    const NON_ROM_MAGICS: [&[u8]; 8] = [
        b"\x89PNG\r\n\x1a\n",
        b"\xFF\xD8\xFF",
        b"%PDF-",
        b"\x7fELF",
        b"PK\x03\x04",
        b"GIF8",
        b"RIFF",
        b"<!DOCTYPE",
    ];
    if NON_ROM_MAGICS.iter().any(|m| s.starts_with(m)) {
        return false;
    }
    // Reject all-zero/all-FF dummy files and plain UTF-8 text.
    if s.iter().all(|&b| b == 0) || s.iter().all(|&b| b == 0xFF) {
        return false;
    }
    let text_like = std::str::from_utf8(s).is_ok()
        && s.iter()
            .all(|&b| b == b'\n' || b == b'\r' || b == b'\t' || (0x20..=0x7E).contains(&b));
    !text_like
}

// ---------------------------------------------------------------------------
// Archives (.zip, .7z)
// ---------------------------------------------------------------------------

struct ZipEntry {
    name: String,
    method: u16,
    flags: u16,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: u64,
}

/// Parses a ZIP archive's central directory without extracting to disk.
fn read_zip_entries(f: &mut File) -> Result<Vec<ZipEntry>, String> {
    let file_len = f
        .metadata()
        .map_err(|e| format!("cannot stat zip: {e}"))?
        .len();
    if file_len < 22 {
        return Err("not a valid ZIP archive".into());
    }
    // Locate End of Central Directory (EOCD) in the last 64 KiB + 22 bytes.
    let search_len = file_len.min(65557) as usize;
    let search_start = file_len - search_len as u64;
    let mut tail = vec![0u8; search_len];
    read_at(f, search_start, &mut tail).map_err(|_| "cannot read ZIP tail".to_owned())?;
    let eocd_pos = (0..=search_len - 22)
        .rev()
        .find(|&i| &tail[i..i + 4] == b"PK\x05\x06")
        .ok_or_else(|| "not a valid ZIP archive".to_owned())?;

    let le16 = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
    let le32 = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);

    let total_entries = le16(&tail, eocd_pos + 10) as usize;
    let cd_size = u64::from(le32(&tail, eocd_pos + 12));
    let cd_offset = u64::from(le32(&tail, eocd_pos + 16));
    if total_entries == 0 || total_entries > 4096 || cd_size > 16 * 1024 * 1024 {
        return Err("ZIP archive empty or too many entries".into());
    }
    if cd_offset.saturating_add(cd_size) > file_len {
        return Err("corrupt ZIP central directory".into());
    }
    let mut cd = vec![0u8; cd_size as usize];
    read_at(f, cd_offset, &mut cd).map_err(|_| "cannot read ZIP directory".to_owned())?;

    let mut entries = Vec::with_capacity(total_entries);
    let mut pos = 0usize;
    for _ in 0..total_entries {
        if pos + 46 > cd.len() || &cd[pos..pos + 4] != b"PK\x01\x02" {
            return Err("corrupt ZIP central directory entry".into());
        }
        let flags = le16(&cd, pos + 8);
        let method = le16(&cd, pos + 10);
        let crc32 = le32(&cd, pos + 16);
        let compressed_size = u64::from(le32(&cd, pos + 20));
        let uncompressed_size = u64::from(le32(&cd, pos + 24));
        let name_len = le16(&cd, pos + 28) as usize;
        let extra_len = le16(&cd, pos + 30) as usize;
        let comment_len = le16(&cd, pos + 32) as usize;
        let local_header_offset = u64::from(le32(&cd, pos + 42));
        let end = pos + 46 + name_len + extra_len + comment_len;
        if end > cd.len() {
            return Err("truncated ZIP entry".into());
        }
        let name = String::from_utf8_lossy(&cd[pos + 46..pos + 46 + name_len]).into_owned();
        entries.push(ZipEntry {
            name,
            method,
            flags,
            crc32,
            compressed_size,
            uncompressed_size,
            local_header_offset,
        });
        pos = end;
    }
    Ok(entries)
}

/// Extracts up to `max_bytes` of a ZIP entry (stored or deflate) into memory for header inspection.
/// For formats that check the footer (WonderSwan) or ExHiROM (4.5 MiB), capped at `max_bytes`.
fn read_zip_entry_bytes(f: &mut File, e: &ZipEntry, max_bytes: usize) -> Result<Vec<u8>, String> {
    if e.flags & 1 != 0 {
        return Err("encrypted ZIP entries are not supported".into());
    }
    let mut local = [0u8; 30];
    read_at(f, e.local_header_offset, &mut local)
        .map_err(|_| "cannot read ZIP local header".to_owned())?;
    if &local[..4] != b"PK\x03\x04" {
        return Err("invalid ZIP local header".into());
    }
    let name_len = u16::from_le_bytes([local[26], local[27]]) as u64;
    let extra_len = u16::from_le_bytes([local[28], local[29]]) as u64;
    let data_offset = e.local_header_offset + 30 + name_len + extra_len;
    f.seek(SeekFrom::Start(data_offset))
        .map_err(|_| "cannot seek ZIP entry data".to_owned())?;

    let full_check = e.uncompressed_size <= max_bytes as u64;
    let want = (e.uncompressed_size as usize).min(max_bytes);
    let out = match e.method {
        0 => {
            let mut out = vec![0u8; want];
            f.read_exact(&mut out)
                .map_err(|_| "truncated stored ZIP entry".to_owned())?;
            out
        }
        8 => {
            // Stream deflate in 32 KiB chunks into a linear (non-wrapping) output buffer.
            // TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF is mandatory whenever `out.len()` is not a
            // 32 KiB power-of-two ring buffer (e.g. 1.5 MiB or 3 MiB SNES ROMs).
            use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
            use miniz_oxide::inflate::TINFLStatus;
            let mut state = DecompressorOxide::new();
            // When checking the full entry, allocate 1 extra byte so a stream longer than its
            // declared uncompressed size is caught instead of silently truncated.
            let buf_cap = if full_check {
                want + 1
            } else {
                want.max(64 * 1024)
            };
            let mut out = vec![0u8; buf_cap];
            let mut out_pos = 0usize;
            let mut remaining = e.compressed_size;
            let mut in_buf = vec![0u8; 32 * 1024];
            let mut leftover: Vec<u8> = Vec::new();
            let mut finished = false;

            while out_pos < buf_cap {
                if leftover.len() < 4096 && remaining > 0 {
                    let take = (remaining as usize).min(in_buf.len());
                    let n = f
                        .read(&mut in_buf[..take])
                        .map_err(|_| "error reading compressed ZIP entry".to_owned())?;
                    if n == 0 {
                        remaining = 0;
                    } else {
                        remaining -= n as u64;
                        leftover.extend_from_slice(&in_buf[..n]);
                    }
                }
                let mut flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
                if remaining > 0 {
                    flags |= inflate_flags::TINFL_FLAG_HAS_MORE_INPUT;
                }
                let (status, in_used, out_made) =
                    decompress(&mut state, &leftover, &mut out, out_pos, flags);
                leftover.drain(..in_used);
                out_pos += out_made;
                match status {
                    TINFLStatus::Done => {
                        finished = true;
                        break;
                    }
                    TINFLStatus::HasMoreOutput => {
                        if !full_check && out_pos >= want {
                            break;
                        }
                        if out_pos >= buf_cap {
                            return Err("corrupt ZIP (uncompressed size mismatch)".into());
                        }
                    }
                    TINFLStatus::NeedsMoreInput => {
                        if remaining == 0 {
                            return Err("truncated deflate stream in ZIP".into());
                        }
                        // Force reading more compressed bytes even if `leftover` has >4 KiB.
                        let take = (remaining as usize).min(in_buf.len());
                        let n = f
                            .read(&mut in_buf[..take])
                            .map_err(|_| "error reading compressed ZIP entry".to_owned())?;
                        if n == 0 {
                            return Err("truncated deflate stream in ZIP".into());
                        }
                        remaining -= n as u64;
                        leftover.extend_from_slice(&in_buf[..n]);
                    }
                    _ => return Err("corrupt deflate stream in ZIP".into()),
                }
            }
            if full_check && (!finished || out_pos != want) {
                return Err("corrupt ZIP (uncompressed size mismatch)".into());
            }
            out.truncate(out_pos.min(want));
            out
        }
        m => return Err(format!("unsupported ZIP compression method {m}")),
    };
    if full_check && e.crc32 != 0 && crc32_ieee(&out) != e.crc32 {
        return Err("corrupt ZIP (CRC mismatch)".into());
    }
    Ok(out)
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn classify_zip(
    path: &Path,
    file_name: &str,
    index: &EmulatorIndex,
) -> Result<Classification, String> {
    let mut f = File::open(path).map_err(|e| format!("cannot open zip: {e}"))?;
    let entries = read_zip_entries(&mut f)?;

    // Check known Game & Watch / Arcade MAME sets first (their zips hold raw chip dumps without
    // cartridge extensions).
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if index.is_gw_set(stem) {
        return Ok(Classification {
            console: Console::GameWatch,
            preserve_subdir: false,
        });
    }
    if index.is_arcade_set(stem) {
        return Ok(Classification {
            console: Console::Arcade,
            preserve_subdir: false,
        });
    }

    // Otherwise expect a single cartridge ROM entry (ignoring directories and tiny readme/nfo).
    const CART_EXTS: &[&str] = &[
        "nds", "gba", "gb", "gbc", "sfc", "smc", "nes", "unf", "unif", "fds", "md", "gen", "smd",
        "32x", "sms", "sg", "gg", "pce", "lnx", "ngp", "ngc", "ws", "wsc", "mgw", "bin",
    ];
    let mut rom_entries: Vec<&ZipEntry> = Vec::new();
    for e in &entries {
        if e.name.ends_with('/') || e.uncompressed_size == 0 {
            continue;
        }
        let base = e.name.rsplit('/').next().unwrap_or(&e.name);
        if base.starts_with('.') {
            continue;
        }
        let ext = ext_lower(base).unwrap_or_default();
        if CART_EXTS.contains(&ext.as_str()) {
            rom_entries.push(e);
        } else if !matches!(ext.as_str(), "txt" | "nfo" | "diz") {
            return Err("ZIP contains non-ROM files".into());
        }
    }
    if rom_entries.len() != 1 {
        return Err("ZIP does not contain a recognised ROM".into());
    }
    let entry = rom_entries[0];
    let inner_ext =
        ext_lower(&entry.name).ok_or_else(|| "ZIP entry has no extension".to_owned())?;
    // Cap how much we inflate for header/footer checks: WonderSwan needs the footer (max 16 MiB),
    // SNES ExHiROM needs up to ~4.2 MiB, everything else needs <= 64 KiB.
    let cap: usize = match inner_ext.as_str() {
        "ws" | "wsc" => 16 * 1024 * 1024,
        "sfc" | "smc" => 5 * 1024 * 1024,
        _ => 64 * 1024,
    };
    if entry.uncompressed_size > 512 * 1024 * 1024 {
        return Err("ZIP entry too large".into());
    }
    let bytes = read_zip_entry_bytes(&mut f, entry, cap)?;
    let mut cursor = Cursor::new(&bytes);
    let console = if matches!(inner_ext.as_str(), "ws" | "wsc") {
        // WonderSwan footer is in the last 16 bytes, so `bytes` must be the full ROM.
        if bytes.len() as u64 != entry.uncompressed_size {
            return Err("WonderSwan ROM inside ZIP too large to verify".into());
        }
        classify_raw_rom(&inner_ext, entry.uncompressed_size, &mut cursor)?
    } else {
        classify_raw_rom(&inner_ext, entry.uncompressed_size, &mut cursor)?
    };
    Ok(Classification {
        console,
        preserve_subdir: false,
    })
}

fn classify_7z(
    path: &Path,
    file_name: &str,
    index: &EmulatorIndex,
) -> Result<Classification, String> {
    let mut f = File::open(path).map_err(|e| format!("cannot open 7z: {e}"))?;
    let mut magic = [0u8; 6];
    read_at(&mut f, 0, &mut magic).map_err(|_| "cannot read 7z header".to_owned())?;
    if magic != [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] {
        return Err("not a valid 7-Zip archive".into());
    }
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if index.is_gw_set(stem) {
        return Ok(Classification {
            console: Console::GameWatch,
            preserve_subdir: false,
        });
    }
    if index.is_arcade_set(stem) {
        return Ok(Classification {
            console: Console::Arcade,
            preserve_subdir: false,
        });
    }
    Err("7-Zip archive is not a recognised Arcade or Game & Watch set".into())
}

// ---------------------------------------------------------------------------
// Disc images (.pbp, .cso, .iso, .chd, .cue/.bin)
// ---------------------------------------------------------------------------

fn classify_pbp(path: &Path, file_name: &str) -> Result<Classification, String> {
    let mut f = File::open(path).map_err(|e| format!("cannot open PBP: {e}"))?;
    let mut head = [0u8; 40];
    read_at(&mut f, 0, &mut head).map_err(|_| "cannot read PBP header".to_owned())?;
    if &head[0..4] != b"\x00PBP" {
        return Err("not a valid PBP file".into());
    }
    let sfo_off = u32::from_le_bytes(head[8..12].try_into().unwrap()) as u64;
    let sfo_end = u32::from_le_bytes(head[12..16].try_into().unwrap()) as u64;
    let sfo_len = sfo_end.saturating_sub(sfo_off);
    if !(20..=256 * 1024).contains(&sfo_len) {
        return Err("PBP missing valid PARAM.SFO".into());
    }
    let mut sfo = vec![0u8; sfo_len as usize];
    read_at(&mut f, sfo_off, &mut sfo).map_err(|_| "cannot read PBP PARAM.SFO".to_owned())?;
    let category = parse_sfo_string(&sfo, "CATEGORY")
        .ok_or_else(|| "PBP PARAM.SFO has no CATEGORY".to_owned())?;
    match category.as_str() {
        "ME" => Ok(Classification {
            console: Console::PlayStation,
            preserve_subdir: false,
        }),
        "UG" | "HG" | "EG" | "DG" | "MG" => Ok(Classification {
            console: Console::Psp,
            preserve_subdir: file_name.eq_ignore_ascii_case("EBOOT.PBP"),
        }),
        other => Err(format!("unsupported PBP category ({other})")),
    }
}

/// Extracts a UTF-8 string key from a PSP/PS1 `PARAM.SFO` table.
fn parse_sfo_string(sfo: &[u8], want: &str) -> Option<String> {
    if sfo.len() < 20 || &sfo[0..4] != b"\x00PSF" {
        return None;
    }
    let le32 = |at: usize| u32::from_le_bytes(sfo[at..at + 4].try_into().unwrap()) as usize;
    let le16 = |at: usize| u16::from_le_bytes([sfo[at], sfo[at + 1]]) as usize;
    let key_table = le32(8);
    let data_table = le32(12);
    let count = le32(16);
    if count > 512 || 20 + count * 16 > sfo.len() {
        return None;
    }
    for i in 0..count {
        let entry = 20 + i * 16;
        let key_off = key_table + le16(entry);
        let data_len = le32(entry + 4);
        let data_off = data_table + le32(entry + 12);
        if key_off >= sfo.len() || data_off + data_len > sfo.len() {
            continue;
        }
        let key_end = sfo[key_off..].iter().position(|&b| b == 0)?;
        let key = std::str::from_utf8(&sfo[key_off..key_off + key_end]).ok()?;
        if key == want {
            let raw = &sfo[data_off..data_off + data_len];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            return std::str::from_utf8(&raw[..end]).ok().map(str::to_owned);
        }
    }
    None
}

fn classify_cso(path: &Path) -> Result<Classification, String> {
    let mut f = File::open(path).map_err(|e| format!("cannot open CSO: {e}"))?;
    let mut head = [0u8; 24];
    read_at(&mut f, 0, &mut head).map_err(|_| "cannot read CSO header".to_owned())?;
    if &head[0..4] != b"CISO" {
        return Err("not a valid CSO file".into());
    }
    let total_bytes = u64::from_le_bytes(head[8..16].try_into().unwrap());
    let block_size = u32::from_le_bytes(head[16..20].try_into().unwrap()) as usize;
    let align = head[21];
    if block_size != 2048 || total_bytes < 17 * 2048 || align > 16 {
        return Err("invalid CSO geometry".into());
    }
    // Block index table starts at byte 24; we need block 16 (ISO9660 PVD).
    let mut idx = [0u8; 8];
    read_at(&mut f, 24 + 16 * 4, &mut idx).map_err(|_| "cannot read CSO index".to_owned())?;
    let raw0 = u32::from_le_bytes(idx[0..4].try_into().unwrap());
    let raw1 = u32::from_le_bytes(idx[4..8].try_into().unwrap());
    let uncompressed = (raw0 & 0x8000_0000) != 0;
    let start = u64::from(raw0 & 0x7FFF_FFFF) << align;
    let end = u64::from(raw1 & 0x7FFF_FFFF) << align;
    let len = end.saturating_sub(start) as usize;
    if len == 0 || len > 65536 {
        return Err("corrupt CSO block index".into());
    }
    let mut packed = vec![0u8; len];
    read_at(&mut f, start, &mut packed).map_err(|_| "cannot read CSO block 16".to_owned())?;
    let sector16 = if uncompressed {
        packed
    } else {
        miniz_oxide::inflate::decompress_to_vec(&packed)
            .map_err(|_| "corrupt CSO deflate block".to_owned())?
    };
    if is_psp_pvd(&sector16) {
        return Ok(Classification {
            console: Console::Psp,
            preserve_subdir: false,
        });
    }
    Err("CSO is not a PSP UMD image".into())
}

fn classify_iso(path: &Path) -> Result<Classification, String> {
    let mut f = File::open(path).map_err(|e| format!("cannot open ISO: {e}"))?;
    let size = f
        .metadata()
        .map_err(|e| format!("cannot stat ISO: {e}"))?
        .len();
    if size < 17 * 2048 {
        return Err("too small for a disc ISO".into());
    }
    let mut sec0 = [0u8; 2048];
    read_at(&mut f, 0, &mut sec0).map_err(|_| "cannot read ISO sector 0".to_owned())?;
    if sec0.starts_with(b"SEGADISCSYSTEM  ") {
        return Ok(Classification {
            console: Console::MegaCd,
            preserve_subdir: false,
        });
    }
    let mut sec16 = [0u8; 2048];
    read_at(&mut f, 16 * 2048, &mut sec16).map_err(|_| "cannot read ISO sector 16".to_owned())?;
    if is_psp_pvd(&sec16) {
        return Ok(Classification {
            console: Console::Psp,
            preserve_subdir: false,
        });
    }
    // Some PSP ISOs have blank system_id in PVD; check root directory record for PSP_GAME / UMD_DATA.BIN.
    if is_iso9660_pvd(&sec16) && iso_root_contains(&mut f, &sec16, &[b"PSP_GAME", b"UMD_DATA.BIN"])
    {
        return Ok(Classification {
            console: Console::Psp,
            preserve_subdir: false,
        });
    }
    Err("ISO is not a recognised Mega-CD or PSP game disc".into())
}

fn classify_chd(path: &Path) -> Result<Classification, String> {
    let mut f = File::open(path).map_err(|e| format!("cannot open CHD: {e}"))?;
    let mut head = [0u8; 124];
    let n = f
        .read(&mut head)
        .map_err(|_| "cannot read CHD header".to_owned())?;
    if n < 76 || &head[0..8] != b"MComprHD" {
        return Err("not a valid CHD file".into());
    }
    let be32 = |at: usize| u32::from_be_bytes(head[at..at + 4].try_into().unwrap());
    let be64 = |at: usize| u64::from_be_bytes(head[at..at + 8].try_into().unwrap());
    let version = be32(12);
    if version != 5 {
        return Err(format!("unsupported CHD version {version} (expected v5)"));
    }
    if n < 124 {
        return Err("truncated CHD v5 header".into());
    }
    let codecs = [&head[16..20], &head[20..24], &head[24..28], &head[28..32]];
    let logical_bytes = be64(32);
    let map_offset = be64(40);
    let hunk_bytes = be32(56) as usize;
    let unit_bytes = be32(60) as usize;
    if hunk_bytes == 0 || hunk_bytes > 256 * 1024 || unit_bytes == 0 || logical_bytes == 0 {
        return Err("invalid CHD v5 geometry".into());
    }
    let frames_per_hunk = hunk_bytes / unit_bytes;
    if frames_per_hunk == 0 {
        return Err("invalid CHD unit/hunk ratio".into());
    }
    // We need sectors 0, 1, 2 (Mega-CD / PCE-CD) and sector 16 (PS1 / PSP ISO9660 PVD).
    let needed_hunks = (17 / frames_per_hunk) + 1;
    let compressed = codecs[0] != [0, 0, 0, 0];

    let mut sectors: Vec<Vec<u8>> = Vec::new();
    for hunk_idx in 0..needed_hunks {
        let data = if !compressed {
            // Uncompressed v5 map: 4-byte block index per hunk.
            let mut m = [0u8; 4];
            read_at(&mut f, map_offset + (hunk_idx as u64) * 4, &mut m)
                .map_err(|_| "cannot read CHD uncompressed map".to_owned())?;
            let block = u64::from(u32::from_be_bytes(m));
            let mut buf = vec![0u8; hunk_bytes];
            read_at(&mut f, block * hunk_bytes as u64, &mut buf)
                .map_err(|_| "cannot read CHD uncompressed hunk".to_owned())?;
            buf
        } else {
            read_chd_v5_compressed_hunk(
                &mut f, map_offset, codecs, hunk_bytes, unit_bytes, hunk_idx,
            )?
        };
        for frame in data.chunks_exact(unit_bytes) {
            sectors.push(extract_sector_user_data(frame).to_vec());
        }
    }

    if let Some(console) = identify_disc_sectors(&sectors) {
        return Ok(Classification {
            console,
            preserve_subdir: false,
        });
    }
    Err("CHD is not a recognised PS1, PSP, Mega-CD, or PC Engine CD disc".into())
}

/// Decodes a single hunk from a compressed CHD v5 map (handling standard CD codecs `cdzl`, `cdzs`,
/// `cdlz`, plus plain `zlib`, `zstd`, `lzma`, and uncompressed self/none entries).
fn read_chd_v5_compressed_hunk(
    f: &mut File,
    map_offset: u64,
    codecs: [&[u8]; 4],
    hunk_bytes: usize,
    unit_bytes: usize,
    want_hunk: usize,
) -> Result<Vec<u8>, String> {
    // CHD v5 compressed map header (16 bytes at `map_offset`):
    //   0..4: compressed map bytes
    //   4..10: first_offs (48-bit BE)
    //  10..12: map_crc (16-bit BE)
    //  12: length_bits, 13: hunk_bits, 14: parent_unit_bits, 15: reserved
    // To keep the reader minimal and avoid full-table Huffman decoding for every hunk on huge
    // images, parse the exact v5 compressed map stream up to `want_hunk`.
    let mut mh = [0u8; 16];
    read_at(f, map_offset, &mut mh).map_err(|_| "cannot read CHD v5 map header".to_owned())?;
    let map_len = u32::from_be_bytes(mh[0..4].try_into().unwrap()) as usize;
    let first_offs = (u64::from(mh[4]) << 40)
        | (u64::from(mh[5]) << 32)
        | (u64::from(mh[6]) << 24)
        | (u64::from(mh[7]) << 16)
        | (u64::from(mh[8]) << 8)
        | u64::from(mh[9]);
    let length_bits = mh[12];
    let self_bits = mh[13];
    let parent_bits = mh[14];
    if map_len == 0 || map_len > 16 * 1024 * 1024 {
        return Err("invalid CHD v5 compressed map size".into());
    }
    let mut map_bytes = vec![0u8; map_len];
    read_at(f, map_offset + 16, &mut map_bytes)
        .map_err(|_| "cannot read CHD v5 map bytes".to_owned())?;

    // Decode Huffman-coded compression types for hunks 0..=want_hunk, then read their payloads.
    // CHD v5 stores the RLE-Huffman table of 16 symbols (compression types) first, followed by the
    // bitstream of types, followed by per-hunk length/offset fields for all hunks. Because per-hunk
    // lengths follow ALL compression types, we need total_hunks from the header.
    let mut hdr_full = [0u8; 64];
    read_at(f, 0, &mut hdr_full).map_err(|_| "cannot re-read CHD header".to_owned())?;
    let logical_bytes = u64::from_be_bytes(hdr_full[32..40].try_into().unwrap());
    let total_hunks = logical_bytes.div_ceil(hunk_bytes as u64) as usize;
    if want_hunk >= total_hunks || total_hunks > 2_000_000 {
        return Err("CHD hunk index out of range".into());
    }

    let mut br = BitReader::new(&map_bytes);
    let huff = ChdHuffman::read_rle(&mut br, 16, 8)?;
    let mut comp_types = vec![0u8; total_hunks];
    let mut i = 0usize;
    let mut rep = 0usize;
    let mut last_comp = 0u8;
    while i < total_hunks {
        if rep > 0 {
            comp_types[i] = last_comp;
            rep -= 1;
            i += 1;
            continue;
        }
        let sym = huff.decode(&mut br)? as u8;
        match sym {
            // COMPRESSION_RLE_SMALL = 6, COMPRESSION_RLE_LARGE = 7
            6 => {
                rep = 2 + br.read(4)? as usize;
                comp_types[i] = last_comp;
                i += 1;
            }
            7 => {
                rep = 2 + 16 + br.read(8)? as usize;
                comp_types[i] = last_comp;
                i += 1;
            }
            c => {
                last_comp = c;
                comp_types[i] = c;
                i += 1;
            }
        }
    }

    let mut cur_offset = first_offs;
    let mut target_entry: Option<(u8, u64, usize)> = None;
    for (idx, &ctype) in comp_types.iter().enumerate().take(want_hunk + 1) {
        let (off, len) = match ctype {
            // 0..=3: codecs[0..=3]
            0..=3 => {
                let len = br.read(length_bits)? as usize;
                let o = cur_offset;
                cur_offset += len as u64;
                (o, len)
            }
            // 4: COMPRESSION_NONE (uncompressed)
            4 => {
                let o = cur_offset;
                cur_offset += hunk_bytes as u64;
                (o, hunk_bytes)
            }
            // 5: COMPRESSION_SELF (reference earlier hunk)
            5 => {
                let _ref_hunk = br.read(self_bits)?;
                (0, 0)
            }
            // 8+: parent / mini
            _ => {
                let _ = br.read(parent_bits);
                (0, 0)
            }
        };
        if idx == want_hunk {
            target_entry = Some((ctype, off, len));
        }
    }

    let (ctype, offset, len) = target_entry.ok_or_else(|| "CHD hunk not found".to_owned())?;
    if ctype == 4 {
        let mut raw = vec![0u8; hunk_bytes];
        read_at(f, offset, &mut raw).map_err(|_| "cannot read CHD raw hunk".to_owned())?;
        return Ok(raw);
    }
    if ctype > 3 {
        return Ok(vec![0u8; hunk_bytes]);
    }
    let mut packed = vec![0u8; len];
    read_at(f, offset, &mut packed).map_err(|_| "cannot read CHD compressed hunk".to_owned())?;
    decompress_chd_hunk(codecs[ctype as usize], &packed, hunk_bytes, unit_bytes)
}

fn decompress_chd_hunk(
    codec: &[u8],
    packed: &[u8],
    hunk_bytes: usize,
    unit_bytes: usize,
) -> Result<Vec<u8>, String> {
    let frames = hunk_bytes / unit_bytes;
    let decompress_stream = |tag: &[u8], src: &[u8], out_len: usize| -> Result<Vec<u8>, String> {
        match tag {
            b"zlib" | b"cdzl" => miniz_oxide::inflate::decompress_to_vec(src)
                .map_err(|_| "CHD deflate decode failed".to_owned()),
            b"zstd" | b"cdzs" => {
                let mut dec = ruzstd::decoding::StreamingDecoder::new(src)
                    .map_err(|_| "CHD zstd init failed".to_owned())?;
                let mut out = Vec::with_capacity(out_len);
                dec.read_to_end(&mut out)
                    .map_err(|_| "CHD zstd decode failed".to_owned())?;
                Ok(out)
            }
            b"lzma" | b"cdlz" => {
                // CHD LZMA uses raw LZMA with lc=3, lp=0, pb=2 (prop byte 0x5D) and dict size
                // derived from `out_len` (no 13-byte header in the stream).
                let dict_size = (out_len as u32).clamp(4096, 1 << 24);
                let mut with_hdr = Vec::with_capacity(13 + src.len());
                with_hdr.push(0x5D);
                with_hdr.extend_from_slice(&dict_size.to_le_bytes());
                with_hdr.extend_from_slice(&(out_len as u64).to_le_bytes());
                with_hdr.extend_from_slice(src);
                let mut out = Vec::with_capacity(out_len);
                lzma_rs::lzma_decompress(&mut Cursor::new(&with_hdr), &mut out)
                    .map_err(|_| "CHD lzma decode failed".to_owned())?;
                Ok(out)
            }
            _ => Err(format!(
                "unsupported CHD codec ({})",
                String::from_utf8_lossy(tag)
            )),
        }
    };

    if matches!(codec, b"zlib" | b"zstd" | b"lzma") {
        let out = decompress_stream(codec, packed, hunk_bytes)?;
        return Ok(out);
    }

    // CD codecs (`cdzl`, `cdzs`, `cdlz`):
    //   [ecc_bitmap: ceil(frames/8) bytes]
    //   [base_comp_len: 2 bytes BE if hunk_bytes < 65536 else 3 bytes BE]
    //   [base_stream: base_comp_len bytes -> frames * 2352 bytes]
    //   [subcode_stream: remaining bytes -> frames * 96 bytes]
    if matches!(codec, b"cdzl" | b"cdzs" | b"cdlz") && unit_bytes == 2448 {
        let ecc_bytes = frames.div_ceil(8);
        let len_bytes = if hunk_bytes < 65536 { 2 } else { 3 };
        if packed.len() < ecc_bytes + len_bytes {
            return Err("truncated CD CHD hunk".into());
        }
        let base_len = if len_bytes == 2 {
            u16::from_be_bytes([packed[ecc_bytes], packed[ecc_bytes + 1]]) as usize
        } else {
            ((packed[ecc_bytes] as usize) << 16)
                | ((packed[ecc_bytes + 1] as usize) << 8)
                | (packed[ecc_bytes + 2] as usize)
        };
        let base_start = ecc_bytes + len_bytes;
        if base_start + base_len > packed.len() {
            return Err("corrupt CD CHD base length".into());
        }
        let base = decompress_stream(
            codec,
            &packed[base_start..base_start + base_len],
            frames * 2352,
        )?;
        if base.len() < frames * 2352 {
            return Err("short CD CHD base output".into());
        }
        // Reassemble 2448-byte units (2352 raw sector + 96 zeroed subcode; we only read user data).
        let mut out = vec![0u8; hunk_bytes];
        for fi in 0..frames {
            let dst = &mut out[fi * 2448..fi * 2448 + 2352];
            dst.copy_from_slice(&base[fi * 2352..(fi + 1) * 2352]);
            // Restore Mode 1 sync pattern if ECC bit was set.
            if (packed[fi / 8] & (1 << (fi % 8))) != 0 {
                dst[0..12].copy_from_slice(&[
                    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
                ]);
            }
        }
        return Ok(out);
    }

    Err(format!(
        "unsupported CHD codec ({})",
        String::from_utf8_lossy(codec)
    ))
}

struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read(&mut self, bits: u8) -> Result<u32, String> {
        let mut val = 0u32;
        for _ in 0..bits {
            let byte_idx = self.bit_pos / 8;
            if byte_idx >= self.data.len() {
                return Err("unexpected EOF in CHD bitstream".into());
            }
            let shift = 7 - (self.bit_pos % 8);
            val = (val << 1) | u32::from((self.data[byte_idx] >> shift) & 1);
            self.bit_pos += 1;
        }
        Ok(val)
    }
}

struct ChdHuffman {
    /// Canonical table: `(symbol, code, bit_len)` sorted by `bit_len`.
    codes: Vec<(u16, u32, u8)>,
}

impl ChdHuffman {
    fn read_rle(br: &mut BitReader<'_>, num_codes: usize, max_bits: u8) -> Result<Self, String> {
        let mut lengths = vec![0u8; num_codes];
        let mut cur = 0usize;
        while cur < num_codes {
            let val = br.read(4)? as u8;
            if val != 1 {
                lengths[cur] = val;
                cur += 1;
            } else {
                let next = br.read(4)? as u8;
                if next == 1 {
                    lengths[cur] = 1;
                    cur += 1;
                } else {
                    let count = br.read(4)? as usize + 3;
                    for _ in 0..count {
                        if cur < num_codes {
                            lengths[cur] = next;
                            cur += 1;
                        }
                    }
                }
            }
        }
        // Build canonical Huffman codes.
        let mut codes = Vec::new();
        let mut next_code = 0u32;
        for bits in 1..=max_bits {
            next_code <<= 1;
            for (sym, &len) in lengths.iter().enumerate() {
                if len == bits {
                    codes.push((sym as u16, next_code, bits));
                    next_code += 1;
                }
            }
        }
        if codes.is_empty() {
            return Err("empty CHD Huffman table".into());
        }
        Ok(Self { codes })
    }

    fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, String> {
        let mut acc = 0u32;
        let mut bits = 0u8;
        for &(sym, code, len) in &self.codes {
            while bits < len {
                acc = (acc << 1) | br.read(1)?;
                bits += 1;
            }
            if acc == code {
                return Ok(sym);
            }
        }
        Err("invalid CHD Huffman code".into())
    }
}

/// Extracts the 2048-byte user data payload from a 2048-, 2352-, or 2448-byte CD sector.
fn extract_sector_user_data(sector: &[u8]) -> &[u8] {
    if sector.len() == 2048 {
        return sector;
    }
    if sector.len() >= 2352 {
        // Raw CD-ROM sector: check sync bytes 00 FF..FF 00.
        let sync = sector[0..12]
            == [
                0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
            ];
        if sync {
            // Mode 2 Form 1 (PS1 XA) puts user data at offset 24; Mode 1 puts it at offset 16.
            let mode = sector[15];
            let off = if mode == 2 { 24 } else { 16 };
            return &sector[off..off + 2048];
        }
        return &sector[..2048];
    }
    sector
}

/// Identifies a disc track file (`.bin` / `.iso` from a `.cue` set) by reading sectors 0..18.
fn identify_disc_track_file(path: &Path) -> Option<Console> {
    let mut f = File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    if size < 17 * 2048 {
        return None;
    }
    let mut probe = [0u8; 16];
    read_at(&mut f, 0, &mut probe).ok()?;
    let sync = [
        0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
    ];
    let mut probe16 = [0u8; 16];
    let raw16 = size >= 17 * 2352
        && read_at(&mut f, 16 * 2352, &mut probe16).is_ok()
        && probe16[0..12] == sync;
    let sector_size: u64 = if probe[0..12] == sync || raw16 {
        2352
    } else {
        2048
    };
    let mut sectors = Vec::with_capacity(18);
    let mut buf = vec![0u8; sector_size as usize];
    for s in 0..18u64 {
        if read_at(&mut f, s * sector_size, &mut buf).is_err() {
            break;
        }
        sectors.push(extract_sector_user_data(&buf).to_vec());
    }
    identify_disc_sectors(&sectors)
}

/// Identifies Mega-CD, PlayStation, PSP, or PC Engine CD from decoded 2048-byte user-data sectors.
fn identify_disc_sectors(sectors: &[Vec<u8>]) -> Option<Console> {
    if let Some(s0) = sectors.first() {
        if s0.starts_with(b"SEGADISCSYSTEM  ") {
            return Some(Console::MegaCd);
        }
    }
    for s in sectors.iter().take(4) {
        if contains_subslice(s, b"PC Engine CD-ROM SYSTEM") {
            return Some(Console::PcEngineCd);
        }
    }
    if let Some(s16) = sectors.get(16) {
        if is_iso9660_pvd(s16) {
            let sys_id = &s16[8..40];
            if contains_subslice(sys_id, b"PLAYSTATION") {
                return Some(Console::PlayStation);
            }
            if contains_subslice(sys_id, b"PSP GAME") {
                return Some(Console::Psp);
            }
        }
    }
    None
}

fn is_iso9660_pvd(sector: &[u8]) -> bool {
    sector.len() >= 164 && sector[0] == 1 && &sector[1..6] == b"CD001"
}

fn is_psp_pvd(sector: &[u8]) -> bool {
    is_iso9660_pvd(sector) && contains_subslice(&sector[8..40], b"PSP GAME")
}

fn iso_root_contains(f: &mut File, pvd: &[u8], needles: &[&[u8]]) -> bool {
    // Root directory record starts at byte 156 of PVD; LBA at +2 (LE u32), size at +10 (LE u32).
    if pvd.len() < 190 {
        return false;
    }
    let root_lba = u64::from(u32::from_le_bytes(pvd[158..162].try_into().unwrap()));
    let root_len = u32::from_le_bytes(pvd[166..170].try_into().unwrap()).min(65536) as usize;
    if root_lba == 0 || root_len == 0 {
        return false;
    }
    let mut dir = vec![0u8; root_len];
    if read_at(f, root_lba * 2048, &mut dir).is_err() {
        return false;
    }
    needles.iter().any(|n| contains_subslice(&dir, n))
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

fn ext_lower(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nibos-drop-rom-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_stored_zip(path: &Path, entry_name: &str, data: &[u8]) {
        let mut out = Vec::new();
        let name = entry_name.as_bytes();
        // Local file header
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes()); // version
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u32.to_le_bytes()); // time/date
        out.extend_from_slice(&0u32.to_le_bytes()); // crc32 (not checked)
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        let cd_offset = out.len() as u32;
        // Central directory header
        out.extend_from_slice(b"PK\x01\x02");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // local offset
        out.extend_from_slice(name);
        let cd_size = out.len() as u32 - cd_offset;
        // EOCD
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        fs::write(path, out).unwrap();
    }

    fn make_gb_rom(cgb_flag: u8) -> Vec<u8> {
        let logo: [u8; 48] = [
            0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C,
            0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6,
            0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
            0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
        ];
        let mut rom = vec![0u8; 32 * 1024];
        rom[0x104..0x134].copy_from_slice(&logo);
        rom[0x134..0x13A].copy_from_slice(b"TETRIS");
        rom[0x143] = cgb_flag;
        rom[0x14D] = rom[0x134..0x14D]
            .iter()
            .fold(0u8, |acc, &b| acc.wrapping_sub(b).wrapping_sub(1));
        rom
    }

    fn make_snes_rom(offset: usize, mode: u8, copier: usize) -> Vec<u8> {
        let mut rom = vec![0u8; (offset + 0x50).next_power_of_two() + copier];
        let h = &mut rom[offset + copier..][..0x50];
        h[0x10..0x25].fill(b' ');
        h[0x10..0x18].copy_from_slice(b"MOON BAY");
        h[0x25] = mode;
        h[0x2C..0x30].copy_from_slice(&[0xCB, 0xED, 0x34, 0x12]);
        h[0x4C..0x4E].copy_from_slice(&0x8000u16.to_le_bytes());
        rom
    }

    fn make_pbp(category: &str) -> Vec<u8> {
        let mut sfo = Vec::new();
        sfo.extend_from_slice(b"\x00PSF");
        sfo.extend_from_slice(&0x0000_0101u32.to_le_bytes());
        let key_table = 20 + 16u32;
        let data_table = key_table + 12; // "CATEGORY\0" padded
        sfo.extend_from_slice(&key_table.to_le_bytes());
        sfo.extend_from_slice(&data_table.to_le_bytes());
        sfo.extend_from_slice(&1u32.to_le_bytes());
        // Entry 0
        sfo.extend_from_slice(&0u16.to_le_bytes()); // key_off
        sfo.extend_from_slice(&0x0204u16.to_le_bytes()); // utf8
        sfo.extend_from_slice(&(category.len() as u32 + 1).to_le_bytes());
        sfo.extend_from_slice(&4u32.to_le_bytes());
        sfo.extend_from_slice(&0u32.to_le_bytes()); // data_off
                                                    // Key table
        sfo.extend_from_slice(b"CATEGORY\0\0\0\0");
        // Data table
        let mut cat = category.as_bytes().to_vec();
        cat.resize(4, 0);
        sfo.extend_from_slice(&cat);

        let mut pbp = vec![0u8; 40];
        pbp[0..4].copy_from_slice(b"\x00PBP");
        pbp[8..12].copy_from_slice(&40u32.to_le_bytes());
        pbp[12..16].copy_from_slice(&(40 + sfo.len() as u32).to_le_bytes());
        pbp.extend_from_slice(&sfo);
        pbp
    }

    #[test]
    fn stage1_rejects_non_roms_and_groups_cue_sets() {
        let plan = stage1_plan(&[
            Candidate {
                id: "1",
                file_name: "photo.jpg",
                size: 1000,
            },
            Candidate {
                id: "2",
                file_name: "manual.pdf",
                size: 2000,
            },
            Candidate {
                id: "3",
                file_name: "Metroid.gba",
                size: 8_000_000,
            },
            Candidate {
                id: "4",
                file_name: "Castlevania.cue",
                size: 120,
            },
            Candidate {
                id: "5",
                file_name: "Castlevania.bin",
                size: 400_000_000,
            },
            Candidate {
                id: "6",
                file_name: ".DS_Store",
                size: 6148,
            },
            Candidate {
                id: "7",
                file_name: "../escape.nds",
                size: 1000,
            },
        ]);
        assert_eq!(plan.accepted.len(), 3);
        assert_eq!(plan.rejected.len(), 4);
        assert_eq!(plan.accepted[0].role, FileRole::Single);
        assert_eq!(plan.accepted[1].role, FileRole::DiscSetMember);
        assert_eq!(plan.accepted[2].role, FileRole::DiscSetMember);
    }

    #[test]
    fn classifies_cartridge_roms_and_rejects_fakes() {
        let dir = tmp_dir("carts");
        let idx = EmulatorIndex::default();

        // NES
        let mut nes = vec![0u8; 16 + 16384];
        nes[..4].copy_from_slice(b"NES\x1a");
        let p = dir.join("Contra.nes");
        fs::write(&p, &nes).unwrap();
        assert_eq!(
            classify_file(&p, "Contra.nes", &idx).unwrap().console,
            Console::Nes
        );

        // Fake NES (plain text renamed .nes)
        let fake = dir.join("fake.nes");
        fs::write(&fake, b"not a rom at all, just some text padding...").unwrap();
        assert!(classify_file(&fake, "fake.nes", &idx).is_err());

        // GB vs GBC-only
        let gb = dir.join("Tetris.gb");
        fs::write(&gb, make_gb_rom(0x00)).unwrap();
        assert_eq!(
            classify_file(&gb, "Tetris.gb", &idx).unwrap().console,
            Console::Gb
        );
        let gbc_flag = dir.join("Shantae.gb");
        fs::write(&gbc_flag, make_gb_rom(0xC0)).unwrap();
        assert_eq!(
            classify_file(&gbc_flag, "Shantae.gb", &idx)
                .unwrap()
                .console,
            Console::Gbc
        );

        // SNES + zipped SNES
        let sfc_bytes = make_snes_rom(0x7FB0, 0x20, 512);
        let sfc = dir.join("Moon.smc");
        fs::write(&sfc, &sfc_bytes).unwrap();
        assert_eq!(
            classify_file(&sfc, "Moon.smc", &idx).unwrap().console,
            Console::Snes
        );

        let zip_sfc = dir.join("Moon.zip");
        write_stored_zip(&zip_sfc, "Moon.smc", &sfc_bytes);
        assert_eq!(
            classify_file(&zip_sfc, "Moon.zip", &idx).unwrap().console,
            Console::Snes
        );

        // Non-ROM zip rejected
        let bad_zip = dir.join("vacation.zip");
        write_stored_zip(&bad_zip, "beach.jpg", b"\xFF\xD8\xFF\xE0not a rom");
        assert!(classify_file(&bad_zip, "vacation.zip", &idx).is_err());

        // Mega Drive vs Sega Pico vs random .bin
        let mut md = vec![0u8; 0x400];
        md[0x100..0x10F].copy_from_slice(b"SEGA MEGA DRIVE");
        let md_p = dir.join("Sonic.bin");
        fs::write(&md_p, &md).unwrap();
        assert_eq!(
            classify_file(&md_p, "Sonic.bin", &idx).unwrap().console,
            Console::MegaDrive
        );

        let mut pico = vec![0u8; 0x400];
        pico[0x100..0x109].copy_from_slice(b"SEGA PICO");
        let pico_p = dir.join("Story.md");
        fs::write(&pico_p, &pico).unwrap();
        assert_eq!(
            classify_file(&pico_p, "Story.md", &idx).unwrap().console,
            Console::Pico
        );

        let rand_bin = dir.join("firmware.bin");
        fs::write(&rand_bin, vec![0x42u8; 4096]).unwrap();
        assert!(classify_file(&rand_bin, "firmware.bin", &idx).is_err());

        // Lynx (headered & headerless)
        let mut lnx = vec![0u8; 128];
        lnx[..4].copy_from_slice(b"LYNX");
        let lnx_p = dir.join("Chip.lnx");
        fs::write(&lnx_p, &lnx).unwrap();
        assert_eq!(
            classify_file(&lnx_p, "Chip.lnx", &idx).unwrap().console,
            Console::Lynx
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn classifies_pbp_iso_cue_and_mame_sets() {
        let dir = tmp_dir("discs");
        let mut idx = EmulatorIndex::default();
        idx.arcade.insert("mslug".into());
        idx.game_watch.insert("gnw_dkong".into());

        // PBP PS1 vs PSP
        let ps1 = dir.join("FF7.pbp");
        fs::write(&ps1, make_pbp("ME")).unwrap();
        assert_eq!(
            classify_file(&ps1, "FF7.pbp", &idx).unwrap().console,
            Console::PlayStation
        );

        let psp = dir.join("EBOOT.PBP");
        fs::write(&psp, make_pbp("UG")).unwrap();
        let c = classify_file(&psp, "EBOOT.PBP", &idx).unwrap();
        assert_eq!(c.console, Console::Psp);
        assert!(c.preserve_subdir);

        // ISO: Mega-CD vs PSP vs random Linux ISO
        let mut mcd = vec![0u8; 17 * 2048];
        mcd[0..16].copy_from_slice(b"SEGADISCSYSTEM  ");
        let mcd_p = dir.join("SonicCD.iso");
        fs::write(&mcd_p, &mcd).unwrap();
        assert_eq!(
            classify_file(&mcd_p, "SonicCD.iso", &idx).unwrap().console,
            Console::MegaCd
        );

        let mut psp_iso = vec![0u8; 17 * 2048];
        let pvd = &mut psp_iso[16 * 2048..17 * 2048];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[8..16].copy_from_slice(b"PSP GAME");
        let psp_p = dir.join("Lumines.iso");
        fs::write(&psp_p, &psp_iso).unwrap();
        assert_eq!(
            classify_file(&psp_p, "Lumines.iso", &idx).unwrap().console,
            Console::Psp
        );

        let mut linux_iso = vec![0u8; 17 * 2048];
        linux_iso[16 * 2048] = 1;
        linux_iso[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"CD001");
        linux_iso[16 * 2048 + 8..16 * 2048 + 13].copy_from_slice(b"LINUX");
        let lin_p = dir.join("arch.iso");
        fs::write(&lin_p, &linux_iso).unwrap();
        assert!(classify_file(&lin_p, "arch.iso", &idx).is_err());

        // CUE + BIN (PS1)
        let cue_p = dir.join("Symphony.cue");
        fs::write(
            &cue_p,
            "FILE \"Symphony.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        let mut ps_bin = vec![0u8; 18 * 2352];
        let s16 = &mut ps_bin[16 * 2352..17 * 2352];
        s16[0..12].copy_from_slice(&[
            0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
        ]);
        s16[15] = 2; // Mode 2
        s16[24] = 1;
        s16[25..30].copy_from_slice(b"CD001");
        s16[32..43].copy_from_slice(b"PLAYSTATION");
        let bin_p = dir.join("Symphony.bin");
        fs::write(&bin_p, &ps_bin).unwrap();
        let mut staged = HashMap::new();
        staged.insert("symphony.cue".into(), ("Symphony.cue".into(), cue_p));
        staged.insert("symphony.bin".into(), ("Symphony.bin".into(), bin_p));
        assert_eq!(classify_disc_set(&staged).unwrap(), Console::PlayStation);

        // Arcade & G&W MAME zip sets
        let mslug = dir.join("mslug.zip");
        write_stored_zip(&mslug, "201-p1.bin", &[0x00; 32]);
        assert_eq!(
            classify_file(&mslug, "mslug.zip", &idx).unwrap().console,
            Console::Arcade
        );

        let gw = dir.join("gnw_dkong.zip");
        write_stored_zip(&gw, "dkong.rom", &[0x00; 32]);
        assert_eq!(
            classify_file(&gw, "gnw_dkong.zip", &idx).unwrap().console,
            Console::GameWatch
        );

        // Unknown 7z rejected, known Arcade 7z accepted
        let mut f = File::create(dir.join("mslug.7z")).unwrap();
        f.write_all(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0, 4])
            .unwrap();
        assert_eq!(
            classify_file(&dir.join("mslug.7z"), "mslug.7z", &idx)
                .unwrap()
                .console,
            Console::Arcade
        );
        let mut f2 = File::create(dir.join("unknown.7z")).unwrap();
        f2.write_all(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0, 4])
            .unwrap();
        assert!(classify_file(&dir.join("unknown.7z"), "unknown.7z", &idx).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    fn write_deflated_zip(
        path: &Path,
        entry_name: &str,
        data: &[u8],
        declared_crc: u32,
        declared_usize: u32,
    ) {
        let comp = miniz_oxide::deflate::compress_to_vec(data, 6);
        let name = entry_name.as_bytes();
        let mut out = Vec::new();
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes()); // deflate
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&declared_crc.to_le_bytes());
        out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        out.extend_from_slice(&declared_usize.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&comp);
        let cd_offset = out.len() as u32;
        out.extend_from_slice(b"PK\x01\x02");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&declared_crc.to_le_bytes());
        out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        out.extend_from_slice(&declared_usize.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name);
        let cd_size = out.len() as u32 - cd_offset;
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        fs::write(path, out).unwrap();
    }

    #[test]
    fn deflated_non_power_of_two_snes_zip_succeeds_and_corrupt_zip_is_rejected() {
        let dir = tmp_dir("deflate-snes");
        let idx = EmulatorIndex::default();

        // 1.5 MiB SNES ROM filled with pseudo-random bytes so compressed size > 64 KiB
        // (exercises multi-chunk streaming + non-power-of-two buffer).
        let mut rom = vec![0u8; 1536 * 1024];
        let mut x = 0x1234_5678u32;
        for b in &mut rom {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (x >> 24) as u8;
        }
        let h = &mut rom[0x7FB0..][..0x50];
        h[0x10..0x25].fill(b' ');
        h[0x10..0x17].copy_from_slice(b"ALADDIN");
        h[0x25] = 0x20;
        h[0x2C..0x30].copy_from_slice(&[0xCB, 0xED, 0x34, 0x12]);
        h[0x4C..0x4E].copy_from_slice(&0x8000u16.to_le_bytes());

        let crc = crc32_ieee(&rom);
        let ok_zip = dir.join("Aladdin (USA).zip");
        write_deflated_zip(&ok_zip, "Aladdin (USA).sfc", &rom, crc, rom.len() as u32);
        assert_eq!(
            classify_file(&ok_zip, "Aladdin (USA).zip", &idx)
                .unwrap()
                .console,
            Console::Snes
        );

        // Bad CRC rejected
        let bad_crc = dir.join("BadCrc.zip");
        write_deflated_zip(&bad_crc, "Bad.sfc", &rom, crc ^ 1, rom.len() as u32);
        assert!(classify_file(&bad_crc, "BadCrc.zip", &idx).is_err());

        // Stream longer than declared uncompressed size (like a damaged TorrentZip) rejected
        let bad_len = dir.join("Axelay.zip");
        write_deflated_zip(&bad_len, "Axelay.sfc", &rom, crc, (rom.len() - 505) as u32);
        assert!(classify_file(&bad_len, "Axelay.zip", &idx).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
