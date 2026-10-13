//! V6 bow callers: equipped reach, accepted-start arrow/MP payment and bow timing
//! (design §5.4; L2Character.java:711-758,1007-1045,4335-4357).
use super::*;
use crate::domain::zone::item_equipment::PaperdollMutation;
use crate::domain::zone::item_inventory::{
    InventoryBoundary, InventoryEdit, InventoryLimits, InventoryMutation,
};
use crate::domain::zone::phase4_projection::{
    ItemAuditEvidence, ItemDeltaPlan, ItemEvidenceValue as V, ItemOperation,
};
use crate::domain::zone::{EquipSlot, ItemInstanceId, ItemLocation};

/// Outcome of the bow guard at an accepted swing start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::domain::zone) enum BowStart {
    /// Not a bow attack: ordinary melee timing.
    Melee,
    /// Paid; the swing lands at `impact_ticks` and the next may start after `cycle_ticks`.
    Fire { impact_ticks: u64, cycle_ticks: u64 },
    /// Not enough MP: retry on a later tick (`NotifyAITask` after 100ms).
    Wait,
    /// No usable arrows: the attack intention ends (`AI_INTENTION_IDLE`).
    Stop,
}

fn l2_reach(range: i128) -> Result<Fixed, TickError> {
    let units = range
        .checked_mul(i128::from(crate::domain::zone::fixed::UNITS_PER_TILE))
        .and_then(|v| v.checked_div(i128::from(crate::domain::zone::combat::L2_UNITS_PER_TILE)))
        .ok_or_else(missing)?;
    i32::try_from(units).map(Fixed::from_raw).map_err(invariant)
}

/// Integer square root (floor) for the exact bow distance multiplier.
fn isqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut x = value;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = x.saturating_add(value.checked_div(x).unwrap_or(0)) >> 1;
    }
    x
}

impl ZoneState {
    pub(super) fn held_weapon_v6(
        &self,
        id: EntityId,
    ) -> Option<(ItemInstanceId, &crate::domain::zone::item_rules::ItemDefinition)> {
        let items = self.phase4.as_ref()?;
        let player = items.canonical.players.get(&id)?;
        let weapon = *player.paperdoll.get(&EquipSlot::Rhand)?;
        let held = player.inventory.get(&weapon)?;
        Some((weapon, items.rules.parts().items.get(&held.template_id)?))
    }

    /// CharStat.java:424-436 physical attack range: the active weapon's base range — the
    /// equipped weapon, else the class fists weapon (D84, 40) — else the current class
    /// template's `baseAtkRange` (D75: 20, orcs 25), then the integer `pAtkRange` getter.
    pub(in crate::domain::zone) fn attack_range_v6(
        &self,
        id: EntityId,
        effective: &EffectiveStats,
    ) -> Result<Option<Fixed>, TickError> {
        if self
            .phase4
            .as_ref()
            .is_none_or(|items| !items.canonical.players.contains_key(&id))
        {
            return Ok(None);
        }
        let base = match self.held_weapon_v6(id) {
            Some((_, definition)) => definition.attack_range,
            None => self.unarmed_range_v6(id)?,
        };
        let range = self
            .damage_stat_v5(
                id,
                effective,
                "pAtkRange",
                i64::from(base).checked_mul(Q).ok_or_else(missing)?,
            )?
            .trunc()
            .map_err(invariant)?;
        Ok(Some(l2_reach(range.max(0))?))
    }

    /// D75: the player's current-class unarmed range. Every descendant shares its base
    /// class value (E1 generation census), so the row is found by lineage membership.
    fn unarmed_range_v6(&self, id: EntityId) -> Result<u32, TickError> {
        let combat = self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?;
        let CombatRole::Player {
            progression: Some(progression),
            ..
        } = &combat.role
        else {
            return Err(missing());
        };
        let current = progression.class_state.current_class_id.0;
        let parts = self.phase4.as_ref().ok_or_else(missing)?.rules.parts();
        // D84: getActiveWeaponItem() falls back to the class fists weapon (range 40); the
        // template baseAtkRange is reached only for classes without one (3rd class/Kamael).
        if let Some(fists) = parts.fists_weapon(current) {
            return Ok(fists.attack_range);
        }
        parts
            .class_combat
            .iter()
            .find(|row| row.lineage_class_ids.contains(&current))
            .map(|row| row.attack_range)
            .ok_or_else(missing)
    }

    /// D84/CharStat.java:438-447: the fists weapon's base angle, else the unarmed 120.
    pub(super) fn unarmed_angle_v6(&self, id: EntityId) -> Result<u32, TickError> {
        let parts = self.phase4.as_ref().ok_or_else(missing)?.rules.parts();
        Ok(self
            .unarmed_fists_v6(id)?
            .map_or(parts.unarmed_combat.attack_angle, |fists| fists.attack_angle))
    }

    /// The current class fists weapon (D84), if the class has one.
    pub(super) fn unarmed_fists_v6(
        &self,
        id: EntityId,
    ) -> Result<Option<&crate::domain::zone::item_rules::ItemDefinition>, TickError> {
        let combat = self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?;
        let CombatRole::Player {
            progression: Some(progression),
            ..
        } = &combat.role
        else {
            return Ok(None);
        };
        Ok(self
            .phase4
            .as_ref()
            .ok_or_else(missing)?
            .rules
            .parts()
            .fists_weapon(progression.class_state.current_class_id.0))
    }

    /// Whether the attacker's equipped weapon is a bow (distance falloff and the shield
    /// ×1.3 apply to BOW only; crossbows share the ranged start but not those terms).
    pub(in crate::domain::zone) fn bow_attacker_v6(&self, id: EntityId) -> bool {
        self.held_weapon_v6(id)
            .is_some_and(|(_, definition)| definition.weapon_type == "BOW")
    }

    /// canUseRangeWeapon gate: BOW and CROSSBOW are range weapons. The attack-time refill
    /// `checkAndEquipArrows` (L2PcInstance.java:5334-5352) always uses `findArrowForBow`
    /// (Inventory.java:679-694), which matches same-S-grade ARROWs only, for both kinds
    /// (D85); bolts are selected only on the equip/pickup paths (`findBoltForCrossBow`,
    /// Inventory.java:702-713).
    fn ranged_ammo_v6(&self, id: EntityId) -> Option<&'static str> {
        self.held_weapon_v6(id)
            .and_then(|(_, definition)| match definition.weapon_type.as_str() {
                "BOW" | "CROSSBOW" => Some("ARROW"),
                _ => None,
            })
    }

    /// canUseRangeWeapon + doAttackHitByBow/CrossBow at an accepted start: arrows are
    /// auto-selected when the left hand is empty (D85), the optional reduced-MP draw happens before the MP check,
    /// MP and one arrow are paid before the hit (an arrow is spent even on a miss), and bow
    /// timing replaces the melee half-interval impact.
    #[allow(clippy::too_many_lines)] // Source guard order and one owner settlement.
    pub(in crate::domain::zone) fn bow_start_v6(
        &mut self,
        tick: Tick,
        id: EntityId,
        initial_revision: Option<u64>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<BowStart, TickError> {
        let Some(ammo_type) = self.ranged_ammo_v6(id) else {
            return Ok(BowStart::Melee);
        };
        let (_, definition) = self.held_weapon_v6(id).ok_or_else(missing)?;
        let definition = definition.clone();
        let items = self.phase4.as_ref().ok_or_else(missing)?;
        let player = items.canonical.players.get(&id).ok_or_else(missing)?;
        // checkAndEquipArrows: keep a held arrow, else the lowest matching inventory arrow.
        let held_arrow = player.paperdoll.get(&EquipSlot::Lhand).copied();
        let candidate = if held_arrow.is_some() {
            None
        } else {
            let grade = items
                .rules
                .parts()
                .grades
                .iter()
                .find(|grade| grade.grade == definition.grade)
                .ok_or_else(missing)?;
            player
                .inventory
                .values()
                .find(|item| {
                    item.location == ItemLocation::Inventory
                        && items
                            .rules
                            .parts()
                            .items
                            .get(&item.template_id)
                            .is_some_and(|ammo| {
                                ammo.etcitem_type == ammo_type
                                    && items
                                        .rules
                                        .parts()
                                        .grades
                                        .iter()
                                        .find(|g| g.grade == ammo.grade)
                                        .is_some_and(|g| g.shot_grade == grade.shot_grade)
                            })
                })
                .map(|item| item.id)
        };
        if held_arrow.is_none() && candidate.is_none() {
            return Ok(BowStart::Stop);
        }
        // Reduced MP: Rnd.get(100) < chance only when a reduced cost exists.
        let mut cost = definition.mp_consume;
        if let [chance, reduced] = definition.reduced_mp_consume.as_slice() {
            if *reduced > 0 && self.roll_below(100) < *chance {
                cost = *reduced;
            }
        }
        let effective = self.effective(id)?;
        let cost = self
            .damage_stat_v5(
                id,
                &effective,
                "bowMpConsumeRate",
                i64::from(cost).checked_mul(Q).ok_or_else(missing)?,
            )?
            .trunc()
            .map_err(invariant)?;
        let cost = u32::try_from(cost.max(0)).map_err(invariant)?;
        let combat = self
            .entities
            .get(&id)
            .and_then(|entity| entity.combat.as_ref())
            .ok_or_else(missing)?;
        let remainder = self.sidecar(id)?.fractions.mp;
        let available = i128::from(combat.mp)
            .checked_mul(i128::from(Q))
            .and_then(|mp| mp.checked_add(i128::from(remainder)))
            .ok_or_else(missing)?;
        if available
            < i128::from(cost)
                .checked_mul(i128::from(Q))
                .ok_or_else(missing)?
        {
            return Ok(BowStart::Wait);
        }
        let (previous, revision) = self.attack_projection_v6(tick, id, initial_revision, events)?;
        let original_revision = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&id))
            .ok_or_else(missing)?
            .inventory_revision;
        let mut projection = match previous.clone() {
            Some(projection) => projection,
            None => self.empty_projection_v6(tick, id)?,
        };
        let subject = self.binding(id).ok_or_else(missing)?;
        let resolution = format!("attack:{id}:{}", tick.0);
        let boundary = InventoryBoundary {
            tick,
            revision,
            origin: crate::domain::zone::ItemCreationOrigin {
                zone: self.seed.zone,
                epoch: self.seed.epoch,
            },
        };
        let items = self.phase4.as_ref().ok_or_else(missing)?;
        let mut player = items
            .canonical
            .players
            .get(&id)
            .ok_or_else(missing)?
            .clone();
        let mut changes = Vec::new();
        let mut next_sequence = None;
        let arrow = if let Some(candidate) = candidate {
            let equip = PaperdollMutation::install_ammunition(
                &player.inventory,
                &player.paperdoll,
                candidate,
                &items.rules,
                revision,
            )
            .map_err(invariant)?;
            // The installed arrow becomes an equipment provider like any equip (D85).
            let sequence = self
                .phase3
                .as_ref()
                .ok_or_else(missing)?
                .canonical
                .next_install_sequence;
            let (staged, next) = crate::domain::zone::item_sources::apply_paperdoll(
                &player,
                &equip,
                &items.rules,
                sequence,
            )
            .map_err(invariant)?;
            player = staged;
            next_sequence = Some(next);
            for change in equip.changes {
                let ordinal = projection.next_operation_ordinal().map_err(invariant)?;
                let instance = change
                    .after
                    .as_ref()
                    .or(change.before.as_ref())
                    .ok_or_else(missing)?
                    .id;
                projection.ledger.push(ItemDeltaPlan {
                    operation: ItemOperation::Equip,
                    operation_ordinal: ordinal,
                    instance,
                    before: change.before.clone(),
                    after: change.after.clone(),
                    count_delta: 0,
                    receipt: None,
                    receipt_refs: Vec::new(),
                    causes: Vec::new(),
                });
                changes.push((change, ItemOperation::Equip, ordinal));
            }
            candidate
        } else {
            held_arrow.ok_or_else(missing)?
        };
        let payment = InventoryMutation::prepare(
            &player.inventory,
            items.canonical.allocator,
            &[InventoryEdit::TakeInstance {
                instance: arrow,
                count: 1,
            }],
            &items.rules,
            InventoryLimits::from_player(&player),
            boundary,
        )
        .map_err(invariant)?;
        player.inventory = payment.inventory;
        if !player.inventory.contains_key(&arrow) {
            // Exhausted arrows leave the left hand (and the provider list) before the
            // weight/penalty rebuild.
            player.paperdoll.remove(&EquipSlot::Lhand);
            let mut sequence = match next_sequence {
                Some(next) => next,
                None => {
                    self.phase3
                        .as_ref()
                        .ok_or_else(missing)?
                        .canonical
                        .next_install_sequence
                },
            };
            crate::domain::zone::item_sources::reconcile_sources(
                &mut player,
                &items.rules,
                &mut sequence,
            )
            .map_err(invariant)?;
            next_sequence = Some(sequence);
        }
        player.inventory_revision = revision;
        let template = definition.template_id;
        for change in payment.changes {
            let cause = ItemAuditEvidence {
                family: "arrow_payment".into(),
                fields: BTreeMap::from([
                    ("count_delta".into(), V::Signed(-1)),
                    ("resolution_id".into(), V::Text(resolution.clone())),
                    ("weapon".into(), V::Unsigned(u64::from(template))),
                ]),
            };
            changes.push((change.clone(), ItemOperation::Consume, 0));
            super::super::super::item_commands::append_item_consumption(
                &mut projection,
                change,
                -1,
                cause,
            )
            .map_err(invariant)?;
        }
        let combat = self
            .entities
            .get_mut(&id)
            .and_then(|entity| entity.combat.as_mut())
            .ok_or_else(missing)?;
        combat.mp = combat.mp.checked_sub(cost).ok_or_else(missing)?;
        if let Some(next) = next_sequence {
            self.phase3
                .as_mut()
                .ok_or_else(missing)?
                .canonical
                .next_install_sequence = next;
        }
        self.phase4
            .as_mut()
            .ok_or_else(missing)?
            .canonical
            .players
            .insert(id, player);
        for (change, operation, ordinal) in &changes {
            self.item_evidence_delta(subject, change, *operation, *ordinal, None, &resolution);
        }
        self.publish_attack_projection_v6(
            tick,
            id,
            previous.is_some(),
            projection,
            original_revision,
            events,
        )?;
        // Timing: attack = trunc(500000/pAtkSpd) (rounded speed, as the melee path), impact at
        // the full interval, next start after attack + trunc(reuse_delay×333/pAtkSpd).
        let speed = i128::from(
            self.entities
                .get(&id)
                .and_then(|entity| entity.combat.as_ref())
                .ok_or_else(missing)?
                .sheet
                .attack_speed()
                .raw(),
        );
        let speed =
            crate::domain::zone::scaled::round_div(speed, i128::from(Q)).map_err(invariant)?;
        if speed <= 0 {
            return Err(missing());
        }
        let attack_ms = 500_000_i128.checked_div(speed).ok_or_else(missing)?;
        let reuse_ms = i128::from(definition.reuse_delay)
            .checked_mul(333)
            .and_then(|v| v.checked_div(speed))
            .ok_or_else(missing)?;
        // D46 `bow_reuse` (D17): ordinary-weapon reuse from the rounded speed; the weapon
        // reuse modifier does not participate (L2Character.java:4353-4357).
        self.item_formula_v6(
            id,
            "bow_reuse",
            serde_json::json!({"reuse_delay": definition.reuse_delay,
                "attack_speed": i64::try_from(speed).map_err(invariant)?, "weapon_reuse_modifier": 1}),
            serde_json::json!(i64::try_from(reuse_ms).map_err(invariant)?),
            &format!("bow_reuse:{id}:{}", tick.0),
            Some(serde_json::json!({"weapon_template": definition.template_id, "event": "attack"})),
        );
        let tick_ms = i128::from(crate::domain::zone::TICK_MS);
        let ceil = |ms: i128| -> Result<u64, TickError> {
            let ticks = ms
                .checked_add(tick_ms.checked_sub(1).ok_or_else(missing)?)
                .and_then(|v| v.checked_div(tick_ms))
                .ok_or_else(missing)?;
            u64::try_from(ticks)
                .map(|ticks| ticks.max(1))
                .map_err(invariant)
        };
        Ok(BowStart::Fire {
            impact_ticks: ceil(attack_ms)?,
            cycle_ticks: ceil(attack_ms.checked_add(reuse_ms).ok_or_else(missing)?)?,
        })
    }

    /// L2Character.java:1029-1033: the caller truncates ordinary damage, then truncates its
    /// product with `distance/4000 + 4/5` (distance in L2 units, exact root, flat Z).
    pub(in crate::domain::zone) fn bow_distance_v6(
        &self,
        attacker: EntityId,
        target: EntityId,
        damage: u32,
    ) -> Result<u32, TickError> {
        let from = self.entities.get(&attacker).ok_or_else(missing)?.pos;
        let to = self.entities.get(&target).ok_or_else(missing)?.pos;
        let dx = i128::from(from.x.raw())
            .checked_sub(i128::from(to.x.raw()))
            .ok_or_else(missing)?;
        let dy = i128::from(from.y.raw())
            .checked_sub(i128::from(to.y.raw()))
            .ok_or_else(missing)?;
        let squared = u128::try_from(
            dx.checked_mul(dx)
                .and_then(|x| dy.checked_mul(dy).and_then(|y| x.checked_add(y)))
                .ok_or_else(missing)?,
        )
        .map_err(invariant)?;
        // distance_L2 = sqrt(S)×32/1000; result = floor(D×(sqrt(S)×32/4,000,000 + 4/5))
        //             = floor((4×D×M + isqrt(25×N²×S)) / (5×M)), N = 32×D, M = 4,000,000.
        let whole = u128::from(damage);
        let scale = 4_000_000_u128;
        let scaled = whole.checked_mul(32).ok_or_else(missing)?;
        let root = isqrt(
            scaled
                .checked_mul(scaled)
                .and_then(|v| v.checked_mul(25))
                .and_then(|v| v.checked_mul(squared))
                .ok_or_else(missing)?,
        );
        let numerator = whole
            .checked_mul(4)
            .and_then(|v| v.checked_mul(scale))
            .and_then(|v| v.checked_add(root))
            .ok_or_else(missing)?;
        let denominator = scale.checked_mul(5).ok_or_else(missing)?;
        u32::try_from(numerator.checked_div(denominator).ok_or_else(missing)?).map_err(invariant)
    }
}

#[cfg(test)]
mod tests {
    use super::isqrt;

    #[test]
    fn integer_square_root_is_the_exact_floor() {
        for value in [
            0_u128,
            1,
            2,
            3,
            4,
            15,
            16,
            17,
            24,
            25,
            26,
            1 << 64,
            (1 << 100) + 7,
        ] {
            let root = isqrt(value);
            assert!(root * root <= value && (root + 1) * (root + 1) > value, "{value}");
        }
    }
}
