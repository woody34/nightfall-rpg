//! Canonical item identities and storage scalars (Phase 4 design §§1.1, 3.1, 9.1).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::Tick;

/// Production reservation width; the low block is reserved exclusively for fixtures.
pub const ITEM_ID_RANGE_WIDTH: u64 = 1_u64 << 32;

/// Invalid item state is rejected before a draft can spend items or allocate identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("invalid item state: {0}")]
pub struct ItemError(pub &'static str);

/// Positive, signed-bigint-safe identity. Never drawn from the combat RNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct ItemInstanceId(u64);

impl ItemInstanceId {
    /// Validate imported identities as well as locally allocated ones.
    pub const fn new(value: u64) -> Result<Self, ItemError> {
        if value == 0 || value > i64::MAX as u64 {
            Err(ItemError("instance ID outside positive bigint"))
        } else {
            Ok(Self(value))
        }
    }

    /// Database and wire representation.
    pub const fn get(self) -> u64 {
        self.0
    }
}
impl TryFrom<u64> for ItemInstanceId {
    type Error = ItemError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<ItemInstanceId> for u64 {
    fn from(value: ItemInstanceId) -> Self {
        value.0
    }
}

/// An externally reserved range. Ownership is established by recorded admission; carried
/// items may belong to older ranges. `next == end` is valid exhaustion, never an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RangeFields", into = "RangeFields")]
pub struct ItemInstanceRange {
    range_start: u64,
    next_item_instance: u64,
    range_end_exclusive: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeFields {
    range_start: u64,
    next_item_instance: u64,
    range_end_exclusive: u64,
}

impl ItemInstanceRange {
    /// Accept only signed-bigint-safe endpoints (D8/D19).
    pub const fn new(start: u64, next: u64, end: u64) -> Result<Self, ItemError> {
        if start == 0 || start >= end || next < start || next > end || end > i64::MAX as u64 {
            return Err(ItemError("invalid allocator range"));
        }
        Ok(Self {
            range_start: start,
            next_item_instance: next,
            range_end_exclusive: end,
        })
    }

    /// Recorded inclusive start, next allocation, exclusive end.
    pub const fn bounds(self) -> (u64, u64, u64) {
        (self.range_start, self.next_item_instance, self.range_end_exclusive)
    }

    /// Allocate one identity. Failed exhaustion leaves the cursor unchanged. Multi-item
    /// operations allocate on their staged draft, then commit the entire plan together.
    pub fn allocate(&mut self) -> Result<ItemInstanceId, ItemError> {
        if self.next_item_instance == self.range_end_exclusive {
            return Err(ItemError("item allocator exhausted"));
        }
        let id = ItemInstanceId::new(self.next_item_instance)?;
        self.next_item_instance = self
            .next_item_instance
            .checked_add(1)
            .ok_or(ItemError("item allocator exhausted"))?;
        Ok(id)
    }

    /// Apply a recorded replacement reservation only after exhausting this range. The API
    /// proves global nonoverlap; monotonicity and absence of skipped IDs are checked here.
    pub fn extend(&mut self, replacement: Self) -> Result<(), ItemError> {
        if self.next_item_instance != self.range_end_exclusive
            || replacement.range_start < self.range_end_exclusive
            || replacement.next_item_instance != replacement.range_start
        {
            return Err(ItemError("invalid allocator extension"));
        }
        *self = replacement;
        Ok(())
    }
}
impl TryFrom<RangeFields> for ItemInstanceRange {
    type Error = ItemError;
    fn try_from(value: RangeFields) -> Result<Self, Self::Error> {
        Self::new(value.range_start, value.next_item_instance, value.range_end_exclusive)
    }
}
impl From<ItemInstanceRange> for RangeFields {
    fn from(value: ItemInstanceRange) -> Self {
        Self {
            range_start: value.range_start,
            next_item_instance: value.next_item_instance,
            range_end_exclusive: value.range_end_exclusive,
        }
    }
}

/// Source paperdoll slots, Inventory.java:54–79. Wire values are these ordinals plus one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
#[allow(missing_docs)]
pub enum EquipSlot {
    Under,
    Head,
    Hair,
    Hair2,
    Neck,
    Rhand,
    Chest,
    Lhand,
    Rear,
    Lear,
    Gloves,
    Legs,
    Feet,
    Rfinger,
    Lfinger,
    Lbracelet,
    Rbracelet,
    Deco1,
    Deco2,
    Deco3,
    Deco4,
    Deco5,
    Deco6,
    Cloak,
    Belt,
}

/// Only carried and equipped storage is admitted in Phase 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemLocation {
    /// Carried, including quest-pool templates.
    Inventory,
    /// Equipped; its slot is also present in the paperdoll index.
    Paperdoll,
}

/// Immutable clock namespace of an item's creation; retained items can cross epochs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemCreationOrigin {
    pub zone: super::ZoneId,
    pub epoch: u64,
}
impl ItemCreationOrigin {
    /// Provenance must fit the persisted positive zone/epoch namespace.
    pub fn validate(self) -> Result<(), ItemError> {
        if self.zone.0 == 0 || self.epoch == 0 || i64::try_from(self.epoch).is_err() {
            return Err(ItemError("invalid item creation origin"));
        }
        Ok(())
    }
}

/// Persistent scalar item record. SA is the template ID, not an independent modifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemInstance {
    /// Absent only for historical inputs; persistence resolves their recorded row origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_origin: Option<ItemCreationOrigin>,
    pub id: ItemInstanceId,
    pub template_id: u32,
    pub count: u64,
    pub location: ItemLocation,
    pub slot: Option<EquipSlot>,
    pub enchant: u32,
    pub augment_id: Option<u32>,
    pub charges: u8,
    pub created_tick: Tick,
    pub revision: u64,
}
impl ItemInstance {
    /// Structural checks shared by admission and checkpoints. Template compatibility and
    /// option references additionally require the immutable item rules.
    pub fn validate(&self) -> Result<(), ItemError> {
        if let Some(origin) = self.creation_origin {
            origin.validate()?;
        }
        if self.template_id == 0 || self.template_id > i32::MAX as u32 {
            return Err(ItemError("invalid item template ID"));
        }
        if self.count == 0 || self.count > i64::MAX as u64 {
            return Err(ItemError("item count outside positive bigint"));
        }
        if (self.location == ItemLocation::Paperdoll) != self.slot.is_some() {
            return Err(ItemError("item location and slot disagree"));
        }
        if self.charges & !7 != 0 || (self.charges != 0 && self.slot != Some(EquipSlot::Rhand)) {
            return Err(ItemError("invalid weapon charge bits"));
        }
        if self
            .augment_id
            .is_some_and(|id| id.trailing_zeros() >= 16 || id >> 16 == 0)
        {
            return Err(ItemError("invalid augmentation option pair"));
        }
        if self.created_tick.0 > i64::MAX as u64 || self.revision > i64::MAX as u64 {
            return Err(ItemError("item tick or revision outside bigint"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn creation_origin_is_optional_only_for_historical_scalars_and_validates_its_namespace() {
        let original = serde_json::json!({"id":1,"template_id":57,"count":1,
            "location":"inventory","slot":null,"enchant":0,"augment_id":null,
            "charges":0,"created_tick":900,"revision":3});
        let mut item: ItemInstance = serde_json::from_value(original.clone()).unwrap();
        item.validate().unwrap();
        assert_eq!(item.creation_origin, None);
        assert_eq!(serde_json::to_value(&item).unwrap(), original);
        let valid = ItemCreationOrigin {
            zone: super::super::ZoneId(7),
            epoch: 2,
        };
        item.creation_origin = Some(valid);
        item.validate().unwrap();
        let encoded = serde_json::to_value(&item).unwrap();
        assert_eq!(encoded.get("creation_origin"), Some(&serde_json::json!({"zone":7,"epoch":2})));
        assert_eq!(serde_json::from_value::<ItemInstance>(encoded).unwrap(), item);
        for origin in [
            ItemCreationOrigin {
                zone: super::super::ZoneId(0),
                ..valid
            },
            ItemCreationOrigin { epoch: 0, ..valid },
            ItemCreationOrigin {
                epoch: u64::MAX,
                ..valid
            },
        ] {
            item.creation_origin = Some(origin);
            assert!(item.validate().is_err());
        }
        assert!(serde_json::from_value::<ItemCreationOrigin>(
            serde_json::json!({"zone":7,"epoch":2,"tick":900})
        )
        .is_err());
        assert!(
            serde_json::from_value::<ItemCreationOrigin>(serde_json::json!({"zone":7})).is_err()
        );
    }

    #[test]
    fn signed_bigint_endpoints_exhaustion_and_extensions_are_strict() {
        assert!(ItemInstanceId::new(0).is_err());
        assert!(ItemInstanceId::new(1_u64 << 63).is_err());
        assert!(ItemInstanceId::new(i64::MAX as u64).is_ok());
        assert!(ItemInstanceRange::new(1, 1, 1_u64 << 63).is_err());
        let mut range = ItemInstanceRange::new(1, 1, 3).unwrap();
        assert!(range
            .extend(ItemInstanceRange::new(3, 3, 5).unwrap())
            .is_err());
        assert_eq!(range.allocate().unwrap().get(), 1);
        assert_eq!(range.allocate().unwrap().get(), 2);
        let before = range;
        assert!(range.allocate().is_err());
        assert_eq!(range, before);
        assert!(range
            .extend(ItemInstanceRange::new(2, 2, 4).unwrap())
            .is_err());
        range
            .extend(ItemInstanceRange::new(3, 3, 5).unwrap())
            .unwrap();
        assert_eq!(range.allocate().unwrap().get(), 3);
        let encoded = serde_json::to_vec(&range).unwrap();
        assert_eq!(serde_json::from_slice::<ItemInstanceRange>(&encoded).unwrap(), range);
        assert!(serde_json::from_str::<ItemInstanceRange>(
            r#"{"range_start":1,"next_item_instance":0,"range_end_exclusive":3}"#
        )
        .is_err());
        assert!(serde_json::from_str::<ItemInstanceId>("0").is_err());
    }

    proptest! {
        #[test]
        fn allocation_is_monotone_or_atomic(start in 1_u64..=1_000_000, width in 1_u64..1000) {
            let end = start.checked_add(width).unwrap();
            let mut range = ItemInstanceRange::new(start, start, end).unwrap();
            for expected in start..end {
                prop_assert_eq!(range.allocate().unwrap().get(), expected);
            }
            let exhausted = range;
            prop_assert!(range.allocate().is_err());
            prop_assert_eq!(range, exhausted);
        }
    }
}
