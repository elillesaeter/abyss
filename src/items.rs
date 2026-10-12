//! Items and the inventory that holds them.

use crate::icons::{self, Icon};
use crate::tiles::{DIRT, STONE, TORCH, TileId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Sword,
    Dirt,
    Stone,
    Torch,
    /// Carried inside gel cubes and dropped when they burst; an early crafting
    /// material.
    Bone,
}

impl Item {
    pub fn max_stack(self) -> u16 {
        match self {
            Item::Sword => 1,
            _ => 999,
        }
    }

    pub fn weapon(self) -> Option<Weapon> {
        match self {
            Item::Sword => Some(STARTER_SWORD),
            _ => None,
        }
    }

    /// The tile this item puts down, if it is placeable.
    pub fn places(self) -> Option<TileId> {
        match self {
            Item::Dirt => Some(DIRT),
            Item::Stone => Some(STONE),
            Item::Torch => Some(TORCH),
            Item::Sword | Item::Bone => None,
        }
    }
}

/// What a swung weapon does. Its reach isn't a number here: it's however far
/// the painted sprite extends from its grip, so a longer blade reaches farther.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weapon {
    pub sprite: Icon,
    /// Where the hand holds the sprite, in its pixels.
    pub grip: [f32; 2],
    pub damage: i32,
    /// Seconds per swing; held down, swings repeat at this rate.
    pub use_time: f32,
    /// How hard a hit throws a mob, in tiles per second before its size.
    pub knockback: f32,
}

impl Weapon {
    /// Art pixels from the grip to the farthest painted pixel.
    pub fn reach(&self) -> f32 {
        icons::reach(self.sprite, self.grip)
    }
}

pub const STARTER_SWORD: Weapon = Weapon {
    sprite: icons::SWORD,
    grip: icons::SWORD_GRIP,
    damage: 20,
    use_time: 0.4,
    knockback: 4.5,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stack {
    pub item: Item,
    pub count: u16,
}

pub const HOTBAR: usize = 10;

/// For now the inventory is the hotbar: ten slots, one of them held.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    pub slots: [Option<Stack>; HOTBAR],
    pub selected: usize,
}

impl Inventory {
    pub fn starter() -> Inventory {
        let mut inv = Inventory {
            slots: [None; HOTBAR],
            selected: 0,
        };
        inv.add(Item::Sword, 1);
        inv.add(Item::Dirt, 99);
        inv.add(Item::Stone, 99);
        inv.add(Item::Torch, 50);
        inv
    }

    pub fn held(&self) -> Option<Item> {
        self.slots[self.selected].map(|s| s.item)
    }

    /// Selects a slot, wrapping, so the scroll wheel can step past either end.
    pub fn select(&mut self, slot: i32) {
        self.selected = slot.rem_euclid(HOTBAR as i32) as usize;
    }

    /// Puts `count` of `item` away: topping up stacks of it first, then empty
    /// slots. Returns what didn't fit.
    pub fn add(&mut self, item: Item, mut count: u16) -> u16 {
        for slot in self.slots.iter_mut().flatten().filter(|s| s.item == item) {
            let moved = count.min(item.max_stack() - slot.count);
            slot.count += moved;
            count -= moved;
        }
        for slot in self.slots.iter_mut().filter(|s| s.is_none()) {
            if count == 0 {
                break;
            }
            let moved = count.min(item.max_stack());
            *slot = Some(Stack { item, count: moved });
            count -= moved;
        }
        count
    }

    /// Uses up one of the held item, emptying the slot at zero.
    pub fn consume_held(&mut self) {
        let slot = &mut self.slots[self.selected];
        if let Some(stack) = slot {
            stack.count -= 1;
            if stack.count == 0 {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_up_tops_up_stacks_before_taking_empty_slots() {
        let mut inv = Inventory::starter();
        assert_eq!(inv.add(Item::Bone, 2), 0);
        let bones = inv.slots.iter().flatten().filter(|s| s.item == Item::Bone).count();
        assert_eq!(bones, 1);
        inv.add(Item::Bone, 3);
        let stacks: Vec<_> = inv.slots.iter().flatten().filter(|s| s.item == Item::Bone).collect();
        assert_eq!(stacks.len(), 1, "bones merge into one stack");
        assert_eq!(stacks[0].count, 5);
    }

    #[test]
    fn a_full_inventory_hands_back_what_does_not_fit() {
        let mut inv = Inventory {
            slots: [None; HOTBAR],
            selected: 0,
        };
        for _ in 0..HOTBAR {
            inv.add(Item::Sword, 1);
        }
        assert_eq!(inv.add(Item::Sword, 1), 1, "swords don't stack");
        assert_eq!(inv.add(Item::Bone, 4), 4, "no room");
    }

    #[test]
    fn placing_uses_up_the_held_stack_and_selection_wraps() {
        let mut inv = Inventory::starter();
        inv.select(3);
        assert_eq!(inv.held(), Some(Item::Torch));
        for _ in 0..50 {
            inv.consume_held();
        }
        assert_eq!(inv.held(), None, "the last torch empties the slot");
        inv.select(-1);
        assert_eq!(inv.selected, HOTBAR - 1);
        inv.select(HOTBAR as i32);
        assert_eq!(inv.selected, 0);
    }
}
