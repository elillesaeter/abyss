//! Small sprites painted by code at startup into a grid of 16-pixel cells in
//! the atlas: gel cubes, the sword and tools, item icons, cracks, digits and the
//! bottom bar's pieces.
//! A sprite spans one or more cells; an instance names its first cell and size.
//!
//! Some are painted in greys so a tint colours them (cubes, digits, slots, the
//! orb and bars); the rest are painted in their own colours and drawn untinted.

use crate::atlas::{DIRT_RAMP, ICON, ICON_BAND_Y, ICON_COLS, STONE_RAMP, WIDTH};
use crate::items::Item;

/// A sprite's first cell and its size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon {
    pub index: u16,
    pub cells: [u8; 2],
}

const fn icon(index: u16, w: u8, h: u8) -> Icon {
    Icon { index, cells: [w, h] }
}

/// Gel cubes, by width in art pixels: 24, 32 and 40.
pub const CUBES: [(Icon, u32); 3] = [(icon(0, 2, 2), 24), (icon(2, 2, 2), 32), (icon(4, 3, 3), 40)];
/// Upright, tip at the top; the hand holds it at `SWORD_GRIP`.
pub const SWORD: Icon = icon(7, 1, 2);
pub const SWORD_GRIP: [f32; 2] = [8.0, 27.5];
pub const SWORD_ICON: Icon = icon(8, 1, 1);
pub const BONE: Icon = icon(9, 1, 1);
pub const TORCH_ICON: Icon = icon(10, 1, 1);
/// A frame 24 pixels square in the top left of its cells.
pub const SLOT: Icon = icon(12, 2, 2);
pub const SLOT_PX: u32 = 24;
pub const DIRT_ICON: Icon = icon(14, 1, 1);
pub const STONE_ICON: Icon = icon(15, 1, 1);
/// Digits 0 to 9, each 5 x 7 pixels one pixel in from the top left of its
/// cell, inside a dark rim.
pub const DIGITS: u16 = 16;
pub const DIGIT_ADVANCE: u32 = 6;
/// Held tools, upright with the head at the top, gripped at `TOOL_GRIP`.
pub const PICKAXE: Icon = icon(26, 2, 2);
pub const AXE: Icon = icon(28, 2, 2);
pub const TOOL_GRIP: [f32; 2] = [16.0, 27.5];
pub const PICKAXE_ICON: Icon = icon(30, 1, 1);
pub const AXE_ICON: Icon = icon(31, 1, 1);
pub const WOOD_ICON: Icon = icon(32, 1, 1);
/// Cracks over a damaged tile, one cell per stage from the first.
pub const CRACKS: u16 = 33;
/// The smart cursor's orb: a ring 24 pixels across in the top left of its cells.
pub const ORB: Icon = icon(36, 2, 2);
/// A solid square, stretched and tinted into bars.
pub const BAR: Icon = icon(38, 1, 1);
/// A torch seen end on, set in the wall behind: the flame, a hint of stick.
pub const TORCH_FRONT: Icon = icon(39, 1, 1);

pub fn item_icon(item: Item) -> Icon {
    match item {
        Item::Sword => SWORD_ICON,
        Item::Pickaxe => PICKAXE_ICON,
        Item::Axe => AXE_ICON,
        Item::Wood => WOOD_ICON,
        Item::Dirt => DIRT_ICON,
        Item::Stone => STONE_ICON,
        Item::Torch => TORCH_ICON,
        Item::Bone => BONE,
    }
}

pub fn digit(d: u32) -> Icon {
    icon(DIGITS + d as u16, 1, 1)
}

/// The cracks for `stage`, from 1.
pub fn crack(stage: u32) -> Icon {
    icon(CRACKS + stage as u16 - 1, 1, 1)
}

/// Rows of the grid in use.
pub const ROWS: u32 = 3;

/// A sprite being painted, in its own pixel coordinates.
struct Paint<'a> {
    px: &'a mut [u8],
    origin: (u32, u32),
    size: (u32, u32),
}

impl Paint<'_> {
    fn on(px: &mut [u8], icon: Icon) -> Paint<'_> {
        let (col, row) = (icon.index as u32 % ICON_COLS, icon.index as u32 / ICON_COLS);
        Paint {
            px,
            origin: (col * ICON, ICON_BAND_Y + row * ICON),
            size: (icon.cells[0] as u32 * ICON, icon.cells[1] as u32 * ICON),
        }
    }

    fn set(&mut self, x: i32, y: i32, rgba: [u8; 4]) {
        if x >= 0 && y >= 0 && (x as u32) < self.size.0 && (y as u32) < self.size.1 {
            let i = (((self.origin.1 + y as u32) * WIDTH + self.origin.0 + x as u32) * 4) as usize;
            self.px[i..i + 4].copy_from_slice(&rgba);
        }
    }

    fn grey(&mut self, x: i32, y: i32, g: u8, a: u8) {
        self.set(x, y, [g, g, g, a]);
    }

    fn opaque(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && (x as u32) < self.size.0
            && (y as u32) < self.size.1
            && self.px[(((self.origin.1 + y as u32) * WIDTH + self.origin.0 + x as u32) * 4 + 3) as usize] > 0
    }

    /// A dark one-pixel rim round everything painted so far: small sprites carry
    /// their own outline, since the outline pass would swallow them whole.
    fn rim(&mut self) {
        let (w, h) = (self.size.0 as i32, self.size.1 as i32);
        let edge: Vec<(i32, i32)> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| !self.opaque(x, y))
            .filter(|&(x, y)| (-1..=1).any(|dy| (-1..=1).any(|dx| self.opaque(x + dx, y + dy))))
            .collect();
        for (x, y) in edge {
            self.set(x, y, RIM);
        }
    }
}

const RIM: [u8; 4] = [22, 18, 26, 255];

fn rgb(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

pub fn paint(px: &mut [u8]) {
    for (icon, width) in CUBES {
        cube(&mut Paint::on(px, icon), width as i32);
    }
    slot(&mut Paint::on(px, SLOT));
    orb(&mut Paint::on(px, ORB));
    let mut bar = Paint::on(px, BAR);
    for y in 0..ICON as i32 {
        for x in 0..ICON as i32 {
            bar.grey(x, y, 255, 255);
        }
    }
    for stage in 1..=crate::mining::CRACK_STAGES {
        cracks(&mut Paint::on(px, crack(stage)), stage as usize);
    }
    let rimmed: [(Icon, &dyn Fn(&mut Paint)); 12] = [
        (SWORD, &sword),
        (SWORD_ICON, &sword_icon),
        (PICKAXE, &pickaxe),
        (AXE, &axe),
        (PICKAXE_ICON, &pickaxe_icon),
        (AXE_ICON, &axe_icon),
        (WOOD_ICON, &wood_icon),
        (BONE, &bone),
        (TORCH_ICON, &torch_icon),
        (TORCH_FRONT, &torch_front),
        (DIRT_ICON, &|p| block_icon(p, &DIRT_RAMP)),
        (STONE_ICON, &|p| block_icon(p, &STONE_RAMP)),
    ];
    for (icon, painter) in rimmed {
        let mut p = Paint::on(px, icon);
        painter(&mut p);
        p.rim();
    }
    for d in 0..10 {
        let mut p = Paint::on(px, digit(d));
        digit_glyph(&mut p, d as usize);
        p.rim();
    }
}

/// A see-through jelly cube in greys, flat like the rest of the world,
/// standing on the bottom of its cells: a bright top edge where light catches
/// it, jelly darkening toward the bottom, a streak of shine, bubbles and drips.
fn cube(p: &mut Paint, w: i32) {
    let ox = (p.size.0 as i32 - w) / 2;
    let oy = p.size.1 as i32 - w;
    for y in 0..w {
        for x in 0..w {
            if (x == 0 || x == w - 1) && (y == 0 || y == w - 1) {
                continue; // softened corners
            }
            let (g, a) = match () {
                _ if y == 0 => (250, 255),
                _ if y == 1 => (226, 215),
                _ if y == w - 1 => (90, 245),
                _ if y == w - 2 => (112, 220),
                _ if x == 0 => (160, 215),
                _ if x == w - 1 => (108, 230),
                _ => {
                    let down = y as f32 / (w - 1) as f32;
                    ((196.0 - 70.0 * down) as u8, (145.0 + 45.0 * down) as u8)
                }
            };
            p.grey(ox + x, oy + y, g, a);
        }
    }
    for y in 3..3 + w / 4 {
        p.grey(ox + 2, oy + y, 248, 230);
    }
    p.grey(ox + 3, oy + 3, 248, 230);
    p.grey(ox + w - 6, oy + w / 2, 240, 210);
    p.grey(ox + w / 2 - 1, oy + w - 5, 235, 200);
    for (dx, len) in [(w / 3, 2), (w * 2 / 3, 3)] {
        for k in 1..=len {
            p.grey(ox + dx, oy + 1 + k, 236, 225);
        }
    }
}

/// How far a sprite's painted pixels reach from `grip`, in art pixels: the
/// sprite is painted once and measured, so its art decides.
pub fn reach(icon: Icon, grip: [f32; 2]) -> f32 {
    static PAINTED: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let px = PAINTED.get_or_init(|| {
        let mut px = vec![0u8; (WIDTH * crate::atlas::HEIGHT * 4) as usize];
        paint(&mut px);
        px
    });
    let (col, row) = (icon.index as u32 % ICON_COLS, icon.index as u32 / ICON_COLS);
    let origin = (col * ICON, ICON_BAND_Y + row * ICON);
    let size = (icon.cells[0] as u32 * ICON, icon.cells[1] as u32 * ICON);
    farthest(size, grip, |x, y| {
        px[(((origin.1 + y) * WIDTH + origin.0 + x) * 4 + 3) as usize] > 0
    })
}

/// The greatest distance from `grip` to the far edge of an opaque pixel.
fn farthest(size: (u32, u32), grip: [f32; 2], opaque: impl Fn(u32, u32) -> bool) -> f32 {
    let mut best: f32 = 0.0;
    for y in 0..size.1 {
        for x in 0..size.0 {
            if opaque(x, y) {
                let far = |c: u32, g: f32| {
                    if (c as f32 + 0.5) < g {
                        c as f32 - g
                    } else {
                        c as f32 + 1.0 - g
                    }
                };
                best = best.max(far(x, grip[0]).hypot(far(y, grip[1])));
            }
        }
    }
    best
}

const STEEL: [[u8; 3]; 3] = [[150, 160, 178], [205, 214, 226], [240, 244, 248]];
const BRASS: [[u8; 3]; 2] = [[140, 104, 48], [196, 156, 74]];
const GRIP: [u8; 3] = [104, 66, 44];

fn sword(p: &mut Paint) {
    // Blade: lit left edge, bright fuller, shaded right edge, a point on top.
    p.set(7, 2, rgb(STEEL[2]));
    p.set(8, 2, rgb(STEEL[1]));
    for y in 3..23 {
        p.set(6, y, rgb(STEEL[1]));
        p.set(7, y, rgb(STEEL[2]));
        p.set(8, y, rgb(STEEL[1]));
        p.set(9, y, rgb(STEEL[0]));
    }
    for x in 3..13 {
        p.set(x, 23, rgb(BRASS[1]));
        p.set(x, 24, rgb(BRASS[0]));
    }
    for y in 25..30 {
        p.set(7, y, rgb(GRIP));
        p.set(8, y, rgb(GRIP));
    }
    for x in 6..10 {
        p.set(x, 30, rgb(BRASS[1]));
    }
}

fn sword_icon(p: &mut Paint) {
    // The same sword laid diagonally, tip at the top right.
    for i in 0..9 {
        p.set(5 + i, 10 - i, rgb(STEEL[2]));
        p.set(6 + i, 10 - i, rgb(STEEL[1]));
        p.set(5 + i, 11 - i, rgb(STEEL[0]));
    }
    for i in 0..5 {
        p.set(2 + i, 8 + i, rgb(BRASS[1]));
    }
    p.set(3, 12, rgb(GRIP));
    p.set(2, 13, rgb(GRIP));
    p.set(1, 14, rgb(BRASS[0]));
}

const BONE_RAMP: [[u8; 3]; 3] = [[176, 164, 140], [222, 212, 188], [246, 240, 222]];

fn bone(p: &mut Paint) {
    for i in 0..8 {
        p.set(4 + i, 11 - i, rgb(BONE_RAMP[2]));
        p.set(4 + i, 12 - i, rgb(BONE_RAMP[0]));
    }
    // Knobbed ends.
    for (x, y) in [(2, 11), (3, 13), (11, 2), (13, 3)] {
        p.set(x, y, rgb(BONE_RAMP[1]));
        p.set(x + 1, y, rgb(BONE_RAMP[2]));
        p.set(x, y + 1, rgb(BONE_RAMP[0]));
        p.set(x + 1, y + 1, rgb(BONE_RAMP[1]));
    }
}

fn torch_icon(p: &mut Paint) {
    for y in 7..15 {
        p.set(7, y, rgb([132, 92, 58]));
        p.set(8, y, rgb([96, 64, 42]));
    }
    for (x, y, c) in [
        (7, 2, [255, 236, 150]),
        (6, 3, [255, 190, 70]),
        (7, 3, [255, 236, 150]),
        (8, 3, [255, 190, 70]),
        (6, 4, [240, 130, 40]),
        (7, 4, [255, 210, 100]),
        (8, 4, [240, 130, 40]),
        (6, 5, [220, 100, 30]),
        (7, 5, [255, 190, 70]),
        (8, 5, [220, 100, 30]),
        (7, 6, [200, 90, 30]),
        (8, 6, [200, 90, 30]),
    ] {
        p.set(x, y, rgb(c));
    }
}

const WOOD: [[u8; 3]; 3] = [[92, 60, 38], [124, 84, 52], [156, 112, 70]];

/// A two-pixel wooden handle down the middle, from `top` to the bottom.
fn handle(p: &mut Paint, top: i32) {
    for y in top..31 {
        p.set(15, y, rgb(WOOD[2]));
        p.set(16, y, rgb(WOOD[1]));
    }
    p.set(15, 31, rgb(WOOD[0]));
    p.set(16, 31, rgb(WOOD[0]));
}

fn pickaxe(p: &mut Paint) {
    handle(p, 6);
    // A head curving down to two points, thickest over the handle.
    for x in 3..29 {
        let off = x as f32 - 15.5;
        let y = (4.0 + off * off / 28.0).round() as i32;
        let thick = if off.abs() < 6.0 {
            3
        } else if off.abs() < 11.0 {
            2
        } else {
            1
        };
        for k in 0..thick {
            p.set(x, y + k, rgb(STEEL[2 - k as usize]));
        }
    }
    for y in 3..9 {
        p.set(14, y, rgb(STEEL[0]));
        p.set(17, y, rgb(STEEL[0]));
    }
}

fn axe(p: &mut Paint) {
    handle(p, 3);
    // A head on the leading side, flaring to a bright edge; a short poll behind.
    for y in 5..12 {
        for x in 17..24 {
            let shade = if y == 5 {
                2
            } else if y == 11 {
                0
            } else {
                1
            };
            p.set(x, y, rgb(STEEL[shade]));
        }
    }
    for x in 24..27 {
        let flare = x - 23;
        for y in 5 - flare..12 + flare {
            p.set(x, y, rgb(if x == 26 { STEEL[2] } else { STEEL[1] }));
        }
    }
    for y in 6..11 {
        for x in 12..15 {
            p.set(x, y, rgb(STEEL[0]));
        }
    }
}

/// A handle laid diagonally, bottom left to top right, ending at (`end`, 16 - `end`).
fn diagonal_handle(p: &mut Paint, end: i32) {
    for x in 2..end {
        p.set(x, 15 - x, rgb(WOOD[2]));
        p.set(x + 1, 15 - x, rgb(WOOD[0]));
    }
}

fn pickaxe_icon(p: &mut Paint) {
    diagonal_handle(p, 11);
    // The head crosses the handle's end at right angles, its points bent down.
    for i in 0..9 {
        p.set(6 + i, 1 + i, rgb(STEEL[2]));
        if (2..7).contains(&i) {
            p.set(6 + i, 2 + i, rgb(STEEL[0]));
        }
    }
    p.set(5, 2, rgb(STEEL[1]));
    p.set(14, 10, rgb(STEEL[1]));
}

fn axe_icon(p: &mut Paint) {
    diagonal_handle(p, 12);
    for y in 1..8 {
        for x in 9..15 {
            let d = x - 9 + (7 - y);
            if (3..9).contains(&d) {
                p.set(x, y, rgb(if d >= 7 { STEEL[2] } else { STEEL[1] }));
            }
        }
    }
}

fn wood_icon(p: &mut Paint) {
    // A log lying across, its cut end showing rings on the left.
    for y in 5..11 {
        for x in 3..14 {
            let shade = if y == 5 {
                2
            } else if y == 10 {
                0
            } else {
                1
            };
            p.set(x, y, rgb(WOOD[shade]));
        }
    }
    for y in 4..12 {
        for x in 1..5 {
            let (dx, dy) = (x as f32 - 2.5, y as f32 - 7.5);
            let r = (dx * dx / 2.25 + dy * dy / 12.0).sqrt();
            if r <= 1.05 {
                let ring = (r * 3.0) as i32 % 2 == 0;
                p.set(x, y, rgb(if ring { [214, 176, 122] } else { [176, 134, 88] }));
            }
        }
    }
}

fn torch_front(p: &mut Paint) {
    // The stick's end, foreshortened to a stub under the flame.
    for (x, y) in [(7, 10), (8, 10), (7, 11), (8, 11)] {
        p.set(x, y, rgb(if x == 7 { [132, 92, 58] } else { [96, 64, 42] }));
    }
    for y in 3..10 {
        for x in 4..12 {
            let (dx, dy) = (x as f32 - 7.5, (y as f32 - 7.0) * if y < 7 { 0.7 } else { 1.2 });
            let d = dx.hypot(dy);
            let c = match () {
                _ if d < 1.2 => [255, 244, 190],
                _ if d < 2.2 => [255, 200, 90],
                _ if d < 3.2 => [235, 120, 40],
                _ => continue,
            };
            p.set(x, y, rgb(c));
        }
    }
}

/// Crack lines from the middle of a tile, more of them each stage.
const CRACK_LINES: [&[([i32; 2], [i32; 2])]; 3] = [
    &[([8, 8], [4, 5]), ([8, 8], [11, 10])],
    &[([8, 8], [9, 3]), ([8, 8], [5, 12]), ([4, 5], [1, 6])],
    &[
        ([8, 8], [13, 6]),
        ([11, 10], [14, 14]),
        ([9, 3], [11, 0]),
        ([5, 12], [3, 15]),
    ],
];

fn cracks(p: &mut Paint, stage: usize) {
    for &(a, b) in CRACK_LINES[..stage].iter().flat_map(|lines| lines.iter()) {
        let n = (b[0] - a[0]).abs().max((b[1] - a[1]).abs());
        for k in 0..=n {
            let t = k as f32 / n as f32;
            let x = a[0] as f32 + (b[0] - a[0]) as f32 * t;
            let y = a[1] as f32 + (b[1] - a[1]) as f32 * t;
            p.set(x.round() as i32, y.round() as i32, [24, 18, 20, 255]);
        }
    }
}

/// A round frame like a slot's, lit from the top left.
fn orb(p: &mut Paint) {
    for y in 0..24 {
        for x in 0..24 {
            let (dx, dy) = (x as f32 - 11.5, y as f32 - 11.5);
            let d = dx.hypot(dy);
            if d > 11.8 {
                continue;
            }
            if d > 10.4 {
                p.grey(x, y, if dx + dy < 0.0 { 235 } else { 160 }, 255);
            } else {
                p.grey(x, y, 150, 150);
            }
        }
    }
}

fn slot(p: &mut Paint) {
    let n = SLOT_PX as i32;
    for y in 0..n {
        for x in 0..n {
            let edge = x == 0 || y == 0 || x == n - 1 || y == n - 1;
            let corner = (x == 0 || x == n - 1) && (y == 0 || y == n - 1);
            if corner {
                continue;
            }
            if edge {
                p.grey(x, y, if x == 0 || y == 0 { 235 } else { 160 }, 255);
            } else {
                p.grey(x, y, 150, 150);
            }
        }
    }
}

fn block_icon(p: &mut Paint, ramp: &[[u8; 3]; 5]) {
    for y in 2..14 {
        for x in 2..14 {
            // A few speckles so it reads as the material, lit from the top left.
            let speck = (x * 7 + y * 13) % 11 == 0;
            let shade = match () {
                _ if x == 13 || y == 13 => 0,
                _ if x == 2 || y == 2 => 4,
                _ if speck => 1,
                _ => 2 + ((x + y) % 5 == 0) as usize,
            };
            p.set(x, y, rgb(ramp[shade]));
        }
    }
}

/// 5 x 7 digits, bold enough to read when small.
const FONT: [[&str; 7]; 10] = [
    [".###.", "##.##", "##.##", "##.##", "##.##", "##.##", ".###."],
    ["..##.", ".###.", "..##.", "..##.", "..##.", "..##.", ".####"],
    [".###.", "##.##", "...##", "..##.", ".##..", "##...", "#####"],
    [".###.", "##.##", "...##", "..##.", "...##", "##.##", ".###."],
    ["...##", "..###", ".#.##", "#..##", "#####", "...##", "...##"],
    ["#####", "##...", "####.", "...##", "...##", "##.##", ".###."],
    [".###.", "##...", "####.", "##.##", "##.##", "##.##", ".###."],
    ["#####", "...##", "..##.", "..##.", ".##..", ".##..", ".##.."],
    [".###.", "##.##", "##.##", ".###.", "##.##", "##.##", ".###."],
    [".###.", "##.##", "##.##", ".####", "...##", "...##", ".###."],
];

fn digit_glyph(p: &mut Paint, d: usize) {
    for (y, row) in FONT[d].iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c == b'#' {
                p.grey(x as i32 + 1, y as i32 + 1, 255, 255);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reach_is_how_far_the_painting_extends_from_the_grip() {
        let blade = |len: u32| move |x: u32, y: u32| x == 8 && y >= 30 - len && y < 30;
        let short = farthest((16, 32), [8.5, 30.0], blade(10));
        let long = farthest((16, 32), [8.5, 30.0], blade(20));
        assert!(
            (short - 10.0).abs() < 0.1 && (long - 20.0).abs() < 0.1,
            "{short} {long}"
        );
        let sword = reach(SWORD, SWORD_GRIP);
        assert!((24.0..30.0).contains(&sword), "the sword reaches {sword} px");
    }

    #[test]
    fn every_sprite_is_painted_inside_its_own_cells() {
        let mut px = vec![0u8; (WIDTH * crate::atlas::HEIGHT * 4) as usize];
        paint(&mut px);
        let mut icons: Vec<Icon> = CUBES.iter().map(|c| c.0).collect();
        icons.extend([
            SWORD,
            SWORD_ICON,
            PICKAXE,
            AXE,
            PICKAXE_ICON,
            AXE_ICON,
            WOOD_ICON,
            BONE,
            TORCH_ICON,
            SLOT,
            DIRT_ICON,
            STONE_ICON,
            ORB,
            BAR,
            TORCH_FRONT,
        ]);
        icons.extend((1..=crate::mining::CRACK_STAGES).map(crack));
        icons.extend((0..10).map(digit));
        let mut owner = std::collections::HashMap::new();
        for icon in &icons {
            let mut painted = 0;
            for cy in 0..icon.cells[1] as u16 {
                for cx in 0..icon.cells[0] as u16 {
                    let cell = icon.index + cy * ICON_COLS as u16 + cx;
                    assert!(
                        owner.insert(cell, icon.index).is_none(),
                        "{icon:?} overlaps another sprite"
                    );
                }
            }
            let p = Paint::on(&mut px, *icon);
            let (origin, size) = (p.origin, p.size);
            for y in 0..size.1 {
                for x in 0..size.0 {
                    painted += (px[(((origin.1 + y) * WIDTH + origin.0 + x) * 4 + 3) as usize] > 0) as u32;
                }
            }
            assert!(painted > 0, "{icon:?} is blank");
            assert!(
                icon.index as u32 / ICON_COLS + icon.cells[1] as u32 <= ROWS,
                "{icon:?} runs off the band"
            );
        }
    }

    #[test]
    fn a_cube_stands_on_the_bottom_of_its_cells_and_is_see_through() {
        let mut px = vec![0u8; (WIDTH * crate::atlas::HEIGHT * 4) as usize];
        paint(&mut px);
        for (icon, w) in CUBES {
            let p = Paint::on(&mut px, icon);
            let (origin, size) = (p.origin, p.size);
            let alpha = |x: u32, y: u32| px[(((origin.1 + y) * WIDTH + origin.0 + x) * 4 + 3) as usize];
            let left = (size.0 - w) / 2;
            assert!(
                alpha(left + 1, size.1 - 1) > 0,
                "a {w} px cube should touch its cells' bottom"
            );
            let middle = alpha(left + w / 3, size.1 - w / 2);
            assert!(
                middle > 100 && middle < 230,
                "the jelly's middle should be partly see-through: {middle}"
            );
        }
    }
}
