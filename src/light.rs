//! Per-tile RGB lighting for the visible region.
//!
//! Light is computed over the view plus a margin — never the whole world — so the
//! cost tracks screen area, and a torch just off-screen still reaches you. The
//! result is uploaded as a small texture and sampled with linear filtering by world
//! position, which makes the gradient smooth across and between tiles for free, and
//! lights tiles, overlays, the player and mobs through one lookup.
//!
//! Propagation is a pair of raster passes per round — one forward over the grid,
//! one backward — each taking light from the neighbours already visited, including
//! the diagonals. Carrying only along the axes makes falloff Manhattan rather than
//! radial: a torch lights a diamond, and the lit band along a slope thickens and
//! thins with the terrain angle. Including diagonals, charged `sqrt(2)` tiles of
//! decay, approximates true distance closely for the same cost.
//!
//! Outdoor light comes in two parts (see `sky`). Ambient skylight seeds open air
//! straight down and spreads like any other light. Direct sun or moonlight is
//! swept separately along its direction at half-tile resolution, through leaves and
//! stopped by solid ground, and is *not* spread: spreading would wash out the very
//! shadows it casts. The shader adds it on top, from its own texture.

use crate::camera::TileRect;
use crate::sky::Lighting;
use crate::tiles::{is_solid, tile_type};
use crate::world::World;

/// How far outside the view light is computed, so off-screen sources still reach in.
pub const MARGIN: i32 = 20;

/// Fraction of light surviving one tile of travel.
const AIR_DECAY: f32 = 0.91;
const SOLID_DECAY: f32 = 0.60;
/// Two passes carry light in all directions once; a path that alternates between
/// directions served by different passes — a tunnel climbing up and right — needs
/// a second round. Measured on exactly that shape: one round leaves its top at
/// 0.00008, two at 0.0175, three identical to two.
const ROUNDS: usize = 2;

/// Direct light is swept on cells this many to a tile edge, matching the foliage
/// layer, so light through a canopy keeps the canopy's detail.
pub const SUN_SUB: i32 = 2;
/// Share of direct light a fully dense half-tile of leaves stops.
const FOLIAGE_OPACITY: f32 = 0.3;
/// Share of the direct light reaching open air that also spreads as bounce light,
/// so a sunlit cave mouth glows a little way past where the beam ends.
const BOUNCE: f32 = 0.3;

#[derive(Default)]
pub struct LightMap {
    origin: [i32; 2],
    size: [i32; 2],
    light: Vec<[f32; 3]>,
    texels: Vec<u8>,
    /// Direct light reaching each half-tile cell, 0..1. For open air, what passes
    /// through it; for solid ground, what lands on its face.
    sun: Vec<f32>,
    /// What leaves each cell downstream, which is nothing for solid ground.
    through: Vec<f32>,
    sun_texels: Vec<u8>,
}

impl LightMap {
    pub fn origin(&self) -> [i32; 2] {
        self.origin
    }

    pub fn size(&self) -> [i32; 2] {
        self.size
    }

    pub fn texels(&self) -> &[u8] {
        &self.texels
    }

    /// Size of the direct-light grid, in half-tile cells over the same region.
    pub fn sun_size(&self) -> [i32; 2] {
        [self.size[0] * SUN_SUB, self.size[1] * SUN_SUB]
    }

    pub fn sun_texels(&self) -> &[u8] {
        &self.sun_texels
    }

    /// Share of direct light reaching a world tile, averaged over its cells.
    pub fn direct_at(&self, x: i32, y: i32) -> f32 {
        let (i, j) = (x - self.origin[0], y - self.origin[1]);
        if i < 0 || j < 0 || i >= self.size[0] || j >= self.size[1] {
            return 0.0;
        }
        let w = self.sun_size()[0];
        let mut sum = 0.0;
        for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            sum += self.sun[((j * SUN_SUB + dj) * w + i * SUN_SUB + di) as usize];
        }
        sum / 4.0
    }

    /// Light at a world tile, or black outside the computed region.
    pub fn at(&self, x: i32, y: i32) -> [f32; 3] {
        let (i, j) = (x - self.origin[0], y - self.origin[1]);
        if i < 0 || j < 0 || i >= self.size[0] || j >= self.size[1] {
            return [0.0; 3];
        }
        self.light[(j * self.size[0] + i) as usize]
    }

    pub fn update(&mut self, world: &World, view: TileRect, lighting: &Lighting) {
        self.resize(world, view);
        self.sweep(world, lighting.toward);
        self.seed(world, lighting);
        // [orthogonal, diagonal] decay for [air, solid]; a diagonal step covers
        // sqrt(2) tiles, so it costs that much decay.
        let diagonal = std::f32::consts::SQRT_2;
        let decay = [
            [AIR_DECAY, AIR_DECAY.powf(diagonal)],
            [SOLID_DECAY, SOLID_DECAY.powf(diagonal)],
        ];
        for _ in 0..ROUNDS {
            self.pass_forward(world, &decay);
            self.pass_backward(world, &decay);
        }
        self.write_texels();
    }

    fn resize(&mut self, world: &World, view: TileRect) {
        self.origin = [(view.x0 - MARGIN).max(0), (view.y0 - MARGIN).max(0)];
        let end = [
            (view.x1 + MARGIN).min(world.width()),
            (view.y1 + MARGIN).min(world.height()),
        ];
        self.size = [(end[0] - self.origin[0]).max(1), (end[1] - self.origin[1]).max(1)];
        let cells = (self.size[0] * self.size[1]) as usize;
        self.light.clear();
        self.light.resize(cells, [0.0; 3]);
        self.texels.resize(cells * 4, 0);
        let sun_cells = cells * (SUN_SUB * SUN_SUB) as usize;
        self.sun.clear();
        self.sun.resize(sun_cells, 0.0);
        self.through.clear();
        self.through.resize(sun_cells, 0.0);
        self.sun_texels.resize(sun_cells, 0);
    }

    /// Carries direct light across the region along its direction, one row or
    /// column at a time, each cell taking what reached the cell behind it. One
    /// step per cell, however low the sun. Light from outside the region is
    /// assumed to arrive wherever the sky is open above.
    ///
    /// Which cell is "behind" steps sideways by 0 or 1 on a pattern shared by every
    /// ray, as a Bresenham line does. Blending the two nearest cells instead
    /// blurred a wall's shadow edge across ten tiles, since the blend compounds
    /// with every step; this keeps shadows as sharp as the grid.
    fn sweep(&mut self, world: &World, toward: [f32; 2]) {
        let [w, h] = self.sun_size();
        let base = [self.origin[0] * SUN_SUB, self.origin[1] * SUN_SUB];
        // Travel is away from the light. The sun and moon never sink below the
        // horizon while lighting (see `sky::ARC`), so travel always heads down.
        let d = [-toward[0], (-toward[1]).max(1e-3)];
        let steep = d[1] >= d[0].abs();
        let (major, minor) = if steep { (h, w) } else { (w, h) };
        let step = if steep || d[0] > 0.0 { 1 } else { -1 };
        // How far the ray shifts across, per row (or column) travelled.
        let slope = if steep { d[0] / d[1] } else { d[1] / d[0].abs() };
        let shift = |m: i32| (m as f32 * slope.abs()).floor() as i32;
        for m in 0..major {
            let a = if step > 0 { m } else { major - 1 - m };
            let prev = a - step;
            let sideways = (shift(m) - shift(m - 1)) * slope.signum() as i32;
            for n in 0..minor {
                let (pi, pj) = if steep {
                    (n - sideways, prev)
                } else {
                    (prev, n - sideways)
                };
                let incoming = if pi < 0 || pj < 0 || pi >= w || pj >= h {
                    open_sky(world, base[0] + pi, base[1] + pj)
                } else {
                    self.through[(pj * w + pi) as usize]
                };
                let (i, j) = if steep { (n, a) } else { (a, n) };
                let (sx, sy) = (base[0] + i, base[1] + j);
                let solid = is_solid(world.get(sx.div_euclid(SUN_SUB), sy.div_euclid(SUN_SUB)));
                let leaves = world.foliage(sx, sy) as f32 / 255.0;
                let after = if solid {
                    0.0
                } else {
                    incoming * (1.0 - FOLIAGE_OPACITY * leaves)
                };
                let cell = (j * w + i) as usize;
                self.through[cell] = after;
                self.sun[cell] = if solid { incoming } else { after };
            }
        }
    }

    /// Skylight above each column's surface, bounce from direct light, and
    /// anything that glows.
    fn seed(&mut self, world: &World, lighting: &Lighting) {
        for j in 0..self.size[1] {
            for i in 0..self.size[0] {
                let (x, y) = (self.origin[0] + i, self.origin[1] + j);
                let id = world.get(x, y);
                let bounce = if is_solid(id) {
                    0.0
                } else {
                    self.direct_at(x, y) * BOUNCE
                };
                let cell = &mut self.light[(j * self.size[0] + i) as usize];
                if !is_solid(id) && y < world.sky_level(x) {
                    *cell = lighting.ambient;
                }
                for c in 0..3 {
                    cell[c] += lighting.direct[c] * bounce;
                }
                let emitted = tile_type(id).light;
                for c in 0..3 {
                    cell[c] = cell[c].max(emitted[c]);
                }
            }
        }
    }

    /// Top to bottom, left to right, taking from the four neighbours already seen.
    fn pass_forward(&mut self, world: &World, decay: &[[f32; 2]; 2]) {
        let (w, h) = (self.size[0], self.size[1]);
        for j in 0..h {
            for i in 0..w {
                let step = self.decay_for(world, i, j, decay);
                let mut best = self.light[(j * w + i) as usize];
                if i > 0 {
                    take(&mut best, self.light[(j * w + i - 1) as usize], step[0]);
                }
                if j > 0 {
                    take(&mut best, self.light[((j - 1) * w + i) as usize], step[0]);
                    if i > 0 {
                        take(&mut best, self.light[((j - 1) * w + i - 1) as usize], step[1]);
                    }
                    if i + 1 < w {
                        take(&mut best, self.light[((j - 1) * w + i + 1) as usize], step[1]);
                    }
                }
                self.light[(j * w + i) as usize] = best;
            }
        }
    }

    /// Bottom to top, right to left, taking from the other four.
    fn pass_backward(&mut self, world: &World, decay: &[[f32; 2]; 2]) {
        let (w, h) = (self.size[0], self.size[1]);
        for j in (0..h).rev() {
            for i in (0..w).rev() {
                let step = self.decay_for(world, i, j, decay);
                let mut best = self.light[(j * w + i) as usize];
                if i + 1 < w {
                    take(&mut best, self.light[(j * w + i + 1) as usize], step[0]);
                }
                if j + 1 < h {
                    take(&mut best, self.light[((j + 1) * w + i) as usize], step[0]);
                    if i + 1 < w {
                        take(&mut best, self.light[((j + 1) * w + i + 1) as usize], step[1]);
                    }
                    if i > 0 {
                        take(&mut best, self.light[((j + 1) * w + i - 1) as usize], step[1]);
                    }
                }
                self.light[(j * w + i) as usize] = best;
            }
        }
    }

    /// Decay applied when light enters this cell, as [orthogonal, diagonal].
    fn decay_for(&self, world: &World, i: i32, j: i32, decay: &[[f32; 2]; 2]) -> [f32; 2] {
        let solid = is_solid(world.get(self.origin[0] + i, self.origin[1] + j));
        decay[solid as usize]
    }

    fn write_texels(&mut self) {
        for (cell, texel) in self.light.iter().zip(self.texels.chunks_exact_mut(4)) {
            for c in 0..3 {
                texel[c] = (cell[c].clamp(0.0, 1.0) * 255.0) as u8;
            }
            texel[3] = 255;
        }
        for (cell, texel) in self.sun.iter().zip(self.sun_texels.iter_mut()) {
            *texel = (cell.clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
}

/// Direct light arriving from outside the region: whatever the sky above gives.
fn open_sky(world: &World, sx: i32, sy: i32) -> f32 {
    let (x, y) = (sx.div_euclid(SUN_SUB), sy.div_euclid(SUN_SUB));
    if y < world.sky_level(x) { 1.0 } else { 0.0 }
}

fn take(into: &mut [f32; 3], from: [f32; 3], decay: f32) {
    for c in 0..3 {
        let carried = from[c] * decay;
        if carried > into[c] {
            into[c] = carried;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::Sky;
    use crate::tiles::{AIR, STONE, TORCH};

    /// Solid from row 32 down, open sky above.
    fn world() -> World {
        let mut w = World::new(128, 128);
        for x in 0..w.width() {
            for y in 32..w.height() {
                w.set(x, y, STONE);
            }
        }
        w
    }

    fn view() -> TileRect {
        TileRect {
            x0: 20,
            y0: 10,
            x1: 100,
            y1: 110,
        }
    }

    fn brightness(light: [f32; 3]) -> f32 {
        (light[0] + light[1] + light[2]) / 3.0
    }

    /// Mid-morning: full daylight, with the sun about 22 degrees off vertical.
    fn day() -> Lighting {
        Sky::at(0, 10.0).lighting()
    }

    /// What a tile shows: spread light plus the direct light the shader adds.
    fn lit(map: &LightMap, x: i32, y: i32) -> f32 {
        brightness(map.at(x, y)) + brightness(day().direct) * map.direct_at(x, y)
    }

    #[test]
    fn open_sky_is_lit_and_deep_rock_is_not() {
        let world = world();
        let mut map = LightMap::default();
        map.update(&world, view(), &day());

        assert!(lit(&map, 60, 20) > 0.9, "open air above the surface should be daylit");
        assert!(lit(&map, 60, 100) < 0.01, "rock far from any opening should be black");
    }

    #[test]
    fn a_narrow_shaft_is_lit_from_its_mouth_rather_than_along_its_whole_length() {
        let mut world = world();
        for y in 32..80 {
            world.set(53, y, AIR);
        }
        let mut map = LightMap::default();
        map.update(&world, view(), &day());
        assert!(
            lit(&map, 53, 75) < 0.02,
            "the bottom of a deep shaft should be dark, got {}",
            lit(&map, 53, 75)
        );
    }

    #[test]
    fn a_wide_pit_still_counts_as_outdoors() {
        let mut world = world();
        for x in 45..65 {
            for y in 32..40 {
                world.set(x, y, AIR);
            }
        }
        let mut map = LightMap::default();
        map.update(&world, view(), &day());
        assert!(
            lit(&map, 55, 38) > 0.8,
            "the floor of a wide open pit should be daylit, got {}",
            lit(&map, 55, 38)
        );
    }

    #[test]
    fn an_unlit_cave_is_dark_even_though_it_is_open_air() {
        let mut world = world();
        // A sealed pocket well below the surface.
        for x in 50..56 {
            for y in 80..84 {
                world.set(x, y, AIR);
            }
        }
        let mut map = LightMap::default();
        map.update(&world, view(), &day());
        assert!(
            lit(&map, 53, 82) < 0.01,
            "a sealed cave must be black, not sky-coloured"
        );
    }

    #[test]
    fn light_reaches_down_a_shaft_and_fades_with_depth() {
        let mut world = world();
        for y in 32..60 {
            world.set(53, y, AIR);
        }
        let mut map = LightMap::default();
        map.update(&world, view(), &day());

        let near = lit(&map, 53, 34);
        let far = lit(&map, 53, 55);
        assert!(near > 0.5, "the top of an open shaft should be bright, got {near}");
        assert!(far < near, "light should fade with depth: {far} vs {near}");
    }

    #[test]
    fn a_torch_lights_its_surroundings_in_its_own_colour() {
        let mut world = world();
        for x in 50..60 {
            for y in 80..86 {
                world.set(x, y, AIR);
            }
        }
        world.set(55, 84, TORCH);
        let mut map = LightMap::default();
        map.update(&world, view(), &day());

        let near = map.at(55, 83);
        assert!(
            brightness(near) > 0.3,
            "the tile beside a torch should be lit, got {near:?}"
        );
        assert!(near[0] > near[2], "torchlight should be warm, got {near:?}");
        assert!(
            lit(&map, 55, 83) > lit(&map, 59, 83),
            "light should fall off with distance from the torch"
        );
    }

    #[test]
    fn digging_down_lets_daylight_into_what_it_exposes() {
        let mut world = World::new(256, 256);
        for x in 0..world.width() {
            for y in 40..world.height() {
                world.set(x, y, STONE);
            }
        }
        world.snapshot_surface();
        let region = TileRect {
            x0: 10,
            y0: 0,
            x1: 240,
            y1: 200,
        };

        let mut map = LightMap::default();
        map.update(&world, region, &day());
        assert!(lit(&map, 120, 40) > 0.4, "the original surface starts lit");

        // Mine a wide, shallow pit, as a player levelling ground would.
        for x in 100..140 {
            for y in 40..46 {
                world.set(x, y, AIR);
            }
        }
        map.update(&world, region, &day());
        let floor = lit(&map, 120, 46);
        assert!(
            floor > 0.4,
            "the floor exposed by digging should be daylit, got {floor}"
        );
    }

    #[test]
    fn a_sloped_surface_is_lit_before_and_after_its_top_layer_is_mined() {
        let mut world = World::new(256, 256);
        let surface = |x: i32| 40 + x / 2; // a staircase descending to the right
        for x in 0..world.width() {
            for y in surface(x)..world.height() {
                world.set(x, y, STONE);
            }
        }
        world.snapshot_surface();
        let region = TileRect {
            x0: 10,
            y0: 0,
            x1: 240,
            y1: 200,
        };

        let mut map = LightMap::default();
        map.update(&world, region, &day());
        let before = lit(&map, 120, surface(120));
        assert!(
            before > 0.3,
            "a sloped surface should be lit to begin with, got {before}"
        );

        for x in 100..140 {
            world.set(x, surface(x), AIR);
        }
        map.update(&world, region, &day());
        let after = lit(&map, 120, surface(120) + 1);
        assert!(after > 0.3, "the tile uncovered by mining should be lit, got {after}");
    }

    #[test]
    fn falloff_follows_distance_rather_than_axes() {
        // In an open room, brightness should depend on how far away a tile is, not
        // on whether it is reached by straight or diagonal steps. Carrying light
        // along the axes alone makes this fail: a torch lights a diamond.
        let mut world = world();
        for x in 40..70 {
            for y in 70..100 {
                world.set(x, y, AIR);
            }
        }
        world.set(55, 85, TORCH);
        let mut map = LightMap::default();
        map.update(
            &world,
            TileRect {
                x0: 20,
                y0: 60,
                x1: 90,
                y1: 110,
            },
            &day(),
        );
        let at = |dx: i32, dy: i32| lit(&map, 55 + dx, 85 + dy);

        // 7.00 and 7.07 tiles away: near-equal distances, so near-equal light.
        let (straight, diagonal) = (at(7, 0), at(5, 5));
        assert!(
            (straight - diagonal).abs() < 0.05,
            "equal distances should be equally lit: {straight} straight vs {diagonal} diagonal"
        );
        // 4.24 tiles diagonally is closer than 5.00 straight, so it must be brighter.
        assert!(
            at(3, 3) > at(5, 0),
            "a nearer diagonal tile is dimmer than a farther straight one: {} vs {}",
            at(3, 3),
            at(5, 0)
        );
    }

    #[test]
    fn a_notch_dug_at_the_surface_is_not_in_shadow() {
        let mut world = World::new(128, 128);
        for x in 0..world.width() {
            for y in 40..world.height() {
                world.set(x, y, STONE);
            }
        }
        world.snapshot_surface();
        // A notch mined into flat ground, three wide and three deep.
        for x in 60..63 {
            for y in 40..43 {
                world.set(x, y, AIR);
            }
        }
        let mut map = LightMap::default();
        map.update(
            &world,
            TileRect {
                x0: 20,
                y0: 10,
                x1: 110,
                y1: 110,
            },
            &day(),
        );

        let open_sky = lit(&map, 61, 38);
        for depth in 0..3 {
            let y = 40 + depth;
            let seen = lit(&map, 61, y) * world.skyness(61, y);
            assert!(
                seen > open_sky * 0.75,
                "a hole three tiles deep should read close to open sky; at depth {depth} it \
                 is {seen:.3} against {open_sky:.3}"
            );
        }
    }

    #[test]
    fn light_turns_corners() {
        // An L-shaped tunnel: a torch in the vertical arm must light the horizontal
        // one, which it can only do by going round the bend.
        let mut world = world();
        for y in 70..80 {
            world.set(50, y, AIR);
        }
        for x in 50..60 {
            world.set(x, 79, AIR);
        }
        world.set(50, 71, TORCH);

        let mut map = LightMap::default();
        map.update(&world, view(), &day());
        let round_the_corner = lit(&map, 56, 79);
        assert!(
            round_the_corner > 0.02,
            "light should reach around a corner, got {round_the_corner}"
        );
        assert!(
            round_the_corner < lit(&map, 50, 75),
            "but it should be dimmer than the straight path"
        );
    }

    #[test]
    fn light_follows_a_winding_tunnel() {
        // Down, right, down, left: each reversal needs propagation the previous
        // pass cannot provide, so this is what decides how many rounds are needed.
        let mut world = world();
        let mut carve = |x0: i32, y0: i32, x1: i32, y1: i32| {
            for x in x0.min(x1)..=x0.max(x1) {
                for y in y0.min(y1)..=y0.max(y1) {
                    world.set(x, y, AIR);
                }
            }
        };
        carve(50, 70, 50, 76); // down
        carve(50, 76, 60, 76); // right
        carve(60, 76, 60, 82); // down
        carve(52, 82, 60, 82); // left
        world.set(50, 71, TORCH);

        let mut map = LightMap::default();
        map.update(
            &world,
            TileRect {
                x0: 20,
                y0: 60,
                x1: 90,
                y1: 110,
            },
            &day(),
        );
        let far_end = lit(&map, 53, 82);
        assert!(
            far_end > 0.01,
            "light should reach the far end of a winding tunnel, got {far_end}"
        );
    }

    #[test]
    fn light_climbs_a_staircase_tunnel() {
        // Alternating up and right: "up" is only carried by the backward pass and
        // "right" only by the forward one, so every step needs the other pass again.
        // This is the shape that decides whether one round of passes is enough.
        let mut world = world();
        let (mut x, mut y) = (45, 95);
        let bottom = [x, y];
        world.set(x, y, AIR);
        for _ in 0..10 {
            for _ in 0..2 {
                y -= 1;
                world.set(x, y, AIR);
            }
            for _ in 0..2 {
                x += 1;
                world.set(x, y, AIR);
            }
        }

        // Placed after carving, or the staircase would dig the torch out again.
        world.set(bottom[0], bottom[1], TORCH);

        let mut map = LightMap::default();
        map.update(
            &world,
            TileRect {
                x0: 20,
                y0: 60,
                x1: 90,
                y1: 110,
            },
            &day(),
        );
        let top = lit(&map, x, y);
        assert!(top > 0.005, "light should climb the staircase, got {top}");
    }

    #[test]
    fn light_does_not_leak_through_solid_rock() {
        let mut world = world();
        for y in 80..86 {
            world.set(50, y, AIR);
        }
        world.set(50, 79, TORCH);
        let mut map = LightMap::default();
        map.update(&world, view(), &day());
        assert!(
            lit(&map, 50 + 8, 82) < 0.05,
            "light should not carry eight tiles through stone"
        );
    }

    #[test]
    fn the_region_covers_the_view_with_margin_and_is_clamped_to_the_world() {
        let world = world();
        let mut map = LightMap::default();
        map.update(
            &world,
            TileRect {
                x0: 0,
                y0: 0,
                x1: 40,
                y1: 40,
            },
            &day(),
        );
        assert_eq!(map.origin(), [0, 0], "clamped at the world edge");
        assert_eq!(map.size(), [40 + MARGIN, 40 + MARGIN]);
        assert_eq!(map.texels().len(), (map.size()[0] * map.size()[1] * 4) as usize);
    }
    #[test]
    fn a_low_sun_casts_a_long_shadow_behind_a_wall() {
        let mut world = world();
        for y in 22..32 {
            world.set(60, y, STONE);
        }
        world.snapshot_surface();
        let mut map = LightMap::default();
        // 06:00: the sun is low on the left, so the shadow falls to the right.
        map.update(&world, view(), &Sky::at(0, 6.0).lighting());
        assert!(map.direct_at(55, 31) > 0.9, "the sunward side is in full sun");
        assert!(map.direct_at(64, 31) < 0.01, "behind the wall is in shadow");
        // tan(66°) × 10 tiles of wall: about 22 tiles of shadow.
        assert!(map.direct_at(76, 31) < 0.01, "the shadow is long at a low sun");
        assert!(map.direct_at(90, 31) > 0.9, "and it ends");
        let wall = map.direct_at(60, 25);
        assert!(
            (0.4..=0.6).contains(&wall),
            "the wall is lit on its sunward half only, got {wall}"
        );
    }

    #[test]
    fn leaves_let_part_of_the_light_through() {
        let mut world = world();
        world.snapshot_surface();
        // A canopy block 6 tiles wide, 4 deep, dense but not solid.
        for sx in 100..112 {
            for sy in 40..48 {
                world.set_foliage(sx, sy, 200);
            }
        }
        let mut map = LightMap::default();
        map.update(&world, view(), &Sky::at(0, 12.0).lighting());
        let under = map.direct_at(53, 28);
        assert!(
            under > 0.01 && under < 0.3,
            "light under the canopy is dappled, got {under}"
        );
        assert!(map.direct_at(40, 28) > 0.95, "beside it is full sun");
        assert!(
            map.direct_at(53, 28) < map.direct_at(53, 21),
            "it dims through the leaves"
        );
    }

    #[test]
    fn a_low_sun_reaches_under_an_overhang_that_shades_it_at_noon() {
        let mut world = world();
        for x in 50..70 {
            for y in 25..27 {
                world.set(x, y, STONE);
            }
        }
        world.snapshot_surface();
        let mut map = LightMap::default();
        map.update(&world, view(), &Sky::at(0, 12.0).lighting());
        assert!(
            map.direct_at(52, 30) < 0.01,
            "at noon the overhang shades what is under it"
        );
        map.update(&world, view(), &Sky::at(0, 6.0).lighting());
        assert!(map.direct_at(52, 30) > 0.9, "the morning sun slants in underneath");
    }

    #[test]
    fn ground_facing_the_light_is_lit_on_its_face_only() {
        let world = world();
        let mut map = LightMap::default();
        map.update(&world, view(), &day());
        assert!(
            map.direct_at(60, 32) > 0.4,
            "the top tile of the ground catches the sun"
        );
        assert_eq!(map.direct_at(60, 34), 0.0, "rock below it gets none");
    }
}
