//! nibos-splash: the kernel's `init=` on nibOS, and its goodbye at power-off.
//!
//! As init (PID 1), it forks, and execs systemd at once; the child puts the boot picture on the
//! screens straight through KMS, and keeps the picture's framebuffers alive until something
//! else (DSlot, or the console's login prompt in the base edition) has set both outputs, or
//! 30 s have passed. No picture when the charger switched the device on: it charges asleep,
//! dark (`nibos-charge-boot`).
//!
//!   nibos-splash --goodbye     nibos-goodbye.service, at power-off and restart, once the boot
//!                              UI has let go of the screens: the goodbye picture, held until
//!                              the power goes (not after a charge-sleep: that stays dark)
//!   nibos-splash --hold PICTURE [SECONDS] [--while PID]
//!                              a program's picture over a handover, as the user: up while no
//!                              one has the screens, then let go of them (the next program
//!                              becomes their master), kept until the top screen shows
//!                              something else or SECONDS (30) have passed. With --while, kept
//!                              (card open) for as long as process PID runs, then until the
//!                              top screen shows something else again: the program's SDL puts
//!                              this picture back as it quits, and the frontend (DSlot) then
//!                              replaces it, with no dark frame between. PortMaster's
//!                              "Starting PortMaster..." while its own app starts.
//!
//! As init it runs before anything is mounted but the root file system and devtmpfs, so it is
//! a static binary with no files to load but the picture. Whatever goes wrong, PID 1 still
//! execs systemd.
//!
//! The pictures (`/usr/lib/nibos/pictures/{boot,goodbye}.pspl`, made from `pictures/` by
//! `nibos-pspl`; DSlot's `/var/lib/dslot/splash` if nibOS's boot picture is missing) are
//! "PSPL", a version (1) and a count, then for each screen, top first, width, height, a byte
//! count and an LZ4 block of XRGB8888 rows (all u32 little-endian).
//!
//! It mounts /run (systemd leaves a mounted /run alone, as after an initrd) to leave
//! /run/nibos/splash for DSlot and bench.sh: when the picture went up, CLOCK_BOOTTIME. It mounts
//! /sys too, to set each backlight to the level nibos-hw saved for it
//! (/var/lib/nibos/backlight/NAME) before the picture goes up: udev, which sets them otherwise,
//! runs seconds later.

mod kms;

use kms::{Kms, Screen};
use std::ffi::CString;

const SYSTEMD: &str = "/usr/lib/systemd/systemd";
const PICTURES: [&str; 2] = ["/usr/lib/nibos/pictures/boot.pspl", "/var/lib/dslot/splash"];
const GOODBYE: [&str; 1] = ["/usr/lib/nibos/pictures/goodbye.pspl"];
/// Left by nibos-charge-boot while the device charges asleep.
const CHARGE_BOOT: &str = "/run/nibos/charge-boot";
/// Left by the goodbye's holder once the picture is up.
const GOODBYE_UP: &str = "/run/nibos/goodbye";
/// The board's description: what switched the device on (`NIBOS_POWER_ON_SOURCE`).
const BOARD: [&str; 2] = ["/usr/lib/nibos/board.conf", "/etc/nibos/board.conf"];
const HOLD_SECS: u32 = 30;
/// Saved backlight levels, one file per backlight, named as in /sys/class/backlight.
const BACKLIGHT_LEVELS: &str = "/var/lib/nibos/backlight";

fn main() {
    if std::process::id() != 1 {
        match std::env::args().nth(1).as_deref() {
            Some("--goodbye") => goodbye(),
            Some("--goodbye-hold") => goodbye_hold(),
            Some("--hold") => {
                let mut args = std::env::args().skip(2);
                match args.next() {
                    Some(picture) => {
                        let rest: Vec<String> = args.collect();
                        let secs = rest
                            .first()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(HOLD_SECS);
                        let pid = rest
                            .iter()
                            .position(|a| a == "--while")
                            .and_then(|i| rest.get(i + 1)?.parse::<i32>().ok());
                        if let Err(e) = hold_picture(&picture, secs, pid) {
                            eprintln!("nibos-splash: {picture}: {e}");
                            std::process::exit(1);
                        }
                    }
                    None => eprintln!("usage: nibos-splash --hold PICTURE [SECONDS] [--while PID]"),
                }
            }
            _ => eprintln!(
                "usage: nibos-splash --goodbye | --hold PICTURE [SECONDS] [--while PID] (as init: no arguments of its own)"
            ),
        }
        return;
    }
    if PICTURES.iter().any(|p| std::path::Path::new(p).exists()) {
        // Before systemd looks: it leaves a mounted /run and /sys alone. Mounted here, in
        // PID 1 before the fork, so the child doesn't race systemd for /sys.
        mount_run();
        mount_sys();
        // The framebuffers live as long as the DRM file, and systemd closes the fds it
        // inherits, so the picture is put up and kept by a child. Closing the card before DSlot
        // has replaced the picture on every screen would turn the outputs off; after it, with
        // no one else holding the card, gives them back to the console.
        if unsafe { libc::fork() } == 0 {
            let t0 = boottime();
            if by_charger() {
                kmsg("nibos-splash: switched on by the charger: no picture (it charges asleep)");
                unsafe { libc::_exit(0) };
            }
            restore_backlights();
            match std::panic::catch_unwind(|| splash(t0)) {
                Ok(Ok(kms)) => hold(&kms),
                Ok(Err(e)) => kmsg(&format!("nibos-splash: {e}")),
                Err(_) => kmsg("nibos-splash: panicked"),
            }
            unsafe { libc::_exit(0) };
        }
    }
    // The kernel's arguments and environment for init go to systemd as they came.
    let args: Vec<CString> = std::iter::once(CString::new(SYSTEMD).unwrap())
        .chain(
            std::env::args_os()
                .skip(1)
                .filter_map(|a| CString::new(a.into_encoded_bytes()).ok()),
        )
        .collect();
    let mut argv: Vec<*const libc::c_char> = args.iter().map(|a| a.as_ptr()).collect();
    argv.push(std::ptr::null());
    unsafe { libc::execv(argv[0], argv.as_ptr()) };
    kmsg(&format!(
        "nibos-splash: exec {SYSTEMD}: {}",
        std::io::Error::last_os_error()
    ));
    let sbin = CString::new("/sbin/init").unwrap();
    argv[0] = sbin.as_ptr();
    unsafe { libc::execv(argv[0], argv.as_ptr()) };
    // Nothing left to run: PID 1 exiting panics the kernel, which says why on the console.
}

fn splash(t0: f64) -> Result<Kms, String> {
    let screens = read_picture(&PICTURES)?;
    let t_read = boottime();
    let kms = Kms::open()?;
    let n = kms.show(&screens)?;
    let t = boottime();
    // Let DSlot (SDL) become master; what is on screen stays there.
    kms.drop_master();
    let _ = std::fs::create_dir_all("/run/nibos");
    let _ = std::fs::write("/run/nibos/splash", format!("{t:.3}\n"));
    kmsg(&format!(
        "nibos-splash: {t:.3} s picture on {n} screens (started {t0:.3}, read {:.0} ms, shown {:.0} ms)",
        (t_read - t0) * 1e3,
        (t - t_read) * 1e3
    ));
    Ok(kms)
}

fn read_picture(paths: &[&str]) -> Result<Vec<Screen>, String> {
    let (path, data) = paths
        .iter()
        .find_map(|p| std::fs::read(p).ok().map(|d| (p, d)))
        .ok_or("no picture")?;
    let bad = || format!("{path}: not a picture");
    let u32_at = |at: usize| -> Result<usize, String> {
        Ok(u32::from_le_bytes(data.get(at..at + 4).ok_or_else(bad)?.try_into().unwrap()) as usize)
    };
    if data.get(..4) != Some(b"PSPL") || u32_at(4)? != 1 {
        return Err(bad());
    }
    let mut at = 12;
    let mut screens = Vec::new();
    for _ in 0..u32_at(8)?.min(2) {
        let (w, h, len) = (u32_at(at)?, u32_at(at + 4)?, u32_at(at + 8)?);
        at += 12;
        if w > 4096 || h > 4096 {
            return Err(bad());
        }
        let mut px = vec![0u8; w * h * 4];
        let block = data.get(at..at + len).ok_or_else(bad)?;
        lz4_flex::block::decompress_into(block, &mut px).map_err(|e| format!("{path}: {e}"))?;
        at += len;
        screens.push(Screen { w, h, px });
    }
    Ok(screens)
}

/// Whether the PMIC says the charger, and not the power key, switched the device on
/// (`NIBOS_POWER_ON_SOURCE="BUS ADDR REG CHARGER-BIT KEY-BIT"`, as nibos-charge-boot reads it).
/// False whenever it can't tell: the picture goes up.
fn by_charger() -> bool {
    let conf: String = BOARD
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let Some(value) = conf
        .lines()
        .filter_map(|l| l.trim().strip_prefix("NIBOS_POWER_ON_SOURCE="))
        .next_back()
    else {
        return false;
    };
    let num = |s: &str| match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    };
    let f: Vec<u32> = value
        .trim_matches(|c| c == '"' || c == '\'')
        .split_whitespace()
        .filter_map(num)
        .collect();
    let [bus, addr, reg, charger, key] = f[..] else {
        return false;
    };
    match i2c_read(bus, addr as u16, reg as u8) {
        Some(v) => v & charger as u8 != 0 && v & key as u8 == 0,
        None => false,
    }
}

/// One byte from a register of an I2C device, as one write-then-read transaction (I2C_RDWR),
/// so it can't interleave with the kernel's own accesses.
fn i2c_read(bus: u32, addr: u16, reg: u8) -> Option<u8> {
    #[repr(C)]
    struct Msg {
        addr: u16,
        flags: u16,
        len: u16,
        buf: *mut u8,
    }
    #[repr(C)]
    struct RdWr {
        msgs: *mut Msg,
        nmsgs: u32,
    }
    let path = CString::new(format!("/dev/i2c-{bus}")).ok()?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let (mut reg, mut value) = (reg, 0u8);
    let mut msgs = [
        Msg {
            addr,
            flags: 0,
            len: 1,
            buf: &mut reg,
        },
        Msg {
            addr,
            flags: 1, // I2C_M_RD
            len: 1,
            buf: &mut value,
        },
    ];
    let mut data = RdWr {
        msgs: msgs.as_mut_ptr(),
        nmsgs: 2,
    };
    let r = unsafe { libc::ioctl(fd, 0x0707 as _, &mut data) }; // I2C_RDWR
    unsafe { libc::close(fd) };
    (r >= 0).then_some(value)
}

/// At power-off or restart (systemd says "stopping"), unless the device was charging asleep:
/// starts the holder, out of this service's cgroup so that stopping the service doesn't kill
/// it, and waits (up to 2 s) for its picture to go up.
fn goodbye() {
    if std::path::Path::new(CHARGE_BOOT).exists() {
        return;
    }
    let stopping = std::process::Command::new("systemctl")
        .arg("is-system-running")
        .output()
        .is_ok_and(|o| o.stdout.starts_with(b"stopping"));
    if !stopping {
        eprintln!("nibos-splash: not shutting down: no goodbye");
        return;
    }
    let _ = std::fs::remove_file(GOODBYE_UP);
    if unsafe { libc::fork() } == 0 {
        // The root cgroup, outside the service's; then a name starting with '@', which
        // systemd's last kill at shutdown passes over (as for root storage daemons).
        let _ = std::fs::write(
            "/sys/fs/cgroup/cgroup.procs",
            format!("{}\n", std::process::id()),
        );
        unsafe { libc::setsid() };
        let exe = CString::new("/proc/self/exe").unwrap();
        let argv = [
            c"@nibos-goodbye".as_ptr(),
            c"--goodbye-hold".as_ptr(),
            std::ptr::null(),
        ];
        unsafe { libc::execv(exe.as_ptr(), argv.as_ptr()) };
        unsafe { libc::_exit(1) };
    }
    for _ in 0..200 {
        if std::path::Path::new(GOODBYE_UP).exists() {
            return;
        }
        unsafe { libc::usleep(10_000) };
    }
    eprintln!("nibos-splash: the goodbye picture didn't go up");
}

/// A program's picture over a handover (`--hold`): shown while no one is the screens' master
/// (the program before it has quit), then mastership let go, so the program after it gets the
/// screens when it opens them; the card is kept open (and the picture with it) until the top
/// screen shows something else, or `secs` have passed.
///
/// With `pid`, the picture's framebuffers also outlive that program: SDL puts back, as it quits,
/// the framebuffers that were on screen when it started (these), and a frontend replaces them
/// a moment later. Freed before then, the panel would go dark in between. So once something
/// else shows, the card stays open while `pid` runs (however long), then until the top screen
/// shows something other than this picture again, for 5 s at most.
fn hold_picture(path: &str, secs: u32, pid: Option<i32>) -> Result<(), String> {
    let screens = read_picture(&[path])?;
    let kms = Kms::open()?;
    kms.show(&screens)?;
    kms.drop_master();
    let top = kms.outputs.first().ok_or("no output")?;
    let ours = kms.fb(top.crtc);
    let running = |pid: i32| unsafe { libc::kill(pid, 0) } == 0;
    let mut replaced = false;
    for _ in 0..secs * 100 {
        if kms.fb(top.crtc) != ours {
            replaced = true;
            break;
        }
        if pid.is_some_and(|p| !running(p)) {
            break;
        }
        unsafe { libc::usleep(10_000) };
    }
    let Some(pid) = pid else {
        return Ok(());
    };
    if !replaced {
        return Ok(());
    }
    while running(pid) {
        unsafe { libc::usleep(100_000) };
    }
    for _ in 0..500 {
        let now = kms.fb(top.crtc);
        if now != ours && now != 0 {
            break;
        }
        unsafe { libc::usleep(10_000) };
    }
    Ok(())
}

/// The holder: the goodbye picture on the screens, kept until the power goes.
fn goodbye_hold() {
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let shown = read_picture(&GOODBYE).and_then(|screens| {
        let kms = Kms::open()?;
        kms.show(&screens)?;
        Ok(kms)
    });
    match shown {
        Ok(_kms) => {
            let _ = std::fs::write(GOODBYE_UP, "");
            loop {
                unsafe { libc::pause() };
            }
        }
        Err(e) => {
            eprintln!("nibos-splash: goodbye: {e}");
            let _ = std::fs::write(GOODBYE_UP, "");
        }
    }
}

fn boottime() -> f64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
}

/// To the kernel's log (dmesg, and the journal's kernel messages).
fn kmsg(msg: &str) {
    let _ = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/kmsg")
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(format!("<6>{msg}\n").as_bytes())
        });
}

/// /run as systemd would mount it.
fn mount_run() {
    let (src, dir, fs, opts) = (
        c"tmpfs",
        c"/run",
        c"tmpfs",
        c"mode=0755,size=20%,nr_inodes=800k",
    );
    let flags = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_STRICTATIME;
    unsafe {
        libc::mount(
            src.as_ptr(),
            dir.as_ptr(),
            fs.as_ptr(),
            flags,
            opts.as_ptr().cast(),
        )
    };
}

fn mount_sys() {
    let flags = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;
    unsafe {
        libc::mount(
            c"sysfs".as_ptr(),
            c"/sys".as_ptr(),
            c"sysfs".as_ptr(),
            flags,
            std::ptr::null(),
        )
    };
}

/// Each backlight back at its saved level. The levels were kept between the board's floor and
/// the maximum when they were saved (/usr/lib/nibos/backlight); a level of 0 or above the
/// maximum is left alone, whatever wrote it.
fn restore_backlights() {
    let Ok(entries) = std::fs::read_dir(BACKLIGHT_LEVELS) else {
        return;
    };
    for entry in entries.flatten() {
        let dev = std::path::Path::new("/sys/class/backlight").join(entry.file_name());
        let read = |p: &std::path::Path| -> Option<u32> {
            std::fs::read_to_string(p).ok()?.trim().parse().ok()
        };
        let (Some(level), Some(max)) = (read(&entry.path()), read(&dev.join("max_brightness")))
        else {
            continue;
        };
        if (1..=max).contains(&level) {
            let _ = std::fs::write(dev.join("brightness"), format!("{level}\n"));
        }
    }
}

/// In the child: keeps the card open until every output shows something else.
fn hold(kms: &Kms) {
    let ours: Vec<u32> = kms.outputs.iter().map(|o| kms.fb(o.crtc)).collect();
    let mut gone = vec![false; ours.len()];
    for _ in 0..HOLD_SECS * 100 {
        for ((o, &fb), gone) in kms.outputs.iter().zip(&ours).zip(gone.iter_mut()) {
            let now = kms.fb(o.crtc);
            if !*gone && now != fb {
                *gone = true;
                // When, and to what: DSlot's first frame should come at the same moment.
                kmsg(&format!(
                    "nibos-splash: {:.3} s CRTC {} taken over (fb {now})",
                    boottime(),
                    o.crtc
                ));
            }
        }
        if gone.iter().all(|&g| g) {
            // For nibos-boot-good: the boot UI (or the console) is up.
            let _ = std::fs::write("/run/nibos/screens-taken", format!("{:.3}\n", boottime()));
            return;
        }
        unsafe { libc::usleep(10_000) };
    }
    kmsg("nibos-splash: nothing took the screens over; giving them back to the console");
}
