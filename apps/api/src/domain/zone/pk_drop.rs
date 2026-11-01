//! PK (chaotic) drop planning (design §3.9, F3, R13, E24, E53): the pinned `onDieDropItem`
//! (`l2j-server-game` abfde049, `model/actor/instance/L2PcInstance.java:4473-4553`) as a pure
//! planner over a draw source. It decides the gate, which items are unequipped (before their
//! roll, whether or not they drop), which drop, and their offsets; applying the plan to the
//! inventory and the ground is a B13 `transfer_out → ground` pair settled by the V7 ledger
//! planner (story 3.12), never a V6 removal.
//!
//! Branches: the karma branch applies iff `karma > 0 ∧ pk_kills ≥ MinimumPKRequiredToDrop`
//! (`:4496`) for **any** non-null killer, NPCs included; otherwise the NPC-killer branch uses
//! `PlayerRateDrop = 0` (`:4503`), whose gate `dropPercent > 0` is false: no draw. The clan-war
//! exemption (`:4479`, karma ≤ 0 only) and the PvP-zone/GM guard (`:4485`) are vacuous.

use super::drops::{DropDraws, DropError};
use super::item_rules::ItemDefinition;
use super::phase5_rules::PkPolicy;
use super::wide::chance_bound;
use super::{ItemInstance, ItemInstanceId, ItemLocation};

/// `Rnd.get(50) − 25` (`L2PcInstance.dropItem`, `:3178`): offsets in `[−25, 24]` L2 units.
const PK_DROP_SPREAD: u32 = 50;
const PK_DROP_OFFSET: i32 = 25;

/// One inventory item as the PK loop sees it, in ascending `ItemInstanceId` order (X4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PkCandidate {
    /// The instance.
    pub instance: ItemInstanceId,
    /// One of the pinned exclusions (`:4524-4531`): skipped with no draw and no unequip.
    pub excluded: bool,
    /// Equipped (`isEquipped()`): unequipped before its roll.
    pub equipped: bool,
    /// `getType2() == WEAPON` (selects `KarmaRateDropEquipWeapon` when equipped).
    pub weapon: bool,
}

/// What the PK loop decided, in pinned order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PkPlan {
    /// The `Rnd.get(100) < KarmaRateDrop` gate was drawn and its outcome.
    pub gate: Option<bool>,
    /// Equipped candidates unequipped before their roll (dropped or not).
    pub unequipped: Vec<ItemInstanceId>,
    /// Dropped instances with their `(dx, dy)` offsets in L2 units.
    pub dropped: Vec<(ItemInstanceId, i32, i32)>,
}

/// The pinned exclusions of one instance (design §3.9 `excluded`): adena, `NonDroppableItems`,
/// `PetItems`, augmented, shadow (`duration ≥ 0`), time-limited (`time ≠ −1`), `!isDroppable`
/// and `type2 == QUEST`. The lists are read first, so a listed template needs no definition;
/// any other template must have one (fail closed).
pub fn pk_excluded(
    policy: &PkPolicy,
    item: &ItemInstance,
    definition: Option<&ItemDefinition>,
) -> Result<bool, DropError> {
    let template = item.template_id;
    if template == 57
        || policy.non_droppable_items.contains(&template)
        || policy.pet_items.contains(&template)
        || item.augment_id.is_some()
    {
        return Ok(true);
    }
    let definition =
        definition.ok_or(DropError::Rules("PK candidate without an item definition"))?;
    let number = |key: &str| -> Result<i64, DropError> {
        definition
            .properties
            .get(key)
            .map_or(Ok(-1), |value| value.parse().map_err(|_| DropError::Rules("item property")))
    };
    let flag = |key: &str, default: bool| definition.flags.get(key).copied().unwrap_or(default);
    Ok(number("duration")? >= 0
        || number("time")? != -1
        || !flag("is_droppable", true)
        || (definition.kind == "EtcItem" && flag("is_questitem", false)))
}

/// The loop's candidate view of an instance (`isEquipped()`, `type2 == WEAPON`).
#[must_use]
pub fn pk_candidate(
    item: &ItemInstance,
    definition: Option<&ItemDefinition>,
    excluded: bool,
) -> PkCandidate {
    PkCandidate {
        instance: item.id,
        excluded,
        equipped: item.location == ItemLocation::Paperdoll,
        weapon: definition.is_some_and(|d| d.kind == "Weapon"),
    }
}

/// Plan the PK drop of one death (design §3.9). `candidates` must be in ascending
/// `ItemInstanceId` order. `killer` is `false` when the death has no killer (no draw).
pub fn plan_pk_drop(
    policy: &PkPolicy,
    karma: u32,
    pk_kills: u32,
    killer: bool,
    candidates: &[PkCandidate],
    draws: &mut impl DropDraws,
) -> Result<PkPlan, DropError> {
    let mut plan = PkPlan::default();
    let karma_drop = karma > 0 && u64::from(pk_kills) >= policy.minimum_pk_required_to_drop;
    // The NPC-killer branch's dropPercent is the hashed PlayerRateDrop, which
    // `Phase5Rules::try_new` pins to 0 (z-review-3 P3-3): the gate is never drawn.
    if !killer || !karma_drop {
        return Ok(plan);
    }
    if candidates
        .windows(2)
        .any(|pair| pair.first().map(|a| a.instance) >= pair.get(1).map(|b| b.instance))
    {
        return Err(DropError::Rules("PK candidates out of ItemInstanceId order"));
    }
    let gate = chance_bound(policy.karma_rate_drop, 100, draws.draw_pair())?;
    plan.gate = Some(gate);
    if !gate {
        return Ok(plan);
    }
    for candidate in candidates.iter().filter(|c| !c.excluded) {
        let percent = if candidate.equipped {
            plan.unequipped.push(candidate.instance);
            if candidate.weapon {
                policy.karma_rate_drop_equip_weapon
            } else {
                policy.karma_rate_drop_equip
            }
        } else {
            policy.karma_rate_drop_item
        };
        if chance_bound(percent, 100, draws.draw_pair())? {
            let dx = i32::try_from(draws.draw_below(PK_DROP_SPREAD))
                .unwrap_or(0)
                .saturating_sub(PK_DROP_OFFSET);
            let dy = i32::try_from(draws.draw_below(PK_DROP_SPREAD))
                .unwrap_or(0)
                .saturating_sub(PK_DROP_OFFSET);
            plan.dropped.push((candidate.instance, dx, dy));
            if u64::try_from(plan.dropped.len()).unwrap_or(u64::MAX) >= policy.karma_drop_limit {
                break;
            }
        }
    }
    Ok(plan)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::similar_names,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    /// The oracle's `Words(seed)` stream (xorshift64*, `economy_oracle.py`).
    struct Xorshift(u64);

    impl Xorshift {
        fn word(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            u32::try_from(x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32).unwrap()
        }
    }

    impl DropDraws for Xorshift {
        fn draw_pair(&mut self) -> u64 {
            let hi = self.word();
            let lo = self.word();
            (u64::from(hi) << 32) | u64::from(lo)
        }

        fn draw_below(&mut self, n: u32) -> u32 {
            let n = u64::from(n.max(1));
            let limit = (1_u64 << 32) - (1_u64 << 32) % n;
            loop {
                let v = u64::from(self.word());
                if v < limit {
                    return u32::try_from(v % n).unwrap();
                }
            }
        }
    }

    fn policy() -> PkPolicy {
        crate::infrastructure::economy_data::embedded_phase5_rules()
            .unwrap()
            .parts()
            .ground
            .pk
            .clone()
    }

    fn instance(id: u64, template: u32) -> ItemInstance {
        ItemInstance {
            creation_origin: None,
            id: ItemInstanceId::new(id).unwrap(),
            template_id: template,
            count: 1,
            location: ItemLocation::Inventory,
            slot: None,
            enchant: 0,
            augment_id: None,
            charges: 0,
            created_tick: super::super::Tick(0),
            revision: 1,
        }
    }

    /// Family D-P on actual inputs: the oracle's 10-drop vector, draw for draw.
    #[test]
    fn the_pk_loop_reproduces_the_oracle_break_at_ten_vector() {
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/economy-oracle-drops.json"
        ))
        .unwrap();
        let vector = &oracle["dp_vectors"]["break_at_10"][0];
        let items = crate::infrastructure::item_data::embedded_phase4_bundle()
            .unwrap()
            .items;
        let policy = policy();
        let plain: Vec<u32> = vector["inventory_templates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| u32::try_from(t.as_u64().unwrap()).unwrap())
            .collect();
        let shadow = u32::try_from(vector["shadow"].as_u64().unwrap()).unwrap();
        let timed = u32::try_from(vector["time_limited"].as_u64().unwrap()).unwrap();
        // The oracle's synthetic inventory: equipped = i % 3 == 2, "weapon" = i % 6 == 5.
        let mut rows: Vec<(ItemInstance, bool, bool)> = plain
            .iter()
            .enumerate()
            .map(|(i, t)| (instance(u64::try_from(i).unwrap() + 1, *t), i % 3 == 2, i % 6 == 5))
            .collect();
        let mut augmented = instance(53, plain[0]);
        augmented.augment_id = Some(0x0001_0001);
        rows.extend([
            (instance(50, 57), false, false),
            (instance(51, 7694), false, false),
            (instance(52, 2375), false, false),
            (augmented, false, false),
            (instance(54, shadow), true, false),
            (instance(55, timed), false, false),
        ]);
        let candidates: Vec<PkCandidate> = rows
            .iter()
            .map(|(item, equipped, weapon)| {
                let definition = items.parts().items.get(&item.template_id);
                PkCandidate {
                    instance: item.id,
                    excluded: pk_excluded(&policy, item, definition).unwrap(),
                    equipped: *equipped,
                    weapon: *weapon,
                }
            })
            .collect();
        assert!(
            candidates[24..].iter().all(|c| c.excluded),
            "adena, lists, augmented, shadow, timed"
        );
        assert!(candidates[..24].iter().all(|c| !c.excluded));
        let seed = vector["seed"].as_u64().unwrap();
        let plan = plan_pk_drop(&policy, 10, 6, true, &candidates, &mut Xorshift(seed)).unwrap();
        let dropped: Vec<(u64, i64, i64)> = plan
            .dropped
            .iter()
            .map(|(id, dx, dy)| (id.get(), i64::from(*dx), i64::from(*dy)))
            .collect();
        let expected: Vec<(u64, i64, i64)> = vector["dropped"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d[0].as_u64().unwrap(), d[1].as_i64().unwrap(), d[2].as_i64().unwrap()))
            .collect();
        assert_eq!(plan.gate, Some(true));
        assert_eq!(dropped, expected);
        let unequipped: Vec<u64> = vector["trace"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|step| step[0] == "unequip")
            .map(|step| step[1].as_u64().unwrap())
            .collect();
        assert_eq!(
            plan.unequipped
                .iter()
                .map(|id| id.get())
                .collect::<Vec<_>>(),
            unequipped
        );
        // Items after the 10th drop stay equipped; the excluded shadow item is never unequipped.
        assert!(!plan.unequipped.iter().any(|id| id.get() == 54));
        assert!(plan.unequipped.iter().all(|id| id.get() <= dropped[9].0));
    }

    #[test]
    fn branches_follow_the_pinned_conditions() {
        let policy = policy();
        let one = [PkCandidate {
            instance: ItemInstanceId::new(1).unwrap(),
            excluded: false,
            equipped: false,
            weapon: false,
        }];
        // pk_kills < 6, karma 0, or no killer: no draw at all.
        for (karma, pk, killer) in [(10, 5, true), (0, 9, true), (10, 9, false)] {
            let mut draws = Xorshift(9);
            let plan = plan_pk_drop(&policy, karma, pk, killer, &one, &mut draws).unwrap();
            assert_eq!(plan, PkPlan::default());
            assert_eq!(draws.0, 9, "no word drawn");
        }
        // Karma ∧ pk ≥ 6 applies to ANY killer (an NPC killer included): the gate is drawn.
        let plan = plan_pk_drop(&policy, 10, 6, true, &one, &mut Xorshift(9)).unwrap();
        assert!(plan.gate.is_some());
        assert!(plan
            .dropped
            .iter()
            .all(|(_, dx, dy)| (-25..=24).contains(dx) && (-25..=24).contains(dy)));
        // Out-of-order candidates fail closed.
        let reversed = [
            one[0],
            PkCandidate {
                instance: ItemInstanceId::new(1).unwrap(),
                ..one[0]
            },
        ];
        assert!(plan_pk_drop(&policy, 10, 6, true, &reversed, &mut Xorshift(9)).is_err());
    }
}
