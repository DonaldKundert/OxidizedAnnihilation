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

#[test]
fn stick_surface_preserves_visible_cursor_source_and_restores_terrain() {
    let world = world();
    let mut mapper = Mapper::new(Profile::wp_s_combined());
    mapper.warp_cursor(pos(30, 40));
    let visible = mapper.cursor_world_unmapped();
    let kind = EventKind::StickMapPointer {
        world: Some(pos(12000, 9000)),
    };
    let event = InputEvent::new(0, kind);
    assert_eq!(
        serde_json::from_str::<InputEvent>(&serde_json::to_string(&event).unwrap()).unwrap(),
        event
    );
    mapper.handle(&event, &world, Tick(0));
    assert!(mapper.stick_cursor());
    assert_eq!(mapper.cursor_screen(), pos(30, 40));
    assert_eq!(mapper.cursor_world_unmapped(), visible);
    assert_eq!(mapper.cursor_world(), pos(12000, 9000));
    mapper.handle(&InputEvent::new(16, EventKind::Frame), &world, Tick(0));
    assert_eq!(mapper.cursor_screen(), pos(30, 40));
    mapper.handle(
        &InputEvent::new(16, EventKind::StickMapPointer { world: None }),
        &world,
        Tick(0),
    );
    assert_eq!(mapper.cursor_world(), visible);
    assert!(mapper.stick_cursor());
}

#[test]
fn stick_surface_event_does_not_take_control_from_mouse() {
    let world = world();
    let mut mapper = Mapper::new(Profile::wp_s_combined());
    mapper.handle(&InputEvent::new(0, map_event()), &world, Tick(0));
    for target in [None, Some(pos(100, 200))] {
        mapper.handle(
            &InputEvent::new(1, EventKind::StickMapPointer { world: target }),
            &world,
            Tick(0),
        );
        assert!(!mapper.stick_cursor());
        assert_eq!(mapper.cursor_world(), pos(12000, 9000));
    }
}

#[test]
fn recording_keeps_visible_ghost_cursor_and_exact_map_target_separate() {
    let world = world();
    let mut mapper = Mapper::new(Profile::wp_s_combined());
    mapper.warp_cursor(pos(30, 40));
    let visible = mapper.cursor_world_unmapped();
    let target = EventKind::StickMapPointer {
        world: Some(pos(12000, 9000)),
    };
    mapper.handle(&InputEvent::new(0, target.clone()), &world, Tick(0));
    let mut recorder = tak_input::Recorder::new(serde_json::json!({}), &mapper);
    recorder.frame(
        &mapper,
        0,
        0,
        tak_input::FrameIo {
            events: std::slice::from_ref(&target),
            ..Default::default()
        },
    );
    let recording = recorder.recording();
    let decoded =
        tak_input::Recording::from_json(&serde_json::to_string(recording).unwrap()).unwrap();
    assert_eq!(decoded.frames[0].events, [target]);
    assert_eq!(decoded.ghost_at(0).unwrap().cursor_world, visible);
    assert!(decoded.ghost_at(0).unwrap().stick);
    assert_eq!(mapper.cursor_world(), pos(12000, 9000));
}
