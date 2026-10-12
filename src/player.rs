//! The player: input intent, gravity, medium drag, and mining.
//!
//! Stored as a plain struct because there is exactly one of them. The physics it
//! uses are free functions over `(pos, vel, half, &World)`, so the same movement
//! code will drive mobs unchanged — that's what keeps a later move to an ECS a
//! change of where positions live rather than a rewrite of how bodies move.

use crate::character::{self, Appearance, frames};
use crate::items::{Inventory, Weapon};
use crate::physics::{self, Contacts};
use crate::tiles::{AIR, TileId, is_breakable, is_solid, tile_type};
use crate::world::World;

/// Half-extents in tiles: the painted figure, 20 x 52 art pixels, so what
/// collides is what you see. It fits a tunnel two wide and four tall.
pub const HALF: [f32; 2] = [0.625, 1.625];

/// A walk frame per this many tiles walked, so feet keep pace with the ground:
/// the eight-frame cycle covers two steps of about 2.6 tiles.
pub const STRIDE: f32 = 0.65;
/// How long mining and placing take, in seconds; weapons set their own.
pub const USE_TIME: f32 = 0.3;

/// Five hearts of twenty.
pub const MAX_HEALTH: i32 = 100;
pub const HEART: i32 = 20;
/// After a hit: how long nothing can hurt again, and how long the knockback
/// owns the player's movement.
pub const HURT_TIME: f32 = 0.6;
const KNOCK_TIME: f32 = 0.25;
const KNOCK: [f32; 2] = [10.0, 9.0];
pub const RESPAWN_TIME: f32 = 3.0;
/// Slower than this counts as standing still, in tiles per second.
const STILL: f32 = 0.5;

pub const WALK_SPEED: f32 = 9.0;
/// v^2 / 2g with these values peaks a little over 5 tiles.
pub const JUMP_SPEED: f32 = 21.5;
/// Mining range from the player's centre, in tiles.
pub const REACH: f32 = 5.5;

/// How quickly the drawn position catches up after a step up or down, per
/// second. The step itself is instantaneous in collision terms, so running over
/// steps costs no speed; this is what stops it looking like a teleport.
const STEP_CATCH_UP: f32 = 16.0;

#[derive(Debug, Default, Clone, Copy)]
pub struct Intent {
    /// -1, 0 or 1.
    pub walk: f32,
    pub jump: bool,
}

pub struct Player {
    pub pos: [f32; 2],
    pub vel: [f32; 2],
    pub grounded: bool,
    /// Remaining vertical lag, in tiles, between the drawn and actual position.
    step_lag: f32,
    /// 1 facing right, -1 left.
    pub facing: f32,
    /// Tiles walked on the ground, which the walk cycle steps through.
    walked: f32,
    /// Time left showing an action, out of how long it takes.
    using: f32,
    use_time: f32,
    pub look: Appearance,
    pub inventory: Inventory,
    pub health: i32,
    /// Time left untouchable after a hit.
    pub invulnerable: f32,
    /// Time left that a knockback overrides walking.
    knocked: f32,
    /// Time left before respawning; zero while alive.
    pub dead_for: f32,
    /// Counts sword swings, so a mob can tell it was already hit by this one.
    pub swing: u32,
    /// The weapon being swung, if the current action is a swing.
    swung: Option<Weapon>,
    spawn: [f32; 2],
}

/// Which frames to draw, and whether mirrored to face left. The legs follow
/// movement while the rest of the body follows any action, so a swing doesn't
/// stop the run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The upper body: head, torso and arms.
    pub index: u16,
    /// Trousers and shoes.
    pub legs: u16,
    pub mirrored: bool,
}

impl Frame {
    /// Art pixels the upper body moves down to sit on these legs: frames lower
    /// the body by different amounts to stand on their feet.
    pub fn upper_shift(&self) -> f32 {
        (character::drop(self.legs) - character::drop(self.index)) as f32
    }
}

impl Player {
    pub fn new(pos: [f32; 2]) -> Player {
        Player {
            pos,
            vel: [0.0, 0.0],
            grounded: false,
            step_lag: 0.0,
            facing: 1.0,
            walked: 0.0,
            using: 0.0,
            use_time: USE_TIME,
            look: Appearance::default(),
            inventory: Inventory::starter(),
            health: MAX_HEALTH,
            invulnerable: 0.0,
            knocked: 0.0,
            dead_for: 0.0,
            swing: 0,
            swung: None,
            spawn: pos,
        }
    }

    pub fn is_dead(&self) -> bool {
        self.dead_for > 0.0
    }

    /// The weapon mid-swing and how far along its arc, from 0 to 1.
    pub fn wielded(&self) -> Option<(Weapon, f32)> {
        let weapon = self.swung.filter(|_| self.using > 0.0)?;
        Some((weapon, character::swing_arc(1.0 - self.using / self.use_time)))
    }

    /// Uses the held item toward world point `at`, unless still busy with the
    /// last use: swings a sword, puts down a block.
    pub fn use_held(&mut self, world: &mut World, at: [f32; 2]) {
        if self.using > 0.0 || self.is_dead() {
            return;
        }
        let Some(item) = self.inventory.held() else {
            return;
        };
        if let Some(tile) = item.places() {
            if self.place(world, [at[0].floor() as i32, at[1].floor() as i32], tile) {
                self.inventory.consume_held();
                self.act_toward(at[0], USE_TIME);
            }
        } else if let Some(weapon) = item.weapon() {
            self.swing += 1;
            self.act_toward(at[0], weapon.use_time);
            self.swung = Some(weapon);
        }
    }

    /// Takes a hit from something at world x `from`: knocked away from it and
    /// untouchable for a moment. False, and no effect, while untouchable.
    pub fn hurt(&mut self, damage: i32, from: f32) -> bool {
        if self.invulnerable > 0.0 || self.is_dead() {
            return false;
        }
        self.health -= damage;
        self.invulnerable = HURT_TIME;
        self.knocked = KNOCK_TIME;
        let away = if self.pos[0] >= from { 1.0 } else { -1.0 };
        self.vel = [away * KNOCK[0], -KNOCK[1]];
        if self.health <= 0 {
            self.health = 0;
            self.dead_for = RESPAWN_TIME;
        }
        true
    }

    /// Starts an action aimed at world x `x`: the player turns to face it while
    /// the arm moves.
    pub fn act_toward(&mut self, x: f32, time: f32) {
        self.using = time;
        self.use_time = time;
        self.swung = None;
        if x != self.pos[0] {
            self.facing = (x - self.pos[0]).signum();
        }
    }

    /// The frames for the player as it is: the legs show being in the air,
    /// walking or standing; the upper body shows an action over the same.
    pub fn frame(&self) -> Frame {
        let legs = if !self.grounded {
            if self.vel[1] < 0.0 { frames::RISE } else { frames::FALL }
        } else if self.vel[0].abs() > STILL {
            frames::WALK[(self.walked / STRIDE) as usize % frames::WALK.len()]
        } else {
            frames::IDLE
        };
        let index = if self.using > 0.0 {
            character::use_frame(character::swing_arc(1.0 - self.using / self.use_time))
        } else {
            legs
        };
        Frame {
            index,
            legs,
            mirrored: self.facing < 0.0,
        }
    }

    /// Where the player is drawn: the true position plus the unspent step lag.
    pub fn visual_pos(&self) -> [f32; 2] {
        [self.pos[0], self.pos[1] + self.step_lag]
    }

    pub fn update(&mut self, world: &World, mut intent: Intent, dt: f32) -> Contacts {
        if self.is_dead() {
            self.dead_for -= dt;
            if self.dead_for <= 0.0 {
                self.respawn();
            }
            return Contacts::default();
        }
        self.invulnerable = (self.invulnerable - dt).max(0.0);
        // Grounded from the start of the frame: it gates both jumping and step-up.
        self.grounded = physics::grounded(world, self.pos, HALF);
        let drag = medium_drag(world, self.pos, HALF);

        if self.knocked > 0.0 {
            // The knockback carries the player; walking and jumping wait.
            self.knocked -= dt;
            intent = Intent::default();
        } else {
            self.vel[0] = intent.walk * WALK_SPEED * drag;
        }
        if intent.jump && self.grounded {
            self.vel[1] = -JUMP_SPEED;
        }
        self.vel[1] = (self.vel[1] + physics::GRAVITY * dt).min(physics::MAX_FALL * drag);

        // Stepping up needs ground under the body; `supported` rather than
        // `grounded` because the body may be a fraction of a tile up.
        let may_step_up = physics::supported(world, self.pos, HALF);
        let was_grounded = self.grounded;
        let contacts = physics::move_box(&mut self.pos, &mut self.vel, HALF, world, dt, may_step_up);

        if contacts.stepped_up {
            self.step_lag += physics::STEP_HEIGHT;
        }
        // Walking off a step down keeps to the ground rather than falling off it,
        // unless jumping or thrown.
        if was_grounded && self.vel[1] >= 0.0 && self.knocked <= 0.0 {
            if let Some(down) = physics::step_down(world, self.pos, HALF) {
                self.pos[1] += down;
                self.vel[1] = 0.0;
                self.step_lag -= down;
            }
        }
        self.step_lag *= (-STEP_CATCH_UP * dt).exp();
        if self.step_lag.abs() < 0.01 {
            self.step_lag = 0.0;
        }
        self.grounded = physics::grounded(world, self.pos, HALF);

        self.using = (self.using - dt).max(0.0);
        // Walking turns the player, except mid-action, when it faces its target.
        if intent.walk != 0.0 && self.using == 0.0 {
            self.facing = intent.walk.signum();
        }
        if self.grounded {
            self.walked += self.vel[0].abs() * dt;
        }
        contacts
    }

    fn respawn(&mut self) {
        let (inventory, look, swing) = (
            std::mem::replace(&mut self.inventory, Inventory::starter()),
            self.look,
            self.swing,
        );
        *self = Player {
            inventory,
            look,
            swing,
            ..Player::new(self.spawn)
        };
    }

    /// Removes the breakable tile at `target` if it is within reach and no tree
    /// stands on it. Returns whether anything was mined.
    pub fn mine(&self, world: &mut World, target: [i32; 2]) -> bool {
        if !self.can_reach(world, target)
            || !is_breakable(world.get(target[0], target[1]))
            || world.forest.anchors(target[0], target[1])
        {
            return false;
        }
        world.set(target[0], target[1], AIR);
        true
    }

    /// Puts a tile into empty space within reach, and not inside the player's own
    /// box — walling yourself into a solid tile would be unrecoverable.
    pub fn place(&self, world: &mut World, target: [i32; 2], id: TileId) -> bool {
        if !self.can_reach(world, target) || world.get(target[0], target[1]) != AIR {
            return false;
        }
        if is_solid(id) && overlaps_tile(self.pos, HALF, target) {
            return false;
        }
        world.set(target[0], target[1], id);
        true
    }

    fn can_reach(&self, world: &World, target: [i32; 2]) -> bool {
        if !world.in_bounds(target[0], target[1]) {
            return false;
        }
        let center = [target[0] as f32 + 0.5, target[1] as f32 + 0.5];
        (center[0] - self.pos[0]).hypot(center[1] - self.pos[1]) <= REACH
    }
}

fn overlaps_tile(pos: [f32; 2], half: [f32; 2], tile: [i32; 2]) -> bool {
    let (x, y) = (tile[0] as f32, tile[1] as f32);
    pos[0] - half[0] < x + 1.0 && pos[0] + half[0] > x && pos[1] - half[1] < y + 1.0 && pos[1] + half[1] > y
}

/// Strongest drag among the non-solid tiles the box overlaps. All tile types are
/// currently unimpeded; liquids and cobwebs will set a value below 1.0.
pub fn medium_drag(world: &World, pos: [f32; 2], half: [f32; 2]) -> f32 {
    let x0 = (pos[0] - half[0]).floor() as i32;
    let x1 = (pos[0] + half[0]).floor() as i32;
    let y0 = (pos[1] - half[1]).floor() as i32;
    let y1 = (pos[1] + half[1]).floor() as i32;
    let mut drag: f32 = 1.0;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let id = world.get(x, y);
            if !is_solid(id) {
                drag = drag.min(tile_type(id).drag);
            }
        }
    }
    drag
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn standing_player() -> Player {
        Player::new([20.0, GROUND as f32 - HALF[1]])
    }

    fn walk(player: &mut Player, world: &World, walk: f32, seconds: f32) {
        for _ in 0..(seconds / DT).round() as i32 {
            player.update(world, Intent { walk, jump: false }, DT);
        }
    }

    #[test]
    fn standing_still_shows_the_idle_frame_facing_the_last_way_walked() {
        let world = flat_world();
        let mut player = standing_player();
        walk(&mut player, &world, -1.0, 0.3);
        walk(&mut player, &world, 0.0, 0.1);
        assert_eq!(
            player.frame(),
            Frame {
                index: frames::IDLE,
                legs: frames::IDLE,
                mirrored: true
            }
        );
    }

    #[test]
    fn the_walk_cycle_follows_distance_walked_not_time() {
        let world = flat_world();
        let mut player = standing_player();
        let mut seen = Vec::new();
        for _ in 0..60 {
            player.update(&world, Intent { walk: 1.0, jump: false }, DT);
            let f = player.frame();
            assert!(frames::WALK.contains(&f.index) && !f.mirrored);
            if seen.last() != Some(&f.index) {
                seen.push(f.index);
            }
        }
        // A second at walking speed is WALK_SPEED / STRIDE frame changes.
        let expected = (WALK_SPEED / STRIDE).round() as usize;
        assert!(
            seen.len().abs_diff(expected) <= 1,
            "{} frame changes, expected about {expected}",
            seen.len()
        );
        // In the mud, at half speed, the same distance takes the same frames.
        let mut slow = standing_player();
        let mut fast = standing_player();
        for _ in 0..30 {
            fast.update(&world, Intent { walk: 1.0, jump: false }, DT);
        }
        for _ in 0..60 {
            slow.update(&world, Intent { walk: 0.5, jump: false }, DT);
        }
        assert_eq!(
            slow.frame().index,
            fast.frame().index,
            "the same distance shows the same frame"
        );
    }

    #[test]
    fn jumping_rises_then_falls_in_their_own_frames() {
        let world = flat_world();
        let mut player = standing_player();
        player.update(&world, Intent { walk: 0.0, jump: true }, DT);
        assert_eq!(player.frame().index, frames::RISE);
        while player.vel[1] < 0.0 {
            player.update(&world, Intent::default(), DT);
        }
        player.update(&world, Intent::default(), DT);
        assert_eq!(player.frame().index, frames::FALL);
    }

    #[test]
    fn an_action_turns_toward_its_target_and_plays_the_arm_frames() {
        let world = flat_world();
        let mut player = standing_player();
        walk(&mut player, &world, 1.0, 0.2);
        player.act_toward(player.pos[0] - 3.0, USE_TIME);
        assert_eq!(player.frame().index, frames::USE[0]);
        assert!(player.frame().mirrored);
        // Walking on doesn't turn it back mid-action, and the arm swings on...
        walk(&mut player, &world, 1.0, USE_TIME * 0.6);
        let mid = player.frame();
        assert!(mid.mirrored && frames::USE[1..].contains(&mid.index), "{mid:?}");
        // ...but does once the action is over.
        walk(&mut player, &world, 1.0, USE_TIME);
        assert!(!player.frame().mirrored && frames::WALK.contains(&player.frame().index));
    }

    #[test]
    fn the_player_fits_a_two_wide_four_tall_tunnel_but_not_three() {
        assert!(HALF[0] * 2.0 < 2.0 && HALF[1] * 2.0 < 4.0 && HALF[1] * 2.0 > 3.0);
    }

    #[test]
    fn mining_removes_a_tile_within_reach_and_bumps_its_chunk() {
        let mut world = flat_world();
        let player = standing_player();
        let before = world.chunk_revision(0, 1);

        assert!(
            player.mine(&mut world, [21, GROUND]),
            "a tile at the player's feet is in reach"
        );
        assert_eq!(world.get(21, GROUND), AIR);
        assert!(
            world.chunk_revision(0, 1) > before,
            "the mined tile's chunk should be bumped so its cache rebuilds"
        );
    }

    #[test]
    fn mining_ignores_targets_out_of_reach_and_tiles_that_are_not_solid() {
        let mut world = flat_world();
        let player = standing_player();

        assert!(
            !player.mine(&mut world, [60, GROUND]),
            "far tile should be out of reach"
        );
        assert_eq!(world.get(60, GROUND), STONE);
        assert!(!player.mine(&mut world, [20, GROUND - 5]), "air is not mineable");
        assert!(
            !player.mine(&mut world, [-1, GROUND]),
            "outside the world is not mineable"
        );
    }

    #[test]
    fn a_trunk_is_chopped_and_walked_through_but_its_ground_cannot_be_mined() {
        use crate::tiles::TRUNK;
        use crate::trees::Tree;
        let mut world = flat_world();
        for i in 0..8 {
            world.set(22, GROUND - 1 - i, TRUNK);
        }
        world.forest.trees.push(Tree {
            x: 22,
            ground: GROUND,
            width: 1,
            height: 8,
            full: 8,
            seed: 1,
        });
        let mut player = standing_player();

        assert!(
            !player.mine(&mut world, [22, GROUND]),
            "a standing tree anchors its ground"
        );
        assert!(player.mine(&mut world, [22, GROUND - 2]), "trunk cells are choppable");
        for _ in 0..30 {
            player.update(&world, Intent { walk: 1.0, jump: false }, DT);
        }
        assert!(
            player.pos[0] > 23.0,
            "the player walks in front of the trunk, not into it"
        );
    }

    #[test]
    fn placing_puts_a_tile_in_empty_space_within_reach() {
        use crate::tiles::{STONE, TORCH};
        let mut world = flat_world();
        let player = standing_player();

        assert!(
            player.place(&mut world, [22, GROUND - 2], TORCH),
            "empty space in reach"
        );
        assert_eq!(world.get(22, GROUND - 2), TORCH);

        assert!(!player.place(&mut world, [22, GROUND - 2], TORCH), "already occupied");
        assert!(!player.place(&mut world, [60, GROUND - 2], TORCH), "out of reach");
    }

    #[test]
    fn a_solid_tile_cannot_be_placed_inside_the_player() {
        use crate::tiles::{STONE, TORCH};
        let mut world = flat_world();
        let player = standing_player();
        let inside = [player.pos[0] as i32, player.pos[1] as i32];

        assert!(!player.place(&mut world, inside, STONE), "would entomb the player");
        assert!(
            player.place(&mut world, inside, TORCH),
            "a torch is not solid, so it is fine"
        );
    }

    #[test]
    fn running_up_and_down_steps_keeps_full_speed_and_the_run() {
        // A staircase: up one tile at 45, down one at 52.
        let mut world = flat_world();
        for x in 45..52 {
            world.set(x, GROUND - 1, STONE);
        }
        let mut player = Player::new([40.0, GROUND as f32 - HALF[1]]);
        walk(&mut player, &world, 1.0, 0.1);
        let mut top = false;
        for _ in 0..((16.0 / WALK_SPEED) / DT) as i32 {
            player.update(&world, Intent { walk: 1.0, jump: false }, DT);
            assert!(player.grounded, "left the ground at x {}", player.pos[0]);
            assert_eq!(player.vel[0], WALK_SPEED, "lost speed at x {}", player.pos[0]);
            assert!(
                frames::WALK.contains(&player.frame().legs),
                "run broke at x {}",
                player.pos[0]
            );
            top |= (player.pos[1] - (GROUND as f32 - 1.0 - HALF[1])).abs() < 0.01;
        }
        assert!(top, "never climbed the step");
        assert!(
            player.pos[0] > 54.0 && (player.pos[1] - (GROUND as f32 - HALF[1])).abs() < 0.01,
            "not back down"
        );
    }

    #[test]
    fn a_two_tile_drop_is_still_a_fall() {
        let mut world = flat_world();
        for x in 0..45 {
            world.set(x, GROUND - 1, STONE);
            world.set(x, GROUND - 2, STONE);
        }
        let mut player = Player::new([43.0, GROUND as f32 - 2.0 - HALF[1]]);
        let mut fell = false;
        for _ in 0..30 {
            player.update(&world, Intent { walk: 1.0, jump: false }, DT);
            fell |= player.frame().legs == frames::FALL;
        }
        assert!(fell, "snapped down two tiles");
    }

    #[test]
    fn swinging_while_running_keeps_the_legs_running() {
        let world = flat_world();
        let mut player = standing_player();
        walk(&mut player, &world, 1.0, 0.2);
        player.act_toward(player.pos[0] - 3.0, USE_TIME);
        walk(&mut player, &world, 1.0, USE_TIME * 0.5);
        let f = player.frame();
        assert!(
            frames::USE.contains(&f.index) && frames::WALK.contains(&f.legs),
            "{f:?}"
        );
        assert!(f.mirrored, "faces the swing while running away: a moonwalk");
    }

    #[test]
    fn a_step_is_drawn_catching_up_rather_than_jumping() {
        let mut world = flat_world();
        for x in 45..64 {
            world.set(x, GROUND - 1, STONE);
        }
        let mut player = Player::new([44.0, GROUND as f32 - HALF[1]]);
        let walk = Intent { walk: 1.0, jump: false };

        let mut drawn_at_step = None;
        for _ in 0..60 {
            let before = player.pos[1];
            player.update(&world, walk, DT);
            if player.pos[1] < before - 0.5 && drawn_at_step.is_none() {
                drawn_at_step = Some(player.visual_pos()[1]);
            }
        }
        let drawn = drawn_at_step.expect("the player should have stepped");
        assert!(
            drawn > player.pos[1] + 0.5,
            "drawn position jumped with the step: {drawn} vs {}",
            player.pos[1]
        );

        // And it settles rather than lagging forever.
        for _ in 0..60 {
            player.update(&world, Intent::default(), DT);
        }
        assert!(
            (player.visual_pos()[1] - player.pos[1]).abs() < 0.001,
            "step lag never cleared"
        );
    }

    #[test]
    fn a_jump_leaves_the_ground_and_returns_to_it() {
        let world = flat_world();
        let mut player = standing_player();
        let resting = player.pos[1];

        player.update(&world, Intent { walk: 0.0, jump: true }, DT);
        assert!(!player.grounded, "should be airborne the frame after jumping");

        let mut peak = player.pos[1];
        for _ in 0..200 {
            player.update(&world, Intent::default(), DT);
            peak = peak.min(player.pos[1]);
        }
        let height = resting - peak;
        assert!(
            (4.0..7.0).contains(&height),
            "jump peaked {height} tiles above the ground"
        );
        assert!(player.grounded, "should have landed again");
        assert!(
            (player.pos[1] - resting).abs() < 0.01,
            "landed at {} not {resting}",
            player.pos[1]
        );
    }

    #[test]
    fn walking_off_a_ledge_falls() {
        let mut world = World::new(64, 64);
        for x in 0..20 {
            for y in GROUND..40 {
                world.set(x, y, STONE);
            }
        }
        let mut player = Player::new([18.0, GROUND as f32 - HALF[1]]);
        for _ in 0..120 {
            player.update(&world, Intent { walk: 1.0, jump: false }, DT);
        }
        assert!(player.pos[1] > GROUND as f32, "should have fallen past the ledge");
    }

    #[test]
    fn drag_is_unimpeded_in_air_and_on_solid_ground() {
        let world = flat_world();
        let player = standing_player();
        assert_eq!(medium_drag(&world, player.pos, HALF), 1.0);
    }
}
