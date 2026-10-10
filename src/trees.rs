//! Trees: trunk cells live in the tile grid, where chopping (and later climbing)
//! acts on them; the tree draws itself on top as one bending shape.
//!
//! The grid draws a trunk cell as the air behind it (`tiles::drawn_as`), so what
//! you see is the drawn tree and what you hit is the grid. Sway is kept small for
//! that reason: much more and the drawing would drift off the cells you chop.

use std::collections::BTreeMap;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::ops::Range;

use crate::atlas::{self, LEAF_MASK, LEAF_VARIANTS, MAT_BARK, MAT_LEAVES, SOLID_MASK};
use crate::autotile::{Instance, textured};
use crate::camera::TileRect;
use crate::physics::{self, GRAVITY, MAX_FALL};
use crate::tiles::{AIR, TRUNK};
use crate::world::World;
use crate::worldgen::hash;

pub const MIN_HEIGHT: i32 = 7;
pub const MAX_HEIGHT: i32 = 20;
/// Sideways sway at the crown in full wind, in tiles.
const SWAY: f32 = 0.15;
/// Trunks are drawn a little narrower than their cells, so they read as trunks
/// rather than as a column of blocks.
const TRUNK_FILL: f32 = 0.8;
/// How much narrower the top is than the base, as a fraction.
const TAPER: f32 = 0.3;
/// Extra width on the bottom row, so the trunk meets the ground with a flare.
const FLARE: f32 = 0.35;
const BRANCH_THICKNESS: f32 = 0.26;
/// How far the bottom row reaches down into the ground, in tiles: deeper than any
/// bite in a ragged edge, so a trunk stands planted instead of hovering over one.
const ROOT_SINK: f32 = 2.0 / 16.0;
/// Foliage cells per tile along each axis.
const SUB: i32 = 2;
/// Foliage thinner than this is left out entirely, which is what gives crowns
/// ragged edges and gaps for light to come through.
const FOLIAGE_MIN: f32 = 0.18;

/// An elliptical mass of foliage: a crown or a clump at a branch tip.
struct Blob {
    centre: [f32; 2],
    radius: [f32; 2],
    /// Height above the ground it sways as, in trunk rows.
    h: f32,
    /// Drawn in front of the trunk rather than behind it.
    front: bool,
}

/// Foliage cells, keyed by half-tile coordinate, with density 0..1 and sway height.
type Foliage = BTreeMap<(i32, i32), (f32, f32)>;

/// Deterministic 0..1 from a seed and a key.
fn unit(seed: u64, key: i64) -> f32 {
    (hash(seed, key) % 10_000) as f32 / 10_000.0
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tree {
    /// Left trunk column.
    pub x: i32,
    /// Row of the ground tile the trunk stands on; trunk row `i` is at `ground - 1 - i`.
    pub ground: i32,
    /// 1 or 2 columns.
    pub width: i32,
    /// Trunk rows still standing.
    pub height: i32,
    /// Height as generated. Branch placement and taper key off this, so a tree
    /// keeps its shape as it is cut down.
    pub full: i32,
    pub seed: u64,
}

struct Branch {
    row: i32,
    side: f32,
    len: f32,
    /// Upward angle from horizontal, radians.
    angle: f32,
}

impl Tree {
    pub fn row_y(&self, i: i32) -> i32 {
        self.ground - 1 - i
    }

    fn row_of(&self, x: i32, y: i32) -> Option<i32> {
        let i = self.ground - 1 - y;
        (x >= self.x && x < self.x + self.width && i >= 0 && i < self.height).then_some(i)
    }

    fn present(&self, world: &World, i: i32, c: i32) -> bool {
        i >= 0 && i < self.height && world.get(self.x + c, self.row_y(i)) == TRUNK
    }

    fn centre(&self) -> f32 {
        self.x as f32 + self.width as f32 * 0.5
    }

    fn branches(&self) -> Vec<Branch> {
        let count = 1 + (self.full / 6) + (hash(self.seed, 1) % 2) as i32;
        let lowest = (self.full / 3).max(2);
        let span = (self.full - 1 - lowest).max(1);
        let mut side = if hash(self.seed, 2) % 2 == 0 { 1.0 } else { -1.0 };
        (0..count)
            .map(|k| {
                let h = hash(self.seed, 10 + k as i64);
                side = -side;
                Branch {
                    row: lowest + (h % span as u64) as i32,
                    side,
                    len: 1.2 + (h >> 16) as f32 % 1000.0 / 1000.0 * 1.8 + self.width as f32 * 0.4,
                    angle: 0.35 + (h >> 32) as f32 % 1000.0 / 1000.0 * 0.5,
                }
            })
            .collect()
    }

    /// Where a branch leaves the trunk, before sway.
    fn branch_attach(&self, b: &Branch) -> [f32; 2] {
        let half_w = self.width as f32 * TRUNK_FILL * 0.5 * 0.6;
        [self.centre() + b.side * half_w, self.row_y(b.row) as f32 + 0.5]
    }

    fn branch_dir(b: &Branch) -> [f32; 2] {
        [b.side * b.angle.cos(), -b.angle.sin()]
    }

    /// The masses of foliage on rows `rows`: the crown as 2-3 overlapping lobes
    /// (only while the trunk is whole, since it sits on the top row) and a clump at
    /// each branch tip. Ordered back to front.
    fn blobs(&self, rows: Range<i32>) -> Vec<Blob> {
        let mut back = Vec::new();
        let mut front = Vec::new();
        let top = self.full - 1;
        let crown = self.height == self.full && rows.contains(&top);
        if crown {
            let rx = 1.8 + self.full as f32 * 0.12 + self.width as f32 * 0.6 + unit(self.seed, 20) * 0.8;
            let ry = rx * (0.65 + unit(self.seed, 21) * 0.2);
            let cx = self.centre() + (unit(self.seed, 22) - 0.5) * 0.8;
            let cy = self.row_y(top) as f32 - ry * 0.45;
            let s = if hash(self.seed, 23) % 2 == 0 { 1.0 } else { -1.0 };
            let h = self.full as f32;
            let lobe = |dx: f32, dy: f32, sx: f32, sy: f32, front: bool| Blob {
                centre: [cx + dx * rx, cy + dy * ry],
                radius: [rx * sx, ry * sy],
                h,
                front,
            };
            // Back lobes sit behind the trunk; the front one hangs lower and covers
            // the top of it, which is what makes the crown read as having depth.
            if hash(self.seed, 24) % 2 == 0 {
                back.push(lobe(s * 0.15, -0.1, 0.95, 0.9, false));
            } else {
                back.push(lobe(-0.38, -0.05, 0.68, 0.8, false));
                back.push(lobe(0.38, -0.15, 0.68, 0.8, false));
            }
            front.push(lobe(-s * 0.2, 0.25, 0.62, 0.62, true));
        }
        let mut clumps = Vec::new();
        for b in self.branches() {
            if !rows.contains(&b.row) {
                continue;
            }
            let (a, d) = (self.branch_attach(&b), Self::branch_dir(&b));
            let r = 0.9 + b.len * 0.3;
            let tip = [a[0] + d[0] * b.len, a[1] + d[1] * b.len];
            clumps.push(Blob { centre: tip, radius: [r, r * 0.8], h: b.row as f32 + 0.5, front: true });
        }
        back.into_iter().chain(clumps).chain(front).collect()
    }

    /// One blob's foliage cells and their densities: its ellipse broken up by noise.
    fn blob_cells(&self, blob: &Blob) -> Vec<((i32, i32), f32)> {
        let sub = SUB as f32;
        let [cx, cy] = blob.centre;
        let [rx, ry] = blob.radius;
        let mut cells = Vec::new();
        for sy in ((cy - ry) * sub).floor() as i32..=((cy + ry) * sub).ceil() as i32 {
            for sx in ((cx - rx) * sub).floor() as i32..=((cx + rx) * sub).ceil() as i32 {
                let p = [(sx as f32 + 0.5) / sub, (sy as f32 + 0.5) / sub];
                let e = ((p[0] - cx) / rx).powi(2) + ((p[1] - cy) / ry).powi(2);
                if e >= 1.0 {
                    continue;
                }
                let n = unit(hash(self.seed ^ 0xF011, sx as i64), sy as i64);
                let v = (1.0 - e).sqrt() * 1.3 + (n - 0.5) * 0.9;
                if v >= FOLIAGE_MIN {
                    cells.push(((sx, sy), v.min(1.0)));
                }
            }
        }
        cells
    }

    /// Foliage cells this tree's rows `rows` carry, densest blob winning where
    /// blobs overlap. This is what goes into the grid.
    // ponytail: recomputed per call (per frame for visible trees); cache on the tree
    // if it shows up in a profile.
    fn foliage(&self, rows: Range<i32>) -> Foliage {
        let mut cells = Foliage::new();
        for blob in self.blobs(rows) {
            for (cell, v) in self.blob_cells(&blob) {
                let entry = cells.entry(cell).or_insert((0.0, blob.h));
                if v > entry.0 {
                    *entry = (v, blob.h);
                }
            }
        }
        cells
    }

    /// Draws rows `rows`: back lobes, trunk and branches, then branch clumps and the
    /// front lobe, each as its own outlined object. `density` gives a foliage cell's
    /// density from its own blob's value; zero leaves it out.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        rows: Range<i32>,
        cells: impl Fn(i32, i32) -> bool,
        sway: impl Fn(f32) -> f32,
        density: impl Fn((i32, i32), f32) -> f32,
        next_id: &mut u32,
        out: &mut Vec<Instance>,
    ) {
        let blobs = self.blobs(rows.clone());
        let mut take_id = || {
            *next_id += 1;
            *next_id - 1
        };
        for blob in blobs.iter().filter(|b| !b.front) {
            self.leaves(blob, &density, &sway, take_id(), out);
        }
        self.quads(rows, cells, &sway, take_id(), out);
        for blob in blobs.iter().filter(|b| b.front) {
            self.leaves(blob, &density, &sway, take_id(), out);
        }
    }

    /// One leaf cluster per foliage cell of `blob`. Drawn a tile across, wider than
    /// the half-tile cell, so neighbours overlap into a mass rather than a grid.
    /// Never rotated or scaled, and placed on whole art pixels, sway included, so
    /// every leaf pixel sits on the same pixel grid as the terrain; how dense the
    /// cell is picks how full a clump to draw instead.
    fn leaves(
        &self,
        blob: &Blob,
        density: &impl Fn((i32, i32), f32) -> f32,
        sway: &impl Fn(f32) -> f32,
        id: u32,
        out: &mut Vec<Instance>,
    ) {
        let sub = SUB as f32;
        let offset = sway(blob.h);
        for ((sx, sy), own) in self.blob_cells(blob) {
            let density = density((sx, sy), own);
            if density <= 0.0 {
                continue;
            }
            let key = hash(self.seed ^ 0x1EAF, sx as i64);
            let r = |k: i64| unit(key, ((sy as i64) << 4) | k);
            let centre = [
                (sx as f32 + 0.5) / sub + (r(0) - 0.5) * 0.3 + offset,
                (sy as f32 + 0.5) / sub + (r(1) - 0.5) * 0.3,
            ];
            let variant = ((density * 3.0 + r(2)).floor() as u16).min(LEAF_VARIANTS - 1);
            let snap = |v: f32| (v * atlas::CELL as f32).round() / atlas::CELL as f32;
            let shade = 0.78 + 0.22 * r(3);
            let warm = 0.92 + 0.16 * r(4);
            out.push(Instance {
                pos: [snap(centre[0] - 0.5), snap(centre[1] - 0.5)],
                axis_x: [1.0, 0.0],
                axis_y: [0.0, 1.0],
                mask: LEAF_MASK + variant,
                mat: textured(MAT_LEAVES, [sx.div_euclid(SUB), sy.div_euclid(SUB)]),
                tint: [
                    (shade * warm * 255.0).min(255.0) as u8,
                    (shade * 255.0) as u8,
                    (shade * 0.9 * 255.0) as u8,
                    255,
                ],
                edges: 0,
                id,
            });
        }
    }

    /// Sideways offset of the trunk at `h` rows above the ground.
    fn sway(&self, h: f32, clock: f32) -> f32 {
        let phase = unit(self.seed, 3) * TAU;
        let bend = (h / self.full as f32).powi(2);
        wind(clock) * SWAY * bend * (clock * 1.3 + phase).sin()
    }

    /// Trunk and branch quads for rows `rows`, drawn where `cells` says a cell is
    /// present. One object: branches join the trunk without a line between them.
    fn quads(
        &self,
        rows: Range<i32>,
        cells: impl Fn(i32, i32) -> bool,
        sway: &impl Fn(f32) -> f32,
        id: u32,
        out: &mut Vec<Instance>,
    ) {
        for i in rows.clone() {
            let present: Vec<i32> = (0..self.width).filter(|&c| cells(i, c)).collect();
            if present.is_empty() {
                continue;
            }
            let taper = 1.0 - TAPER * i as f32 / self.full as f32;
            let flare = if i == 0 { FLARE } else { 0.0 };
            let (centre, w) = if present.len() as i32 == self.width {
                (self.centre(), self.width as f32 * TRUNK_FILL * taper + flare)
            } else {
                // Half of a two-wide trunk is chopped through: draw only the half left.
                (self.x as f32 + present[0] as f32 + 0.5, TRUNK_FILL * taper + flare)
            };
            let top = self.row_y(i) as f32;
            let (s_top, s_bottom) = (sway((i + 1) as f32), sway(i as f32));
            let sink = if i == 0 { ROOT_SINK } else { 0.0 };
            out.push(Instance {
                pos: [centre - w * 0.5 + s_top, top],
                axis_x: [w, 0.0],
                axis_y: [s_bottom - s_top, 1.0 + sink],
                mask: SOLID_MASK,
                mat: textured(MAT_BARK, [self.x, self.row_y(i)]),
                tint: [255; 4],
                edges: 0,
                id,
            });
        }
        for b in self.branches() {
            if !rows.contains(&b.row) || !(0..self.width).any(|c| cells(b.row, c)) {
                continue;
            }
            let attach = self.branch_attach(&b);
            let attach = [attach[0] + sway(b.row as f32 + 0.5), attach[1]];
            let dir = Self::branch_dir(&b);
            let normal = [-dir[1], dir[0]];
            out.push(Instance {
                pos: [
                    attach[0] - normal[0] * BRANCH_THICKNESS * 0.5,
                    attach[1] - normal[1] * BRANCH_THICKNESS * 0.5,
                ],
                axis_x: [dir[0] * b.len, dir[1] * b.len],
                axis_y: [normal[0] * BRANCH_THICKNESS, normal[1] * BRANCH_THICKNESS],
                mask: SOLID_MASK,
                mat: textured(MAT_BARK, [self.x, self.row_y(b.row)]),
                tint: [255; 4],
                edges: 0,
                id,
            });
        }
    }
}

/// Wind strength, 0..1, drifting slowly so calm and gusty spells alternate.
// ponytail: global wind from two slow sines; a weather system replaces it.
fn wind(clock: f32) -> f32 {
    (0.55 + 0.45 * (clock * 0.21).sin() * (clock * 0.13 + 1.0).sin()).clamp(0.0, 1.0)
}

/// Felled parts at least this many rows long topple once they reach the ground;
/// shorter ones just hit it and break.
const TOPPLE_MIN_ROWS: i32 = 4;
/// How far above a point terrain must start for the trunk's centreline to count as
/// touching it. Keeps a foot resting exactly on a tile top from reading as buried.
const CONTACT_SLACK: f32 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Dropping straight down onto whatever is under the cut.
    Drop,
    /// Tipping about its foot, faster as it leans.
    Hinged,
    /// Tipped past horizontal without landing — off a ledge — so it falls freely,
    /// keeping the spin it built up, until any part of it hits terrain.
    Free,
}

/// The part of a tree above a cut, on its way down. Wherever the cut was, it first
/// drops straight onto solid ground — hinging where it was cut would pivot on the
/// air the cut left — then, if long enough, tips about its foot. If it reaches
/// horizontal still in the air, it was toppled over a ledge and falls on from there.
#[derive(Clone, Debug, PartialEq)]
pub struct Falling {
    tree: Tree,
    /// First trunk row of the fallen part; it runs to `tree.height`.
    from: i32,
    /// Which cells were present, per row from `from`, per column.
    cells: Vec<[bool; 2]>,
    /// Which way it tips: 1 right, -1 left.
    dir: f32,
    phase: Phase,
    /// Bottom-centre of the fallen part, where it currently is.
    foot: [f32; 2],
    /// Velocity of the foot while dropping, of the centre while falling free.
    vel: [f32; 2],
    angle: f32,
    spin: f32,
}

impl Falling {
    fn new(tree: Tree, from: i32, cells: Vec<[bool; 2]>, dir: f32) -> Falling {
        let foot = [tree.centre(), (tree.ground - from) as f32];
        Falling { tree, from, cells, dir, phase: Phase::Drop, foot, vel: [0.0; 2], angle: 0.05, spin: 0.0 }
    }

    fn length(&self) -> f32 {
        (self.tree.height - self.from) as f32
    }

    fn topples(&self) -> bool {
        self.tree.height - self.from >= TOPPLE_MIN_ROWS
    }

    /// The point `t` tiles along the trunk from its foot.
    fn along(&self, t: f32) -> [f32; 2] {
        let d = rotate([0.0, -t], self.dir * self.angle);
        [self.foot[0] + d[0], self.foot[1] + d[1]]
    }

    /// Whether the trunk's centreline touches terrain anywhere from `t0` to the tip.
    fn touches(&self, world: &World, t0: f32) -> bool {
        let length = self.length();
        let steps = ((length - t0) / 0.5).ceil().max(1.0) as i32;
        (0..=steps).any(|k| {
            let p = self.along(t0 + (length - t0) * k as f32 / steps as f32);
            physics::solid_at(world, p[0].floor() as i32, (p[1] - CONTACT_SLACK).floor() as i32)
        })
    }

    /// Steps the fall; false once it has broken on the ground.
    fn update(&mut self, world: &World, dt: f32) -> bool {
        let length = self.length();
        match self.phase {
            Phase::Drop => {
                self.vel[1] = (self.vel[1] + GRAVITY * dt).min(MAX_FALL);
                self.foot[1] += self.vel[1] * dt;
                let row = self.foot[1].floor() as i32;
                if (0..self.tree.width).any(|c| physics::solid_at(world, self.tree.x + c, row)) {
                    // Snap the foot onto the top of the tile it reached.
                    self.foot[1] = row as f32;
                    self.phase = Phase::Hinged;
                    return self.topples();
                }
                self.foot[1] < world.height() as f32
            }
            Phase::Hinged => {
                // A rod pivoting on its end: angular acceleration 3g/2L * sin(angle).
                self.spin += 1.5 * GRAVITY / length * self.angle.sin() * dt;
                self.angle += self.spin * dt;
                // The first tile out from the foot is the ground it stands on.
                if self.touches(world, 1.0) {
                    return false;
                }
                if self.angle >= FRAC_PI_2 {
                    // ponytail: the hinge stays at the foot, so a trunk whose middle
                    // catches a ledge lip breaks there instead of tipping over it;
                    // move the pivot to the contact point if that case matters.
                    // Released with the velocity its centre had on the hinge.
                    let a = self.dir * self.angle;
                    let v = rotate([0.0, -length * 0.5], a + FRAC_PI_2);
                    self.vel = [v[0] * self.dir * self.spin, v[1] * self.dir * self.spin];
                    self.phase = Phase::Free;
                }
                true
            }
            Phase::Free => {
                self.vel[1] = (self.vel[1] + GRAVITY * dt).min(MAX_FALL);
                self.foot[0] += self.vel[0] * dt;
                self.foot[1] += self.vel[1] * dt;
                // Spin about the centre: move the foot by the centre's rotation.
                let before = rotate([0.0, length * 0.5], self.dir * self.angle);
                self.angle += self.spin * dt;
                let after = rotate([0.0, length * 0.5], self.dir * self.angle);
                self.foot[0] += after[0] - before[0];
                self.foot[1] += after[1] - before[1];
                // Contact is ignored in the first tile from the foot, as on the hinge:
                // that end is sliding off the lip it was pivoting on.
                !self.touches(world, 1.0) && self.foot[1] < world.height() as f32 + length
            }
        }
    }

    /// How fast the trunk's fastest point is moving, in tiles per second — the
    /// number damage will scale with.
    #[cfg(test)]
    fn speed(&self) -> f32 {
        let spin = self.spin * self.length();
        match self.phase {
            Phase::Drop => self.vel[1].abs(),
            Phase::Hinged => spin,
            Phase::Free => self.vel[0].hypot(self.vel[1]) + spin * 0.5,
        }
    }

    fn emit(&self, next_id: &mut u32, out: &mut Vec<Instance>) {
        let start = out.len();
        let cells = |i: i32, c: i32| self.cells[(i - self.from) as usize][c as usize];
        // The grid's foliage was cleared when it fell, so leaves draw from its shape.
        self.tree.draw(self.from..self.tree.height, cells, |_| 0.0, |_, own| own, next_id, out);
        // Quads come out where the part stood; move its foot to where it is, then
        // turn the part about it.
        let shift = [self.foot[0] - self.tree.centre(), self.foot[1] - (self.tree.ground - self.from) as f32];
        let a = self.dir * self.angle;
        for q in &mut out[start..] {
            let rel = rotate([q.pos[0] + shift[0] - self.foot[0], q.pos[1] + shift[1] - self.foot[1]], a);
            q.pos = [self.foot[0] + rel[0], self.foot[1] + rel[1]];
            q.axis_x = rotate(q.axis_x, a);
            q.axis_y = rotate(q.axis_y, a);
        }
    }
}

/// Rotation in screen space (y down): positive turns clockwise on screen.
fn rotate(v: [f32; 2], a: f32) -> [f32; 2] {
    let (s, c) = a.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

#[derive(Default)]
pub struct Forest {
    // ponytail: linear scans over every tree (a few hundred); bucket by chunk column
    // if worlds or tree counts grow by an order of magnitude.
    pub trees: Vec<Tree>,
    pub falling: Vec<Falling>,
    /// Seconds of simulated time, driving the wind.
    pub clock: f32,
}

impl Forest {
    /// Whether the tile at (x, y) is ground a trunk stands on. Removing it would
    /// leave the tree floating, so it is not mined while the tree stands.
    pub fn anchors(&self, x: i32, y: i32) -> bool {
        self.trees
            .iter()
            .any(|t| t.ground == y && x >= t.x && x < t.x + t.width && t.height > 0)
    }
}

/// Call after a trunk cell may have been removed at (x, y). Fells whatever part of
/// the tree is no longer held up, toppling away from `from_x` if cut at the base.
///
/// A one-wide trunk is severed by any cut. A two-wide trunk needs two cuts within
/// one 2x2 window of its rows — side by side, diagonal or stacked; everything from
/// the window's upper row up falls, and anything left in its lower row stays.
pub fn after_cut(world: &mut World, x: i32, y: i32, from_x: f32) {
    let Some(index) = world.forest.trees.iter().position(|t| t.row_of(x, y).is_some()) else {
        return;
    };
    let tree = world.forest.trees[index];
    let cut = |i: i32, c: i32| i < tree.height && !tree.present(world, i, c);
    let window_rows = if tree.width == 1 { 1 } else { 2 };
    let severed = (0..tree.height).find(|&i| {
        let cuts = (i..i + window_rows)
            .flat_map(|r| (0..tree.width).map(move |c| (r, c)))
            .filter(|&(r, c)| cut(r, c))
            .count();
        cuts >= tree.width as usize
    });
    let Some(i) = severed else { return };
    // First falling row: the row above a one-wide cut, or a two-wide window's upper row.
    let from = i + 1;

    let cells: Vec<[bool; 2]> = (from..tree.height)
        .map(|r| [tree.present(world, r, 0), tree.width == 2 && tree.present(world, r, 1)])
        .collect();
    for r in from..tree.height {
        for c in 0..tree.width {
            if tree.present(world, r, c) {
                world.set(tree.x + c, tree.row_y(r), AIR);
            }
        }
    }

    let fallen = cells.iter().any(|row| row[0] || row[1]);
    if fallen {
        let dir = if tree.centre() >= from_x { 1.0 } else { -1.0 };
        world.forest.falling.push(Falling::new(tree, from, cells, dir));
    }

    let mut stump = Tree { height: from, ..tree };
    while stump.height > 0 && !(0..stump.width).any(|c| stump.present(world, stump.height - 1, c)) {
        stump.height -= 1;
    }
    if stump.height > 0 {
        world.forest.trees[index] = stump;
    } else {
        world.forest.trees.remove(index);
    }
    clear_foliage(world, &tree);
}

/// Plants a tree: trunk cells into the grid, its foliage into the foliage layer
/// (keeping the denser value where crowns overlap), and the tree into the forest.
pub fn plant(world: &mut World, tree: Tree) {
    for i in 0..tree.height {
        for c in 0..tree.width {
            world.set(tree.x + c, tree.row_y(i), TRUNK);
        }
    }
    for ((sx, sy), (density, _)) in tree.foliage(0..tree.height) {
        let byte = (density * 255.0) as u8;
        world.set_foliage(sx, sy, world.foliage(sx, sy).max(byte));
    }
    world.forest.trees.push(tree);
}

/// After `before` (the tree as it stood) lost some of itself, lowers each of its
/// foliage cells to what the trees now standing still provide there. Lowers but
/// never raises, so a neighbour's overlapping crown survives and thinning (from
/// trampling, later) is not undone.
fn clear_foliage(world: &mut World, before: &Tree) {
    let cells = before.foliage(0..before.height);
    let mut remaining: BTreeMap<(i32, i32), f32> = BTreeMap::new();
    // Crowns reach a few tiles past their trunks; anything this far away can't overlap.
    let reach = 16;
    for t in world.forest.trees.iter().filter(|t| (t.x - before.x).abs() <= reach) {
        for (cell, (density, _)) in t.foliage(0..t.height) {
            if cells.contains_key(&cell) {
                let d = remaining.entry(cell).or_insert(0.0);
                *d = d.max(density);
            }
        }
    }
    for &(sx, sy) in cells.keys() {
        let keep = (remaining.get(&(sx, sy)).copied().unwrap_or(0.0) * 255.0) as u8;
        world.set_foliage(sx, sy, world.foliage(sx, sy).min(keep));
    }
}

/// Advances the wind and every falling piece; pieces vanish when they land.
// ponytail: landing leaves nothing behind; it drops wood once items exist.
pub fn update(world: &mut World, dt: f32) {
    world.forest.clock += dt;
    let mut falling = std::mem::take(&mut world.forest.falling);
    falling.retain_mut(|f| f.update(world, dt));
    world.forest.falling = falling;
}

/// Appends quads for every tree and falling piece near the view.
/// Appends quads for every tree and falling piece near the view, taking object ids
/// from `next_id` in draw order.
pub fn emit(world: &World, view: TileRect, next_id: &mut u32, out: &mut Vec<Instance>) {
    let margin = 6;
    let clock = world.forest.clock;
    for tree in &world.forest.trees {
        if tree.x + tree.width < view.x0 - margin || tree.x > view.x1 + margin {
            continue;
        }
        // Shape from the tree, density from the grid, so thinned foliage draws thinner.
        let density = |(sx, sy): (i32, i32), _own: f32| world.foliage(sx, sy) as f32 / 255.0;
        let cells = |i, c| tree.present(world, i, c);
        tree.draw(0..tree.height, cells, |h| tree.sway(h, clock), density, next_id, out);
    }
    for f in &world.forest.falling {
        f.emit(next_id, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_leaf(mask: u16) -> bool {
        (LEAF_MASK..LEAF_MASK + LEAF_VARIANTS).contains(&mask)
    }

    #[test]
    fn standing_leaves_sit_on_the_pixel_grid() {
        let mut world = world_with_tree(1, 14);
        let mut out = Vec::new();
        let mut id = FIRST_OBJECT_ID;
        // Swaying moves leaves, in whole pixels only.
        for clock in [0.0, 1.3, 2.9] {
            out.clear();
            world.forest.clock = clock;
            emit(&world, TileRect { x0: 0, y0: 0, x1: 64, y1: 64 }, &mut id, &mut out);
            assert!(out.iter().any(|q| is_leaf(q.mask)), "the tree has a crown");
            for q in out.iter().filter(|q| is_leaf(q.mask)) {
                assert_eq!((q.axis_x, q.axis_y), ([1.0, 0.0], [0.0, 1.0]), "leaves are never rotated or scaled");
                for v in q.pos {
                    let px = v * atlas::CELL as f32;
                    assert!((px - px.round()).abs() < 1e-3, "a leaf is off the pixel grid at {v}");
                }
            }
        }
    }
    use crate::autotile::FIRST_OBJECT_ID;
    use crate::tiles::DIRT;

    const GROUND: i32 = 40;

    /// Flat dirt from `GROUND` down, one tree of the given width and height at x=10.
    fn world_with_tree(width: i32, height: i32) -> World {
        let mut world = World::new(64, 64);
        for x in 0..64 {
            for y in GROUND..64 {
                world.set(x, y, DIRT);
            }
        }
        plant(&mut world, Tree { x: 10, ground: GROUND, width, height, full: height, seed: 7 });
        world
    }

    fn chop(world: &mut World, c: i32, i: i32) {
        let (x, y) = (10 + c, GROUND - 1 - i);
        world.set(x, y, AIR);
        after_cut(world, x, y, 0.0);
    }

    fn trunk_cells(world: &World) -> usize {
        (0..64).flat_map(|x| (0..64).map(move |y| (x, y))).filter(|&(x, y)| world.get(x, y) == TRUNK).count()
    }

    #[test]
    fn a_one_wide_trunk_falls_above_any_cut() {
        let mut world = world_with_tree(1, 10);
        chop(&mut world, 0, 4);
        assert_eq!(world.forest.trees[0].height, 4);
        assert_eq!(trunk_cells(&world), 4, "rows above the cut leave the grid");
        assert_eq!(world.forest.falling[0].cells.len(), 5);
    }

    #[test]
    fn a_felled_part_tips_away_from_the_chopper() {
        let mut world = world_with_tree(1, 10);
        chop(&mut world, 0, 0);
        assert!(world.forest.trees.is_empty());
        assert_eq!(world.forest.falling[0].dir, 1.0, "chopper at x=0 fells it rightwards");
    }

    /// Steps a felled part until it breaks; returns whether it ever started to tip.
    fn tipped_before_breaking(world: &mut World) -> bool {
        let mut tipped = false;
        for _ in 0..300 {
            update(world, 1.0 / 60.0);
            let Some(f) = world.forest.falling.first() else { return tipped };
            if f.phase == Phase::Drop {
                assert_eq!(f.angle, 0.05, "no tipping while it is still falling");
            } else {
                assert_eq!(f.foot[1], GROUND as f32, "it rests on the ground, not mid-air");
                assert_eq!(f.phase, Phase::Hinged, "on flat ground it never falls free");
                tipped |= f.angle > 0.05;
            }
        }
        panic!("never broke");
    }

    #[test]
    fn wherever_it_is_cut_a_long_part_settles_on_the_ground_then_topples() {
        for row in [0, 3, 6] {
            let mut world = world_with_tree(1, 12);
            chop(&mut world, 0, row);
            assert!(tipped_before_breaking(&mut world), "cut at row {row} did not topple");
        }
    }

    #[test]
    fn a_part_shorter_than_four_rows_breaks_on_landing_without_toppling() {
        let mut world = world_with_tree(1, 12);
        chop(&mut world, 0, 8);
        assert_eq!(world.forest.falling[0].length(), 3.0);
        assert!(!tipped_before_breaking(&mut world));
    }

    #[test]
    fn a_tree_toppled_over_a_ledge_falls_on_and_gathers_speed() {
        // Ground at GROUND up to the trunk's own column, then a 20-tile drop.
        let mut world = world_with_tree(1, 10);
        for x in 11..64 {
            for y in GROUND..GROUND + 20 {
                world.set(x, y, AIR);
            }
        }
        chop(&mut world, 0, 0);
        let (mut freed, mut lowest, mut fastest) = (false, 0.0f32, 0.0f32);
        for _ in 0..600 {
            update(&mut world, 1.0 / 60.0);
            let Some(f) = world.forest.falling.first() else { break };
            freed |= f.phase == Phase::Free;
            lowest = lowest.max(f.foot[1]);
            fastest = fastest.max(f.speed());
        }
        assert!(world.forest.falling.is_empty(), "never broke");
        assert!(freed, "it should leave the hinge once past horizontal in the air");
        assert!(lowest > (GROUND + 10) as f32, "it should fall down the drop, reached {lowest}");
        assert!(fastest > 25.0, "a 20-tile fall should build real speed, got {fastest}");
    }

    #[test]
    fn one_cut_in_a_two_wide_trunk_does_not_fell_it() {
        let mut world = world_with_tree(2, 10);
        chop(&mut world, 0, 3);
        assert!(world.forest.falling.is_empty());
        assert_eq!(world.forest.trees[0].height, 10);
    }

    #[test]
    fn two_cuts_in_one_window_fell_a_two_wide_trunk_whatever_their_arrangement() {
        for (name, cuts) in [
            ("side by side", [(0, 3), (1, 3)]),
            ("diagonal", [(0, 3), (1, 4)]),
            ("other diagonal", [(1, 3), (0, 4)]),
            ("stacked", [(0, 3), (0, 4)]),
        ] {
            let mut world = world_with_tree(2, 10);
            for (c, i) in cuts {
                chop(&mut world, c, i);
            }
            assert_eq!(world.forest.falling.len(), 1, "{name}");
            assert!(world.forest.trees[0].height <= 4, "{name}: the stump ends at the window's lower row");
            let above = (4..10).flat_map(|r| [(10, r), (11, r)]).any(|(x, r)| world.get(x, GROUND - 1 - r) == TRUNK);
            assert!(!above, "{name}: nothing above the window stays standing");
        }
    }

    #[test]
    fn cuts_too_far_apart_do_not_fell_a_two_wide_trunk() {
        let mut world = world_with_tree(2, 10);
        chop(&mut world, 0, 2);
        chop(&mut world, 1, 5);
        assert!(world.forest.falling.is_empty());
    }

    #[test]
    fn chopping_the_top_cell_shrinks_the_tree_without_anything_falling() {
        let mut world = world_with_tree(1, 10);
        chop(&mut world, 0, 9);
        assert!(world.forest.falling.is_empty());
        assert_eq!(world.forest.trees[0].height, 9);
    }

    #[test]
    fn falling_pieces_land_and_vanish() {
        for base in [0, 5] {
            let mut world = world_with_tree(1, 12);
            chop(&mut world, 0, base);
            assert_eq!(world.forest.falling.len(), 1);
            for _ in 0..300 {
                update(&mut world, 1.0 / 60.0);
            }
            assert!(world.forest.falling.is_empty(), "cut at row {base} never landed");
        }
    }

    #[test]
    fn the_ground_under_a_standing_trunk_is_anchored() {
        let world = world_with_tree(2, 8);
        assert!(world.forest.anchors(10, GROUND) && world.forest.anchors(11, GROUND));
        assert!(!world.forest.anchors(12, GROUND) && !world.forest.anchors(10, GROUND + 1));
    }

    #[test]
    fn a_tree_draws_one_quad_per_row_plus_its_branches_and_follows_cuts() {
        let mut world = world_with_tree(1, 10);
        let branches = world.forest.trees[0].branches().len();
        let view = TileRect { x0: 0, y0: 0, x1: 64, y1: 64 };
        let wood = |world: &World| {
            let mut out = Vec::new();
            emit(world, view, &mut FIRST_OBJECT_ID.clone(), &mut out);
            out.iter().filter(|q| q.mask == SOLID_MASK).count()
        };
        assert_eq!(wood(&world), 10 + branches);
        chop(&mut world, 0, 0);
        assert_eq!(wood(&world), 9 + branches, "the toppling part still draws, minus the cut cell");
    }

    fn foliage_cells(world: &World) -> usize {
        (0..128).flat_map(|sx| (0..128).map(move |sy| (sx, sy))).filter(|&(sx, sy)| world.foliage(sx, sy) > 0).count()
    }

    #[test]
    fn a_canopy_is_a_crown_above_the_trunk_plus_clumps_at_branch_tips() {
        let tree = Tree { x: 10, ground: GROUND, width: 1, height: 14, full: 14, seed: 7 };
        let top = tree.row_y(13) as f32 * SUB as f32;
        let foliage = tree.foliage(0..14);
        assert_eq!(foliage, tree.foliage(0..14), "the same tree always grows the same canopy");
        let crown = foliage.keys().filter(|(_, sy)| (*sy as f32) < top).count();
        assert!(crown > 40, "only {crown} foliage cells above the top of the trunk");
        for b in tree.branches() {
            let (a, d) = (tree.branch_attach(&b), Tree::branch_dir(&b));
            let tip = [((a[0] + d[0] * b.len) * 2.0) as i32, ((a[1] + d[1] * b.len) * 2.0) as i32];
            let near = foliage.keys().any(|&(sx, sy)| (sx - tip[0]).abs() <= 1 && (sy - tip[1]).abs() <= 1);
            assert!(near, "no foliage at the tip of the branch on row {}", b.row);
        }
        let other = Tree { seed: 8, ..tree };
        assert_ne!(foliage.keys().collect::<Vec<_>>(), other.foliage(0..14).keys().collect::<Vec<_>>());
    }

    #[test]
    fn planting_writes_foliage_and_felling_clears_it() {
        let mut world = world_with_tree(1, 14);
        assert!(foliage_cells(&world) > 40);
        chop(&mut world, 0, 0);
        assert_eq!(foliage_cells(&world), 0, "a felled tree takes its canopy with it");
    }

    #[test]
    fn a_cut_below_the_crown_keeps_the_clumps_on_the_stump() {
        let mut world = world_with_tree(1, 14);
        let tree = world.forest.trees[0];
        let low = tree.branches().iter().map(|b| b.row).min().unwrap();
        chop(&mut world, 0, low + 1);
        let kept = tree.foliage(0..low + 1);
        assert!(!kept.is_empty());
        for (sx, sy) in kept.keys() {
            assert!(world.foliage(*sx, *sy) > 0, "clump on the stump lost foliage at ({sx}, {sy})");
        }
        assert!(foliage_cells(&world) < tree.foliage(0..14).len(), "the crown went with the top");
    }

    #[test]
    fn felling_one_tree_leaves_an_overlapping_neighbours_crown_intact() {
        let mut world = world_with_tree(1, 14);
        let neighbour = Tree { x: 14, ground: GROUND, width: 1, height: 14, full: 14, seed: 9 };
        plant(&mut world, neighbour);
        let theirs = neighbour.foliage(0..14);
        let mine = world.forest.trees[0].foliage(0..14);
        assert!(theirs.keys().any(|c| mine.contains_key(c)), "the test needs overlapping crowns");
        chop(&mut world, 0, 0);
        for ((sx, sy), (density, _)) in &theirs {
            assert!(
                world.foliage(*sx, *sy) as f32 >= (density * 255.0).floor(),
                "the neighbour's crown thinned at ({sx}, {sy})"
            );
        }
    }

    #[test]
    fn thinned_foliage_draws_fewer_leaves_and_felling_does_not_restore_it() {
        let mut world = world_with_tree(1, 14);
        let view = TileRect { x0: 0, y0: 0, x1: 64, y1: 64 };
        let leaves = |world: &World| {
            let mut out = Vec::new();
            emit(world, view, &mut FIRST_OBJECT_ID.clone(), &mut out);
            out.iter().filter(|q| is_leaf(q.mask)).count()
        };
        let before = leaves(&world);
        let cells: Vec<_> = world.forest.trees[0].foliage(0..14).into_keys().take(10).collect();
        for &(sx, sy) in &cells {
            world.set_foliage(sx, sy, 0);
        }
        assert_eq!(leaves(&world), before - 10);

        // Thin cells a neighbour's crown shares with ours, then fell ours: the
        // neighbour's thinned cells must stay thin, not be refilled from its shape.
        let neighbour = Tree { x: 14, ground: GROUND, width: 1, height: 14, full: 14, seed: 9 };
        plant(&mut world, neighbour);
        let mine = world.forest.trees[0].foliage(0..14);
        let shared: Vec<_> = neighbour.foliage(0..14).into_keys().filter(|c| mine.contains_key(c)).collect();
        assert!(!shared.is_empty());
        for &(sx, sy) in &shared {
            world.set_foliage(sx, sy, 1);
        }
        chop(&mut world, 0, 0);
        assert!(shared.iter().all(|&(sx, sy)| world.foliage(sx, sy) == 1));
    }

    #[test]
    fn a_tree_draws_back_lobes_then_trunk_then_clumps_and_front_lobe_each_as_its_own_object() {
        let world = world_with_tree(1, 14);
        let tree = world.forest.trees[0];
        let blobs = tree.blobs(0..14);
        let mut out = Vec::new();
        let mut next_id = FIRST_OBJECT_ID;
        emit(&world, TileRect { x0: 0, y0: 0, x1: 64, y1: 64 }, &mut next_id, &mut out);
        assert_eq!(next_id - FIRST_OBJECT_ID, blobs.len() as u32 + 1, "one id per lobe, one for the wood");

        let wood = out.iter().find(|q| q.mask == SOLID_MASK).unwrap().id;
        assert!(out.iter().filter(|q| q.mask == SOLID_MASK).all(|q| q.id == wood), "trunk and branches are one object");
        let backs = blobs.iter().filter(|b| !b.front).count() as u32;
        assert!(backs >= 1 && wood == FIRST_OBJECT_ID + backs, "back lobes come before the trunk");
        let leaf_ids: Vec<u32> = out.iter().filter(|q| is_leaf(q.mask)).map(|q| q.id).collect();
        assert!(leaf_ids.iter().any(|&id| id < wood) && leaf_ids.iter().any(|&id| id > wood));
        assert!(out.windows(2).all(|w| w[0].id <= w[1].id), "ids grow with draw order");
    }

    #[test]
    fn a_trunk_sinks_its_roots_into_the_ground() {
        let world = world_with_tree(1, 14);
        let ground = world.forest.trees[0].ground as f32;
        let mut out = Vec::new();
        emit(&world, TileRect { x0: 0, y0: 0, x1: 64, y1: 64 }, &mut FIRST_OBJECT_ID.clone(), &mut out);
        let base = out.iter().filter(|q| q.mask == SOLID_MASK).map(|q| q.pos[1] + q.axis_y[1]).fold(0.0, f32::max);
        assert!((base - (ground + ROOT_SINK)).abs() < 1e-4, "the trunk reaches below the ground line, to {base}");
    }

    #[test]
    fn sway_is_zero_at_the_root_and_small_at_the_crown() {
        let tree = Tree { x: 0, ground: 10, width: 1, height: 20, full: 20, seed: 3 };
        for t in 0..200 {
            let clock = t as f32 * 0.37;
            assert_eq!(tree.sway(0.0, clock), 0.0);
            assert!(tree.sway(20.0, clock).abs() <= SWAY + 1e-6);
        }
    }
}
