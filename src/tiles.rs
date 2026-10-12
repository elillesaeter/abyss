//! Tile kinds, the decorations that grow on them, and the properties they share.
//!
//! Anything constant for every tile of a kind lives here, indexed by `TileId`,
//! so the world grid only ever stores the id. Adding kinds costs no per-tile bytes.

use crate::atlas::{MAT_DIRT, MAT_GRASS, MAT_STONE};

pub type TileId = u16;

pub const AIR: TileId = 0;
pub const DIRT: TileId = 1;
pub const STONE: TileId = 2;
pub const TORCH: TileId = 3;
/// A cell of a standing tree's trunk. Drawn by the tree, not the grid; the tile is
/// what chopping and (later) climbing act on.
pub const TRUNK: TileId = 4;

pub struct TileType {
    pub name: &'static str,
    pub solid: bool,
    /// Whether mining removes it. Separate from `solid` because trunks are chopped
    /// but walked through, and torches block nothing but are not mined.
    pub breakable: bool,
    /// Material block in the atlas.
    pub atlas_mat: u16,
    /// Higher-precedence materials bleed over lower ones at a boundary. Air is
    /// highest, so an exposed edge always wins.
    pub precedence: u8,
    /// Base palette colour the atlas generator textures from.
    pub color: [u8; 4],
    /// Light this kind emits, per channel. Zero for everything that does not glow.
    pub light: [f32; 3],
    /// Movement multiplier for a body overlapping this tile. 1.0 is unimpeded;
    /// liquids and cobwebs will be below 1.0. Ignored for solid tiles.
    pub drag: f32,
}

pub const TILE_TYPES: &[TileType] = &[
    // Air's own material is the sky; the dark edge it casts on neighbours is a
    // separate material, since one is a surface and the other is a shadow.
    TileType {
        name: "air",
        solid: false,
        breakable: false,
        atlas_mat: 5,
        precedence: 255,
        color: [0, 0, 0, 0],
        light: [0.0; 3],
        drag: 1.0,
    },
    TileType {
        name: "dirt",
        solid: true,
        breakable: true,
        atlas_mat: MAT_DIRT,
        precedence: 2,
        color: [114, 79, 57, 255],
        light: [0.0; 3],
        drag: 1.0,
    },
    TileType {
        name: "stone",
        solid: true,
        breakable: true,
        atlas_mat: MAT_STONE,
        precedence: 1,
        color: [92, 85, 91, 255],
        light: [0.0; 3],
        drag: 1.0,
    },
    TileType {
        name: "torch",
        solid: false,
        breakable: false,
        atlas_mat: 6,
        precedence: 254,
        color: [250, 196, 120, 255],
        light: [1.0, 0.72, 0.38],
        drag: 1.0,
    },
    // No colour: the grid draws a trunk cell as the air behind it (see `drawn_as`).
    TileType {
        name: "trunk",
        solid: false,
        breakable: true,
        atlas_mat: 5,
        precedence: 255,
        color: [0, 0, 0, 0],
        light: [0.0; 3],
        drag: 1.0,
    },
];

/// Growth living on a tile's exposed faces rather than replacing the tile. One bit
/// per kind in the chunk's `decor` array, so a tile can carry several at once.
pub const DECOR_GRASS: u8 = 1 << 0;

pub struct DecorType {
    pub bit: u8,
    pub name: &'static str,
    pub atlas_mat: u16,
    /// Drawn after the materials it covers, before the air edge.
    pub precedence: u8,
    pub color: [u8; 4],
}

pub const DECOR_TYPES: &[DecorType] = &[DecorType {
    bit: DECOR_GRASS,
    name: "grass",
    atlas_mat: MAT_GRASS,
    precedence: 10,
    color: [96, 122, 54, 255],
}];

pub fn tile_type(id: TileId) -> &'static TileType {
    &TILE_TYPES[id as usize]
}

pub fn is_solid(id: TileId) -> bool {
    tile_type(id).solid
}

pub fn precedence(id: TileId) -> u8 {
    tile_type(id).precedence
}

pub fn atlas_mat(id: TileId) -> u16 {
    tile_type(id).atlas_mat
}

pub fn is_breakable(id: TileId) -> bool {
    tile_type(id).breakable
}

/// What the grid draws for a tile. A trunk cell is drawn as air — sky behind it,
/// grass and edge shadow on the ground under it — because the tree draws itself.
pub fn drawn_as(id: TileId) -> TileId {
    if id == TRUNK { AIR } else { id }
}
