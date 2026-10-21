# Controller play (WP-S)

The headline goal of Oxidized Annihilation is that a game controller is a
first-class way to play TA:K. This page is the reference for how the
controller works, every feel value and what it does, how the defaults were
chosen, and how to record a play session and send it back.

Where things live: all feel logic and every tuning value are in
`tak-input` (core, integer-only, deterministic, tested headless). The
client (`client/`) adapts devices, draws the cursor, wheels and hints,
and hosts the tuning overlay. Tags as elsewhere: **Known**, **Believed**,
**Chosen**, **Unknown**.

## Try this first (Tyler)

```sh
cd client
export TAK_DATA_DIR="$HOME/Games/Total Annihilation Kingdoms"
cargo run --release -- --demo --input-profile ~/tak-input.json \
    --record-session ~/tak-session-1.json
```

1. Plug in the pad before starting. The hint bar (bottom centre) shows
   Xbox or PlayStation glyphs depending on the pad (`--glyphs xbox` or
   `--glyphs playstation` forces one).
2. Play for 5–10 minutes with the default `console` preset. Select the
   monarch (A / Cross), move (X / Square), open the orders wheel (LT / L2),
   build (LB / L1), brush-select (hold A), select everything on screen
   (Y / Triangle), zoom (RB + right stick), pan (right stick, or push the
   cursor into the screen edge).
3. Press **Menu / Options** (or F11) for the tuning overlay. Try the other
   presets on its first row (`assisted`, `precise`), nudge anything that
   feels off, and press **Y / Triangle** (or Enter) to save. The file is
   `~/tak-input.json`; editing it in a text editor also applies at once.
4. Quit normally (close the window). `~/tak-session-1.json` now holds the
   whole session. Send it back together with `~/tak-input.json` and a line
   or two on what felt wrong.

## Control reference

The default profile merges the controller and the mouse/keyboard bindings
(`Profile::default_combined`), so both devices drive one cursor and one
selection.

| Intent | Xbox | PlayStation | Mouse / keyboard |
|---|---|---|---|
| Move the cursor | Left stick (aim assist, see below) | Left stick | Mouse |
| Precision cursor (slower, no assist) | hold RT | hold R2 | |
| Select one unit | A tap | Cross tap | LMB click |
| Same type on screen | A A (second tap) | Cross Cross | Z |
| Same type on the whole map | LS click | L3 | Ctrl+Z |
| Brush select (grows while held) | hold A, paint with the stick | hold Cross | LMB drag (circle) |
| All own units on screen ("local") | Y tap | Triangle tap | Q |
| All own units on the map ("global") | Y Y (second tap) | Triangle Triangle | Ctrl+Q |
| Context order: move, attack enemy, guard friend, place building | X | Square | RMB |
| Queue the order | RB + X | R1 + Square | Shift + RMB |
| Orders wheel (stop, patrol, guard, fire states, spell, idle builder) | hold LT, stick to a slot, release | hold L2 | S, P, G, H, R, F, C |
| Build wheel (selected builder) | hold LB, stick to a slot, release | hold L1 | F1–F8, then RMB to place |
| Cancel wheel / brush / order mode, then deselect | B | Circle | Esc |
| Control groups 1–4: recall / store | D-pad / RB + D-pad | D-pad / R1 + D-pad | 0–9 / Ctrl+0–9 |
| Pan | Right stick, or push the cursor into the screen edge | Right stick | Arrow keys |
| Zoom | RB + right stick up/down | R1 + right stick | Mouse wheel |
| Centre on monarch (glides) | RS click tap | R3 tap | Home |
| Follow the selection (toggle; panning stops it) | hold RS click | hold R3 | V |
| Last alert / next idle builder | View tap / hold View | Share tap / hold Share | Space / Period |
| Tuning overlay | Menu | Options | F11 |
| Pause, game speed | | | Pause or Ctrl+P, `=` / `-` |

With nothing selected the orders wheel shows the *idle wheel* instead
(idle builder, last alert, monarch, all on screen, all units). With units
selected it shows only what they can do (no "Cast spell" without a
caster), AoE IV style (`wheel_context`).

**Halo Wars comparison (Chosen).** WP-J put same-type select on Y /
double Y. Halo Wars puts it on a double tap of A and has separate "local"
and "global" select buttons. The default now follows Halo Wars: a second
tap of A widens the selection to that type on screen *without* delaying
the first tap (new gesture `second_tap`: the first tap selects at once, a
WP-J-style `double_tap` would hold every A tap back by 300 ms), and Y /
second Y are local / global select, the most-used Halo Wars shortcut.
Halo Wars puts local/global on the bumpers; ours are taken by the build
wheel and the queue/chord modifier, so they moved to Y. The WP-J mapping is
still available as `Profile::wp_j_controller()` (its JSON is
`crates/tak-input/src/profiles/wp_j_controller.json`) and can be saved as
a profile file.

### Glyphs

The hint bar and the overlay use Xbox (A B X Y, LB RB LT RT, View, Menu)
or PlayStation (Cross Circle Square Triangle, L1 R1 L2 R2, Share,
Options) glyphs, chosen from the connected pad: Sony's USB vendor id
(`0x054c`) or a name containing DualSense / DualShock / PlayStation /
PS4 / PS5 (or exactly "Wireless Controller") gives PlayStation, anything
else Xbox (`--glyphs auto`, the default). The chips are drawn in code (no
art). The hints are read from the profile's bindings, so they stay right
after rebinding.

### Cursors

The game's own cursors from `anims/CURSORS.GAF` (palette
`anims/CURSORS.PCX`, **Known**: decodes cleanly, 23 sequences) replace
the OS cursor, by context (`Mapper::cursor_kind`; mapping **Chosen** from
the sequence names):

| Context | Sequence |
|---|---|
| nothing to do | `cursornormal` |
| over an own unit that a click selects | `cursorselect` |
| order would move | `CursorMove` |
| order would attack | `CursorAttack` |
| order would guard | `CursorDefend` |
| patrol mode | `CursorPatrol` |
| build mode, placeable / blocked (the sim's site check) | `cursorgrn` / `cursorred` |
| spell mode | `Cursorresurrect` |
| pending order not possible here | `cursortoofar` |

Animated sequences play at 30 ticks per second with each frame's duration
word as ticks (as other GAF animations, **Believed**). The current snap
target gets a thin yellow ring; selection rings flash when the selection
changes (`select_flash_ms`).

### Rumble

Off by default (`rumble`). When on, a short pulse on snapping to a target,
on a selection change and when own units lose health, through Bevy's
`GamepadRumbleRequest` (gilrs force feedback; strengths and length in
`rumble_*`).

## Feel model

### Cursor

The stick goes through an inner radial deadzone and an outer deadzone
(`stick_deadzone`, `stick_outer_deadzone`), then a response curve
(`cursor_curve_pct`: exponent × 100; non-integer exponents blend the
neighbouring integer powers, which keeps it integer-only and monotonic).
Speed is `cursor_speed` at full deflection, scaled by the zoom
(`cursor_zoom_pct` 100 = the cursor covers the same ground per second at
any zoom, relative to `zoom_ref_milli`), by an optional acceleration ramp
after holding near-full deflection (`cursor_accel_*`), by friction, and by
the precision multiplier (`precision_pct`, RT): one code path, every
modifier a multiplier. Motion is integrated in 16 ms substeps from event
timestamps, so it does not depend on the frame rate (tested: 20 and 50 ms
frames end within a pixel).

### Aim assist

- **Friction**: near a target the cursor slows to `friction_pct` at its
  centre, rising linearly to full speed at `friction_radius_px`.
- **Soft snap**: while the stick is near idle (`snap_idle_pct`), the
  nearest target within `magnet_radius_px` pulls the cursor with a
  half-life of `snap_half_life_ms` (frame-rate independent exponential;
  `0` keeps WP-J's linear per-frame pull) and locks on within
  `magnet_snap_px`. **Hysteresis**: the current target is kept until
  another is `snap_hysteresis_px` nearer, so it never flickers between
  neighbours.
- **Target priority** (`assist_priority`): with own units selected only
  enemies are targets (orders aim at them); with nothing selected only own
  units (you are selecting); in build mode only own frames under
  construction (help-build), otherwise free placement; patrol has none.
- Assist is off in precision mode and while brushing (the brush centre
  should be where you aim).

### Camera

Right-stick pan with smoothing (`pan_smooth_ms`) at a speed that follows
the zoom (`pan_zoom_pct`). Edge pan when the stick cursor pushes into the
outer `edge_pan_px` of the screen, ramping up over `edge_pan_ramp_ms`
(mouse edge pan only with `edge_pan_mouse`). Zoom with the mouse wheel or
RB + right stick, smoothed (`zoom_smooth_ms`) and clamped
(`zoom_min_milli` .. `zoom_max_milli`). Recentre (monarch, idle builder)
and follow-selection glide with half-life `glide_ms`. With
`cursor_world_lock` the stick cursor keeps its ground point while the
right stick pans or the zoom changes, so it stays on its target.

### Wheels

Slot from stick angle (slot 0 at the top, clockwise), a deadzone
(`wheel_deadzone`), angle hysteresis (`wheel_hysteresis_deg`), flick and
release keeps the last slot unless `wheel_recentre_clears`. Confirm on
release (default) or, with `wheel_confirm_on_press`, the wheel stays open
after its button is released and A / X confirms, B or the wheel button
cancels. Slot count and labels are drawn around the cursor; the wheel's
centre says how many slots and how to confirm.

### Brush

Starts at `brush_min` after `hold_ms`, grows to `brush_max` at an average
`brush_growth_per_s` along a curve (`brush_curve_pct` below 100 grows
fast first and slows down near the end, which helps let go at the right
size). Moving the cursor while holding paints additional circles.

## All tuning values

Every value lives in `tak_input::Tuning`; `TUNING_FIELDS` carries the
step, range and help text the overlay shows. A profile's `"tuning"`
object may set any subset; missing values take the `console` default.
Regenerate this table with
`cargo run -p tak-input --example tuning_table`.

| Group | Value | Unit | console | assisted | precise | wp_j | What it does |
|---|---|---|---:|---:|---:|---:|---|
| Cursor | `cursor_speed` | px/s | 1100 | 900 | 1400 | 900 | Cursor speed at full stick |
| Cursor | `cursor_curve_pct` | % | 110 | 110 | 100 | 100 | Response curve exponent x100 (100 linear) |
| Cursor | `stick_deadzone` | raw | 7000 | 7000 | 7000 | 8000 | Inner stick deadzone |
| Cursor | `stick_outer_deadzone` | raw | 2000 | 2000 | 2000 | 0 | Outer deadzone: this near full counts as full |
| Cursor | `cursor_accel_pct` | % | 0 | 0 | 0 | 0 | Extra speed after holding full stick |
| Cursor | `cursor_accel_ms` | ms | 350 | 350 | 350 | 400 | Time to reach the full acceleration bonus |
| Cursor | `cursor_accel_threshold_pct` | % | 90 | 90 | 90 | 90 | Deflection that counts as full for acceleration |
| Cursor | `cursor_zoom_pct` | % | 100 | 100 | 100 | 0 | Cursor speed follows zoom (100: same ground speed) |
| Cursor | `precision_pct` | % | 35 | 35 | 30 | 25 | Precision (RT) speed multiplier |
| Aim assist | `assist_priority` |  | on | on | on | off | Context targets: enemies with units selected, own units otherwise |
| Aim assist | `friction_pct` | % | 30 | 20 | 45 | 100 | Cursor speed over a target's centre (100: off) |
| Aim assist | `friction_radius_px` | px | 26 | 34 | 22 | 24 | Friction reach around a target |
| Aim assist | `magnet_radius_px` | px | 44 | 56 | 24 | 40 | Soft snap radius |
| Aim assist | `magnet_snap_px` | px | 8 | 12 | 6 | 12 | Lock-on distance |
| Aim assist | `snap_half_life_ms` | ms | 45 | 30 | 60 | 0 | Snap pull half-life (0: linear WP-J pull) |
| Aim assist | `magnet_pull_per_s` | /s | 8 | 8 | 8 | 8 | Linear pull per second (when half-life is 0) |
| Aim assist | `snap_idle_pct` | % | 15 | 25 | 5 | 0 | Snap acts below this stick deflection |
| Aim assist | `snap_hysteresis_px` | px | 10 | 14 | 6 | 0 | A new target must be this much nearer to take over |
| Buttons | `hold_ms` | ms | 250 | 250 | 250 | 250 | Press longer than this is a hold |
| Buttons | `double_tap_ms` | ms | 300 | 300 | 300 | 300 | Second press within this is a double tap |
| Brush | `brush_min` | wu | 16 | 16 | 16 | 16 | Brush start radius (world units) |
| Brush | `brush_growth_per_s` | wu/s | 140 | 120 | 170 | 160 | Brush growth speed |
| Brush | `brush_max` | wu | 192 | 192 | 192 | 192 | Brush largest radius |
| Brush | `brush_curve_pct` | % | 80 | 80 | 80 | 100 | Brush growth curve x100 (above 100: slow start) |
| Brush | `drag_min_px` | px | 4 | 4 | 4 | 4 | Mouse drag shorter than this is a click |
| Wheels | `wheel_deadzone` | raw | 16000 | 16000 | 16000 | 16000 | Stick deflection to highlight a slot |
| Wheels | `wheel_max_slots` |  | 16 | 16 | 16 | 16 | Most slots on a wheel |
| Wheels | `wheel_hysteresis_deg` | deg | 8 | 8 | 8 | 0 | Stay on a slot until this far past its edge |
| Wheels | `wheel_confirm_on_press` |  | off | off | off | off | Wheel stays open; A confirms (off: release confirms) |
| Wheels | `wheel_context` |  | on | on | on | off | Wheel shows what the selection can do |
| Wheels | `wheel_recentre_clears` |  | off | off | off | off | Stick back to centre clears the highlight |
| Camera | `pan_speed` | wu/s | 1200 | 1200 | 1200 | 1200 | Right-stick pan speed |
| Camera | `pan_zoom_pct` | % | 100 | 100 | 100 | 0 | Pan speed follows zoom (100: same screen speed) |
| Camera | `pan_smooth_ms` | ms | 60 | 60 | 60 | 0 | Pan smoothing half-life (0: instant) |
| Camera | `edge_pan_px` | px | 24 | 24 | 24 | 0 | Edge pan margin for the stick cursor (0: off) |
| Camera | `edge_pan_speed` | wu/s | 1000 | 1000 | 1000 | 900 | Edge pan speed |
| Camera | `edge_pan_ramp_ms` | ms | 300 | 300 | 300 | 300 | Edge pan ramp-up time |
| Camera | `edge_pan_mouse` |  | off | off | off | off | Edge pan with the mouse too |
| Camera | `zoom_min_milli` | /1000 | 150 | 150 | 150 | 150 | Closest zoom (world units per pixel x1000) |
| Camera | `zoom_max_milli` | /1000 | 4000 | 4000 | 4000 | 4000 | Farthest zoom |
| Camera | `zoom_step_pct` | % | 10 | 10 | 10 | 10 | Zoom per mouse-wheel notch |
| Camera | `zoom_stick_pct_per_s` | %/s | 150 | 150 | 150 | 150 | RB + right stick zoom speed |
| Camera | `zoom_smooth_ms` | ms | 70 | 70 | 70 | 0 | Zoom smoothing half-life (0: instant) |
| Camera | `glide_ms` | ms | 90 | 90 | 90 | 0 | Recentre / follow glide half-life (0: jump) |
| Camera | `cursor_world_lock` |  | on | on | on | off | Panning and zooming keep the cursor on its ground point |
| Camera | `zoom_ref_milli` | /1000 | 500 | 500 | 500 | 500 | Reference zoom for the zoom-scaled speeds |
| Feedback | `select_flash_ms` | ms | 250 | 250 | 250 | 0 | Selection ring flash length |
| Feedback | `rumble` |  | off | off | off | off | Controller rumble on snap, selection and damage |
| Feedback | `rumble_snap_pct` | % | 20 | 20 | 20 | 20 | Rumble strength on snap |
| Feedback | `rumble_select_pct` | % | 30 | 30 | 30 | 30 | Rumble strength on selection |
| Feedback | `rumble_damage_pct` | % | 60 | 60 | 60 | 60 | Rumble strength on damage |
| Feedback | `rumble_ms` | ms | 80 | 80 | 80 | 80 | Rumble pulse length |

## Presets and the feel lab

| Preset | For | Character |
|---|---|---|
| `console` (default) | everyone first | Halo Wars-like: near-linear stick, strong friction, context snap, smooth camera |
| `assisted` | relaxed play, small screens, couch distance | slower cursor, strongest friction and snap |
| `precise` | players who find `console` sticky | faster linear cursor, light friction and snap |
| `wp_j` | comparison only | the WP-J values: linear, no friction, idle-only linear pull to any unit, instant camera |

`--input-preset NAME` picks one at start; the overlay's first row switches
at runtime.

### How the defaults were chosen

`tak_input::lab` is a headless harness with a simulated player (all
integer, seeded, the same trials for every preset). The player reacts
after 180–280 ms, sees the cursor 100 ms late, wants to close the gap in
about 170 ms, plans sub-movements every 90–150 ms with ±18 % gain and ±7°
direction errors, has ±500 raw units of thumb tremor, and knows the stick
curve and base speed (a practised player) but not friction, snap or
acceleration. Half the aim trials are played carefully (let go, confirm,
press) and half "on the fly" (press as soon as it looks close, 40–90 ms
motor latency with the stick still held). Worlds are dense (70 units at
least 36 px apart, mixed owners) or sparse (14 units, 160 px apart) at the
reference zoom.

- **Acquire**: select a given own unit (nothing selected) or attack a
  given enemy (own unit selected), from 150–600 px away. Time from the
  target appearing to the press that issued the right command.
- **Overshoot**: how far past the target the cursor went along the
  approach.
- **Wrong target**: another unit selected or attacked/guarded. **Miss**:
  the press hit nothing (no selection, or a move order). **Errors** =
  both.
- **Brush**: aim at the centre of a cluster of 5–10 own units with other
  own units around it, hold A and let go 150–260 ms after the brush covers
  the cluster. F1 of the selection against the cluster; time from start to
  release.

Results, 300 trials per task and density (`cargo run --release -p tak-input
--example feel_lab -- --trials 300`):

| Preset | Acquire ms | Acquire ms (dense) | Overshoot px | Wrong target | Miss | Errors | Brush F1 | Brush ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| wp_j | 1071 | 1083 | 24.0 | 0.5% | 16.1% | 16.6% | 67.7% | 1698 |
| console | 910 | 921 | 8.4 | 0.0% | 0.0% | 0.0% | 68.4% | 1590 |
| assisted | 983 | 1006 | 0.8 | 0.0% | 0.0% | 0.0% | 63.4% | 2119 |
| precise | 977 | 982 | 24.5 | 0.3% | 16.5% | 16.8% | 66.9% | 1483 |

`console` beats the WP-J defaults on acquisition time (−15 %, −15 % on
dense worlds), wrong targets (0.5 % → 0.0 %) and total errors (16.6 % →
0.0 %), with no brush regression (F1 67.7 % → 68.4 %, and 6 % faster).
`tests/feel.rs` (`feel_lab_console_beats_wp_j`) keeps checking this on a
smaller run.

Robustness, 150 trials each:

- Slower player (perceives 150 ms late, wants 220 ms closing time):
  WP-J 1,416 ms / 35.3 % errors; console 1,079 ms / 1.9 %; assisted
  1,058 ms / 0.0 %; precise 1,298 ms / 28.5 %. Brush F1: WP-J 66.0 %,
  console 65.0 % (a point lower here).
- Faster player (70 ms, 140 ms, 80 % on the fly): WP-J 945 ms / 27.4 %;
  console 852 ms / 0.1 %; assisted 934 ms / 0.0 %; precise 885 ms /
  31.6 %.

What the sweeps showed (around `console`, 100 trials):

- **Friction is the biggest win**: 100 % (off) → 30 % cut errors from
  25.7 % to 12.9 % and time by 90 ms on its own; below 30 % it starts to
  slow brushing.
- **Curves and acceleration hurt in this model**: exponent 2.0 vs 1.0 cost
  +90 ms and doubled errors, acceleration +40 % cost +35 ms and doubled
  the misses. The simulated player's motor noise is proportional to thumb
  deflection, so a curve multiplies the speed error. Real thumbs may like
  a curve for slow fine motion, which is why `console` keeps a slight one
  (1.1) and the overlay makes it a one-press experiment. **Believed**, to
  be checked with Tyler's sessions.
- Snap radius 30 → 44 px cut misses by a quarter; hysteresis makes no
  difference to these numbers (it prevents flicker, which the lab does not
  score); `snap_idle_pct` 15 is a little faster than 0, 40 hurts the brush.
- Cursor speed: 1,100 px/s is the fastest without new errors (1,300 is
  40 ms faster with 0.7 % errors).
- Brush: slower growth is more accurate (120: 70.9 %, 160: 67.5 %); 140
  with a fast-start curve (80) keeps WP-J's accuracy and is about 100 ms
  faster.

`precise` is fast but error-prone for this simulated player; it is there
for players whose aim beats the model. The lab measures proxies, not
feel: Tyler's recordings are the real test.

Run your own: `--sweep name=a,b,c` around a preset, `--profile FILE` for
a saved profile, `--perceive-ms`, `--tau-ms`, `--fly-permille` for a
different player.

## Live tuning

- `--input-profile FILE`: the profile (bindings and tuning) is read from
  it, created from the preset if missing, polled twice a second and
  re-applied on every save (a broken file is reported and ignored).
- The overlay (Menu / Options / F11) lists the preset and every value.
  Pad: D-pad up/down to choose, left/right to adjust, LB/RB ×10, A to
  toggle or advance, Y to save, B or Menu to close. Keyboard: arrows
  (Shift ×10), PgUp/PgDn, Enter to save, F11/Esc to close. Mouse: the − / +
  buttons. While it is open the pad and keyboard do not reach the game.
- Saving writes the full profile to the `--input-profile` file, or
  `tak-input-profile.json` in the working directory without one.

## Recording a session and sending it back

`--record-session FILE` writes, on exit:

- every raw device event fed to the input layer, with the client clock
  (ms) and the sim tick;
- the cursor (screen and ground), whether the stick or mouse drives it,
  the camera, the selection when it changes, the open wheel, the brush,
  the snap target;
- every output (client and sim commands), the sim orders as submitted
  with the tick they run on, feedback events, and tuning changes;
- the profile at the start, and the match: map, players, seed, the full
  command log, tick count and final hash.

Only frames in which something changed are stored (about 50 KB for a
15-second scripted session). Send the file back as is.

### Watching one (ghost playback)

```sh
cargo run --release -- --ghost ~/tak-session-1.json [--ghost-camera]
cargo run --release -- --ghost s.json --screenshot g_{tick}.png --at-tick 300
```

The recorded match is replayed from its own command log (map, players and
seed come from the file), with the player's cursor as a magenta
crosshair plus a tinted game cursor, their brush, their wheel, and their
selection as magenta rings. `--ghost-camera` follows the recorded camera.
The sim is deterministic, so this is the match exactly as it was played
(`tests/recording.rs` replays a recorded session through `tak-headless`
and compares every tick's hash).

`Recording::summary()` gives a quick look (orders by type, selections,
snaps, stick time and speed); the exit message prints it.

### Scripted sessions

`--synth-events EVENTS.json --record-session OUT.json [--synth-ticks N]`
drives the match headless with scripted `tak-input` events through the
real mapper (no window) and writes a recording, e.g.
`client/fixtures/pad_session.json`. In screenshot mode, `--pad-events
EVENTS.json` feeds events to the mapper before each capture so wheels,
cursors and hints can be checked headless.

## Testing checklist (Tyler)

1. Pad detected: hint bar shows your pad's glyphs.
2. Cursor: small corrections near a unit are easy; full stick crosses the
   screen in about a second; the cursor is equally fast in ground terms at
   any zoom; RT slows it and disables the pull.
3. Select a unit with A; tap A again quickly: all of that type on screen.
   Y: everything on screen; Y Y: everything.
4. With the monarch selected, cursor over an enemy: the swords cursor,
   the cursor slows and settles on it; X attacks. Over a friend: shield
   cursor, X guards. Over ground: move cursor.
5. Hold A and paint a group; let go when the circle covers it.
6. LT wheel: flick to a slot and release; does the highlight ever jump
   between neighbouring slots? Try `wheel_confirm_on_press` in the
   overlay.
7. LB build wheel, pick a building, the cursor turns green/red, X places.
8. Push the cursor into a screen edge: the camera starts slowly and
   speeds up. Right stick pan is smooth; the cursor stays on the unit it
   was on. RB + right stick zooms smoothly. R3 glides to the monarch; hold
   R3 to follow the selection.
9. Try `assisted` and `precise` in the overlay; note which feels best and
   any value you changed.
10. Optional: turn `rumble` on.
11. Quit, send back the session recording and the profile file.

## Tests

- `crates/tak-input/tests/feel.rs`: presets, field metadata, curve and
  approach properties, zoom/precision/acceleration, frame-rate
  independence, friction, snap hysteresis, context targets, second tap,
  local/global select, context wheel, wheel hysteresis, sticky wheel,
  smoothed zoom, edge pan, cursor world lock, glide, follow, cursor kinds,
  recording round trip and ghosts, the WP-J cursor kept under `wp_j`, and
  the feel-lab verdict.
- The WP-J tests (`pieces.rs`, `gamepad_fixture.rs`, `equivalence.rs`)
  now pin `Profile::wp_j_controller()` and `Tuning::wp_j()`: they keep
  checking the gesture recogniser and the original mapping exactly, while
  the new default's behaviour is covered by `feel.rs`.
- `client/tak-client/tests/recording.rs` (data-gated): scripted pad
  session → recording → identical headless replay; `replay.rs` unchanged
  and passing.
