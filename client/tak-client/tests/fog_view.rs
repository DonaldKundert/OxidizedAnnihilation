//! The input layer sees only what the local player sees (gated on
//! `TAK_DATA_DIR`): an enemy outside current sight is not in the
//! `WorldView` at all, so picking, the aim-assist snap and context orders
//! ignore it; an order onto the ground where it stands is a move.

use tak_client::fog::FogGrid;
use tak_client::game::{Session, open_vfs};
use tak_commands::{CommandLog, QueueMode, SimCommand, UnitSet};
use tak_input::{
    Camera, EventKind, InputEvent, Mapper, Mode, Profile, WorldView, pick_unit, resolve_context,
    units_sorted,
};
use tak_math::{Fx, Vec2};

#[test]
fn hidden_enemies_are_not_picked_snapped_or_attacked() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("fog view") else {
        return;
    };
    let vfs = open_vfs(&dir).expect("open install");
    let mut s = Session::load(
        &vfs,
        "kmap/Adamantine Gate",
        "ARA,ZON",
        1,
        CommandLog::new(),
    )
    .expect("load");
    s.step(); // tick 0 spawns the monarchs
    let w = s.sim.world();
    let me = s.local;
    let (own, own_pos) = w
        .units()
        .find(|(_, u)| u.owner == me)
        .map(|(id, u)| (id, u.pos))
        .expect("own monarch");
    let (foe, foe_pos) = w
        .units()
        .find(|(_, u)| u.owner != me)
        .map(|(id, u)| (id, u.pos))
        .expect("enemy monarch");
    // The local player sees only around its own monarch (the monarchs start
    // far apart on this map).
    let mut fog = FogGrid::new(512, 512);
    let f = |v: Fx| v.to_int_round() as f32;
    fog.update(&[(f(own_pos.x), f(own_pos.y), 300.0)]);
    let open = s.view();
    let fogged = s.view_fogged(Some(&fog));
    assert!(open.unit(foe).is_some(), "without fog the enemy is there");
    assert!(
        fogged.unit(foe).is_none(),
        "the hidden enemy is not in the view"
    );
    assert!(fogged.unit(own).is_some(), "own units always are");
    assert!(units_sorted(&fogged).iter().all(|u| u.id != foe));
    // Picking at the enemy finds nothing; a context order there is a move.
    assert_eq!(pick_unit(&fogged, foe_pos).map(|u| u.id), None);
    assert_eq!(pick_unit(&open, foe_pos).map(|u| u.id), Some(foe));
    let units = UnitSet::single(own);
    let hovered = pick_unit(&fogged, foe_pos);
    let order = resolve_context(
        &fogged,
        &units,
        foe_pos,
        hovered.as_ref(),
        Mode::None,
        QueueMode::Replace,
    );
    assert!(matches!(order, Some(SimCommand::Move { .. })), "{order:?}");
    // The aim-assist snap does not pull toward it: own monarch selected,
    // free cursor parked a few pixels off the hidden enemy.
    let mut p = Profile::wp_s_controller();
    p.tuning.reticle = false;
    let run = |view: &dyn WorldView| {
        let mut m = Mapper::new(p.clone());
        let cam = Camera {
            origin: foe_pos - Vec2::from_ints(320, 240),
            world_per_px: Fx::ONE,
            width_px: 640,
            height_px: 480,
        };
        m.set_camera(cam);
        // Select the own units through the mapper.
        let _ = m.perform(
            tak_input::Action::SelectAll {
                scope: tak_commands::Scope::Map,
            },
            view,
            tak_commands::Tick(1),
        );
        m.warp_cursor(Vec2::from_ints(320 + 20, 240));
        for t in 0..20u32 {
            m.handle(
                &InputEvent::new(1000 + t * 16, EventKind::Frame),
                view,
                tak_commands::Tick(1),
            );
        }
        m.snap_target()
    };
    assert_eq!(run(&open), Some(foe), "visible: the snap takes it");
    assert_eq!(run(&fogged), None, "hidden: no snap");
}
