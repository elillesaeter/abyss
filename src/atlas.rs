//! The texture atlas: material textures plus the transition mask set.
//!
//! Layout:
//!   top band    — one 8x8-tile material block per material, sampled by world
//!                 position so the pattern repeats every 8 tiles instead of
//!                 resetting at every tile edge
//!   below it    — masks of one tile each: 256 transitions keyed by the 8-neighbour
//!                 bitmask of the bleeding material, `SOLID_MASK`, the leaf clumps,
//!                 then 256 thin edges for the lit rim and shaded lip facing air.
//!
//! The art follows pixel-art rules so procedural output reads as drawn: each
//! material keeps to a ramp of a few shades (shadows cooler, highlights warmer),
//! detail comes in shapes rather than per-pixel noise, every shape is lit from
//! above, and mask edges are hard — jagged fringes instead of soft blends.
//!
//! Masks are shapes, not art per material: an overlay samples the *bleeding
//! material's* texture and multiplies it by the mask's alpha. One overlay quad per
//! bleeding material therefore covers edges and corners together, which keeps
//! art cost at one texture per material and makes corner coverage automatic.
//!
//! Generated procedurally and written to `assets/tiles.png`, which is then loaded
//! at startup — so it can be replaced cell by cell with hand-drawn art later.

use std::path::Path;

use crate::tiles::{DECOR_TYPES, TILE_TYPES};

pub const CELL: u32 = 16;
/// A material texture spans this many tiles on each axis, which is what stops the
/// eye finding a per-tile repeat. Larger breaks it up further at linear atlas cost.
pub const BLOCK_TILES: u32 = 8;
pub const BLOCK: u32 = CELL * BLOCK_TILES;

pub const MASK_COLS: u32 = 16;
pub const MASK_ROWS: u32 = (MASK_COUNT as u32).div_ceil(MASK_COLS);
/// Masks start below the material band.
pub const MASK_BAND_Y: u32 = BLOCK;

pub const MATERIALS: u32 = 10;
pub const WIDTH: u32 = MATERIALS * BLOCK;
/// Character frames sit in a band below the masks: a row per paper-doll part,
/// painted at startup by `character::paint`, so the atlas file leaves it empty.
pub const SPRITE_BAND_Y: u32 = MASK_BAND_Y + MASK_ROWS * CELL;
/// A character frame, in art pixels: room for a 3.25-tile figure and its arms.
pub const FRAME_W: u32 = 40;
pub const FRAME_H: u32 = 64;
/// Frames per atlas row.
pub const FRAME_COLS: u32 = WIDTH / FRAME_W;
/// Small sprites (`icons`) sit in a grid of square cells below the character.
pub const ICON_BAND_Y: u32 = SPRITE_BAND_Y + crate::character::PARTS.len() as u32 * FRAME_H;
pub const ICON: u32 = 16;
pub const ICON_COLS: u32 = WIDTH / ICON;
pub const HEIGHT: u32 = ICON_BAND_Y + crate::icons::ROWS * ICON;

/// Material blocks, in order.
/// The darkening an exposed edge casts on its neighbour, not a surface itself.
pub const MAT_AIR_SHADOW: u16 = 0;
pub const MAT_DIRT: u16 = 1;
pub const MAT_STONE: u16 = 2;
pub const MAT_GRASS: u16 = 3;
pub const MAT_WHITE: u16 = 4;
pub const MAT_SKY: u16 = 5;
pub const MAT_BARK: u16 = 7;
pub const MAT_LEAVES: u16 = 8;
/// The warm highlight along a face lit from above.
pub const MAT_RIM: u16 = 9;

/// Fully opaque mask, used by base tiles and by untextured quads.
pub const SOLID_MASK: u16 = 256;
/// Irregular clumps of leaves, smallest first, for sparser and denser foliage.
pub const LEAF_MASK: u16 = 257;
pub const LEAF_VARIANTS: u16 = 4;
/// Thin edges, plus the 8-neighbour bitmask: the rim and lip a face to air gets.
pub const THIN_MASKS: u16 = LEAF_MASK + LEAF_VARIANTS;
pub const MASK_COUNT: u16 = THIN_MASKS + 256;

/// Open air. Drawn like any other tile so that unlit air reads as black rather
/// than as whatever the frame was cleared to.
pub const SKY_COLOR: [u8; 4] = [107, 153, 219, 255];
pub const LEAVES_COLOR: [u8; 4] = [74, 128, 58, 255];

/// How far a transition reaches into the tile, in pixels, before its fringe.
const BAND: f32 = 7.0;
/// How far a fringe's strands vary either side of `BAND`, in whole pixels: grass
/// hanging as blades, stone biting into dirt in ragged steps.
const FRINGE: i32 = 2;
/// Width of the lit rim and the shaded lip along faces to air.
const THIN: f32 = 2.0;

/// Shades darkest to lightest. Dark ends lean cool and violet, light ends warm:
/// muted and earthy, a little warm, a little threatening.
pub const DIRT_RAMP: [[u8; 3]; 5] = [[52, 34, 36], [84, 55, 45], [114, 79, 57], [143, 105, 73], [174, 136, 95]];
pub const STONE_RAMP: [[u8; 3]; 5] = [[40, 37, 49], [64, 59, 71], [92, 85, 91], [121, 112, 109], [152, 141, 127]];
pub const GRASS_RAMP: [[u8; 3]; 5] = [[48, 64, 40], [70, 94, 46], [96, 122, 54], [126, 148, 64], [160, 172, 84]];
pub const BARK_RAMP: [[u8; 3]; 5] = [[46, 30, 28], [74, 50, 40], [102, 74, 52], [130, 98, 68], [160, 126, 88]];

/// A pebbly material: stones of a ramp's lighter shades set in a matrix of its
/// darker ones.
struct Rocks {
    ramp: [[u8; 3]; 5],
    /// Pebbles sit on a jittered grid of this many pixels; it divides `BLOCK` so
    /// the grid wraps evenly.
    spacing: u32,
    /// Chance a grid cell holds a pebble.
    chance: f32,
    radius: (f32, f32),
    /// Shades a pebble's body can take.
    bodies: &'static [u8],
    /// Matrix shades for low, middle and high clustered noise.
    matrix: [u8; 3],
    seed: u32,
}

/// Soil with small clods and stones scattered through it.
const DIRT_ROCKS: Rocks = Rocks {
    ramp: DIRT_RAMP,
    spacing: 8,
    chance: 0.45,
    radius: (1.4, 2.8),
    bodies: &[3],
    matrix: [1, 2, 2],
    seed: 0xD127,
};
/// Big chunks packed tight, dark cracks between.
const STONE_ROCKS: Rocks = Rocks {
    ramp: STONE_RAMP,
    spacing: 8,
    chance: 0.95,
    radius: (3.2, 5.4),
    bodies: &[2, 2, 3],
    matrix: [0, 1, 1],
    seed: 0x5701,
};

pub const PATH: &str = "assets/tiles.png";

/// Neighbour bit order, matching `autotile`: N, E, S, W, NE, SE, SW, NW.
pub const DIRS: [(i32, i32); 8] = [
    (0, -1),
    (1, 0),
    (0, 1),
    (-1, 0),
    (1, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
];

/// Top-left pixel of a material's 4x4-tile block.
pub fn material_block(mat: u16) -> (u32, u32) {
    (mat as u32 * BLOCK, 0)
}

/// Top-left pixel of a mask cell.
pub fn mask_cell(mask: u16) -> (u32, u32) {
    let i = mask as u32;
    (
        (i % MASK_COLS) * CELL,
        MASK_BAND_Y + (i / MASK_COLS) * CELL,
    )
}

/// Loads the atlas, generating and writing it first if the file is absent, so a
/// fresh checkout runs without a separate build step.
pub fn load_or_generate() -> (Vec<u8>, u32, u32) {
    if !Path::new(PATH).exists() {
        if let Err(e) = generate_to_file() {
            eprintln!("could not write {PATH} ({e}); using the in-memory atlas");
            return (generate(), WIDTH, HEIGHT);
        }
    }
    match image::open(PATH) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            if (w, h) != (WIDTH, HEIGHT) {
                // A file from an older layout would silently misplace every cell,
                // because the shader addresses cells by the expected dimensions.
                eprintln!("{PATH} is {w}x{h}, expected {WIDTH}x{HEIGHT}; regenerating");
                let _ = generate_to_file();
                return (generate(), WIDTH, HEIGHT);
            }
            (rgba.into_raw(), w, h)
        }
        Err(e) => {
            eprintln!("could not read {PATH} ({e}); using the in-memory atlas");
            (generate(), WIDTH, HEIGHT)
        }
    }
}

pub fn generate_to_file() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(dir) = Path::new(PATH).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let image = image::RgbaImage::from_raw(WIDTH, HEIGHT, generate())
        .ok_or("generated atlas has the wrong length")?;
    image.save(PATH)?;
    Ok(())
}

/// RGBA8 atlas pixels, row-major.
pub fn generate() -> Vec<u8> {
    let mut px = vec![0u8; (WIDTH * HEIGHT * 4) as usize];

    for tile in TILE_TYPES {
        match tile.atlas_mat {
            MAT_DIRT => rock_texture(&mut px, MAT_DIRT, &DIRT_ROCKS),
            MAT_STONE => rock_texture(&mut px, MAT_STONE, &STONE_ROCKS),
            mat => material_texture(&mut px, mat, tile.color),
        }
    }
    for decor in DECOR_TYPES {
        match decor.atlas_mat {
            MAT_GRASS => grass_texture(&mut px, MAT_GRASS),
            mat => material_texture(&mut px, mat, decor.color),
        }
    }
    // Faces to air: a darker lip on the sides and underside, a warm rim on top.
    // Both translucent, so they shade whatever material they land on.
    flat_material(&mut px, MAT_AIR_SHADOW, [26, 22, 34, 90]);
    flat_material(&mut px, MAT_RIM, [255, 236, 196, 64]);
    material_texture(&mut px, MAT_WHITE, [255, 255, 255, 255]);
    // Flat: speckle reads as a surface, and sky is not one.
    flat_material(&mut px, MAT_SKY, SKY_COLOR);
    shade(&mut px, MAT_BARK, &BARK_RAMP, &bark_shades());
    material_texture(&mut px, MAT_LEAVES, LEAVES_COLOR);

    for mask in 0..MASK_COUNT {
        mask_texture(&mut px, mask);
    }
    px
}

fn put(px: &mut [u8], x: u32, y: u32, rgba: [u8; 4]) {
    let i = ((y * WIDTH + x) * 4) as usize;
    px[i..i + 4].copy_from_slice(&rgba);
}

fn flat_material(px: &mut [u8], mat: u16, color: [u8; 4]) {
    let (ox, oy) = material_block(mat);
    for y in 0..BLOCK {
        for x in 0..BLOCK {
            put(px, ox + x, oy + y, color);
        }
    }
}

fn shade(px: &mut [u8], mat: u16, ramp: &[[u8; 3]; 5], shades: &[u8]) {
    let (ox, oy) = material_block(mat);
    for y in 0..BLOCK {
        for x in 0..BLOCK {
            let [r, g, b] = ramp[shades[(y * BLOCK + x) as usize] as usize];
            put(px, ox + x, oy + y, [r, g, b, 255]);
        }
    }
}

fn rock_texture(px: &mut [u8], mat: u16, rocks: &Rocks) {
    shade(px, mat, &rocks.ramp, &rock_shades(rocks).0);
}

/// Ramp index per pixel of a rock block, and which pebble each pixel belongs to
/// (-1 for the matrix between them). Everything wraps at the block edge.
fn rock_shades(rocks: &Rocks) -> (Vec<u8>, Vec<i32>) {
    let size = BLOCK as i32;
    let at = |x: i32, y: i32| (y.rem_euclid(size) * size + x.rem_euclid(size)) as usize;
    let mut ids = vec![-1; (BLOCK * BLOCK) as usize];
    let mut bodies = Vec::new();
    let cells = (BLOCK / rocks.spacing) as i32;
    let spacing = rocks.spacing as f32;
    for cy in 0..cells {
        for cx in 0..cells {
            let r = |k: u32| noise(rocks.seed ^ k, cx as u32, cy as u32);
            if r(1) > rocks.chance {
                continue;
            }
            let centre = [(cx as f32 + r(2)) * spacing, (cy as f32 + r(3)) * spacing];
            let (lo, hi) = rocks.radius;
            // Wider than tall, mostly: stones settle flat.
            let rx = lo + (hi - lo) * r(4);
            let ry = rx * (0.65 + 0.35 * r(5));
            let (lumps, phase) = (2.0 + (r(6) * 3.0).floor(), r(7) * 6.283);
            let id = bodies.len() as i32;
            bodies.push(rocks.bodies[(r(8) * rocks.bodies.len() as f32) as usize % rocks.bodies.len()]);
            let reach = (rx * 1.3).ceil() as i32 + 1;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (px, py) = (centre[0].floor() as i32 + dx, centre[1].floor() as i32 + dy);
                    let (ex, ey) = ((px as f32 + 0.5 - centre[0]) / rx, (py as f32 + 0.5 - centre[1]) / ry);
                    // Lumpy rather than elliptical, so pebbles read as chunks.
                    let edge = 1.0 + 0.18 * (ey.atan2(ex) * lumps + phase).sin();
                    if ex * ex + ey * ey < edge * edge {
                        ids[at(px, py)] = id;
                    }
                }
            }
        }
    }
    let mut shades = vec![0u8; ids.len()];
    for y in 0..size {
        for x in 0..size {
            let me = ids[at(x, y)];
            let (up, down, left, right) = (ids[at(x, y - 1)], ids[at(x, y + 1)], ids[at(x - 1, y)], ids[at(x + 1, y)]);
            shades[at(x, y)] = if me < 0 {
                if [up, down, left, right].iter().any(|&n| n >= 0) {
                    0 // the dark gap that sets each pebble off
                } else {
                    let v = noise(rocks.seed ^ 0xC1, (x / 3) as u32, (y / 3) as u32);
                    rocks.matrix[if v < 0.3 { 0 } else if v < 0.88 { 1 } else { 2 }]
                }
            } else {
                let body = bodies[me as usize];
                if (up >= 0 && up != me) || (left >= 0 && left != me) {
                    0 // a crack where two pebbles meet
                } else if up != me {
                    (body + 1).min(4) // lit from above
                } else if down != me {
                    body - 1 // shaded underneath
                } else if left != me {
                    (body + 1).min(4)
                } else {
                    body
                }
            };
        }
    }
    (shades, ids)
}

/// Blades: vertical streaks of the ramp's middle shades, a few bright tips.
fn grass_texture(px: &mut [u8], mat: u16) {
    let mut shades = vec![0u8; (BLOCK * BLOCK) as usize];
    for y in 0..BLOCK {
        for x in 0..BLOCK {
            let streak = noise(0x6A55, x, y / 5 + x * 7);
            let tip = noise(0x6A56, x / 2, y / 2);
            shades[(y * BLOCK + x) as usize] = if tip > 0.93 {
                4
            } else if streak < 0.3 {
                1
            } else if streak < 0.8 {
                2
            } else {
                3
            };
        }
    }
    shade(px, mat, &GRASS_RAMP, &shades);
}

fn material_texture(px: &mut [u8], mat: u16, color: [u8; 4]) {
    if color[3] == 0 {
        return; // air's own tile type carries no texture; MAT_AIR is written separately
    }
    let (ox, oy) = material_block(mat);
    for y in 0..BLOCK {
        for x in 0..BLOCK {
            // Fine speckle over coarser blotches, both varying across the whole block.
            // Deliberately no per-cell gradient: anything keyed to a tile's own
            // coordinates draws a visible grid line at every tile boundary.
            let speckle = (noise(mat as u32, x, y) - 0.5) * 18.0;
            let blotch = (noise(mat as u32 ^ 0x55, x / 4, y / 4) - 0.5) * 24.0;
            let shift = speckle + blotch;
            let rgb = [0, 1, 2].map(|c| (color[c] as f32 + shift).clamp(0.0, 255.0) as u8);
            put(px, ox + x, oy + y, [rgb[0], rgb[1], rgb[2], color[3]]);
        }
    }
}

/// Bark, as ramp indices: plates running down the trunk between wandering
/// grooves, broken across by cracks, each plate bevelled like a pebble (lit on
/// top and on the left, shaded underneath and on the right), pitted here and
/// there, with a few knots. Wraps at the block edge both ways, so a trunk taller
/// or wider than the block has no seam.
fn bark_shades() -> Vec<u8> {
    let size = BLOCK as i32;
    let seed = MAT_BARK as u32 ^ 0xBA4C;
    let at = |x: i32, y: i32| (y.rem_euclid(size) * size + x.rem_euclid(size)) as usize;
    let period = std::f32::consts::TAU / BLOCK as f32;

    // Grooves 4-6 pixels apart, the last gap closing the block.
    let mut grooves = Vec::new();
    let mut gx = 0;
    while gx < size - 3 {
        grooves.push((gx, noise(seed, gx as u32, 1) * 6.3));
        gx += 4 + (noise(seed, gx as u32, 2) * 3.0) as i32;
    }
    let groove_at = |g: usize, y: i32| {
        let (x, phase) = grooves[g];
        let wander = (y as f32 * period * 2.0 + phase).sin() * 0.9 + (y as f32 * period * 5.0 + phase * 1.7).sin() * 0.5;
        x + wander.round() as i32
    };

    // Which plate each pixel belongs to; -1 for grooves and cracks.
    let mut ids = vec![-1i32; (BLOCK * BLOCK) as usize];
    let mut bodies = Vec::new();
    for g in 0..grooves.len() {
        // Plates 12-30 pixels long down this strip, cracks between them: long
        // and narrow, as bark splits, not brick-shaped.
        let mut cracks = vec![0];
        let mut y = 0;
        loop {
            y += 12 + (noise(seed ^ 0xC4, g as u32, y as u32) * 19.0) as i32;
            if y > size - 8 {
                break;
            }
            cracks.push(y);
        }
        for y in 0..size {
            let plate = cracks.iter().rposition(|&c| c <= y).unwrap();
            if cracks[plate] == y {
                continue;
            }
            let id = (g * 64 + plate) as i32;
            let (left, right) = (groove_at(g, y), groove_at((g + 1) % grooves.len(), y));
            let right = if g + 1 == grooves.len() { right + size } else { right };
            for x in left + 1..right {
                ids[at(x, y)] = id;
            }
            if bodies.len() <= id as usize {
                bodies.resize(id as usize + 1, 2);
            }
            let v = noise(seed ^ 0xB0, g as u32, plate as u32);
            bodies[id as usize] = if v < 0.2 { 1 } else if v > 0.85 { 3 } else { 2 };
        }
    }

    let mut shades = vec![0u8; ids.len()];
    for y in 0..size {
        for x in 0..size {
            let me = ids[at(x, y)];
            if me < 0 {
                continue; // groove or crack: the darkest shade
            }
            let body = bodies[me as usize];
            // Only the right side goes unshaded: the groove already darkens it,
            // and shading it too would frame every plate like a brick.
            let (up, down, left) = (ids[at(x, y - 1)], ids[at(x, y + 1)], ids[at(x - 1, y)]);
            shades[at(x, y)] = if up != me || left != me {
                (body + 1).min(4)
            } else if down != me {
                body - 1
            } else if noise(seed ^ 0x917, x as u32, y as u32) < 0.07 {
                body - 1 // a pit
            } else {
                body
            };
        }
    }

    // Knots: about one tile in three, kept inside the tile. A dark core, a shaded
    // ring, and the ring's upper left catching the light.
    for t in 0..BLOCK_TILES * BLOCK_TILES {
        if noise(seed ^ 0x4B, t, 0) >= 0.33 {
            continue;
        }
        let (tx, ty) = ((t % BLOCK_TILES) * CELL, (t / BLOCK_TILES) * CELL);
        let c = [tx as f32 + 5.0 + noise(seed ^ 0x4C, t, 0) * 6.0, ty as f32 + 4.0 + noise(seed ^ 0x4D, t, 0) * 8.0];
        for y in ty..ty + CELL {
            for x in tx..tx + CELL {
                let (dx, dy) = (x as f32 + 0.5 - c[0], y as f32 + 0.5 - c[1]);
                let e = (dx / 1.6).powi(2) + (dy / 2.4).powi(2);
                let i = at(x as i32, y as i32);
                if e < 1.0 {
                    shades[i] = 0;
                } else if e < 2.6 {
                    shades[i] = if dx + dy < 0.0 { 4 } else { 1 };
                }
            }
        }
    }
    shades
}

fn mask_texture(px: &mut [u8], mask: u16) {
    let (ox, oy) = mask_cell(mask);
    for y in 0..CELL {
        for x in 0..CELL {
            let alpha = if mask == SOLID_MASK {
                255
            } else if mask >= THIN_MASKS {
                edge_alpha(x, y, (mask - THIN_MASKS) as u8, THIN, 0)
            } else if mask >= LEAF_MASK {
                leaf_alpha(x, y, mask - LEAF_MASK)
            } else {
                edge_alpha(x, y, mask as u8, BAND, FRINGE)
            };
            put(px, ox + x, oy + y, [255, 255, 255, alpha]);
        }
    }
}

/// A clump of a few overlapping round leaves, `variant` 0 smallest. Its edge is
/// roughened in 2-pixel steps so it reads as chunky pixel foliage, not a circle.
/// Even the smallest overlaps its neighbours: a gap inside a canopy picks up an
/// outline and reads as dark speckle.
fn leaf_alpha(x: u32, y: u32, variant: u16) -> u8 {
    let scale = 0.85 + 0.1 * variant as f32;
    let p = [x as f32 + 0.5 - 8.0, y as f32 + 0.5 - 8.0];
    let lobes = [([0.0, 0.5], 5.2), ([-3.0, -2.0], 3.4), ([3.2, -1.8], 3.2), ([-1.8, 3.6], 3.0), ([3.0, 3.0], 3.3)];
    let rough = (noise(0x1EAF ^ variant as u32, x / 2, y / 2) - 0.5) * 1.6;
    let inside = lobes.iter().any(|&(c, r): &([f32; 2], f32)| {
        (p[0] - c[0] * scale).hypot(p[1] - c[1] * scale) < r * scale + rough
    });
    if inside { 255 } else { 0 }
}

/// Coverage of a pixel by a material occupying the neighbours in `bits`, reaching
/// `width` pixels in. Hard: a pixel is covered or it is not. Each strand of the
/// edge — a column for top and bottom, a row for the sides — reaches up to
/// `fringe` pixels further or shorter, so edges are ragged rather than ruled.
fn edge_alpha(x: u32, y: u32, bits: u8, width: f32, fringe: i32) -> u8 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cell = CELL as f32;
    for (bit, (dx, dy)) in DIRS.iter().enumerate() {
        if bits & (1 << bit) == 0 {
            continue;
        }
        let (dist, strand) = match (dx, dy) {
            (0, -1) => (py, x),
            (0, 1) => (cell - py, x),
            (-1, 0) => (px, y),
            (1, 0) => (cell - px, y),
            // Diagonals measure from the corner itself, giving a quarter-round nub.
            (cx, cy) => {
                let corner_x = if *cx > 0 { cell } else { 0.0 };
                let corner_y = if *cy > 0 { cell } else { 0.0 };
                (((px - corner_x).powi(2) + (py - corner_y).powi(2)).sqrt(), x + y)
            }
        };
        let span = (2 * fringe + 1) as f32;
        let reach = (noise(0x51DE ^ bit as u32, strand, 0) * span).floor() as i32 - fringe;
        if dist < width + reach as f32 {
            return 255;
        }
    }
    0
}

/// Deterministic value noise in 0..1 for a pixel.
fn noise(seed: u32, x: u32, y: u32) -> f32 {
    let mut h = (seed as u64) ^ ((x as u64) << 32) ^ (y as u64).wrapping_mul(0x9E37_79B9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(px: &[u8], x: u32, y: u32) -> u8 {
        px[((y * WIDTH + x) * 4 + 3) as usize]
    }

    #[test]
    fn atlas_has_the_expected_size_and_every_cell_is_in_bounds() {
        let px = generate();
        assert_eq!(px.len(), (WIDTH * HEIGHT * 4) as usize);
        for mask in 0..MASK_COUNT {
            let (x, y) = mask_cell(mask);
            assert!(x + CELL <= WIDTH && y + CELL <= HEIGHT, "mask {mask} cell at ({x}, {y})");
        }
        for mat in 0..MATERIALS as u16 {
            let (x, y) = material_block(mat);
            assert!(x + BLOCK <= WIDTH && y + BLOCK <= HEIGHT, "material {mat} block at ({x}, {y})");
        }
        assert!(MASK_BAND_Y >= BLOCK, "masks must not overlap the material band");
    }

    #[test]
    fn solid_mask_is_opaque_and_the_empty_mask_is_clear() {
        let px = generate();
        let (sx, sy) = mask_cell(SOLID_MASK);
        let (ex, ey) = mask_cell(0);
        for y in 0..CELL {
            for x in 0..CELL {
                assert_eq!(alpha_at(&px, sx + x, sy + y), 255, "solid mask has a hole");
                assert_eq!(alpha_at(&px, ex + x, ey + y), 0, "empty mask is not clear");
            }
        }
    }

    #[test]
    fn a_north_mask_covers_the_top_edge_and_not_the_bottom() {
        let px = generate();
        let (ox, oy) = mask_cell(1); // bit 0 is north
        for x in 0..CELL {
            assert!(alpha_at(&px, ox + x, oy) > 200, "top row is not covered at x {x}");
            assert_eq!(alpha_at(&px, ox + x, oy + CELL - 1), 0, "bottom row leaked at x {x}");
        }
    }

    #[test]
    fn a_diagonal_only_mask_covers_its_corner_and_nothing_else() {
        let px = generate();
        let (ox, oy) = mask_cell(1 << 4); // bit 4 is north-east
        assert!(alpha_at(&px, ox + CELL - 1, oy) > 150, "the north-east corner is not covered");
        assert_eq!(alpha_at(&px, ox, oy + CELL - 1), 0, "the opposite corner leaked");
        assert_eq!(alpha_at(&px, ox, oy), 0, "the north-west corner leaked");
    }

    #[test]
    fn rock_materials_keep_to_their_ramp() {
        let px = generate();
        for (mat, ramp) in [(MAT_DIRT, DIRT_RAMP), (MAT_STONE, STONE_RAMP), (MAT_GRASS, GRASS_RAMP), (MAT_BARK, BARK_RAMP)] {
            let (ox, oy) = material_block(mat);
            let mut used = std::collections::HashSet::new();
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let i = (((oy + y) * WIDTH + ox + x) * 4) as usize;
                    let rgb = [px[i], px[i + 1], px[i + 2]];
                    let shade = ramp.iter().position(|&c| c == rgb);
                    assert!(shade.is_some(), "material {mat} has {rgb:?}, which is off its ramp");
                    used.insert(shade);
                }
            }
            assert!(used.len() >= 4, "material {mat} uses only {} shades", used.len());
        }
    }

    #[test]
    fn bark_plates_are_lit_on_top_and_shaded_underneath() {
        let shades = bark_shades();
        let size = BLOCK as usize;
        let mean = |row: &dyn Fn(usize, usize) -> bool| {
            let picked: Vec<f32> = (1..size - 1)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|&(x, y)| row(x, y))
                .map(|(x, y)| shades[y * size + x] as f32)
                .collect();
            picked.iter().sum::<f32>() / picked.len() as f32
        };
        // Pixels just below a crack or groove versus just above one.
        let dark = |x: usize, y: usize| shades[y * size + x] == 0;
        let below = mean(&|x, y| !dark(x, y) && dark(x, y - 1));
        let above = mean(&|x, y| !dark(x, y) && dark(x, y + 1));
        assert!(below > above + 0.8, "plate tops {below:.2} should be well lighter than bottoms {above:.2}");
    }

    #[test]
    fn pebbles_are_lit_from_above() {
        for rocks in [&DIRT_ROCKS, &STONE_ROCKS] {
            let (shades, ids) = rock_shades(rocks);
            let size = BLOCK as usize;
            let (mut top, mut bottom) = ((0.0, 0), (0.0, 0));
            for y in 1..size - 1 {
                for x in 0..size {
                    let me = ids[y * size + x];
                    if me < 0 {
                        continue;
                    }
                    let shade = shades[y * size + x] as f32;
                    if ids[(y - 1) * size + x] < 0 {
                        top = (top.0 + shade, top.1 + 1);
                    }
                    if ids[(y + 1) * size + x] < 0 {
                        bottom = (bottom.0 + shade, bottom.1 + 1);
                    }
                }
            }
            let (top, bottom) = (top.0 / top.1 as f32, bottom.0 / bottom.1 as f32);
            assert!(top > bottom + 1.5, "pebble tops ({top:.2}) should be lighter than undersides ({bottom:.2})");
        }
    }

    #[test]
    fn pebbles_wrap_across_the_block_edge() {
        let (_, ids) = rock_shades(&STONE_ROCKS);
        let size = BLOCK as usize;
        let shared = (0..size).filter(|&y| ids[y * size] >= 0 && ids[y * size] == ids[y * size + size - 1]).count();
        assert!(shared > 0, "no pebble continues across the seam, so the block edge shows");
    }

    #[test]
    fn masks_are_hard_edged() {
        let px = generate();
        for mask in (0..SOLID_MASK).chain(LEAF_MASK..MASK_COUNT) {
            let (ox, oy) = mask_cell(mask);
            for y in 0..CELL {
                for x in 0..CELL {
                    let a = alpha_at(&px, ox + x, oy + y);
                    assert!(a == 0 || a == 255, "mask {mask} has a half-covered pixel ({a})");
                }
            }
        }
    }

    #[test]
    fn transition_edges_are_ragged_and_thin_edges_are_not() {
        let px = generate();
        let depth = |mask: u16, x: u32| {
            let (ox, oy) = mask_cell(mask);
            (0..CELL).take_while(|&y| alpha_at(&px, ox + x, oy + y) == 255).count()
        };
        let band: std::collections::HashSet<_> = (0..CELL).map(|x| depth(1, x)).collect();
        assert!(band.len() > 1, "a transition fringe should vary column to column");
        let thin: std::collections::HashSet<_> = (0..CELL).map(|x| depth(THIN_MASKS + 1, x)).collect();
        assert_eq!(thin, [THIN as usize].into(), "the rim is a clean two-pixel line");
    }

    #[test]
    fn material_textures_do_not_repeat_every_tile() {
        let px = generate();
        let value = |mat: u16, x: u32, y: u32| {
            let (ox, oy) = material_block(mat);
            px[(((oy + y) * WIDTH + ox + x) * 4) as usize]
        };
        for tile in TILE_TYPES.iter().filter(|t| t.color[3] > 0) {
            // Tile-sized sub-cells must differ, or the eye sees a grid.
            let differs = (0..CELL).any(|y| {
                (0..CELL).any(|x| value(tile.atlas_mat, x, y) != value(tile.atlas_mat, x + CELL, y))
            });
            assert!(differs, "{}'s second tile repeats the first exactly", tile.name);
        }
    }

    #[test]
    fn material_textures_have_no_per_tile_banding() {
        let px = generate();
        let row_mean = |mat: u16, y: u32| {
            let (ox, oy) = material_block(mat);
            let sum: u32 = (0..BLOCK)
                .map(|x| px[(((oy + y) * WIDTH + ox + x) * 4) as usize] as u32)
                .sum();
            sum as f32 / BLOCK as f32
        };
        for tile in TILE_TYPES.iter().filter(|t| t.color[3] > 0) {
            // A vertical gradient inside a tile shows up as the top and bottom rows of
            // each tile-sized band differing systematically.
            let top = row_mean(tile.atlas_mat, 0);
            let bottom = row_mean(tile.atlas_mat, CELL - 1);
            assert!(
                (top - bottom).abs() < 8.0,
                "{} is {:.1} brighter at the top of a tile, which draws a grid",
                tile.name,
                top - bottom
            );
        }
    }

    #[test]
    fn the_atlas_survives_a_png_round_trip() {
        let generated = generate();
        let dir = std::env::temp_dir().join("abyss-atlas-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("tiles.png");
        image::RgbaImage::from_raw(WIDTH, HEIGHT, generated.clone())
            .expect("atlas length")
            .save(&path)
            .expect("write png");
        let read = image::open(&path).expect("read png").to_rgba8();
        assert_eq!(read.width(), WIDTH);
        assert_eq!(read.height(), HEIGHT);
        assert_eq!(read.into_raw(), generated, "png round trip changed pixels");
        let _ = std::fs::remove_file(&path);
    }
}
