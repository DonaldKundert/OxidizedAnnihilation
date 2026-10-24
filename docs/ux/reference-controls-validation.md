# Reference-led controls and checkpoint browser

## Scope and reference evidence

This slice builds on main `681546adaabf4230a41b067333ec38e7859dff12`.
It does not change simulation balance, introduce retail game files, or parse
original `.tak` saves. The original executable and private reference runtime
were not modified by this implementation.

Manual observations of the owned original TA: Kingdoms executable on
2026-10-23, using Software MMX rendering:

- In an Abnar's Terrace skirmish, left-click selected the local monarch;
  left-click ground ordered movement and left-click the minimap ordered a
  distant move. Unit displacement, walking animation and fog reveal were visible.
- Left-click an enemy structure ordered an attack. Projectiles, an attacking
  status, and subsequent destruction were observed.
- `T` followed a moving monarch. This reconstruction already supports follow;
  no redundant follow feature was added here.
- The original load browser displayed map, faction and elapsed match time.
  Loading restored an earlier world with an enemy structure alive again.
- In a campaign, explicit `M` then minimap targeting moved the selected army.
  Some ordinary viewport clicks did not reliably command it; their cause was
  not isolated. This is not evidence that every original control detail matches.

These are behavioral observations, not code or binary analysis. No original
screenshots, archives, models, sprites or extracted retail text are part of this
repository. The optional `original` control scheme preserves the existing
`kingdoms` default and right-click workflow; exact compatibility choices are
specified in [controller.md](../controller.md).

## Replay checkpoints are not original save files

The browser uses the existing versioned replay-checkpoint format. It reconstructs
the source and executed command history, checks the installed gameplay-content
identity and final simulation hash, and only then exposes a restored session.
The existing F10 quick-save and `--load-checkpoint` command line remain supported.

Browser saves reject unexecuted/future log entries instead of silently omitting
them. Already executed orders whose effects remain in progress are reconstructed
by replay. F10 retains its existing behavior and reports excluded future orders.
Camera position, selected units and client-side explored fog are not serialized.
The browser says this explicitly; restored local sight is initialized immediately.

Load and overwrite require confirmation with Cancel initially selected. The
selected checkpoint is revalidated against the catalog's change fingerprint.
A failed parse, changed file, unsupported source, replay-budget failure, changed
content, state mismatch or render-content preparation failure leaves the current
match intact. The live scene is replaced only after the fallible preparation,
without destroying/recreating Winit's event loop. A successful load keeps the
browser open and the new match paused; Close/Esc/B resumes it.

## Verification

See the pull request checks for the exact published revision. Synthetic tests
exercise the actual Bevy input adapter with simulated `Gamepad` state; they are
not physical gamepad/hardware certification. Data-dependent tests use a private
owned installation and are deliberately skipped on public CI when
`TAK_DATA_DIR` is absent.

Local verification on Rust 1.99.1 (2026-10-24):

- Core formatting and workspace/all-target Clippy with warnings denied pass.
- Full core workspace tests: 621 reported passed, zero failed, two ignored
  documentation examples, both without data and with the owned installation.
  Without data, corpus tests intentionally return early. In the owned-install
  run the directory was present and there were no `skipping ... TAK_DATA_DIR`
  messages; archive/content/script/map/replay corpus work actually ran.
- Separate client formatting, all-target Clippy with warnings denied, and
  executable build pass.
- Full client suite: 139 reported passed, zero failed, both without data and
  with the owned installation. The eight install-gated cases return early in
  the no-data run; in the owned-data run they exercise skirmish/campaign
  checkpoint continuation, replay, campaign scripts, training, fog targeting,
  session recordings, heal artwork and weapon artwork.
- The 23 dependency-notice collector tests pass; no dependencies were added.
- New coverage includes original left-click/drag/double-click behavior,
  minimap explicit targets and same-poll modifiers, simulated-pad release/
  cancel/motion/reticle/recording paths, browser held gestures and modal close,
  default-Cancel prompts, cancelled deferred operations, corrupt/changed files,
  pending-order refusal, and failed scene preparation preserving the live match.

## Real-window observations

On the cloud Linux desktop, the built client ran against the owned installation
using Mesa llvmpipe software Vulkan. This is a software-rendered GUI check, not
hardware/GPU or audio certification (the desktop has no audio device). Software
GPU dependencies remain outside the source tree and are not shipped.

Mouse/keyboard checks observed:

- Original-mode left-click on the monarch's ground ring selects it; left-click
  ground issues movement, with a visible route, walking displacement and Moving
  status. The context prompts now label LMB rather than the default RMB.
- F8 opens a paused checkpoint browser; saving shows map, faction, elapsed time,
  executed-order count and gameplay/state fingerprints.
- Load starts with Cancel selected. Enter on that default returns to browsing;
  explicit confirmation restores the saved tick and leaves the new scene paused.
  Close/Escape resumes it. Camera/selection/explored-fog reset is disclosed.
- Overwrite cancellation leaves the checkpoint file's SHA-256 unchanged.
- A deliberately malformed external test checkpoint remains visible with a
  parse error rather than crashing or replacing the match.
- Legacy F10 creates a separate verified replay checkpoint.

The first GUI pass exposed a launcher click becoming an unintended world order.
The cause was non-recursive tuple chaining in the production Bevy schedule.
Production and adapter tests now share explicit browser → settings → gameplay
ordering, and selected-unit launcher press/release/cancel is covered for all
three profiles. In the final real-window recheck, clicking the launcher with a
selected monarch and saving produced a tick-804 checkpoint with zero executed
orders, no pending-order refusal, and the monarch still Idle. Mouse close
resumed the match without a launcher-directed route.

Controller evidence is separate: the asset-free tests inject Bevy Gamepad
button/axis state into the actual adapter, including held buttons and modal
release/cancel. No physical controller or haptic/audio output was tested.
