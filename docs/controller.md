# Controller and keyboard play (WP-S, WP-V)

The headline goal of Oxidized Annihilation is that a game controller is a
first-class way to play TA:K. This page is the reference for how the
controller and the keyboard work, every feel value and what it does, how
the defaults were chosen, and how to record a play session and send it
back. The design behind it is [`ux/design-system.md`](ux/design-system.md)
(TA:K decides what the game does and what things are called; Halo Wars
decides how a controller does it).

Where things live: all feel logic, bindings and every tuning value are in
`tak-input` (core, integer-only, deterministic, tested headless). The
client (`client/`) adapts devices, draws the cursor, wheels, HUD and
prompts, plays the order sounds and hosts the tuning overlay. Tags as
elsewhere: **Known**, **Believed**, **Chosen**, **Unknown**.

## What changed in WP-V (Tyler)

- **New default pad scheme, "Kingdoms Console"** (design system section 4):
  the cursor is a **reticle fixed at the screen centre** and the left stick
  moves the camera under it (Halo Wars; Microsoft's patent US 8,882,594).
  Aim assist now slows and nudges the camera instead of the cursor. A is
  select, X is the context order, Y casts the first spell (hold: all
  spells), B cancels (hold: Stop), RB is "all on screen" (double: all;
  hold: queue / add), LB is the next idle builder (hold: build wheel), LT
  is the orders wheel, RT cycles the unit types in the selection (hold:
  fast pan), the D-pad is squads 1-4, View is the last alert (hold: the
  Crystal Ball), L3 the monarch, R3 resets the zoom (hold: track). The
  right stick zooms.
- **New default keyboard, "Classic TA:K"**: TA:K's default keys with the
  existing left-select / right-order mouse scheme (section 5). Your
  install's `Keys.TDF` is imported at start, so customised keys carry over.
  The optional `--controls original` scheme adds original-style contextual
  left clicks without changing the default or the controller bindings.
- **Wheels keep their shape**: Stop is always at 12 o'clock; Stance and Unit
  open sub-rings; build and spell wheels have pages of 8 (RB turns the
  page) with the game's build portraits.
- **HUD in the game's own art**: your race's button panel and info panel
  with TA:K's order buttons, build portraits, a unit card, a strip for
  mixed selections, and a bottom bar that says what X (or right click)
  does here. The button panel sits at the right edge as in the original
  TA:K; on the pad it folds away.
- **Order sounds, alerts and the Crystal Ball**: the original
  MOVE/ATTACK/GUARD/PATROL/STOP acknowledgements; alerts (attacked,
  construction complete, unit trained, mana full, monarch in danger) ping
  the minimap and View / F3 jumps to them, newest first.
- The previous schemes are still there for comparison: `--controls wp_s`
  (WP-S: free cursor, Y local/global, RB zoom modifier) and
  `--controls wp_j`.

## Try this first (Tyler)

```sh
cd client
export TAK_DATA_DIR="$HOME/Games/Total Annihilation Kingdoms"
cargo run --release -- --demo --input-profile ~/tak-input.json \
    --record-session ~/tak-session-2.json
```

1. Plug in the pad before starting. Push the left stick: the map slides
   under the reticle. Put the reticle on Elsin, tap **A**; push toward an
   enemy, tap **X**.
2. Hold **LB**: the build wheel. Flick toward Wall and let go of LB; the
   footprint is green or red with the reason in the bottom bar; **X**
   places it.
3. Hold **LT**, flick to the lower right (Stance) and let go: the Stance
   ring stays open; pick Passive and press **A**. **B** backs out.
4. Select two kinds of units (hold **A** to brush), tap **RT** to step
   through the types (the strip lights the active one), once more for all.
5. Hold **View** for the Crystal Ball; tap it after an alert to jump there.
6. Try the keyboard: `A` then click to attack, `Ctrl+M` for the monarch,
   `Ctrl+1` / `1` squads, `Ctrl+F5` / `F5` bookmarks, `Tab`, `~`.
7. Quit normally. Send back `~/tak-session-2.json` and `~/tak-input.json`
   with a line or two on what felt wrong. If the reticle is not for you,
   switch `reticle` off in the overlay (Menu / F11) and compare.

## Kingdoms Console (pad)

`Profile::kingdoms_console()`, `crates/tak-input/src/profiles/kingdoms_console.json`.
Merged with Classic TA:K into one mapper (`Profile::default_combined`), so
the pad, mouse and keyboard drive one selection.

| Xbox | PlayStation | Tap | Double tap | Hold |
|---|---|---|---|---|
| Left stick | Left stick | Reticle: move the camera (free mode: move the cursor) | | |
| Right stick | Right stick | Up/down: zoom (free mode: pan; zoom with RB held) | | |
| L3 | L3 | Camera to the monarch | Select the monarch | Select and track the monarch |
| R3 | R3 | Reset zoom | | Track the selection (toggle) |
| A | Cross | Select under the reticle (adds with RB held) | Same type on screen | Brush select (grows while held) |
| X | Square | Context order (queued with RB held) | | Attack-move: **reserved**, sim lacks it |
| Y | Triangle | Cast the first spell, then X on the target | | Spell wheel (every spell) |
| B | Circle | Cancel wheel / mode, then deselect | | Stop |
| RB | R1 | All own units on screen | All own units on the map | Queue / add modifier |
| LB | L1 | Next idle builder | | Build wheel (selected builder or factory) |
| LT | L2 | | | Orders wheel (nothing selected: squads 5-9, monarch, idle builder, all) |
| RT | R2 | Sub-select cycle: next unit type in the selection, then all | | Fast pan (free mode: precision cursor) |
| D-pad up / right / down / left | same | Recall squad 1 / 2 / 3 / 4 | Camera to that squad | Assign squad |
| View | Share | Jump to the last alert (cycles, newest first) | | Crystal Ball (full-screen minimap) |
| Menu | Options | Game menu (the tuning overlay) | | |

Gesture details (`tak_input::Gesture`): a *tap* is shorter than `hold_ms`
(250 ms); a *second tap* within `double_tap_ms` (300 ms) does not delay the
first tap (tap A selects at once, the second tap widens it). Pressing
another button while RB is down uses RB as the modifier, so its own tap
(all on screen) does not fire; a long RB press is never a tap either.
Inside a paged wheel RB turns the page. Squads 5-9 are on the idle orders
wheel (nothing selected) and in the Unit > Squads sub-ring (confirm
recalls, confirm with RB held assigns).

### Wheels

```
Orders wheel (LT)              Build / spell wheel (LB / hold Y)
          Stop                 the builder's menu in TA:K order,
   Clear       Attack          8 per page, RB for the next page;
Patrol           Move          centre: portrait, name, cost and
   Guard    Stance >           build time, greyed with the mana
       Unit >                  shortfall in red when short
Stance >: Offensive / Defensive / Passive
Unit >:   Squads > (5-9), Heal, Load, Unload, Cloak, Activate
          (all reserved until the sim has them)
```

- Slots never move (`Profile::fixed_slots`): a slot the selection cannot
  use stays where it is, empty (a reserved order is a faint frame, a page's
  unused positions are not drawn).
- Flick and release confirms; releasing without having pointed anywhere
  (the dead centre) cancels. With `wheel_confirm_on_press` the wheel stays
  open and A / X confirms.
- A ring slot (Stance >, Unit >) opens its sub-ring when confirmed. The
  sub-ring stays open after LT is released: the stick chooses, A / X
  confirms, B goes back one ring, LT closes. The title says where you are
  ("Orders > Stance").
- Build options and spells come from the selection's first builder and
  caster (lowest unit id). A spell the caster cannot pay for is greyed and
  refused with "Not enough mana"; a build short of mana is allowed (TA:K
  drains mana while building) but greyed with the shortfall.

### Glyphs

The prompts use Xbox (A B X Y, LB RB LT RT, View, Menu) or PlayStation
(Cross Circle Square Triangle, L1 R1 L2 R2, Share, Options) glyphs, chosen
from the connected pad: Sony's USB vendor id (`0x054c`) or a name
containing DualSense / DualShock / PlayStation / PS4 / PS5 (or exactly
"Wireless Controller") gives PlayStation, anything else Xbox (`--glyphs
auto`, the default). The chips are drawn in code (no art) and read from the
profile's bindings, so they stay right after rebinding.

## Classic TA:K (keyboard and mouse)

`Profile::classic_tak()`, `crates/tak-input/src/profiles/classic_tak.json`:
TA:K's keyboard defaults from the install's readme and `Keys.TDF`, with the
existing right-context mouse scheme retained for compatibility.

| Keys | Action |
|---|---|
| Left click / drag | Select / select everything in the dragged circle (adds with Shift) |
| Double click | Same type on screen |
| Right click | Context order (Shift queues) |
| A, M, G, P then left click | Attack (unit or ground), Move (even over a unit), Guard, Patrol |
| S | Stop |
| C, H, K, L, U, O | Clear, Heal, Cloak, Load, Unload, Open: **reserved** (sim lacks them) |
| Ctrl+A / Ctrl+U | All units / all on screen |
| Ctrl+B / E / F / G / N / R / T / W / Y | Builders / melee / factories / magic / naval / ballistic / troops / armed / fliers (Ctrl+Shift adds) |
| Ctrl+Z | All of the selected types on the map |
| Ctrl+M | Select and track the monarch |
| Ctrl+D | Dismiss: **reserved** |
| Ctrl+0-9 / 0-9 | Create / retrieve squad (0 is squad 10) |
| Alt+0-9 | Formation: **reserved** |
| Ctrl+F5-F8 / F5-F8 | Set / go to bookmark |
| N / T / F3 | Next unit off screen / track the selection / last alert |
| Tab | Crystal Ball (full-screen minimap) |
| Ctrl+F1-F3 | Spells 1-3 of the selection's caster (TA's "select weapon"; **Chosen**) |
| `+` (`=`) / `-` / Pause | Game speed / pause (client only: ticks per wall second) |
| `~` | Damage bars: all, selected, off |
| F1 / F11 | Game menu / tuning overlay |
| Esc | Cancel mode, then deselect |
| Arrow keys, mouse wheel | Pan, zoom (client) |

### Original-style left-click orders (optional)

`--controls original` combines the unchanged Kingdoms Console pad with
`Profile::original_tak()`. A saved profile can opt in with
`"left_click_context": true`; older profiles omit it and keep their existing
behavior. This option changes `drag_select` clicks, not the controller's
`select` action or its bindings.

- Click an owned unit to select it, including when another unit is selected.
  Shift-click adds to the selection; double-click the same owned unit selects
  its type on screen.
- With owned units selected, click ground to move or an enemy to attack.
  Shift-click queues orders. Rapid repeated command clicks issue repeated
  orders rather than accidentally triggering same-type selection.
- Left drag still selects the dragged circle and never issues a move or
  attack. Orders are resolved on release, after distinguishing click from
  drag. Escape cancels a gesture before it can issue a late order.
- Explicit order modes take priority, including over friendly units. Right
  click remains the ordinary context order, so guarding an owned unit is
  still available; clicking another player's non-enemy unit also guards it.
- Empty selection cannot issue orders. Escape still clears selection when
  no gesture or order mode is active.

**Known** (reference-play observation, October 2026): the original game
supports left-click contextual selection, movement and attacks. **Chosen**:
keep this an explicit named option, retain right-click orders and the
existing circular drag region, and reuse the current guard / queue / order
mode rules. This is not a claim of complete one-to-one mouse fidelity.

### Keyboard add-ons and imports

- **No build hotkeys by default**, as in TA:K. `--grid-keys` (or
  `"grid_build_keys": true` in a profile) turns on the "Grid" add-on: with
  a builder selected, Q W E R / A S D F / Z X C V pick build options 1-12
  ahead of their normal meaning; with anything else selected they keep it.
- **`Keys.TDF` import** (`tak_input::parse_keys_tdf`, `Profile::with_keys_tdf`):
  at start the client reads `$TAK_DATA_DIR/Keys.TDF` (or `--keys-tdf FILE`;
  `--no-keys-tdf` skips it; a saved `--input-profile` wins) and replaces
  every key the file binds or unbinds. On the retail file (344 lines):
  **68 mapped**, **23 reserved** (Clear, Heal, Cloak, Load, Unload, Open,
  formations, self-destruct: bound but inert), 54 explicitly empty, 16
  unsupported (diplomacy, briefing, scorecard, save/load, chat, the debug
  and clipboard keys: no such screens yet; the profile keeps its own
  binding there, e.g. F11), 173 skipped (Ctrl+Shift and Alt+Shift lines:
  our chords take one modifier and TA's Ctrl+Shift lines are the additive
  versions of the Ctrl lines, which Shift already does; the `XCHAR_` keys
  of other layouts; symbols listed "for French / German keyboards").
  **Known**: importing the retail file reproduces Classic TA:K exactly
  (`tests/keys_tdf.rs`, data-gated; counts only).

## Reserved actions (design system section 9)

The interface offers only what the sim does. These keep their key, wheel
slot or panel well but do nothing and are hidden (empty well, faint wheel
frame, no prompt); pressing their key says "Not available yet" in the
bottom bar: Clear, Heal / repair, Load / Unload / Pickup / Drop, Cloak,
Open / Activate, attack-move, formations, dismiss, persistent weapon
select, veterancy and kill count (not on the unit card). Adding one to the
sim is a change of `Action::Reserved { .. }` to the real action; no layout
moves.

## Feedback and information

| Event | What you see / hear |
|---|---|
| Selection change | Ring flash, unit card and strip update, rumble tick (opt-in) |
| Order | Order line, the original acknowledgement (`Sounds/MOVE`, `ATTACK`, `GUARD`, `PATROL`, `STOP.WAV`; build sounds like a move, a spell like an attack, a stance has none: **Chosen**) |
| Refusal | Bottom-bar reason in red (no mana, nothing selected, not available yet, nothing there) |
| Placement | Footprint green, or red with the sim's reason ("must be built on a free sacred site") |
| Alert | Minimap ping (an expanding ring for 3 s), bottom-bar line; View / F3 jumps newest first |

Alerts (`client/tak-client/src/alerts.rs`, display only, checked after
every tick): an own unit or structure losing health (once per unit per
10 s; frames gaining health do not count), construction complete, unit
trained, mana storage full, monarch below a third of its health. No
dedicated alert sound was found in the data (**Unknown**), so alerts are
silent.

Audio is Bevy's (WAV only, `bevy_audio` + `wav`); with no audio device
nothing plays and nothing fails (headless runs, CI).

### Cursors

The game's own cursors from `anims/CURSORS.GAF` (palette
`anims/CURSORS.PCX`, **Known**: decodes cleanly, 23 sequences) replace
the OS cursor, by context (`Mapper::cursor_kind`; mapping **Chosen** from
the sequence names):

| Context | Sequence |
|---|---|
| nothing to do | `cursornormal` |
| over an own unit that a click selects | `cursorselect` |
| order would move (or Move mode) | `CursorMove` |
| order would attack (or Attack mode) | `CursorAttack` |
| order would guard | `CursorDefend` |
| patrol mode | `CursorPatrol` |
| build mode, placeable / blocked (the sim's site check) | `cursorgrn` / `cursorred` |
| spell mode | `Cursorresurrect` |
| pending order not possible here | `cursortoofar` |

In reticle mode the cursor sprite sits on the reticle at the screen
centre. The mapper's camera maps the ground plane at the height of the
terrain under the screen centre (`ViewGeom::input_camera_at`), so the
reticle picks exactly what is drawn under it on hills too.

### Rumble

Off by default (`rumble`). When on, a short pulse on snapping to a target,
on a selection change, on a refusal and when own units lose health,
through Bevy's `GamepadRumbleRequest` (strengths and length in
`rumble_*`).

## Feel model

### Reticle mode (default) and free cursor

With `reticle` on and the stick in use, the cursor stays at the screen
centre and the left stick moves the camera at exactly the speed law of the
free cursor (curve, zoom scaling, acceleration, friction), turned into
ground units; precision (hold RT) becomes fast pan (`fast_pan_pct`). Aim
assist acts on the camera: friction slows it near a target and the soft
snap slides the target under the reticle. `reticle_smooth_ms` adds camera
inertia (0 by default: the feel lab says it costs time and accuracy). The
mouse always moves a free cursor; moving the stick again returns to the
reticle. With `reticle` off the left stick moves a free cursor with edge
pan, as in WP-S. The right stick zooms (`right_stick_zoom`); in free mode
it pans and zooms with RB held.

### Cursor speed

The stick goes through an inner radial deadzone and an outer deadzone
(`stick_deadzone`, `stick_outer_deadzone`), then a response curve
(`cursor_curve_pct`: exponent × 100; non-integer exponents blend the
neighbouring integer powers, which keeps it integer-only and monotonic).
Speed is `cursor_speed` at full deflection, scaled by the zoom
(`cursor_zoom_pct` 100 = the same ground per second at any zoom, relative
to `zoom_ref_milli`), by an optional acceleration ramp after holding
near-full deflection (`cursor_accel_*`), by friction, and by the precision
multiplier: one code path, every modifier a multiplier. Motion is
integrated in 16 ms substeps from event timestamps, so it does not depend
on the frame rate.

### Aim assist

- **Friction**: near a target the cursor (or camera) slows to
  `friction_pct` at its centre, rising linearly to full speed at
  `friction_radius_px`.
- **Soft snap**: while the stick is near idle (`snap_idle_pct`), the
  nearest target within `magnet_radius_px` pulls with a half-life of
  `snap_half_life_ms` and locks on within `magnet_snap_px`.
  **Hysteresis**: the current target is kept until another is
  `snap_hysteresis_px` nearer.
- **Target priority** (`assist_priority`): with own units selected only
  enemies are targets; with nothing selected only own units; in build mode
  only own frames under construction; Attack mode any unit; Move and
  Patrol modes none.
- Assist is off in precision / fast-pan mode and while brushing.
- **Fog of war**: the input layer sees only what the local player sees.
  The client's `WorldView` (`SimView` with the fog grid) leaves out enemy
  units outside current sight, so they are never picked, snapped to,
  brushed, counted by same-type select or attacked by a context order (an
  order onto the ground where one stands is a move). Own and allied units
  are always visible. With the fog off (F9, `--no-fog`) everything is
  (`client/tak-client/tests/fog_view.rs`).

### Camera

Pan smoothing (`pan_smooth_ms`), zoom smoothing (`zoom_smooth_ms`) and
limits (`zoom_min_milli` .. `zoom_max_milli`); R3 returns to
`zoom_default_milli`. Recentre, alert and bookmark jumps glide with
half-life `glide_ms`; an explicit jump (squad, bookmark, next unit) stops
tracking. In free mode the stick cursor pushing the screen edge pans
(`edge_pan_*`) and `cursor_world_lock` keeps it on its ground point while
the camera moves.

### Brush

Starts at `brush_min` after `hold_ms`, grows to `brush_max` at an average
`brush_growth_per_s` along a curve (`brush_curve_pct`). Moving while
holding (in reticle mode: panning) paints additional circles.

## All tuning values

Every value lives in `tak_input::Tuning`; `TUNING_FIELDS` carries the
step, range and help text the overlay shows. A profile's `"tuning"`
object may set any subset; missing values take the `console` default.
The presets set the feel values; the control scheme keeps its own cursor
mode (`reticle`) and right-stick role. Regenerate this table with
`cargo run -p tak-input --example tuning_table`.

| Group | Value | Unit | console | assisted | precise | wp_j | What it does |
|---|---|---|---:|---:|---:|---:|---|
| Cursor | `reticle` |  | on | on | on | off | Reticle mode: cursor fixed at screen centre, stick moves the camera |
| Cursor | `reticle_smooth_ms` | ms | 0 | 0 | 0 | 0 | Reticle camera smoothing half-life (0: instant) |
| Cursor | `fast_pan_pct` | % | 200 | 200 | 200 | 200 | Reticle mode: precision button pans this much faster |
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
| Camera | `right_stick_zoom` |  | on | on | on | off | Right stick up/down zooms (free mode: pans; zooms with RB held) |
| Camera | `zoom_default_milli` | /1000 | 500 | 500 | 500 | 500 | Zoom that reset zoom (R3) returns to |
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
| `assisted` | relaxed play, small screens, couch distance | slower, strongest friction and snap |
| `precise` | players who find `console` sticky | faster linear stick, light friction and snap |
| `wp_j` | comparison only | the WP-J values: linear, no friction, idle-only linear pull, instant camera, free cursor |

`--input-preset NAME` picks one at start; the overlay's first row switches
at runtime. `--controls kingdoms|wp_s|wp_j` picks the scheme.

### How the defaults were chosen

`tak_input::lab` is a headless harness with a simulated player (all
integer, seeded, the same trials for every preset and mode). The player
reacts after 180-280 ms, sees the cursor 100 ms late, wants to close the
gap in about 170 ms, plans sub-movements every 90-150 ms with ±18 % gain
and ±7° direction errors, has ±500 raw units of thumb tremor, and knows
the stick curve and base speed but not friction, snap or acceleration.
Half the aim trials are careful (let go, confirm, press), half on the fly.
In reticle mode the player aims the ground point under the reticle the
same way (the camera moves instead of the cursor).

- **Acquire**: select a given own unit or attack a given enemy, 150-600 px
  away (on screen). Time to the press that issued the right command.
- **Overshoot**, **Wrong target**, **Miss**, **Errors** as in WP-S.
- **Brush**: F1 of a brushed cluster of 5-10 units among others.
- **Far** (new): attack an enemy 700-1,400 px away, off screen, with 60
  other units spread over a large area. The free cursor has to reach the
  edge and edge-pan; the reticle just pushes.

Results, 300 trials per task and density (`cargo run --release -p tak-input
--example feel_lab -- --trials 300`):

| Preset | Acquire ms | Acquire ms (dense) | Overshoot px | Wrong target | Miss | Errors | Brush F1 | Brush ms | Far ms | Far errors |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| wp_j | 1071 | 1083 | 24.0 | 0.5% | 16.1% | 16.6% | 67.7% | 1698 | 1694 | 99.0% |
| console | 910 | 921 | 8.4 | 0.0% | 0.0% | 0.0% | 68.4% | 1590 | 1957 | 10.6% |
| console reticle | 910 | 921 | 8.4 | 0.0% | 0.0% | 0.0% | 68.4% | 1590 | 1581 | 0.3% |
| assisted | 983 | 1006 | 0.8 | 0.0% | 0.0% | 0.0% | 63.4% | 2119 | 2115 | 9.3% |
| assisted reticle | 983 | 1006 | 0.8 | 0.0% | 0.0% | 0.0% | 63.4% | 2119 | 1793 | 0.0% |
| precise | 977 | 982 | 24.5 | 0.3% | 16.5% | 16.8% | 66.9% | 1483 | 1841 | 17.6% |
| precise reticle | 976 | 981 | 24.5 | 0.3% | 16.5% | 16.8% | 66.9% | 1483 | 1510 | 16.3% |

**Reticle vs free cursor.** On screen the two are the same to the
millisecond: the reticle moves the camera with the cursor's speed law, and
the assist acts on the same relative motion. Off screen the reticle wins
on every preset: `console` 1,581 ms and 0.3 % errors against 1,957 ms and
10.6 % (no edge-pan ramp, no cursor parked at the edge while the camera
catches up). Robustness, 150 trials: a slower player (150 ms perception,
220 ms closing) far task 1,754 ms / 0 % reticle vs 2,221 ms / 11.3 % free;
a faster player (70 ms, 140 ms, 80 % on the fly) 1,520 ms / 0 % vs
1,870 ms / 20.6 %.

**Default: reticle mode with the WP-S `console` assist values.** Sweeps
under the reticle (100 trials): friction 20 / 30 / 45 % and snap radius
30 / 44 / 56 px change nothing beyond noise, so WP-S's values stay; camera
smoothing hurts (`reticle_smooth_ms` 30: +200 ms and 11.7 % errors; 60:
+500 ms, 30.7 %), so it is 0. The simulated player has no model of the
camera's own momentum, which real players may like a little of: one
overlay row to try (**Believed**, to be checked with Tyler's sessions).
`tests/feel.rs` keeps checking both verdicts on smaller runs
(`feel_lab_console_beats_wp_j`, `feel_lab_reticle_matches_near_aim_and_wins_far`).

Earlier WP-S findings still hold (curves and acceleration hurt this
player; friction is the biggest single win; 1,100 px/s is the fastest
cursor without new errors). Run your own: `--sweep name=a,b,c` around
`--base PRESET` (on the Kingdoms Console profile), `--profile FILE`,
`--perceive-ms`, `--tau-ms`, `--fly-permille`.

## Live tuning

- `--input-profile FILE`: the profile (bindings and tuning) is read from
  it, created from the scheme and preset if missing (with your `Keys.TDF`
  applied), polled twice a second and re-applied on every save.
- The overlay (Menu / Options / F1 / F11) lists the preset and every value.
  Pad: D-pad up/down to choose, left/right to adjust, LB/RB ×10, A to
  toggle or advance, Y to save, B or Menu to close. Keyboard: arrows
  (Shift ×10), PgUp/PgDn, Enter to save, F11/Esc to close. Mouse: the − /
  + buttons. While it is open the pad and keyboard do not reach the game.

## Recording a session and sending it back

`--record-session FILE` writes, on exit: every raw device event with the
client clock and the sim tick; the cursor (screen and ground), the camera,
the selection, the open wheel, the brush, the snap target; every output
(client and sim commands), the orders as submitted with the tick they run
on, feedback events and tuning changes; the profile at the start; and the
match (map, players, seed, full command log, final hash). HUD buttons
carry out their action directly (`Mapper::perform`), so their orders are in
the outputs and the log rather than as device events.

Map-surface events store their exact order targets separately from the visible
cursor position, so a controller cursor, its wheel, and a recorded ghost do
not jump to the distant world point represented by a minimap pixel. The
additive `stick_map_pointer` event preserves controller ownership; older
recordings still load, but recordings containing this event need a build
that understands it.

### Watching one (ghost playback)

```sh
cargo run --release -- --ghost ~/tak-session-2.json [--ghost-camera]
```

The recorded match is replayed from its own command log, with the player's
cursor as a magenta crosshair, their brush, wheel and selection. The sim
is deterministic, so this is the match exactly as played
(`tests/recording.rs`).

### Scripted sessions

`--synth-events EVENTS.json --record-session OUT.json [--synth-ticks N]`
drives the match headless with scripted `tak-input` events through the real
mapper and writes a recording (`client/fixtures/pad_session.json`, now a
Kingdoms Console script). In screenshot mode `--pad-events EVENTS.json`
feeds events to the mapper before each capture (then 1.5 s of frames so
glides settle).

## Testing checklist (Tyler)

1. Pad detected: the prompts show your pad's glyphs.
2. Reticle: the map slides under it; small corrections near a unit are
   easy; full stick crosses the screen in about a second; RT pans faster.
3. A selects, A A same type, hold A brushes, RB all on screen, RB RB all.
4. Units selected, reticle on an enemy: the swords cursor, X attacks; the
   bottom bar says "X: Attack <name>". RB + X queues.
5. LT: Stop at 12; Stance > opens the ring; B goes back. Does a flick ever
   land on the neighbouring slot?
6. Hold LB: portraits, cost and build time; RB turns the page on long
   menus; the footprint turns red with the reason off a valid site.
7. RT with a mixed selection: the strip lights each type in turn.
8. D-pad: hold to assign, tap to recall, double tap to jump.
9. View after a fight starts: the ping, the jump; hold View: Crystal Ball.
10. Keyboard: your own `Keys.TDF` keys work; `~` cycles damage bars.
11. Try `reticle` off in the overlay, and `assisted` / `precise`; note
    which feels best. Quit, send back the session and the profile.

## Tests

- `crates/tak-input/tests/kingdoms_console.rs`: a recorded Kingdoms Console
  session (`tests/fixtures/kingdoms_console_session.json`) exercising every
  section-4 binding produces the expected commands and state, segment by
  segment; build-wheel pages; old schemes stay selectable.
- `tests/classic_tak.rs`: the same for Classic TA:K
  (`tests/fixtures/classic_tak_session.json`) and the Grid add-on.
- `tests/original_tak.rs`: opt-in left-click select / move / attack,
  repeat-click and same-unit double-click disambiguation, additive and queued
  input, drag thresholds, cancellation / hot reload, stale targets, cursor
  hints, explicit orders, and unchanged default / controller behavior.
- `tests/keys_tdf.rs`: a synthetic `Keys.TDF` (mapping, reserved,
  unbinding, skipped chords, bad input), and the install's file giving
  exactly Classic TA:K (data-gated).
- `tests/feel.rs`: presets, field metadata, curves, frame-rate
  independence, friction, snap, context targets, wheels, camera, recording
  round trip, and both feel-lab verdicts (on the WP-S profile, kept for
  comparison).
- The WP-J tests (`pieces.rs`, `gamepad_fixture.rs`, `equivalence.rs`) pin
  `Profile::wp_j_controller()` and the WP-S keyboard.
- `client/tak-client/tests/recording.rs` (data-gated): scripted Kingdoms
  Console session → recording → identical headless replay; `replay.rs`
  unchanged.
