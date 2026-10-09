//! The sky and the parallax silhouettes behind the world.
//!
//! A backdrop set is data: a sky gradient and a handful of layers, each a
//! horizontally repeating silhouette that scrolls at a fraction of the camera's
//! speed. Biomes and depth zones add sets, not code. The silhouettes are generated
//! here once, on the same 16-pixels-per-tile grid as the tile atlas, and the shader
//! only decides which layer covers a pixel and which of its three shades to use.

use crate::atlas;
use crate::camera::Camera;
use crate::sky::Lighting;
use crate::worldgen::hash;

/// Art pixels per tile, shared with the atlas so the backdrop sits on the same grid.
const ART_PX: f32 = atlas::CELL as f32;
/// Strip width in art pixels; every layer repeats after this many (128 tiles).
pub const STRIP: usize = 2048;
pub const LAYERS: usize = 5;
/// Marks a column a cloud layer leaves empty.
const NONE: f32 = 1.0e6;

#[derive(Clone, Copy)]
enum Shape {
    /// Peaked ranges: ridged noise.
    Ridges,
    /// Rolling ground: smooth noise only.
    Hills,
    /// Low ground carrying a row of round and pointed crowns.
    Treeline,
    /// Flat-bottomed puffs with gaps between them.
    Clouds,
}

pub struct Layer {
    shape: Shape,
    /// Share of the camera's movement the layer follows: 0 is pinned to the
    /// screen, 1 moves with the world. Smaller reads as further away.
    pub factor: f32,
    /// Height of the layer's base above the horizon, in tiles.
    baseline: f32,
    /// How far the silhouette rises above its base, in tiles.
    height: f32,
    colour: [u8; 3],
    /// How far the colour has faded toward the horizon sky: distance, in the
    /// atmospheric-perspective sense.
    haze: f32,
    /// Sideways drift in tiles per wind-second; only clouds move on their own.
    drift: f32,
}

pub struct BackdropSet {
    /// The daytime horizon colour far layers fade toward. The sky itself comes
    /// from the time of day.
    sky_horizon: [u8; 3],
    /// Back to front.
    pub layers: [Layer; LAYERS],
    seed: u64,
}

pub const FOREST: BackdropSet = BackdropSet {
    sky_horizon: [178, 214, 238],
    layers: [
        Layer { shape: Shape::Clouds, factor: 0.04, baseline: 26.0, height: 9.0, colour: [238, 242, 250], haze: 0.35, drift: 0.4 },
        Layer { shape: Shape::Ridges, factor: 0.1, baseline: -4.0, height: 34.0, colour: [84, 104, 146], haze: 0.55, drift: 0.0 },
        Layer { shape: Shape::Clouds, factor: 0.18, baseline: 18.0, height: 8.0, colour: [252, 252, 255], haze: 0.08, drift: 1.0 },
        Layer { shape: Shape::Treeline, factor: 0.35, baseline: 3.0, height: 12.0, colour: [40, 84, 74], haze: 0.32, drift: 0.0 },
        Layer { shape: Shape::Hills, factor: 0.6, baseline: -6.0, height: 14.0, colour: [56, 102, 50], haze: 0.1, drift: 0.0 },
    ],
    seed: 0xF0_2E57,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LayerUniform {
    rim: [f32; 4],
    body: [f32; 4],
    low: [f32; 4],
    factor: f32,
    /// In art pixels above the horizon.
    baseline: f32,
    drift: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BackdropUniform {
    sky_top: [f32; 4],
    sky_horizon: [f32; 4],
    /// World position of the screen's top-left pixel corner.
    origin: [f32; 2],
    camera: [f32; 2],
    ppt: f32,
    horizon: f32,
    time: f32,
    stars: f32,
    /// Sky light on the layers: ambient plus direct, so they darken at night and
    /// warm at dusk along with the world.
    light: [f32; 4],
    /// Centre in world tiles, then radius in art pixels (0 while below the horizon).
    sun: [f32; 4],
    sun_colour: [f32; 4],
    /// As `sun`, with the phase (0 new, 0.5 full) last.
    moon: [f32; 4],
    layers: [LayerUniform; LAYERS],
}

/// Disc radii, in art pixels.
const SUN_RADIUS: f32 = 22.0;
const MOON_RADIUS: f32 = 16.0;

impl BackdropSet {
    /// The frame's view of the set. `time` drives cloud drift and twinkling.
    pub fn uniform(&self, camera: &Camera, horizon: f32, time: f32, lighting: &Lighting) -> BackdropUniform {
        let horizon_srgb = self.sky_horizon.map(|c| c as f32 / 255.0);
        let light = [0, 1, 2].map(|c| (lighting.ambient[c] + lighting.direct[c]).min(1.0));
        // The sun's disc is its light's colour, paled toward white so it reads as
        // a source rather than an orange coin.
        let peak = lighting.direct.iter().cloned().fold(1e-3, f32::max);
        let disc = lighting.direct.map(|c| 0.5 + 0.5 * c / peak);
        let body = |angle: Option<f32>, radius: f32, last: f32| match angle {
            Some(a) => {
                let [x, y] = body_centre(camera, a);
                [x, y, radius, last]
            }
            None => [0.0, 0.0, 0.0, last],
        };
        BackdropUniform {
            sky_top: linear(lighting.sky_top),
            sky_horizon: linear(lighting.sky_horizon),
            origin: camera.px_to_tile([0.0, 0.0]),
            camera: camera.snapped_pos(),
            ppt: camera.pixels_per_tile(),
            horizon,
            time,
            stars: lighting.stars,
            light: [light[0], light[1], light[2], 1.0],
            sun: body(lighting.sun, SUN_RADIUS, 0.0),
            sun_colour: [disc[0], disc[1], disc[2], 1.0],
            moon: body(lighting.moon, MOON_RADIUS, lighting.phase),
            layers: self.layers.each_ref().map(|layer| {
                let [rim, body, low] = shades(layer, horizon_srgb);
                LayerUniform {
                    rim: linear(rim),
                    body: linear(body),
                    low: linear(low),
                    factor: layer.factor,
                    baseline: (layer.baseline * ART_PX).round(),
                    drift: layer.drift,
                    _pad: 0.0,
                }
            }),
        }
    }

    /// One row per layer, `STRIP` columns, each column the (top, bottom) of the
    /// silhouette in art pixels above the layer's base. Ground layers have no
    /// bottom; cloud columns without a cloud have nothing.
    pub fn strips(&self) -> Vec<[f32; 2]> {
        self.layers
            .iter()
            .enumerate()
            .flat_map(|(i, layer)| silhouette(layer, self.seed ^ hash(self.seed, i as i64)))
            .collect()
    }
}

/// Where the sun or moon sits on screen, in world tiles: pinned to the view,
/// swinging from low on one side through high overhead to low on the other.
/// Shared by the backdrop that draws it and the god rays that fan out from it.
pub fn body_centre(camera: &Camera, angle: f32) -> [f32; 2] {
    let centre = camera.snapped_pos();
    let corner = camera.px_to_tile([0.0, 0.0]);
    let half = [centre[0] - corner[0], centre[1] - corner[1]];
    [
        centre[0] + angle.sin() * half[0] * 0.85,
        centre[1] - half[1] * (0.15 + 0.8 * angle.cos()),
    ]
}

/// Rim, body and low shade, in sRGB. The body is faded toward the horizon by the
/// layer's haze, which is what makes far layers read as far. The rim is the lit
/// top edge; the low shade is the shadow under a cloud.
fn shades(layer: &Layer, horizon: [f32; 3]) -> [[f32; 3]; 3] {
    let body = mix(layer.colour.map(|c| c as f32 / 255.0), horizon, layer.haze);
    [mix(body, [1.0, 1.0, 0.94], 0.2), body, mix(body, horizon, 0.3)]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn linear(c: [f32; 3]) -> [f32; 4] {
    let l = c.map(|v| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) });
    [l[0], l[1], l[2], 1.0]
}

fn silhouette(layer: &Layer, seed: u64) -> Vec<[f32; 2]> {
    let tall = layer.height * ART_PX;
    let ground = |top: f32| [top.round(), -NONE];
    match layer.shape {
        Shape::Hills => (0..STRIP)
            .map(|x| {
                let n = 0.6 * noise(seed, x, 512) + 0.3 * noise(seed ^ 1, x, 128) + 0.1 * noise(seed ^ 2, x, 32);
                ground(tall * n)
            })
            .collect(),
        Shape::Ridges => (0..STRIP)
            .map(|x| {
                let ridge = 1.0 - (2.0 * noise(seed, x, 256) - 1.0).abs();
                let n = 0.65 * noise(seed ^ 1, x, 512) + 0.25 * noise(seed ^ 2, x, 64) + 0.1 * noise(seed ^ 3, x, 16);
                ground(tall * (0.45 * n + 0.55 * ridge * ridge))
            })
            .collect(),
        Shape::Treeline => {
            let ground_at: Vec<f32> = (0..STRIP)
                .map(|x| tall * (0.1 + 0.25 * noise(seed, x, 512)))
                .collect();
            let mut top = ground_at.clone();
            let mut x = 0;
            let mut i = 0;
            while x < STRIP {
                // Crowns sit low and overlap, so the line reads as canopy, not as a
                // row of towers: a crown's sides must reach the ground beneath it.
                let r = 8.0 + 10.0 * unit(seed, i, 0);
                let crown = ground_at[x] + r * (0.2 + 0.6 * unit(seed, i, 1));
                let pointed = unit(seed, i, 2) < 0.2;
                let reach = r.ceil() as i32;
                for dx in -reach..=reach {
                    let d = dx as f32;
                    let h = if pointed {
                        crown + r * 1.3 - d.abs() * 1.3
                    } else if d.abs() <= r {
                        crown + (r * r - d * d).sqrt()
                    } else {
                        continue;
                    };
                    let column = &mut top[(x as i32 + dx).rem_euclid(STRIP as i32) as usize];
                    *column = column.max(h.min(tall));
                }
                x += (r * (0.6 + 0.6 * unit(seed, i, 3))) as usize;
                i += 1;
            }
            top.into_iter().map(ground).collect()
        }
        Shape::Clouds => {
            let mut spans = vec![[-NONE, NONE]; STRIP];
            let clouds = 7;
            for c in 0..clouds {
                // One cloud per slot, kept inside it: two clouds sharing a column
                // at different heights would fill the gap between them.
                let slot = (STRIP / clouds) as f32;
                let centre = slot * (c as f32 + 0.5) + 40.0 * (unit(seed, c as i64, 0) - 0.5);
                let half = 30.0 + 60.0 * unit(seed, c as i64, 1);
                let base = (tall * 0.5 * unit(seed, c as i64, 2)).round();
                let puffs = (half / 14.0) as i64 + 2;
                for p in 0..puffs {
                    let along = centre - half + 2.0 * half * p as f32 / (puffs - 1) as f32;
                    // Middle puffs are the biggest, so each cloud swells in the middle.
                    let mid = 1.0 - (2.0 * p as f32 / (puffs - 1) as f32 - 1.0).abs();
                    let r = 10.0 + 14.0 * mid + 8.0 * unit(seed ^ 7, c as i64 * 64 + p, 3);
                    let reach = r.ceil() as i32;
                    for dx in -reach..=reach {
                        let d = dx as f32;
                        if d.abs() > r {
                            continue;
                        }
                        let h = (base + r * 0.4 + (r * r - d * d).sqrt()).min(tall).round();
                        let span = &mut spans[(along as i32 + dx).rem_euclid(STRIP as i32) as usize];
                        span[0] = span[0].max(h);
                        span[1] = span[1].min(base);
                    }
                }
            }
            spans
        }
    }
}

/// Value noise in 0..1 that repeats every `STRIP` pixels, so strips tile seamlessly.
/// `cell` must divide `STRIP`.
fn noise(seed: u64, x: usize, cell: usize) -> f32 {
    let count = (STRIP / cell) as i64;
    let i = (x / cell) as i64;
    let t = (x % cell) as f32 / cell as f32;
    let t = t * t * (3.0 - 2.0 * t);
    let a = unit(seed, i % count, 9);
    let b = unit(seed, (i + 1) % count, 9);
    a + (b - a) * t
}

fn unit(seed: u64, i: i64, salt: i64) -> f32 {
    (hash(seed ^ salt as u64, i) >> 40) as f32 / (1u64 << 24) as f32
}

/// A layer's height above the horizon at world row `y`, as the shader computes it.
/// Kept here so the anchoring contract is tested where it is written down.
#[cfg(test)]
fn layer_height(y: f32, camera_y: f32, horizon: f32, factor: f32) -> f32 {
    horizon - y + (camera_y - horizon) * (1.0 - factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_are_anchored_to_the_horizon_and_move_by_their_factor() {
        let horizon = 200.0;
        // Factor 0 is pinned to the screen: the same screen row shows the same
        // height wherever the camera is.
        for camera in [150.0, 200.0, 260.0] {
            assert_eq!(layer_height(camera - 10.0, camera, horizon, 0.0), 10.0);
        }
        // Factor 1 moves with the world: a world row always shows the same height.
        assert_eq!(layer_height(190.0, 150.0, horizon, 1.0), layer_height(190.0, 260.0, horizon, 1.0));
        // With the camera on the horizon every layer agrees, whatever its factor.
        assert_eq!(layer_height(190.0, horizon, horizon, 0.1), layer_height(190.0, horizon, horizon, 0.6));
        // Climbing 10 tiles sinks a far layer less than a near one: the same screen
        // row now shows a higher part of the layer, by `factor` of the climb.
        let sink = |factor| layer_height(180.0, 190.0, horizon, factor) - layer_height(190.0, 200.0, horizon, factor);
        assert!(sink(0.1) < sink(0.6), "the far layer should sink less on screen as you climb");
    }

    #[test]
    fn the_sun_crosses_the_view_from_low_left_through_high_centre_to_low_right() {
        let camera = Camera::new([500.0, 200.0], (1280, 720));
        let [dawn, noon, dusk] = [-crate::sky::ARC, 0.0, crate::sky::ARC].map(|a| body_centre(&camera, a));
        let (left, right) = (camera.px_to_tile([0.0, 0.0]), camera.px_to_tile([1280.0, 720.0]));
        assert!(dawn[0] > left[0] && dawn[0] < 500.0 && dusk[0] < right[0] && dusk[0] > 500.0, "on screen at either end");
        assert_eq!(noon[0], 500.0, "noon is centred");
        assert!(noon[1] < dawn[1] && noon[1] > left[1], "highest at noon, still on screen");
        assert_eq!(dawn[1], dusk[1], "the arc is symmetric");
    }

    #[test]
    fn strips_repeat_without_a_seam() {
        let strips = FOREST.strips();
        for (i, layer) in FOREST.layers.iter().enumerate() {
            if matches!(layer.shape, Shape::Clouds) {
                continue;
            }
            let row = &strips[i * STRIP..(i + 1) * STRIP];
            let steepest = row.windows(2).map(|w| (w[0][0] - w[1][0]).abs()).fold(0.0, f32::max);
            let seam = (row[STRIP - 1][0] - row[0][0]).abs();
            assert!(seam <= steepest, "layer {i} jumps {seam} px at the wrap, steepest step is {steepest}");
        }
    }

    #[test]
    fn silhouettes_stay_within_their_layer() {
        let strips = FOREST.strips();
        for (i, layer) in FOREST.layers.iter().enumerate() {
            let tall = layer.height * ART_PX;
            let row = &strips[i * STRIP..(i + 1) * STRIP];
            if matches!(layer.shape, Shape::Clouds) {
                let covered = row.iter().filter(|s| s[1] < s[0]).count();
                assert!(covered > STRIP / 8 && covered < STRIP * 3 / 4, "clouds should leave gaps, got {covered}");
                assert!(row.iter().all(|s| s[1] >= s[0] || (s[1] >= 0.0 && s[0] <= tall)));
            } else {
                assert!(row.iter().all(|s| s[0] >= 0.0 && s[0] <= tall), "layer {i} leaves its height");
                assert!(row.iter().all(|s| s[1] < -tall), "ground layers fill all the way down");
            }
        }
    }

    #[test]
    fn far_layers_fade_toward_the_horizon_and_scroll_slower() {
        let horizon = FOREST.sky_horizon.map(|c| c as f32 / 255.0);
        let distance = |c: [f32; 3]| (0..3).map(|i| (c[i] - horizon[i]).powi(2)).sum::<f32>();
        let ground: Vec<&Layer> = FOREST.layers.iter().filter(|l| !matches!(l.shape, Shape::Clouds)).collect();
        for pair in ground.windows(2) {
            let [far, near] = [pair[0], pair[1]].map(|l| distance(shades(l, horizon)[1]));
            assert!(far < near, "a further layer should sit closer to the horizon colour");
        }
        for pair in FOREST.layers.windows(2) {
            assert!(pair[0].factor < pair[1].factor, "layers are listed back to front");
        }
    }
}
