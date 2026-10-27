//! Centre-cell occupancy for real-map fixture selection and collision assertions.

use tak_sim::map::terrain;
use tak_sim::{World, WorldPos};

/// Preserve the fixtures' original ground-cell test across both blocker planes.
/// Placed features and corpses are live counted obstacles, not immutable terrain
/// flags. This is not a movement-class footprint or structure-yard query.
pub fn blocked(world: &World, p: WorldPos) -> bool {
    let map = world.map();
    map.terrain_at(p) & terrain::BLOCKS != 0
        || map.terrain.as_ref().is_some_and(|t| {
            let (x, z) = t.cell_of(p);
            tak_sim::pathfinding::is_obstacle(world, x as i32, z as i32)
        })
}
