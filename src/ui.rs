//! Where the bottom bar and the inventory screen's windows and slots sit, in
//! screen pixels. One layout both draws them and tells which slot the mouse
//! is over, so the two can't disagree.

use crate::icons;
use crate::items::{BACKPACK, BAG_SLOTS, GEAR, HOTBAR, Inventory, Slot, TOOL_SLOTS};

pub const MARGIN: f32 = 10.0;
pub const SLOT_GAP: f32 = 2.0;
/// Space inside a window around its slots, and between stacked windows.
const PAD: f32 = 6.0;

/// Screen pixels per interface art pixel: chunky enough to read on a large
/// screen without crowding a small one.
pub fn scale(viewport_height: u32) -> f32 {
    (viewport_height / 540).max(1) as f32
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] < self.x + self.w && p[1] >= self.y && p[1] < self.y + self.h
    }
}

/// The hotbar's slots, centred along the bottom of the screen.
pub fn hotbar(viewport: (u32, u32)) -> [Rect; HOTBAR] {
    let s = scale(viewport.1);
    let slot = icons::SLOT_PX as f32 * s;
    let step = slot + SLOT_GAP * s;
    let width = HOTBAR as f32 * step - SLOT_GAP * s;
    let left = ((viewport.0 as f32 - width) * 0.5).round();
    let top = viewport.1 as f32 - MARGIN - slot;
    std::array::from_fn(|i| Rect {
        x: left + i as f32 * step,
        y: top,
        w: slot,
        h: slot,
    })
}

/// A window of `cols` by `rows` slots, and the slot at a column and row in it.
struct Grid {
    window: Rect,
    s: f32,
}

impl Grid {
    fn size(cols: usize, rows: usize, s: f32) -> [f32; 2] {
        let slot = icons::SLOT_PX as f32 * s;
        let span = |n: usize| 2.0 * PAD * s + n as f32 * slot + (n as f32 - 1.0) * SLOT_GAP * s;
        [span(cols), span(rows)]
    }

    fn at(x: f32, y: f32, cols: usize, rows: usize, s: f32) -> Grid {
        let [w, h] = Grid::size(cols, rows, s);
        Grid {
            window: Rect { x, y, w, h },
            s,
        }
    }

    fn cell(&self, col: usize, row: usize) -> Rect {
        let slot = icons::SLOT_PX as f32 * self.s;
        let step = slot + SLOT_GAP * self.s;
        let inset = PAD * self.s;
        Rect {
            x: self.window.x + inset + col as f32 * step,
            y: self.window.y + inset + row as f32 * step,
            w: slot,
            h: slot,
        }
    }
}

/// Gear in a cross like RuneScape's, by column and row: head on top, back and
/// neck, hands and chest, legs, feet; the accessories down the right.
const GEAR_CELLS: [(usize, usize); GEAR] = [
    (1, 0),
    (0, 1),
    (1, 1),
    (0, 2),
    (1, 2),
    (1, 3),
    (1, 4),
    (3, 0),
    (3, 1),
    (3, 2),
    (3, 3),
    (3, 4),
];
/// The tool belt along the bottom of the character pane.
const TOOL_ROW: usize = 5;

/// What the open inventory screen shows: its windows, and every slot with
/// where it is.
pub struct Layout {
    pub windows: Vec<Rect>,
    pub slots: Vec<(Slot, Rect)>,
}

impl Layout {
    pub fn slot_at(&self, p: [f32; 2]) -> Option<Slot> {
        self.slots.iter().find(|(_, r)| r.contains(p)).map(|&(slot, _)| slot)
    }

    /// Whether the mouse is over the screen at all, rather than the world.
    pub fn covers(&self, p: [f32; 2]) -> bool {
        self.windows.iter().any(|w| w.contains(p)) || self.slot_at(p).is_some()
    }
}

/// The open inventory screen: the character pane against the left edge, the
/// backpack with its bag slots in the bottom right and a window per worn bag
/// stacked above it, and the hotbar where it always is.
pub fn layout(inv: &Inventory, viewport: (u32, u32)) -> Layout {
    let s = scale(viewport.1);
    let (vw, vh) = (viewport.0 as f32, viewport.1 as f32);
    let mut windows = Vec::new();
    let mut slots: Vec<(Slot, Rect)> = hotbar(viewport)
        .iter()
        .enumerate()
        .map(|(i, &r)| (Slot::Hotbar(i), r))
        .collect();

    let [_, h] = Grid::size(4, TOOL_ROW + 1, s);
    let pane = Grid::at(MARGIN, ((vh - h) * 0.5).round(), 4, TOOL_ROW + 1, s);
    slots.extend(
        GEAR_CELLS
            .iter()
            .enumerate()
            .map(|(i, &(c, r))| (Slot::Gear(i), pane.cell(c, r))),
    );
    slots.extend((0..TOOL_SLOTS).map(|i| (Slot::Tool(i), pane.cell(i, TOOL_ROW))));
    windows.push(pane.window);

    let [w, h] = Grid::size(5, 3, s);
    let pack = Grid::at(vw - MARGIN - w, vh - MARGIN - h, 5, 3, s);
    slots.extend((0..BACKPACK).map(|i| (Slot::Backpack(i), pack.cell(i % 5, i / 5))));
    slots.extend((0..BAG_SLOTS).map(|b| (Slot::BagSlot(b), pack.cell(b, 2))));
    windows.push(pack.window);

    let mut top = pack.window.y;
    for (b, bag) in inv.bags.iter().enumerate() {
        let Some(bag) = bag else { continue };
        let rows = bag.slots.len().div_ceil(4).max(1);
        let [w, h] = Grid::size(4, rows, s);
        top -= PAD * s + h;
        let grid = Grid::at(vw - MARGIN - w, top, 4, rows, s);
        slots.extend((0..bag.slots.len()).map(|i| (Slot::Bag(b, i), grid.cell(i % 4, i / 4))));
        windows.push(grid.window);
    }
    Layout { windows, slots }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{Item, Stack};

    fn full_of_bags() -> Inventory {
        let mut inv = Inventory::starter();
        for b in 1..BAG_SLOTS {
            inv.cursor = Some(Stack {
                item: Item::Pouch,
                count: 1,
            });
            inv.click(Slot::BagSlot(b));
        }
        inv
    }

    #[test]
    fn every_slot_is_on_screen_apart_from_the_others_and_found_by_the_mouse() {
        let inv = full_of_bags();
        for viewport in [(1280, 720), (1920, 1080), (2560, 1440)] {
            let layout = layout(&inv, viewport);
            assert_eq!(
                layout.slots.len(),
                HOTBAR + GEAR + TOOL_SLOTS + BACKPACK + BAG_SLOTS + 4 * 8
            );
            for (i, &(slot, r)) in layout.slots.iter().enumerate() {
                assert!(
                    r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= viewport.0 as f32 && r.y + r.h <= viewport.1 as f32,
                    "{slot:?} off a {viewport:?} screen: {r:?}"
                );
                for &(other, o) in &layout.slots[i + 1..] {
                    let apart = r.x + r.w <= o.x || o.x + o.w <= r.x || r.y + r.h <= o.y || o.y + o.h <= r.y;
                    assert!(apart, "{slot:?} overlaps {other:?} on {viewport:?}");
                }
                assert_eq!(layout.slot_at([r.x + r.w * 0.5, r.y + r.h * 0.5]), Some(slot));
            }
            let middle = [viewport.0 as f32 * 0.5, viewport.1 as f32 * 0.4];
            assert!(!layout.covers(middle), "the middle of the screen stays on the world");
        }
    }

    #[test]
    fn the_hotbar_is_centred_along_the_bottom() {
        let viewport = (1280, 720);
        let bar = hotbar(viewport);
        let (left, right) = (bar[0].x, bar[HOTBAR - 1].x + bar[HOTBAR - 1].w);
        assert!((left + right - viewport.0 as f32).abs() <= 1.0, "{left} to {right}");
        assert_eq!(bar[0].y + bar[0].h, viewport.1 as f32 - MARGIN);
    }
}
