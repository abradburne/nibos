//! `nibos-drop`: headless LocalSend v2.1 ROM receiver daemon for nibOS.
//!
//! Run by `nibos-drop.service` (off by default; toggled by `nib drop on|off` or DSlot's Network
//! menu), or directly on a desktop with `--roms-dir` for live testing against LocalSend.

use nibos_hw::drop::server::{DropServer, ServerOptions};
use nibos_hw::drop::{
    board_model, resolve_target, Config, Status, TargetMode, DEFAULT_PORT, EMULATORS_DIR,
    STATE_DIR, STATUS_FILE,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn usage() -> ! {
    eprintln!(
        "usage: nibos-drop [OPTIONS]\n\
         \x20      nibos-drop show [--state-dir DIR] [--status FILE]\n\
         \x20      nibos-drop config [--state-dir DIR] [--alias NAME] [--pin CODE|off] [--target auto|tf1|tf2] [--artwork on|off]\n\
         \n\
         options:\n\
         \x20 --roms-dir DIR        destination Roms/ folder (default: TF2 /mnt/ds/Roms else TF1 /mnt/roms/Roms)\n\
         \x20 --state-dir DIR       persistent config and TLS cert folder (default: {STATE_DIR})\n\
         \x20 --status FILE         runtime status JSON path (default: {STATUS_FILE})\n\
         \x20 --emulators-dir DIR   DSlot emulator .toml folder (default: {EMULATORS_DIR})\n\
         \x20 --port N              TCP/UDP port (default: {DEFAULT_PORT})\n\
         \x20 --alias NAME          device name shown in LocalSend\n\
         \x20 --pin CODE            require a PIN for uploads\n\
         \x20 --target auto|tf1|tf2 card preference\n\
         \x20 --no-multicast        disable UDP multicast discovery"
    );
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut show = false;
    let mut set_config = false;
    let mut artwork_override: Option<bool> = None;
    let mut opts = ServerOptions {
        port: DEFAULT_PORT,
        state_dir: PathBuf::from(STATE_DIR),
        status_path: PathBuf::from(STATUS_FILE),
        emulators_dir: PathBuf::from(EMULATORS_DIR),
        roms_override: None,
        alias_override: None,
        pin_override: None,
        target_override: None,
        model: board_model(),
        enable_multicast: true,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "show" => show = true,
            "config" => set_config = true,
            "-h" | "--help" => usage(),
            "--roms-dir" => {
                opts.roms_override = Some(PathBuf::from(args.next().unwrap_or_else(|| usage())))
            }
            "--state-dir" => opts.state_dir = PathBuf::from(args.next().unwrap_or_else(|| usage())),
            "--status" => opts.status_path = PathBuf::from(args.next().unwrap_or_else(|| usage())),
            "--emulators-dir" => {
                opts.emulators_dir = PathBuf::from(args.next().unwrap_or_else(|| usage()))
            }
            "--port" => {
                let s = args.next().unwrap_or_else(|| usage());
                opts.port = s.parse().unwrap_or_else(|_| usage());
            }
            "--alias" => opts.alias_override = Some(args.next().unwrap_or_else(|| usage())),
            "--pin" => opts.pin_override = Some(args.next().unwrap_or_else(|| usage())),
            "--target" => {
                let s = args.next().unwrap_or_else(|| usage());
                opts.target_override = Some(TargetMode::parse(&s).unwrap_or_else(|| usage()));
            }
            "--artwork" => {
                let s = args.next().unwrap_or_else(|| usage());
                artwork_override = Some(match s.to_ascii_lowercase().as_str() {
                    "on" | "true" | "1" | "yes" => true,
                    "off" | "false" | "0" | "no" => false,
                    _ => usage(),
                });
            }
            "--no-multicast" => opts.enable_multicast = false,
            _ => usage(),
        }
    }

    if set_config {
        let mut cfg = Config::load(&opts.state_dir);
        if let Some(alias) = opts.alias_override {
            cfg.alias = if alias.eq_ignore_ascii_case("default") {
                String::new()
            } else {
                alias
            };
        }
        if let Some(pin) = opts.pin_override {
            cfg.pin = if pin.eq_ignore_ascii_case("off") || pin.eq_ignore_ascii_case("none") {
                String::new()
            } else {
                pin
            };
        }
        if let Some(target) = opts.target_override {
            cfg.target = target;
        }
        if let Some(on) = artwork_override {
            cfg.auto_artwork = on;
        }
        if let Err(e) = cfg.save(&opts.state_dir) {
            eprintln!("nibos-drop: cannot save config: {e}");
            std::process::exit(1);
        }
        return;
    }

    if show {
        print_status(&opts);
        return;
    }

    // SAFETY: `on_signal` only sets an `AtomicBool` (async-signal-safe).
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }

    let srv = match DropServer::start(opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nibos-drop: failed to start: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "nibos-drop: listening on https://0.0.0.0:{} as \"{}\" (fingerprint {}...)",
        srv.bound_addr.port(),
        srv.info.alias,
        &srv.info.fingerprint[..12]
    );

    let stop_flag: Arc<AtomicBool> = srv.stop_flag();
    while !SHUTDOWN.load(Ordering::Relaxed) && !stop_flag.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(100));
    }
    eprintln!("nibos-drop: stopping");
    srv.shutdown();
}

fn print_status(opts: &ServerOptions) {
    let cfg = Config::load(&opts.state_dir);
    let status = Status::load(&opts.status_path);
    let active = status.as_ref().is_some_and(|s| s.active);
    let alias = status
        .as_ref()
        .map(|s| s.alias.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| (!cfg.alias.is_empty()).then(|| cfg.alias.clone()))
        .unwrap_or_else(|| nibos_hw::drop::DEFAULT_ALIAS.to_owned());
    let pin = if !cfg.pin.is_empty() { "set" } else { "off" };
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let target_line = match resolve_target(cfg.target, opts.roms_override.as_deref(), &mounts) {
        Ok(t) => format!("{} ({})", t.label, t.roms_dir.display()),
        Err(e) => e,
    };

    println!("state:   {}", if active { "on" } else { "off" });
    println!("alias:   {alias}");
    println!("target:  {} -> {target_line}", cfg.target.as_str());
    println!("pin:     {pin}");
    println!("artwork: {}", if cfg.auto_artwork { "on" } else { "off" });
    if let Some(st) = status {
        if let Some(t) = &st.transfer {
            let pct = (t.received * 100)
                .checked_div(t.total)
                .unwrap_or(0)
                .min(100);
            println!("active:  {} ({pct}%)", t.file);
        }
        if st.counts.accepted > 0 || st.counts.rejected > 0 {
            println!(
                "session: {} accepted, {} rejected",
                st.counts.accepted, st.counts.rejected
            );
        }
        for ev in st.recent.iter().take(5) {
            if ev.outcome == "accepted" {
                println!("  {}  OK   {} -> {}", ev.time, ev.file, ev.console);
            } else {
                println!("  {}  NO   {} ({})", ev.time, ev.file, ev.reason);
            }
        }
    }
}
