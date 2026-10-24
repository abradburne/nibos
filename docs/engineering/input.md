# Input

## What the kernel sees

The device tree describes the controls as the stock tree does, with mainline drivers (as
`/proc/bus/input/devices` lists them on an RG DS+ running nibOS, 2026-10-18):

| Input device | Driver | What |
|---|---|---|
| `gpio-keys-control` | gpio-keys | Face buttons, shoulders and triggers, Select/Start, the D-pad, Menu (KEY_HOME), stick click |
| `adc-joystick` | adc-joystick (SAR ADC through a GPIO mux) | The stick, as raw ADC values |
| `gpio-keys-volume` | gpio-keys | Volume up and down |
| `adc-keys-back` | adc-keys | Back (KEY_BACK) |
| `adc-keys-play` | adc-keys | A key reported as KEY_PLAYPAUSE |
| `gpio-keys-hall` | gpio-keys | The lid (`SW_LID`) |
| `rk805 pwrkey` | rk8xx | The power key: `nibos-input`'s (a tap sleeps, a hold opens the power menu or, with no boot UI, powers off; [hw/README.md](../../hw/README.md), "Lid and power") |
| `rk817_ext Headphones` | rk817 codec | The headphone jack switch |
| `Goodix Capacitive TouchScreen` | goodix | The bottom panel's touch screen (GT9xx, 1024×768) |
| `pwm-vibrator` | pwm-vibra | The rumble motor |

The face buttons are named by position, as the kernel's gamepad spec asks: `BTN_SOUTH` is the
bottom one, whatever is printed on it (the DS+ has Nintendo's layout, so that is B).

## One gamepad: nibos-input

Games and SDL want one gamepad, laid out as the kernel's
[gamepad spec](https://docs.kernel.org/input/gamepad.html) describes. `nibos-input`
([hw/daemons](../../hw/README.md)) grabs the button and stick devices and re-emits them as one
uinput device, **"Anbernic RG DS+ Gamepad"** (the names come from the board's `board.conf`):

- face, shoulder, trigger, Select/Start and stick-click buttons pass through;
- Menu (KEY_HOME) becomes `BTN_MODE`, SDL's "guide";
- the D-pad becomes the `ABS_HAT0X/Y` hat, −1 left/up, +1 right/down; opposite directions held
  together cancel out instead of the last one winning;
- the stick is re-centred on its resting value, read at start-up, and scaled to ±32767 on each
  side of centre on its own, since the ADC's range is not symmetric around the rest position;
  a 2048 dead zone is reported to readers.

The logic is a library with unit tests (`src/gamepad.rs`); the binary only opens the devices
and runs the loop. The board's udev rules hide the raw devices from games and libinput and
give the merged gamepad `uaccess`. Volume, Back, the lid, power and touch stay on their own
devices, for the desktop, the shell or the program that wants them.

## Menu + volume: brightness

Holding **Menu** and pressing **Volume up / down** changes both screens' brightness, a press to
the next multiple of 5% (39% → 40% or 35%, as DSlot's sliders step) and, held, a step for every
fifth key repeat (about three seconds over the range), within the floor
([hw/README.md](../../hw/README.md), "Brightness"). It works everywhere, because
`nibos-input` does it below every program. Menu is also the games' own key (the emulator's
menu, and its modifier for Menu + Start, Menu + L/R and so on), so `nibos-input` holds Menu's
press back until it knows which it is ([`src/controls.rs`](../../hw/daemons/src/controls.rs),
with tests):

- **Menu alone**: on release, the gamepad sends Menu's press and release together. A tap
  arrives late by as long as Menu was held; programs that act on Menu's release, as the
  emulator's menu does, see no difference.
- **Menu with a gamepad button or the D-pad**: the held press goes out first, then the button,
  so the game's Menu combinations work as before; moving the stick doesn't count.
- **Menu with a volume key**: the shortcut. Menu, its release included, never reaches the
  gamepad, and the volume key never reaches the volume keys device, so neither the game nor the
  volume changes.

To keep the volume keys out of the shortcut, `nibos-input` grabs the raw volume device too and
passes the keys on through a device of its own, **"nibOS volume keys"**, which `nibos-audio`
and desktops read. No other button or key is delayed.

## Keys that aren't on the gamepad

Back (KEY_BACK) is a key on a device of its own (`adc-keys-back`), not a gamepad button, so a
program binds it as a key; the same goes for the KEY_PLAYPAUSE key on `adc-keys-play`. SDL3 reports it as `AC_BACK`, but sdl2-compat 2.32.74 passes SDL3's keycode for it to SDL2
programs untranslated, where that number is SDL2's "Sleep": an SDL2 program should bind both
names.

## Sleep and wake

Closing the lid or a short press of power suspends (logind, the board's `logind.conf.d`);
holding power shuts down; opening the lid or power wakes (the board's udev rule enables both as
wake-up sources). logind's hold-off after waking is 5 s instead of 30, so a lid closed right
after waking still counts.

## Latency

From a button press to the screen depends on the program: ~60–100 ms median in an emulator,
measured from an injected press timed where it enters the kernel and where `nibos-input`
passes it on, to the page flip. `nibos-input` itself adds well under a millisecond.

## SDL in the boot UI

SDL's controller scan opens and closes every input device to see what it is, and each
`close()` of an evdev node waits for an RCU grace period: ~0.3–0.5 s at a game's start and at
DSlot's return to the shelf. RCU grace periods are expedited at run time
([nibos-rcu.conf](../../rootfs/dslot/nibos-rcu.conf), ~0.1 s), and DSlot starts its controller
scan after its first frame.

## Dead ends: don't repeat

Tried and measured, or found not to work; the source says more. Add to it when something doesn't pay.

- SDL's controller scan opening all 17 evdev nodes at start: ~0.5 s of RCU waits; after the first frame.
- logind handling the power key: acts on key-down, can't tell a tap from a hold; nibos-input owns it.
- A sleep hook's "resumed" mark to ignore the waking press: written too late; the clocks' jump instead.
- nibos-charge-boot reading the power key through evdev's synced device, and before the pwrkey device existed (~4 s into the boot): raw events, after waiting for the device.
- SDL2 programs binding only `AC_BACK` for Back: sdl2-compat turns it into SDL2's "Sleep"; bind both.
- udev triggered with "change" for the input rules: not applied until a restart; "add".
