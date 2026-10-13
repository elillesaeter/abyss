//! Items and the inventory that holds them.

use crate::icons::{self, Icon};
use crate::tiles::{DIRT, STONE, TORCH, TileId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Sword,
    Pickaxe,
    Axe,
    /// Left where a felled tree lands; a building and crafting material.
    Wood,
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
            Item::Sword | Item::Pickaxe | Item::Axe => 1,
            _ => 999,
        }
    }

    pub fn weapon(self) -> Option<Weapon> {
        match self {
            Item::Sword => Some(STARTER_SWORD),
            Item::Pickaxe => Some(STARTER_PICKAXE),
            Item::Axe => Some(STARTER_AXE),
            _ => None,
        }
    }

    /// Which tool belt slot this item belongs in, and how hard it hits tiles.
    pub fn tool(self) -> Option<(ToolKind, i32)> {
        match self {
            Item::Pickaxe => Some((ToolKind::Pickaxe, 35)),
            Item::Axe => Some((ToolKind::Axe, 35)),
            _ => None,
        }
    }

    /// The tile this item puts down, if it is placeable.
    pub fn places(self) -> Option<TileId> {
        match self {
            Item::Dirt => Some(DIRT),
            Item::Stone => Some(STONE),
            Item::Torch => Some(TORCH),
            Item::Sword | Item::Pickaxe | Item::Axe | Item::Wood | Item::Bone => None,
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

/// What breaks what: a pickaxe mines blocks, an axe chops trunks, a hammer
/// (later) knocks down walls. Also the order of the tool belt's slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Pickaxe,
    Axe,
    #[allow(dead_code)] // a slot kept for hammers, once walls exist to knock down
    Hammer,
}

pub const TOOL_SLOTS: usize = 3;

pub const STARTER_PICKAXE: Weapon = Weapon {
    sprite: icons::PICKAXE,
    grip: icons::TOOL_GRIP,
    damage: 5,
    use_time: 0.25,
    knockback: 2.0,
};

pub const STARTER_AXE: Weapon = Weapon {
    sprite: icons::AXE,
    grip: icons::TOOL_GRIP,
    damage: 7,
    use_time: 0.35,
    knockback: 3.0,
};

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

/// For now the inventory is the hotbar: ten slots, one of them held. Beside it
/// the tool belt, one slot per kind of tool, which takes nothing else.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    pub slots: [Option<Stack>; HOTBAR],
    pub selected: usize,
    pub tools: [Option<Item>; TOOL_SLOTS],
}

impl Inventory {
    pub fn starter() -> Inventory {
        let mut inv = Inventory {
            slots: [None; HOTBAR],
            selected: 0,
            tools: [None; TOOL_SLOTS],
        };
        inv.tools = [Some(Item::Pickaxe), Some(Item::Axe), None];
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

    /// Puts a tool in its belt slot, handing back whatever was there; anything
    /// that isn't a tool is refused and handed straight back.
    pub fn equip(&mut self, item: Item) -> Result<Option<Item>, Item> {
        let (kind, _) = item.tool().ok_or(item)?;
        Ok(self.tools[kind as usize].replace(item))
    }

    pub fn tool(&self, kind: ToolKind) -> Option<Item> {
        self.tools[kind as usize]
    }

    /// Whether at least one more `item` would go in.
    pub fn fits(&self, item: Item) -> bool {
        self.slots
            .iter()
            .any(|s| s.is_none_or(|s| s.item == item && s.count < item.max_stack()))
    }

    pub fn has(&self, item: Item) -> bool {
        self.slots.iter().flatten().any(|s| s.item == item)
    }

    /// Uses up one `item`: from the held slot if it holds one, else the first
    /// slot that does.
    pub fn consume(&mut self, item: Item) {
        let slot = if self.held() == Some(item) {
            Some(self.selected)
        } else {
            self.slots.iter().position(|s| s.is_some_and(|s| s.item == item))
        };
        if let Some(i) = slot {
            let stack = &mut self.slots[i];
            if let Some(s) = stack {
                s.count -= 1;
                if s.count == 0 {
                    *stack = None;
                }
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
            tools: [None; TOOL_SLOTS],
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
            inv.consume(Item::Torch);
        }
        assert_eq!(inv.held(), None, "the last torch empties the slot");
        inv.select(-1);
        assert_eq!(inv.selected, HOTBAR - 1);
        inv.select(HOTBAR as i32);
        assert_eq!(inv.selected, 0);
    }

    #[test]
    fn the_tool_belt_takes_only_its_own_tool_in_each_slot() {
        let mut inv = Inventory::starter();
        assert_eq!(inv.tool(ToolKind::Pickaxe), Some(Item::Pickaxe));
        assert_eq!(inv.tool(ToolKind::Axe), Some(Item::Axe));
        assert_eq!(inv.tool(ToolKind::Hammer), None, "the hammer slot waits");
        assert_eq!(inv.equip(Item::Dirt), Err(Item::Dirt));
        assert_eq!(
            inv.equip(Item::Pickaxe),
            Ok(Some(Item::Pickaxe)),
            "swapping hands back the old one"
        );
        assert!(
            !inv.slots.iter().flatten().any(|s| s.item.tool().is_some()),
            "tools stay off the hotbar"
        );
    }

    #[test]
    fn consuming_takes_from_the_held_slot_first() {
        let mut inv = Inventory::starter();
        inv.add(Item::Torch, 999);
        inv.select(3);
        let before = inv.slots[3].unwrap().count;
        inv.consume(Item::Torch);
        assert_eq!(inv.slots[3].unwrap().count, before - 1);
        inv.select(0);
        inv.consume(Item::Torch);
        assert_eq!(
            inv.slots[3].unwrap().count,
            before - 2,
            "falls back to the first torch stack"
        );
    }
}
