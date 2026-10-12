//! Components for the entity store.
//!
//! The store itself is `hecs`, used purely as storage and queries: there is no
//! scheduler, and systems run in an order this crate decides explicitly, so frames
//! are reproducible and tests can drive a single system in isolation.

/// A kinematic axis-aligned box.
///
/// Deliberately not a rigid body: it never rotates, responds to no forces beyond
/// the gravity its system applies, and resolves against tiles only — not against
/// other bodies. Position, velocity, extents and grounded state are one component
/// because nothing in this game has any of them without the rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub pos: [f32; 2],
    pub vel: [f32; 2],
    pub half: [f32; 2],
    pub grounded: bool,
}

impl Body {
    pub fn new(pos: [f32; 2], half: [f32; 2]) -> Body {
        Body {
            pos,
            vel: [0.0, 0.0],
            half,
            grounded: false,
        }
    }
}

/// Index into `mob::MOB_KINDS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind(pub u16);

/// How an entity is drawn, which is deliberately not how it collides: a character
/// is wider and taller than its hitbox, and stands on its feet rather than being
/// centred on the box. Kept apart from `Body` because this is the half that will
/// grow — atlas cell, animation frame, facing, draw order — while `Body` will not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub half: [f32; 2],
    /// Added to the body's centre to place the sprite's centre.
    pub offset: [f32; 2],
    pub tint: [u8; 4],
}

impl Sprite {
    /// A sprite of `half` extents standing on the same ground as a body of
    /// `body_half`, so extra height rises rather than sinking into the floor.
    pub fn standing_on(body_half: [f32; 2], half: [f32; 2], tint: [u8; 4]) -> Sprite {
        Sprite {
            half,
            offset: [0.0, body_half[1] - half[1]],
            tint,
        }
    }

    /// Top-left corner of the drawn quad for a body centred at `pos`.
    pub fn corner(&self, pos: [f32; 2]) -> [f32; 2] {
        [
            pos[0] + self.offset[0] - self.half[0],
            pos[1] + self.offset[1] - self.half[1],
        ]
    }

    pub fn size(&self) -> [f32; 2] {
        [self.half[0] * 2.0, self.half[1] * 2.0]
    }
}

pub type Entities = hecs::World;
