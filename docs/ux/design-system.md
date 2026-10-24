# Interaction design system

How players interact with Oxidized Annihilation: one model for selecting, ordering, building, casting, moving the camera and reading the game, expressed three ways (controller, mouse and keyboard, HUD). It merges two sources:

- **TA: Kingdoms (1999)**: what experienced players know: its vocabulary, orders, stances, squads, bookmarks, key bindings and HUD. This is the default for everything that exists in the original.
- **Halo Wars (2009) and Halo Wars 2 (2017)**: the console RTS that proved a controller can run an RTS fluently. This is the template for the controller.

Where the two disagree, the rule is: **TA:K decides what the game does and what things are called; Halo Wars decides how a controller does it.**

## 1. Research summary

### TA: Kingdoms (from the install itself)

**Default keys** (the install's readme and `Keys.TDF`, which is user-editable in the original):

| Keys | Action |
|---|---|
| A / G / M / P / S | Attack / Guard / Move / Patrol / Stop orders |
| C | Clear (cancel queued orders) |
| H | Heal |
| K / L / U / O | Cloak / Load / Unload / Open-Activate |
| Ctrl+A / Ctrl+U | Select all units / all on screen |
| Ctrl+B / E / F / N / R / W / Y | Select builders / melee / factories / naval / ballistic / armed / fliers |
| Ctrl+Z | Select all like units |
| Ctrl+M | Select and track the monarch |
| Ctrl+D | Dismiss selected units |
| Ctrl+1–9, 1–9 | Create / select squad |
| Alt+1–9 | Create formation |
| Ctrl+F5–F8, F5–F8 | Set / go to bookmark |
| N / T / F3 | Next unit off screen / track unit / track last reporting unit |
| Tab | Full-screen radar |
| Ctrl+F1–F3 | Select weapon 1–3 |
| + / − / Pause | Game speed / pause |
| ~ | Toggle damage bars |
| F1 / F2 / F4 / D | Options / briefing / leaderboard / diplomacy |

**In-game HUD vocabulary** (gadget names in `guis/<race>ingame.gui`):
- **Order buttons:** Attack, Move, Guard, Patrol, Stop, Clear, Heal/Repair, Load, Unload, Pickup, Drop, Cloak on/off, Activate/Deactivate.
- **Stance:** Offensive, Defensive, Passive. These are TA:K's names for fire states.
- **Weapon select:** Primary, Secondary, Special.
- **Unit info panel:** portrait, health bar, mana bar, experience/level, kill count.
- **Minimap:** the "Crystal Ball".
- **Other:** a build menu, and a bottom bar with help text and action text.
- **Art:** each race has its own skin (`anims/aramonig.gaf`, `verunaig.gaf`, `tarosig.gaf`, `zhonig.gaf`, and The Iron Plague's `creonig.gaf`), with shared `anims/igcommonbuttons.gaf`; order acknowledgements play `Sounds/MOVE.WAV`, `ATTACK.WAV`, `GUARD.WAV`, `PATROL.WAV` and `STOP.WAV`; 171 build portraits live in `anims/buildpic/*.jpg`.

### Halo Wars

- **Cursor fixed at screen centre; the camera moves under it.** The left stick pans, and the right stick tilts, rotates or zooms. Jumping and scrolling both keep the cursor in place. This is the core idea of the scheme, per Microsoft's patent US 8,882,594, "Control scheme for real-time strategy game".
- **Selection:**
  - A selects the unit under the reticle; double-tap A selects that type on screen; hold A grows a selection circle.
  - RB selects everything on screen. In HW1, holding RB selected the whole map; in HW2 one press selects local.
  - RT cycles through the unit types inside the current selection.
- **Orders:** X issues the context order (move or attack), Y the selected unit's special ability, and B cancels.
- **Radial "circle menu" for building, research and leader powers.** At most 8 options per ring ("the magic rule of eight"), with the item's name and cost in the centre. The menu holds the cursor until you exit.
- **D-pad:**
  - HW1: cycle bases (left), last alert (right), cycle armies (down).
  - HW2: customisable group shortcuts, the console version of control groups.
- **Halo Wars 2 changes:** added control groups and hold position; larger views.

Sources: patentarcade.com (US 8,882,594); gamepressure.com Halo Wars guide, part I; Joystiq E3 2008 hands-on; GamesBeat Halo Wars 2 hands-on (2017); halopedia.org "Circle menu"; waywardstrategy.com (2020); caniplaythat.com on Age of Empires IV console (2023).

## 2. Principles

1. **One model, three surfaces.** Every action exists once in `tak-input` as an `Action` and is reachable from the controller, from mouse and keyboard, and from the HUD. No surface-only features.
2. **TA:K words and behaviours.** Use the original names (Attack, Guard, Clear, Offensive/Defensive/Passive, Crystal Ball, squads, bookmarks). Use the original defaults (stances, queueing, select-type groups).
3. **Context first, explicit second.** One button does the obvious thing for what is under the cursor. The explicit version is always one wheel slot away.
4. **The rule of eight.** No radial ring holds more than 8 items. More items means pages, never a crowded ring.
5. **Spatial memory.** An action keeps its wheel slot and HUD position everywhere it appears. Stop is always at 12 o'clock.
6. **Every input answers.** Each press produces feedback within a frame: a cursor change, sound, ring flash, order line or a refusal with a reason.
7. **Show, don't hide.** Costs, mana, cooldowns and reasons for refusal are visible before you commit.
8. **No hidden modes.** The current mode (order targeting, build placement, casting) is shown on the cursor and in the bottom bar. B always backs out one level.
9. **Deterministic.** Input becomes commands stamped with ticks, so every session replays exactly.

## 3. Interaction model

**Nouns:** own units, own structures, enemy units, allied units, features (trees, rocks, wrecks, sacred sites), ground points, squads, bookmarks.
**Verbs:** select, order, build, cast, set stance, camera, inspect.

### 3.1 Selection

| Intent | Meaning |
|---|---|
| Select | Replace the selection with the unit under the cursor |
| Add | Add or remove the unit under the cursor |
| Same type, local / global | All own units of that type on screen / on the map (TA Ctrl+Z is global) |
| All local / all global | Everything on screen / map (TA Ctrl+U / Ctrl+A) |
| Brush | Grow a circle while held and select everything inside |
| Sub-select cycle | Narrow the selection to one unit type at a time, cycling; one more press returns to the full selection (HW RT) |
| Category | Builders, melee, factories, ballistic, armed, fliers, naval, monarch (TA Ctrl+letters) |
| Squads | Assign or recall squads 1–9 (TA); the controller reaches 1–4 directly, 5–9 through a wheel |

### 3.2 Orders

- **Context order:** enemy means Attack, ground means Move, friendly frame or builder means Guard (which assists building), friendly unit means Guard, sacred site in build mode means place a lodestone, and so on.
- **Explicit orders** (TA set): Attack, Move, Guard, Patrol, Stop, Clear, plus whichever of Heal, Load, Unload, Cloak and Activate the selection supports.
- **Queueing:** TA's Shift. On the controller, hold RB while ordering. Queued waypoints draw as order lines.
- **Order targeting mode:** an explicit order that needs a target shows its cursor, and the next confirm places it.

### 3.3 Stances and weapons

- **Stance:** Offensive / Defensive / Passive (sim: FireAtWill / ReturnFire / HoldFire). It is a per-unit toggle, shown on the unit card.
- **Weapons and spells:** Primary / Secondary / Special. Spells are weapon slots that cost mana (`manapershot > 0`). The controller's Y casts the first spell (Halo Wars' special ability); more spells sit in a sub-wheel.

### 3.4 Building

- A builder selected means the Build wheel or panel shows its menu with portrait, name, cost and build time.
- Picking a structure enters placement: a footprint ghost snaps to the grid, green means valid, red means invalid with the reason ("Not a sacred site", "Blocked by feature").
- Confirm places it; hold the queue modifier to place several.
- Factories: picking a unit queues it, repeated picks queue more, and the factory's count is shown. Context order on the ground sets a rally point.

### 3.5 Camera

- **Reticle mode** (controller default, Halo Wars): the cursor stays at screen centre and the left stick moves the camera. Aim assist acts on the camera (slow-down and soft snap).
- **Free-cursor mode** (controller option; always for the mouse): the cursor moves, and pushing the screen edge pans.
- **Jumps:** to the monarch (TA Ctrl+M), last alert (TA F3), next idle builder, next squad, bookmarks (TA F5–F8), track selected (TA T).
- **Crystal Ball:** the minimap is always visible; full-screen radar on hold (TA Tab).

### 3.6 Information

- **Unit card:** portrait (build picture), name, health, mana, experience/level, kill count, stance, current order, queue.
- **Multi-selection:** portrait strip grouped by type with counts (HW selection icons). The sub-select cycle highlights the active type.
- **Alerts:** unit under attack, structure attacked, construction complete, unit trained, mana storage full, monarch in danger. Each one is a minimap ping, a bottom-bar message and a sound. The alert jump cycles through them, newest first.
- **Damage bars toggle** (TA ~): health bars off / selected only / all.

## 4. Controller scheme (default profile "Kingdoms Console")

| Input | Tap | Double-tap | Hold |
|---|---|---|---|
| **Left stick** | Reticle mode: pan camera. Free mode: move cursor | | |
| **Right stick** | Zoom (up/down); fine pan in free mode | | |
| **L3** | Jump to monarch | Select monarch | Track monarch |
| **R3** | Reset zoom | | Track selected |
| **A** | Select (add with RB held) | Same type on screen | Brush select |
| **X** | Context order (queue with RB held) | | Explicit Attack-move (attack anything met on the way) |
| **Y** | Cast first spell / special | | Spell wheel (all spells) |
| **B** | Cancel mode → clear selection | | Stop (selected) |
| **RB** | All local (on screen) | All global | Modifier: queue / add |
| **LB** | Next idle builder | | **Build wheel** for selected builder or factory |
| **LT** | | | **Orders wheel** (orders, stances, unit-specific) |
| **RT** | Sub-select cycle (unit types in selection) | | Precision (free mode) / fast pan (reticle mode) |
| **D-pad ↑ → ↓ ←** | Recall squad 1–4 | Jump camera to squad | Assign squad |
| **View / Select** | Jump to last alert (cycle) | | Crystal Ball (full-screen radar) |
| **Menu / Start** | Game menu (options, tuning, save) | | |

Wheel layout (spatial memory, the same everywhere):

```
Orders wheel (LT)              Build wheel (LB)
          Stop                 the builder's menu in TA:K order,
   Clear       Attack          8 per page; RB/LB inside the wheel
Patrol           Move          turns pages; centre: portrait, name,
   Guard    Stance ▸           cost, build time; greyed when mana
       Unit ▸                  is short, with the shortfall shown
(Stance ▸ and Unit ▸ open sub-rings: Offensive/Defensive/Passive;
 Heal/Load/Unload/Cloak/Activate where supported)
```

Confirm on release by default (option: confirm on press). Releasing in the dead centre cancels.

## 5. Mouse and keyboard ("Classic TA:K" profile)

The original keyboard defaults from section 1, including squads, formations, bookmarks, select-type groups, Ctrl+F1–F3 weapon or spell select, + / − speed, ~ damage bars, Tab radar. The existing mouse scheme stays the default for compatibility:
- Left click selects, left drag box-selects.
- Right click gives the context order; Shift+right click queues.
- Double-click selects the same type on screen.
- An order key followed by a left click gives an explicit order.

**Optional original-style mouse scheme:** `--controls original` (or
`left_click_context` in a saved profile) leaves controller and keyboard
bindings alone. A left click selects an owned unit, moves the selection on
ground, or attacks an enemy; Shift adds / queues. Drag remains selection,
explicit order modes take precedence, and right-click context remains
available. Same-type double-click selection requires two clicks on the same
owned unit, so rapid command clicks are not swallowed. **Known**: contextual
left clicks were observed in reference play (October 2026). **Chosen**: keep
the current default, circular drag region and right-click alternative; this
option is not a claim of full one-to-one original mouse fidelity.

Additions where the original had nothing, chosen so they don't collide with it:

| Keys | Action |
|---|---|
| Build hotkeys | None by default (authentic); optional "Grid" add-on: Q–R, A–F, Z–V over the build panel |
| F11 | Tuning overlay |
| F9 | Screenshot (as the original) |
| Esc | Cancel mode, like B |

**Import the player's own `Keys.TDF`** from their install when present, so a veteran's custom bindings carry over.

## 6. HUD

**Layout** follows the original's regions, on the original's side. Evidence (**Known**, `guis/<race>ingame.gui`, gadget rectangles on the 640×480 desktop): `UnitMenu` (the race's `ButtonPanel`, 128×352) is at x 512, y 128, against the **right** edge with its bottom on the screen's bottom; the `CrystalBall` (561, 434, 36×36) and `HelpText` (526, 397) sit inside it; `BottomBar` (the race's `InfoPanel`, 512×49) runs along the bottom from x 0 with `BottomEnd` (the end cap) at its left and `UnitInfo1` (portrait, name, health and mana bars, kills, experience) on it.
- side panel (right, bottom-aligned): order buttons, stances and weapon/spell buttons on the race's button panel; the build menu with portraits beside it;
- unit card: the bottom-left info bar (portrait, name, bars, stance, current order);
- top bar: player mana, income and storage, game time;
- bottom bar: help and action text (what the cursor will do, refusal reasons, alerts), between the unit card and the side panel;
- Crystal Ball: the minimap, above the side panel (bottom right when the panel is collapsed); full screen on Tab.

The spec first said "left panel"; TA:K fidelity wins (`CLAUDE.md`), so the panel is on the right as in the original.

On controller profiles the side panel collapses to the unit card and the minimap returns to the bottom right. Orders and building live in the wheels, and the bottom bar shows button prompts for the current context (Halo Wars style).

**Visual tokens:**
- **Art:** the original race skins (`<race>ig.gaf`, `IGCommonButtons.gaf`) and build portraits (`anims/buildpic/*.jpg`) where they decode cleanly. Plain panels are a fallback only.
- **Button and slot states:** normal, highlighted (gold), active (pressed or current), disabled (greyed), insufficient mana (greyed, with the shortfall in red), cooldown (radial sweep).
- **Team colours:** from the side's palette. A colour-blind-safe alternate set is an option.
- **Bars:** health (green, then yellow, then red), mana (blue), build progress (gold), experience (white ticks).
- **Cursors:** the game's own `CURSORS.GAF` (move, attack, guard, build, cast, invalid, select).
- **Sounds:** the order-acknowledge sounds named by the GUI (`move.wav`, `attack.wav`, `guard.wav`, `patrol.wav`, `stop.wav`) for every order, from any surface.
- **Text:** one UI font, two sizes (labels, values). Help text uses verbs, e.g. "X: Attack Zhon Warrior".

## 7. Feedback catalogue

| Event | Visual | Audio | Haptic (opt-in) |
|---|---|---|---|
| Selection change | Ring flash, unit card update | soft click | light tick |
| Order issued | Order line + destination marker | order sound | — |
| Order refused | Red cursor flash + bottom-bar reason | error blip | short buzz |
| Build placed | Ghost turns solid frame | build sound | — |
| Spell cast / no mana | Cast flash / mana bar pulse red | cast / blip | — |
| Alert | Minimap ping, bottom-bar line | alert sound | double tick |
| Aim-assist snap | Cursor settles, target outline | — | light tick |

## 8. Accessibility and settings

- Every binding remappable, saved per profile.
- Hold-to-toggle option for all holds (brush, wheels, precision).
- Wheel confirm on press or release.
- Aim assist strength from 0 to strong (presets: Console, Assisted, Precise).
- Rumble off by default.
- Colour-blind team colours.
- Text scale.
- Game speed (TA + / −) in single player.

## 9. What the sim must support (gaps)

The interface may only offer what the sim does. Currently missing in `tak-sim`, so hidden in the UI until built:
- Heal / repair
- Load / Unload / Pickup / Drop (transports)
- Cloak
- Formations (TA Alt+1–9)
- Dismiss (TA Ctrl+D)
- Attack-move as an explicit order
- Weapon select (Primary/Secondary/Special) as a persistent per-unit setting
- Veterancy and kill count

Each is a sim work package; the design reserves its slot so adding it changes no layout.

## 10. Measuring it

The feel lab (`tak_input::lab`) gains reticle mode, so reticle and free-cursor control can be compared on acquisition time, wrong-target rate and brush accuracy. Tyler's recorded sessions (`--record-session`) are the final judge.
