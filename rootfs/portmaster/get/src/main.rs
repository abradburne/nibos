//! portmaster-get: gets PortMaster for nibOS. nibOS doesn't ship PortMaster (its release bundles
//! files nibOS may not redistribute), so the first time the user opens it this asks, downloads
//! the release nibOS pins from PortMaster's GitHub, checks its SHA-256 and unpacks it into
//! ~/.local/share/PortMaster, as PortMaster's own installer does. With `--update` it offers the
//! pinned release over an older copy.
//!
//! The bottom (touch) screen is DSlot's menu, drawn by nibkit; the top screen says what
//! PortMaster is, where it comes from and how the download is going.
//!
//! Exit status: 0 PortMaster is in place (installed or updated), 3 keep the copy there is
//! (`--update`, "Not now"), 1 not installed (the user said no, or it failed).

use nibkit::{
    input::{sdl_name, Face},
    menu::{
        colour,
        layout::{self, Draw},
        raster::{Canvas, Fonts},
        Effect, Key, Menu, Row, Slot, Status, Value, H, W,
    },
    net::{self, Wifi},
    screens::{place, Rect as Bounds},
};
use sdl2::{
    controller::{Button, GameController},
    event::Event,
    pixels::PixelFormatEnum,
    render::{Canvas as SdlCanvas, Texture, TextureCreator},
    video::{Window, WindowContext},
};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

/// The release the package pins: written by the PKGBUILD from its source line and checksum.
const RELEASE_CONF: &str = "/usr/lib/portmaster/release.conf";

/// The pinned release.
#[derive(Clone, Debug, PartialEq)]
struct Release {
    version: String,
    url: String,
    sha256: String,
    size: u64,
}

impl Release {
    /// `NAME=value` lines (a shell file); `#` comments.
    fn parse(text: &str) -> Option<Release> {
        let get = |key: &str| {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().trim_matches('"').to_owned())
        };
        Some(Release {
            version: get("PORTMASTER_VERSION")?,
            url: get("PORTMASTER_URL")?,
            sha256: get("PORTMASTER_SHA256")?.to_ascii_lowercase(),
            size: get("PORTMASTER_SIZE")?.parse().ok()?,
        })
    }
}

/// Where PortMaster goes (every port's script looks there), and where the download waits.
fn folders() -> (PathBuf, PathBuf) {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()));
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cache"));
    (data, cache.join("portmaster"))
}

/// "23.2 MB".
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// What the getter is doing, as the worker thread reports it.
#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Downloading,
    Checking,
    Unpacking,
    Done,
    Failed(String),
}

/// The download, check and unpack, on a thread of their own.
struct Job {
    stage: Arc<Mutex<Stage>>,
    bytes: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn start(release: Release) -> Job {
        let job = Job {
            stage: Arc::new(Mutex::new(Stage::Downloading)),
            bytes: Arc::new(AtomicU64::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let (stage, bytes, cancel) = (job.stage.clone(), job.bytes.clone(), job.cancel.clone());
        eprintln!(
            "portmaster-get: getting {} from {}",
            release.version, release.url
        );
        thread::spawn(move || {
            let (data, cache) = folders();
            let result = get(&release, (&data, &cache), &stage, &bytes, &cancel);
            *stage.lock().unwrap() = match result {
                Ok(()) => {
                    eprintln!("portmaster-get: PortMaster {} installed", release.version);
                    Stage::Done
                }
                Err(e) => {
                    eprintln!("portmaster-get: {e}");
                    Stage::Failed(e)
                }
            };
        });
        job
    }

    fn stage(&self) -> Stage {
        self.stage.lock().unwrap().clone()
    }
}

/// Waits for `child`, reporting the size of `watch` as it grows, and kills it on cancel.
fn follow(
    mut child: Child,
    watch: &Path,
    bytes: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), (Option<i32>, String)> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err((None, CANCELLED.into()));
        }
        if let Ok(m) = fs::metadata(watch) {
            bytes.store(m.len(), Ordering::Relaxed);
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                let err = err.trim().lines().last().unwrap_or("it failed").to_owned();
                return Err((status.code(), err));
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err((None, e.to_string())),
        }
    }
}

/// What a failed job says when the user cancelled it.
const CANCELLED: &str = "cancelled";

/// curl's failure in words (its exit status, then its last line of complaint).
fn curl_failed(code: Option<i32>, err: &str) -> String {
    match code {
        _ if err == CANCELLED => CANCELLED.into(),
        Some(6) => "No internet: github.com couldn't be found. Is Wi-Fi connected?".into(),
        Some(7) => "Couldn't reach GitHub. Is Wi-Fi connected to the internet?".into(),
        Some(22) => format!("GitHub answered with an error ({err})"),
        Some(28) => "The download stalled: no data for 30 s".into(),
        Some(35) | Some(60) => {
            "No secure connection to GitHub could be made. Is the clock right?".into()
        }
        Some(23) => "The download couldn't be saved. Is the card full?".into(),
        _ => format!("The download failed: {err}"),
    }
}

/// Bytes free for this user on the file system holding `path`.
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs writes the struct we own; the path is a NUL-terminated string we own.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0)
        .then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

/// Downloads the release into `cache`, checks it and unpacks it into `data`/PortMaster.
fn get(
    release: &Release,
    (data, cache): (&Path, &Path),
    stage: &Mutex<Stage>,
    bytes: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    // The zip, and it unpacked (about 1.7 times its size), with room to spare.
    let need = release.size * 3;
    if let Some(free) = free_bytes(data) {
        if free < need {
            return Err(format!(
                "Not enough room: {} free, {} needed",
                megabytes(free),
                megabytes(need)
            ));
        }
    }
    let zip = cache.join(format!("PortMaster-{}.zip", release.version));
    let part = zip.with_extension("zip.part");
    let _ = fs::remove_file(&part);

    let curl = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--retry",
            "2",
        ])
        .args([
            "--connect-timeout",
            "10",
            "--speed-time",
            "30",
            "--speed-limit",
            "1000",
        ])
        .arg("--output")
        .arg(&part)
        .arg(&release.url)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let downloaded = follow(curl, &part, bytes, cancel);
    if let Err((code, err)) = downloaded {
        let _ = fs::remove_file(&part);
        return Err(curl_failed(code, &err));
    }

    *stage.lock().unwrap() = Stage::Checking;
    let out = Command::new("sha256sum")
        .arg(&part)
        .output()
        .map_err(|e| format!("sha256sum: {e}"))?;
    let sum = String::from_utf8_lossy(&out.stdout);
    let sum = sum.split_whitespace().next().unwrap_or("");
    if sum != release.sha256 {
        let _ = fs::remove_file(&part);
        return Err(format!(
            "The download isn't the release nibOS expects (SHA-256 {sum}): not installed"
        ));
    }
    fs::rename(&part, &zip).map_err(|e| e.to_string())?;

    // Unpacked beside the old copy, then moved over it, so a failure leaves the old one whole.
    *stage.lock().unwrap() = Stage::Unpacking;
    let staging = data.join(".PortMaster-unpacking");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    let bsdtar = Command::new("bsdtar")
        .arg("-xf")
        .arg(&zip)
        .arg("-C")
        .arg(&staging)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("bsdtar: {e}"))?;
    let unpacking = follow(bsdtar, &zip, &AtomicU64::new(0), cancel);
    if let Err((_, err)) = unpacking {
        let _ = fs::remove_dir_all(&staging);
        return Err(if err == CANCELLED {
            err
        } else {
            format!("Unpacking failed: {err}")
        });
    }
    let unpacked = staging.join("PortMaster");
    if !unpacked.join("version").is_file() {
        return Err("The download has no PortMaster in it".into());
    }
    // As PortMaster's installer does: the release over what is there (its settings, runtimes
    // and downloads stay).
    let target = data.join("PortMaster");
    let status = if target.exists() {
        Command::new("cp")
            .arg("-a")
            .arg(format!("{}/.", unpacked.display()))
            .arg(&target)
            .status()
            .map_err(|e| e.to_string())
            .and_then(|s| {
                s.success()
                    .then_some(())
                    .ok_or_else(|| "copying failed".to_owned())
            })
    } else {
        fs::rename(&unpacked, &target).map_err(|e| e.to_string())
    };
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(&zip);
    status.map_err(|e| format!("Installing failed: {e}"))
}

/// What the bottom screen offers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Choice {
    Get,
    WifiOn,
    Cancel,
    Retry,
    Leave,
}

/// The screens' state.
enum Phase {
    /// Asking: what Wi-Fi allows.
    Ask,
    /// Turning Wi-Fi on, since when.
    WifiStarting(Instant),
    /// Getting it.
    Busy(Job),
    /// It failed: why.
    Failed(String),
    /// nibOS's package is incomplete (no release.conf): nothing to offer.
    Broken(String),
}

struct App {
    release: Release,
    update: Option<String>,
    phase: Phase,
    menu: Menu,
    choices: Vec<Choice>,
}

impl App {
    /// The menu for the phase and the radio: rows and what each does.
    fn rebuild(&mut self, wifi: Option<Wifi>) {
        let size = megabytes(self.release.size);
        let mut rows = Vec::new();
        let mut choices = Vec::new();
        let mut push = |row: Row, choice: Option<Choice>| {
            rows.push(row);
            choices.push(choice);
        };
        let get_label = if self.update.is_some() {
            "Update PortMaster"
        } else {
            "Download PortMaster"
        };
        let leave = if self.update.is_some() {
            "Not now: open the one there is"
        } else {
            "Not now"
        };
        match &self.phase {
            Phase::Ask => match wifi {
                // Not read yet: the radio thread answers within a moment.
                None => {
                    push(Row::heading("Checking Wi-Fi…"), None);
                    push(Row::new(leave, Value::None), Some(Choice::Leave));
                }
                Some(Wifi::Connected { .. }) => {
                    push(
                        Row::new(get_label, Value::None).with_note(size),
                        Some(Choice::Get),
                    );
                    push(Row::new(leave, Value::None), Some(Choice::Leave));
                }
                Some(Wifi::Connecting) => {
                    push(
                        Row::heading("Wi-Fi is on, not connected to a network yet"),
                        None,
                    );
                    push(
                        Row::heading("Join one on DSlot's System page if this doesn't change"),
                        None,
                    );
                    push(Row::new(leave, Value::None), Some(Choice::Leave));
                }
                _ => {
                    push(
                        Row::heading("Wi-Fi is off: PortMaster comes from the internet"),
                        None,
                    );
                    push(Row::new("Turn Wi-Fi on", Value::None), Some(Choice::WifiOn));
                    push(Row::new(leave, Value::None), Some(Choice::Leave));
                }
            },
            Phase::WifiStarting(_) => {
                push(Row::heading("Turning Wi-Fi on…"), None);
                push(Row::new(leave, Value::None), Some(Choice::Leave));
            }
            Phase::Busy(_) => push(Row::new("Cancel", Value::None), Some(Choice::Cancel)),
            Phase::Failed(_) => {
                push(Row::new("Try again", Value::None), Some(Choice::Retry));
                push(Row::new(leave, Value::None), Some(Choice::Leave));
            }
            Phase::Broken(_) => push(Row::new("Back", Value::None), Some(Choice::Leave)),
        }
        let keep = self.menu.row;
        self.menu = Menu {
            tabs: Vec::new(),
            title: if self.update.is_some() {
                "Update PortMaster".into()
            } else {
                "Get PortMaster".into()
            },
            rows,
            row: 0,
            scroll: 0,
            hint: if matches!(self.phase, Phase::Busy(_)) {
                "B cancel".into()
            } else {
                "A choose · B back".into()
            },
        };
        self.choices = choices
            .into_iter()
            .map(|c| c.unwrap_or(Choice::Leave))
            .collect();
        // The first row that does something, or the one that was highlighted if it still does.
        let takes = |i: usize| !self.menu.rows[i].heading;
        self.menu.row = if keep < self.menu.rows.len() && takes(keep) {
            keep
        } else {
            (0..self.menu.rows.len()).find(|&i| takes(i)).unwrap_or(0)
        };
        self.menu.settle();
    }
}

/// The top screen: what PortMaster is, or how getting it goes.
fn top_draws(app: &App) -> Vec<(String, f32, bool, f32, [u8; 3])> {
    let r = &app.release;
    let mut lines: Vec<(String, f32, bool, f32, [u8; 3])> = Vec::new();
    let mut y = 120.0;
    let mut line = |text: &str, size: f32, bold: bool, colour: [u8; 3], gap: f32| {
        lines.push((text.to_owned(), size, bold, y, colour));
        y += gap;
    };
    match &app.phase {
        Phase::Busy(job) => {
            line("Getting PortMaster", 56.0, true, colour::WHITE, 110.0);
            let done = job.bytes.load(Ordering::Relaxed).min(r.size);
            let what = match job.stage() {
                Stage::Downloading => format!(
                    "Downloading {} of {} from github.com/PortsMaster",
                    megabytes(done),
                    megabytes(r.size)
                ),
                Stage::Checking => "Checking it is the release nibOS expects…".into(),
                Stage::Unpacking => "Installing…".into(),
                Stage::Done => "Done".into(),
                Stage::Failed(_) => String::new(),
            };
            line(&what, 30.0, false, colour::DIM, 60.0);
        }
        Phase::Broken(why) => {
            line("PortMaster can't be got", 56.0, true, colour::WHITE, 110.0);
            line(why, 26.0, false, colour::RED, 50.0);
            line(
                "nibOS's portmaster package is incomplete: reinstall it.",
                26.0,
                false,
                colour::DIM,
                40.0,
            );
        }
        Phase::Failed(why) => {
            line(
                "PortMaster didn't install",
                56.0,
                true,
                colour::WHITE,
                110.0,
            );
            line(why, 26.0, false, colour::RED, 50.0);
            line("Nothing was changed.", 26.0, false, colour::DIM, 40.0);
        }
        _ => {
            let title = if app.update.is_some() {
                "Update PortMaster"
            } else {
                "Get PortMaster"
            };
            line(title, 56.0, true, colour::WHITE, 100.0);
            line(
                "PortMaster installs ports: Linux games and engines made for handhelds.",
                26.0,
                false,
                colour::WHITE,
                60.0,
            );
            match &app.update {
                Some(have) => line(
                    &format!("You have {have}; this nibOS knows {}.", r.version),
                    26.0,
                    false,
                    colour::DIM,
                    40.0,
                ),
                None => line(
                    "nibOS doesn't include it. This downloads it, once:",
                    26.0,
                    false,
                    colour::DIM,
                    40.0,
                ),
            }
            line(
                &format!(
                    "PortMaster {}, {}, from github.com/PortsMaster,",
                    r.version,
                    megabytes(r.size)
                ),
                26.0,
                false,
                colour::DIM,
                40.0,
            );
            line(
                "checked against the copy nibOS knows.",
                26.0,
                false,
                colour::DIM,
                60.0,
            );
            line(
                "PortMaster and everything it downloads come under their own licences.",
                26.0,
                false,
                colour::DIM,
                40.0,
            );
            line(
                "Ports are separate downloads you choose in PortMaster.",
                26.0,
                false,
                colour::DIM,
                40.0,
            );
        }
    }
    lines
}

/// The local time as nibOS shows it.
fn clock() -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r writes the struct we own; time(NULL) has no pointer to write.
    let ok = unsafe {
        let t = libc::time(std::ptr::null_mut());
        !libc::localtime_r(&t, &mut tm).is_null()
    };
    if !ok {
        return String::new();
    }
    nibkit::clock::format(tm.tm_hour as u32, tm.tm_min as u32, nibkit::clock::hour12())
}

fn status() -> Status {
    Status {
        clock: clock(),
        battery: nibkit::battery::read()
            .map(|b| (b.percent, b.state == nibkit::battery::State::Charging)),
        slot: Slot::One,
    }
}

/// One screen's window and the texture its picture goes into.
struct Screen {
    canvas: SdlCanvas<Window>,
    creator: TextureCreator<WindowContext>,
}

fn screens(video: &sdl2::VideoSubsystem) -> Result<[Screen; 2], String> {
    let bounds: Vec<Bounds> = (0..video.num_video_displays()?)
        .map(|i| {
            video
                .display_bounds(i)
                .map(|b| Bounds::new(b.x(), b.y(), b.width(), b.height()))
        })
        .collect::<Result<_, _>>()?;
    let placement = place(&bounds, false).ok_or("these displays don't look like the two panels")?;
    let open = |display: usize| -> Result<Screen, String> {
        let b = video.display_bounds(display as i32)?;
        let window = video
            .window("portmaster-get", b.width(), b.height())
            .position(b.x(), b.y())
            .fullscreen_desktop()
            .opengl()
            .build()
            .map_err(|e| e.to_string())?;
        let canvas = window
            .into_canvas()
            .accelerated()
            .present_vsync()
            .build()
            .map_err(|e| e.to_string())?;
        let creator = canvas.texture_creator();
        Ok(Screen { canvas, creator })
    };
    Ok([
        open(placement.top.display)?,
        open(placement.bottom.display)?,
    ])
}

/// Copies an RGBA picture to the screen.
fn show(screen: &mut Screen, picture: &Canvas) -> Result<(), String> {
    let mut texture: Texture = screen
        .creator
        .create_texture_static(
            PixelFormatEnum::ABGR8888,
            picture.w as u32,
            picture.h as u32,
        )
        .map_err(|e| e.to_string())?;
    texture
        .update(None, &picture.px, picture.w * 4)
        .map_err(|e| e.to_string())?;
    screen.canvas.copy(&texture, None, None)?;
    screen.canvas.present();
    Ok(())
}

/// The key a gamepad button is, by what is printed on it.
fn key(button: Button) -> Option<Key> {
    let face = |f: Face| match sdl_name(f) {
        Face::A => Button::A,
        Face::B => Button::B,
        Face::X => Button::X,
        Face::Y => Button::Y,
    };
    Some(match button {
        b if b == face(Face::A) => Key::A,
        b if b == face(Face::B) => Key::B,
        Button::DPadUp => Key::Up,
        Button::DPadDown => Key::Down,
        Button::DPadLeft => Key::Left,
        Button::DPadRight => Key::Right,
        _ => return None,
    })
}

fn main() {
    std::process::exit(match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("portmaster-get: {e}");
            1
        }
    });
}

/// The Wi-Fi radio, read on a thread of its own every 700 ms: the reads run small commands,
/// which must never hold up a frame. `None` until the first read.
fn radio() -> Arc<Mutex<Option<Wifi>>> {
    let now = Arc::new(Mutex::new(None));
    let shared = now.clone();
    thread::spawn(move || loop {
        let wifi = net::radios().wifi;
        *shared.lock().unwrap() = Some(wifi);
        thread::sleep(Duration::from_millis(700));
    });
    now
}

/// A spinner of 8 dots in a ring around `(cx, cy)`, the bright one at `tick` (one step a
/// tenth of a second), the ones behind it fading.
fn spinner(cx: f32, cy: f32, tick: u32) -> Vec<Draw> {
    (0..8u32)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 8.0 - std::f32::consts::FRAC_PI_2;
            let behind = (tick + 8 - i) % 8;
            let t = 1.0 - behind as f32 / 8.0;
            let mix = |a: u8, b: u8| (a as f32 * (1.0 - t) + b as f32 * t) as u8;
            let (p, g) = (colour::PANEL, colour::GOLD);
            Draw::Round {
                x: cx + a.cos() * 26.0 - 6.0,
                y: cy + a.sin() * 26.0 - 6.0,
                w: 12.0,
                h: 12.0,
                radius: 6.0,
                colour: [mix(p[0], g[0]), mix(p[1], g[1]), mix(p[2], g[2])],
            }
        })
        .collect()
}

/// Draws both screens; `spin` is the spinner's step while something is happening.
fn draw(
    screens: &mut [Screen; 2],
    fonts: &Fonts,
    app: &App,
    spin: Option<u32>,
) -> Result<(), String> {
    let mut top = Canvas::new(1.0);
    top.clear(colour::BLACK);
    for (text, size, bold, y, c) in top_draws(app) {
        top.text(fonts, &text, (size, bold), (80.0, y), 0.0, c);
    }
    if let Some(tick) = spin {
        top.draw(fonts, &spinner(W - 120.0, 150.0, tick), |_| None);
    }
    if let Phase::Busy(job) = &app.phase {
        // The progress bar under the words.
        let done = job.bytes.load(Ordering::Relaxed).min(app.release.size) as f32
            / app.release.size.max(1) as f32;
        let (x, y, w, h) = (80.0, 330.0, W - 160.0, 14.0);
        let bar = [
            Draw::Round {
                x,
                y,
                w,
                h,
                radius: 7.0,
                colour: colour::PANEL,
            },
            Draw::Round {
                x,
                y,
                w: (w * done).max(h),
                h,
                radius: 7.0,
                colour: colour::GOLD,
            },
        ];
        top.draw(fonts, &bar, |_| None);
    }
    let mut bottom = Canvas::new(1.0);
    bottom.clear(colour::BLACK);
    bottom.draw(fonts, &layout::menu(&app.menu, &status()), |_| None);
    let [t, b] = screens;
    show(t, &top)?;
    show(b, &bottom)
}

/// An RGBA canvas as nibos-splash's picture: "PSPL", version 1, a count, then per screen (top
/// first) width, height, a byte count and an LZ4 block of XRGB8888 rows, all u32 little-endian.
fn pspl(screens: &[&Canvas]) -> Vec<u8> {
    let mut out = b"PSPL".to_vec();
    out.extend(1u32.to_le_bytes());
    out.extend((screens.len() as u32).to_le_bytes());
    for c in screens {
        // XRGB8888 is B, G, R, X in memory.
        let px: Vec<u8> =
            c.px.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[2], p[1], p[0], 255])
                .collect();
        let block = lz4_flex::block::compress(&px);
        for v in [c.w as u32, c.h as u32, block.len() as u32] {
            out.extend(v.to_le_bytes());
        }
        out.extend(block);
    }
    out
}

/// The screens while PortMaster's own app starts (it takes seconds): nibos-splash holds them
/// up over the handover (`/usr/bin/portmaster`).
fn starting_picture() -> Vec<u8> {
    let fonts = Fonts::new();
    let mut top = Canvas::new(1.0);
    top.clear(colour::BLACK);
    top.text(
        &fonts,
        "Starting PortMaster…",
        (56.0, true),
        (80.0, 120.0),
        0.0,
        colour::WHITE,
    );
    top.text(
        &fonts,
        "It opens on this screen in a few seconds.",
        (26.0, false),
        (80.0, 230.0),
        0.0,
        colour::DIM,
    );
    let mut bottom = Canvas::new(1.0);
    bottom.clear(colour::BLACK);
    let menu = Menu {
        title: "PortMaster".into(),
        rows: vec![Row::heading("Starting PortMaster…")],
        ..Menu::default()
    };
    bottom.draw(&fonts, &layout::menu(&menu, &status()), |_| None);
    pspl(&[&top, &bottom])
}

fn run() -> Result<i32, String> {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("--picture") {
        let path = args.next().ok_or("usage: portmaster-get --picture FILE")?;
        fs::write(&path, starting_picture()).map_err(|e| format!("{path}: {e}"))?;
        return Ok(0);
    }
    let update = match first.as_deref() {
        Some("--update") => Some(args.next().unwrap_or_else(|| "an older one".into())),
        _ => None,
    };
    let leave_code = if update.is_some() { 3 } else { 1 };
    // Without its release.conf it says so on the screens rather than leave them black.
    let release = fs::read_to_string(RELEASE_CONF)
        .ok()
        .and_then(|t| Release::parse(&t));
    eprintln!(
        "portmaster-get: started ({}, pinned {})",
        update
            .as_deref()
            .map_or("first install".to_owned(), |v| format!("update from {v}")),
        release.as_ref().map_or("none", |r| &r.version)
    );

    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let controllers = sdl.game_controller()?;
    let mut events = sdl.event_pump()?;
    let mut screens = screens(&video)?;
    let mut pads: Vec<GameController> = Vec::new();
    let fonts = Fonts::new();

    let mut app = App {
        phase: match &release {
            Some(_) => Phase::Ask,
            None => Phase::Broken(format!("{RELEASE_CONF} is missing or incomplete.")),
        },
        release: release.unwrap_or(Release {
            version: String::new(),
            url: String::new(),
            sha256: String::new(),
            size: 0,
        }),
        update,
        menu: Menu::default(),
        choices: Vec::new(),
    };
    let radio = radio();
    let mut wifi = None;
    app.rebuild(wifi);
    let mut dirty = true;
    let mut shown = false;
    let started = Instant::now();

    loop {
        // Something is happening that the user waits for: a spinner, redrawn ten times a second.
        let spinning = matches!(app.phase, Phase::Busy(_) | Phase::WifiStarting(_))
            || (matches!(app.phase, Phase::Ask) && wifi.is_none());
        let tick = (started.elapsed().as_millis() / 100) as u32;
        if dirty || spinning {
            dirty = false;
            draw(&mut screens, &fonts, &app, spinning.then_some(tick))?;
            if !shown {
                shown = true;
                eprintln!("portmaster-get: showing the screens");
            }
        }

        // Events, or a tick: the download's progress and the radio change on their own.
        let waited = events.wait_event_timeout(if spinning { 100 } else { 250 });
        let mut effects = Vec::new();
        for event in waited.into_iter().chain(events.poll_iter()) {
            match event {
                // SIGTERM (DSlot's power menu, shutdown) arrives as Quit.
                Event::Quit { .. } => effects.push(Effect::Back),
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Ok(pad) = controllers.open(which) {
                        pads.push(pad);
                    }
                }
                // Menu (guide) reaches apps as a tap: leave, as everywhere on nibOS.
                Event::ControllerButtonUp {
                    button: Button::Guide,
                    ..
                } => effects.push(Effect::Back),
                // Select + Start leaves too, as in the ports.
                Event::ControllerButtonDown {
                    button: Button::Start | Button::Back,
                    ..
                } if pads
                    .iter()
                    .any(|p| p.button(Button::Back) && p.button(Button::Start)) =>
                {
                    effects.push(Effect::Back)
                }
                Event::ControllerButtonDown { button, .. } => {
                    if let Some(k) = key(button) {
                        effects.extend(app.menu.key(k));
                        dirty = true;
                    }
                }
                Event::FingerDown { x, y, .. } => {
                    effects.extend(app.menu.touch(x * W, y * H));
                    dirty = true;
                }
                _ => {}
            }
        }

        for effect in effects {
            let choice = match effect {
                Effect::Choose(row) => app.choices.get(row).copied(),
                Effect::Back => Some(Choice::Leave),
                _ => None,
            };
            match choice {
                Some(Choice::Get) | Some(Choice::Retry) => {
                    app.phase = Phase::Busy(Job::start(app.release.clone()));
                }
                Some(Choice::WifiOn) => {
                    eprintln!("portmaster-get: turning Wi-Fi on");
                    nibkit::device::set_wifi(true);
                    app.phase = Phase::WifiStarting(Instant::now());
                }
                Some(Choice::Cancel) => {
                    eprintln!("portmaster-get: cancelled");
                    if let Phase::Busy(job) = &app.phase {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    app.phase = Phase::Ask;
                }
                Some(Choice::Leave) => {
                    if let Phase::Busy(job) = &app.phase {
                        job.cancel.store(true, Ordering::Relaxed);
                        // The worker removes what it half wrote when it sees the cancel.
                        let until = Instant::now() + Duration::from_secs(3);
                        while matches!(
                            job.stage(),
                            Stage::Downloading | Stage::Checking | Stage::Unpacking
                        ) && Instant::now() < until
                        {
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                    eprintln!("portmaster-get: left without installing");
                    return Ok(leave_code);
                }
                None => {}
            }
            app.rebuild(wifi);
            dirty = true;
        }

        // The radio, as its thread last read it, while asking.
        if matches!(app.phase, Phase::Ask | Phase::WifiStarting(_)) {
            let now = *radio.lock().unwrap();
            if now != wifi {
                eprintln!("portmaster-get: Wi-Fi {now:?}");
                wifi = now;
                if let (Phase::WifiStarting(_), Some(Wifi::Connected { .. })) = (&app.phase, wifi) {
                    app.phase = Phase::Ask;
                }
                app.rebuild(wifi);
                dirty = true;
            }
            if let Phase::WifiStarting(since) = app.phase {
                // nibOS joins a known network within seconds; after 30 say what it is.
                if since.elapsed() > Duration::from_secs(30) {
                    app.phase = Phase::Ask;
                    app.rebuild(wifi);
                    dirty = true;
                }
            }
        }

        if let Phase::Busy(job) = &app.phase {
            match job.stage() {
                Stage::Done => return Ok(0),
                Stage::Failed(why) if why != CANCELLED => {
                    app.phase = Phase::Failed(why);
                    app.rebuild(wifi);
                }
                _ => {}
            }
            dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_conf_as_the_pkgbuild_writes_it() {
        let r = Release::parse(
            "# portmaster: the release nibOS pins\nPORTMASTER_VERSION=2026.10.21-0058\n\
             PORTMASTER_URL=https://example.org/PortMaster.zip\nPORTMASTER_SHA256=AB12\n\
             PORTMASTER_SIZE=24364552\n",
        )
        .unwrap();
        assert_eq!(r.version, "2026.10.21-0058");
        assert_eq!(r.sha256, "ab12");
        assert_eq!(r.size, 24_364_552);
        assert!(Release::parse("PORTMASTER_VERSION=1\n").is_none());
    }

    /// DSlot reads the Apps entry this package ships: it parses, runs the launcher, and goes by
    /// "Get PortMaster" until PortMaster is in place.
    #[test]
    fn apps_entry_is_what_dslot_reads() {
        let app = nibkit::dslot::App::parse(include_str!(
            "../../files/usr/share/dslot/apps/portmaster/app.toml"
        ))
        .unwrap();
        assert_eq!(app.command, ["/usr/bin/portmaster"]);
        let until = app.until.expect("an [until] table");
        assert_eq!(until.name, "Get PortMaster");
        assert_eq!(until.file, "~/.local/share/PortMaster/version");
    }

    fn release() -> Release {
        Release {
            version: "2026.10.21-0058".into(),
            url: "https://example.org/PortMaster.zip".into(),
            sha256: "ab12".into(),
            size: 24_364_552,
        }
    }

    /// Every screen the app can show is laid out and drawn without a panic (a panic aborts:
    /// the user would be left with black screens).
    #[test]
    fn every_screen_draws() {
        let fonts = Fonts::new();
        let wifis = [
            None,
            Some(Wifi::Unknown),
            Some(Wifi::Off),
            Some(Wifi::Connecting),
            Some(Wifi::Connected { bars: 3 }),
        ];
        let phases = || {
            vec![
                Phase::Ask,
                Phase::WifiStarting(Instant::now()),
                Phase::Failed(curl_failed(Some(6), "Could not resolve host")),
                Phase::Broken("/usr/lib/portmaster/release.conf is missing.".into()),
            ]
        };
        for update in [None, Some("2026.09.30-1207".to_owned())] {
            for wifi in wifis {
                for phase in phases() {
                    let mut app = App {
                        release: release(),
                        update: update.clone(),
                        phase,
                        menu: Menu::default(),
                        choices: Vec::new(),
                    };
                    app.rebuild(wifi);
                    assert!(!app.menu.rows.is_empty());
                    assert_eq!(app.menu.rows.len(), app.choices.len());
                    let mut bottom = Canvas::new(1.0);
                    bottom.draw(&fonts, &layout::menu(&app.menu, &Status::default()), |_| {
                        None
                    });
                    let mut top = Canvas::new(1.0);
                    for (text, size, bold, y, c) in top_draws(&app) {
                        top.text(&fonts, &text, (size, bold), (80.0, y), 0.0, c);
                    }
                }
            }
        }
    }

    /// A folder of its own under the system's temporary one, removed when dropped.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Temp {
            let p =
                std::env::temp_dir().join(format!("portmaster-get-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Temp(p)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A zip laid out as PortMaster's release is (PortMaster/version and a file), and its
    /// release description, served from a file:// address.
    fn fake_release(t: &Temp) -> Release {
        let src = t.0.join("src/PortMaster");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("version"), "2026.10.21-0058\n").unwrap();
        fs::write(src.join("control.txt"), "#!/bin/bash\n").unwrap();
        let zip = t.0.join("PortMaster.zip");
        let ok = Command::new("bsdtar")
            .args(["--format", "zip", "-cf"])
            .arg(&zip)
            .arg("-C")
            .arg(t.0.join("src"))
            .arg("PortMaster")
            .status()
            .unwrap();
        assert!(ok.success());
        let out = Command::new("sha256sum").arg(&zip).output().unwrap();
        let sum = String::from_utf8_lossy(&out.stdout);
        Release {
            version: "2026.10.21-0058".into(),
            url: format!("file://{}", zip.display()),
            sha256: sum.split_whitespace().next().unwrap().to_owned(),
            size: fs::metadata(&zip).unwrap().len(),
        }
    }

    fn try_get(t: &Temp, release: &Release, cancel: bool) -> Result<(), String> {
        let (data, cache) = (t.0.join("data"), t.0.join("cache"));
        get(
            release,
            (&data, &cache),
            &Mutex::new(Stage::Downloading),
            &AtomicU64::new(0),
            &AtomicBool::new(cancel),
        )
    }

    #[test]
    fn installs_a_good_release() {
        let t = Temp::new("good");
        let r = fake_release(&t);
        try_get(&t, &r, false).unwrap();
        let v = fs::read_to_string(t.0.join("data/PortMaster/version")).unwrap();
        assert_eq!(v.trim(), "2026.10.21-0058");
        // Nothing left behind: no zip, no staging folder.
        assert!(!t.0.join("data/.PortMaster-unpacking").exists());
        assert_eq!(fs::read_dir(t.0.join("cache")).unwrap().count(), 0);
        // Again, over the copy there is (an update), keeping the user's files.
        fs::write(t.0.join("data/PortMaster/config.json"), "{}").unwrap();
        try_get(&t, &r, false).unwrap();
        assert!(t.0.join("data/PortMaster/config.json").exists());
    }

    #[test]
    fn refuses_a_release_with_the_wrong_checksum() {
        let t = Temp::new("sum");
        let mut r = fake_release(&t);
        r.sha256 = "00".repeat(32);
        let e = try_get(&t, &r, false).unwrap_err();
        assert!(e.contains("isn't the release nibOS expects"), "{e}");
        assert!(!t.0.join("data/PortMaster").exists());
        assert_eq!(fs::read_dir(t.0.join("cache")).unwrap().count(), 0);
    }

    #[test]
    fn says_no_internet_when_the_name_doesnt_resolve() {
        let t = Temp::new("dns");
        let mut r = fake_release(&t);
        r.url = "https://portmaster.invalid/PortMaster.zip".into();
        let e = try_get(&t, &r, false).unwrap_err();
        assert!(e.starts_with("No internet"), "{e}");
    }

    #[test]
    fn says_what_failed_when_the_file_is_missing() {
        let t = Temp::new("missing");
        let mut r = fake_release(&t);
        r.url = format!("file://{}/nothing.zip", t.0.display());
        let e = try_get(&t, &r, false).unwrap_err();
        assert!(e.starts_with("The download failed"), "{e}");
        assert_eq!(fs::read_dir(t.0.join("cache")).unwrap().count(), 0);
    }

    #[test]
    fn checks_for_room_first() {
        let t = Temp::new("room");
        let mut r = fake_release(&t);
        r.size = u64::MAX / 4;
        let e = try_get(&t, &r, false).unwrap_err();
        assert!(e.starts_with("Not enough room"), "{e}");
    }

    #[test]
    fn a_cancel_leaves_nothing() {
        let t = Temp::new("cancel");
        let r = fake_release(&t);
        assert_eq!(try_get(&t, &r, true).unwrap_err(), CANCELLED);
        assert!(!t.0.join("data/PortMaster").exists());
        assert_eq!(fs::read_dir(t.0.join("cache")).unwrap().count(), 0);
    }

    #[test]
    fn curl_failures_in_words() {
        assert!(curl_failed(Some(7), "").starts_with("Couldn't reach GitHub"));
        assert!(curl_failed(Some(22), "404").contains("404"));
        assert_eq!(curl_failed(None, CANCELLED), CANCELLED);
    }

    /// The "Starting PortMaster..." picture is nibos-splash's format: two 1024 x 768 screens,
    /// each an LZ4 block of XRGB8888 that decompresses to its size.
    #[test]
    fn starting_picture_is_a_splash_picture() {
        let data = starting_picture();
        let u32_at = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(&data[..4], b"PSPL");
        assert_eq!((u32_at(4), u32_at(8)), (1, 2));
        let mut at = 12;
        for _ in 0..2 {
            let (w, h, len) = (u32_at(at), u32_at(at + 4), u32_at(at + 8));
            assert_eq!((w, h), (1024, 768));
            at += 12;
            let px = lz4_flex::block::decompress(&data[at..at + len], w * h * 4).unwrap();
            assert_eq!(px.len(), w * h * 4);
            // Something drawn: not all black.
            assert!(px.as_chunks::<4>().0.iter().any(|p| p[..3] != [0, 0, 0]));
            at += len;
        }
        assert_eq!(at, data.len());
    }

    /// With PORTMASTER_GET_SHOTS=DIR, the main screens as nibos-splash pictures in DIR, to look
    /// at off the device (the pictures' format is the splash's).
    #[test]
    fn screens_for_review() {
        let Some(dir) = std::env::var_os("PORTMASTER_GET_SHOTS") else {
            return;
        };
        let fonts = Fonts::new();
        let shots: Vec<(&str, Phase, Option<Wifi>)> = vec![
            ("wifi-off", Phase::Ask, Some(Wifi::Off)),
            ("offer", Phase::Ask, Some(Wifi::Connected { bars: 3 })),
            ("checking-wifi", Phase::Ask, None),
            (
                "failed",
                Phase::Failed(curl_failed(Some(6), "")),
                Some(Wifi::Off),
            ),
        ];
        for (name, phase, wifi) in shots {
            let mut app = App {
                release: release(),
                update: None,
                phase,
                menu: Menu::default(),
                choices: Vec::new(),
            };
            app.rebuild(wifi);
            let mut top = Canvas::new(1.0);
            top.clear(colour::BLACK);
            for (text, size, bold, y, c) in top_draws(&app) {
                top.text(&fonts, &text, (size, bold), (80.0, y), 0.0, c);
            }
            if wifi.is_none() {
                top.draw(&fonts, &spinner(W - 120.0, 150.0, 3), |_| None);
            }
            let mut bottom = Canvas::new(1.0);
            bottom.clear(colour::BLACK);
            bottom.draw(&fonts, &layout::menu(&app.menu, &Status::default()), |_| {
                None
            });
            fs::write(
                Path::new(&dir).join(format!("{name}.pspl")),
                pspl(&[&top, &bottom]),
            )
            .unwrap();
        }
    }

    #[test]
    fn a_spinner_has_eight_dots_one_bright() {
        let dots = spinner(100.0, 100.0, 3);
        assert_eq!(dots.len(), 8);
        let gold = dots
            .iter()
            .filter(|d| matches!(d, Draw::Round { colour, .. } if *colour == colour::GOLD))
            .count();
        assert_eq!(gold, 1);
    }

    #[test]
    fn sizes_in_megabytes() {
        assert_eq!(megabytes(24_364_552), "24.4 MB");
    }
}
