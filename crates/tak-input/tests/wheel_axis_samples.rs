//! Device polling emits X and Y separately with one timestamp. A wheel must
//! steer from that coherent sample, not latch an intermediate cardinal angle.
mod common;

use common::{TestWorld, WorldSpec};
use tak_commands::{Scope, Tick};
use tak_input::{Action, Axis, Button, EventKind, InputEvent, Mapper, Profile, WheelKind};

fn rig(recentre_clears: bool) -> (Mapper, TestWorld) {
    let world = TestWorld::new(serde_json::from_str::<WorldSpec>(
        r#"{"local_player":0,"enemies":[1],"units":[{"index":1,"owner":0,"def":1,"x":640,"y":480}]}"#,
    ).unwrap());
    let mut profile = Profile::kingdoms_console();
    profile.tuning.wheel_recentre_clears = recentre_clears;
    let mut mapper = Mapper::new(profile);
    mapper.perform(Action::SelectAll { scope: Scope::Map }, &world, Tick(0));
    (mapper, world)
}

fn feed(m: &mut Mapper, w: &TestWorld, t: u32, kind: EventKind) {
    m.handle(&InputEvent::new(t, kind), w, Tick(t * 30 / 1000));
}

fn axes(m: &mut Mapper, w: &TestWorld, t: u32, x: i16, y: i16, reverse: bool) {
    let mut pair = [(Axis::LeftX, x), (Axis::LeftY, y)];
    if reverse {
        pair.reverse();
    }
    for (axis, value) in pair {
        feed(m, w, t, EventKind::Axis { axis, value });
    }
}

#[test]
fn diagonal_flick_then_neutral_opens_stance_in_both_axis_orders() {
    for reverse in [false, true] {
        let (mut m, w) = rig(false);
        feed(
            &mut m,
            &w,
            100,
            EventKind::ButtonDown { button: Button::Lt },
        );
        axes(&mut m, &w, 200, 32767, -32767, reverse);
        assert_eq!(m.wheel().unwrap().highlighted, Some(3));
        axes(&mut m, &w, 300, 0, 0, reverse);
        assert_eq!(m.wheel().unwrap().highlighted, Some(3), "reverse={reverse}");
        feed(&mut m, &w, 400, EventKind::ButtonUp { button: Button::Lt });
        assert_eq!(m.wheel().unwrap().kind, WheelKind::Stance);
    }
}

#[test]
fn neutral_sample_preserves_explicit_clear_setting_and_dead_centre_cancel() {
    for reverse in [false, true] {
        let (mut m, w) = rig(true);
        feed(
            &mut m,
            &w,
            100,
            EventKind::ButtonDown { button: Button::Lt },
        );
        axes(&mut m, &w, 200, 32767, -32767, reverse);
        axes(&mut m, &w, 300, 0, 0, reverse);
        assert_eq!(m.wheel().unwrap().highlighted, None);
        feed(&mut m, &w, 400, EventKind::ButtonUp { button: Button::Lt });
        assert!(m.wheel().is_none());
        feed(
            &mut m,
            &w,
            600,
            EventKind::ButtonDown { button: Button::Lt },
        );
        feed(&mut m, &w, 700, EventKind::ButtonUp { button: Button::Lt });
        assert!(m.wheel().is_none());
    }
}

#[test]
fn sampled_axes_match_atomic_steering_across_slots_hysteresis_and_centre() {
    let directions = [
        (0, 32767),
        (23170, 23170),
        (32767, 0),
        (23170, -23170),
        (0, -32767),
        (-23170, -23170),
        (-32767, 0),
        (-23170, 23170),
        (0, 0),
        (12000, 12000),
        (30000, 11000),
        (30000, 16000),
    ];
    for clear in [false, true] {
        for &(from_x, from_y) in &directions {
            for &(to_x, to_y) in &directions {
                for reverse in [false, true] {
                    let (mut m, w) = rig(clear);
                    feed(
                        &mut m,
                        &w,
                        100,
                        EventKind::ButtonDown { button: Button::Lt },
                    );
                    axes(&mut m, &w, 200, from_x, from_y, false);
                    let mut expected = m.wheel().unwrap().clone();
                    let tuning = *m.tuning();
                    expected.steer_with(
                        tak_input::stick_vector(to_x, to_y, tuning.wheel_deadzone),
                        tuning.wheel_hysteresis_deg,
                        clear,
                    );
                    axes(&mut m, &w, 300, to_x, to_y, reverse);
                    assert_eq!(
                        m.wheel().unwrap(),
                        &expected,
                        "from=({from_x},{from_y}), to=({to_x},{to_y}), reverse={reverse}, clear={clear}"
                    );
                }
            }
        }
    }
}

#[test]
fn separate_samples_and_non_axis_events_keep_immediate_steering() {
    let (mut m, w) = rig(false);
    feed(
        &mut m,
        &w,
        100,
        EventKind::ButtonDown { button: Button::Lt },
    );
    axes(&mut m, &w, 200, 32767, -32767, false);
    feed(
        &mut m,
        &w,
        300,
        EventKind::Axis {
            axis: Axis::LeftX,
            value: 0,
        },
    );
    assert_eq!(m.wheel().unwrap().highlighted, Some(4));
    // A distinct sample really did point south; neutral should retain south.
    feed(
        &mut m,
        &w,
        301,
        EventKind::Axis {
            axis: Axis::LeftY,
            value: 0,
        },
    );
    assert_eq!(m.wheel().unwrap().highlighted, Some(4));
    // A frame also delimits an axis batch even at the same timestamp.
    axes(&mut m, &w, 400, 32767, -32767, false);
    feed(
        &mut m,
        &w,
        500,
        EventKind::Axis {
            axis: Axis::LeftX,
            value: 0,
        },
    );
    feed(&mut m, &w, 500, EventKind::Frame);
    feed(
        &mut m,
        &w,
        500,
        EventKind::Axis {
            axis: Axis::LeftY,
            value: 0,
        },
    );
    assert_eq!(m.wheel().unwrap().highlighted, Some(4));
}
