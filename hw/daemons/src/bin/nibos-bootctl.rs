//! nibos-bootctl: the A/B slots' boot control block, which SPL reads at power-on to pick the
//! slot it boots ([`nibos_hw::ab`], `docs/design/ab-rollback.md`).
//!
//!   nibos-bootctl [status]          the active slot, each slot's tries left and whether it's good
//!   nibos-bootctl unconfirmed       exit 0 if the running slot isn't confirmed good yet, else 1
//!   nibos-bootctl init [--force]    a new block: slot A active and good (refused over a valid one)
//!   nibos-bootctl good              the running slot is good
//!   nibos-bootctl confirm-when-up   nibos-boot-good.service: `good` once the boot UI is up (the
//!                                   splash saw both screens taken over; at most 2 minutes'
//!                                   wait) and, 30 s later, still running (not in the safe mode)
//!   nibos-bootctl try A|B           boot that slot next, with 3 tries, then back to the other
//!   nibos-bootctl rollback          back to the other slot, if it's good (the safe mode)
//!   nibos-bootctl write-fit A|B FILE    a Falcon FIT into that slot's place
//!
//! The block lives in the board's Falcon partition (the one on the root's disk that starts at
//! `NIBOS_FALCON_SECTOR`), at its end; slot A's FIT at its start, B's 32 MiB in. Both copies are
//! written, the older first, each synced, so a power cut leaves one whole.

use nibos_hw::{
    ab::{Block, Slot, BLOCK_OFFSET, SECTOR, SLOT_B_OFFSET},
    board::parse_env,
};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
    process::exit,
};

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("nibos-bootctl: {msg}");
    exit(2);
}

/// The Falcon partition: on the root's disk, starting at NIBOS_FALCON_SECTOR.
fn partition() -> PathBuf {
    let conf = fs::read_to_string("/usr/lib/nibos/board.conf").unwrap_or_default();
    let start: u64 = parse_env(&conf)
        .get("NIBOS_FALCON_SECTOR")
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| fail("board.conf has no NIBOS_FALCON_SECTOR"));
    // The root's partition by its device number (/proc/self/mounts may say /dev/root), and
    // its disk, the directory above it in sysfs.
    let dev = fs::metadata("/")
        .unwrap_or_else(|e| fail(format!("/: {e}")))
        .dev();
    // Linux's encoding of a device number (as glibc's major() and minor()).
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    let disk = fs::canonicalize(format!("/sys/dev/block/{major}:{minor}/.."))
        .unwrap_or_else(|e| fail(format!("the root's disk ({major}:{minor}): {e}")));
    for e in fs::read_dir(&disk).into_iter().flatten().flatten() {
        let part = e.file_name().to_string_lossy().into_owned();
        if !part.starts_with(&*disk.file_name().unwrap().to_string_lossy()) {
            continue;
        }
        let s = fs::read_to_string(e.path().join("start")).unwrap_or_default();
        if s.trim().parse::<u64>().ok() == Some(start) {
            return PathBuf::from(format!("/dev/{part}"));
        }
    }
    fail(format!(
        "no partition at sector {start} on {}",
        disk.display()
    ))
}

fn open(write: bool) -> fs::File {
    let p = partition();
    fs::OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(if write { libc::O_SYNC } else { 0 })
        .open(&p)
        .unwrap_or_else(|e| fail(format!("{}: {e}", p.display())))
}

fn read_copies(f: &mut fs::File) -> ([u8; SECTOR], [u8; SECTOR]) {
    let mut both = [0u8; 2 * SECTOR];
    f.seek(SeekFrom::Start(BLOCK_OFFSET * SECTOR as u64))
        .and_then(|_| f.read_exact(&mut both))
        .unwrap_or_else(|e| fail(format!("reading the block: {e}")));
    let (a, b) = both.split_at(SECTOR);
    (a.try_into().unwrap(), b.try_into().unwrap())
}

fn read() -> Option<Block> {
    let (a, b) = read_copies(&mut open(false));
    Block::from_copies(&a, &b)
}

/// Both copies, the older first, each synced.
fn write(block: &Block) {
    let mut f = open(true);
    let (a, b) = read_copies(&mut f);
    let first = Block::older_copy(&a, &b) as u64;
    let s = block.encode();
    for copy in [first, 1 - first] {
        f.seek(SeekFrom::Start((BLOCK_OFFSET + copy) * SECTOR as u64))
            .and_then(|_| f.write_all(&s))
            .and_then(|_| f.sync_all())
            .unwrap_or_else(|e| fail(format!("writing the block: {e}")));
    }
}

fn need() -> Block {
    read().unwrap_or_else(|| fail("no boot control block (nibos-bootctl init)"))
}

fn show(b: &Block) {
    println!("active: {}", b.active.name());
    for s in [Slot::A, Slot::B] {
        println!(
            "slot {}: {}, {} tries left",
            s.name(),
            if b.good[s.index()] {
                "good"
            } else {
                "not confirmed"
            },
            b.tries[s.index()]
        );
    }
}

fn slot(arg: Option<&String>) -> Slot {
    arg.and_then(|s| Slot::parse(s))
        .unwrap_or_else(|| fail("give a slot: A or B"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => match read() {
            Some(b) => show(&b),
            None => println!("no boot control block: SPL boots slot A"),
        },
        "unconfirmed" => exit(match read() {
            Some(b) if !b.good[b.active.index()] => 0,
            _ => 1,
        }),
        "init" => {
            if read().is_some() && args.get(1).map(String::as_str) != Some("--force") {
                fail("there is a block already (--force to start again)");
            }
            write(&Block::new());
            show(&Block::new());
        }
        "good" => {
            let b = need();
            if !b.good[b.active.index()] {
                write(&b.mark_good());
                eprintln!("nibos-bootctl: slot {} is good", b.active.name());
            }
        }
        "confirm-when-up" => {
            if !matches!(read(), Some(b) if !b.good[b.active.index()]) {
                return;
            }
            let taken = std::path::Path::new("/run/nibos/screens-taken");
            for _ in 0..120 {
                if taken.exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            std::thread::sleep(std::time::Duration::from_secs(30));
            let systemctl = |args: &[&str]| {
                std::process::Command::new("systemctl")
                    .args(args)
                    .status()
                    .is_ok_and(|s| s.success())
            };
            if systemctl(&["-q", "is-enabled", "dslot.service"])
                && !systemctl(&["-q", "is-active", "dslot.service"])
            {
                println!("the boot UI isn't running: the slot stays unconfirmed");
                return;
            }
            let b = need();
            write(&b.mark_good());
            println!("slot {} is good", b.active.name());
        }
        "try" => {
            let b = need().try_slot(slot(args.get(1)));
            write(&b);
            show(&b);
        }
        "rollback" => match need().rollback() {
            Some(b) => {
                write(&b);
                show(&b);
            }
            None => fail("the other slot isn't good: nothing to go back to"),
        },
        "write-fit" => {
            let s = slot(args.get(1));
            let file = args.get(2).unwrap_or_else(|| fail("give the FIT file"));
            let fit = fs::read(file).unwrap_or_else(|e| fail(format!("{file}: {e}")));
            if fit.len() as u64 > SLOT_B_OFFSET * SECTOR as u64 {
                fail("the FIT is larger than a slot's 32 MiB");
            }
            let at = if s == Slot::B { SLOT_B_OFFSET } else { 0 } * SECTOR as u64;
            let mut f = open(true);
            f.seek(SeekFrom::Start(at))
                .and_then(|_| f.write_all(&fit))
                .and_then(|_| f.sync_all())
                .unwrap_or_else(|e| fail(format!("writing slot {}'s FIT: {e}", s.name())));
            println!("slot {}: {} bytes", s.name(), fit.len());
        }
        _ => fail("usage: nibos-bootctl [status|unconfirmed|init|good|confirm-when-up|try A|B|rollback|write-fit A|B FILE]"),
    }
}
