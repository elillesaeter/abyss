//! Breaking tiles with tools, the way Terraria does it: each swing takes the
//! tool's power off the tile's health, cracks show how far along it is, and a
//! tile left alone for a few seconds heals.

use std::collections::HashMap;

use crate::items::{Item, ToolKind};
use crate::tiles::{AIR, DIRT, STONE, TRUNK, TileId, is_breakable, tile_type};
use crate::world::World;

/// Seconds without a hit before a damaged tile is whole again.
pub const HEAL_AFTER: f32 = 4.0;
/// How many crack drawings there are, from a scratch to nearly broken.
pub const CRACK_STAGES: u32 = 3;

/// Damage on tiles that have been hit but not broken.
#[derive(Clone, Debug, Default)]
pub struct Wear(HashMap<[i32; 2], Hit>);

#[derive(Clone, Copy, Debug)]
struct Hit {
    lost: i32,
    since: f32,
}

impl Wear {
    /// Tiles heal once they have gone `HEAL_AFTER` seconds without a hit.
    pub fn update(&mut self, dt: f32) {
        for hit in self.0.values_mut() {
            hit.since += dt;
        }
        self.0.retain(|_, hit| hit.since < HEAL_AFTER);
    }

    /// Each damaged tile with its crack stage, 1 to `CRACK_STAGES`.
    pub fn cracks<'a>(&'a self, world: &'a World) -> impl Iterator<Item = ([i32; 2], u32)> + 'a {
        self.0.iter().filter_map(|(&at, hit)| {
            let health = tile_type(world.get(at[0], at[1])).health;
            (health > 0).then(|| {
                let stage = (hit.lost as f32 / health as f32 * CRACK_STAGES as f32).ceil() as u32;
                (at, stage.clamp(1, CRACK_STAGES))
            })
        })
    }
}

/// The tool that breaks `id`: trunks are chopped, other breakable tiles mined.
pub fn tool_for(id: TileId) -> Option<ToolKind> {
    if id == TRUNK {
        Some(ToolKind::Axe)
    } else if is_breakable(id) {
        Some(ToolKind::Pickaxe)
    } else {
        None
    }
}

/// What a broken tile leaves behind. A trunk leaves nothing itself: the tree
/// gives its wood when it lands.
pub fn drop_for(id: TileId) -> Option<Item> {
    match id {
        DIRT => Some(Item::Dirt),
        STONE => Some(Item::Stone),
        _ => None,
    }
}

/// One swing of a `kind` tool of `power` at the tile `at`. Returns the tile it
/// broke, which is now air; the wrong tool, or a tile a tree stands on, takes
/// no damage.
pub fn hit(world: &mut World, at: [i32; 2], kind: ToolKind, power: i32) -> Option<TileId> {
    let id = world.get(at[0], at[1]);
    if tool_for(id) != Some(kind) || world.forest.anchors(at[0], at[1]) {
        return None;
    }
    let hit = world.wear.0.entry(at).or_insert(Hit { lost: 0, since: 0.0 });
    hit.lost += power;
    hit.since = 0.0;
    if hit.lost < tile_type(id).health {
        return None;
    }
    world.wear.0.remove(&at);
    world.set(at[0], at[1], AIR);
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with(id: TileId) -> World {
        let mut w = World::new(32, 32);
        w.set(5, 5, id);
        w
    }

    #[test]
    fn dirt_takes_two_pickaxe_hits_and_stone_three() {
        for (id, hits) in [(DIRT, 2), (STONE, 3)] {
            let mut world = world_with(id);
            for _ in 1..hits {
                assert_eq!(hit(&mut world, [5, 5], ToolKind::Pickaxe, 35), None);
            }
            assert_eq!(hit(&mut world, [5, 5], ToolKind::Pickaxe, 35), Some(id));
            assert_eq!(world.get(5, 5), AIR);
        }
    }

    #[test]
    fn the_wrong_tool_does_nothing() {
        let mut world = world_with(DIRT);
        for _ in 0..5 {
            assert_eq!(hit(&mut world, [5, 5], ToolKind::Axe, 35), None);
        }
        assert_eq!(world.get(5, 5), DIRT);
        assert_eq!(world.wear.cracks(&world).count(), 0, "not even a crack");
    }

    #[test]
    fn cracks_deepen_and_heal_when_left_alone() {
        let mut world = world_with(STONE);
        hit(&mut world, [5, 5], ToolKind::Pickaxe, 35);
        assert_eq!(world.wear.cracks(&world).collect::<Vec<_>>(), vec![([5, 5], 2)]);
        world.wear.update(HEAL_AFTER * 0.9);
        hit(&mut world, [5, 5], ToolKind::Pickaxe, 35);
        assert_eq!(
            world.wear.cracks(&world).next().unwrap().1,
            3,
            "a fresh hit restarts the clock"
        );
        world.wear.update(HEAL_AFTER * 0.9);
        assert_eq!(world.wear.cracks(&world).count(), 1);
        world.wear.update(HEAL_AFTER * 0.2);
        assert_eq!(world.wear.cracks(&world).count(), 0, "healed");
        hit(&mut world, [5, 5], ToolKind::Pickaxe, 35);
        assert_eq!(world.get(5, 5), STONE, "healed stone takes three hits again");
    }
}
