//! Fighting: the sword's sweep against mobs, mobs touching the player, cubes
//! bursting into drops, drops being picked up, and the damage numbers that
//! float off each hit.

use crate::atlas::{CELL, FRAME_H, FRAME_W};
use crate::character;
use crate::entity::{Body, Entities, Kind};
use crate::items::{Item, Stack};
use crate::mob::{self, Contents, MobState};
use crate::physics;
use crate::player::{self, Player};
use crate::world::World;

/// A hit's upward throw, as a share of the weapon's knockback.
const KNOCK_LIFT: f32 = 0.9;
const HIT_STUN: f32 = 0.3;
/// Drops this close drift toward the player, in tiles, at this speed.
const PULL_REACH: f32 = 4.0;
const PULL_SPEED: f32 = 7.0;

/// An item lying in the world, waiting to be picked up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drop {
    pub item: Item,
    pub count: u16,
    /// Seconds before it can be picked up: a thrown item isn't pulled straight
    /// back into the hand that threw it.
    pub wait: f32,
}

/// How long a thrown item lies before it can be picked up again.
pub const THROWN_WAIT: f32 = 1.5;

const DROP_HALF: [f32; 2] = [0.3, 0.3];

/// A number floating up off a hit and fading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Popup {
    pub pos: [f32; 2],
    pub value: i32,
    pub age: f32,
    /// Hits on the player read red; hits on mobs white.
    pub hurt_player: bool,
}

pub const POPUP_TIME: f32 = 0.9;
const POPUP_RISE: f32 = 2.0;

/// A frame pixel of the player's sprite, in the world.
fn frame_to_world(player: &Player, p: [f32; 2]) -> [f32; 2] {
    let at = player.visual_pos();
    let feet = at[1] + player::HALF[1];
    let frame = player.frame();
    let mirror = if frame.mirrored { -1.0 } else { 1.0 };
    [
        at[0] + mirror * (p[0] - FRAME_W as f32 * 0.5) / CELL as f32,
        feet + (p[1] + frame.upper_shift() - FRAME_H as f32) / CELL as f32,
    ]
}

/// Where a torch held in the hand burns, while one is held and the hand is free.
pub fn held_torch(player: &Player) -> Option<[f32; 2]> {
    let free = !player.busy() && !player.is_dead();
    (free && player.inventory.held() == Some(Item::Torch))
        .then(|| frame_to_world(player, character::hand(player.frame().index)))
}

/// Puts `count` of `item` into the world at `pos` with a little hop, nudged up
/// out of any block it would start inside.
pub fn spawn_drop(entities: &mut Entities, world: &World, mut pos: [f32; 2], item: Item, count: u16) {
    for _ in 0..3 {
        if !physics::overlaps_solid(world, pos, DROP_HALF) {
            break;
        }
        pos[1] -= 1.0;
    }
    let mut body = Body::new(pos, DROP_HALF);
    body.vel = [0.0, -6.0];
    entities.spawn((body, Drop { item, count, wait: 0.0 }));
}

/// The player throws `stack` out ahead of them.
pub fn throw(entities: &mut Entities, player: &Player, stack: Stack) {
    let mut body = Body::new([player.pos[0] + player.facing * 0.8, player.pos[1] - 0.5], DROP_HALF);
    body.vel = [player.facing * 8.0, -6.0];
    entities.spawn((
        body,
        Drop {
            item: stack.item,
            count: stack.count,
            wait: THROWN_WAIT,
        },
    ));
}

/// The hand `arc` of the way along a swing, and which way a held weapon points
/// from it, in the world.
pub fn grip(player: &Player, arc: f32) -> ([f32; 2], [f32; 2]) {
    let (hand, dir) = character::swing_hand(arc);
    let mirror = if player.frame().mirrored { -1.0 } else { 1.0 };
    (frame_to_world(player, hand), [dir[0] * mirror, dir[1]])
}

/// The weapon, hand to tip, at small steps along the arc swept so far. A mob
/// anywhere in it has been hit.
fn swept(player: &Player) -> Vec<([f32; 2], [f32; 2])> {
    let Some((weapon, now)) = player.wielded() else {
        return Vec::new();
    };
    let reach = weapon.reach() / CELL as f32;
    let steps = (now / 0.04).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|i| {
            let (hand, dir) = grip(player, now * i as f32 / steps as f32);
            (hand, [hand[0] + dir[0] * reach, hand[1] + dir[1] * reach])
        })
        .collect()
}

fn segment_hits_box(a: [f32; 2], b: [f32; 2], body: &Body) -> bool {
    (0..=8).any(|i| {
        let t = i as f32 / 8.0;
        let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        (p[0] - body.pos[0]).abs() <= body.half[0] && (p[1] - body.pos[1]).abs() <= body.half[1]
    })
}

fn overlap(a: [f32; 2], ha: [f32; 2], b: [f32; 2], hb: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < ha[0] + hb[0] && (a[1] - b[1]).abs() < ha[1] + hb[1]
}

/// One frame of fighting, after everything has moved.
pub fn update(world: &World, player: &mut Player, entities: &mut Entities, popups: &mut Vec<Popup>, dt: f32) {
    for popup in popups.iter_mut() {
        popup.age += dt;
        popup.pos[1] -= POPUP_RISE * dt * (1.0 - popup.age / POPUP_TIME).max(0.0);
    }
    popups.retain(|p| p.age < POPUP_TIME);

    // The weapon: each mob in the swept arc is hit once per swing.
    let blade = swept(player);
    let weapon = player.wielded().map(|w| w.0);
    let mut burst = Vec::new();
    for (entity, body, kind, state) in entities.query_mut::<(hecs::Entity, &mut Body, &Kind, &mut MobState)>() {
        let hit = weapon
            .filter(|_| state.last_swing != player.swing && blade.iter().any(|&(a, b)| segment_hits_box(a, b, body)));
        if let Some(weapon) = hit {
            state.last_swing = player.swing;
            state.health -= weapon.damage;
            state.stun = HIT_STUN;
            // Bigger cubes are heavier: thrown less far.
            let heft = (body.half[0] / 0.75).max(1.0);
            let away = if body.pos[0] >= player.pos[0] { 1.0 } else { -1.0 };
            body.vel = [away * weapon.knockback / heft, -weapon.knockback * KNOCK_LIFT / heft];
            popups.push(Popup {
                pos: [body.pos[0], body.pos[1] - body.half[1]],
                value: weapon.damage,
                age: 0.0,
                hurt_player: false,
            });
            if state.health <= 0 {
                burst.push(entity);
            }
            continue;
        }
        // Touching a mob hurts, unless just hurt.
        if !player.is_dead() && overlap(body.pos, body.half, player.pos, player::HALF) {
            let damage = mob::kind(kind.0).damage;
            if player.hurt(damage, body.pos[0]) {
                popups.push(Popup {
                    pos: [player.pos[0], player.pos[1] - player::HALF[1]],
                    value: damage,
                    age: 0.0,
                    hurt_player: true,
                });
            }
        }
    }
    for entity in burst {
        let pos = entities.get::<&Body>(entity).map(|b| b.pos).expect("a mob has a body");
        let carried = entities.get::<&Contents>(entity).ok().map(|c| *c);
        let _ = entities.despawn(entity);
        if let Some(c) = carried {
            spawn_drop(entities, world, pos, c.item, c.count);
        }
    }

    // Drops fall, and are picked up by walking over them.
    let mut collected = Vec::new();
    for (entity, body, drop) in entities.query_mut::<(hecs::Entity, &mut Body, &mut Drop)>() {
        let to = [player.pos[0] - body.pos[0], player.pos[1] - body.pos[1]];
        // Only what there is room for is drawn in, straight at the player, so
        // it climbs out of the pit it was mined from.
        let distance = to[0].hypot(to[1]);
        drop.wait = (drop.wait - dt).max(0.0);
        let pulled = !player.is_dead() && drop.wait == 0.0 && distance < PULL_REACH && player.inventory.fits(drop.item);
        if pulled {
            body.vel = [to[0] / distance * PULL_SPEED, to[1] / distance * PULL_SPEED];
        } else {
            body.vel[0] *= (-4.0 * dt).exp();
            body.vel[1] = (body.vel[1] + physics::GRAVITY * dt).min(physics::MAX_FALL);
        }
        physics::move_box(&mut body.pos, &mut body.vel, body.half, world, dt, false);
        if pulled && overlap(body.pos, body.half, player.pos, player::HALF) {
            drop.count = player.inventory.add(drop.item, drop.count);
            if drop.count == 0 {
                collected.push(entity);
            }
        }
    }
    for entity in collected {
        let _ = entities.despawn(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::STARTER_SWORD;
    use crate::player::Intent;
    use crate::tiles::STONE;

    const GROUND: i32 = 32;
    const DT: f32 = 1.0 / 60.0;

    fn flat_world() -> World {
        let mut w = World::new(64, 64);
        for x in 0..w.width() {
            for y in GROUND..w.height() {
                w.set(x, y, STONE);
            }
        }
        w
    }

    fn standing_player(world: &World) -> Player {
        let mut p = Player::new([20.0, GROUND as f32 - player::HALF[1]]);
        p.update(world, Intent::default(), DT);
        p
    }

    fn cube_at(entities: &mut Entities, id: u16, x: f32) -> hecs::Entity {
        let half = mob::kind(id).half[1];
        mob::spawn(entities, id, [x, GROUND as f32 - half], 1)
    }

    /// Runs fighting and the player for a while, mobs held still.
    fn run(world: &World, player: &mut Player, entities: &mut Entities, popups: &mut Vec<Popup>, seconds: f32) {
        for _ in 0..(seconds / DT) as i32 {
            player.update(world, Intent::default(), DT);
            update(world, player, entities, popups, DT);
        }
    }

    #[test]
    fn a_mined_block_drops_and_stays_put_when_the_inventory_is_full() {
        let mut world = flat_world();
        let (mut entities, mut popups) = (Entities::new(), Vec::new());
        let mut player = standing_player(&world);
        // A block dug out of the ground, spawned inside the stone it came from:
        // it pops up out of the stone rather than being stuck in it.
        world.set(21, GROUND, crate::tiles::AIR);
        spawn_drop(&mut entities, &world, [21.5, GROUND as f32 + 1.5], Item::Dirt, 1);
        for slot in player.inventory.storage() {
            player.inventory.cursor = Some(Stack {
                item: Item::Sword,
                count: 1,
            });
            player.inventory.click(slot);
        }
        run(&world, &mut player, &mut entities, &mut popups, 2.0);
        assert_eq!(entities.query::<&Drop>().iter().count(), 1, "nowhere to put it");
        player.inventory.click(crate::items::Slot::Hotbar(4));
        player.inventory.cursor = None;
        run(&world, &mut player, &mut entities, &mut popups, 2.0);
        assert_eq!(
            entities.query::<&Drop>().iter().count(),
            0,
            "picked up once there is room"
        );
        assert!(player.inventory.has(Item::Dirt));
    }

    #[test]
    fn a_thrown_stack_lands_ahead_and_is_only_picked_up_after_a_wait() {
        let world = flat_world();
        let (mut entities, mut popups) = (Entities::new(), Vec::new());
        let mut player = standing_player(&world);
        throw(
            &mut entities,
            &player,
            Stack {
                item: Item::Bone,
                count: 3,
            },
        );
        run(&world, &mut player, &mut entities, &mut popups, THROWN_WAIT * 0.8);
        let (body, _) = entities
            .query::<(&Body, &Drop)>()
            .iter()
            .next()
            .map(|(b, d)| (*b, *d))
            .expect("still lying there");
        assert!(body.pos[0] > player.pos[0] + 1.0, "thrown ahead: {:?}", body.pos);
        run(&world, &mut player, &mut entities, &mut popups, 3.0);
        assert_eq!(
            entities.query::<&Drop>().iter().count(),
            0,
            "picked up once the wait is over"
        );
        assert!(player.inventory.has(Item::Bone));
    }

    #[test]
    fn one_swing_hits_a_cube_in_reach_once_and_knocks_it_away() {
        let world = flat_world();
        let mut player = standing_player(&world);
        let mut entities = Entities::new();
        let cube = cube_at(&mut entities, mob::GREEN_CUBE, 21.8);
        let mut popups = Vec::new();
        player.use_held(&mut World::new(32, 32), [30.0, 20.0], None);
        run(&world, &mut player, &mut entities, &mut popups, STARTER_SWORD.use_time);
        let state = *entities.get::<&MobState>(cube).unwrap();
        assert_eq!(
            state.health,
            mob::kind(mob::GREEN_CUBE).health - STARTER_SWORD.damage,
            "hit exactly once"
        );
        assert!(
            entities.get::<&Body>(cube).unwrap().vel[0] > 0.0,
            "thrown away from the player"
        );
        assert_eq!(
            popups.iter().filter(|p| !p.hurt_player).count(),
            1,
            "one number per hit"
        );
    }

    #[test]
    fn a_swing_misses_what_is_behind_or_out_of_reach() {
        let world = flat_world();
        let mut player = standing_player(&world);
        let mut entities = Entities::new();
        let behind = cube_at(&mut entities, mob::GREEN_CUBE, 15.0);
        let far = cube_at(&mut entities, mob::GREEN_CUBE, 27.0);
        let mut popups = Vec::new();
        player.use_held(&mut World::new(32, 32), [30.0, 20.0], None);
        run(&world, &mut player, &mut entities, &mut popups, STARTER_SWORD.use_time);
        for cube in [behind, far] {
            assert_eq!(
                entities.get::<&MobState>(cube).unwrap().health,
                mob::kind(mob::GREEN_CUBE).health
            );
        }
    }

    #[test]
    fn a_cube_bursts_into_its_bones_which_are_picked_up() {
        let world = flat_world();
        let mut player = standing_player(&world);
        let mut entities = Entities::new();
        let seed = (0..).find(|&s| mob::contents(s).is_some()).unwrap();
        let carried = mob::contents(seed).unwrap();
        mob::spawn(
            &mut entities,
            mob::GREEN_CUBE,
            [21.8, GROUND as f32 - mob::kind(mob::GREEN_CUBE).half[1]],
            seed,
        );
        let mut popups = Vec::new();
        for _ in 0..2 {
            player.use_held(&mut World::new(32, 32), [30.0, 20.0], None);
            run(
                &world,
                &mut player,
                &mut entities,
                &mut popups,
                STARTER_SWORD.use_time + DT,
            );
            // Hold it in place for the next swing.
            for body in entities.query_mut::<&mut Body>() {
                body.pos[0] = 21.8;
            }
        }
        assert_eq!(
            entities.query_mut::<&MobState>().into_iter().count(),
            0,
            "two hits burst a green cube"
        );
        run(&world, &mut player, &mut entities, &mut popups, 1.0);
        let bones: u16 = player
            .inventory
            .hotbar
            .iter()
            .flatten()
            .filter(|s| s.item == Item::Bone)
            .map(|s| s.count)
            .sum();
        assert_eq!(bones, carried.count, "its bones end up in the inventory");
        assert_eq!(entities.query_mut::<&Drop>().into_iter().count(), 0);
    }

    #[test]
    fn touching_a_cube_hurts_once_until_the_blink_wears_off_and_death_respawns() {
        let world = flat_world();
        let mut player = standing_player(&world);
        let mut entities = Entities::new();
        let damage = mob::kind(mob::RED_CUBE).damage;
        let mut popups = Vec::new();
        cube_at(&mut entities, mob::RED_CUBE, 20.5);
        update(&world, &mut player, &mut entities, &mut popups, DT);
        update(&world, &mut player, &mut entities, &mut popups, DT);
        assert_eq!(
            player.health,
            player::MAX_HEALTH - damage,
            "hurt once, then untouchable"
        );
        assert!(popups.iter().any(|p| p.hurt_player && p.value == damage));

        // Keep touching until dead, then wait out the respawn.
        let spawn_x = 20.0;
        for _ in 0..600 {
            for body in entities.query_mut::<&mut Body>() {
                body.pos = player.pos;
            }
            player.invulnerable = 0.0;
            update(&world, &mut player, &mut entities, &mut popups, DT);
            if player.is_dead() {
                break;
            }
        }
        assert!(player.is_dead());
        entities.clear();
        run(
            &world,
            &mut player,
            &mut entities,
            &mut popups,
            player::RESPAWN_TIME + 0.1,
        );
        assert!(!player.is_dead());
        assert_eq!(player.health, player::MAX_HEALTH);
        assert!((player.pos[0] - spawn_x).abs() < 0.01, "back at the spawn point");
    }
}
