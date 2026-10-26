//! Match setup must not choose an army unable to target the opposing monarch.
use tak_client::skirmish::{self, Faction, Opponent, SetupConfig};

#[test]
fn installed_training_recipe_can_target_the_opposing_monarch() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("training_target_compatibility") else {
        return;
    };
    for human in [Faction::Aramon, Faction::Zhon] {
        let session = skirmish::prepare_session(
            &data,
            &SetupConfig {
                faction: human,
                opponent: Opponent::Training,
                ..Default::default()
            },
        )
        .unwrap();
        let world = session.sim.world();
        let king = world.defs().get(world.players()[0].monarch_def).unwrap();
        let config = session.controllers();
        let name = &config.controllers[0].recipe.troop.name;
        let troop = world
            .defs()
            .as_slice()
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .unwrap();
        eprintln!(
            "training troop {} against monarch {} (flies={}): {} compatible ordinary slots",
            troop.name,
            king.name,
            king.motion.canfly,
            troop
                .combat
                .weapons
                .iter()
                .filter(|w| !w.is_spell()
                    && w.damage.default > 0
                    && (!king.motion.canfly || !w.no_air_weapon))
                .count()
        );
        assert!(
            troop.combat.weapons.iter().any(|w| !w.is_spell()
                && w.damage.default > 0
                && (!king.motion.canfly || !w.no_air_weapon)),
            "training recipe {} cannot target {}",
            troop.name,
            king.name
        );
    }
}
