//! Property: the default controller and mouse/keyboard profiles produce the
//! same sim commands (and end with the same selection and order mode) for
//! the same sequence of player intents.
//!
//! Each intent is driven twice: once as gamepad events (stick moves at full
//! deflection in 45 px steps, magnetism to finish onto units, wheels for
//! orders) and once as mouse/keyboard events (absolute pointer, hotkeys).

mod common;

use common::{TestWorld, WorldSpec, tick_at};
use proptest::prelude::*;
use tak_commands::SimCommand;
use tak_input::{
    Axis, Button, EventKind, InputEvent, Key, Mapper, Output, Profile, slot_direction,
};
use tak_math::{Fx, Vec2};

const STEP_PX: i32 = 45; // 900 px/s for 50 ms
const VIEW_W: i32 = 1280;
const VIEW_H: i32 = 960;

fn world() -> TestWorld {
    let spec: WorldSpec = serde_json::from_str(
        r#"{
        "local_player": 0,
        "enemies": [1],
        "units": [
          {"index": 1, "owner": 0, "def": 10, "x": 200, "y": 200, "builder": "mobile", "build_options": [20, 21, 22], "idle": true},
          {"index": 2, "owner": 0, "def": 11, "x": 400, "y": 200},
          {"index": 3, "owner": 0, "def": 11, "x": 600, "y": 200},
          {"index": 4, "owner": 0, "def": 13, "x": 800, "y": 200, "spells": [7]},
          {"index": 5, "owner": 0, "def": 12, "x": 1000, "y": 200, "builder": "factory", "build_options": [30, 31]},
          {"index": 6, "owner": 0, "def": 10, "x": 200, "y": 500, "builder": "mobile", "build_options": [21, 23], "idle": true},
          {"index": 7, "owner": 0, "def": 11, "x": 290, "y": 500},
          {"index": 8, "owner": 1, "def": 50, "x": 600, "y": 500},
          {"index": 9, "owner": 1, "def": 51, "x": 800, "y": 500},
          {"index": 10, "owner": 1, "def": 50, "x": 1000, "y": 700}
        ]
    }"#,
    )
    .unwrap();
    TestWorld::new(spec)
}

const OWN: [usize; 7] = [0, 1, 2, 3, 4, 5, 6];
const ENEMY: [usize; 3] = [7, 8, 9];

#[derive(Clone, Debug)]
enum Intent {
    Select(usize),
    Brush(usize, u8),
    Move(i8, i8, bool),
    Attack(usize),
    Stop,
    Fire(u8),
    Patrol(i8, i8),
    Guard(usize),
    Build(u8, i8, i8),
    Cast(usize),
    Assign(u8),
    Recall(u8),
    Cancel,
    SameType(bool),
}

fn intent() -> impl Strategy<Value = Intent> {
    let step = -4i8..=4;
    prop_oneof![
        3 => (0..OWN.len()).prop_map(Intent::Select),
        2 => (0..OWN.len(), 0u8..20).prop_map(|(u, k)| Intent::Brush(u, k)),
        3 => (step.clone(), step.clone(), any::<bool>()).prop_map(|(x, y, a)| Intent::Move(x, y, a)),
        2 => (0..ENEMY.len()).prop_map(Intent::Attack),
        1 => Just(Intent::Stop),
        1 => (0u8..3).prop_map(Intent::Fire),
        1 => (step.clone(), step.clone()).prop_map(|(x, y)| Intent::Patrol(x, y)),
        1 => (0..10usize).prop_map(Intent::Guard),
        2 => (0u8..4, step.clone(), step).prop_map(|(i, x, y)| Intent::Build(i, x, y)),
        1 => (0..10usize).prop_map(Intent::Cast),
        1 => (1u8..=4).prop_map(Intent::Assign),
        1 => (1u8..=4).prop_map(Intent::Recall),
        1 => Just(Intent::Cancel),
        1 => any::<bool>().prop_map(Intent::SameType),
    ]
}

/// One device's mapper plus its clock and collected sim commands.
struct Rig<'w> {
    m: Mapper,
    w: &'w TestWorld,
    t: u32,
    sims: Vec<SimCommand>,
}

impl<'w> Rig<'w> {
    fn new(profile: Profile, w: &'w TestWorld) -> Rig<'w> {
        Rig {
            m: Mapper::new(profile),
            w,
            t: 0,
            sims: Vec::new(),
        }
    }
    fn ev(&mut self, kind: EventKind) {
        let e = InputEvent::new(self.t, kind);
        for o in self.m.handle(&e, self.w, tick_at(self.t)) {
            if let Output::Sim { command, .. } = o {
                self.sims.push(command);
            }
        }
    }
    fn wait(&mut self, ms: u32) {
        self.t += ms;
    }
    fn down(&mut self, b: Button) {
        self.ev(EventKind::ButtonDown { button: b });
    }
    fn up(&mut self, b: Button) {
        self.ev(EventKind::ButtonUp { button: b });
    }
    fn tap(&mut self, b: Button) {
        self.down(b);
        self.wait(50);
        self.up(b);
        self.wait(50);
    }
    fn key_down(&mut self, k: &str) {
        self.ev(EventKind::KeyDown { key: Key::new(k) });
    }
    fn key_up(&mut self, k: &str) {
        self.ev(EventKind::KeyUp { key: Key::new(k) });
    }
    fn key(&mut self, k: &str) {
        self.key_down(k);
        self.wait(30);
        self.key_up(k);
        self.wait(30);
    }
    fn chord_key(&mut self, m: &str, k: &str) {
        self.key_down(m);
        self.wait(20);
        self.key(k);
        self.key_up(m);
        self.wait(20);
    }
    fn axis(&mut self, axis: Axis, value: i16) {
        self.ev(EventKind::Axis { axis, value });
    }

    // ---- gamepad helpers ----

    fn steps(&mut self, axis: Axis, n: i32) {
        if n == 0 {
            return;
        }
        self.axis(axis, if n > 0 { 32767 } else { -32767 });
        for _ in 0..n.abs() {
            self.wait(50);
            self.ev(EventKind::Frame);
        }
        self.axis(axis, 0);
        self.wait(20);
    }

    fn lattice_steps(d: Fx) -> i32 {
        let unit = STEP_PX * Fx::ONE_RAW;
        let raw = d.raw();
        (raw + raw.signum() * unit / 2) / unit
    }

    /// Move the virtual cursor: whole steps, then idle frames so magnetism
    /// finishes onto a unit. Panics if the target is not reached.
    fn pad_goto(&mut self, target: Vec2) {
        let c = self.m.cursor_screen();
        self.steps(Axis::LeftX, Self::lattice_steps(target.x - c.x));
        // Screen +y is down; stick +y is up.
        self.steps(Axis::LeftY, -Self::lattice_steps(target.y - c.y));
        for _ in 0..30 {
            if self.m.cursor_screen() == target {
                break;
            }
            self.wait(50);
            self.ev(EventKind::Frame);
        }
        assert_eq!(
            self.m.cursor_screen(),
            target,
            "pad cursor did not reach target"
        );
    }

    fn pad_wheel(&mut self, trigger: Button, slot: usize, n: usize) {
        self.down(trigger);
        self.wait(20);
        let (s, c) = slot_direction(slot, n).sin_cos();
        let scale = |v: Fx| ((i64::from(v.raw()) * 32767) >> 16) as i16;
        self.axis(Axis::LeftX, scale(c));
        self.axis(Axis::LeftY, scale(s));
        self.wait(20);
        self.up(trigger);
        self.axis(Axis::LeftX, 0);
        self.axis(Axis::LeftY, 0);
        self.wait(20);
    }

    /// Resolve pending taps without a frame (frames trigger magnetism).
    fn settle(&mut self) {
        self.wait(400);
        self.axis(Axis::RightX, 0);
    }

    // ---- mouse helpers ----

    fn point(&mut self, p: Vec2) {
        self.ev(EventKind::PointerMove {
            x: p.x.to_int_floor(),
            y: p.y.to_int_floor(),
        });
        self.wait(10);
    }

    fn click(&mut self, b: Button) {
        self.tap(b);
    }
}

fn unit_screen(w: &TestWorld, idx: usize) -> Vec2 {
    let u = &w.spec.units[idx];
    Vec2::from_ints(u.x, u.y)
}

fn lattice_target(from: Vec2, dx: i8, dy: i8) -> Option<Vec2> {
    let x = from.x.to_int_floor() + i32::from(dx) * STEP_PX;
    let y = from.y.to_int_floor() + i32::from(dy) * STEP_PX;
    ((0..VIEW_W).contains(&x) && (0..VIEW_H).contains(&y)).then(|| Vec2::from_ints(x, y))
}

const WHEEL_N: usize = 8;

fn drive(pad: &mut Rig<'_>, mk: &mut Rig<'_>, intent: &Intent) {
    let w = pad.w;
    let gpad = |n: u8| match n {
        1 => Button::DpadUp,
        2 => Button::DpadRight,
        3 => Button::DpadDown,
        _ => Button::DpadLeft,
    };
    match *intent {
        Intent::Select(i) => {
            let p = unit_screen(w, OWN[i]);
            pad.pad_goto(p);
            pad.tap(Button::A);
            mk.point(p);
            mk.click(Button::MouseLeft);
        }
        Intent::Brush(i, k) => {
            let p = unit_screen(w, OWN[i]);
            pad.pad_goto(p);
            pad.down(Button::A);
            pad.wait(250 + 25 * u32::from(k));
            pad.ev(EventKind::Frame);
            pad.up(Button::A);
            pad.wait(50);
            let r = 16 + 4 * i32::from(k);
            mk.point(p - Vec2::from_ints(r, 0));
            mk.down(Button::MouseLeft);
            mk.wait(30);
            mk.point(p + Vec2::from_ints(r, 0));
            mk.up(Button::MouseLeft);
            mk.wait(30);
        }
        Intent::Move(dx, dy, append) => {
            let Some(p) = lattice_target(pad.m.cursor_screen(), dx, dy) else {
                return;
            };
            pad.pad_goto(p);
            mk.point(p);
            if append {
                pad.down(Button::Rb);
                pad.wait(20);
                pad.tap(Button::X);
                pad.up(Button::Rb);
                mk.key_down("shift");
                mk.click(Button::MouseRight);
                mk.key_up("shift");
            } else {
                pad.tap(Button::X);
                mk.click(Button::MouseRight);
            }
        }
        Intent::Attack(i) => {
            let p = unit_screen(w, ENEMY[i]);
            pad.pad_goto(p);
            pad.tap(Button::X);
            mk.point(p);
            mk.click(Button::MouseRight);
        }
        Intent::Stop => {
            pad.pad_wheel(Button::Lt, 0, WHEEL_N);
            mk.key("s");
        }
        Intent::Fire(f) => {
            pad.pad_wheel(Button::Lt, 3 + usize::from(f), WHEEL_N);
            mk.key(["h", "r", "f"][usize::from(f)]);
        }
        Intent::Patrol(dx, dy) => {
            let Some(p) = lattice_target(pad.m.cursor_screen(), dx, dy) else {
                return;
            };
            pad.pad_wheel(Button::Lt, 1, WHEEL_N);
            pad.pad_goto(p);
            pad.tap(Button::X);
            mk.key("p");
            mk.point(p);
            mk.click(Button::MouseRight);
        }
        Intent::Guard(i) => {
            let p = unit_screen(w, i);
            pad.pad_wheel(Button::Lt, 2, WHEEL_N);
            pad.pad_goto(p);
            pad.tap(Button::X);
            mk.key("g");
            mk.point(p);
            mk.click(Button::MouseRight);
        }
        Intent::Cast(i) => {
            let p = unit_screen(w, i);
            pad.pad_wheel(Button::Lt, 6, WHEEL_N);
            pad.pad_goto(p);
            pad.tap(Button::X);
            mk.key("c");
            mk.point(p);
            mk.click(Button::MouseRight);
        }
        Intent::Build(idx, dx, dy) => {
            let Some(p) = lattice_target(pad.m.cursor_screen(), dx, dy) else {
                return;
            };
            // Gamepad: open the build wheel and aim at the slot if it exists.
            pad.down(Button::Lb);
            pad.wait(20);
            let n = pad.m.wheel().map_or(0, |w| w.slots.len());
            if usize::from(idx) < n {
                let (s, c) = slot_direction(usize::from(idx), n).sin_cos();
                let scale = |v: Fx| ((i64::from(v.raw()) * 32767) >> 16) as i16;
                pad.axis(Axis::LeftX, scale(c));
                pad.axis(Axis::LeftY, scale(s));
                pad.wait(20);
            }
            pad.up(Button::Lb);
            pad.axis(Axis::LeftX, 0);
            pad.axis(Axis::LeftY, 0);
            pad.wait(20);
            pad.pad_goto(p);
            pad.tap(Button::X);
            mk.key(&format!("f{}", idx + 1));
            mk.point(p);
            mk.click(Button::MouseRight);
        }
        Intent::Assign(g) => {
            pad.down(Button::Rb);
            pad.wait(20);
            pad.tap(gpad(g));
            pad.up(Button::Rb);
            pad.wait(20);
            mk.chord_key("ctrl", &g.to_string());
        }
        Intent::Recall(g) => {
            pad.tap(gpad(g));
            mk.key(&g.to_string());
        }
        Intent::Cancel => {
            pad.tap(Button::B);
            mk.key("escape");
        }
        Intent::SameType(on_screen) => {
            if on_screen {
                pad.tap(Button::Y);
                mk.key("z");
            } else {
                pad.tap(Button::Y);
                pad.tap(Button::Y);
                mk.chord_key("ctrl", "z");
            }
        }
    }
    pad.settle();
    mk.settle();
}

fn check(intents: &[Intent]) -> Vec<SimCommand> {
    let w = world();
    let mut pad = Rig::new(Profile::wp_j_controller(), &w);
    let mut mk = Rig::new(common::wp_j_mk(), &w);
    for (n, i) in intents.iter().enumerate() {
        drive(&mut pad, &mut mk, i);
        assert_eq!(
            pad.sims, mk.sims,
            "sim commands diverged after intent {n}: {i:?}"
        );
        assert_eq!(
            pad.m.selection(),
            mk.m.selection(),
            "selection diverged after intent {n}: {i:?}"
        );
        assert_eq!(
            pad.m.mode(),
            mk.m.mode(),
            "mode diverged after intent {n}: {i:?}"
        );
    }
    pad.sims
}

#[test]
fn scripted_intents_match_and_issue_orders() {
    use Intent::*;
    let script = [
        Select(1),
        Brush(5, 19), // u6 + u7
        Move(2, -1, false),
        Move(1, 1, true),
        Attack(0),
        Assign(2),
        Cancel,
        Recall(2),
        Fire(0),
        Patrol(-2, 0),
        Stop,
        Select(0),
        Build(1, 3, 2),
        Select(4),
        Build(0, 0, 0), // factory: immediate
        Select(3),
        Cast(8),
        Guard(1),
        SameType(false),
        SameType(true),
    ];
    let sims = check(&script);
    let kinds: Vec<&str> = sims
        .iter()
        .map(|c| match c {
            SimCommand::Move { .. } => "move",
            SimCommand::AttackMove { .. } => "attack_move",
            SimCommand::Attack { .. } => "attack",
            SimCommand::Build { at: Some(_), .. } => "place",
            SimCommand::Build { at: None, .. } => "produce",
            SimCommand::Guard { .. } => "guard",
            SimCommand::Heal { .. } => "heal",
            SimCommand::Patrol { .. } => "patrol",
            SimCommand::CastSpell { .. } => "cast",
            SimCommand::Stop { .. } => "stop",
            SimCommand::SetFireState { .. } => "fire",
            SimCommand::SetFactoryRally { .. } => "rally",
            SimCommand::CancelProduction { .. } => "cancel_production",
            SimCommand::SetFactoryRepeat { .. } => "set_factory_repeat",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "move", "move", "attack", "fire", "patrol", "stop", "place", "produce",
            // The factory build intent's trailing X sets a rally point.
            "rally", "cast", "guard"
        ]
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    #[test]
    fn controller_and_mouse_keyboard_agree(intents in proptest::collection::vec(intent(), 1..16)) {
        check(&intents);
    }
}
