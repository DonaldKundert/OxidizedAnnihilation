//! Absolute map pointers retain off-camera orders without changing the view.
mod common;
use common::{TestWorld, pos};
use tak_commands::Tick;
use tak_input::{Axis, EventKind, InputEvent, Mapper, Profile};

fn world() -> TestWorld {
    TestWorld::new(serde_json::from_str(r#"{"local_player":0,"enemies":[],"units":[]}"#).unwrap())
}

fn map_event() -> EventKind {
    EventKind::MapPointer {
        x: 0,
        y: 0,
        world: pos(12000, 9000),
    }
}

#[test]
fn map_pointer_round_trips_and_stays_absolute_across_camera_updates_without_edge_pan() {
    let legacy: InputEvent =
        serde_json::from_str(r#"{"t":0,"type":"pointer_move","x":30,"y":40}"#).unwrap();
    assert_eq!(legacy.kind, EventKind::PointerMove { x: 30, y: 40 });
    let event = InputEvent::new(0, map_event());
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<InputEvent>(&json).unwrap(), event);
    let mut profile = Profile::classic_tak();
    profile.tuning.edge_pan_mouse = true;
    let mut mapper = Mapper::new(profile);
    let world = world();
    mapper.handle(&event, &world, Tick(0));
    let mut camera = *mapper.camera();
    camera.origin = pos(300, 400);
    mapper.set_camera(camera);
    mapper.handle(&InputEvent::new(1000, EventKind::Frame), &world, Tick(30));
    assert_eq!(mapper.cursor_world(), pos(12000, 9000));
    assert_eq!(mapper.take_camera_pan(), pos(0, 0));
    assert_eq!(*mapper.camera(), camera);
}

#[test]
fn terrain_pointer_and_stick_clear_map_override_while_profile_keeps_cursor() {
    let world = world();
    for kind in [
        EventKind::PointerMove { x: 30, y: 40 },
        EventKind::Axis {
            axis: Axis::LeftX,
            value: 32767,
        },
    ] {
        let mut mapper = Mapper::new(Profile::classic_tak());
        mapper.handle(&InputEvent::new(0, map_event()), &world, Tick(0));
        mapper.handle(&InputEvent::new(1, kind), &world, Tick(0));
        assert_ne!(mapper.cursor_world(), pos(12000, 9000));
    }
    let mut mapper = Mapper::new(Profile::classic_tak());
    mapper.handle(&InputEvent::new(0, map_event()), &world, Tick(0));
    mapper.set_profile(Profile::classic_tak());
    assert_eq!(mapper.cursor_world(), pos(12000, 9000));
}
