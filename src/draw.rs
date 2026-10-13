//! Instances for what moves and what's on screen: gel cubes with what they
//! carry, the swung weapon or tool, a held torch, cracks in damaged tiles,
//! drops, damage numbers, and the bottom bar.
//! Plain functions from game state to quads, so the renderer only uploads.

use crate::atlas::{CELL, ICON};
use crate::autotile::{self, GLASS, Instance, NO_OUTLINE, UNLIT};
use crate::camera::{Camera, TileRect};
use crate::combat::{self, Drop, POPUP_TIME, Popup};
use crate::entity::{Body, Entities, Kind, Sprite};
use crate::icons::{self, Icon};
use crate::items::{HOTBAR, Item, Slot, Stack};
use crate::mob::{self, Contents, Look, MobState};
use crate::player::{self, Player};
use crate::tiles;
use crate::ui::{self, Rect};
use crate::world::World;

const TILE_PX: f32 = CELL as f32;

/// A sprite's width and height in tiles, drawn at one art pixel per world pixel.
fn tiles(icon: Icon) -> [f32; 2] {
    [
        icon.cells[0] as f32 * ICON as f32 / TILE_PX,
        icon.cells[1] as f32 * ICON as f32 / TILE_PX,
    ]
}

/// A sprite turned by `angle`, centred on `centre`.
fn turned(icon: Icon, centre: [f32; 2], angle: f32, tint: [u8; 4], id: u32) -> Instance {
    let [w, h] = tiles(icon);
    let (s, c) = angle.sin_cos();
    let (ax, ay) = ([c * w, s * w], [-s * h, c * h]);
    let pos = [centre[0] - (ax[0] + ay[0]) * 0.5, centre[1] - (ax[1] + ay[1]) * 0.5];
    autotile::icon(icon, pos, ax, ay, tint, 0, id)
}

/// Gel cubes, each with what it carries floating inside first so it shows
/// through the jelly, squashed about the middle of its base. Other mobs are
/// still their placeholder boxes.
pub fn mobs(entities: &Entities, next_id: &mut u32, out: &mut Vec<Instance>) {
    let mut query = entities.query::<(&Body, &Kind, &MobState, &Sprite, Option<&Contents>)>();
    for (body, kind, state, sprite, carried) in query.iter() {
        *next_id += 1;
        let kind = mob::kind(kind.0);
        let Look::Cube(size) = kind.look else {
            out.push(autotile::tinted_box(
                sprite.corner(body.pos),
                sprite.size(),
                sprite.tint,
                *next_id,
            ));
            continue;
        };
        let (icon, width_px) = icons::CUBES[size];
        let [sx, sy] = state.deform(body);
        let feet = body.pos[1] + body.half[1];
        let side = width_px as f32 / TILE_PX;
        if let Some(c) = carried {
            for spot in c.spots.iter().take(c.count as usize) {
                let centre = [body.pos[0] + spot[0] * side * sx, feet - side * sy * (0.5 - spot[1])];
                out.push(turned(icons::BONE, centre, spot[2], [255; 4], *next_id));
            }
        }
        let [w, h] = tiles(icon);
        let (w, h) = (w * sx, h * sy);
        out.push(autotile::icon(
            icon,
            [body.pos[0] - w * 0.5, feet - h],
            [w, 0.0],
            [0.0, h],
            kind.color,
            GLASS,
            *next_id,
        ));
    }
}

/// The sword in the front hand while it swings, drawn with the player's id so
/// the two stay one figure. It paints its own rim, being too thin to outline.
pub fn sword(player: &Player, id: u32) -> Option<Instance> {
    let (weapon, arc) = player.wielded()?;
    let (grip, dir) = combat::grip(player, arc);
    let [w, h] = tiles(weapon.sprite);
    // The sprite's down runs from tip to pommel, against the blade's direction,
    // and its across is rightward for an upright blade, flipped facing left.
    let mirror = if player.frame().mirrored { -1.0 } else { 1.0 };
    let ay = [-dir[0] * h, -dir[1] * h];
    let ax = [-dir[1] * w * mirror, dir[0] * w * mirror];
    let g = [weapon.grip[0] / (w * TILE_PX), weapon.grip[1] / (h * TILE_PX)];
    let pos = [
        grip[0] - ax[0] * g[0] - ay[0] * g[1],
        grip[1] - ax[1] * g[0] - ay[1] * g[1],
    ];
    Some(autotile::icon(weapon.sprite, pos, ax, ay, [255; 4], 0, id | NO_OUTLINE))
}

/// A torch upright in the free front hand, drawn like the sword.
pub fn torch(player: &Player, id: u32) -> Option<Instance> {
    let hand = combat::held_torch(player)?;
    // The handle's lower middle sits in the hand.
    let pos = [hand[0] - 7.5 / TILE_PX, hand[1] - 12.0 / TILE_PX];
    Some(autotile::icon(
        icons::TORCH_ICON,
        pos,
        [1.0, 0.0],
        [0.0, 1.0],
        [255; 4],
        0,
        id | NO_OUTLINE,
    ))
}

/// What a placed torch sits on, which decides how it's drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mount {
    /// Standing upright on the block below.
    Floor,
    /// Leaning out from a block to one side: -1 on the left, 1 on the right.
    Wall(f32),
    /// Set into the wall behind, facing out.
    Front,
}

pub fn mount(world: &World, x: i32, y: i32) -> Mount {
    let solid = |dx: i32, dy: i32| tiles::is_solid(world.get(x + dx, y + dy));
    if solid(0, 1) {
        Mount::Floor
    } else if solid(-1, 0) {
        Mount::Wall(-1.0)
    } else if solid(1, 0) {
        Mount::Wall(1.0)
    } else {
        Mount::Front
    }
}

/// How far a wall torch leans out from upright, in radians.
const TORCH_LEAN: f32 = 0.45;

/// Placed torches, glowing whatever the light.
pub fn torches(world: &World, view: TileRect, next_id: &mut u32, out: &mut Vec<Instance>) {
    for y in view.y0..=view.y1 {
        for x in view.x0..=view.x1 {
            if world.get(x, y) != tiles::TORCH {
                continue;
            }
            *next_id += 1;
            let id = *next_id | NO_OUTLINE;
            let tile = [x as f32, y as f32];
            out.push(match mount(world, x, y) {
                Mount::Floor => autotile::icon(icons::TORCH_ICON, tile, [1.0, 0.0], [0.0, 1.0], [255; 4], UNLIT, id),
                Mount::Front => autotile::icon(icons::TORCH_FRONT, tile, [1.0, 0.0], [0.0, 1.0], [255; 4], UNLIT, id),
                Mount::Wall(side) => {
                    // The stick's foot rests against the wall, the flame leaning away.
                    let (s, c) = (-side * TORCH_LEAN).sin_cos();
                    let (ax, ay) = ([c, s], [-s, c]);
                    let foot = [x as f32 + 0.5 + side * 0.42, y as f32 + 0.8];
                    let grip = [8.0 / TILE_PX, 15.0 / TILE_PX];
                    let pos = [
                        foot[0] - ax[0] * grip[0] - ay[0] * grip[1],
                        foot[1] - ax[1] * grip[0] - ay[1] * grip[1],
                    ];
                    autotile::icon(icons::TORCH_ICON, pos, ax, ay, [255; 4], UNLIT, id)
                }
            });
        }
    }
}

/// Cracks over tiles partway to breaking, deeper the closer they are.
pub fn cracks(world: &World, view: TileRect, next_id: &mut u32, out: &mut Vec<Instance>) {
    *next_id += 1;
    for ([x, y], stage) in world.wear.cracks(world) {
        if x >= view.x0 && x <= view.x1 && y >= view.y0 && y <= view.y1 {
            let pos = [x as f32, y as f32];
            out.push(autotile::icon(
                icons::crack(stage),
                pos,
                [1.0, 0.0],
                [0.0, 1.0],
                [255; 4],
                0,
                *next_id | NO_OUTLINE,
            ));
        }
    }
}

pub fn drops(entities: &Entities, next_id: &mut u32, out: &mut Vec<Instance>) {
    for (body, drop) in entities.query::<(&Body, &Drop)>().iter() {
        *next_id += 1;
        out.push(turned(
            icons::item_icon(drop.item),
            body.pos,
            0.0,
            [255; 4],
            *next_id | NO_OUTLINE,
        ));
    }
}

/// Writes `value` as digits `scale` world pixels each, centred on `centre`.
fn number(value: u32, centre: [f32; 2], scale: f32, tint: [u8; 4], flags: u32, id: u32, out: &mut Vec<Instance>) {
    let text = value.to_string();
    let px = scale / TILE_PX;
    let advance = icons::DIGIT_ADVANCE as f32 * px;
    let left = centre[0] - advance * text.len() as f32 * 0.5;
    let cell = ICON as f32 * px;
    for (i, d) in text.bytes().enumerate() {
        let pos = [left + advance * i as f32 - px, centre[1] - 4.5 * px];
        out.push(autotile::icon(
            icons::digit((d - b'0') as u32),
            pos,
            [cell, 0.0],
            [0.0, cell],
            tint,
            flags | UNLIT,
            id,
        ));
    }
}

/// Damage numbers, fading as they rise.
pub fn popups(popups: &[Popup], next_id: &mut u32, out: &mut Vec<Instance>) {
    for p in popups {
        *next_id += 1;
        let fade = (1.0 - p.age / POPUP_TIME).clamp(0.0, 1.0);
        let rgb = if p.hurt_player { [255, 90, 90] } else { [255, 255, 255] };
        let tint = [rgb[0], rgb[1], rgb[2], (255.0 * fade.sqrt()) as u8];
        number(
            p.value.max(0) as u32,
            p.pos,
            2.0,
            tint,
            GLASS,
            *next_id | NO_OUTLINE,
            out,
        );
    }
}

const SELECTED: [u8; 4] = [255, 214, 120, 255];
const UNSELECTED: [u8; 4] = [140, 160, 200, 255];
/// Slots for things worn rather than carried.
const WORN: [u8; 4] = [196, 170, 128, 255];
// Tints are linear, so a window dark on screen needs very small values.
const WINDOW: [u8; 4] = [3, 2, 6, 200];
const SMART: [u8; 4] = [255, 214, 120, 255];
const SMART_OFF: [u8; 4] = [110, 120, 150, 255];
const BAR_BACK: [u8; 4] = [30, 24, 34, 255];
const HEALTH: [u8; 4] = [214, 56, 64, 255];
const MANA: [u8; 4] = [64, 110, 232, 255];
/// Bar height and the gap above the hotbar, in interface pixels.
const BAR_H: f32 = 5.0;
const BAR_GAP: f32 = 3.0;

/// What the interface shows beyond the player: the smart cursor's pick for
/// the hovered tile and whether Shift is down, whether the inventory screen is
/// open, and where the mouse is, in screen pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hud {
    pub smart: Option<Item>,
    pub shift: bool,
    pub open: bool,
    pub pointer: [f32; 2],
}

/// Interface quads in screen pixels, fixed whatever the zoom and lit by
/// nothing, all under one id.
struct Screen<'a> {
    camera: &'a Camera,
    s: f32,
    id: u32,
}

impl Screen<'_> {
    fn sprite(&self, icon: Icon, x: f32, y: f32, tint: [u8; 4], flags: u32, out: &mut Vec<Instance>) {
        let [w, h] = tiles(icon);
        let ppt = self.camera.pixels_per_tile();
        let size = [w * TILE_PX * self.s / ppt, h * TILE_PX * self.s / ppt];
        let at = self.camera.px_to_tile([x, y]);
        out.push(autotile::icon(
            icon,
            at,
            [size[0], 0.0],
            [0.0, size[1]],
            tint,
            UNLIT | flags,
            self.id,
        ));
    }

    fn rect(&self, r: Rect, tint: [u8; 4], out: &mut Vec<Instance>) {
        let ppt = self.camera.pixels_per_tile();
        let at = self.camera.px_to_tile([r.x, r.y]);
        let flags = if tint[3] < 255 { GLASS } else { 0 };
        out.push(autotile::icon(
            icons::BAR,
            at,
            [r.w / ppt, 0.0],
            [0.0, r.h / ppt],
            tint,
            UNLIT | flags,
            self.id,
        ));
    }

    /// An item's icon with its count, its top left at `x`, `y`.
    fn stack(&self, stack: Stack, x: f32, y: f32, out: &mut Vec<Instance>) {
        let s = self.s;
        self.sprite(icons::item_icon(stack.item), x + 4.0 * s, y + 4.0 * s, [255; 4], 0, out);
        if stack.count > 1 {
            let slot = icons::SLOT_PX as f32 * s;
            let digits = stack.count.to_string().len() as f32;
            let centre = self
                .camera
                .px_to_tile([x + slot - (2.0 + digits * 3.0) * s, y + slot - 5.0 * s]);
            let px = s * TILE_PX / self.camera.pixels_per_tile();
            number(stack.count as u32, centre, px, [255; 4], 0, self.id, out);
        }
    }

    fn slot(&self, r: Rect, tint: [u8; 4], stack: Option<Stack>, out: &mut Vec<Instance>) {
        self.sprite(icons::SLOT, r.x, r.y, tint, 0, out);
        if let Some(stack) = stack {
            self.stack(stack, r.x, r.y, out);
        }
    }
}

/// The bottom bar, always shown: the hotbar centred along the bottom, health
/// over its left half and mana over its right, and to its left the smart
/// cursor's orb, showing what Shift would use on the hovered tile and lit while
/// Shift is held. With the inventory open, its windows too, and whatever the
/// mouse has picked up, following the pointer.
pub fn hud(
    player: &Player,
    hud: &Hud,
    camera: &Camera,
    viewport: (u32, u32),
    next_id: &mut u32,
    out: &mut Vec<Instance>,
) {
    *next_id += 1;
    let screen = Screen {
        camera,
        s: ui::scale(viewport.1),
        id: *next_id | NO_OUTLINE,
    };
    let s = screen.s;
    let inv = &player.inventory;

    let bar = ui::hotbar(viewport);
    for (i, &r) in bar.iter().enumerate() {
        let tint = if i == inv.selected { SELECTED } else { UNSELECTED };
        screen.slot(r, tint, inv.hotbar[i], out);
    }

    let (left, width) = (bar[0].x, bar[HOTBAR - 1].x + bar[HOTBAR - 1].w - bar[0].x);
    let bar_y = bar[0].y - (BAR_GAP + BAR_H) * s;
    let half = (width - BAR_GAP * s) * 0.5;
    let health = player.health as f32 / player::MAX_HEALTH as f32;
    // ponytail: mana shows full until there is magic to spend it.
    for (x, fill, tint) in [(left, health, HEALTH), (left + half + BAR_GAP * s, 1.0, MANA)] {
        screen.rect(
            Rect {
                x,
                y: bar_y,
                w: half,
                h: BAR_H * s,
            },
            BAR_BACK,
            out,
        );
        let inner = (half - 2.0 * s) * fill.clamp(0.0, 1.0);
        if inner > 0.0 {
            screen.rect(
                Rect {
                    x: x + s,
                    y: bar_y + s,
                    w: inner,
                    h: (BAR_H - 2.0) * s,
                },
                tint,
                out,
            );
        }
    }

    let orb_x = left - bar[0].w - 3.0 * ui::SLOT_GAP * s;
    screen.sprite(
        icons::ORB,
        orb_x,
        bar[0].y,
        if hud.shift { SMART } else { SMART_OFF },
        0,
        out,
    );
    if let Some(item) = hud.smart {
        screen.sprite(
            icons::item_icon(item),
            orb_x + 4.0 * s,
            bar[0].y + 4.0 * s,
            [255; 4],
            0,
            out,
        );
    }

    if !hud.open {
        return;
    }
    let layout = ui::layout(inv, viewport);
    for &w in &layout.windows {
        screen.rect(w, WINDOW, out);
    }
    for &(slot, r) in &layout.slots {
        let tint = match slot {
            Slot::Hotbar(_) => continue,
            Slot::Backpack(_) | Slot::Bag(..) => UNSELECTED,
            Slot::BagSlot(_) | Slot::Gear(_) | Slot::Tool(_) => WORN,
        };
        screen.slot(r, tint, inv.get(slot), out);
    }
    if let Some(c) = inv.cursor {
        let half_slot = icons::SLOT_PX as f32 * s * 0.5;
        screen.stack(c, hud.pointer[0] - half_slot, hud.pointer[1] - half_slot, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_squashed_cube_keeps_its_base_on_the_ground_and_its_bones_inside() {
        let mut entities = Entities::new();
        let seed = (0..).find(|&s| mob::contents(s).is_some_and(|c| c.count == 2)).unwrap();
        let e = mob::spawn(&mut entities, mob::BLUE_CUBE, [10.0, 20.0], seed);
        entities.get::<&mut MobState>(e).unwrap().squash = 0.3;
        entities.get::<&mut Body>(e).unwrap().grounded = true;
        let mut out = Vec::new();
        mobs(&entities, &mut 5, &mut out);
        let body = *entities.get::<&Body>(e).unwrap();
        assert_eq!(out.len(), 3, "two bones, then the cube");
        let cube = out[2];
        assert_eq!(cube.edges & GLASS, GLASS);
        assert!(
            (cube.pos[1] + cube.axis_y[1] - (body.pos[1] + body.half[1])).abs() < 1e-4,
            "base on the ground"
        );
        assert!(cube.axis_y[1] < cube.axis_x[0], "squashed wider than tall");
        for bone in &out[..2] {
            let centre = [
                bone.pos[0] + (bone.axis_x[0] + bone.axis_y[0]) * 0.5,
                bone.pos[1] + (bone.axis_x[1] + bone.axis_y[1]) * 0.5,
            ];
            assert!(
                (centre[0] - body.pos[0]).abs() < body.half[0] && (centre[1] - body.pos[1]).abs() < body.half[1],
                "a bone outside its cube"
            );
            assert_eq!(bone.id, cube.id);
        }
    }

    #[test]
    fn numbers_lay_out_their_digits_side_by_side() {
        let mut out = Vec::new();
        number(120, [0.0, 0.0], 1.0, [255; 4], 0, 1, &mut out);
        assert_eq!(
            out.iter().map(|i| i.mat).collect::<Vec<_>>(),
            vec![icons::DIGITS + 1, icons::DIGITS + 2, icons::DIGITS]
        );
        assert!(out.windows(2).all(|w| w[1].pos[0] > w[0].pos[0]));
        assert!(out.iter().all(|i| i.edges & UNLIT != 0));
    }

    #[test]
    fn the_bottom_bar_sits_centred_with_health_filling_its_share() {
        let viewport = (1280, 720);
        let mut camera = Camera::new([50.0, 50.0], viewport);
        camera.set_zoom(3);
        let mut player = Player::new([50.0, 50.0]);
        player.health = player::MAX_HEALTH / 2;
        let mut out = Vec::new();
        let state = Hud {
            smart: Some(Item::Pickaxe),
            shift: true,
            ..Hud::default()
        };
        hud(&player, &state, &camera, viewport, &mut 0, &mut out);
        let slots: Vec<_> = out.iter().filter(|i| i.mat == icons::SLOT.index).collect();
        assert_eq!(slots.len(), HOTBAR);
        let left = camera.tile_to_px(slots[0].pos)[0];
        // The slot frame fills the top left 24 pixels of its 32-pixel quad.
        let right = camera.tile_to_px(slots[HOTBAR - 1].pos)[0] + icons::SLOT_PX as f32 * ui::scale(viewport.1);
        assert!(
            (left + right - viewport.0 as f32).abs() <= 2.0,
            "centred: {left} to {right}"
        );
        let bottom = camera.tile_to_px([0.0, slots[0].pos[1] + slots[0].axis_y[1]])[1];
        assert!(bottom > viewport.1 as f32 - 20.0, "along the bottom: {bottom}");
        let fill = |tint| out.iter().find(|i| i.tint == tint).map(|i| i.axis_x[0]).unwrap();
        assert!(
            (fill(HEALTH) / fill(MANA) - 0.5).abs() < 0.01,
            "half health fills half a bar"
        );
        let orb = out.iter().find(|i| i.mat == icons::ORB.index).unwrap();
        assert_eq!(orb.tint, SMART, "lit while Shift is held");
        assert!(
            out.iter().any(|i| i.mat == icons::PICKAXE_ICON.index),
            "showing what it would use"
        );
    }

    #[test]
    fn a_torch_sits_on_the_floor_first_then_a_wall_else_faces_out() {
        use crate::tiles::{STONE, TORCH};
        let mut world = World::new(32, 32);
        world.set(5, 11, STONE);
        world.set(4, 10, STONE);
        assert_eq!(mount(&world, 5, 10), Mount::Floor, "the floor wins over a wall");
        world.set(10, 10, STONE);
        assert_eq!(mount(&world, 11, 10), Mount::Wall(-1.0));
        assert_eq!(mount(&world, 9, 10), Mount::Wall(1.0));
        assert_eq!(mount(&world, 20, 5), Mount::Front);

        world.set(11, 10, TORCH);
        let view = TileRect {
            x0: 0,
            y0: 0,
            x1: 31,
            y1: 31,
        };
        let mut out = Vec::new();
        torches(&world, view, &mut 0, &mut out);
        let t = out[0];
        assert!(
            t.axis_y[0] < 0.0,
            "leans away from the wall on its left, flame to the right"
        );
        assert!(t.edges & UNLIT != 0);
    }
}
