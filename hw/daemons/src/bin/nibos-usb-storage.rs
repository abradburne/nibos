//! nibos-usb-storage: the data cards as drives on a computer, while Menu is held as the USB
//! cable goes in. The logic is [`nibos_hw::usb_storage`]; hw/README.md, "USB storage", says what it does.
//!
//!   nibos-usb-storage --event NAME   udev, on every event of a power supply: when it is the
//!                                    board's charger and it has just come online (a plug),
//!                                    starts nibos-usb-storage.service; otherwise only notes
//!                                    the charger's state
//!   nibos-usb-storage                nibos-usb-storage.service: when Menu is held and a
//!                                    computer enumerates the device, the cards go to it until
//!                                    the cable comes out or the computer ejects them
//!
//! With Menu not held, or a charger rather than a computer at the other end, it leaves at once
//! and nothing else has happened: the boot UI, the game and the cards are as they were.

// The splash's KMS, for the message on both screens.
#[path = "../../../../rootfs/splash/src/kms.rs"]
#[allow(dead_code)]
mod kms;

use evdev::Key;
use nibkit::menu::colour::{DIM, GOLD, WHITE};
use nibkit::menu::raster::{Canvas, Fonts};
use nibkit::menu::{Draw, H, W};
use nibos_hw::board::Board;
use nibos_hw::usb_storage::{self, Card, Gadget, Over, Seen, GADGET, LAST_ONLINE};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// How long after the plug Menu may still be pressed (udev and the service start take some).
const MENU_WINDOW: Duration = Duration::from_millis(1500);
/// How long a computer takes to enumerate the device: a charger never does.
const ENUMERATE_WAIT: Duration = Duration::from_secs(6);
/// How often the cable and the drives are looked at while the cards are out.
const POLL: Duration = Duration::from_millis(250);
/// The boot UI and the console's units.
const BOOT_UI: &str = "dslot.service";
const CONSOLE: &str = "getty@tty1.service";
/// Sleep is masked while the cards are out: the lid or the power key mustn't suspend the
/// device under the computer's writes. Runtime masks: a reboot clears them whatever happens.
const SLEEP_UNITS: [&str; 5] = [
    "sleep.target",
    "suspend.target",
    "hibernate.target",
    "hybrid-sleep.target",
    "suspend-then-hibernate.target",
];

static TERM: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    TERM.store(true, Ordering::Relaxed);
}

fn read(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn write(p: impl AsRef<Path>, v: &str) -> Result<(), String> {
    fs::write(p.as_ref(), v).map_err(|e| format!("{}: {e}", p.as_ref().display()))
}

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

fn is_active(unit: &str) -> bool {
    systemctl(&["--quiet", "is-active", unit])
}

/// Whether systemd has the unit (a mount point's automount may not exist).
fn loaded(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["show", "--property=LoadState", "--value", unit])
        .output()
        .is_ok_and(|o| o.stdout.starts_with(b"loaded"))
}

/// The units that require or are bound to `unit`.
fn needing(unit: &str) -> Vec<String> {
    Command::new("systemctl")
        .args(["show", "--property=RequiredBy,BoundBy", "--value", unit])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn charger_online(charger: &str) -> bool {
    read(format!("/sys/class/power_supply/{charger}/online")).as_deref() == Some("1")
}

/// `--event NAME`: notes the charger's state, and starts the service on a plug.
fn event(name: &str) -> ExitCode {
    let Ok(board) = Board::load() else {
        return ExitCode::SUCCESS;
    };
    if board.charger.as_deref() != Some(name) {
        return ExitCode::SUCCESS;
    }
    let online = charger_online(name);
    let last = read(LAST_ONLINE).map(|s| s == "1");
    let _ = fs::create_dir_all("/run/nibos");
    let _ = fs::write(LAST_ONLINE, if online { "1\n" } else { "0\n" });
    if usb_storage::plugged(last, online) {
        // udev waits for RUN programs: don't wait for the service.
        systemctl(&["--no-block", "start", "nibos-usb-storage.service"]);
    }
    ExitCode::SUCCESS
}

/// Whether Menu is held now (the key's state, which a grab by nibos-input doesn't hide).
fn menu_held(device: &str) -> bool {
    evdev::enumerate().any(|(_, d)| {
        d.name() == Some(device)
            && d.get_key_state()
                .is_ok_and(|keys| keys.contains(Key::KEY_HOME))
    })
}

/// The cards in /etc/fstab whose device is there, an `auto` one with its own file system
/// (shared only if it is exFAT or FAT: a computer can't use anything else).
fn present_cards() -> Vec<(Card, PathBuf)> {
    let fstab = read("/etc/fstab").unwrap_or_default();
    usb_storage::cards(&fstab)
        .into_iter()
        .filter_map(|mut c| {
            let dev = fs::canonicalize(c.device()).ok()?;
            if c.fs == "auto" {
                let out = Command::new("blkid")
                    .args(["-o", "value", "-s", "TYPE"])
                    .arg(&dev)
                    .output()
                    .ok()?;
                c.fs = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
            matches!(c.fs.as_str(), "exfat" | "vfat").then_some((c, dev))
        })
        .collect()
}

/// A serial number that stays the same: the SoC's, else the machine id's start.
fn serial() -> String {
    read("/proc/device-tree/serial-number")
        .map(|s| s.trim_end_matches('\0').to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| read("/etc/machine-id").map(|s| s.chars().take(16).collect()))
        .unwrap_or_else(|| "0".into())
}

fn udc() -> Option<String> {
    fs::read_dir("/sys/class/udc")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .next()
}

/// The gadget, made and bound to the USB controller, with no card in its LUNs yet.
fn gadget_up(g: &Gadget, udc: &str) -> Result<(), String> {
    let _ = Command::new("modprobe").arg("libcomposite").status();
    let root = Path::new(GADGET);
    fs::create_dir(root).map_err(|e| format!("{GADGET}: {e}"))?;
    for d in g.dirs() {
        fs::create_dir(root.join(&d)).map_err(|e| format!("{d}: {e}"))?;
    }
    for (f, v) in g.files() {
        write(root.join(f), &v)?;
    }
    std::os::unix::fs::symlink(
        root.join("functions/mass_storage.0"),
        root.join("configs/c.1/mass_storage.0"),
    )
    .map_err(|e| format!("configs/c.1: {e}"))?;
    if write(root.join("UDC"), udc).is_ok() {
        return Ok(());
    }
    // A DWC3 bound once with no cable in (ep0 fails to enable) refuses every bind after until
    // its driver probes it again: do that, and try once more.
    eprintln!("nibos-usb-storage: binding {udc} failed; probing the controller again");
    let _ = fs::write("/sys/bus/platform/drivers/dwc3/unbind", udc);
    sleep(Duration::from_millis(500));
    let _ = fs::write("/sys/bus/platform/drivers/dwc3/bind", udc);
    sleep(Duration::from_millis(500));
    write(root.join("UDC"), udc)
}

/// The gadget unbound and taken apart, as far as it was made. Emptying the LUNs first lets the
/// card's last writes go out before the function is gone.
fn gadget_down(g: &Gadget) {
    let root = Path::new(GADGET);
    if !root.exists() {
        return;
    }
    for i in 0..g.luns.len() {
        let _ = fs::write(
            root.join(format!("functions/mass_storage.0/lun.{i}/file")),
            "",
        );
    }
    let _ = fs::write(root.join("UDC"), "");
    let _ = fs::remove_file(root.join("configs/c.1/mass_storage.0"));
    for d in g.teardown_dirs() {
        let _ = fs::remove_dir(root.join(d));
    }
    if let Err(e) = fs::remove_dir(root) {
        eprintln!("nibos-usb-storage: removing the gadget: {e}");
    }
}

fn udc_state(udc: &str) -> String {
    read(format!("/sys/class/udc/{udc}/state")).unwrap_or_default()
}

/// Whether a block device is mounted anywhere (by its device number in mountinfo).
fn mounted_at(dev: &Path) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let Ok(rdev) = fs::metadata(dev).map(|m| m.rdev()) else {
        return Vec::new();
    };
    let want = format!(
        "{}:{}",
        libc::major(rdev as libc::dev_t),
        libc::minor(rdev as libc::dev_t)
    );
    read("/proc/self/mountinfo")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (f.get(2) == Some(&want.as_str())).then(|| f[4].to_string())
        })
        .collect()
}

/// Whether an autofs is mounted at `path`.
fn autofs_at(path: &str) -> bool {
    read("/proc/self/mountinfo")
        .unwrap_or_default()
        .lines()
        .any(|l| {
            let (pre, post) = l.split_once(" - ").unwrap_or((l, ""));
            pre.split(' ').nth(4) == Some(path) && post.starts_with("autofs ")
        })
}

/// Both screens: a title on the top one, lines on the bottom one. Holds the card (DRM master)
/// until dropped.
struct Screens {
    kms: kms::Kms,
    buffers: Vec<kms::Buffer>,
    fonts: Fonts,
}

impl Screens {
    fn open() -> Option<Screens> {
        let kms = kms::Kms::open()
            .map_err(|e| eprintln!("nibos-usb-storage: screens: {e}"))
            .ok()?;
        let mut buffers = Vec::new();
        for o in &kms.outputs {
            let b = kms.buffer(o).ok()?;
            if let Err(e) = kms.set(o, &b) {
                eprintln!("nibos-usb-storage: screens in use: {e}");
                return None;
            }
            buffers.push(b);
        }
        Some(Screens {
            kms,
            buffers,
            fonts: Fonts::new(),
        })
    }

    fn show(&mut self, title: &str, lines: &[String]) {
        let text = |text: &str, size: f32, bold: bool, y: f32, colour| Draw::Text {
            text: text.into(),
            size,
            bold,
            x: W / 2.0,
            y,
            anchor: 0.5,
            colour,
        };
        let top = vec![
            text("USB", 40.0, true, H / 2.0 - 110.0, GOLD),
            text(title, 64.0, true, H / 2.0 - 40.0, WHITE),
        ];
        let bottom: Vec<Draw> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                if i == 0 {
                    text(l, 52.0, true, H / 2.0 - 120.0, WHITE)
                } else {
                    text(l, 32.0, false, H / 2.0 - 20.0 + 60.0 * (i - 1) as f32, DIM)
                }
            })
            .collect();
        // The panels are W × H, as nibkit draws (nibos-keys does the same).
        for (b, draws) in self.buffers.iter_mut().zip([top, bottom]) {
            let mut c = Canvas::new(1.0);
            c.draw(&self.fonts, &draws, |_| None);
            b.put_rgba(&c.px, c.w, (0, 0, c.w, c.h));
        }
    }
}

impl Drop for Screens {
    fn drop(&mut self) {
        self.buffers.clear();
        unsafe { libc::close(self.kms.fd) };
    }
}

/// What was running before, to put back afterwards.
#[derive(Default)]
struct Before {
    boot_ui: bool,
    console: bool,
    /// Per card: its mount unit and automount unit, and whether each was active.
    mounts: Vec<(String, bool, String, bool)>,
    /// The active units that need a card mounted (RequiresMountsFor=: the settings and logs
    /// saved at power-off). Stopping the mount stops them, and they are started again after.
    users: Vec<String>,
    masked: bool,
}

impl Before {
    /// The screens taken from the boot UI or the console, and sleep masked.
    fn take_screens(&mut self) {
        self.boot_ui = is_active(BOOT_UI);
        self.console = is_active(CONSOLE);
        let mut mask = vec!["mask", "--runtime", "--quiet"];
        mask.extend(SLEEP_UNITS);
        self.masked = systemctl(&mask);
        if self.boot_ui {
            // SIGTERM: a running game is saved and put away first (up to 10 s).
            systemctl(&["stop", BOOT_UI]);
        }
        // Leaving the boot UI starts the console (its OnSuccess=); its keyboard holds the screen.
        systemctl(&["stop", CONSOLE]);
    }

    /// The cards unmounted, by their units so nothing mounts them again behind our back.
    fn unmount(&mut self, cards: &[(Card, PathBuf)]) -> Result<(), String> {
        for (card, dev) in cards {
            let (mount, automount) = card.units();
            let (m, a) = (is_active(&mount), is_active(&automount));
            self.mounts.push((mount.clone(), m, automount.clone(), a));
            for u in needing(&mount) {
                if is_active(&u) && !self.users.contains(&u) {
                    self.users.push(u);
                }
            }
            // Stopped one at a time (a unit that isn't there, such as the games card's
            // automount, fails the whole job otherwise), the mount before its automount.
            for unit in [&mount, &automount] {
                if loaded(unit) {
                    systemctl(&["stop", unit]);
                }
            }
            // Then masked (until reboot at most): udev mounts the games card again on the
            // "change" event closing it sends (70-nibos-games-card.rules), which would mount
            // it before it is checked. Masking an active automount instead makes systemd
            // abandon its autofs, and starting it again then fails.
            systemctl(&["mask", "--runtime", "--quiet", &automount, &mount]);
            if autofs_at(&card.mount_point) {
                let _ = Command::new("umount").arg(&card.mount_point).status();
            }
            for at in mounted_at(dev) {
                let _ = Command::new("umount").arg(&at).status();
            }
            if let Some(at) = mounted_at(dev).first() {
                return Err(format!("{} is still mounted at {at} (busy)", dev.display()));
            }
        }
        Ok(())
    }

    /// Everything as it was: the cards mounted, sleep, the boot UI or the console.
    fn restore(&self) {
        for (mount, m, automount, a) in &self.mounts {
            systemctl(&["unmask", "--runtime", "--quiet", automount, mount]);
            if *a {
                systemctl(&["start", automount]);
            }
            if *m {
                systemctl(&["start", mount]);
            }
        }
        for unit in &self.users {
            systemctl(&["start", unit]);
        }
        if self.masked {
            let mut unmask = vec!["unmask", "--runtime", "--quiet"];
            unmask.extend(SLEEP_UNITS);
            systemctl(&unmask);
        }
        if self.boot_ui {
            systemctl(&["start", BOOT_UI]);
        } else if self.console {
            systemctl(&["start", CONSOLE]);
        }
    }
}

/// A card the computer may have left mid-write, checked and repaired: exFAT when its dirty
/// flag says so, FAT always (it is small, and its flag isn't as dependable).
fn check(card: &Card, dev: &Path) {
    let run = |prog: &str, args: &[&str]| {
        let t = Instant::now();
        match Command::new(prog).args(args).arg(dev).status() {
            // fsck: 0 clean, 1 errors corrected.
            Ok(s) => eprintln!(
                "nibos-usb-storage: {prog} {}: exit {:?} in {:.1} s",
                dev.display(),
                s.code(),
                t.elapsed().as_secs_f32()
            ),
            Err(e) => eprintln!("nibos-usb-storage: {prog}: {e}"),
        }
    };
    match card.fs.as_str() {
        "exfat" => {
            let mut boot = [0u8; 512];
            let dirty = fs::File::open(dev)
                .and_then(|mut f| f.read_exact(&mut boot))
                .ok()
                .and_then(|_| usb_storage::exfat_dirty(&boot));
            if dirty == Some(true) {
                run("fsck.exfat", &["-p"]);
            }
        }
        "vfat" => run("fsck.fat", &["-a", "-w"]),
        _ => {}
    }
}

fn run() -> Result<(), String> {
    let board = Board::load()?;
    let charger = board.charger.clone().ok_or("no NIBOS_CHARGER")?;

    // Menu, held as the cable went in.
    let t = Instant::now();
    while !menu_held(&board.buttons_device) {
        if t.elapsed() > MENU_WINDOW || !charger_online(&charger) {
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }
    let cards = present_cards();
    if cards.is_empty() {
        return Err("no data card to share".into());
    }
    let udc = udc().ok_or("no USB device controller")?;
    let gadget = Gadget {
        product: board.name.clone(),
        serial: serial(),
        luns: cards.iter().map(|(c, _)| c.inquiry()).collect(),
    };

    // A computer, not a charger: it enumerates the device (with no card in it yet).
    if Path::new(GADGET).exists() {
        gadget_down(&gadget);
    }
    if let Err(e) = gadget_up(&gadget, &udc) {
        gadget_down(&gadget);
        return Err(e);
    }
    let t = Instant::now();
    while udc_state(&udc) != "configured" {
        if t.elapsed() > ENUMERATE_WAIT || !charger_online(&charger) || TERM.load(Ordering::Relaxed)
        {
            eprintln!(
                "nibos-usb-storage: not a computer (USB state {:?}); nothing changed",
                udc_state(&udc)
            );
            gadget_down(&gadget);
            return Ok(());
        }
        sleep(Duration::from_millis(100));
    }
    eprintln!("nibos-usb-storage: a computer enumerated the device; sharing the cards");

    let mut before = Before::default();
    before.take_screens();
    let mut screens = Screens::open();
    let names: Vec<String> = cards.iter().map(|(c, _)| c.name()).collect();
    if let Some(s) = screens.as_mut() {
        s.show("Connecting…", &["Getting the cards ready".to_string()]);
    }
    let result = share(&mut before, &cards, &gadget, &charger, &mut screens, &names);
    gadget_down(&gadget);
    let _ = Command::new("sync").status();
    if result.is_ok() {
        if let Some(s) = screens.as_mut() {
            s.show("Checking the cards", &["One moment".to_string()]);
        }
        for (card, dev) in &cards {
            check(card, dev);
        }
    }
    drop(screens);
    before.restore();
    result.map(|over| eprintln!("nibos-usb-storage: cards back ({over:?})"))
}

/// The cards out to the computer until the cable comes out, the computer ejects them all, or
/// the system stops.
fn share(
    before: &mut Before,
    cards: &[(Card, PathBuf)],
    gadget: &Gadget,
    charger: &str,
    screens: &mut Option<Screens>,
    names: &[String],
) -> Result<Over, String> {
    before.unmount(cards)?;
    let root = Path::new(GADGET);
    for (i, (_, dev)) in cards.iter().enumerate() {
        write(
            root.join(format!("functions/mass_storage.0/lun.{i}/file")),
            &dev.to_string_lossy(),
        )?;
    }
    let (title, lines) = usb_storage::message(names);
    if let Some(s) = screens.as_mut() {
        s.show(title, &lines);
    }
    loop {
        let seen = Seen {
            online: charger_online(charger),
            luns_with_card: (0..gadget.luns.len())
                .filter(|i| {
                    read(root.join(format!("functions/mass_storage.0/lun.{i}/file")))
                        .is_some_and(|f| !f.is_empty())
                })
                .count(),
            term: TERM.load(Ordering::Relaxed),
        };
        if let Some(over) = usb_storage::watch(seen) {
            return Ok(over);
        }
        sleep(POLL);
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, name] = args.as_slice() {
        if flag == "--event" {
            return event(name);
        }
    }
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as *const () as libc::sighandler_t);
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nibos-usb-storage: {e}");
            ExitCode::FAILURE
        }
    }
}
