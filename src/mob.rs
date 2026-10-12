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
use crate::items::Item;
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
    pub health: i32,
    /// What touching it costs the player.
    pub damage: i32,
    pub look: Look,
    /// Moves only by leaping: still on the ground, unsteerable in the air.
    pub leaps: bool,
    pub brain: fn(&BrainCtx) -> MobAction,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// A flat tinted box: the placeholder.
    Box,
    /// A gel cube: which of `icons::CUBES`, tinted by the kind's colour.
    Cube(usize),
}

pub const GREEN_CUBE: u16 = 0;
pub const BLUE_CUBE: u16 = 1;
pub const RED_CUBE: u16 = 2;
/// Shelved: it gets stuck at trees, and should become a worm. Never spawned.
#[allow(dead_code)]
pub const DIGGER: u16 = 3;

/// A gel cube's table entry: its hitbox is its drawn width square.
const fn cube(
    name: &'static str,
    size: usize,
    speed: f32,
    jump_speed: f32,
    color: [u8; 4],
    health: i32,
    damage: i32,
) -> MobKind {
    let half = crate::icons::CUBES[size].1 as f32 / 32.0;
    MobKind {
        name,
        half: [half, half],
        sprite_half: [half, half],
        speed,
        jump_speed,
        hearing: 0.0,
        mine_interval: 0.0,
        color,
        health,
        damage,
        look: Look::Cube(size),
        leaps: true,
        brain: leaper,
    }
}

pub const MOB_KINDS: &[MobKind] = &[
    cube("green cube", 0, 6.0, 12.0, [120, 214, 96, 255], 40, 15),
    cube("blue cube", 1, 7.0, 14.5, [92, 158, 236, 255], 60, 25),
    cube("red cube", 2, 8.0, 17.0, [232, 88, 84, 255], 100, 40),
    MobKind {
        name: "digger",
        half: [0.7, 0.6],
        sprite_half: [0.95, 0.8],
        speed: 3.5,
        jump_speed: 12.0,
        hearing: 90.0,
        mine_interval: 0.35,
        color: [196, 112, 70, 255],
        health: 50,
        damage: 10,
        look: Look::Box,
        leaps: false,
        brain: digger,
    },
];

/// What the spawner picks from, with weights: small cubes are commonest.
const SPAWNS: [(u16, u64); 3] = [(GREEN_CUBE, 50), (BLUE_CUBE, 35), (RED_CUBE, 15)];

pub fn kind(id: u16) -> &'static MobKind {
    &MOB_KINDS[id as usize]
}

/// Everything a brain is allowed to see.
pub struct BrainCtx<'a> {
    pub world: &'a World,
    pub kind: &'static MobKind,
    pub body: Body,
    pub player: [f32; 2],
    pub player_vel: [f32; 2],
    /// A leaper has waited out its pause on the ground and may jump.
    pub ready: bool,
    /// The coming leap is a big one.
    pub big: bool,
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
    /// For a leaper: the velocity to launch at, honoured on the ground.
    pub leap: Option<[f32; 2]>,
}

/// Per-mob scratch the systems own, as opposed to the brains.
#[derive(Clone, Copy, Debug, Default)]
pub struct MobState {
    pub mine_cooldown: f32,
    pub health: i32,
    /// Time left that a knockback carries it instead of its brain.
    pub stun: f32,
    /// The player swing that last hit it, so one swing hits once.
    pub last_swing: u32,
    /// Squash and its rate: a spring, positive flatter, kicked by landings.
    pub squash: f32,
    squash_rate: f32,
    /// A leaper's time on the ground, the pause it takes before the next leap,
    /// and how many leaps it has made.
    ground_time: f32,
    wait: f32,
    leaps: u32,
    seed: u64,
}

/// A leaper pauses this long on the ground, plus up to `WAIT_SPREAD` more,
/// squishing down over the last `WINDUP` as a warning.
const WAIT: f32 = 0.6;
const WAIT_SPREAD: f32 = 0.6;
const WINDUP: f32 = 0.3;
const WINDUP_SQUASH: f32 = 0.22;
/// Every this many leaps is a big one: this much faster up (about twice as
/// high) and this much more reach sideways.
const BIG_EVERY: u32 = 3;
const BIG_LIFT: f32 = 1.4;
const BIG_REACH: f32 = 1.5;
/// Further than this from the player, a leaper just hops idly where it is.
const CHASE: f32 = 30.0;

/// How hard a landing squashes, and the spring that wobbles it back.
const SQUASH_KICK: f32 = 0.09;
const SQUASH_STIFFNESS: f32 = 260.0;
const SQUASH_DAMPING: f32 = 11.0;
/// How far rising fast stretches it, at most.
const STRETCH_MAX: f32 = 0.22;

impl MobState {
    pub fn new(kind: &MobKind) -> MobState {
        MobState {
            health: kind.health,
            wait: WAIT,
            ..MobState::default()
        }
    }

    fn with_seed(kind: &MobKind, seed: u64) -> MobState {
        MobState {
            seed,
            wait: pause(seed, 0),
            ..MobState::new(kind)
        }
    }

    /// Drawn scale (x, y): squashed flat on landing, stretched tall rising, and
    /// always the same area, so it reads as soft rather than resized.
    pub fn deform(&self, body: &Body) -> [f32; 2] {
        let stretch = if body.grounded {
            0.0
        } else {
            (-body.vel[1] / 60.0).clamp(0.0, STRETCH_MAX)
        };
        // Squishing down before a leap, so the player can see it coming.
        let windup = if body.grounded {
            ((self.ground_time - (self.wait - WINDUP)) / WINDUP).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let tall = ((1.0 - self.squash - WINDUP_SQUASH * windup) * (1.0 + stretch)).clamp(0.55, 1.45);
        [1.0 / tall, tall]
    }
}

/// Items a gel cube carries, floating where they show through the jelly, and
/// dropped when it bursts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contents {
    pub item: Item,
    pub count: u16,
    /// Per item: where in the cube (fractions of its size from the centre) and
    /// turned how far.
    pub spots: [[f32; 3]; 2],
}

/// Most cubes carry a bone or two; the rest nothing. Ores and rarer items join
/// this list when they exist.
pub fn contents(seed: u64) -> Option<Contents> {
    let h = hash(seed);
    if h % 100 >= 80 {
        return None;
    }
    let spot = |k: u64| {
        let r = hash(seed ^ k);
        let frac = |shift: u32| ((r >> shift) % 1000) as f32 / 1000.0;
        [
            frac(0) * 0.5 - 0.25,
            frac(10) * 0.4 - 0.15,
            frac(20) * std::f32::consts::TAU,
        ]
    };
    Some(Contents {
        item: Item::Bone,
        count: 1 + (h >> 8) as u16 % 2,
        spots: [spot(1), spot(2)],
    })
}

pub fn hash(mut h: u64) -> u64 {
    h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 29)
}

/// How long a leaper pauses before leap number `leaps`.
fn pause(seed: u64, leaps: u32) -> f32 {
    WAIT + WAIT_SPREAD * (hash(seed ^ (leaps as u64) << 32) % 1000) as f32 / 1000.0
}

/// Once its pause is over, leaps for where the player will be when it lands:
/// their position plus their velocity over the flight's time. Every third leap
/// is a big one. Far from the player it only hops in place.
fn leaper(ctx: &BrainCtx) -> MobAction {
    if !ctx.ready {
        return MobAction::default();
    }
    let kind = ctx.kind;
    let dx = ctx.player[0] - ctx.body.pos[0];
    if dx.abs() > CHASE {
        return MobAction {
            leap: Some([0.0, -kind.jump_speed * 0.5]),
            ..MobAction::default()
        };
    }
    let (up, reach) = if ctx.big {
        (kind.jump_speed * BIG_LIFT, kind.speed * BIG_REACH)
    } else {
        (kind.jump_speed, kind.speed)
    };
    // Up and back down to the same height.
    let flight = 2.0 * up / physics::GRAVITY;
    let target = ctx.player[0] + ctx.player_vel[0] * flight;
    let across = ((target - ctx.body.pos[0]) / flight).clamp(-reach, reach);
    MobAction {
        leap: Some([across, -up]),
        ..MobAction::default()
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
        let ahead = [(pos[0] + walk * (half[0] + 0.4)).floor() as i32, pos[1].floor() as i32];
        if is_solid(ctx.world.get(ahead[0], ahead[1])) {
            return MobAction {
                walk,
                jump: false,
                mine: Some(ahead),
                leap: None,
            };
        }
    }

    // The noise is below and the floor is in the way: tunnel downward.
    if to[1] > 1.0 {
        let below = [pos[0].floor() as i32, (pos[1] + half[1] + 0.3).floor() as i32];
        if is_solid(ctx.world.get(below[0], below[1])) {
            return MobAction {
                walk: 0.0,
                jump: false,
                mine: Some(below),
                leap: None,
            };
        }
    }

    MobAction {
        walk,
        jump: to[1] < -0.5 && ctx.body.grounded,
        mine: None,
        leap: None,
    }
}

/// Runs every mob for one frame: think, move, then act on the world.
pub fn update(
    entities: &mut Entities,
    world: &mut World,
    player: [f32; 2],
    player_vel: [f32; 2],
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
                kind,
                body: *body,
                player,
                player_vel,
                ready: body.grounded && state.ground_time >= state.wait,
                big: state.leaps % BIG_EVERY == BIG_EVERY - 1,
                noise: if kind.hearing > 0.0 {
                    noise.heard(body.pos, kind.hearing, 12.0)
                } else {
                    None
                },
            };
            (kind.brain)(&ctx)
        };

        if state.stun > 0.0 {
            // Knocked back: the hit carries it, slowing in the air, and its brain waits.
            state.stun -= dt;
            body.vel[0] *= (-2.0 * dt).exp();
        } else if kind.leaps {
            // Stuck where it lands; carried by its leap in the air.
            if body.grounded {
                body.vel[0] = 0.0;
                if let Some(v) = action.leap {
                    body.vel = v;
                    state.leaps += 1;
                    state.ground_time = 0.0;
                    state.wait = pause(state.seed, state.leaps);
                }
            }
        } else {
            body.vel[0] = action.walk * kind.speed;
            if action.jump && body.grounded {
                body.vel[1] = -kind.jump_speed;
            }
        }
        body.vel[1] = (body.vel[1] + physics::GRAVITY * dt).min(physics::MAX_FALL);

        let (was_grounded, falling) = (body.grounded, body.vel[1]);
        let may_step = physics::supported(world, body.pos, body.half);
        physics::move_box(&mut body.pos, &mut body.vel, body.half, world, dt, may_step);
        body.grounded = physics::grounded(world, body.pos, body.half);

        if body.grounded && !was_grounded {
            state.squash_rate += falling.max(0.0) * SQUASH_KICK * 60.0;
            state.ground_time = 0.0;
        }
        if body.grounded {
            state.ground_time += dt;
        }
        state.squash_rate += (-SQUASH_STIFFNESS * state.squash - SQUASH_DAMPING * state.squash_rate) * dt;
        state.squash = (state.squash + state.squash_rate * dt).clamp(-0.3, 0.4);

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

/// Spawns a mob; `seed` decides what a gel cube carries.
pub fn spawn(entities: &mut Entities, id: u16, pos: [f32; 2], seed: u64) -> hecs::Entity {
    let kind = kind(id);
    let entity = entities.spawn((
        Body::new(pos, kind.half),
        Kind(id),
        Sprite::standing_on(kind.half, kind.sprite_half, kind.color),
        MobState::with_seed(kind, seed),
    ));
    if let (Look::Cube(_), Some(carried)) = (kind.look, contents(seed)) {
        let _ = entities.insert_one(entity, carried);
    }
    entity
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

/// Cubes are the only early mobs: a few around a new player, not a swarm.
/// None in the first `GRACE` seconds after starting.
const GRACE: f32 = 10.0;

impl Default for Spawner {
    fn default() -> Spawner {
        Spawner {
            cap: 4,
            interval: 8.0,
            timer: GRACE,
            seed: 0x5EED,
        }
    }
}

impl Spawner {
    pub fn tick(&mut self, entities: &mut Entities, world: &World, view: TileRect, player: [f32; 2], dt: f32) {
        self.forget_distant(entities, player);

        self.timer -= dt;
        if self.timer > 0.0 || count(entities) >= self.cap {
            return;
        }
        self.timer = self.interval;

        let total: u64 = SPAWNS.iter().map(|s| s.1).sum();
        let mut roll = self.next() % total;
        let mut id = GREEN_CUBE;
        for (kind_id, weight) in SPAWNS {
            if roll < weight {
                id = kind_id;
                break;
            }
            roll -= weight;
        }
        // A band to the left or right of the view, never inside it.
        let left = self.next() % 2 == 0;
        let offset = SPAWN_MARGIN + (self.next() % SPAWN_BAND as u64) as i32;
        let x = if left { view.x0 - offset } else { view.x1 + offset };
        if let Some(y) = standing_room(world, x, view.y0, view.y1, kind(id).half) {
            let seed = self.next();
            spawn(entities, id, [x as f32 + 0.5, y], seed);
        }
    }

    fn forget_distant(&self, entities: &mut Entities, player: [f32; 2]) {
        // `Entity` is itself a query term, which is how an id comes back alongside
        // the components in hecs.
        let gone: Vec<hecs::Entity> = entities
            .query::<(hecs::Entity, &Body)>()
            .iter()
            .filter(|(_, body)| (body.pos[0] - player[0]).hypot(body.pos[1] - player[1]) > FORGET_DISTANCE)
            .map(|(entity, _)| entity)
            .collect();
        for entity in gone {
            let _ = entities.despawn(entity);
        }
    }

    /// Deterministic, so a replay of the same frames spawns the same mobs.
    fn next(&mut self) -> u64 {
        self.seed = self.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
        hash(self.seed)
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
    fn a_cube_squashes_on_landing_and_keeps_its_area() {
        let mut world = flat_world();
        let mut entities = Entities::new();
        let noise = NoiseLog::default();
        // Dropped from above, with the player straight below so it doesn't hop off.
        spawn(&mut entities, RED_CUBE, [20.0, resting(RED_CUBE) - 4.0], 1);
        let mut flattest: f32 = 1.0;
        for _ in 0..90 {
            update(&mut entities, &mut world, [20.0, 0.0], [0.0, 0.0], &noise, DT);
            for (body, state) in entities.query_mut::<(&Body, &MobState)>() {
                let [sx, sy] = state.deform(body);
                assert!((sx * sy - 1.0).abs() < 1e-4, "squash changed the area: {sx} x {sy}");
                flattest = flattest.min(sy);
            }
        }
        assert!(flattest < 0.85, "landing should squash it visibly: {flattest}");
    }

    #[test]
    fn most_cubes_carry_a_bone_or_two_and_the_spawner_never_sends_the_digger() {
        let carried = (0..1000).filter_map(contents).collect::<Vec<_>>();
        assert!(
            (700..900).contains(&carried.len()),
            "{} of 1000 carry something",
            carried.len()
        );
        assert!(
            carried
                .iter()
                .all(|c| c.item == Item::Bone && (1..=2).contains(&c.count))
        );

        let world = flat_world();
        let mut entities = Entities::new();
        let mut spawner = Spawner {
            cap: 200,
            interval: 0.0,
            timer: 0.0,
            ..Spawner::default()
        };
        let view = TileRect {
            x0: 25,
            y0: 10,
            x1: 30,
            y1: 50,
        };
        for _ in 0..400 {
            spawner.tick(&mut entities, &world, view, [27.0, 20.0], DT);
        }
        assert!(count(&entities) > 20);
        assert!(entities.query_mut::<&Kind>().into_iter().all(|k| k.0 != DIGGER));
    }

    /// Runs one cube toward a player and records (time, x, y, grounded).
    fn track(player: [f32; 2], player_vel: [f32; 2], seconds: f32) -> Vec<(f32, f32, f32, bool)> {
        let mut world = flat_world();
        let mut entities = Entities::new();
        let noise = NoiseLog::default();
        spawn(&mut entities, GREEN_CUBE, [20.0, resting(GREEN_CUBE)], 7);
        let mut out = Vec::new();
        for i in 0..(seconds / DT) as i32 {
            update(&mut entities, &mut world, player, player_vel, &noise, DT);
            let body = *entities.query_mut::<&Body>().into_iter().next().unwrap();
            out.push((i as f32 * DT, body.pos[0], body.pos[1], body.grounded));
        }
        out
    }

    #[test]
    fn a_cube_waits_on_the_ground_then_leaps_without_steering() {
        let path = track([26.0, resting(GREEN_CUBE)], [0.0, 0.0], 3.0);
        let first_air = path.iter().position(|p| !p.3).expect("it leaps");
        assert!(
            path[first_air].0 >= WAIT - 0.05,
            "left the ground after only {}s",
            path[first_air].0
        );
        assert!(
            path[..first_air].iter().all(|p| (p.1 - 20.0).abs() < 1e-3),
            "it slides while waiting"
        );
        // In the air it keeps a constant sideways speed: no steering.
        let air: Vec<_> = path[first_air..].iter().take_while(|p| !p.3).collect();
        let steps: Vec<f32> = air.windows(2).map(|w| w[1].1 - w[0].1).collect();
        assert!(steps.iter().all(|d| (d - steps[0]).abs() < 1e-3), "it steered mid-air");
        assert!(steps[0] > 0.0, "it leapt toward the player");
    }

    #[test]
    fn a_cube_aims_at_where_a_moving_player_will_be() {
        let landing = |vel: f32| {
            // Close enough that no leap is clamped to the cube's reach.
            let path = track([22.0, resting(GREEN_CUBE)], [vel, 0.0], 3.0);
            let first_air = path.iter().position(|p| !p.3).unwrap();
            path[first_air..].iter().find(|p| p.3).unwrap().1
        };
        assert!(landing(2.0) > landing(0.0) + 0.8, "a player running away should be led");
        assert!(
            landing(-2.0) < landing(0.0) - 0.8,
            "and one running toward it met early"
        );
    }

    #[test]
    fn every_third_leap_is_a_big_one() {
        let path = track([20.0, resting(GREEN_CUBE)], [0.0, 0.0], 8.0);
        // The highest point of each leap.
        let mut peaks = Vec::new();
        let mut top = f32::MAX;
        for w in path.windows(2) {
            if !w[1].3 {
                top = top.min(w[1].2);
            } else if !w[0].3 {
                peaks.push(resting(GREEN_CUBE) - top);
                top = f32::MAX;
            }
        }
        assert!(peaks.len() >= 4, "{peaks:?}");
        assert!(peaks[2] > peaks[0] * 1.6 && peaks[2] > peaks[1] * 1.6, "{peaks:?}");
        assert!((peaks[0] - peaks[1]).abs() < 0.2, "{peaks:?}");
    }

    #[test]
    fn a_bouncer_closes_on_the_player_without_leaving_the_terrain() {
        let mut world = flat_world();
        let mut entities = Entities::new();
        let noise = NoiseLog::default();
        let player = [40.0, resting(GREEN_CUBE)];
        spawn(&mut entities, GREEN_CUBE, [20.0, resting(GREEN_CUBE)], 1);

        for _ in 0..600 {
            update(&mut entities, &mut world, player, [0.0, 0.0], &noise, DT);
        }
        let body = *entities
            .query_mut::<&Body>()
            .into_iter()
            .next()
            .expect("the mob exists");
        assert!(body.pos[0] > 30.0, "did not close the distance: x {}", body.pos[0]);
        assert!(
            body.pos[1] <= resting(GREEN_CUBE) + 0.01,
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
        spawn(&mut entities, DIGGER, start, 1);

        for _ in 0..300 {
            update(&mut entities, &mut world, [40.0, start[1]], [0.0, 0.0], &noise, DT);
        }
        let body = *entities
            .query_mut::<&Body>()
            .into_iter()
            .next()
            .expect("the mob exists");
        assert!((body.pos[0] - start[0]).abs() < 0.5, "wandered to x {}", body.pos[0]);
    }

    #[test]
    fn a_digger_pursues_mining_it_can_hear_and_ignores_what_it_cannot() {
        let mut world = flat_world();
        let mut entities = Entities::new();
        let start = [20.0, resting(DIGGER)];
        spawn(&mut entities, DIGGER, start, 1);

        let mut far = NoiseLog::default();
        far.record([20.0 + kind(DIGGER).hearing + 20.0, start[1]]);
        for _ in 0..120 {
            update(&mut entities, &mut world, [40.0, start[1]], [0.0, 0.0], &far, DT);
        }
        let body = *entities
            .query_mut::<&Body>()
            .into_iter()
            .next()
            .expect("the mob exists");
        assert!((body.pos[0] - start[0]).abs() < 0.5, "chased a noise out of earshot");

        let mut near = NoiseLog::default();
        near.record([40.0, start[1]]);
        for _ in 0..300 {
            update(&mut entities, &mut world, [40.0, start[1]], [0.0, 0.0], &near, DT);
        }
        let body = *entities
            .query_mut::<&Body>()
            .into_iter()
            .next()
            .expect("the mob exists");
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
        spawn(&mut entities, DIGGER, [22.0, resting(DIGGER)], 1);

        let before = world.chunk_revision(0, 0);
        let one_interval = (kind(DIGGER).mine_interval / DT) as usize;
        for _ in 0..(one_interval * 2) {
            update(&mut entities, &mut world, [40.0, resting(DIGGER)], [0.0, 0.0], &log, DT);
            log.advance(DT);
        }

        let removed = (GROUND - 4..GROUND).filter(|y| world.get(25, *y) == AIR).count();
        assert!(removed >= 1, "never broke through the wall");
        assert!(removed <= 3, "dug {removed} tiles in two intervals; rate is unbounded");
        assert!(
            world.chunk_revision(0, 0) > before,
            "mined tiles must rebuild their chunk"
        );
    }

    #[test]
    fn the_spawner_respects_its_cap_and_places_mobs_outside_the_view_on_solid_ground() {
        let world = flat_world();
        let mut entities = Entities::new();
        let mut spawner = Spawner {
            cap: 5,
            interval: 0.0,
            timer: 0.0,
            ..Spawner::default()
        };
        let view = TileRect {
            x0: 25,
            y0: 10,
            x1: 45,
            y1: 50,
        };
        let player = [35.0, resting(GREEN_CUBE)];

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
        spawn(&mut entities, GREEN_CUBE, [20.0, resting(GREEN_CUBE)], 1);
        assert_eq!(count(&entities), 1);

        let far_away = [20.0 + FORGET_DISTANCE + 10.0, resting(GREEN_CUBE)];
        spawner.tick(
            &mut entities,
            &world,
            TileRect {
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 1,
            },
            far_away,
            DT,
        );
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
            spawn(&mut entities, id, [20.0 + id as f32 * 3.0, resting(id)], 1);
        }
        for _ in 0..120 {
            update(&mut entities, &mut world, [40.0, 30.0], [0.0, 0.0], &log, DT);
        }
        assert_eq!(count(&entities), MOB_KINDS.len());
        for body in entities.query_mut::<&Body>() {
            assert!(
                body.pos[0].is_finite() && body.pos[1].is_finite(),
                "{body:?} went non-finite"
            );
        }
    }
}
