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
/// Cloud layers, and the height of each one's density map in art pixels.
pub const CLOUD_SLOTS: usize = 2;
pub const CLOUD_PX: usize = 192;

#[derive(Clone, Copy, Debug, PartialEq)]
enum CloudKind {
    /// Tall and puffy: a row of puffs along a flattish base, more piled on top.
    Heap,
    /// Long and low.
    Bank,
    /// Thin, slanting streaks, faint.
    Wisp,
}

#[derive(Clone, Copy, Debug)]
struct Cloud {
    kind: CloudKind,
    /// Centre along the strip, in art pixels.
    x: f32,
    /// Height of its base above the layer's, in art pixels.
    base: f32,
    width: f32,
    seed: u64,
}

#[derive(Clone, Copy)]
enum Shape {
    /// Peaked ranges: ridged noise.
    Ridges,
    /// Rolling ground: smooth noise only.
    Hills,
    /// Low ground carrying a row of round and pointed crowns.
    Treeline,
    /// Vapour drawn from a density map rather than a silhouette: clusters of
    /// heaps, banks and wisps. The number is the share of wisps; `soft` far
    /// layers are sampled smoothly, as high haze, near ones on the pixel grid.
    Clouds { wisps: f32, soft: bool },
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
    /// How much further the layer's foot fades into the sky than its top: haze
    /// lies thickest near the ground.
    foot_haze: f32,
    /// Morning mist gathers at this layer's foot.
    mist: bool,
}

pub struct BackdropSet {
    /// Back to front.
    pub layers: [Layer; LAYERS],
    seed: u64,
}

pub const FOREST: BackdropSet = BackdropSet {
    layers: [
        Layer { shape: Shape::Clouds { wisps: 0.55, soft: true }, factor: 0.04, baseline: 26.0, height: 12.0, colour: [238, 242, 250], haze: 0.35, drift: 0.4, foot_haze: 0.0, mist: false },
        Layer { shape: Shape::Ridges, factor: 0.1, baseline: -4.0, height: 34.0, colour: [84, 104, 146], haze: 0.55, drift: 0.0, foot_haze: 0.6, mist: false },
        Layer { shape: Shape::Clouds { wisps: 0.12, soft: false }, factor: 0.18, baseline: 16.0, height: 12.0, colour: [252, 252, 255], haze: 0.08, drift: 1.0, foot_haze: 0.0, mist: false },
        Layer { shape: Shape::Treeline, factor: 0.35, baseline: 3.0, height: 12.0, colour: [40, 84, 74], haze: 0.32, drift: 0.0, foot_haze: 0.4, mist: true },
        Layer { shape: Shape::Hills, factor: 0.6, baseline: -6.0, height: 14.0, colour: [56, 102, 50], haze: 0.1, drift: 0.0, foot_haze: 0.15, mist: true },
    ],
    seed: 0xF0_2E57,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LayerUniform {
    rim: [f32; 4],
    body: [f32; 4],
    low: [f32; 4],
    /// Ground (0) or which cloud map (1 or 2), its height in art pixels, foot haze
    /// 0..1 (for clouds: whether to sample softly), and whether mist gathers at
    /// its foot (1).
    shape: [f32; 4],
    factor: f32,
    /// In art pixels above the horizon.
    baseline: f32,
    drift: f32,
    haze: f32,
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
    /// Morning mist 0..1, how low the sun is 0..1 (1 on the horizon), and the
    /// unit vector toward the light.
    air: [f32; 4],
    layers: [LayerUniform; LAYERS],
}

/// Disc radii, in art pixels.
const SUN_RADIUS: f32 = 20.0;
const MOON_RADIUS: f32 = 18.0;

impl BackdropSet {
    /// The frame's view of the set. `time` drives cloud drift and twinkling.
    pub fn uniform(&self, camera: &Camera, horizon: f32, time: f32, lighting: &Lighting) -> BackdropUniform {
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
            air: [
                lighting.mist,
                lighting.sun.map_or(0.0, |a| (a.abs() / crate::sky::ARC).powi(2)),
                lighting.toward[0],
                lighting.toward[1],
            ],
            layers: self.layers.each_ref().map(|layer| {
                let [rim, body, low] = shades(layer);
                LayerUniform {
                    rim: linear(rim),
                    body: linear(body),
                    low: linear(low),
                    shape: [
                        cloud_slot(&self.layers, layer).map_or(0.0, |slot| slot as f32 + 1.0),
                        layer.height * ART_PX,
                        match layer.shape {
                            Shape::Clouds { soft, .. } => soft as u8 as f32,
                            _ => layer.foot_haze,
                        },
                        layer.mist as u8 as f32,
                    ],
                    factor: layer.factor,
                    baseline: (layer.baseline * ART_PX).round(),
                    drift: layer.drift,
                    haze: layer.haze,
                }
            }),
        }
    }

    /// The cloud layers' density maps, one after the other: `CLOUD_PX` rows each
    /// of `STRIP` columns, row `r` being `r` art pixels above the layer's base,
    /// vapour 0..255.
    pub fn clouds(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CLOUD_SLOTS * CLOUD_PX * STRIP);
        for (i, layer) in self.layers.iter().enumerate() {
            if let Shape::Clouds { wisps, .. } = layer.shape {
                let plan = cloud_plan(wisps, self.seed ^ hash(self.seed, i as i64));
                out.extend(cloud_density(&plan, self.seed ^ i as u64).iter().map(|d| (d * 255.0).round() as u8));
            }
        }
        out
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

/// Rim, body and low shade, in sRGB, before haze. The rim is the lit top edge;
/// the low shade is the shadow under a cloud. Haze is the shader's: it fades a
/// lit layer toward the sky's horizon as it is now, so far layers turn pale by
/// day and dark at night, rather than glowing with daytime haze under the moon.
fn shades(layer: &Layer) -> [[f32; 3]; 3] {
    let body = layer.colour.map(|c| c as f32 / 255.0);
    [mix(body, [1.0, 1.0, 0.94], 0.2), body, mix(body, [0.0; 3], 0.15)]
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
        // Clouds come from their density map; the strip row stays empty.
        Shape::Clouds { .. } => vec![[-NONE, NONE]; STRIP],
    }
}

/// Which cloud map a layer reads, if it is a cloud layer: they are numbered in
/// order back to front.
fn cloud_slot(layers: &[Layer], layer: &Layer) -> Option<usize> {
    matches!(layer.shape, Shape::Clouds { .. })
        .then(|| layers.iter().take_while(|l| !std::ptr::eq(*l, layer)).filter(|l| matches!(l.shape, Shape::Clouds { .. })).count())
}

/// Where the clouds of one layer go: in clusters, like crowns in a wood, with
/// open sky of varying width between them. `wisps` is the share of wisps.
fn cloud_plan(wisps: f32, seed: u64) -> Vec<Cloud> {
    let mut plan = Vec::new();
    let mut x = 0.0;
    let mut i = 0;
    while x < STRIP as f32 {
        let r = |k: i64| unit(seed, i, k);
        let count = 1 + (r(0) * r(0) * 4.0) as i64;
        let spread = 40.0 + 160.0 * r(1);
        for c in 0..count {
            let q = |k: i64| unit(seed ^ 0xC10D, i * 16 + c, k);
            let roll = q(0);
            let kind = if roll < wisps { CloudKind::Wisp } else if roll < wisps + 0.2 { CloudKind::Bank } else { CloudKind::Heap };
            // Mostly small and middling, now and then a big one.
            let size = q(1).powf(1.6);
            let width = match kind {
                CloudKind::Heap => 40.0 + 240.0 * size,
                CloudKind::Bank => 100.0 + 240.0 * size,
                CloudKind::Wisp => 80.0 + 260.0 * size,
            };
            let base = match kind {
                CloudKind::Heap => 8.0 + 40.0 * q(2),
                CloudKind::Bank => 10.0 + 90.0 * q(2),
                CloudKind::Wisp => 30.0 + 130.0 * q(2),
            };
            plan.push(Cloud { kind, x: x + (q(3) - 0.5) * spread, base, width, seed: hash(seed, i * 16 + c) });
        }
        x += spread + 160.0 + 640.0 * r(2) * r(2);
        i += 1;
    }
    plan
}

/// Vapour 0..1 per art pixel for a layer's clouds, `CLOUD_PX` rows of `STRIP`,
/// row 0 at the base. Each cloud is soft blobs of vapour, merged where they
/// overlap, then broken up by noise so edges thin out raggedly to nothing.
fn cloud_density(plan: &[Cloud], seed: u64) -> Vec<f32> {
    let mut vapour = vec![0.0f32; CLOUD_PX * STRIP];
    // A soft ellipse of vapour, `peak` thick at its centre, sheared sideways by
    // `slant` per pixel of height.
    let mut blob = |cx: f32, cy: f32, rx: f32, ry: f32, peak: f32, slant: f32| {
        let top = (CLOUD_PX as f32 - 3.0).min(cy + ry);
        for y in (cy - ry).floor().max(1.0) as i32..=top as i32 {
            let shift = (y as f32 - cy) * slant;
            for x in (cx + shift - rx).floor() as i32..=(cx + shift + rx).ceil() as i32 {
                let e = ((x as f32 + 0.5 - cx - shift) / rx).powi(2) + ((y as f32 + 0.5 - cy) / ry).powi(2);
                if e >= 1.0 {
                    continue;
                }
                let v = peak * (1.0 - e).powf(1.2);
                let cell = &mut vapour[y as usize * STRIP + x.rem_euclid(STRIP as i32) as usize];
                *cell = 1.0 - (1.0 - *cell) * (1.0 - v);
            }
        }
    };
    let room = CLOUD_PX as f32 - 4.0;
    for cloud in plan {
        let r = |k: i64| unit(cloud.seed, k, 0);
        let half = cloud.width * 0.5;
        match cloud.kind {
            CloudKind::Heap => {
                // A dome: a row of puffs sitting on the base, biggest in the
                // middle, with a few more piled into the middle of it.
                let tall = (cloud.width * (0.3 + 0.2 * r(1))).min(room - cloud.base);
                let puffs = (cloud.width / 22.0) as i64 + 2;
                for p in 0..puffs {
                    let t = p as f32 / (puffs - 1) as f32;
                    let mid = 1.0 - (2.0 * t - 1.0).abs();
                    let rad = (5.0 + tall * (0.2 + 0.3 * mid) * (0.8 + 0.4 * r(10 + p))).min(room * 0.5);
                    blob(cloud.x - half * 0.9 + cloud.width * 0.9 * t, cloud.base + rad * 0.9, rad * 1.2, rad, 1.0, 0.0);
                }
                for p in 0..1 + (r(2) * 3.0) as i64 {
                    let rad = tall * (0.28 + 0.14 * r(20 + p));
                    let cy = (cloud.base + tall - rad).max(cloud.base + rad * 0.9);
                    blob(cloud.x + (r(30 + p) - 0.5) * half * 0.8, cy, rad * 1.1, rad, 1.0, 0.0);
                }
            }
            CloudKind::Bank => {
                let pieces = (cloud.width / 40.0) as i64 + 2;
                let thick = 8.0 + 14.0 * r(1);
                for p in 0..pieces {
                    let t = p as f32 / (pieces - 1) as f32;
                    let rx = 30.0 + 40.0 * r(10 + p);
                    let ry = thick * (0.6 + 0.6 * r(20 + p)) * (1.0 - 0.5 * (2.0 * t - 1.0).abs());
                    blob(cloud.x - half + cloud.width * t, cloud.base + ry + (r(30 + p) - 0.5) * 6.0, rx, ry.max(3.0), 0.85, 0.0);
                }
            }
            CloudKind::Wisp => {
                let slant = (r(1) - 0.5) * 1.2;
                for p in 0..3 + (r(2) * 4.0) as i64 {
                    let rx = 25.0 + half * 0.6 * r(10 + p);
                    let ry = 2.5 + 4.0 * r(20 + p);
                    let along = (r(30 + p) - 0.5) * cloud.width;
                    blob(cloud.x + along, cloud.base + along * 0.06 + (r(40 + p) - 0.5) * 10.0, rx, ry, 0.45 + 0.2 * r(50 + p), slant);
                }
            }
        }
    }
    // Break the blobs up: thin vapour at the edges goes patchy, then nothing.
    for y in 0..CLOUD_PX {
        for x in 0..STRIP {
            let n = 0.5 * noise2(seed, x, y, 32) + 0.3 * noise2(seed ^ 1, x, y, 8) + 0.2 * noise2(seed ^ 2, x, y, 4);
            let v = &mut vapour[y * STRIP + x];
            *v = ((*v * (0.55 + 0.9 * n)) - 0.12).clamp(0.0, 1.0) / 0.88;
        }
    }
    vapour
}

/// 2D value noise in 0..1, repeating every `STRIP` pixels across.
fn noise2(seed: u64, x: usize, y: usize, cell: usize) -> f32 {
    let count = (STRIP / cell) as i64;
    let (i, j) = ((x / cell) as i64, (y / cell) as i64);
    let s = |v: f32| v * v * (3.0 - 2.0 * v);
    let (tx, ty) = (s((x % cell) as f32 / cell as f32), s((y % cell) as f32 / cell as f32));
    let at = |i: i64, j: i64| unit(seed ^ (j as u64).wrapping_mul(0x9E37_79B9), i.rem_euclid(count), 9);
    let top = at(i, j) + (at(i + 1, j) - at(i, j)) * tx;
    let bottom = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * tx;
    top + (bottom - top) * ty
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
            if matches!(layer.shape, Shape::Clouds { .. }) {
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
            if matches!(layer.shape, Shape::Clouds { .. }) {
                assert!(row.iter().all(|s| s[1] >= s[0]), "cloud layers draw from their density map");
                assert_eq!(tall as usize, CLOUD_PX, "a cloud layer is as tall as its map");
            } else {
                assert!(row.iter().all(|s| s[0] >= 0.0 && s[0] <= tall), "layer {i} leaves its height");
                assert!(row.iter().all(|s| s[1] < -tall), "ground layers fill all the way down");
            }
        }
    }

    #[test]
    fn clouds_vary_in_kind_and_size_and_gather_in_clusters() {
        let plans: Vec<Vec<Cloud>> = FOREST.layers.iter().enumerate().filter_map(|(i, l)| match l.shape {
            Shape::Clouds { wisps, .. } => Some(cloud_plan(wisps, FOREST.seed ^ hash(FOREST.seed, i as i64))),
            _ => None,
        }).collect();
        let all: Vec<&Cloud> = plans.iter().flatten().collect();
        for kind in [CloudKind::Heap, CloudKind::Bank, CloudKind::Wisp] {
            assert!(all.iter().any(|c| c.kind == kind), "no {kind:?}");
        }
        let heaps: Vec<f32> = all.iter().filter(|c| c.kind == CloudKind::Heap).map(|c| c.width).collect();
        let (small, big) = (heaps.iter().cloned().fold(f32::MAX, f32::min), heaps.iter().cloned().fold(0.0, f32::max));
        assert!(small < 4.0 * ART_PX && big > 10.0 * ART_PX, "heaps should run from a few tiles to many: {small}..{big}");
        // Clustered: the open gaps between clouds vary a lot, some wide.
        let mut xs: Vec<f32> = plans[1].iter().map(|c| c.x.rem_euclid(STRIP as f32)).collect();
        xs.sort_by(f32::total_cmp);
        let gaps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().any(|&g| g < 30.0) && gaps.iter().any(|&g| g > 150.0), "clouds should cluster: {gaps:?}");
    }

    #[test]
    fn cloud_vapour_thins_to_nothing_and_wraps_without_a_seam() {
        let maps = FOREST.clouds();
        assert_eq!(maps.len(), CLOUD_SLOTS * CLOUD_PX * STRIP);
        for slot in 0..CLOUD_SLOTS {
            let map = &maps[slot * CLOUD_PX * STRIP..(slot + 1) * CLOUD_PX * STRIP];
            let row = |y: usize| &map[y * STRIP..(y + 1) * STRIP];
            assert!(row(0).iter().chain(row(CLOUD_PX - 1)).all(|&v| v == 0), "vapour reaches the edge of the map");
            let covered = (0..STRIP).filter(|&x| (0..CLOUD_PX).any(|y| map[y * STRIP + x] > 25)).count();
            assert!(covered > STRIP / 5 && covered < STRIP * 4 / 5, "slot {slot} covers {covered} of {STRIP} columns");
            // Thin vapour exists, not just solid cores: the clouds are see-through at their edges.
            let thin = map.iter().filter(|&&v| v > 0 && v < 100).count();
            assert!(thin > map.iter().filter(|&&v| v >= 200).count() / 4, "slot {slot} has hardly any thin vapour");
            for y in 0..CLOUD_PX {
                let (a, b) = (row(y)[STRIP - 1] as i32, row(y)[0] as i32);
                assert!((a - b).abs() < 60, "the map jumps at the wrap on row {y}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn far_layers_fade_toward_the_horizon_and_scroll_slower() {
        let ground: Vec<&Layer> = FOREST.layers.iter().filter(|l| !matches!(l.shape, Shape::Clouds { .. })).collect();
        for pair in ground.windows(2) {
            assert!(pair[0].haze > pair[1].haze, "a further layer should fade further toward the horizon");
        }
        for pair in FOREST.layers.windows(2) {
            assert!(pair[0].factor < pair[1].factor, "layers are listed back to front");
        }
    }
}
