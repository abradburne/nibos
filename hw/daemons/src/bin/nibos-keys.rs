//! nibos-keys: a keyboard on the bottom screen while the console (tty1) is in front, typing into
//! it through a uinput keyboard. It runs with getty@tty1 (nibos-keys.service): when DSlot is
//! left with Select + Start, from safe mode's "Open a console", and on a system without a boot
//! UI. The console stays the kernel's, on the top screen; the bottom one is the keyboard's,
//! black while it is hidden. Menu + X shows and hides it, and a touch on the black screen shows
//! it too.
//!
//! It takes the bottom panel with one SETCRTC of a buffer of its own and draws into that buffer
//! in place: no page flips, and the top panel (the console's framebuffer) is never touched. If
//! the card has a master already (DSlot, safe mode), the SETCRTC fails and it leaves, quietly.
//! It exits on SIGTERM (DSlot starting stops it), and closing the card gives the bottom panel
//! back to the console until the next program takes the screens.

// The splash's KMS, of which this uses a buffer of its own on one output.
#[path = "../../../../rootfs/splash/src/kms.rs"]
#[allow(dead_code)]
mod kms;

use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, Key};
use nibkit::menu::keys::{code, Keys, Out};
use nibkit::menu::raster::{Canvas, Fonts};
use nibkit::menu::Draw;
use nibkit::touch::{Panel, Pointer};
use nibos_hw::board::Board;
use nibos_hw::keys::{Gamepad, Pad, Repeat};
use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const NAME: &str = "nibOS on-screen keyboard";

static TERM: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    TERM.store(true, Ordering::Relaxed);
}

/// The merged gamepad's event device, by its name in board.conf, open without blocking.
fn pad(name: &str) -> Option<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::read_dir("/sys/class/input")
        .ok()?
        .flatten()
        .find_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let dev = std::fs::read_to_string(e.path().join("device/name")).ok()?;
            (n.starts_with("event") && dev.trim() == name).then(|| {
                std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(format!("/dev/input/{n}"))
                    .ok()
            })?
        })
}

/// The keyboard's uinput device: every key it can type.
fn keyboard() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    for k in code::all() {
        keys.insert(Key::new(k));
    }
    VirtualDeviceBuilder::new()?
        .name(NAME)
        .input_id(InputId::new(BusType::BUS_VIRTUAL, 0x1209, 0x6e6b, 0x0100))
        .with_keys(&keys)?
        .build()
}

/// A key's box: x, y, width, height.
type KeyBox = (f32, f32, f32, f32);

/// The keyboard's draw list in groups: the first is what's behind the keys (the background and
/// the line of hints), then one per key, from its body (a `Round`, which gives its box) to the
/// next key's.
fn groups(draws: &[Draw]) -> Vec<(Option<KeyBox>, &[Draw])> {
    let mut out = Vec::new();
    let (mut start, mut at) = (0, None);
    for (i, d) in draws.iter().enumerate() {
        if let Draw::Round { x, y, w, h, .. } = *d {
            out.push((at, &draws[start..i]));
            (start, at) = (i, Some((x, y, w, h)));
        }
    }
    out.push((at, &draws[start..]));
    out
}

/// The keyboard on the bottom screen's buffer, drawn again only where it changed. Drawing all
/// of it (fontdue's text and 16-sample rounded keys over 1024 × 768) took ~30 ms on the build
/// machine and several times that on the A55, twice a key (pressed, let go), and the next
/// touch waited behind it: the keyboard felt slow. A key's picture changes alone (its body,
/// label, frame): only that key's box is drawn again and copied.
struct Painter {
    canvas: Canvas,
    draws: Vec<Draw>,
    trace: bool,
}

impl Painter {
    fn new() -> Painter {
        Painter {
            canvas: Canvas::new(1.0),
            draws: Vec::new(),
            trace: std::env::var_os("NIBOS_KEYS_TRACE").is_some(),
        }
    }

    /// Shows `draws` (none: black), drawing and copying only the keys that changed.
    fn show(&mut self, fonts: &Fonts, draws: Vec<Draw>, buffer: &mut kms::Buffer) {
        if draws == self.draws {
            return;
        }
        let t = Instant::now();
        let (old, new) = (groups(&self.draws), groups(&draws));
        let same_layout = old.len() == new.len()
            && old[0].1 == new[0].1
            && old.iter().zip(&new).all(|(a, b)| a.0 == b.0);
        let mut boxes = Vec::new();
        if same_layout && !self.draws.is_empty() {
            for ((_, a), (at, b)) in old.iter().zip(&new).skip(1) {
                let Some((x, y, w, h)) = *at else { continue };
                if a == b {
                    continue;
                }
                // A pixel more each way for the rounded corners' edge, well inside the gap
                // between keys.
                let (x, y, w, h) = (x - 1.0, y - 1.0, w + 2.0, h + 2.0);
                let behind = Draw::Rect {
                    x,
                    y,
                    w,
                    h,
                    colour: nibkit::menu::colour::BLACK,
                };
                self.canvas
                    .draw(fonts, std::slice::from_ref(&behind), |_| None);
                self.canvas.draw(fonts, b, |_| None);
                let (x0, y0) = (x.floor().max(0.0) as usize, y.floor().max(0.0) as usize);
                boxes.push((
                    x0,
                    y0,
                    (x + w).ceil() as usize - x0,
                    (y + h).ceil() as usize - y0,
                ));
            }
        } else {
            self.canvas = Canvas::new(1.0);
            self.canvas.draw(fonts, &draws, |_| None);
            boxes.push((0, 0, self.canvas.w, self.canvas.h));
        }
        for &b in &boxes {
            buffer.put_rgba(&self.canvas.px, self.canvas.w, b);
        }
        if self.trace {
            eprintln!(
                "nibos-keys: drew {} box(es) in {:.1} ms",
                boxes.len(),
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
        self.draws = draws;
    }
}

fn main() -> ExitCode {
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as *const () as libc::sighandler_t);
    }
    let kms = match kms::Kms::open() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("nibos-keys: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Top screen first: the bottom one is the second.
    let Some(bottom) = kms.outputs.get(1) else {
        eprintln!("nibos-keys: no second screen");
        return ExitCode::FAILURE;
    };
    let mut buffer = match kms.buffer(bottom) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("nibos-keys: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = kms.set(bottom, &buffer) {
        // Someone holds the screens (DSlot, safe mode): they're theirs.
        eprintln!("nibos-keys: the screens are in use ({e}); not showing");
        return ExitCode::SUCCESS;
    }
    let mut device = match keyboard() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("nibos-keys: uinput: {e}");
            return ExitCode::FAILURE;
        }
    };
    let fonts = Fonts::new();
    let mut panel = Panel::open();
    let mut gamepad = Board::load().ok().and_then(|b| pad(&b.pad_name));
    if panel.is_none() {
        eprintln!("nibos-keys: no touch panel; the buttons only");
    }
    if gamepad.is_none() {
        eprintln!("nibos-keys: no gamepad; touch only");
    }

    let mut keys = Keys::new();
    let mut shown = true;
    let mut painter = Painter::new();
    let mut pad_state = Gamepad::new();
    let mut repeat = Repeat::default();
    let mut pointers = Vec::new();
    let emit = |device: &mut VirtualDevice, outs: Vec<Out>, repeat: &mut Repeat| -> bool {
        let mut hide = false;
        let now = Instant::now();
        let events: Vec<InputEvent> = outs
            .into_iter()
            .filter_map(|o| match o {
                Out::Press(k) => {
                    repeat.press(k, now);
                    Some(InputEvent::new(EventType::KEY, k, 1))
                }
                Out::Release(k) => {
                    repeat.release(k);
                    Some(InputEvent::new(EventType::KEY, k, 0))
                }
                Out::Hide => {
                    hide = true;
                    None
                }
            })
            .collect();
        // One event and its SYN at a time: the console sees modifiers before the key.
        for e in events {
            if let Err(err) = device.emit(&[e]) {
                eprintln!("nibos-keys: {err}");
            }
        }
        hide
    };

    while !TERM.load(Ordering::Relaxed) {
        // After the keys of the last round went out: drawing never holds a key back.
        painter.show(
            &fonts,
            if shown { keys.draws() } else { Vec::new() },
            &mut buffer,
        );
        let now = Instant::now();
        let wait = if shown { repeat.wait(now) } else { None };
        let timeout = wait.map_or(-1, |d| d.as_millis().min(1000) as libc::c_int);
        let mut fds: Vec<libc::pollfd> = [
            panel.as_ref().map(|p| p.as_raw_fd()),
            gamepad.as_ref().map(|g| g.as_raw_fd()),
        ]
        .into_iter()
        .flatten()
        .map(|fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };

        if let Some(p) = panel.as_mut() {
            pointers.clear();
            p.poll(&mut pointers);
            for pointer in pointers.drain(..).map(|p| p.scaled(1024.0, 768.0)) {
                let outs = match (shown, pointer) {
                    (false, Pointer::Down(..)) => {
                        shown = true;
                        Vec::new()
                    }
                    (true, Pointer::Down(x, y)) => keys.touch_down(x, y),
                    (true, Pointer::Up) => keys.touch_up(),
                    _ => Vec::new(),
                };
                if emit(&mut device, outs, &mut repeat) {
                    shown = false;
                    let held = keys.release_all();
                    emit(&mut device, held, &mut repeat);
                }
            }
        }
        if let Some(g) = gamepad.as_mut() {
            let mut ev = [0u8; 24];
            while g.read_exact(&mut ev).is_ok() {
                let kind = u16::from_ne_bytes([ev[16], ev[17]]);
                let code = u16::from_ne_bytes([ev[18], ev[19]]);
                let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
                for p in pad_state.event(kind, code, value) {
                    let outs = match p {
                        Pad::Toggle => {
                            shown = !shown;
                            if shown {
                                Vec::new()
                            } else {
                                keys.release_all()
                            }
                        }
                        Pad::Button(b, down) if shown => keys.button(b, down),
                        // Hidden: a button let go still comes up.
                        Pad::Button(b, false) => keys.button(b, false),
                        Pad::Button(..) => Vec::new(),
                    };
                    emit(&mut device, outs, &mut repeat);
                }
            }
        }
        if shown {
            if let Some(k) = repeat.due(Instant::now()) {
                let _ = device.emit(&[InputEvent::new(EventType::KEY, k, 2)]);
            }
        }
    }
    let held = keys.release_all();
    emit(&mut device, held, &mut repeat);
    ExitCode::SUCCESS
}
