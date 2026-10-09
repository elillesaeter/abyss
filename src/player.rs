//! The player: input intent, gravity, medium drag, and mining.
//!
//! Stored as a plain struct because there is exactly one of them. The physics it
//! uses are free functions over `(pos, vel, half, &World)`, so the same movement
//! code will drive mobs unchanged — that's what keeps a later move to an ECS a
//! change of where positions live rather than a rewrite of how bodies move.

use crate::physics::{self, Contacts};
use crate::tiles::{AIR, TileId, is_breakable, is_solid, tile_type};
use crate::world::World;

/// Half-extents in tiles: a 1.5 x 2.6 box on a 2x3 visual footprint, so there is
/// slack inside a two-wide tunnel and tile seams can't wedge the player.
pub const HALF: [f32; 2] = [0.75, 1.3];
/// Drawn extents. Larger than the hitbox, as a character sprite always is: the
/// body is what collides, this is what you see.
pub const SPRITE_HALF: [f32; 2] = [1.0, 1.5];

pub const WALK_SPEED: f32 = 9.0;
/// v^2 / 2g with these values peaks a little over 5 tiles.
pub const JUMP_SPEED: f32 = 21.5;
/// Mining range from the player's centre, in tiles.
pub const REACH: f32 = 5.5;

/// How long the player must be pressed against a step before taking it. Without a
/// wind-up a step fires the instant you touch it, which makes it hard to stand at
/// the foot of one — to dig there, say — because walking into it lifts you straight
/// on top.
pub const STEP_DELAY: f32 = 0.11;
/// How quickly the drawn position catches up after a step, per second. The step
/// itself is instantaneous in collision terms; this is what stops it looking like
/// a teleport.
const STEP_CATCH_UP: f32 = 16.0;

pub const COLOR: [u8; 4] = [226, 198, 160, 255];

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
    /// How long the player has been pressed against a steppable obstruction.
    step_press: f32,
    /// Remaining vertical lag, in tiles, between the drawn and actual position.
    step_lag: f32,
}

impl Player {
    pub fn new(pos: [f32; 2]) -> Player {
        Player { pos, vel: [0.0, 0.0], grounded: false, step_press: 0.0, step_lag: 0.0 }
    }

    /// Where the player is drawn: the true position plus the unspent step lag.
    pub fn visual_pos(&self) -> [f32; 2] {
        [self.pos[0], self.pos[1] + self.step_lag]
    }

    pub fn update(&mut self, world: &World, intent: Intent, dt: f32) -> Contacts {
        // Grounded from the start of the frame: it gates both jumping and step-up.
        self.grounded = physics::grounded(world, self.pos, HALF);
        let drag = medium_drag(world, self.pos, HALF);

        self.vel[0] = intent.walk * WALK_SPEED * drag;
        if intent.jump && self.grounded {
            self.vel[1] = -JUMP_SPEED;
        }
        self.vel[1] = (self.vel[1] + physics::GRAVITY * dt).min(physics::MAX_FALL * drag);

        // Stepping needs ground under the body and a moment of intent; `supported`
        // rather than `grounded` because the body may be a fraction of a tile up.
        let may_step_up =
            physics::supported(world, self.pos, HALF) && self.step_press >= STEP_DELAY;
        let contacts =
            physics::move_box(&mut self.pos, &mut self.vel, HALF, world, dt, may_step_up);

        if contacts.stepped_up {
            self.step_press = 0.0;
            self.step_lag += physics::STEP_HEIGHT;
        } else if contacts.step_available {
            self.step_press += dt;
        } else {
            self.step_press = 0.0;
        }
        self.step_lag *= (-STEP_CATCH_UP * dt).exp();
        if self.step_lag < 0.01 {
            self.step_lag = 0.0;
        }
        self.grounded = physics::grounded(world, self.pos, HALF);
        contacts
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
    pos[0] - half[0] < x + 1.0
        && pos[0] + half[0] > x
        && pos[1] - half[1] < y + 1.0
        && pos[1] + half[1] > y
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

    #[test]
    fn mining_removes_a_tile_within_reach_and_bumps_its_chunk() {
        let mut world = flat_world();
        let player = standing_player();
        let before = world.chunk_revision(0, 1);

        assert!(player.mine(&mut world, [21, GROUND]), "a tile at the player's feet is in reach");
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

        assert!(!player.mine(&mut world, [60, GROUND]), "far tile should be out of reach");
        assert_eq!(world.get(60, GROUND), STONE);
        assert!(!player.mine(&mut world, [20, GROUND - 5]), "air is not mineable");
        assert!(!player.mine(&mut world, [-1, GROUND]), "outside the world is not mineable");
    }

    #[test]
    fn a_trunk_is_chopped_and_walked_through_but_its_ground_cannot_be_mined() {
        use crate::tiles::TRUNK;
        use crate::trees::Tree;
        let mut world = flat_world();
        for i in 0..8 {
            world.set(22, GROUND - 1 - i, TRUNK);
        }
        world.forest.trees.push(Tree { x: 22, ground: GROUND, width: 1, height: 8, full: 8, seed: 1 });
        let mut player = standing_player();

        assert!(!player.mine(&mut world, [22, GROUND]), "a standing tree anchors its ground");
        assert!(player.mine(&mut world, [22, GROUND - 2]), "trunk cells are choppable");
        for _ in 0..30 {
            player.update(&world, Intent { walk: 1.0, jump: false }, DT);
        }
        assert!(player.pos[0] > 23.0, "the player walks in front of the trunk, not into it");
    }

    #[test]
    fn placing_puts_a_tile_in_empty_space_within_reach() {
        use crate::tiles::{STONE, TORCH};
        let mut world = flat_world();
        let player = standing_player();

        assert!(player.place(&mut world, [22, GROUND - 2], TORCH), "empty space in reach");
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
        assert!(player.place(&mut world, inside, TORCH), "a torch is not solid, so it is fine");
    }

    #[test]
    fn a_step_waits_for_sustained_intent_then_takes_it() {
        let mut world = flat_world();
        for x in 45..64 {
            world.set(x, GROUND - 1, STONE);
        }
        let mut player = Player::new([44.0, GROUND as f32 - HALF[1]]);
        let walk = Intent { walk: 1.0, jump: false };

        // A brief tap should leave the player at the foot of the step.
        for _ in 0..((STEP_DELAY / DT) as i32 / 2) {
            player.update(&world, walk, DT);
        }
        assert!(
            (player.pos[1] - (GROUND as f32 - HALF[1])).abs() < 0.01,
            "stepped before the wind-up elapsed, at y {}",
            player.pos[1]
        );

        for _ in 0..60 {
            player.update(&world, walk, DT);
        }
        assert!(
            (player.pos[1] - (GROUND as f32 - 1.0 - HALF[1])).abs() < 0.01,
            "never took the step, at y {}",
            player.pos[1]
        );
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
        assert!((4.0..7.0).contains(&height), "jump peaked {height} tiles above the ground");
        assert!(player.grounded, "should have landed again");
        assert!((player.pos[1] - resting).abs() < 0.01, "landed at {} not {resting}", player.pos[1]);
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
