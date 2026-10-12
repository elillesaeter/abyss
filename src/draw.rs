//! Instances for what moves and what's on screen: gel cubes with what they
//! carry, the swung sword, drops, damage numbers, and the hotbar and hearts.
//! Plain functions from game state to quads, so the renderer only uploads.

use crate::atlas::{CELL, ICON};
use crate::autotile::{self, GLASS, Instance, NO_OUTLINE, UNLIT};
use crate::camera::Camera;
use crate::combat::{self, Drop, POPUP_TIME, Popup};
use crate::entity::{Body, Entities, Kind, Sprite};
use crate::icons::{self, Icon};
use crate::items::HOTBAR;
use crate::mob::{self, Contents, Look, MobState};
use crate::player::{self, Player};

const TILE_PX: f32 = CELL as f32;

/// A sprite's width and height in tiles, drawn at one art pixel per world pixel.
fn tiles(icon: Icon) -> [f32; 2] {
    [icon.cells[0] as f32 * ICON as f32 / TILE_PX, icon.cells[1] as f32 * ICON as f32 / TILE_PX]
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
            out.push(autotile::tinted_box(sprite.corner(body.pos), sprite.size(), sprite.tint, *next_id));
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
        out.push(autotile::icon(icon, [body.pos[0] - w * 0.5, feet - h], [w, 0.0], [0.0, h], kind.color, GLASS, *next_id));
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
    let pos = [grip[0] - ax[0] * g[0] - ay[0] * g[1], grip[1] - ax[1] * g[0] - ay[1] * g[1]];
    Some(autotile::icon(weapon.sprite, pos, ax, ay, [255; 4], 0, id | NO_OUTLINE))
}

pub fn drops(entities: &Entities, next_id: &mut u32, out: &mut Vec<Instance>) {
    for (body, drop) in entities.query::<(&Body, &Drop)>().iter() {
        *next_id += 1;
        out.push(turned(icons::item_icon(drop.item), body.pos, 0.0, [255; 4], *next_id | NO_OUTLINE));
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
        out.push(autotile::icon(icons::digit((d - b'0') as u32), pos, [cell, 0.0], [0.0, cell], tint, flags | UNLIT, id));
    }
}

/// Damage numbers, fading as they rise.
pub fn popups(popups: &[Popup], next_id: &mut u32, out: &mut Vec<Instance>) {
    for p in popups {
        *next_id += 1;
        let fade = (1.0 - p.age / POPUP_TIME).clamp(0.0, 1.0);
        let rgb = if p.hurt_player { [255, 90, 90] } else { [255, 255, 255] };
        let tint = [rgb[0], rgb[1], rgb[2], (255.0 * fade.sqrt()) as u8];
        number(p.value.max(0) as u32, p.pos, 2.0, tint, GLASS, *next_id | NO_OUTLINE, out);
    }
}

/// Screen pixels per interface art pixel: chunky enough to read on a large
/// screen without crowding a small one.
pub fn hud_scale(viewport_height: u32) -> f32 {
    (viewport_height / 540).max(1) as f32
}

const MARGIN: f32 = 10.0;
const SLOT_GAP: f32 = 2.0;
const SELECTED: [u8; 4] = [255, 214, 120, 255];
const UNSELECTED: [u8; 4] = [140, 160, 200, 255];
const HEART_FULL: [u8; 3] = [232, 52, 64];
const HEART_EMPTY: [u8; 3] = [60, 42, 48];

/// The hotbar along the top left and the hearts at the top right, fixed in
/// screen pixels whatever the zoom, and lit by nothing.
pub fn hud(player: &Player, camera: &Camera, viewport: (u32, u32), next_id: &mut u32, out: &mut Vec<Instance>) {
    let s = hud_scale(viewport.1);
    let px = s / camera.pixels_per_tile();
    let at = |x: f32, y: f32| camera.px_to_tile([x, y]);
    let quad = |icon: Icon| {
        let [w, h] = tiles(icon);
        ([w * TILE_PX * px, 0.0], [0.0, h * TILE_PX * px])
    };

    *next_id += 1;
    let id = *next_id | NO_OUTLINE;
    let slot_step = (icons::SLOT_PX as f32 + SLOT_GAP) * s;
    for (i, slot) in player.inventory.slots.iter().enumerate().take(HOTBAR) {
        let (x, y) = (MARGIN + i as f32 * slot_step, MARGIN);
        let tint = if i == player.inventory.selected { SELECTED } else { UNSELECTED };
        let (ax, ay) = quad(icons::SLOT);
        out.push(autotile::icon(icons::SLOT, at(x, y), ax, ay, tint, UNLIT, id));
        let Some(stack) = slot else { continue };
        let icon = icons::item_icon(stack.item);
        let (ax, ay) = quad(icon);
        out.push(autotile::icon(icon, at(x + 4.0 * s, y + 4.0 * s), ax, ay, [255; 4], UNLIT, id));
        if stack.count > 1 {
            let digits = stack.count.to_string().len() as f32;
            let centre = at(x + (icons::SLOT_PX as f32 - 2.0 - digits * 3.0) * s, y + (icons::SLOT_PX as f32 - 5.0) * s);
            number(stack.count as u32, centre, s * camera.pixels_per_tile().recip() * TILE_PX, [255; 4], 0, id, out);
        }
    }

    let hearts = player::MAX_HEALTH / player::HEART;
    let heart_step = 12.0 * s;
    let right = viewport.0 as f32 - MARGIN - hearts as f32 * heart_step - 4.0 * s;
    for k in 0..hearts {
        // How full this heart is, from empty to whole.
        let fill = ((player.health - k * player::HEART) as f32 / player::HEART as f32).clamp(0.0, 1.0);
        let mix = |e: u8, f: u8| (e as f32 + (f as f32 - e as f32) * fill) as u8;
        let tint = [mix(HEART_EMPTY[0], HEART_FULL[0]), mix(HEART_EMPTY[1], HEART_FULL[1]), mix(HEART_EMPTY[2], HEART_FULL[2]), 255];
        let (ax, ay) = quad(icons::HEART);
        out.push(autotile::icon(icons::HEART, at(right + k as f32 * heart_step, MARGIN), ax, ay, tint, UNLIT, id));
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
        assert!((cube.pos[1] + cube.axis_y[1] - (body.pos[1] + body.half[1])).abs() < 1e-4, "base on the ground");
        assert!(cube.axis_y[1] < cube.axis_x[0], "squashed wider than tall");
        for bone in &out[..2] {
            let centre = [bone.pos[0] + (bone.axis_x[0] + bone.axis_y[0]) * 0.5, bone.pos[1] + (bone.axis_x[1] + bone.axis_y[1]) * 0.5];
            assert!((centre[0] - body.pos[0]).abs() < body.half[0] && (centre[1] - body.pos[1]).abs() < body.half[1], "a bone outside its cube");
            assert_eq!(bone.id, cube.id);
        }
    }

    #[test]
    fn numbers_lay_out_their_digits_side_by_side() {
        let mut out = Vec::new();
        number(120, [0.0, 0.0], 1.0, [255; 4], 0, 1, &mut out);
        assert_eq!(out.iter().map(|i| i.mat).collect::<Vec<_>>(), vec![icons::DIGITS + 1, icons::DIGITS + 2, icons::DIGITS]);
        assert!(out.windows(2).all(|w| w[1].pos[0] > w[0].pos[0]));
        assert!(out.iter().all(|i| i.edges & UNLIT != 0));
    }
}
