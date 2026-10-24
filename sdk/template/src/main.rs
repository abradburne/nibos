//! nibos-template: a nibOS app to start from. Both screens, the gamepad, touch and sound,
//! as AGENTS.md describes them, in one file:
//!
//! - one fullscreen window per screen on SDL's KMSDRM backend, placed by nibkit, each with an
//!   accelerated (GLES) renderer that waits for vsync: the loop sleeps in `present()`, it never
//!   spins;
//! - the merged gamepad through SDL's game controller API: the stick or the D-pad move a square
//!   on the top screen, the button labelled A beeps, B changes colour, Select + Start quits;
//! - touch on the bottom screen: a dot where each finger is;
//! - sound through SDL's audio (PipeWire underneath): a short beep, generated on the main thread
//!   and queued, so no audio callback runs on a real-time thread.
//!
//! Run it with `sdk/run.sh` from the repository (it gives the screens back to DSlot afterwards).

use nibkit::{
    input::{sdl_name, Face},
    screens::{place, Rect as Bounds},
};
use sdl2::{
    audio::{AudioQueue, AudioSpecDesired},
    controller::{Axis, Button, GameController},
    event::Event,
    pixels::Color,
    rect::Rect,
    render::Canvas,
    video::Window,
};
use std::collections::HashMap;
/// PipeWire runs at 48 kHz; asking for the same rate avoids resampling.
const RATE: i32 = 48_000;
/// Stick values beyond this (of ±32767) move the square; nibos-input reports 2048 as the flat
/// zone, a little more hides the resting noise.
const DEAD_ZONE: i16 = 4000;

struct Screen {
    canvas: Canvas<Window>,
    width: i32,
    height: i32,
}

/// A window on each screen: `[top, bottom]`. nibkit says which SDL display is which (on nibOS
/// display 0 is the bottom, touch panel, 1 the top).
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
        let i = display as i32;
        let b = video.display_bounds(i)?;
        let window = video
            .window("nibos-template", b.width(), b.height())
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
        eprintln!("nibos-template: display {i}: {}x{}", b.width(), b.height());
        Ok(Screen {
            canvas,
            width: b.width() as i32,
            height: b.height() as i32,
        })
    };
    Ok([
        open(placement.top.display)?,
        open(placement.bottom.display)?,
    ])
}

/// SDL's button for a face button's printed label.
fn button(label: Face) -> Button {
    match sdl_name(label) {
        Face::A => Button::A,
        Face::B => Button::B,
        Face::X => Button::X,
        Face::Y => Button::Y,
    }
}

/// A 60 ms sine at 880 Hz, faded in and out so it doesn't click; stereo s16.
fn beep() -> Vec<i16> {
    let n = RATE as usize * 60 / 1000;
    (0..n)
        .flat_map(|i| {
            let t = i as f32 / RATE as f32;
            let fade = (i.min(n - i) as f32 / 480.0).min(1.0);
            let s = ((t * 880.0 * std::f32::consts::TAU).sin() * fade * 8000.0) as i16;
            [s, s]
        })
        .collect()
}

fn main() -> Result<(), String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let controllers = sdl.game_controller()?;
    let audio = sdl.audio()?;
    let mut events = sdl.event_pump()?;

    let mut screens = screens(&video)?;
    let (beep_button, colour_button) = (button(Face::A), button(Face::B));
    // The merged gamepad ("Anbernic RG DS+ Gamepad") is the one controller on the device; a
    // Bluetooth or USB pad shows up as another.
    let mut pads: Vec<GameController> = Vec::new();
    let queue: AudioQueue<i16> = audio.open_queue(
        None,
        &AudioSpecDesired {
            freq: Some(RATE),
            channels: Some(2),
            samples: Some(1024),
        },
    )?;
    queue.resume();
    let tone = beep();

    let (mut x, mut y) = (512.0f32, 384.0f32);
    let mut colour = 0usize;
    let colours = [
        Color::RGB(255, 196, 0),
        Color::RGB(0, 200, 255),
        Color::RGB(255, 80, 120),
    ];
    let mut fingers: HashMap<i64, (f32, f32)> = HashMap::new();

    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. } => break 'running,
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Ok(pad) = controllers.open(which) {
                        eprintln!("nibos-template: gamepad: {}", pad.name());
                        pads.push(pad);
                    }
                }
                // SDL names face buttons by position, not by what is printed on them (the RG
                // DS+ has Nintendo's labels): nibkit translates.
                Event::ControllerButtonDown { button, .. } if button == beep_button => {
                    let _ = queue.queue_audio(&tone);
                }
                Event::ControllerButtonDown { button, .. } if button == colour_button => {
                    colour = (colour + 1) % colours.len();
                }
                Event::FingerDown {
                    finger_id, x, y, ..
                }
                | Event::FingerMotion {
                    finger_id, x, y, ..
                } => {
                    fingers.insert(finger_id, (x, y));
                }
                Event::FingerUp { finger_id, .. } => {
                    fingers.remove(&finger_id);
                }
                _ => {}
            }
        }

        // Select + Start held together quits. (Menu, the "guide" button, reaches apps as a
        // tap on release; Menu + volume is the system's brightness shortcut.)
        if pads
            .iter()
            .any(|p| p.button(Button::Back) && p.button(Button::Start))
        {
            break;
        }
        for pad in &pads {
            let axis = |a: Axis| {
                let v = pad.axis(a);
                if v.abs() > DEAD_ZONE {
                    v as f32 / 32767.0
                } else {
                    0.0
                }
            };
            let dpad = |minus: Button, plus: Button| {
                f32::from(i8::from(pad.button(plus)) - i8::from(pad.button(minus)))
            };
            x += (axis(Axis::LeftX) + dpad(Button::DPadLeft, Button::DPadRight)) * 8.0;
            y += (axis(Axis::LeftY) + dpad(Button::DPadUp, Button::DPadDown)) * 8.0;
        }

        let [top, bottom] = &mut screens;
        x = x.clamp(0.0, (top.width - 64) as f32);
        y = y.clamp(0.0, (top.height - 64) as f32);
        top.canvas.set_draw_color(Color::RGB(16, 16, 24));
        top.canvas.clear();
        top.canvas.set_draw_color(colours[colour]);
        top.canvas
            .fill_rect(Rect::new(x as i32, y as i32, 64, 64))?;

        bottom.canvas.set_draw_color(Color::RGB(24, 16, 16));
        bottom.canvas.clear();
        bottom.canvas.set_draw_color(Color::RGB(240, 240, 240));
        for &(fx, fy) in fingers.values() {
            let (cx, cy) = (
                (fx * bottom.width as f32) as i32,
                (fy * bottom.height as f32) as i32,
            );
            bottom
                .canvas
                .fill_rect(Rect::new(cx - 24, cy - 24, 48, 48))?;
        }

        // Each present waits for its panel's vblank (59.94 Hz); the two panels' vblanks are
        // 8.2 ms apart, so a frame takes about one refresh, not two.
        for s in &mut screens {
            s.canvas.present();
        }
    }
    eprintln!("nibos-template: bye");
    Ok(())
}

#[cfg(test)]
mod tests {
    /// DSlot (optional for a nibOS app) can show the app on its Apps tab: its description file
    /// parses, and its command is this program.
    #[test]
    fn app_toml_is_what_dslot_reads() {
        let app = nibkit::dslot::App::parse(include_str!("../app.toml")).unwrap();
        assert_eq!(app.command, [format!("./{}", env!("CARGO_PKG_NAME"))]);
    }
}
