//! Axis-separated AABB movement against the tile grid.
//!
//! Horizontal is resolved before vertical: falling toward the corner of a one-tile
//! step then completes the sideways move while still above the step's surface and
//! lands on top of it, rather than dropping beside it and stopping at its wall.
//! Step-up lives inside the horizontal pass — resolving vertically first would
//! leave nothing to step onto — and is gated on having been grounded, or a player
//! held against a cliff would climb it.

use crate::tiles::is_solid;
use crate::world::World;

/// Keeps a body from passing through a tile within one step. Movement longer than
/// this is split, so fast projectiles can't tunnel.
pub const MAX_STEP: f32 = 0.5;
/// Height a body can climb without jumping.
pub const STEP_HEIGHT: f32 = 1.0;

/// Downward acceleration, in tiles per second squared. Shared by every body.
pub const GRAVITY: f32 = 45.0;
/// Terminal fall speed, in tiles per second.
pub const MAX_FALL: f32 = 38.0;

/// Slack kept at contact, so a body resting on a boundary is never counted as
/// overlapping the tile it stands on.
const EPS: f32 = 1e-4;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Contacts {
    pub left: bool,
    pub right: bool,
    pub above: bool,
    pub below: bool,
    pub stepped_up: bool,
    /// Horizontal movement was blocked by something a single step would clear, but
    /// stepping was not permitted this frame. Callers use it to decide when to allow
    /// the step — a wind-up, a cooldown, or immediately.
    pub step_available: bool,
}

/// Solidity including the world boundary: the sides and the bottom are walls, the
/// sky above is open. Without this a body could walk off the edge and fall forever.
pub fn solid_at(world: &World, x: i32, y: i32) -> bool {
    if x < 0 || x >= world.width() || y >= world.height() {
        return true;
    }
    if y < 0 {
        return false;
    }
    is_solid(world.get(x, y))
}

/// Tile range a box covers, inset by `EPS` so touching an edge doesn't count.
fn covered(center: f32, half: f32) -> (i32, i32) {
    (
        (center - half + EPS).floor() as i32,
        (center + half - EPS).floor() as i32,
    )
}

pub fn overlaps_solid(world: &World, pos: [f32; 2], half: [f32; 2]) -> bool {
    let (x0, x1) = covered(pos[0], half[0]);
    let (y0, y1) = covered(pos[1], half[1]);
    (y0..=y1).any(|y| (x0..=x1).any(|x| solid_at(world, x, y)))
}

/// Whether solid tiles sit immediately below the box. A probe rather than a flag
/// set during collision, so it stays correct no matter what moved this frame.
pub fn grounded(world: &World, pos: [f32; 2], half: [f32; 2]) -> bool {
    let y = (pos[1] + half[1] + 2.0 * EPS).floor() as i32;
    let (x0, x1) = covered(pos[0], half[0]);
    (x0..=x1).any(|x| solid_at(world, x, y))
}

/// Solid ground within a step's height below the feet.
///
/// Step-up is gated on this rather than on `grounded`, because a climb necessarily
/// lifts the body off the ground: gating on `grounded` lets the first frame rise and
/// then refuses every frame after, so the body oscillates at the foot of the step
/// forever instead of climbing it.
pub fn supported(world: &World, pos: [f32; 2], half: [f32; 2]) -> bool {
    let feet = pos[1] + half[1];
    let (x0, x1) = covered(pos[0], half[0]);
    let (y0, y1) = ((feet + 2.0 * EPS).floor() as i32, (feet + STEP_HEIGHT).floor() as i32);
    (y0..=y1).any(|y| (x0..=x1).any(|x| solid_at(world, x, y)))
}

/// Moves a box by `vel * dt`, resolving x then y, zeroing velocity on contact.
/// `may_step_up` should come from `supported` at the start of the frame.
pub fn move_box(
    pos: &mut [f32; 2],
    vel: &mut [f32; 2],
    half: [f32; 2],
    world: &World,
    dt: f32,
    may_step_up: bool,
) -> Contacts {
    let total = [vel[0] * dt, vel[1] * dt];
    let longest = total[0].abs().max(total[1].abs());
    let steps = (longest / MAX_STEP).ceil().max(1.0) as i32;
    let step = [total[0] / steps as f32, total[1] / steps as f32];

    let mut contacts = Contacts::default();
    for _ in 0..steps {
        resolve_axis(0, step[0], pos, vel, half, world, may_step_up, &mut contacts);
        resolve_axis(1, step[1], pos, vel, half, world, may_step_up, &mut contacts);
    }
    contacts
}

#[allow(clippy::too_many_arguments)]
fn resolve_axis(
    axis: usize,
    delta: f32,
    pos: &mut [f32; 2],
    vel: &mut [f32; 2],
    half: [f32; 2],
    world: &World,
    may_step_up: bool,
    contacts: &mut Contacts,
) {
    if delta == 0.0 {
        return;
    }
    let mut candidate = *pos;
    candidate[axis] += delta;
    if !overlaps_solid(world, candidate, half) {
        *pos = candidate;
        return;
    }

    // Horizontal obstruction: a step clears it if the body fits where it would end
    // up. Only the destination is tested — requiring clearance above the body's
    // current position too would refuse every step inside a tunnel just tall enough
    // to stand in, which is exactly the tunnel a player digs.
    if axis == 0 {
        let mut lifted = candidate;
        lifted[1] -= STEP_HEIGHT;
        if !overlaps_solid(world, lifted, half) {
            if may_step_up {
                *pos = lifted;
                // A step is deliberate upward motion; gravity accumulated this frame
                // would otherwise pull most of it straight back.
                vel[1] = vel[1].min(0.0);
                contacts.stepped_up = true;
                return;
            }
            contacts.step_available = true;
        }
    }

    // A step never exceeds MAX_STEP, so the obstruction is the tile the leading
    // edge just entered; snap flush against its near boundary.
    if delta > 0.0 {
        let blocking = (candidate[axis] + half[axis] - EPS).floor();
        pos[axis] = blocking - half[axis] - EPS;
        if axis == 0 {
            contacts.right = true;
        } else {
            contacts.below = true;
        }
    } else {
        let blocking = (candidate[axis] - half[axis] + EPS).floor();
        pos[axis] = blocking + 1.0 + half[axis] + EPS;
        if axis == 0 {
            contacts.left = true;
        } else {
            contacts.above = true;
        }
    }
    vel[axis] = 0.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{AIR, STONE};

    const HALF: [f32; 2] = [0.75, 1.3];
    const GROUND: i32 = 32;
    const DT: f32 = 1.0 / 60.0;

    /// 64x64 world, solid from row `GROUND` down.
    fn flat_world() -> World {
        let mut w = World::new(64, 64);
        for x in 0..w.width() {
            for y in GROUND..w.height() {
                w.set(x, y, STONE);
            }
        }
        w
    }

    /// Centre height of a box resting on top of `row`.
    fn resting_on(row: i32) -> f32 {
        row as f32 - HALF[1]
    }

    #[test]
    fn walking_into_a_wall_stops_without_penetration() {
        let mut world = flat_world();
        for y in GROUND - 6..GROUND {
            world.set(45, y, STONE);
        }
        let mut pos = [40.0, resting_on(GROUND)];
        let mut vel = [9.0, 0.0];
        for _ in 0..120 {
            vel[0] = 9.0;
            move_box(&mut pos, &mut vel, HALF, &world, DT, true);
        }
        assert!(
            pos[0] + HALF[0] <= 45.0,
            "penetrated the wall: right edge {}",
            pos[0] + HALF[0]
        );
        assert!(
            pos[0] + HALF[0] > 44.9,
            "stopped short of the wall: right edge {}",
            pos[0] + HALF[0]
        );
    }

    #[test]
    fn falling_onto_a_step_corner_lands_on_top_not_beside_it() {
        let mut world = flat_world();
        for x in 40..64 {
            world.set(x, GROUND - 1, STONE);
        }
        let mut pos = [39.0, 29.0];
        let mut vel = [9.0, 12.0];
        for _ in 0..90 {
            vel[0] = 9.0;
            vel[1] = (vel[1] + 45.0 * DT).min(38.0);
            let on_ground = supported(&world, pos, HALF);
            move_box(&mut pos, &mut vel, HALF, &world, DT, on_ground);
        }
        assert!(pos[0] > 40.0, "never cleared the step: x {}", pos[0]);
        assert!(
            (pos[1] - resting_on(GROUND - 1)).abs() < 0.01,
            "landed at y {} instead of on top of the step at {}",
            pos[1],
            resting_on(GROUND - 1)
        );
    }

    #[test]
    fn a_step_is_offered_when_refused_so_the_caller_can_decide_when_to_take_it() {
        let mut world = flat_world();
        world.set(45, GROUND - 1, STONE);
        let mut pos = [44.0, resting_on(GROUND)];
        let mut vel = [9.0, 0.0];

        let mut offered = false;
        for _ in 0..30 {
            vel[0] = 9.0;
            let contacts = move_box(&mut pos, &mut vel, HALF, &world, DT, false);
            offered |= contacts.step_available;
            assert!(!contacts.stepped_up, "must not step while refused");
        }
        assert!(offered, "being blocked by a one-tile step should offer the step");
        assert!(
            pos[0] + HALF[0] <= 45.01,
            "stepped anyway: right edge {}",
            pos[0] + HALF[0]
        );
    }

    #[test]
    fn a_step_works_in_a_tunnel_with_no_room_to_rise_twice() {
        // A dug tunnel: solid ceiling, four tiles of air, floor, and the floor rising
        // by one partway along. The body fits at the destination, but there is not
        // room above it to clear a further step's height — which is what a rise
        // computed one fraction at a time ends up demanding, and why it stalled.
        let mut world = flat_world();
        for x in 0..64 {
            for y in 0..=GROUND - 5 {
                world.set(x, y, STONE);
            }
            if x >= 45 {
                world.set(x, GROUND - 1, STONE);
            }
        }

        let mut pos = [42.0, resting_on(GROUND)];
        let mut vel = [0.0, 0.0];
        // Room for one step above the head, but not for two.
        let headroom = pos[1] - HALF[1] - (GROUND - 4) as f32;
        assert!(
            (STEP_HEIGHT..2.0 * STEP_HEIGHT).contains(&headroom),
            "test geometry should leave {headroom} tiles of headroom, between one and two steps"
        );

        for _ in 0..120 {
            vel[0] = 9.0;
            vel[1] = (vel[1] + 45.0 * DT).min(38.0);
            let on_ground = supported(&world, pos, HALF);
            move_box(&mut pos, &mut vel, HALF, &world, DT, on_ground);
        }
        assert!(
            (pos[1] - resting_on(GROUND - 1)).abs() < 0.01,
            "failed to step inside the tunnel, ended at y {}",
            pos[1]
        );
        assert!(pos[0] > 46.0, "did not get past the step: x {}", pos[0]);
    }

    #[test]
    fn step_up_climbs_one_tile_but_not_two() {
        for (height, should_climb) in [(1, true), (2, false)] {
            let mut world = flat_world();
            for up in 1..=height {
                world.set(45, GROUND - up, STONE);
            }
            let mut pos = [42.0, resting_on(GROUND)];
            let mut vel = [9.0, 0.0];
            for _ in 0..90 {
                vel[0] = 9.0;
                vel[1] = (vel[1] + 45.0 * DT).min(38.0);
                let on_ground = supported(&world, pos, HALF);
                move_box(&mut pos, &mut vel, HALF, &world, DT, on_ground);
            }
            if should_climb {
                assert!(pos[0] > 46.0, "failed to step up one tile: x {}", pos[0]);
                assert!((pos[1] - resting_on(GROUND)).abs() < 0.01 || pos[1] < resting_on(GROUND));
            } else {
                assert!(
                    pos[0] + HALF[0] <= 45.01,
                    "climbed a two-tile step: right edge {}",
                    pos[0] + HALF[0]
                );
            }
        }
    }

    #[test]
    fn steps_are_climbed_in_both_directions() {
        for direction in [-1.0f32, 1.0] {
            let mut world = flat_world();
            let step_x = if direction < 0.0 { 38 } else { 45 };
            // A step one tile high, with open sky above it.
            for x in 0..64 {
                let raised = if direction < 0.0 { x <= step_x } else { x >= step_x };
                if raised {
                    world.set(x, GROUND - 1, STONE);
                }
            }
            let mut pos = [42.0, resting_on(GROUND)];
            let mut vel = [0.0, 0.0];
            for _ in 0..120 {
                vel[0] = 9.0 * direction;
                vel[1] = (vel[1] + 45.0 * DT).min(38.0);
                let on_ground = supported(&world, pos, HALF);
                move_box(&mut pos, &mut vel, HALF, &world, DT, on_ground);
            }
            let climbed = (pos[1] - resting_on(GROUND - 1)).abs() < 0.01;
            assert!(
                climbed,
                "moving {direction} ended at y {} instead of on the step",
                pos[1]
            );
        }
    }

    #[test]
    fn a_step_under_a_low_ceiling_is_refused_rather_than_half_climbed() {
        // A three-tile corridor whose floor rises by one but whose ceiling does not:
        // the body is 2.6 tall, so it simply does not fit above the step.
        let mut world = flat_world();
        for x in 0..64 {
            for y in 0..GROUND - 3 {
                world.set(x, y, STONE);
            }
        }
        world.set(45, GROUND - 1, STONE);

        let mut pos = [42.0, resting_on(GROUND)];
        let mut vel = [0.0, 0.0];
        for _ in 0..120 {
            vel[0] = 9.0;
            vel[1] = (vel[1] + 45.0 * DT).min(38.0);
            let on_ground = supported(&world, pos, HALF);
            move_box(&mut pos, &mut vel, HALF, &world, DT, on_ground);
        }
        assert!(
            pos[0] + HALF[0] <= 45.01,
            "squeezed into a gap it cannot fit: x {}",
            pos[0]
        );
        assert!(
            (pos[1] - resting_on(GROUND)).abs() < 0.01,
            "left hovering mid-climb at y {} instead of settling",
            pos[1]
        );
    }

    #[test]
    fn step_up_is_gated_on_being_grounded() {
        let mut world = flat_world();
        world.set(45, GROUND - 1, STONE);
        // Airborne, but low enough that the box overlaps the step's row, so the
        // only thing deciding the outcome is the gate.
        let start = [42.0, GROUND as f32 - 0.5 - HALF[1]];
        assert!(!grounded(&world, start, HALF), "the test body must start airborne");
        // Airborne bodies get no climb at all, so the gate is what decides this.

        for may_step_up in [false, true] {
            let mut pos = start;
            let mut vel = [9.0, 0.0];
            for _ in 0..90 {
                vel[0] = 9.0;
                move_box(&mut pos, &mut vel, HALF, &world, DT, may_step_up);
            }
            if may_step_up {
                assert!(pos[0] > 46.0, "gate open: should have climbed, x {}", pos[0]);
            } else {
                assert!(
                    pos[0] + HALF[0] <= 45.01,
                    "climbed while airborne: right edge {}",
                    pos[0] + HALF[0]
                );
            }
        }
    }

    #[test]
    fn sliding_along_a_tiled_floor_does_not_snag() {
        let world = flat_world();
        let start = [10.0, resting_on(GROUND)];
        let mut pos = start;
        let mut vel = [9.0, 0.0];
        for _ in 0..180 {
            vel[0] = 9.0;
            vel[1] = (vel[1] + 45.0 * DT).min(38.0);
            move_box(&mut pos, &mut vel, HALF, &world, DT, true);
        }
        let travelled = pos[0] - start[0];
        assert!(travelled > 25.0, "snagged on floor seams after {travelled} tiles");
        assert!(
            (pos[1] - start[1]).abs() < 0.01,
            "drifted vertically along a flat floor to {}",
            pos[1]
        );
    }

    #[test]
    fn fast_movement_does_not_tunnel_through_a_wall() {
        let mut world = flat_world();
        for y in 0..64 {
            world.set(45, y, STONE);
        }
        let mut pos = [39.0, 20.0];
        let mut vel = [1200.0, 0.0]; // 20 tiles in one frame
        move_box(&mut pos, &mut vel, HALF, &world, DT, false);
        assert!(pos[0] + HALF[0] <= 45.01, "tunnelled through the wall to x {}", pos[0]);
        assert_eq!(vel[0], 0.0, "velocity should be cancelled by the contact");
    }

    #[test]
    fn grounded_is_true_on_the_ground_and_false_in_the_air() {
        let world = flat_world();
        assert!(grounded(&world, [20.0, resting_on(GROUND)], HALF));
        assert!(!grounded(&world, [20.0, resting_on(GROUND) - 3.0], HALF));
    }

    #[test]
    fn support_extends_one_step_below_the_feet_but_no_further() {
        let world = flat_world();
        let resting = resting_on(GROUND);
        assert!(supported(&world, [20.0, resting], HALF), "standing is supported");
        assert!(
            supported(&world, [20.0, resting - STEP_HEIGHT * 0.9], HALF),
            "mid-climb, a body just under a step's height up is still supported"
        );
        assert!(
            !supported(&world, [20.0, resting - 3.0], HALF),
            "a body well clear of the ground is not supported"
        );
    }

    #[test]
    fn the_world_boundary_is_solid() {
        let world = flat_world();
        let mut pos = [2.0, 20.0];
        let mut vel = [-600.0, 0.0];
        move_box(&mut pos, &mut vel, HALF, &world, DT, false);
        assert!(pos[0] - HALF[0] >= -0.01, "escaped through the left edge to {}", pos[0]);

        let open = World::new(64, 64); // all air: the floor is the world bottom
        let mut pos = [20.0, 60.0];
        let mut vel = [0.0, 600.0];
        move_box(&mut pos, &mut vel, HALF, &open, DT, false);
        assert!(pos[1] + HALF[1] <= 64.01, "fell through the world bottom to {}", pos[1]);
        assert_eq!(open.get(20, 60), AIR);
    }
}
