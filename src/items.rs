//! Items and the inventory that holds them: the hotbar, a backpack, bags,
//! gear and tool belt, and the stack held on the mouse pointer.

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
    /// A small bag: worn in a bag slot, it adds its slots to the inventory.
    Pouch,
}

impl Item {
    pub fn max_stack(self) -> u16 {
        match self {
            Item::Sword | Item::Pickaxe | Item::Axe | Item::Pouch => 1,
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

    /// How many slots this item adds, if it is a bag.
    pub fn bag_size(self) -> Option<usize> {
        match self {
            Item::Pouch => Some(8),
            _ => None,
        }
    }

    /// The tile this item puts down, if it is placeable.
    pub fn places(self) -> Option<TileId> {
        match self {
            Item::Dirt => Some(DIRT),
            Item::Stone => Some(STONE),
            Item::Torch => Some(TORCH),
            _ => None,
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
/// The backpack every player has, before any bags.
pub const BACKPACK: usize = 10;
pub const BAG_SLOTS: usize = 4;
/// Head, back, neck, hands, chest, legs, feet, then five accessories.
pub const GEAR: usize = 12;

/// Where an item can sit, for the mouse to point at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Hotbar(usize),
    Backpack(usize),
    /// A slot inside the bag worn in bag slot `.0`.
    Bag(usize, usize),
    /// Where a bag is worn.
    BagSlot(usize),
    Gear(usize),
    Tool(usize),
}

/// A worn bag and what it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Bag {
    pub item: Item,
    pub slots: Vec<Option<Stack>>,
}

impl Bag {
    fn new(item: Item) -> Bag {
        Bag {
            item,
            slots: vec![None; item.bag_size().unwrap_or(0)],
        }
    }

    fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
}

/// Everything the player carries. One slot of the hotbar is held; the
/// backpack and worn bags hold the rest; gear and tools are worn; `cursor` is
/// whatever has been picked up with the mouse.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    pub hotbar: [Option<Stack>; HOTBAR],
    pub selected: usize,
    pub backpack: [Option<Stack>; BACKPACK],
    pub bags: [Option<Bag>; BAG_SLOTS],
    pub gear: [Option<Stack>; GEAR],
    pub tools: [Option<Stack>; TOOL_SLOTS],
    pub cursor: Option<Stack>,
}

impl Inventory {
    /// Nothing at all, not even a bag.
    pub fn empty() -> Inventory {
        Inventory {
            hotbar: [None; HOTBAR],
            selected: 0,
            backpack: [None; BACKPACK],
            bags: [const { None }; BAG_SLOTS],
            gear: [None; GEAR],
            tools: [None; TOOL_SLOTS],
            cursor: None,
        }
    }

    pub fn starter() -> Inventory {
        let mut inv = Inventory::empty();
        inv.tools = [Some(Item::Pickaxe), Some(Item::Axe), None].map(|t| t.map(|item| Stack { item, count: 1 }));
        inv.bags[0] = Some(Bag::new(Item::Pouch));
        inv.add(Item::Sword, 1);
        inv.add(Item::Dirt, 99);
        inv.add(Item::Stone, 99);
        inv.add(Item::Torch, 50);
        inv
    }

    pub fn held(&self) -> Option<Item> {
        self.hotbar[self.selected].map(|s| s.item)
    }

    /// Selects a slot, wrapping, so the scroll wheel can step past either end.
    pub fn select(&mut self, slot: i32) {
        self.selected = slot.rem_euclid(HOTBAR as i32) as usize;
    }

    pub fn tool(&self, kind: ToolKind) -> Option<Item> {
        self.tools[kind as usize].map(|s| s.item)
    }

    /// Where things are kept, in the order pickups fill them: the hotbar, the
    /// backpack, then each worn bag.
    pub fn storage(&self) -> Vec<Slot> {
        let mut slots: Vec<Slot> = (0..HOTBAR).map(Slot::Hotbar).collect();
        slots.extend((0..BACKPACK).map(Slot::Backpack));
        for (b, bag) in self.bags.iter().enumerate() {
            if let Some(bag) = bag {
                slots.extend((0..bag.slots.len()).map(|i| Slot::Bag(b, i)));
            }
        }
        slots
    }

    /// The stack in `slot`; a worn bag shows as one of its item.
    pub fn get(&self, slot: Slot) -> Option<Stack> {
        match slot {
            Slot::Hotbar(i) => self.hotbar[i],
            Slot::Backpack(i) => self.backpack[i],
            Slot::Bag(b, i) => self.bags[b]
                .as_ref()
                .and_then(|bag| bag.slots.get(i).copied().flatten()),
            Slot::BagSlot(b) => self.bags[b].as_ref().map(|bag| Stack {
                item: bag.item,
                count: 1,
            }),
            Slot::Gear(i) => self.gear[i],
            Slot::Tool(i) => self.tools[i],
        }
    }

    /// The place a slot's stack is kept. Bag slots aren't stacks, so none.
    fn cell(&mut self, slot: Slot) -> Option<&mut Option<Stack>> {
        match slot {
            Slot::Hotbar(i) => self.hotbar.get_mut(i),
            Slot::Backpack(i) => self.backpack.get_mut(i),
            Slot::Bag(b, i) => self.bags[b].as_mut().and_then(|bag| bag.slots.get_mut(i)),
            Slot::BagSlot(_) => None,
            Slot::Gear(i) => self.gear.get_mut(i),
            Slot::Tool(i) => self.tools.get_mut(i),
        }
    }

    /// Whether `item` may sit in `slot`: tools only in their own belt slot,
    /// bags only in bag slots and never inside a bag.
    pub fn accepts(slot: Slot, item: Item) -> bool {
        match slot {
            Slot::Hotbar(_) | Slot::Backpack(_) => true,
            Slot::Bag(..) => item.bag_size().is_none(),
            Slot::BagSlot(_) => item.bag_size().is_some(),
            Slot::Tool(i) => item.tool().is_some_and(|(kind, _)| kind as usize == i),
            // ponytail: no armour or accessories exist yet, so gear takes nothing.
            Slot::Gear(_) => false,
        }
    }

    /// Puts up to `count` of `item` into `slots`: topping up stacks of it
    /// first, then empty slots. Returns what didn't fit.
    fn add_into(&mut self, slots: &[Slot], item: Item, mut count: u16) -> u16 {
        for &slot in slots {
            if let Some(Some(s)) = self.cell(slot)
                && s.item == item
            {
                let moved = count.min(item.max_stack() - s.count);
                s.count += moved;
                count -= moved;
            }
        }
        for &slot in slots {
            if count == 0 || !Self::accepts(slot, item) {
                continue;
            }
            if let Some(cell) = self.cell(slot)
                && cell.is_none()
            {
                let moved = count.min(item.max_stack());
                *cell = Some(Stack { item, count: moved });
                count -= moved;
            }
        }
        count
    }

    /// Puts `count` of `item` away. Returns what didn't fit.
    pub fn add(&mut self, item: Item, count: u16) -> u16 {
        self.add_into(&self.storage(), item, count)
    }

    /// Whether at least one more `item` would go in.
    pub fn fits(&self, item: Item) -> bool {
        self.storage().into_iter().any(|slot| {
            Self::accepts(slot, item)
                && self
                    .get(slot)
                    .is_none_or(|s| s.item == item && s.count < item.max_stack())
        })
    }

    /// Whether any `item` is carried, the cursor included.
    pub fn has(&self, item: Item) -> bool {
        self.cursor.is_some_and(|c| c.item == item)
            || self
                .storage()
                .into_iter()
                .any(|s| self.get(s).is_some_and(|s| s.item == item))
    }

    /// Uses up one `item`: from the cursor if it holds some, then the held
    /// slot, then the first slot that does.
    pub fn consume(&mut self, item: Item) {
        if let Some(c) = &mut self.cursor
            && c.item == item
        {
            c.count -= 1;
            if c.count == 0 {
                self.cursor = None;
            }
            return;
        }
        let slot = if self.held() == Some(item) {
            Some(Slot::Hotbar(self.selected))
        } else {
            self.storage()
                .into_iter()
                .find(|&s| self.get(s).is_some_and(|s| s.item == item))
        };
        if let Some(cell) = slot.and_then(|s| self.cell(s))
            && let Some(s) = cell
        {
            s.count -= 1;
            if s.count == 0 {
                *cell = None;
            }
        }
    }

    /// A left click on `slot`: picks up, puts down, tops up, or swaps with the
    /// cursor, as far as the slot allows.
    pub fn click(&mut self, slot: Slot) {
        if let Slot::BagSlot(b) = slot {
            return self.click_bag_slot(b);
        }
        let cursor = self.cursor;
        if cursor.is_some_and(|c| !Self::accepts(slot, c.item)) {
            return;
        }
        let Some(cell) = self.cell(slot) else { return };
        match (cursor, *cell) {
            (Some(c), Some(s)) if c.item == s.item => {
                let moved = c.count.min(s.item.max_stack() - s.count);
                *cell = Some(Stack {
                    count: s.count + moved,
                    ..s
                });
                self.cursor = (c.count > moved).then_some(Stack {
                    count: c.count - moved,
                    ..c
                });
            }
            (c, s) => {
                *cell = c;
                self.cursor = s;
            }
        }
    }

    /// Bags go on only into a bag slot, come off only empty, and a bag can be
    /// swapped for another that holds everything the old one did.
    fn click_bag_slot(&mut self, b: usize) {
        match (self.cursor, self.bags[b].take()) {
            (None, Some(bag)) if bag.is_empty() => {
                self.cursor = Some(Stack {
                    item: bag.item,
                    count: 1,
                })
            }
            (Some(c), worn) if c.item.bag_size().is_some() => {
                let mut bag = Bag::new(c.item);
                let carried: Vec<Stack> = worn.iter().flat_map(|w| w.slots.iter().flatten().copied()).collect();
                if carried.len() > bag.slots.len() {
                    self.bags[b] = worn;
                    return;
                }
                for (cell, s) in bag.slots.iter_mut().zip(carried) {
                    *cell = Some(s);
                }
                self.bags[b] = Some(bag);
                self.cursor = worn.map(|w| Stack { item: w.item, count: 1 });
            }
            (_, worn) => self.bags[b] = worn,
        }
    }

    /// A right click on `slot`: takes one onto the cursor, if the cursor is
    /// empty or holds more of the same with room.
    pub fn take_one(&mut self, slot: Slot) {
        let cursor = self.cursor;
        let Some(cell) = self.cell(slot) else { return };
        let Some(s) = *cell else { return };
        let room = cursor.is_none_or(|c| c.item == s.item && c.count < s.item.max_stack());
        if !room {
            return;
        }
        *cell = (s.count > 1).then_some(Stack {
            count: s.count - 1,
            ..s
        });
        self.cursor = Some(Stack {
            count: cursor.map_or(0, |c| c.count) + 1,
            ..s
        });
    }

    /// A Shift-click on `slot`: gear goes where it's worn (a tool to the belt,
    /// swapping out the old one; a bag to an empty bag slot), worn things come
    /// off into the inventory, and anything else hops between the hotbar and
    /// the backpack and bags.
    pub fn quick_move(&mut self, slot: Slot) {
        let Some(stack) = self.get(slot) else { return };
        match slot {
            Slot::BagSlot(b) => {
                if self.bags[b].as_ref().is_some_and(Bag::is_empty) {
                    let bag = self.bags[b].take();
                    if self.add(stack.item, 1) > 0 {
                        self.bags[b] = bag;
                    }
                }
            }
            Slot::Gear(_) | Slot::Tool(_) => {
                let storage = self.storage();
                let cell = self.cell(slot).expect("worn slots are cells");
                let taken = cell.take();
                if self.add_into(&storage, stack.item, 1) > 0 {
                    *self.cell(slot).expect("worn slots are cells") = taken;
                }
            }
            _ => {
                if let Some((kind, _)) = stack.item.tool() {
                    let worn = self.tools[kind as usize].replace(stack);
                    *self.cell(slot).expect("storage slots are cells") = worn;
                } else if stack.item.bag_size().is_some()
                    && let Some(b) = self.bags.iter().position(Option::is_none)
                {
                    self.bags[b] = Some(Bag::new(stack.item));
                    *self.cell(slot).expect("storage slots are cells") = None;
                } else {
                    let to: Vec<Slot> = match slot {
                        Slot::Hotbar(_) => self
                            .storage()
                            .into_iter()
                            .filter(|s| !matches!(s, Slot::Hotbar(_)))
                            .collect(),
                        _ => (0..HOTBAR).map(Slot::Hotbar).collect(),
                    };
                    *self.cell(slot).expect("storage slots are cells") = None;
                    let left = self.add_into(&to, stack.item, stack.count);
                    *self.cell(slot).expect("storage slots are cells") =
                        (left > 0).then_some(Stack { count: left, ..stack });
                }
            }
        }
    }

    /// Puts the cursor's stack back into the inventory; returns what didn't
    /// fit, to be dropped.
    pub fn stow_cursor(&mut self) -> Option<Stack> {
        let c = self.cursor.take()?;
        let left = self.add(c.item, c.count);
        (left > 0).then_some(Stack { count: left, ..c })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_up_tops_up_stacks_before_taking_empty_slots() {
        let mut inv = Inventory::starter();
        assert_eq!(inv.add(Item::Bone, 2), 0);
        let bones = inv.hotbar.iter().flatten().filter(|s| s.item == Item::Bone).count();
        assert_eq!(bones, 1);
        inv.add(Item::Bone, 3);
        let stacks: Vec<_> = inv.hotbar.iter().flatten().filter(|s| s.item == Item::Bone).collect();
        assert_eq!(stacks.len(), 1, "bones merge into one stack");
        assert_eq!(stacks[0].count, 5);
    }

    #[test]
    fn a_full_inventory_hands_back_what_does_not_fit() {
        let mut inv = Inventory::empty();
        for _ in 0..HOTBAR + BACKPACK {
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

    fn stack(item: Item, count: u16) -> Option<Stack> {
        Some(Stack { item, count })
    }

    #[test]
    fn the_tool_belt_takes_only_its_own_tool_and_quick_equip_swaps_it() {
        let mut inv = Inventory::starter();
        assert_eq!(inv.tool(ToolKind::Pickaxe), Some(Item::Pickaxe));
        assert_eq!(inv.tool(ToolKind::Hammer), None, "the hammer slot waits");
        assert!(
            !inv.hotbar.iter().flatten().any(|s| s.item.tool().is_some()),
            "tools stay off the hotbar"
        );

        // Dirt on the cursor won't go into the pickaxe slot.
        inv.click(Slot::Hotbar(1));
        assert_eq!(inv.cursor.unwrap().item, Item::Dirt);
        inv.click(Slot::Tool(0));
        assert_eq!(inv.tool(ToolKind::Pickaxe), Some(Item::Pickaxe));
        inv.click(Slot::Hotbar(1));

        // A second pickaxe, Shift-clicked, takes the belt and hands back the old.
        inv.hotbar[9] = stack(Item::Pickaxe, 1);
        inv.tools[0] = None;
        inv.quick_move(Slot::Hotbar(9));
        assert_eq!(inv.tool(ToolKind::Pickaxe), Some(Item::Pickaxe));
        assert_eq!(inv.hotbar[9], None, "the belt was empty, so nothing comes back");
        inv.hotbar[9] = stack(Item::Pickaxe, 1);
        inv.quick_move(Slot::Hotbar(9));
        assert_eq!(inv.hotbar[9], stack(Item::Pickaxe, 1), "swapped with the worn one");
        inv.quick_move(Slot::Tool(0));
        assert_eq!(inv.tool(ToolKind::Pickaxe), None, "Shift-click takes it off again");
        assert!(inv.has(Item::Pickaxe));
    }

    #[test]
    fn clicks_pick_up_put_down_merge_and_swap() {
        let mut inv = Inventory::starter();
        inv.click(Slot::Hotbar(1)); // 99 dirt onto the cursor
        inv.click(Slot::Backpack(0));
        assert_eq!(inv.backpack[0], stack(Item::Dirt, 99));
        assert_eq!(inv.cursor, None);

        inv.take_one(Slot::Backpack(0));
        inv.take_one(Slot::Backpack(0));
        assert_eq!(inv.cursor, stack(Item::Dirt, 2), "right clicks take one at a time");
        inv.take_one(Slot::Hotbar(2));
        assert_eq!(inv.cursor, stack(Item::Dirt, 2), "not onto a cursor of something else");
        inv.click(Slot::Backpack(0));
        assert_eq!(inv.backpack[0], stack(Item::Dirt, 99), "merged back");

        inv.click(Slot::Hotbar(2)); // stone
        inv.click(Slot::Backpack(0)); // swap with the dirt
        assert_eq!(inv.backpack[0], stack(Item::Stone, 99));
        assert_eq!(inv.cursor, stack(Item::Dirt, 99));
        assert_eq!(inv.stow_cursor(), None, "put away on closing");
        assert!(inv.has(Item::Dirt) && inv.cursor.is_none());
    }

    #[test]
    fn bags_wear_by_the_rules_and_pickups_fill_hotbar_backpack_then_bags() {
        let mut inv = Inventory::starter();
        assert_eq!(inv.storage().len(), HOTBAR + BACKPACK + 8);
        // A bag can't go inside a bag.
        inv.cursor = stack(Item::Pouch, 1);
        inv.click(Slot::Bag(0, 0));
        assert_eq!(inv.get(Slot::Bag(0, 0)), None);
        // Fill order.
        let free = inv.hotbar.iter().chain(&inv.backpack).filter(|s| s.is_none()).count();
        for _ in 0..free {
            inv.add(Item::Sword, 1);
        }
        assert!(inv.hotbar.iter().chain(&inv.backpack).all(Option::is_some));
        assert_eq!(inv.get(Slot::Bag(0, 0)), None, "hotbar and backpack fill first");
        inv.add(Item::Bone, 3);
        assert_eq!(inv.get(Slot::Bag(0, 0)), stack(Item::Bone, 3));
        // A bag with something in it stays on.
        inv.cursor = None;
        inv.click(Slot::BagSlot(0));
        assert!(inv.bags[0].is_some() && inv.cursor.is_none());
        // Swapped for another bag that holds it all, the contents move across.
        inv.cursor = stack(Item::Pouch, 1);
        inv.click(Slot::BagSlot(0));
        assert_eq!(inv.get(Slot::Bag(0, 0)), stack(Item::Bone, 3));
        assert_eq!(
            inv.cursor,
            stack(Item::Pouch, 1),
            "the old bag comes off onto the cursor"
        );
        // Emptied, it comes off; Shift-clicking a bag wears it in an empty bag slot.
        inv.bags[0].as_mut().unwrap().slots[0] = None;
        inv.cursor = None;
        inv.click(Slot::BagSlot(0));
        assert_eq!(inv.cursor, stack(Item::Pouch, 1));
        inv.click(Slot::Hotbar(0)); // swap the pouch with a sword
        inv.cursor = None;
        inv.quick_move(Slot::Hotbar(0));
        assert!(inv.bags[0].is_some() && inv.hotbar[0].is_none());
    }

    #[test]
    fn consuming_takes_from_the_held_slot_first() {
        let mut inv = Inventory::starter();
        inv.add(Item::Torch, 999);
        inv.select(3);
        let before = inv.hotbar[3].unwrap().count;
        inv.consume(Item::Torch);
        assert_eq!(inv.hotbar[3].unwrap().count, before - 1);
        inv.select(0);
        inv.consume(Item::Torch);
        assert_eq!(
            inv.hotbar[3].unwrap().count,
            before - 2,
            "falls back to the first torch stack"
        );
    }
}
