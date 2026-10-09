//! Seeded world generation as an ordered list of passes over the tile grid.
//!
//! Each pass reads and writes the world directly; new passes (ores, caves,
//! structures, biomes) are added to `PASSES` without changing the driver.

use crate::atlas::DIRS;
use crate::tiles::{AIR, DECOR_GRASS, DIRT, STONE};
use crate::trees::{self, MAX_HEIGHT, MIN_HEIGHT, Tree};
use crate::world::World;

type GenPass = fn(&mut World, u64);

pub const PASSES: &[(&str, GenPass)] =
    &[("terrain", terrain_pass), ("grass", grass_pass), ("trees", tree_pass)];

pub fn generate(world: &mut World, seed: u64) {
    for (_name, pass) in PASSES {
        pass(world, seed);
    }
    world.snapshot_surface();
}

/// Surface row for a column: lower y is higher ground, row 0 is sky.
pub fn surface_y(world_height: i32, seed: u64, x: i32) -> i32 {
    let base = world_height as f32 * 0.35;
    let mut offset = 0.0;
    let mut amplitude = 26.0;
    let mut frequency = 1.0 / 320.0;
    for octave in 0..4u64 {
        let n = value_noise(seed ^ octave.wrapping_mul(0x5BF0_3635_9E37_79B9), x as f32 * frequency);
        offset += (n - 0.5) * 2.0 * amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    (base + offset).round().clamp(8.0, (world_height - 64) as f32) as i32
}

fn terrain_pass(world: &mut World, seed: u64) {
    let height = world.height();
    for x in 0..world.width() {
        let surface = surface_y(height, seed, x);
        // Smooth rather than per-column random, so the dirt/stone boundary follows
        // the terrain instead of jittering between neighbouring columns.
        let depth = 4 + (value_noise(seed ^ 0x9E37, x as f32 / 64.0) * 9.0) as i32;
        for y in surface..height {
            world.set(x, y, if y < surface + depth { DIRT } else { STONE });
        }
    }
}

/// Marks dirt touching air as grassy. Grass is decoration on dirt, not a tile of
/// its own, so this seeds the starting state only — spreading it is gameplay.
///
/// Adjacency includes diagonals. Checking only the tile above leaves the inner
/// corner of every step bare: the tile under a step's lip touches air only
/// diagonally, so the grass running down one face stops before the grass on the
/// next face begins.
fn grass_pass(world: &mut World, _seed: u64) {
    for x in 0..world.width() {
        for y in 0..world.height() {
            if world.get(x, y) != DIRT {
                continue;
            }
            let exposed = DIRS.iter().any(|(dx, dy)| world.get(x + dx, y + dy) == AIR);
            if exposed {
                world.set_decor(x, y, DECOR_GRASS);
            }
        }
    }
}

/// Plants trees on grass, one or two wide, with mixed spacing: a quarter of gaps
/// are 2-3 tiles, so trees sometimes cluster, the rest 8-14.
fn tree_pass(world: &mut World, seed: u64) {
    let seed = seed ^ 0x7EE5_7EE5;
    let mut x = 4;
    while x < world.width() - 4 {
        let r = hash(seed, x as i64);
        let width = if r % 10 < 3 { 2 } else { 1 };
        let height = MIN_HEIGHT + ((r >> 8) % (MAX_HEIGHT - MIN_HEIGHT + 1) as u64) as i32;
        let ground = world.highest_solid(x);
        let fits = (0..width).all(|c| {
            world.highest_solid(x + c) == ground && world.decor(x + c, ground) & DECOR_GRASS != 0
        }) && ground - height > 1;
        if !fits {
            x += 1;
            continue;
        }
        trees::plant(world, Tree { x, ground, width, height, full: height, seed: hash(seed, -(x as i64)) });
        let gap = if (r >> 20) % 4 == 0 { 2 + (r >> 24) % 2 } else { 8 + (r >> 24) % 7 };
        x += width + gap as i32;
    }
}

/// Smoothstep-interpolated value noise. Deterministic for a given seed, so the
/// same seed always yields the same world.
// ponytail: hand-rolled 1D value noise; swap in a simplex/Perlin crate when caves
// and biomes need coherent 2D noise.
fn value_noise(seed: u64, x: f32) -> f32 {
    let x0 = x.floor();
    let t = x - x0;
    let t = t * t * (3.0 - 2.0 * t);
    let a = unit_rand(seed, x0 as i64);
    let b = unit_rand(seed, x0 as i64 + 1);
    a + (b - a) * t
}

fn unit_rand(seed: u64, x: i64) -> f32 {
    (hash(seed, x) >> 40) as f32 / (1u64 << 24) as f32
}

pub fn hash(seed: u64, x: i64) -> u64 {
    let mut h = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{TRUNK, drawn_as, is_solid};
    use crate::world::{CHUNK_SIZE, WORLD_HEIGHT, WORLD_WIDTH};

    fn generated(seed: u64) -> World {
        let mut w = World::new(WORLD_WIDTH, WORLD_HEIGHT);
        generate(&mut w, seed);
        w
    }

    #[test]
    fn same_seed_reproduces_the_same_world() {
        let (a, b) = (generated(1337), generated(1337));
        for cy in 0..a.height_chunks() {
            for cx in 0..a.width_chunks() {
                assert_eq!(a.chunk(cx, cy).ids, b.chunk(cx, cy).ids, "chunk ({cx}, {cy}) differs");
            }
        }
    }

    #[test]
    fn different_seeds_produce_different_worlds() {
        let (a, b) = (generated(1337), generated(7331));
        // Chunk row 0 is pure sky at this world height, so compare the whole grid.
        let differs = (0..a.height_chunks())
            .any(|cy| (0..a.width_chunks()).any(|cx| a.chunk(cx, cy).ids != b.chunk(cx, cy).ids));
        assert!(differs, "two seeds generated identical worlds");
    }

    #[test]
    fn grass_marks_exactly_the_dirt_that_touches_air() {
        let w = generated(42);
        for x in 0..w.width() {
            for y in 0..w.height() {
                let grassy = w.decor(x, y) & DECOR_GRASS != 0;
                // Trunk cells stand in open air, so dirt under a tree still counts as exposed.
                let exposed = DIRS.iter().any(|(dx, dy)| drawn_as(w.get(x + dx, y + dy)) == AIR);
                if grassy {
                    assert_eq!(w.get(x, y), DIRT, "grass on a non-dirt tile at ({x}, {y})");
                    assert!(exposed, "buried grass at ({x}, {y})");
                }
                if w.get(x, y) == DIRT && exposed {
                    assert!(grassy, "dirt touching air left bare at ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn grass_is_continuous_around_the_inner_corner_of_a_step() {
        // A single step down to the right: the tile under the lip touches air only
        // diagonally, and is what used to leave a bare notch in the grass.
        let mut w = World::new(64, 64);
        for x in 0..64 {
            let surface = if x < 32 { 20 } else { 21 };
            for y in surface..64 {
                w.set(x, y, DIRT);
            }
        }
        grass_pass(&mut w, 0);

        assert!(w.decor(31, 20) & DECOR_GRASS != 0, "the step's top should be grassy");
        assert!(w.decor(32, 21) & DECOR_GRASS != 0, "the lower step should be grassy");
        assert!(
            w.decor(31, 21) & DECOR_GRASS != 0,
            "the tile under the lip touches air diagonally and must carry grass too"
        );
    }

    #[test]
    fn every_column_has_open_sky_above_solid_ground() {
        let w = generated(99);
        for x in 0..w.width() {
            let surface = surface_y(w.height(), 99, x);
            assert!(surface >= 8 && surface < w.height() - 64, "surface {surface} out of range");
            assert!(!is_solid(w.get(x, surface - 1)), "column {x} is buried at the surface");
            assert!(is_solid(w.get(x, surface)), "column {x} has no ground at the surface");
            assert!(is_solid(w.get(x, w.height() - 1)), "column {x} is hollow at the bottom");
        }
    }

    #[test]
    fn trees_stand_on_grass_within_the_height_range_and_spaced_as_designed() {
        let w = generated(42);
        let trees = &w.forest.trees;
        assert!(trees.len() > 100, "only {} trees on a {}-wide world", trees.len(), w.width());
        let mut close = 0;
        for pair in trees.windows(2) {
            let gap = pair[1].x - (pair[0].x + pair[0].width);
            assert!(gap >= 2, "trees overlap or touch at x={}", pair[1].x);
            close += (gap <= 3) as usize;
        }
        assert!(close > 0 && close < trees.len() / 2, "{close} close gaps of {}", trees.len());
        for t in trees {
            assert!((MIN_HEIGHT..=MAX_HEIGHT).contains(&t.height));
            for c in 0..t.width {
                assert!(w.decor(t.x + c, t.ground) & DECOR_GRASS != 0, "tree at {} not on grass", t.x);
                assert!((0..t.height).all(|i| w.get(t.x + c, t.row_y(i)) == TRUNK));
            }
        }
        assert!(trees.iter().any(|t| t.width == 2) && trees.iter().any(|t| t.width == 1));
    }

    #[test]
    fn terrain_is_continuous_between_adjacent_columns() {
        let w = generated(5);
        // A one-column cliff taller than a chunk would mean the noise is aliasing.
        for x in 1..w.width() {
            let step = (surface_y(w.height(), 5, x) - surface_y(w.height(), 5, x - 1)).abs();
            assert!(step < CHUNK_SIZE, "surface jumps {step} tiles at column {x}");
        }
    }
}
