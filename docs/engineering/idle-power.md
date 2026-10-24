# Idle power and wakeups

What the RG DS+ does when nothing happens on it: which threads wake, how often, and what the
battery says. Measured 2026-10-21 on the shelf (DSlot), battery discharging, kernel #51. Open
questions are at the end.

## How it's measured

- **Wakeups**: every thread's `voluntary_ctxt_switches` (it slept and was woken) and
  `nonvoluntary_ctxt_switches` over 30 s, from `/proc/*/task/*/status`, sorted.
- **Power**: the RK817's coulomb counter (`charge_now`) over 90 s windows, with `current_avg`
  sampled every 2 s beside it (the two agreed within a few mA); the charger unplugged. Note
  DSlot's screensaver state in the journal, which changes the picture.
- **Idle states**: `/sys/devices/system/cpu/cpu*/cpuidle/`.

## Findings

| What | Measured | Verdict |
|---|---|---|
| Total, idle shelf | 1,100-2,800 voluntary switches/s; ~488 mA (~1.9 W) | the starting point |
| DSlot drawing when nothing changes (also under its screensaver), before f93caa3 | `irq/76-panfrost-job` 46-154/s, the Panfrost fragment kworkers, DSlot's main thread up to 270 involuntary switches/s | the main source; fixed in DSlot f93caa3 (row below) |
| `sugov:0`, schedutil's thread | 650-1,400/s: ~7 per frequency change (each is an SCMI call into TF-A), 50-100 changes/s while DSlot draws | follows DSlot's load; a longer rate limit is a [dead end](suspend-and-wifi.md#power-and-suspend) (fewer wakeups, more current) |
| The speakers' keep-alive (`pw-cat` and PipeWire's data loop) | ~23/s each | by design ([audio.md](audio.md#the-speakers-keep-alive)); its current is still to be measured |
| nibOS's daemons (`nibos-input`, `nibos-audio`, the rest) | block on their devices; sleeps only as retry delays | nothing to fix |
| DSlot f93caa3 (stops drawing 3 s after the last input) | idle shelf: DSlot ~19 wakeups/s, no GPU jobs; 967-1,050 switches/s in all; 481 mA. Under the screensaver of that build (a fast bounce at 25 fps): 580 mA | idle drawing gone; the screensaver's cost is below |
| Wi-Fi up, idle (power save off: on, the RTL8821CS drops off) | 505 mA with it, 371 mA after `nib net down` (the screensaver came on during the second window, so the radio's share may be larger) | ~0.5 W, a quarter of the idle draw: images with DSlot start with Wi-Fi off (DSlot's switch turns it on); the base image and the dev card keep it on, for SSH |
| CPU idle states | no cpuidle driver: the cores only ever WFI | a candidate: PSCI idle states in the device tree, if TF-A supports them; to be measured |

### DSlot's screensaver (DSlot c63757d)

Clean run, unplugged, 82% to 74%, 4 minutes to settle and a 90 s window per state (`charge_now`
and `current_avg`):

| State | mA |
|---|---|
| Idle shelf | 516-536 |
| Saver, animated at 25 fps | 488 |
| Saver, 10 fps | 489 |
| Saver, 8 fps | 474 |
| Saver, 5 fps | 474 |
| Saver, still | 474 |
| Screens off | 467 |

A second run (62%): the saver at 15 fps, DSlot's default, 481 mA; still, 495 mA.
DSlot's "Screensaver style" setting picks Animated, Still or Screens off; it draws only when a
frame is due, with sub-pixel drift.

Every saver mode reads below the idle shelf because the saver dims both screens to 10%. At 15
fps or less it costs nothing measurable over a still picture; 25 fps costs about 14 mA more, and turning
the screens off saves only ~7 mA more than the still saver. The ~100 mA of the f93caa3 row was
that build's 25 fps fast-bounce saver.

## Open

- Kernel workers seen on the idle shelf: `thermal_events` (40-110/s), `events_freezable`
  (~150/s, with Wi-Fi up), `events_power_efficient`, `leds`: which work items, and whether
  any polls.
- The keep-alive stream's cost in mA, against the clipped first sound it prevents.
- Deeper CPU idle states: whether rkbin's TF-A takes PSCI `CPU_SUSPEND`, and what it saves.
