//! Small sprites painted by code at startup into a grid of 16-pixel cells in
//! the atlas: gel cubes, the sword, item icons, hearts, digits and hotbar slots.
//! A sprite spans one or more cells; an instance names its first cell and size.
//!
//! Some are painted in greys so a tint colours them (cubes, hearts, digits,
//! slots); the rest are painted in their own colours and drawn untinted.

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
pub const HEART: Icon = icon(11, 1, 1);
/// A frame 24 pixels square in the top left of its cells.
pub const SLOT: Icon = icon(12, 2, 2);
pub const SLOT_PX: u32 = 24;
pub const DIRT_ICON: Icon = icon(14, 1, 1);
pub const STONE_ICON: Icon = icon(15, 1, 1);
/// Digits 0 to 9, each 5 x 7 pixels one pixel in from the top left of its
/// cell, inside a dark rim.
pub const DIGITS: u16 = 16;
pub const DIGIT_ADVANCE: u32 = 6;

pub fn item_icon(item: Item) -> Icon {
    match item {
        Item::Sword => SWORD_ICON,
        Item::Dirt => DIRT_ICON,
        Item::Stone => STONE_ICON,
        Item::Torch => TORCH_ICON,
        Item::Bone => BONE,
    }
}

pub fn digit(d: u32) -> Icon {
    icon(DIGITS + d as u16, 1, 1)
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
    let rimmed: [(Icon, &dyn Fn(&mut Paint)); 7] = [
        (SWORD, &sword),
        (SWORD_ICON, &sword_icon),
        (BONE, &bone),
        (TORCH_ICON, &torch_icon),
        (HEART, &heart),
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

const HEART_SHAPE: [&str; 8] = [
    ".##...##.",
    "####.####",
    "#########",
    "#########",
    ".#######.",
    "..#####..",
    "...###...",
    "....#....",
];

fn heart(p: &mut Paint) {
    for (y, row) in HEART_SHAPE.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c != b'#' {
                continue;
            }
            let (x, y) = (x as i32, y as i32);
            let g = if (x == 1 || x == 2) && y == 1 {
                255
            } else if y >= 5 || x == 8 {
                150
            } else {
                205
            };
            p.grey(3 + x, 4 + y, g, 255);
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
        icons.extend([SWORD, SWORD_ICON, BONE, TORCH_ICON, HEART, SLOT, DIRT_ICON, STONE_ICON]);
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
