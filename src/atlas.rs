//! The texture atlas: material textures plus the transition mask set.
//!
//! Layout:
//!   top band    — one 4x4-tile material block per material (air shadow, dirt, stone,
//!                 grass, white), sampled by world position so the pattern repeats
//!                 every 4 tiles instead of resetting at every tile edge
//!   below it    — 257 transition masks of one tile each: index 0..=255 keyed by the
//!                 8-neighbour bitmask of the bleeding material, plus `SOLID_MASK`.
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
pub const BLOCK_TILES: u32 = 4;
pub const BLOCK: u32 = CELL * BLOCK_TILES;

pub const MASK_COLS: u32 = 16;
pub const MASK_ROWS: u32 = 17;
/// Masks start below the material band.
pub const MASK_BAND_Y: u32 = BLOCK;

pub const MATERIALS: u32 = 9;
pub const WIDTH: u32 = MATERIALS * BLOCK;
pub const HEIGHT: u32 = MASK_BAND_Y + MASK_ROWS * CELL;

/// Material blocks, in order.
/// The darkening an exposed edge casts on its neighbour, not a surface itself.
pub const MAT_AIR_SHADOW: u16 = 0;
pub const MAT_WHITE: u16 = 4;
pub const MAT_SKY: u16 = 5;
pub const MAT_BARK: u16 = 7;
pub const MAT_LEAVES: u16 = 8;

/// Fully opaque mask, used by base tiles and by untextured quads.
pub const SOLID_MASK: u16 = 256;
/// An irregular clump of leaves, for foliage clusters.
pub const LEAF_MASK: u16 = 257;
pub const MASK_COUNT: u16 = 258;

/// Open air. Drawn like any other tile so that unlit air reads as black rather
/// than as whatever the frame was cleared to.
pub const SKY_COLOR: [u8; 4] = [107, 153, 219, 255];
pub const BARK_COLOR: [u8; 4] = [102, 74, 52, 255];
pub const LEAVES_COLOR: [u8; 4] = [74, 128, 58, 255];

/// How far a transition reaches into the tile, in pixels.
const BAND: f32 = 8.0;
/// Width of the fade at the inner edge. The band is fully opaque until the last
/// `FALLOFF` pixels: a pure linear ramp spends its whole width half-blended, which
/// reads as a muddy line rather than a layer of grass.
const FALLOFF: f32 = 4.5;

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
        material_texture(&mut px, tile.atlas_mat, tile.color);
    }
    for decor in DECOR_TYPES {
        material_texture(&mut px, decor.atlas_mat, decor.color);
    }
    // Air bleeding onto a tile reads as a dark inner edge rather than a texture.
    // Kept weak: this is an ambient darkening at exposed edges, and it paints over
    // decoration, so a strong value turns grass into a dark outline.
    material_texture(&mut px, MAT_AIR_SHADOW, [26, 24, 34, 70]);
    material_texture(&mut px, MAT_WHITE, [255, 255, 255, 255]);
    // Flat: speckle reads as a surface, and sky is not one.
    flat_material(&mut px, MAT_SKY, SKY_COLOR);
    bark_texture(&mut px, MAT_BARK, BARK_COLOR);
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

/// Bark runs along the trunk: streaks that change slowly downwards and quickly
/// across, so the grain reads as vertical.
fn bark_texture(px: &mut [u8], mat: u16, color: [u8; 4]) {
    let (ox, oy) = material_block(mat);
    for y in 0..BLOCK {
        for x in 0..BLOCK {
            let streak = (noise(mat as u32, x / 2, y / 10) - 0.5) * 34.0;
            let speckle = (noise(mat as u32 ^ 0x33, x, y) - 0.5) * 10.0;
            let rgb = [0, 1, 2].map(|c| (color[c] as f32 + streak + speckle).clamp(0.0, 255.0) as u8);
            put(px, ox + x, oy + y, [rgb[0], rgb[1], rgb[2], color[3]]);
        }
    }
}

fn mask_texture(px: &mut [u8], mask: u16) {
    let (ox, oy) = mask_cell(mask);
    for y in 0..CELL {
        for x in 0..CELL {
            let alpha = if mask == SOLID_MASK {
                255
            } else if mask == LEAF_MASK {
                leaf_alpha(x, y)
            } else {
                band_alpha(x, y, mask as u8)
            };
            put(px, ox + x, oy + y, [255, 255, 255, alpha]);
        }
    }
}

/// A clump of a few overlapping round leaves with eroded edges, clear at the cell
/// border so rotated clusters never show a square outline.
fn leaf_alpha(x: u32, y: u32) -> u8 {
    let p = [x as f32 + 0.5, y as f32 + 0.5];
    let lobes = [([8.0, 8.5], 5.2), ([5.0, 6.0], 3.4), ([11.2, 6.2], 3.2), ([6.2, 11.6], 3.0), ([11.0, 11.0], 3.3)];
    let inside = lobes.iter().any(|&(c, r): &([f32; 2], f32)| {
        let jitter = (noise(0x1EAF, x, y) - 0.5) * 1.6;
        (p[0] - c[0]).hypot(p[1] - c[1]) < r + jitter
    });
    if inside { 255 } else { 0 }
}

/// Coverage of a pixel by a material occupying the neighbours in `bits`: the
/// strongest contribution of any filled direction, so overlapping directions
/// never compound into a dark seam the way separate additive quads would.
fn band_alpha(x: u32, y: u32, bits: u8) -> u8 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cell = CELL as f32;
    // Jitter the band edge per pixel so transitions look eroded rather than ruled.
    let band = BAND + (noise(0x51DE, x, y) - 0.5) * 3.0;
    let mut best: f32 = 0.0;
    for (bit, (dx, dy)) in DIRS.iter().enumerate() {
        if bits & (1 << bit) == 0 {
            continue;
        }
        let dist = match (dx, dy) {
            (0, -1) => py,
            (0, 1) => cell - py,
            (-1, 0) => px,
            (1, 0) => cell - px,
            // Diagonals measure from the corner itself, giving a quarter-round nub.
            (cx, cy) => {
                let corner_x = if *cx > 0 { cell } else { 0.0 };
                let corner_y = if *cy > 0 { cell } else { 0.0 };
                ((px - corner_x).powi(2) + (py - corner_y).powi(2)).sqrt()
            }
        };
        best = best.max((band - dist) / FALLOFF);
    }
    (best.clamp(0.0, 1.0) * 255.0) as u8
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
    fn material_textures_vary_but_stay_near_their_colour() {
        let px = generate();
        for tile in TILE_TYPES.iter().filter(|t| t.color[3] > 0) {
            let (ox, oy) = material_block(tile.atlas_mat);
            let mut seen = std::collections::HashSet::new();
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let i = (((oy + y) * WIDTH + ox + x) * 4) as usize;
                    seen.insert(px[i]);
                    let drift = px[i] as i32 - tile.color[0] as i32;
                    assert!(drift.abs() <= 32, "{} drifted {drift} from its colour", tile.name);
                }
            }
            assert!(seen.len() > 4, "{} is a flat colour, not a texture", tile.name);
        }
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
