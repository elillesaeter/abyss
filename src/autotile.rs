//! Turns tiles into draw instances: one opaque base per tile, plus one overlay
//! per material that bleeds into it from a higher precedence.
//!
//! An overlay samples the bleeding material's texture through a mask chosen by
//! the 8-neighbour bitmask of *that* material, so a single quad covers edges,
//! outer corners and diagonal-only nubs together. Emitting separate per-edge and
//! per-corner quads would double-blend where they overlap and leave dark seams.
//!
//! Decoration (grass today) uses the same mechanism against the tile's air-facing
//! sides, so grass is a skin on dirt rather than a block of its own.

use crate::atlas::{self, DIRS, SOLID_MASK};
use crate::tiles::{AIR, DECOR_TYPES, TileId, atlas_mat, drawn_as, is_solid, precedence};
use crate::atlas::{MAT_AIR_SHADOW, MAT_SKY};
use crate::world::World;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    /// Corner the quad's edges start from, in tile units (top-left when unrotated).
    pub pos: [f32; 2],
    /// The quad's two edges as vectors: `[w, 0]` and `[0, h]` for a plain box.
    /// Edges rather than a size so a quad can lean (sway) or rotate (a falling tree).
    pub axis_x: [f32; 2],
    pub axis_y: [f32; 2],
    /// Mask cell index; `SOLID_MASK` for fully opaque quads.
    pub mask: u16,
    /// Material column in the low byte, and which cell of its block to sample in
    /// the high byte (see `textured`).
    pub mat: u16,
    pub tint: [u8; 4],
    /// Which object this quad belongs to, for outlining. Grows with draw order, so
    /// a smaller id is behind: an outline is drawn where something meets what is
    /// behind it, and never inside one object.
    pub id: u32,
}

/// Sky, and anything else the world shows through; never outlined itself.
pub const ID_BACKGROUND: u32 = 0;
/// All terrain is one object, so blended material boundaries carry no outline.
pub const ID_TERRAIN: u32 = 1;
/// Trees, mobs and the player take ids from here up, in the order they are drawn.
pub const FIRST_OBJECT_ID: u32 = 2;

impl Instance {
    pub fn tile(x: i32, y: i32, mask: u16, mat: u16) -> Instance {
        Instance {
            pos: [x as f32, y as f32],
            axis_x: [1.0, 0.0],
            axis_y: [0.0, 1.0],
            mask,
            mat: textured(mat, [x, y]),
            tint: [255, 255, 255, 255],
            id: ID_TERRAIN,
        }
    }

    /// The material column, without the packed block cell.
    #[cfg(test)]
    pub fn material(&self) -> u16 {
        self.mat & 0xFF
    }
}

/// Packs a material with the cell of its multi-tile block that `anchor` falls on.
///
/// Chosen on the CPU from a tile position the quad belongs to, rather than in the
/// shader from wherever the quad is drawn: a toppling trunk keeps its bark instead
/// of the texture flickering as its corners cross tile boundaries.
pub fn textured(mat: u16, anchor: [i32; 2]) -> u16 {
    let n = atlas::BLOCK_TILES as i32;
    let (sx, sy) = (anchor[0].rem_euclid(n) as u16, anchor[1].rem_euclid(n) as u16);
    mat | (sx << 8) | (sy << 12)
}

/// At most four materials can bleed into one tile (air plus three others), so the
/// per-tile overlay list never allocates.
const MAX_OVERLAYS: usize = 4;
/// Bleeding materials plus any decoration the tile carries.
const MAX_LAYERS: usize = MAX_OVERLAYS + 4;

/// Appends the base instance for a solid tile to `bases`, and its overlays to
/// `overlays`. Air tiles produce nothing.
pub fn emit_tile(world: &World, x: i32, y: i32, bases: &mut Vec<Instance>, overlays: &mut Vec<Instance>) {
    let id = drawn_as(world.get(x, y));
    if id == AIR {
        // A hole shows what is behind it: the backdrop outdoors, nothing at all
        // underground. So air draws a black veil over the backdrop, thickening with
        // depth, which is what makes a dug tunnel read as a tunnel rather than as a
        // window. A gradient rather than a switch: keying on "is anything above me"
        // put a hard seam between a dug shaft showing sky and untouched rock beside
        // it. Blended like the overlays, and leaving the id as background.
        let veil = 255 - (world.skyness(x, y) * 255.0) as u8;
        if veil > 0 {
            overlays.push(Instance { tint: [0, 0, 0, veil], id: ID_BACKGROUND, ..Instance::tile(x, y, SOLID_MASK, MAT_SKY) });
        }
        return;
    }
    if !is_solid(id) {
        // Torches and the like: drawn whole, and taking no transitions.
        bases.push(Instance { id: ID_TERRAIN, ..Instance::tile(x, y, SOLID_MASK, atlas_mat(id)) });
        return;
    }
    bases.push(Instance::tile(x, y, SOLID_MASK, atlas_mat(id)));

    // Collect the neighbour bitmask per bleeding material, and separately which
    // sides face air, since that is where decoration grows.
    let mut found: [(TileId, u8); MAX_OVERLAYS] = [(AIR, 0); MAX_OVERLAYS];
    let mut count = 0;
    let mut air_bits: u8 = 0;
    for (bit, (dx, dy)) in DIRS.iter().enumerate() {
        let neighbour = drawn_as(world.get(x + dx, y + dy));
        if neighbour == AIR {
            air_bits |= 1 << bit;
        }
        if precedence(neighbour) <= precedence(id) {
            continue;
        }
        match found[..count].iter().position(|(n, _)| *n == neighbour) {
            Some(i) => found[i].1 |= 1 << bit,
            None if count < MAX_OVERLAYS => {
                found[count] = (neighbour, 1 << bit);
                count += 1;
            }
            None => {}
        }
    }

    // (precedence, mask bits, material), painted weakest first.
    let mut layers: [(u8, u16, u16); MAX_LAYERS] = [(0, 0, 0); MAX_LAYERS];
    let mut layer_count = 0;
    let mut push = |layer: (u8, u16, u16)| {
        if layer_count < MAX_LAYERS {
            layers[layer_count] = layer;
            layer_count += 1;
        }
    };
    for &(neighbour, bits) in &found[..count] {
        push((precedence(neighbour), bits as u16, overlay_mat(neighbour)));
    }
    let decor = world.decor(x, y);
    if decor != 0 && air_bits != 0 {
        for kind in DECOR_TYPES {
            if decor & kind.bit != 0 {
                push((kind.precedence, air_bits as u16, kind.atlas_mat));
            }
        }
    }

    layers[..layer_count].sort_by_key(|(p, _, _)| *p);
    for &(_, bits, mat) in &layers[..layer_count] {
        overlays.push(Instance::tile(x, y, bits, mat));
    }
}

/// Rebuilds one chunk's instance lists. Tiles on the chunk border read neighbours
/// from adjacent chunks, which is why `World::set` bumps their revisions too.
pub fn build_chunk(
    world: &World,
    cx: i32,
    cy: i32,
    bases: &mut Vec<Instance>,
    overlays: &mut Vec<Instance>,
) {
    bases.clear();
    overlays.clear();
    let size = crate::world::CHUNK_SIZE;
    for y in cy * size..(cy + 1) * size {
        for x in cx * size..(cx + 1) * size {
            emit_tile(world, x, y, bases, overlays);
        }
    }
}

/// Air bleeding onto a neighbour is a shadow cast on it, not the sky's own surface.
fn overlay_mat(id: TileId) -> u16 {
    if id == AIR { MAT_AIR_SHADOW } else { atlas_mat(id) }
}

/// The player and other sprites, until they have atlas cells of their own.
pub fn tinted_box(pos: [f32; 2], size: [f32; 2], tint: [u8; 4], id: u32) -> Instance {
    Instance {
        pos,
        axis_x: [size[0], 0.0],
        axis_y: [0.0, size[1]],
        mask: SOLID_MASK,
        mat: atlas::MAT_WHITE,
        tint,
        id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{DECOR_GRASS, DIRT, STONE};

    const N: u16 = 1;
    const E: u16 = 1 << 1;
    const S: u16 = 1 << 2;
    const W: u16 = 1 << 3;
    const NE: u16 = 1 << 4;

    /// 64x64 of stone with a 3-tile margin, so (32, 32) is deep interior.
    fn solid_world() -> World {
        let mut w = World::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                w.set(x, y, STONE);
            }
        }
        w
    }

    fn emitted(world: &World, x: i32, y: i32) -> (Vec<Instance>, Vec<Instance>) {
        let (mut bases, mut overlays) = (Vec::new(), Vec::new());
        emit_tile(world, x, y, &mut bases, &mut overlays);
        (bases, overlays)
    }

    #[test]
    fn an_interior_tile_emits_only_an_opaque_base() {
        let world = solid_world();
        let (bases, overlays) = emitted(&world, 32, 32);
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0].mask, SOLID_MASK);
        assert_eq!(bases[0].id, ID_TERRAIN, "every material is one terrain object: no line where they blend");
        assert_eq!(bases[0].material(), atlas_mat(STONE));
        assert!(overlays.is_empty(), "an enclosed tile needs no overlay");
    }

    #[test]
    fn air_above_bleeds_down_as_a_north_overlay() {
        let mut world = solid_world();
        world.set(32, 31, AIR);
        let (_, overlays) = emitted(&world, 32, 32);
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].material(), MAT_AIR_SHADOW);
        assert_eq!(overlays[0].mask, N, "expected only the north bit");
    }

    #[test]
    fn a_diagonal_only_neighbour_still_emits_its_corner() {
        let mut world = solid_world();
        world.set(33, 31, AIR); // north-east only; orthogonals stay solid
        let (_, overlays) = emitted(&world, 32, 32);
        assert_eq!(overlays.len(), 1, "a diagonal neighbour must not be dropped");
        assert_eq!(overlays[0].mask, NE);
    }

    #[test]
    fn several_exposed_sides_combine_into_one_overlay() {
        let mut world = solid_world();
        for (x, y) in [(32, 31), (33, 32), (32, 33), (31, 32), (33, 31)] {
            world.set(x, y, AIR);
        }
        let (_, overlays) = emitted(&world, 32, 32);
        assert_eq!(overlays.len(), 1, "one material bleeds through one quad");
        assert_eq!(overlays[0].mask, N | E | S | W | NE);
    }

    #[test]
    fn precedence_decides_which_way_a_boundary_bleeds() {
        let mut world = solid_world();
        world.set(32, 31, DIRT);

        let (_, onto_stone) = emitted(&world, 32, 32);
        assert_eq!(onto_stone.len(), 1, "dirt outranks stone and should bleed down");
        assert_eq!(onto_stone[0].material(), atlas_mat(DIRT));

        let (_, onto_dirt) = emitted(&world, 32, 31);
        let from_stone = onto_dirt.iter().any(|o| o.material() == atlas_mat(STONE));
        assert!(!from_stone, "stone must not bleed up onto dirt");
    }

    #[test]
    fn grass_grows_on_the_air_facing_sides_of_marked_dirt() {
        let mut world = solid_world();
        world.set(32, 32, DIRT);
        world.set(32, 31, AIR);
        world.set_decor(32, 32, DECOR_GRASS);

        let (_, overlays) = emitted(&world, 32, 32);
        let grass = DECOR_TYPES[0].atlas_mat;
        assert_eq!(overlays.len(), 2, "grass and the air edge both apply");
        assert_eq!(overlays[0].material(), grass, "grass paints before the air edge");
        assert_eq!(overlays[0].mask, N, "grass follows the air-facing sides");
        assert_eq!(overlays[1].material(), MAT_AIR_SHADOW, "the air edge paints last");
    }

    #[test]
    fn buried_dirt_shows_no_grass_even_when_marked() {
        let mut world = solid_world();
        world.set(32, 32, DIRT);
        world.set_decor(32, 32, DECOR_GRASS);
        let (_, overlays) = emitted(&world, 32, 32);
        assert!(overlays.is_empty(), "grass needs a face touching air");
    }

    #[test]
    fn freshly_exposed_dirt_is_bare() {
        let mut world = solid_world();
        for y in 30..34 {
            for x in 30..34 {
                world.set(x, y, DIRT);
            }
        }
        world.set_decor(32, 31, DECOR_GRASS);
        world.set(32, 31, AIR); // mine the grassy tile away

        let (_, overlays) = emitted(&world, 32, 32);
        let grass = DECOR_TYPES[0].atlas_mat;
        assert!(
            !overlays.iter().any(|o| o.material() == grass),
            "dirt uncovered by mining must not sprout grass on its own"
        );
    }

    #[test]
    fn air_shows_the_backdrop_and_veils_it_with_depth() {
        let world = World::new(64, 64);
        let (bases, overlays) = emitted(&world, 32, 32);
        assert!(bases.is_empty(), "open air draws nothing over the backdrop");
        assert!(overlays.is_empty(), "and takes no transitions");

        // The veil thickens with depth: a shallow hole still shows the backdrop, a
        // deep one shows nothing. Both matter — veiling immediately makes a dug
        // notch at the surface look like it is in shadow.
        let mut buried = solid_world();
        buried.snapshot_surface();
        buried.set(32, 3, AIR);
        buried.set(32, 60, AIR);

        let (_, shallow) = emitted(&buried, 32, 3);
        assert!(!shallow.iter().any(|o| o.material() == MAT_SKY), "a hole near the surface is unveiled");
        let (bases, deep) = emitted(&buried, 32, 60);
        assert!(bases.is_empty());
        let veil = deep.iter().find(|o| o.material() == MAT_SKY).expect("a deep pocket is veiled");
        assert_eq!(veil.tint, [0, 0, 0, 255], "a deep pocket shows nothing behind it");
        assert_eq!(veil.id, ID_BACKGROUND, "air stays background, so terrain against it is outlined");
    }

    #[test]
    fn every_mask_an_overlay_can_reference_exists_in_the_atlas() {
        assert!(u16::from(u8::MAX) < atlas::MASK_COUNT, "8-bit masks must be addressable");
        assert!(SOLID_MASK < atlas::MASK_COUNT);
    }

    #[test]
    fn a_chunk_build_covers_every_tile_once() {
        let world = solid_world();
        let (mut bases, mut overlays) = (Vec::new(), Vec::new());
        build_chunk(&world, 0, 0, &mut bases, &mut overlays);
        assert_eq!(bases.len(), crate::world::CHUNK_AREA, "every tile emits a base");
        // The chunk's outer ring borders the world edge, which reads as air.
        assert!(!overlays.is_empty(), "the world edge should bleed inward");
    }
}
