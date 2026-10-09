//! Mobs: one table entry and one brain function per kind.
//!
//! The entity store holds components and answers queries; behaviour stays in a
//! `brain` that reads top to bottom, so adding a mob means adding a `MOB_KINDS`
//! entry and a function, and nothing else in this file changes.
//!
//! A brain is handed a snapshot and returns an intent. It never mutates anything
//! itself, which keeps ordering explicit and makes each brain testable on its own.

use crate::camera::TileRect;
use crate::entity::{Body, Entities, Kind, Sprite};
use crate::noise::{Noise, NoiseLog};
use crate::physics;
use crate::tiles::{AIR, is_solid};
use crate::world::World;

pub struct MobKind {
    pub name: &'static str,
    pub half: [f32; 2],
    /// Drawn extents, independent of the hitbox above.
    pub sprite_half: [f32; 2],
    pub speed: f32,
    pub jump_speed: f32,
    /// How far this kind can hear mining, in tiles. Zero means deaf.
    pub hearing: f32,
    /// Seconds between tiles it can remove. Zero means it cannot dig at all,
    /// which is what bounds a mob's effect on the world.
    pub mine_interval: f32,
    pub color: [u8; 4],
    pub brain: fn(&BrainCtx) -> MobAction,
}

pub const BOUNCER: u16 = 0;
pub const DIGGER: u16 = 1;

pub const MOB_KINDS: &[MobKind] = &[
    MobKind {
        name: "bouncer",
        half: [0.6, 0.5],
        sprite_half: [0.85, 0.7],
        speed: 4.0,
        jump_speed: 13.0,
        hearing: 0.0,
        mine_interval: 0.0,
        color: [86, 170, 200, 255],
        brain: bouncer,
    },
    MobKind {
        name: "digger",
        half: [0.7, 0.6],
        sprite_half: [0.95, 0.8],
        speed: 3.5,
        jump_speed: 12.0,
        hearing: 90.0,
        mine_interval: 0.35,
        color: [196, 112, 70, 255],
        brain: digger,
    },
];

pub fn kind(id: u16) -> &'static MobKind {
    &MOB_KINDS[id as usize]
}

/// Everything a brain is allowed to see.
pub struct BrainCtx<'a> {
    pub world: &'a World,
    pub body: Body,
    pub player: [f32; 2],
    /// The most recent disturbance this mob can hear, if any.
    pub noise: Option<Noise>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MobAction {
    /// -1, 0 or 1.
    pub walk: f32,
    pub jump: bool,
    /// A tile to remove, honoured only if the kind can dig and its cooldown is up.
    pub mine: Option<[i32; 2]>,
}

/// Per-mob scratch the systems own, as opposed to the brains.
#[derive(Clone, Copy, Debug, Default)]
pub struct MobState {
    pub mine_cooldown: f32,
}

/// Hops toward the player whenever it lands. No timer: the cadence falls out of
/// how long it is airborne.
fn bouncer(ctx: &BrainCtx) -> MobAction {
    let dx = ctx.player[0] - ctx.body.pos[0];
    MobAction {
        walk: if dx.abs() < 0.3 { 0.0 } else { dx.signum() },
        jump: ctx.body.grounded,
        mine: None,
    }
}

/// Heads for the last mining it heard, tunnelling through anything in the way.
/// Without a disturbance to investigate it does nothing, so digging only ever
/// happens in response to the player.
fn digger(ctx: &BrainCtx) -> MobAction {
    let Some(noise) = ctx.noise else {
        return MobAction::default();
    };
    let (pos, half) = (ctx.body.pos, ctx.body.half);
    let to = [noise.pos[0] - pos[0], noise.pos[1] - pos[1]];
    if to[0].abs() < 1.0 && to[1].abs() < 1.5 {
        return MobAction::default(); // arrived
    }

    let walk = if to[0].abs() < 0.5 { 0.0 } else { to[0].signum() };

    // Blocked horizontally: remove the tile in the way rather than giving up.
    if walk != 0.0 {
        let ahead = [
            (pos[0] + walk * (half[0] + 0.4)).floor() as i32,
            pos[1].floor() as i32,
        ];
        if is_solid(ctx.world.get(ahead[0], ahead[1])) {
            return MobAction { walk, jump: false, mine: Some(ahead) };
        }
    }

    // The noise is below and the floor is in the way: tunnel downward.
    if to[1] > 1.0 {
        let below = [pos[0].floor() as i32, (pos[1] + half[1] + 0.3).floor() as i32];
        if is_solid(ctx.world.get(below[0], below[1])) {
            return MobAction { walk: 0.0, jump: false, mine: Some(below) };
        }
    }

    MobAction { walk, jump: to[1] < -0.5 && ctx.body.grounded, mine: None }
}

/// Runs every mob for one frame: think, move, then act on the world.
pub fn update(
    entities: &mut Entities,
    world: &mut World,
    player: [f32; 2],
    noise: &NoiseLog,
    dt: f32,
) {
    for (body, id, state) in entities.query_mut::<(&mut Body, &Kind, &mut MobState)>() {
        let kind = kind(id.0);
        body.grounded = physics::grounded(world, body.pos, body.half);
        state.mine_cooldown = (state.mine_cooldown - dt).max(0.0);

        let action = {
            let ctx = BrainCtx {
                world,
                body: *body,
                player,
                noise: if kind.hearing > 0.0 {
                    noise.heard(body.pos, kind.hearing, 12.0)
                } else {
                    None
                },
            };
            (kind.brain)(&ctx)
        };

        body.vel[0] = action.walk * kind.speed;
        if action.jump && body.grounded {
            body.vel[1] = -kind.jump_speed;
        }
        body.vel[1] = (body.vel[1] + physics::GRAVITY * dt).min(physics::MAX_FALL);

        let may_step = physics::supported(world, body.pos, body.half);
        physics::move_box(&mut body.pos, &mut body.vel, body.half, world, dt, may_step);
        body.grounded = physics::grounded(world, body.pos, body.half);

        if let Some(target) = action.mine {
            let can_dig = kind.mine_interval > 0.0 && state.mine_cooldown <= 0.0;
            let anchored = world.forest.anchors(target[0], target[1]);
            if can_dig && world.in_bounds(target[0], target[1]) && !anchored {
                world.set(target[0], target[1], AIR);
                state.mine_cooldown = kind.mine_interval;
            }
        }
    }
}

pub fn spawn(entities: &mut Entities, id: u16, pos: [f32; 2]) -> hecs::Entity {
    let kind = kind(id);
    entities.spawn((
        Body::new(pos, kind.half),
        Kind(id),
        Sprite::standing_on(kind.half, kind.sprite_half, kind.color),
        MobState::default(),
    ))
}

pub fn count(entities: &Entities) -> usize {
    entities.query::<&Kind>().iter().count()
}

/// Keeps a population around the player: spawns just outside the view on solid
/// ground, and forgets mobs that have been left far behind.
pub struct Spawner {
    pub cap: usize,
    pub interval: f32,
    timer: f32,
    seed: u64,
}

/// How far outside the view mobs appear, and how far away they are forgotten.
const SPAWN_MARGIN: i32 = 6;
const SPAWN_BAND: i32 = 24;
const FORGET_DISTANCE: f32 = 220.0;

impl Default for Spawner {
    fn default() -> Spawner {
        Spawner { cap: 24, interval: 1.5, timer: 0.0, seed: 0x5EED }
    }
}

impl Spawner {
    pub fn tick(
        &mut self,
        entities: &mut Entities,
        world: &World,
        view: TileRect,
        player: [f32; 2],
        dt: f32,
    ) {
        self.forget_distant(entities, player);

        self.timer -= dt;
        if self.timer > 0.0 || count(entities) >= self.cap {
            return;
        }
        self.timer = self.interval;

        let id = (self.next() % MOB_KINDS.len() as u64) as u16;
        // A band to the left or right of the view, never inside it.
        let left = self.next() % 2 == 0;
        let offset = SPAWN_MARGIN + (self.next() % SPAWN_BAND as u64) as i32;
        let x = if left { view.x0 - offset } else { view.x1 + offset };
        if let Some(y) = standing_room(world, x, view.y0, view.y1, kind(id).half) {
            spawn(entities, id, [x as f32 + 0.5, y]);
        }
    }

    fn forget_distant(&self, entities: &mut Entities, player: [f32; 2]) {
        // `Entity` is itself a query term, which is how an id comes back alongside
        // the components in hecs.
        let gone: Vec<hecs::Entity> = entities
            .query::<(hecs::Entity, &Body)>()
            .iter()
            .filter(|(_, body)| {
                (body.pos[0] - player[0]).hypot(body.pos[1] - player[1]) > FORGET_DISTANCE
            })
            .map(|(entity, _)| entity)
            .collect();
        for entity in gone {
            let _ = entities.despawn(entity);
        }
    }

    /// Deterministic, so a replay of the same frames spawns the same mobs.
    fn next(&mut self) -> u64 {
        self.seed = self.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
        let mut h = self.seed;
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^ (h >> 29)
    }
}

/// Centre height of a body standing on the first solid ground in `y0..y1` at `x`,
/// if there is room above it.
fn standing_room(world: &World, x: i32, y0: i32, y1: i32, half: [f32; 2]) -> Option<f32> {
    if !world.in_bounds(x, y0) {
        return None;
    }
    (y0..y1).find_map(|y| {
        let floor_here = is_solid(world.get(x, y));
        let pos = [x as f32 + 0.5, y as f32 - half[1]];
        let clear = !physics::overlaps_solid(world, pos, half);
        (floor_here && clear).then_some(pos[1])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::Body;
    use crate::tiles::{DIRT, STONE};

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

    fn resting(id: u16) -> f32 {
        GROUND as f32 - kind(id).half[1]
    }

    #[test]
    fn a_bouncer_closes_on_the_player_without_leaving_the_terrain() {
        let mut world = flat_world();
        let mut entities = Entities::new();
        let noise = NoiseLog::default();
        let player = [40.0, resting(BOUNCER)];
        spawn(&mut entities, BOUNCER, [20.0, resting(BOUNCER)]);

        for _ in 0..600 {
            update(&mut entities, &mut world, player, &noise, DT);
        }
        let body = *entities.query_mut::<&Body>().into_iter().next().expect("the mob exists");
        assert!(body.pos[0] > 30.0, "did not close the distance: x {}", body.pos[0]);
        assert!(
            body.pos[1] <= resting(BOUNCER) + 0.01,
            "sank into the ground: y {}",
            body.pos[1]
        );
    }

    #[test]
    fn a_digger_without_a_disturbance_stays_put() {
        let mut world = flat_world();
        let mut entities = Entities::new();
        let noise = NoiseLog::default();
        let start = [20.0, resting(DIGGER)];
        spawn(&mut entities, DIGGER, start);

        for _ in 0..300 {
            update(&mut entities, &mut world, [40.0, start[1]], &noise, DT);
        }
        let body = *entities.query_mut::<&Body>().into_iter().next().expect("the mob exists");
        assert!((body.pos[0] - start[0]).abs() < 0.5, "wandered to x {}", body.pos[0]);
    }

    #[test]
    fn a_digger_pursues_mining_it_can_hear_and_ignores_what_it_cannot() {
        let mut world = flat_world();
        let mut entities = Entities::new();
        let start = [20.0, resting(DIGGER)];
        spawn(&mut entities, DIGGER, start);

        let mut far = NoiseLog::default();
        far.record([20.0 + kind(DIGGER).hearing + 20.0, start[1]]);
        for _ in 0..120 {
            update(&mut entities, &mut world, [40.0, start[1]], &far, DT);
        }
        let body = *entities.query_mut::<&Body>().into_iter().next().expect("the mob exists");
        assert!((body.pos[0] - start[0]).abs() < 0.5, "chased a noise out of earshot");

        let mut near = NoiseLog::default();
        near.record([40.0, start[1]]);
        for _ in 0..300 {
            update(&mut entities, &mut world, [40.0, start[1]], &near, DT);
        }
        let body = *entities.query_mut::<&Body>().into_iter().next().expect("the mob exists");
        assert!(body.pos[0] > 30.0, "did not investigate the noise: x {}", body.pos[0]);
    }

    #[test]
    fn a_blocked_digger_removes_the_tile_in_its_way_at_a_bounded_rate() {
        let mut world = flat_world();
        // A wall between the digger and the noise.
        for y in GROUND - 4..GROUND {
            world.set(25, y, DIRT);
        }
        let mut entities = Entities::new();
        let mut log = NoiseLog::default();
        log.record([40.0, resting(DIGGER)]);
        spawn(&mut entities, DIGGER, [22.0, resting(DIGGER)]);

        let before = world.chunk_revision(0, 0);
        let one_interval = (kind(DIGGER).mine_interval / DT) as usize;
        for _ in 0..(one_interval * 2) {
            update(&mut entities, &mut world, [40.0, resting(DIGGER)], &log, DT);
            log.advance(DT);
        }

        let removed = (GROUND - 4..GROUND).filter(|y| world.get(25, *y) == AIR).count();
        assert!(removed >= 1, "never broke through the wall");
        assert!(removed <= 3, "dug {removed} tiles in two intervals; rate is unbounded");
        assert!(world.chunk_revision(0, 0) > before, "mined tiles must rebuild their chunk");
    }

    #[test]
    fn the_spawner_respects_its_cap_and_places_mobs_outside_the_view_on_solid_ground() {
        let world = flat_world();
        let mut entities = Entities::new();
        let mut spawner = Spawner { cap: 5, interval: 0.0, ..Spawner::default() };
        let view = TileRect { x0: 25, y0: 10, x1: 45, y1: 50 };
        let player = [35.0, resting(BOUNCER)];

        for _ in 0..500 {
            spawner.tick(&mut entities, &world, view, player, DT);
        }
        assert_eq!(count(&entities), 5, "spawned past the cap");

        for (body, id) in entities.query_mut::<(&Body, &Kind)>() {
            assert!(
                body.pos[0] < view.x0 as f32 || body.pos[0] > view.x1 as f32,
                "{} spawned inside the view at {:?}",
                kind(id.0).name,
                body.pos
            );
            assert!(
                !physics::overlaps_solid(&world, body.pos, body.half),
                "{} spawned inside solid rock at {:?}",
                kind(id.0).name,
                body.pos
            );
        }
    }

    #[test]
    fn the_spawner_forgets_mobs_left_far_behind() {
        let world = flat_world();
        let mut entities = Entities::new();
        let mut spawner = Spawner::default();
        spawn(&mut entities, BOUNCER, [20.0, resting(BOUNCER)]);
        assert_eq!(count(&entities), 1);

        let far_away = [20.0 + FORGET_DISTANCE + 10.0, resting(BOUNCER)];
        spawner.tick(&mut entities, &world, TileRect { x0: 0, y0: 0, x1: 1, y1: 1 }, far_away, DT);
        assert_eq!(count(&entities), 0, "a mob far from the player should be forgotten");
    }

    #[test]
    fn every_kind_in_the_table_runs_without_special_casing() {
        // The extensibility claim: the system drives whatever the table holds.
        let mut world = flat_world();
        let mut entities = Entities::new();
        let mut log = NoiseLog::default();
        log.record([40.0, 30.0]);
        for id in 0..MOB_KINDS.len() as u16 {
            spawn(&mut entities, id, [20.0 + id as f32 * 3.0, resting(id)]);
        }
        for _ in 0..120 {
            update(&mut entities, &mut world, [40.0, 30.0], &log, DT);
        }
        assert_eq!(count(&entities), MOB_KINDS.len());
        for body in entities.query_mut::<&Body>() {
            assert!(body.pos[0].is_finite() && body.pos[1].is_finite(), "{body:?} went non-finite");
        }
    }
}
