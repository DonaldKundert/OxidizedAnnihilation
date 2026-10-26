# Production and army controls verification

This slice keeps factory production separate from standing orders, makes queue
editing visible and accessible, and fixes additive squad recall. It does not
claim complete original-game production parity or physical-controller acceptance.
No retail content, screenshots, checkpoints, or extracted data are included here.

## Consolidated final local verification (2026-10-25)

The final combined source passes both full workspaces, with the final native
executable built from the same source. Earlier totals below are historical
checkpoints, not additional tests to add to these totals.

- Core owned-data: **818 passes**, no missing-data early returns, three default
  ignores (two documentation examples and the separately executed extended soak).
- Core asset-free: 818 reported passes, comprising **773 executed checks and 45
  missing-data early returns**, with the same three ignores.
- Client owned-data: **350 passes**, no missing-data early returns, one default
  ignored long lifecycle diagnostic, also executed separately and passed.
- Client asset-free: 350 reported passes, comprising **336 executed checks and 14
  missing-data early returns**, with the same long diagnostic ignored by default.
- Both formatting checks and all-target Clippy with warnings denied pass; native
  client build passes. Dependency-notice collector: **23 passes**.
- Explicit extended synthetic production soak: **64 seeds, 256 runs, 221,590
  ticks**, passed. The separate installed lifecycle diagnostic passes; its scripted
  wins are against Passive practice, while idle-player losses are against Training.
- Real Bevy input adapter tests include simulated Gamepad resources. The final
  fast minimap/keyboard timing module has **27 passes**. This is not physical
  controller testing or a claim that native UI timing reproduces one event poll.
- Native mouse/keyboard gameplay includes a fresh Aramon victory against Training
  at tick 25,952 (865 simulated seconds), paid economy and army production,
  two-target Attack box, repeat/batch/cancel, rally, minimap army orders, and
  cancel/confirm rematch followed by setup. A separate Zhon Training match ends
  in defeat at tick 10,591. Their private checkpoints independently replay to the
  recorded hashes. No resources, health or outcomes were injected in those games.
- Final HUD acceptance reloads an exact prefix of the recorded Aramon match:
  Guard assistance is visibly identified during construction and disappears on
  completion while Guard continues. This is checkpoint-based UI acceptance, not
  another new match or a changed construction mechanic.

These are **local verification results**, not remote CI. Publishing is pending
approval; no new PR, remote checks, merge, or ready-for-review change is claimed.
Physical-controller acceptance remains outstanding. Training AI is explicitly
omniscient and is not original-game AI parity. Saves remain verified replay
checkpoints rather than complete presentation-state snapshots.

## Reference evidence and chosen behavior

The owned Kingdoms manual, printed pp. 58–59 and 72–74, documents ordered
production, counts, left/right click ±1, Shift ±5, and Move/Patrol standing
orders. Its shipped Keys.TDF documents additive squad recall chords. This is
manual/config evidence, distinct from prior executable observations of selection,
movement and attack in reference-controls-validation.md.

Newest-waiting-first removal, active-inclusive counts, exact refund timing,
multi-factory dispatch and release-position Patrol anchoring are engine choices.
Controller gestures are adaptations. Unsupported factory Attack/AttackMove targeting
is refused rather than clearing production; ordinary mobile orders remain usable.
Legacy commands retain their earlier replay behavior and literal historical
construction hashes are pinned in regression tests.

## Historical initial automated coverage

Initial checks including continuous-production follow-through on the cloud workspace:

- Core: 733 reported passes, zero failures, two ignored documentation examples
  in both runs. One extended synthetic soak is additionally ignored by default
  and was run explicitly as described below. The asset-free run has 45 explicit
  missing-data early returns;
  the owned-data run has none.
- Client: 220 reported passes, zero failures, one separately gated long gameplay
  diagnostic in both ordinary runs. The asset-free run has 14 explicit data
  early returns; all fourteen execute in the owned-data run.
- Both workspaces pass formatting and all-target Clippy with warnings denied.
  The native executable links; the dependency-notice collector has 23 passes.
- Forty-one production/army adapter tests exercise the real Bevy adapter, including
  simulated Gamepad button/axis resources. These are not physical-pad testing.


Asset-free coverage includes:

- Paid-frame preservation while changing/clearing rally routes; consecutive
  inherited Move/Patrol routes and bounded automatic Patrol return legs.
- Matching tail removal, active refund exactly once, same-tick edits, ownership,
  stale IDs, invalid units, queue limits, Stop and callback cleanup.
- Lower-ID Guard and direct Heal workers racing cancellation. Explicit cancellation
  now completes before helpers can finish the cancelled frame; replacement
  production starts on the following tick.
- Cross-process command replay and checkpoint continuation across split points.
- Real Bevy adapter mouse and simulated Gamepad input, not mapper-only calls:
  independent button capture, releases outside HUD, hidden panels, modal resets,
  wheel cancel/confirm/opener release, batched additions/removals, same-poll
  modifiers, squad preservation, and mouse/stick target separation.
- Imported custom chord overrides, full chord labels, and unchanged saved squads.

The new installed-content test constructs a real Aramon factory through ordinary
paid commands, produces a soldier, edits Move/Patrol routes while a paid frame is
active, checks ±1/±5 and exact refund, and compares client checkpoint continuation
and raw replay over 5,166 ticks. It does not mutate resources, health, production
frames or outcomes. Missing installation data produces an explicit early return,
not evidence that these installed-content checks ran.

## Native GUI scope

The client is tested on the cloud Linux desktop with Mesa llvmpipe software
Vulkan. Mouse/keyboard observations are recorded separately from simulated
Gamepad adapter tests. There is no physical controller, hardware GPU or audio
output certification. Replay checkpoints still omit presentation state such as
selection, camera and explored fog, and retain pending-order and fingerprint
constraints described in client.md.

Actual cloud-window mouse/keyboard checks constructed a paid Barracks, queued
five Swordsmen with Shift-click, removed one with right-click and five with
Shift-right-click, and changed a two-leg Move/Patrol rally during production.
The queue survived the rally edits and produced units followed the route.
F10 saved and replay-verified the resulting command history. A discovered
same-poll modifier-release bug was reproduced in the window, fixed in the
adapter, covered by a regression test, and rechecked with the same fast click.

The separately gated ordinary-gameplay diagnostic also ran explicitly with
owned data: the two passive-human defeats against Training and the two scripted-player
victories against Passive practice still reach actual results through ordinary
commands, without world/outcome edits. These are not scripted-player victories
over the Training opponent.

## Continuous-production follow-through

Continuous production adds one bounded recipe rather than an unbounded queue.
The manual supports Ctrl selection, `+++`, and right-click cancellation; finite
FIFO priority and exact active-frame preservation remain documented engine choices.

Sixteen new simulation regressions include paid repeated products, FIFO origin,
recipe change/disable, cancellation, blocked exits, resource stalls, script
handshakes, and lethal factory/product death including chain explosions. New
headless tests cover cross-process replay, checkpoint boundaries, bot idle
eligibility and committed counts. Default finite hashes remain pinned unchanged.

The installed-content repeat test runs 6,174 ticks using ordinary paid Aramon
construction and production. It completes repeated troops, inserts finite work,
disables repeat without discarding the active frame, checks cancellation/refund,
and compares checkpoint continuation and raw replay without world edits.

Real Bevy adapter regressions cover Ctrl and Ctrl+Shift fast clicks, mixed
factory indicators, Y/Triangle hold/release and opener suppression, idle-builder
cycling and modal interruption. Settings opening now rolls back the entire
input poll's mapper/camera effects, including suppressed order feedback and
selection/group/placement changes, rather than only hiding submitted commands.
Recorded physical events remain diagnostic; no suppressed gameplay command is
submitted. These use simulated Gamepad resources, not physical hardware.

Native cloud-window follow-through enabled Swordsman repeat with Ctrl-click,
showed `x1 +++` and a repeating-factory count, inserted one finite Archer, then
observed Swordsman repeat resume after the Archer. Right-click removed the
repeat indicator and active product without clearing the two-leg rally. F10
saved and replay-verified the mixed repeat/finite command history. These are
mouse/keyboard GUI observations, separate from simulated-pad adapter coverage.

The in-game checkpoint browser also listed the new save with map/faction/time
metadata, cancelled replacement without changing the live tick, then loaded it
through explicit confirmation and reported a verified hash. Selection/camera
reset as disclosed; closing the browser resumed the restored match.

## Manual Training match

A separate native-window match used Aramon against the built-in Zhon Training
preset on Adamantine Gate, seed 1. Ordinary mouse/keyboard play built a paid
Lodestone and Barracks, enabled repeated Swordsmen, repaired the damaged Barracks,
assigned and recalled squad 1, and issued three minimap attack-move scouting
orders plus a forward factory rally. Training Goblin raids damaged the monarch
and factory; the produced army repelled them and later reached the opposing base.
The actual result was **Aramon victory at 760 simulation seconds**. No resources,
health, outcome or world state were edited, and no hidden-target commands were
injected. This is one observed match, not a balance or original-AI parity claim.

F10 replay-verified the completed match at tick 23,130 with state hash
`f6b5245bcfa2f0d9`. Its ordinary command history includes two player construction
orders, repeat production, repair, rally and three attack-moves; the Training
opponent issued 61 build and 40 attack commands. Those counts are issued commands,
not claims that every attempted construction completed. The private checkpoint
and native result screenshot are preserved outside the repository.

At the pre-Guard-support checkpoint, this match also exposed a misleading factory-only Guard cursor over a friendly
minimap unit. The initial correction made target hints use the same non-factory recipient eligibility as
dispatch: unsupported factory-only Guard/Attack targets say "Not possible here",
while mobile/mixed selections and factory Move/Patrol routes remain available.
Terrain and minimap regressions cover the distinction, including real SimView.

After preserving that victory, the result-menu Rematch action displayed its
cancel-default replacement confirmation. Confirming reset the scene to starting
resources and tick 27, with no selection, fresh explored fog and starting camera.
F7 then reopened setup with the same map, Aramon faction and Training opponent.

That checkpoint's native client recheck showed "Not possible here" when the selected
Barracks hovered a friendly monarch, while its `x1 +++` recipe and rally survived.
Ground hover still offered Move. The exact minimap factory-recipient distinction
is additionally covered by automated real-SimView and mapper regressions.

A second native match reversed the factions: Zhon versus Aramon Training on the
same map/seed. The player completed a paid Lodestone and Beast Handler, but the
Training raid killed the Handler and monarch before an army was established.
The result was **Zhon defeat at 246 simulation seconds**. Attempted Goblin
placements during the raid did not appear in the submitted command history and
are not counted as successful queue coverage. F10 verified this separate result
at tick 8,011, state hash `ff07a47899240462`; no outcome or balance edits were used.

A separately labelled Zhon Passive practice session isolated direct/mobile-builder
controls from raid pressure. The completed Beast Handler summoned three paid
Goblins from Shift-appended placements. The native window showed all three
complete. Assigning them to squad 1, selecting the Handler, then Ctrl+Shift+1
selected all four; pressing 1 again restored only the three Goblins, confirming
additive recall did not overwrite the saved squad. This is practice-mode control
coverage, not another Training victory.

The practice construction/movement replay verified at tick 8,773 with hash
`c7084fcdcfb29856`: three appended Goblin builds, two earlier replacement builds,
and one squad Move. Selection and squad membership are native GUI observations,
not checkpoint presentation state.

## Unfinished-builder eligibility

The Zhon match exposed a selectable summoning frame advertising an Idle status
and build menu even though simulation rejects unfinished builders' orders.
Construction action eligibility now excludes such frames consistently across
mouse menus, mapper dispatch, controller wheel and idle-builder cycling. The
unit card says "Under construction". Frames remain selectable for inspection and
valid assistance targets; mixed selections use the first eligible complete
builder. Completed units retain ordinary Append and factory repeat/rally behavior.
Eight new regressions cover input eligibility, mixed menu sources, actual Bevy
HUD/card projection, stale buttons and controller wheel presentation. Simulation
and checkpoint rules are unchanged.

Native checkpoint recheck selected the partially summoned Handler at tick 3,484:
its card read "Under construction" and no build portraits were offered. Keeping
that selection through normal completion restored Idle and the full build menu
by tick 4,309 without reselecting. The private checkpoint was reconstructed from
ordinary paid construction, not a manually edited world fixture.

The follow-on audit deliberately did not change the legacy simulation's general
unfinished-unit order policy. Move/Patrol and AttackMove cannot take effect while
unfinished, while some Attack/Guard orders and stance changes can persist.
Generic frame movement affordances still need a separately scoped input-only
correction; this construction fix does not claim to solve every frame command.

A real Bevy Gamepad-resource regression also holds LB and A across a builder
completion transition, checking both confirmation styles and both release orders.
No stale construction or placement leaks through; a fresh wheel gesture then
places exactly one ordinary Build. This remains simulated input, not hardware.


## Multi-factory stress and Stop/requeue regression

Synthetic stress tests cover four seeded 1,620-tick schedules across four
factories/two owners, independent finite/repeat/rally models, exact resource
conservation, blocked exits, starvation/recovery, death/restart and generational
slot reuse. A separate checkpoint stress uses two seeds, each 371 ticks and 169
commands through 21 serialized reload boundaries, plus separate-process runs and
seven chained resume intervals. CLI children have a 30-second wall-clock limit;
hash comparisons include absolute tick labels. These are synthetic regression
fixtures, not additional installed-content gameplay demonstrations.

The soak exposed an existing finite-production edge: raw Stop followed by a new
Build in the same simulation tick could finish the old product while consuming
the new queue entry. A characterization test retains that legacy replay behavior.
New client Stop input explicitly cancels observed factory work before ordinary
Stop, preserving historical command semantics while preventing the UI edge.
The cancellation budget covers earlier same-tick queue additions without relying
on a stale count. Direct real-adapter tests cover keyboard/HUD, paused separate
polls, and simulated controller hold/confirm paths; old paid work is refunded once
and the requested replacement starts on the following tick.

The rebuilt native window queued five finite Swordsmen, then accepted Stop and
an Archer request while paid work existed. It displayed Archer construction and
then an empty completed queue; Stop cleared the old rally. F10 verified tick
25,857, hash `9b602fd09d401969`. That native trace records cancellation and Stop
at tick 25,102, with the Archer request at 25,113, so it is not claimed as native
same-tick timing proof. The real-adapter tests establish that exact edge, including
paused separate polls, stale counts and repeated Stop with exactly one refund.

The separately gated extended soak also ran explicitly: 64 additional fixed seeds,
128 original/JSON production runs (207,360 ticks), and 128 synthetic death/restart
runs (14,230 ticks), totaling 256 runs and 221,590 simulation ticks. It passed in
297.46 seconds with no invariant or replay-hash mismatch. This adds deterministic
regression depth, not new installed-content or controller-hardware evidence.
Run it explicitly with:

```sh
cargo test -p tak-sim --test factory_controls_stress \
  extended_seeded_factory_controls_soak -- --ignored --exact --nocapture
```

## Stop while a build wheel is open

A subsequent real-adapter regression reproduced B-hold Stop followed by ordinary
LB release re-adding the highlighted product without a new confirmation. Stop
now dismisses the pre-existing build wheel and consumes its opener release.
This is an input-gesture correction, not a change to historical Stop commands
or replay schemas. It also prevents an old mobile-builder wheel from entering
placement after Stop; unrelated target modes retain their existing behavior.

The correction passes a 20-case real Bevy adapter matrix covering both wheel
confirmation styles, batching, all B/LB/RB release orders and same-poll releases,
paid-frame/repeat cleanup, unchanged selection/camera and fresh-gesture recovery.
A separate mapper regression covers keyboard Stop with custom opener-release
bindings for factories and mobile builders. These are simulated inputs, not a
new physical-controller or native-pad claim. The deliberate Stop followed by a
new HUD Build regression still passes. Independent static review found no blocker.

## Native multi-factory and low-mana continuation

A second paid Barracks was built in the existing Aramon Passive-practice match.
With both factories selected, the unfinished one was correctly excluded from
production eligibility: Shift-click issued five Swordsmen only to the completed
one. After completion, Shift-click queued five Archers per factory and the
aggregate HUD showed ten. A right-click submitted one cancellation per factory;
concurrent ordinary completions also reduced the visible count.

A shared rally and continuous Swordsmen recipe preserved the finite Archer queue.
Changing only one factory to Archer repeat then showed one of two factories for
each recipe. Cancelling Swordsmen under low mana left Archer repeat active, with
new products reaching the shared rally. F10 replay-verified tick 33,772, hash
`c58db4fc7d5a8570`. The in-game checkpoint browser loaded that exact tick after
confirmation, showed a successful verification message and reset presentation
state as documented. After resuming, the single-factory Archer recipe and both
rally routes remained visible and production continued using ordinary income.
Stop subsequently cleared both factories' queues, recipes and rally lines; the
HUD showed zero queued and zero of two repeating while mana recovered.
The final stopped state replay-verified at tick 35,670, hash `0c12f521f8c2d66c`.

These are native mouse/keyboard observations in Passive practice, not another
Training win or physical-controller test. All construction and production used
ordinary commands and paid resources; the private checkpoint history confirms
the one-factory and two-factory dispatch sets.

## Guard standing-order extension

The owned manual's printed p. 59 specifically documents Move/Patrol inheritance;
pp. 70 and 72–73 list standard Guard and extend standing orders to menu functions.
This is documentary support for Guard, not an original-executable Guard trial.
The extension supersedes the earlier deliberate factory Guard refusal described
above; Attack/AttackMove remain refused. Target validation, lifetime and yard
egress are explicitly Chosen rules detailed in construction.md.

The Guard integration also exposed a client presentation gap: active combat Guard
is promoted out of the ordinary queue, so checking that queue alone can label an
escort Idle/Moving. The HUD now reads active Guard explicitly and hides its
temporary steering Move from the displayed player queue, without mutating sim
orders. Target lines follow the current friendly target; stale/hostile targets
are not used as live display positions.

Automated Guard acceptance includes 16 synthetic simulation cases, seven input
regression tests with routing/eligibility matrices, two command-only checkpoint
replay tests with separate-process checks, and five new real Bevy adapter matrix
tests. The owned integration uses paid ARAKEEP → ARASWORD production and a moving
own monarch over 5,632 ticks, observes active Guard motion rather than merely a
queued Guard entry, and compares checkpoint continuation and raw replay hashes.
All three owned production integration tests execute with installed data; absence
of that data is an explicit early return. Literal pre-Guard Move/Patrol hashes
remain pinned, and independent static review found no outstanding blocker after
correcting the active-state test and Guard-assist display coverage.

Native Guard acceptance loaded the earlier two-factory checkpoint without a
historical hash mismatch. Both factories accepted friendly-target Guard for
Elsin and continued ordinary paid Swordsman production, including repeat. The
selected produced Swordsman visibly read "Guarding Elsin" with a green target
line; after a second ordinary monarch Move it followed the relocated commander.
F10 verified the resulting replay at tick 45,881, hash `be67cabfb98d90b9`. This
was a Passive-practice native mouse/keyboard check, not Training victory or
physical-controller testing. The private command history contains the separate
factory Guard recipe, repeat command and two monarch Move commands.

The same native session exposed a repeat-factory status bug: a paid repeated
product could be shown as Idle because its order is not in the finite queue.
The HUD now prefers the actual paid product, counts finite orders behind a
repeat separately, and says "Waiting to repeat <type>" when only the recipe
remains. It does not guess whether mana, the yard, a script or tick timing is
responsible. Six synthetic HUD regressions also cover changed/disabled recipes,
legacy rally orders before finite production, canceled-frame replacement counts,
and Stop/re-enable, with unchanged-World assertions. The rebuilt native client
loaded tick 45,881 at the same verified hash and visibly showed Building
Swordsman for the two repeating factories while retaining Guard rally lines.

For this presentation-only follow-up the full client workspace reported 226
passes, zero failures and one ignored diagnostic in both runs. Asset-free mode
included 14 explicit missing-data early returns; the owned-data run included
none. All 171 client library tests, all-target client Clippy, formatting and the
native executable build passed. Physical-controller acceptance remains pending.

## Queued mobile placement extension

The owned manual's printed p. 59 describes pre-held Shift plus primary drag to
queue spatial construction, explicitly including walls and Zhon forces. The
new gesture reuses existing Append Build commands. Whole-footprint snapping,
non-overlap filtering, observed-point-only sampling and the 64-site gesture cap
are Chosen implementation bounds, not measured original-executable rules.
The later Shift-after-mouse-down variation remains deferred.

Sixteen synthetic mapper regressions cover both mouse profiles, ordinary small
click targets, unchanged unrelated double-click selection, rapid click-to-drag,
geometry validation, bounded dispatch, interruptions and custom release bindings.
Four read-only SimView cases cover even/odd snapping, sparse whole footprints,
boundaries, terrain, occupancy and unchanged world hashes. A new command-only
200-tick headless case preserves existing paid and queued work, demonstrates
ordinary mana starvation and terrain rejection, completes four legal sites,
and compares serialized logs, raw replay, checkpoint cuts/chains and fresh CLI
processes. No running-world mutation is used to obtain its gameplay results.

The actual Bevy adapter regressions exposed two integration issues before native
acceptance: the rendering ray extrapolates beyond the map, and idle surface-loss
checks must not create redundant raw-recording frames. The adapter now verifies
rendered map extents, always delivers gesture cancellation, and records only a
real terrain-to-nonterrain transition. Independent read-only review also found
Classic's rapid click-to-drag second-tap conflict; the fix bypasses stale tap
history only for an eligible queued mobile-build mouse gesture.

Final automated totals for this slice: core 750 passes, zero failures and three
explicit ignores in both modes (45 missing-data early returns only in the
asset-free run); client 241 passes, zero failures and one explicit ignored
long gameplay diagnostic in both modes (14 missing-data early returns only in
asset-free mode). This includes 52 actual Bevy production-adapter tests, with 11
new placement cases. Workspace Clippy, formatting and the native client build
passed. The earlier long diagnostic and extended production soak are documented
above, not misreported as rerun for this input-only change.

Native mouse/keyboard acceptance loaded the prior Zhon Passive-practice save,
selected Beast Handler, chose Goblin, and Shift-dragged over clear terrain. The
private replay contains exactly five ordinary Append Build commands at five
snapped sites, submitted over ticks 9,844–9,851. Selection stayed on the Handler;
it visibly walked, paid for construction and finished all five Goblins along
the sampled path, then returned to Idle. A mid-production F10 checkpoint verified
at tick 10,207, hash `7101cb912789db16`; a completed checkpoint verified at tick
11,703, hash `22e5932c8291694c`. These are native GUI observations in practice,
not a Training outcome or physical-controller test.
The in-game browser then loaded the mid-production checkpoint after its default-
cancel confirmation. It reported the verified tick/hash and reset presentation
state as documented. After closing the browser and reselecting the Handler,
Building Goblin and the remaining route continued without issuing new orders.

## Combined standing-order stress and status readability

A separate synthetic command-only stress case combines Guard standing orders
with finite/repeat/cancel/Stop editing, blocked pads, ordinary income outages,
moving targets, command-driven target death and arena-slot generation reuse.
Four fixed seeds run 1,320 ticks in three replay modes (raw twice, then serialized
scenario/log), totaling 15,840 invariant-checked ticks. Assertions cover exact
route/queue state, paid-frame ownership, once-only refunds, resource conservation,
finite and repeat Guard release, actual escort movement and post-loss filtering.
No engine defect was found. This is synthetic regression evidence, not an owned
match or original-game behavior measurement.

Native factory inspection exposed overlapping name/status text on the original
fixed-size card. The chosen presentation fix preserves the art, portrait and
bars, but puts complete name/status text in a width-bounded wrapping panel above
it, with the unit-type strip flowing above that. There is no ellipsis, hidden
controller detail action or alteration of order semantics. Four actual-widget
regressions check both HUD modes, full long-name/queue/rally text, selection-state
transitions, real visibility propagation and mouse capture. Their deterministic
hit rectangles do not claim to test glyph shaping; native screenshots check that.

Final checks for this combined slice reported 751 core passes in each mode,
zero failures and three explicit ignores; 45 missing-data early returns occurred
only in the asset-free run. Client checks reported 245 passes in each mode,
zero failures and one explicit ignore, with 14 missing-data early returns only
in asset-free mode. Clippy, formatting and native build passed. The separate
long gameplay diagnostic and extended soak were not rerun for this presentation
change. Native Classic and Console inspection at the actual 1,180-pixel viewport
showed complete factory name, paid Building Swordsman, Guard rally and health
without overlapping the adjacent controls. A requested 960-pixel launch remained
1,180 pixels in the observed desktop; narrow native-window acceptance is therefore
unverified, not counted as passed. Automated bounded-layout assertions cover the
chosen width rules, but are not a substitute for that missing visual check.

### Fresh Zhon Training match after placement/status changes

A new native mouse/keyboard match selected Zhon versus Aramon Training through
setup with the normal seed and starting resources. The paid Lodestone completed
and raised income from 10 to 30 mana/s; a queued Beast Handler completed. A
Shift-drag over terrain produced exactly three ordinary Goblin Append Build
commands (ticks 5,479, 5,479 and 5,481); obstructed sampled terrain did not create
extra commands. The live Handler displayed Building Goblin (+2). Enemy Scouts
raided the army and killed the Handler. A paid replacement Handler completed,
but also died in combat before the attempted new formation could issue orders.
The monarch received a minimap attack-move defense order at tick 7,585 and an
explicit attack at tick 10,494. Ordinary combat ended in defeat at tick 10,591
(the result panel displays 353 seconds). This is not a Training victory or a
balance comparison: the opening included manual inspection delays, and neither
resources nor outcomes were injected or altered.

Four native F10 checkpoints independently restored in the headless executable:
4,071 / `2b8b8464f9df28fd`, 6,227 / `292ad4ff8b99a95c`,
10,498 / `c3efb2d5470013bd`, and 10,751 / `ba76db5645234e03`.
The final replay confirmed winner player 1 at tick 10,591. On the native result
menu, Enter on the default-cancel replacement dialog preserved the finished
match. Explicitly confirming Rematch then reset the starting camera, fog,
selection and economy; the Training opponent began its opening construction
again. F7 returned to paused setup with the correct Zhon/Training choices.
This exercised actual mouse/keyboard UI and ordinary gameplay, not a physical
controller. Simulated-pad coverage remains the separately documented adapter
regressions. All replay data and screenshots remain private, outside the repo.

## Paid factory construction progress

The same native inspection exposed a separate presentation inconsistency:
completed factories described their paid product as Building, but their gold
progress track inspected only the factory's own already-complete frame and hid.
The card now reads the actual product of a completed local stationary builder.
A selected unfinished frame retains precedence; queues and repeat recipes do not
invent progress, enemy factories do not reveal production, and stale generational
product IDs hide safely. Existing progress arithmetic, color and selected-unit
health/mana are unchanged. This is an internal consistency fix, not a claimed
measurement of original-game presentation.

Five rendered-widget regressions cover both HUD modes, finite/repeat work,
recipe changes, pending cancellation, zero-paid progress, precedence, completed
or missing products, enemy/nonfactory gating, arena reuse and selection changes.
Each checked update preserves the full sim/script hash and recorded commands.
Independent read-only review found no blocker. Client formatting, all-target
Clippy, native build and both full suites passed: 250 reported passes per mode,
14 individually reconciled missing-data early returns only in asset-free mode,
zero early returns with owned data, and one explicit long diagnostic ignore.
Core code is unchanged from the 751-pass checkpoint above and was not rerun.

Native mouse/keyboard inspection restored the prior paid-repeat checkpoint,
selected both Barracks, and showed the gold track advance alongside Building
Swordsman while the selected factories' green health bars remained full. A
right-click cancellation cleared the active products/repeat recipes, showed
Idle with the existing Guard rally, and hid the gold track without stale fill.
The complete names/status remained readable. Recipe-change preservation and
enemy privacy are automated-widget evidence, not additional native observations.

The initially missing narrow native check was subsequently completed. The cloud
window manager had retained a maximized state; its native Unmaximize action
restored the requested 960×700 content viewport, independently confirmed in the
window inventory. Both Classic and Console then showed the full selected-factory
name/status, Guard rally, health and gold product progress without overlapping
adjacent controls. Console hints wrapped beside the minimap. Private screenshots
capture each actual 960-pixel window. This supersedes only the earlier narrow-
window verification limit, not the distinction between UI and hardware tests.

## Explicit Attack rectangle and target-route visibility

The owned English manual's printed page 73 describes Attack followed by an enemy
box drag. Source inspection found that all larger primary drags instead used the
owned-unit selection brush, even in explicit Attack mode. A native baseline check
with a selected monarch confirmed that an empty Attack drag cleared selection.
The separate attempted enemy-drag baseline was confounded by the monarch dying
before the result inspection and is not counted as conclusive native evidence.

The new gesture uses existing Attack commands only, preserving army selection,
ordinary selection/short-click behavior and controller bindings. Target ordering,
64-target cap, inclusive world rectangle and release/Frame policy are explicitly
chosen, as documented in controller.md. A terrain-following red outline displays
bounds only, with no hidden target count or location information. Its edges
sample crossed attribute-cell boundaries with bounded renderer work.

Independent review caught and corrected a regression before acceptance: the
initial implementation swallowed an existing explicit Attack minimap short
click. The existing library test failed (213 other tests passed), then passed
after preserving valid map-origin short clicks and distinguishing them from
world strokes interrupted by map/HUD/focus changes. Additional review hardened
same-poll activation, duplicate releases, stale adapters before the target cap,
and recording genuine map-surface loss. Raw input-event fidelity is distinct
from replay checkpoints, whose authoritative executed command logs were not
corrupted by the omitted input event.

The review also exposed a pre-existing rendering leak: queued unit-target order
lines followed raw world positions after enemies left current sight. Rendering
now resolves these positions through the same fog-filtered, exact-generation
view as targeting. A hidden active attack's temporary steering line is removed
without hiding a genuine later Move at identical coordinates. Known ground
orders and friendly/allied target behavior remain visible; simulation pursuit
and order/status semantics are not changed.

The final client gates passed formatting, all-target Clippy, the native build,
and both full suites: 283 reported passes with owned data and zero data-gated
returns; asset-free execution comprises 269 actual tests plus 14 individually
reconciled owned-data early returns. Both retain one explicitly ignored long
lifecycle diagnostic. This includes 24 real Bevy adapter cases and nine synthetic
actual SimView/FogGrid rendering cases. Simulated controller Context remains
covered; no physical-controller acceptance is claimed.

A synthetic headless fixture drives the actual mapper's Replace and Shift-Append
outputs through ordinary commands. It checks a dead first target, subsequent
targets, a target moving outside the original rectangle, direct replay, chained
checkpoint restores and fresh-process CLI hashes. These are controlled fixture
results, not additional wins against the owned-data training opponent.

Native mouse/keyboard validation on the rebuilt client restored the live Zhon
training checkpoint. A visible-enemy rectangle preserved monarch selection,
left Attack mode after success and emitted a normal Attack command at tick 6811.
F10 saved tick 6824 with hash a2e2a947a56282ed; independent headless restore
verified all 6824 ticks and the same hash. Only one eligible enemy survived to
that rectangle's release, so this observation alone does not prove a native
multi-target queue. An empty rectangle showed its red bounds during the gesture;
a settled post-release view retained both the selected monarch and Attack mode,
in contrast to the conclusive pre-change empty-box baseline.

Final core formatting, all-target Clippy and both full suites also passed:
781 reported passes per mode, with 45 expected missing-data early returns only
in asset-free mode (736 actual executions), zero owned-data early returns and
three explicit ignores. Final-source totals include the surface-validity getter
regression, not the superseded intermediate runs. No remote CI was triggered
while publication approval remained pending.

## Selection-tap provenance follow-up

The subsequent real Bevy adapter baseline reproduced a controller selection bug:
a second rapid A on a different unit selected both units of the first type,
instead of the second unit. The positive same-target double-A control passed.
Source audit traced the mismatch to unrestricted same-type tap history, also
seeded by nonselection wheel confirmations. Target/provenance-aware controller
refinement is Chosen UX, not a claim about original gamepad controls. Existing
generic delayed DoubleTap, RB global selection and squad-centering refinements
are separate contracts and must remain compatible.

The fix is covered by 15 mapper regressions and 17 actual Bevy adapter tests.
The adapter baseline also isolated two consumed-click boundaries: HUD A and
fullscreen-map pan A. Their recorded interruption now JSON-round-trips and
reproduces full mapper outputs, feedback and selection; omitting it deliberately
recreates stale same-type expansion. Both normal release-confirm wheels and the
supported press-confirm tuning are covered, not conflated.

Final formatting, all-target Clippy and native build passed. Core suites report
796 passes each: asset-free includes 45 expected early returns (751 executed),
owned-data has none, and both retain three explicit ignores. Client suites report
300 passes each: asset-free includes 14 expected early returns (286 executed),
owned-data has none, and both retain one explicit long lifecycle ignore.

Native mouse verification used the restored passive Zhon practice save, not a
training win. Two separate desktop clicks on the same Goblin completed in 207 ms
and visibly selected all five nearby Goblins. Clicking a Goblin and then the
Beast Handler completed in 284 ms and visibly selected only the Handler, with
its correct build panel. These are desktop call elapsed times, not a hardware
controller latency benchmark. Initial window-bound clicks and a bundled
click-count attempt did not establish the intended timing; they are not counted
as acceptance. The settled successful states were inspected and captured
privately. Controller timing remains simulated actual-adapter evidence only.

## Stop and unfinished order gestures

Native inspection on the Attack-box checkpoint (9f3a0b7) recorded
real Stop commands at ticks 6950 and 7271 while Attack mode and its confirmation
hint remained armed. F10 checkpoint 7282, hash dcc03e083f0e0bb7, independently
replayed successfully. The defect was input state, not failure to execute Stop
in the simulation. Seven focused core cases and eight actual Bevy adapter cases
then reproduced pending modes, wheel releases and held gestures surviving Stop.

Disarming unfinished UI orders is explicitly Chosen defensive behavior. The
owned manual's printed page 70 establishes Stop, not the exact pending-wheel or
held-button reset rules. Existing Stop production cancellation and simulation
commands remain the compatibility contract; no replay rules migration is needed.
The documentation also removes stale claims that already-implemented Heal/repair
and attack-move are reserved or unavailable.

Independent review exposed a further boundary before acceptance: an original-
style friendly selection click preserved across Stop could turn into Move after
target removal or Attack after ownership changed. The actual adapter reproduced
both. A later mode or sticky wheel could similarly be confirmed by an old held
A or delayed selection tap. Surviving selection gestures now retain their
pre-Stop origin and cannot borrow newer order state. Invalid short releases are
inert; valid selection, started box/brush selection, physical modifier cleanup
and genuinely fresh controls are retained. This state is local to the mapper;
simulation state and serialized authoritative command formats are untouched.

Final independent review approved the frozen mapper. All 22 focused core and
17 real adapter regressions pass. Formatting, all-target Clippy and native build
pass. Full core suites report 818 passes each: asset-free includes 45 expected
missing-data early returns (773 executed), owned-data has none, and both retain
three explicit ignores. Full client suites report 317 passes each: asset-free
includes 14 expected early returns (303 executed), owned-data has none, and both
retain one explicit long lifecycle ignore. These are local gates, not remote CI.

The rebuilt native client restored passive Zhon practice. S cleared an explicitly
armed Attack mode while retaining the monarch; selecting a Beast Handler,
arming Goblin placement and pressing S removed the placement outline/hint while
retaining the builder. A fresh portrait selection and terrain click then paid
for and completed another Goblin. The recorded continuation contains exactly
Stop at ticks 12831 and 14062 followed by the fresh Build at 14448, with no build
from the cancelled placement. F10 saved tick 14916, hash d164bf468d65f875.
This is native mouse/keyboard acceptance plus separately simulated actual-adapter
controller coverage, not hardware-controller evidence or another training win.
Independent headless restore verified all 14916 ticks and the same hash.

## Combined-controls ordinary training follow-through

A fresh native Aramon versus Zhon training match on the Stop-gesture build
(40cdb46) exercised the combined controls without resource or outcome injection.
Paid Lodestone income reached 30 mana/second. Barracks construction began at
tick 14587 and was interrupted to defend the monarch. Guard at tick 16963
resumed paid assistance. Completion was observed at next-tick 17449
(execution tick 17448). A later Heal at tick 17538 found the already-complete,
full-health target and did no work. Private
exact-history tracing corrected the initial visual attribution to Heal; this
was not a construction-resume defect. The long delay before factory construction was
operator time spent investigating input behavior, not a balance benchmark.

An explicit Attack box over approaching raiders emitted two commands at tick
15901, first Replace against unit 4 generation 5 and then Append against unit 6
generation 5. This supplies native multi-target queue evidence beyond the
earlier single-survivor box. F10 checkpoint 15908 independently replayed all
49 commands to state hash 6e473a06410c449b. No hidden targets were intentionally
addressed; synthetic visibility and lifetime regression tests remain the stronger
exhaustive evidence for those boundaries.

The completed factory trained repeated Swordsmen and retained paid production
while its standing order changed to Guard the monarch. Shift-click added five
finite copies, right-click removed one and disabled repetition, then Ctrl-click
re-enabled repetition. The native queue count, recipe indicator and progress
bar reflected the transitions. Seven trained Swordsmen were assigned to a group
and received an explicit minimap Attack-move at tick 21760. The factory subsequently accepted a Move rally while retaining the active
recipe. The seven-unit group advanced through a second minimap Attack-move at
tick 24008 and defeated the opponent at tick 25952 (865 seconds displayed),
with five members surviving. A later attempted reinforcement input arrived after
the result modal and issued no additional army order.

F10 checkpoint 27079 independently replayed 114 commands to state hash
8fca9457c8a1afcb and confirmed winner player 0 at tick 25952. Intermediate
checkpoints 19094/a3b03ae1877fc069 and 21765/7c15722d85d76a19 also replayed
independently. This is a fresh native mouse/keyboard training victory, separate
from earlier scripted passive-practice outcomes and simulated-pad coverage.

The result menu defaulted to Cancel and preserved the victorious match when
activated. Explicit rematch confirmation reset time, camera/fog, economy,
selection and opponent; returning to setup retained Aramon versus Training on
Adamantine Gate. These were native mouse/keyboard observations on the same build.

## Fast minimap and HUD capture boundaries

The combined-controls native match exposed a fast minimap click that panned the
camera but cleared a living unit's selection. Actual Bevy-schedule tests proved
that down and up in one poll cleared persistent map capture before swallowing
the current edges. All six default/Classic/imported-profile and small/full-map
combinations failed, while split-poll clicks and explicit target modes passed.
The adapter now retains a per-poll capture snapshot while releasing persistent
state normally. No authoritative command, simulation hash or replay version was
changed.

Independent review found a related reachable HUD boundary. A normal HUD down,
then normal release, retained an extra `swallow_release` poll. A fresh complete
terrain click in the following poll was lost, including selection and actual
HUD-established Move/Attack/Patrol/Guard confirmations. Split clicks or one idle
poll masked the defect. Capture now carries forward only while still held at the
end of the poll; the current release remains consumed. This is defensive input
correctness, not a claim about exact original-game frame polling.

Eight new adapter tests cover these cases, custom Context routing, immediate
world clicks, and JSON raw-event playback with equal outputs, feedback and
selection. All 47 minimap-related focused tests pass. Independent source review
approved the combined change. Full client gates passed formatting, all-target
Clippy, native build and both suites: 325 owned-data tests executed with no data
skips; asset-free reported 325 comprises 311 executed plus 14 explicit no-data
early returns. Both retain one explicitly ignored long lifecycle test. These
remain local gates, not remote CI or hardware-controller acceptance.

Core gates were rerun unchanged and passed: 818 owned-data executions with no
data early returns; asset-free 818 reported passes comprises 773 executions and
45 explicit early returns. Both retain three explicit ignores. Native mouse
validation on the rebuilt client restored passive practice, selected a living
Goblin, panned the minimap into undiscovered terrain and back without losing
selection, then confirmed Move through the actual HUD and a terrain click.
F10 saved tick 17875, hash 8e9dcc1db2cb046e; the continuation contains only the
intended Move at tick 17868. Exact one-poll boundary timing is established by the
adapter tests, not inferred from UI automation wall time. An initial click while
Chromium unexpectedly held focus and an inconclusive timed bound-window HUD
attempt are excluded from acceptance; neither generated a simulation order.

Independent headless restore verified all 17875 ticks and the same state hash.

## Guard assistance visibility, without changing orders

Exact-history replay corrected the apparent construction-resume problem in the
native training match. Guard resumed paid work immediately after tick 16963 and
the Barracks was complete by next-tick 17449. Omitting only Guard in a private
counterfactual left the frame unfinished until the later Heal; omitting only Heal
retained the original completion time. The original checkpoint was unchanged and
the exact baseline reproduced its final hash. These diagnostic counterfactuals
are not additional played matches or altered acceptance outcomes.

The HUD now retains the continuing `Guarding` order and additionally identifies
its valid active assignment as `Assisting` or `Moving to assist`. This describes
an assignment, not a promise of paid progress while mana is unavailable. Exact
target generation, ownership, life/completion, Guard relationship, worker
eligibility, owned steering and range are rechecked. Product/rally precedence
and player queue counts remain intact. Six new synthetic tests and independent
source review cover those boundaries; command and simulation behavior is unchanged.

## Same-poll keyboard target modes

Actual-adapter baselines proved that a target key and map click arriving together
could be classified as a plain camera gesture before the effective binding was
known. A separate terrain control showed that complete Move/Patrol/Guard and
Alt+A taps also lost their target because the key followed both mouse edges.
The adapter now previews effective bindings without dispatching an input twice,
and prioritizes unambiguous keyboard presses for the five existing direct target
modes: Attack, Attack-move, Move, Patrol and Guard. Ctrl+A/G/M remain selection
chords; keyboard remaps and modifiers are resolved through the profile.

The scope is deliberately conservative. Newly predicted/prioritized mode inputs
must be keyboard keys with no potentially overriding Grid or alternate gesture,
and their binding must agree whether a fresh, potentially captured pad A reaches
the mapper or not. Ambiguous LMB chord preview falls back to the prior held-state
classification. Existing controller/mouse-button ordering and legacy Attack
priority remain unchanged. This does not promise full same-poll behavior for
Grid keys, alternate-gesture keys or ambiguous mixed-device chords; build, Heal,
spell and wheel actions are not newly prioritized.

Independent review and regressions prevented unintended Grid construction and
Stop reordering through consumed/delivered A, as well as pad A/Y routing changes.
The final 27-test adapter module includes complete/split map/terrain clicks,
custom remaps, special-action exclusions, raw JSON playback and established
per-mode wheel/Stop/Escape/selection arbitration. Point clicks may issue before
a same-poll Stop or Escape, as the existing Attack path does; this is not a new
universal cancellation transaction, and rectangle Frame-commit behavior differs.

### Final native assistance acceptance

On the final rebuilt executable, an exact original-match prefix was restored at
next tick 16,964 (58 commands, verified hash `069cfb0b3aa2397f`). This is a
checkpoint UI test, not a new gameplay victory. Selecting Elsin with Ctrl+M showed
`Guarding Barracks | Assisting Barracks` while the original Guard order continued
paid construction. After normal completion the suffix disappeared and the status
remained `Guarding Barracks`. No Heal or replacement construction order was sent.
Private screenshots preserve both states. Initial shader compilation delayed
input processing; early key attempts during startup are not timing acceptance.
The native test verifies the final display and ordinary keyboard selection;
exact single-poll edge timing is covered by the 27 real-adapter regressions.

Final native binary SHA-256:
`e34b0f69ddf482f1c4fc087d876f6e43331e77562f791bdbed228eba9cc6ea88`.
