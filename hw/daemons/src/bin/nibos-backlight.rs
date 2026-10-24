//! nibos-backlight: the screens' backlights by name, in percent
//! ([`nibos_hw::backlight`]).
//!
//! ```text
//! nibos-backlight [get [SCREEN|all]]        each screen's level
//! nibos-backlight set SCREEN|all PERCENT    set (0-100; 0 is the floor), and save
//! nibos-backlight set SCREEN|all +N|-N      move by N percent, and save
//! nibos-backlight save                      keep every level (at shutdown)
//! nibos-backlight add NAME                  from udev: link, open to video, set the saved level
//! ```
//!
//! `nib brightness` runs it. Setting needs the video group or root; `add`
//! needs root.

use nibos_hw::{
    backlight::{group_id, Backlight, Backlights, Target},
    board::Board,
};
use std::process::exit;

const USAGE: &str = "usage: nib brightness [SCREEN|all [PERCENT|+N|-N]]
       nibos-backlight get [SCREEN|all] | set SCREEN|all PERCENT|+N|-N | save | add NAME";

fn fail(msg: impl std::fmt::Display, code: i32) -> ! {
    eprintln!("nibos-backlight: {msg}");
    exit(code);
}

fn print(lights: &[Backlight]) {
    for b in lights {
        println!(
            "{:<8} {:>3}%  ({}/{})",
            b.screen,
            b.percent(),
            b.raw,
            b.range.max
        );
    }
}

enum Value {
    Percent(u32),
    Step(i32),
}

/// `40` (a level), `+10` or `-10` (a step).
fn parse_value(text: &str) -> Option<Value> {
    if text.starts_with(['+', '-']) {
        text.parse().ok().map(Value::Step)
    } else {
        text.parse().ok().filter(|&p| p <= 100).map(Value::Percent)
    }
}

fn main() {
    let board = Board::load().unwrap_or_else(|e| fail(e, 1));
    let lights = Backlights::for_board(&board);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        [] | ["get"] => lights.select(&Target::All).map(|l| print(&l)),
        ["get", target] => lights.select(&Target::parse(target)).map(|l| print(&l)),
        ["set", target, value] => {
            let target = Target::parse(target);
            match parse_value(value) {
                Some(Value::Percent(p)) => lights.set(&target, p),
                Some(Value::Step(n)) => lights.shift(&target, n),
                None => fail(format!("{value}: give 0-100, +N or -N\n{USAGE}"), 2),
            }
            .map(|l| print(&l))
        }
        ["save"] => lights.save(),
        ["add", name] => {
            let video = std::fs::read_to_string("/etc/group")
                .ok()
                .and_then(|g| group_id(&g, "video"));
            lights.add(name, video)
        }
        ["-h" | "--help" | "help"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => fail(USAGE, 2),
    };
    if let Err(e) = result {
        fail(e, 1);
    }
}
