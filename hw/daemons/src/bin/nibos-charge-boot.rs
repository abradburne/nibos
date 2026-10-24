//! nibos-charge-boot: charging asleep when the charger switched the device on
//! ([`nibos_hw::charge`] says why and what wakes it how).
//!
//!   nibos-charge-boot --check    dslot.service's ExecCondition: exits 1, so that the boot UI
//!                                isn't started, and leaves MARK, when the charger switched the
//!                                device on and is still plugged in; 0 otherwise, once the
//!                                service has left OVER, and on any error (the boot UI always
//!                                starts unless this is sure)
//!   nibos-charge-boot            nibos-charge-boot.service, once logind runs: when MARK is
//!                                there (or no boot UI is enabled and this boot is a charger
//!                                power-on), the screens off and asleep until the power key or
//!                                the lid (then the boot UI starts) or an unplug (power off)
//!
//! The PMIC's register is read with one I2C read transaction through /dev/i2c-N, so it can't
//! interleave with the kernel's own accesses to the PMIC.

use nibos_hw::{
    board::Board,
    charge::{next, Next, OnSource, Woken},
    power::{time_asleep, Wake},
};
use std::{
    fs,
    io::Read,
    os::fd::AsRawFd,
    path::Path,
    process::{exit, Command},
    thread,
    time::{Duration, Instant},
};

/// Left by `--check` for the service: this boot is charging asleep.
const MARK: &str = "/run/nibos/charge-boot";
/// Left by the service when the power key or the lid asked for the device: the rest of this
/// boot is a normal one, and `--check` lets the boot UI start (the PMIC still says "the
/// charger" until the next power-on).
const OVER: &str = "/run/nibos/charge-boot-over";
/// How long the boot UI takes to put its first frame up after it is started: the backlights
/// come on after that, so the console (tty1) is never seen lit.
const FIRST_FRAME: Duration = Duration::from_millis(1500);
/// How long the power key's and the lid's input devices may take to appear.
const DEVICE_WAIT: Duration = Duration::from_secs(30);
/// What the waking press and the charger's state settle in after a wake.
const SETTLE: Duration = Duration::from_millis(1500);
/// A suspend that hasn't happened by then didn't (refused, or over at once): try again.
const SUSPEND_WAIT: Duration = Duration::from_secs(60);

/// One byte from a register of an I2C device: a write of the register's number and a read,
/// as one transaction (I2C_RDWR).
fn i2c_read(bus: u32, addr: u16, reg: u8) -> std::io::Result<u8> {
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
    const I2C_RDWR: libc::c_ulong = 0x0707;
    const I2C_M_RD: u16 = 0x0001;

    let dev = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("/dev/i2c-{bus}"))?;
    let mut reg = reg;
    let mut value = 0u8;
    let mut msgs = [
        Msg {
            addr,
            flags: 0,
            len: 1,
            buf: &mut reg,
        },
        Msg {
            addr,
            flags: I2C_M_RD,
            len: 1,
            buf: &mut value,
        },
    ];
    let mut data = RdWr {
        msgs: msgs.as_mut_ptr(),
        nmsgs: 2,
    };
    // SAFETY: two valid messages with one-byte buffers that outlive the call.
    if unsafe { libc::ioctl(dev.as_raw_fd(), I2C_RDWR, &mut data) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(value)
}

fn charger_online(board: &Board) -> bool {
    board.charger.as_ref().is_some_and(|c| {
        fs::read_to_string(format!("/sys/class/power_supply/{c}/online"))
            .is_ok_and(|v| v.trim() == "1")
    })
}

/// Whether the charger switched the device on and is still plugged in.
fn charger_boot(board: &Board, source: OnSource) -> bool {
    match i2c_read(source.bus, source.addr, source.reg) {
        Ok(value) => {
            let by_charger = source.by_charger(value);
            eprintln!(
                "nibos-charge-boot: power-on source 0x{value:02x}{}",
                if by_charger { ": the charger" } else { "" }
            );
            by_charger && charger_online(board)
        }
        Err(e) => {
            eprintln!(
                "nibos-charge-boot: reading the power-on source (i2c-{} 0x{:02x}): {e}",
                source.bus, source.addr
            );
            false
        }
    }
}

fn boot_ui_enabled() -> bool {
    Command::new("systemctl")
        .args(["-q", "is-enabled", "dslot.service"])
        .status()
        .is_ok_and(|s| s.success())
}

/// The screens on or off: every backlight (`bl_power`: 0 on, 4 off; the level is left as it
/// is) and the console's framebuffer (`blank`), which otherwise lights the panels, and the
/// console with them, again at every wake.
fn backlights(on: bool) {
    let _ = fs::write(
        "/sys/class/graphics/fb0/blank",
        if on { "0\n" } else { "4\n" },
    );
    for e in fs::read_dir("/sys/class/backlight")
        .into_iter()
        .flatten()
        .flatten()
    {
        let _ = fs::write(e.path().join("bl_power"), if on { "0\n" } else { "4\n" });
    }
}

/// The input device with this name, opened to read its raw events without blocking (not
/// grabbed: others see them too). Raw `input_event`s, not evdev's synced device: that one
/// didn't pass on the press that woke the device, whose down and up arrive together.
/// Waits up to [`DEVICE_WAIT`] for it: the PMIC's power key comes up ~4 s into the boot, after
/// this has started.
fn open(name: &str) -> Option<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let start = Instant::now();
    let path = loop {
        if let Some((path, _)) = evdev::enumerate().find(|(_, d)| d.name() == Some(name)) {
            break path;
        }
        if start.elapsed() > DEVICE_WAIT {
            eprintln!("nibos-charge-boot: no input device named {name}");
            return None;
        }
        thread::sleep(Duration::from_millis(250));
    };
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()
}

/// The kernel's `struct input_event` on a 64-bit system.
#[repr(C)]
struct InputEvent {
    sec: i64,
    usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

const EV_KEY: u16 = 0x01;
const EV_SW: u16 = 0x05;
const KEY_POWER: u16 = 116;
const SW_LID: u16 = 0x00;

/// What the power key and the lid did since last asked.
fn read_events(power: &mut Option<fs::File>, lid: &mut Option<fs::File>) -> Woken {
    let mut woken = Woken::default();
    let size = std::mem::size_of::<InputEvent>();
    let mut buf = vec![0u8; size * 64];
    for dev in [power.as_mut(), lid.as_mut()].into_iter().flatten() {
        // Until there is nothing more to read (WouldBlock).
        while let Ok(n) = dev.read(&mut buf) {
            if n == 0 {
                break;
            }
            for chunk in buf[..n].chunks_exact(size) {
                // SAFETY: a whole input_event as the kernel wrote it.
                let ev: InputEvent = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast()) };
                match (ev.type_, ev.code, ev.value) {
                    (EV_KEY, KEY_POWER, 1) => woken.key = true,
                    (EV_SW, SW_LID, 0) => woken.lid_opened = true,
                    _ => {}
                }
            }
        }
    }
    woken
}

/// Suspend, and come back once the system has slept and woken; false if it didn't sleep.
fn sleep_once() -> bool {
    let mut wake = Wake::default();
    wake.observe(time_asleep(), Instant::now());
    if !Command::new("systemctl")
        .arg("suspend")
        .status()
        .is_ok_and(|s| s.success())
    {
        return false;
    }
    let start = Instant::now();
    while start.elapsed() < SUSPEND_WAIT {
        thread::sleep(Duration::from_millis(200));
        if wake.observe(time_asleep(), Instant::now()) {
            return true;
        }
    }
    false
}

fn main() {
    let board = match Board::load() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("nibos-charge-boot: {e}");
            exit(0);
        }
    };
    let Some(source) = board.power_on_source else {
        exit(0);
    };

    if std::env::args().nth(1).as_deref() == Some("--check") {
        if !Path::new(OVER).exists() && charger_boot(&board, source) {
            // Dark at once: skipping the boot UI starts the console's login prompt on tty1
            // (its OnSuccess=), and the service that sleeps starts only once logind runs.
            backlights(false);
            let _ = fs::create_dir_all("/run/nibos");
            let _ = fs::write(MARK, "");
            eprintln!("nibos-charge-boot: switched on by the charger: charging asleep, not starting the boot UI");
            exit(1);
        }
        exit(0);
    }

    let boot_ui = boot_ui_enabled();
    if !Path::new(MARK).exists() && (boot_ui || !charger_boot(&board, source)) {
        return;
    }
    let mut power = board.power_device.as_deref().and_then(open);
    let mut lid = board.lid_device.as_deref().and_then(open);
    backlights(false);
    loop {
        read_events(&mut power, &mut lid);
        if !sleep_once() {
            eprintln!("nibos-charge-boot: the device didn't sleep; trying again");
            thread::sleep(Duration::from_secs(5));
        }
        thread::sleep(SETTLE);
        let woken = read_events(&mut power, &mut lid);
        let online = charger_online(&board);
        match next(woken, online) {
            Next::Boot => break,
            Next::PowerOff => {
                eprintln!("nibos-charge-boot: unplugged: powering off");
                let _ = Command::new("systemctl").arg("poweroff").status();
                return;
            }
            Next::Sleep => eprintln!("nibos-charge-boot: woke, still charging: sleeping again"),
        }
    }
    eprintln!("nibos-charge-boot: asked for: the normal boot carries on");
    let _ = fs::write(OVER, "");
    let _ = fs::remove_file(MARK);
    if boot_ui {
        // The console's login prompt may have been started on tty1 while the boot UI was
        // skipped; the screens are dark until the boot UI has drawn over it.
        let _ = Command::new("systemctl")
            .args(["stop", "getty@tty1.service"])
            .status();
        let _ = Command::new("systemctl")
            .args(["start", "--no-block", "dslot.service"])
            .status();
        thread::sleep(FIRST_FRAME);
    }
    backlights(true);
}
