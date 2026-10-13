//! D46 sidecar capture; no observation changes canonical state or consumes randomness.
use super::*;
use crate::domain::zone::effects::TargetBinding;
use crate::domain::zone::evidence::{ItemEvidenceData, ItemEvidenceFact, ItemEvidenceOrigin};
use crate::domain::zone::item_inventory::InventoryChange;
use crate::domain::zone::phase4_projection::{ItemOperation, ItemReceiptRef};
use serde_json::json;

impl ZoneState {
    /// Exact replay boundary cursor, without cloning a full snapshot.
    pub fn replay_rng_word_position(&self) -> u128 {
        self.rng.get_word_pos()
    }
    /// Enable the frozen V6 evidence stream and capture the starting inventories.
    pub fn enable_items_replay_evidence(&mut self) -> Result<(), TickError> {
        let owners: Vec<_> = self
            .phase4
            .as_ref()
            .ok_or(TickError::Phase3Invariant)?
            .canonical
            .players
            .keys()
            .copied()
            .collect();
        self.enable_replay_evidence();
        self.evidence
            .as_mut()
            .ok_or(TickError::Phase3Invariant)?
            .items = Some(Vec::new());
        for owner in owners {
            self.item_evidence_baseline(owner)?;
        }
        Ok(())
    }

    /// Drain only after the enclosing replay record has reproduced exactly.
    pub fn take_items_replay_evidence(&mut self) -> Vec<ItemEvidenceFact> {
        self.evidence.as_mut().map_or_else(Vec::new, |buffer| {
            buffer.facts.clear();
            buffer.draws.clear();
            buffer.items.as_mut().map_or_else(Vec::new, std::mem::take)
        })
    }

    pub(super) fn items_evidence_enabled(&self) -> bool {
        self.evidence
            .as_ref()
            .is_some_and(|buffer| buffer.items.is_some())
    }

    pub(super) fn item_evidence(&mut self, subject: TargetBinding, fact: ItemEvidenceData) {
        if let Some(facts) = self
            .evidence
            .as_mut()
            .and_then(|buffer| buffer.items.as_mut())
        {
            facts.push(ItemEvidenceFact {
                entity: subject.entity,
                incarnation: subject.incarnation,
                origin: ItemEvidenceOrigin::Live,
                tick: self.next_tick,
                fact,
            });
        }
    }

    /// The private candidate retains earlier observations; adopt only its new suffix.
    pub(super) fn adopt_offline_item_evidence(
        &mut self,
        candidate: &Self,
        owner: EntityId,
    ) -> Result<(), RejectReason> {
        let Some(facts) = self
            .evidence
            .as_mut()
            .and_then(|buffer| buffer.items.as_mut())
        else {
            return Ok(());
        };
        let suffix = candidate
            .evidence
            .as_ref()
            .and_then(|buffer| buffer.items.as_ref())
            .and_then(|rows| rows.get(facts.len()..))
            .ok_or(RejectReason::InvalidLoad)?;
        if suffix.iter().any(|row| row.entity != owner) {
            return Err(RejectReason::InvalidLoad);
        }
        facts.extend(suffix.iter().cloned().map(|mut row| {
            row.incarnation = 0;
            row.origin = ItemEvidenceOrigin::Offline;
            row
        }));
        Ok(())
    }

    pub(super) fn item_evidence_baseline(&mut self, owner: EntityId) -> Result<(), TickError> {
        if !self.items_evidence_enabled() {
            return Ok(());
        }
        let subject = self.binding(owner).ok_or(TickError::Phase3Invariant)?;
        let player = self
            .phase4
            .as_ref()
            .and_then(|phase| phase.canonical.players.get(&owner))
            .ok_or(TickError::Phase3Invariant)?;
        let data = json!({"inventory_revision": player.inventory_revision,
            "instances": player.inventory.values().collect::<Vec<_>>(),
            "paperdoll": player.paperdoll, "auto_shots": player.auto_shots});
        self.item_evidence(subject, ItemEvidenceData::Baseline(data));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Frozen operation identity accompanies its exact scalar change.
    pub(super) fn item_evidence_delta(
        &mut self,
        subject: TargetBinding,
        change: &InventoryChange,
        operation: ItemOperation,
        ordinal: u32,
        receipt: Option<ItemReceiptRef>,
        resolution: &str,
    ) {
        if !self.items_evidence_enabled() || change.before == change.after {
            return;
        }
        let Some(instance) = change.after.as_ref().or(change.before.as_ref()) else {
            return;
        };
        let count_delta = i128::from(change.after.as_ref().map_or(0, |item| item.count))
            .saturating_sub(i128::from(change.before.as_ref().map_or(0, |item| item.count)));
        self.item_evidence(
            subject,
            ItemEvidenceData::Delta(json!({
                "operation": operation, "instance": instance.id, "before": change.before,
                "after": change.after, "count_delta": count_delta,
                "operation_ordinal": ordinal, "receipt": receipt, "resolution_id": resolution
            })),
        );
    }

    #[allow(clippy::too_many_lines)] // Serialize captured source inputs and resolved values together.
    pub(super) fn item_evidence_enchant(
        &mut self,
        subject: TargetBinding,
        attempt: &crate::domain::zone::item_enchant::EnchantAttempt,
        detail: &crate::domain::zone::phase4_projection::ItemResultDetail,
    ) -> Result<(), RejectReason> {
        use crate::domain::zone::phase4_projection::{
            ItemEnchantOutcome, ItemEvidenceValue, ItemResultDetail,
        };
        if !self.items_evidence_enabled() {
            return Ok(());
        }
        let invalid = || RejectReason::InvalidLoad;
        let text = |key: &str| match attempt.cause.fields.get(key) {
            Some(ItemEvidenceValue::Text(value)) => Ok(value.as_str()),
            _ => Err(invalid()),
        };
        let number = |key: &str| match attempt.cause.fields.get(key) {
            Some(ItemEvidenceValue::Unsigned(value)) => Ok(*value),
            _ => Err(invalid()),
        };
        let ratio = |key: &str| number(key).map(|n| json!({"numerator":n,"denominator":1000}));
        let resolution = text("resolution_id")?.to_owned();
        let before = text("word_before")?
            .parse::<u128>()
            .map_err(|_| invalid())?;
        let scroll: serde_json::Value =
            serde_json::from_str(text("scroll_before")?).map_err(|_| invalid())?;
        let support: serde_json::Value =
            serde_json::from_str(text("support_before")?).map_err(|_| invalid())?;
        let items = self.phase4.as_ref().ok_or_else(invalid)?;
        let rule = items
            .rules
            .parts()
            .enchant_scrolls
            .get(&attempt.scroll_template)
            .ok_or_else(invalid)?;
        let inputs = json!({"base":ratio("base_chance_1e5")?, "bonus":ratio("scroll_bonus_1e5")?,
            "support":ratio("support_bonus_1e5")?});
        let mut outcome_inputs = inputs.clone();
        let fields = outcome_inputs.as_object_mut().ok_or_else(invalid)?;
        fields.insert("residue".into(), json!(attempt.draw));
        fields.insert("enchant".into(), json!(attempt.old_enchant));
        fields.insert("safe".into(), json!(rule.safe));
        fields.insert("blessed".into(), json!(rule.blessed));
        let outcome = match attempt.outcome {
            ItemEnchantOutcome::Success | ItemEnchantOutcome::SuccessNoIncrement => "success",
            ItemEnchantOutcome::SafeFailed => "safe_failure",
            ItemEnchantOutcome::BlessedReset => "blessed_failure",
            ItemEnchantOutcome::Destroyed => "destroyed",
        };
        let ItemResultDetail::Enchant { refunds, .. } = detail else {
            return Err(invalid());
        };
        let refund_fact = if refunds.is_empty() {
            None
        } else {
            let refund_count = refunds
                .iter()
                .try_fold(0_u64, |sum, refund| sum.checked_add(refund.count))
                .ok_or_else(invalid)?;
            let definition = items
                .rules
                .parts()
                .items
                .get(&attempt.template)
                .ok_or_else(invalid)?;
            let grade = items
                .rules
                .parts()
                .grades
                .iter()
                .find(|g| g.grade == definition.grade)
                .ok_or_else(invalid)?;
            let weapon = definition.kind == "Weapon";
            Some(json!({"formula":"failure_refund", "inputs":{
                "base":definition.crystal_count,"bonus":if weapon {grade.weapon_bonus} else {grade.armor_bonus},
                "enchant":attempt.old_enchant,"weapon":weapon,
                "crystallizable":definition.flags.get("is_crystallizable") == Some(&true),"crystal_id":grade.crystal_id},
                "actual":refund_count,"resolution_id":resolution,"baseline":false}))
        };
        self.item_evidence(
            subject,
            ItemEvidenceData::Rng {
                callsite: "enchant".into(),
                word_before: before,
                word_after: self.rng.get_word_pos(),
                bound: 100_000,
                value: attempt.draw,
                resolution_id: resolution.clone(),
            },
        );
        self.item_evidence(
            subject,
            ItemEvidenceData::Formula(json!({
            "formula":"enchant_chance","inputs":inputs,"actual":attempt.chance_1e5,
            "resolution_id":resolution,"baseline":false})),
        );
        self.item_evidence(subject, ItemEvidenceData::Formula(json!({
            "formula":"enchant_outcome","inputs":outcome_inputs,"actual":{
                "outcome":outcome,"enchant":if attempt.outcome == ItemEnchantOutcome::Destroyed {attempt.old_enchant} else {attempt.new_enchant},
                "destroyed":attempt.outcome == ItemEnchantOutcome::Destroyed},
            "operation":{"target_before":attempt.before,"scroll_before":scroll,"support_before":support,"receipt":attempt.receipt},
            "resolution_id":resolution,"baseline":false})));
        if let Some(fact) = refund_fact {
            self.item_evidence(subject, ItemEvidenceData::Formula(fact));
        }
        Ok(())
    }

    /// Retail draws (three `item_rng` facts), both selectors, the category and packing for
    /// Refine; the instance-crystal fee for `CancelRefine`. Inputs are the actual filtered rows.
    pub(super) fn item_evidence_refine(
        &mut self,
        subject: TargetBinding,
        audit: &crate::domain::zone::RefineAudit,
        draws: Option<&(crate::domain::zone::item_refine::RetailSelectors, Vec<(u128, u128, u32)>)>,
        resolution: &str,
    ) -> Result<(), RejectReason> {
        use crate::domain::zone::item_refine::{RetailRow, RETAIL_RESIDUES};
        if !self.items_evidence_enabled() {
            return Ok(());
        }
        let invalid = || RejectReason::InvalidLoad;
        let Some(selection) = audit.selection else {
            let items = self.phase4.as_ref().ok_or_else(invalid)?;
            let definition = items
                .rules
                .parts()
                .items
                .get(&audit.before.template_id)
                .ok_or_else(invalid)?;
            let crystals = crate::domain::zone::item_enchant::crystal_count(
                definition,
                audit.before.enchant,
                &items.rules,
            )
            .map_err(|_| invalid())?;
            let fee = audit.payments.first().ok_or_else(invalid)?.count;
            self.item_evidence(
                subject,
                ItemEvidenceData::Formula(json!({
                "formula":"cancel_fee","inputs":{"grade":definition.grade,"crystals":crystals},
                "actual":fee,"resolution_id":resolution,"baseline":false})),
            );
            return Ok(());
        };
        let (selectors, words) = draws.ok_or_else(invalid)?;
        let rows = |rows: &[RetailRow]| {
            rows.iter()
                .map(|row| json!([row.option, {"numerator": row.hundredths, "denominator": 100}]))
                .collect::<Vec<_>>()
        };
        let sites = ["augment_option12", "augment_category", "augment_option34"];
        if words.len() != sites.len() {
            return Err(invalid());
        }
        for (site, (before, after, raw)) in sites.iter().zip(words) {
            self.item_evidence(
                subject,
                ItemEvidenceData::Rng {
                    callsite: (*site).into(),
                    word_before: *before,
                    word_after: *after,
                    bound: RETAIL_RESIDUES,
                    value: raw % RETAIL_RESIDUES,
                    resolution_id: format!("{resolution}:{site}"),
                },
            );
        }
        let facts = [
            json!({"formula":"retail_select","inputs":{"rows":rows(&selectors.first),
                "residue":selection.first_raw % RETAIL_RESIDUES},"actual":selection.option12,
                "resolution_id":format!("{resolution}:{}", sites[0]),"baseline":false}),
            json!({"formula":"category","inputs":{"residue":selection.category_raw % RETAIL_RESIDUES,
                "accessory":selectors.accessory},"actual":selection.category,
                "resolution_id":format!("{resolution}:{}", sites[1]),"baseline":false}),
            json!({"formula":"retail_select","inputs":{"rows":rows(&selectors.category_rows(selection.category)),
                "residue":selection.second_raw % RETAIL_RESIDUES},"actual":selection.option34,
                "resolution_id":format!("{resolution}:{}", sites[2]),"baseline":false}),
            json!({"formula":"retail_draw_contract","inputs":{"accessory":selectors.accessory},
                "actual":{"draw_sites":[{"site":"option12","bound":RETAIL_RESIDUES},
                    {"site":"category","bound":RETAIL_RESIDUES},{"site":"option34","bound":RETAIL_RESIDUES}],
                    "category_weights":selectors.weights,"non_retail_draws":[]},
                "resolution_id":format!("{resolution}:contract"),"baseline":false}),
            json!({"formula":"pack_options","inputs":{"first":selection.option12,"second":selection.option34},
                "actual":selection.packed().map_err(|_| invalid())?,"resolution_id":resolution,"baseline":false}),
        ];
        for fact in facts {
            self.item_evidence(subject, ItemEvidenceData::Formula(fact));
        }
        Ok(())
    }

    /// D46 `max_load`/`overload`/`grade_penalties` when a dirty rebuild changes them.
    pub(super) fn load_evidence_v6(
        &mut self,
        id: EntityId,
        refreshed: &crate::domain::zone::item_refresh::RefreshedEquipment,
    ) -> Result<(), TickError> {
        if !self.items_evidence_enabled() {
            return Ok(());
        }
        let Some(rules) = self
            .phase4
            .as_ref()
            .map(|items| std::sync::Arc::clone(&items.rules))
        else {
            return Ok(());
        };
        let Some(old) = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&id))
            .cloned()
        else {
            return Ok(());
        };
        let new = &refreshed.player;
        let word = self.rng.get_word_pos();
        let resolution = format!("load:{id}:{}:{word}", self.next_tick.0);
        if (old.max_load, old.weight, old.overload_level, old.overloaded)
            != (new.max_load, new.weight, new.overload_level, new.overloaded)
        {
            let load = &refreshed.load;
            self.item_formula_v6(
                id,
                "max_load",
                json!({"con_bonus": ratio_json(load.con_bonus), "weight_limit": ratio_json(load.weight_limit),
                    "multiplier": ratio_json(load.multiplier), "add": ratio_json(load.add)}),
                json!(new.max_load),
                &resolution,
                None,
            );
            let per_mille = if new.max_load > 0 {
                Some(
                    i128::from(new.weight)
                        .checked_sub(i128::from(new.weight_penalty_bonus))
                        .and_then(|v| v.checked_mul(1000))
                        .and_then(|v| v.checked_div(i128::from(new.max_load)))
                        .and_then(|v| i64::try_from(v).ok())
                        .ok_or(TickError::Phase3Invariant)?,
                )
            } else {
                None
            };
            self.item_formula_v6(
                id,
                "overload",
                json!({"weight": new.weight, "maximum": new.max_load, "bonus": new.weight_penalty_bonus,
                    "previous_level": old.overload_level, "previous_overloaded": old.overloaded}),
                json!({"per_mille": per_mille, "level": new.overload_level,
                    "overloaded": new.overloaded, "changed": new.overload_level != old.overload_level}),
                &resolution,
                None,
            );
        }
        if (old.weapon_penalty, old.armor_penalty, old.expertise, &old.paperdoll)
            != (new.weapon_penalty, new.armor_penalty, new.expertise, &new.paperdoll)
        {
            let mut equipped = Vec::new();
            for item in new.inventory.values().filter(|item| item.slot.is_some()) {
                let definition = rules
                    .parts()
                    .items
                    .get(&item.template_id)
                    .ok_or(TickError::Phase3Invariant)?;
                let grade = rules
                    .parts()
                    .grades
                    .iter()
                    .find(|row| row.grade == definition.grade)
                    .ok_or(TickError::Phase3Invariant)?
                    .id;
                equipped.push(json!({"instance_id": item.id, "grade": grade,
                    "weapon": definition.kind == "Weapon",
                    "arrow": matches!(definition.etcitem_type.as_str(), "ARROW" | "BOLT")}));
            }
            self.item_formula_v6(
                id,
                "grade_penalties",
                json!({"equipped": equipped, "expertise": new.expertise, "bonus": 0}),
                json!({"weapon": new.weapon_penalty, "armor": new.armor_penalty}),
                &resolution,
                None,
            );
        }
        Ok(())
    }

    /// A live V6 owner with an item sidecar (combat evidence subjects).
    pub(in crate::domain::zone) fn phase4_player_v6(&self, id: EntityId) -> bool {
        self.phase4
            .as_ref()
            .is_some_and(|items| items.canonical.players.contains_key(&id))
    }

    /// One `item_formula` fact for a V6 owner; resolution identities are caller-unique.
    #[allow(clippy::too_many_arguments)] // The frozen fact fields plus its owner.
    pub(in crate::domain::zone) fn item_formula_v6(
        &mut self,
        owner: EntityId,
        formula: &str,
        inputs: serde_json::Value,
        actual: serde_json::Value,
        resolution: &str,
        context: Option<serde_json::Value>,
    ) {
        if !self.items_evidence_enabled() || !self.phase4_player_v6(owner) {
            return;
        }
        if let Some(subject) = self.binding(owner) {
            let mut fact = serde_json::Map::new();
            fact.insert("formula".into(), json!(formula));
            fact.insert("inputs".into(), inputs);
            fact.insert("actual".into(), actual);
            fact.insert("resolution_id".into(), json!(resolution));
            fact.insert("baseline".into(), json!(false));
            if let Some(context) = context {
                fact.insert("context".into(), context);
            }
            self.item_evidence(subject, ItemEvidenceData::Formula(serde_json::Value::Object(fact)));
        }
    }

    /// Captured caller identity for scenario phases: weapon (0 = unarmed), recipient, event.
    pub(in crate::domain::zone) fn attack_context_v6(
        &self,
        attacker: EntityId,
        target: EntityId,
    ) -> serde_json::Value {
        let weapon = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&attacker))
            .and_then(|player| {
                player
                    .paperdoll
                    .get(&crate::domain::zone::EquipSlot::Rhand)
                    .and_then(|id| player.inventory.get(id))
            })
            .map_or(0, |item| item.template_id);
        json!({"weapon_template": weapon, "target_entity": target.to_string(), "event": "attack"})
    }

    pub(super) fn item_evidence_npc_roll(&mut self, npc: EntityId, site: &str, bound: u32) -> u32 {
        let before = self.rng.get_word_pos();
        let value = self.roll_below(bound);
        if self.items_evidence_enabled() {
            if let Some(subject) = self.binding(npc) {
                self.item_evidence(
                    subject,
                    ItemEvidenceData::Rng {
                        callsite: site.into(),
                        word_before: before,
                        word_after: self.rng.get_word_pos(),
                        bound,
                        value,
                        resolution_id: format!("npc:{site}:{before}"),
                    },
                );
            }
        }
        value
    }
}

/// D80: an integer outside ±2^53 is an exact decimal string; others stay JSON numbers.
pub(in crate::domain::zone) fn wide_json(value: i128) -> serde_json::Value {
    const LIMIT: u128 = 1 << 53;
    if value.unsigned_abs() > LIMIT {
        json!(value.to_string())
    } else {
        i64::try_from(value).map_or_else(|_| json!(value.to_string()), |v| json!(v))
    }
}

/// Exact reduced ratio in the frozen `{numerator, denominator}` evidence shape (D80 widths).
pub(in crate::domain::zone) fn ratio_json(
    value: crate::domain::zone::skill_math::ExactRatio,
) -> serde_json::Value {
    let (numerator, denominator) = value.parts();
    json!({"numerator": wide_json(numerator), "denominator": wide_json(denominator)})
}

fn big_json(value: &num_bigint::BigUint) -> serde_json::Value {
    if value.bits() <= 53 {
        u64::try_from(value).map_or_else(|_| json!(value.to_string()), |v| json!(v))
    } else {
        json!(value.to_string())
    }
}

#[allow(clippy::arithmetic_side_effects)] // Arbitrary-precision integers; divisors are nonzero.
fn big_gcd(a: &num_bigint::BigUint, b: &num_bigint::BigUint) -> num_bigint::BigUint {
    let (mut a, mut b) = (a.clone(), b.clone());
    while b != num_bigint::BigUint::ZERO {
        let r = &a % &b;
        a = b;
        b = r;
    }
    a
}

#[allow(clippy::arithmetic_side_effects)] // Arbitrary-precision integers; divisors are nonzero.
fn big_ratio(
    numerator: &num_bigint::BigUint,
    denominator: &num_bigint::BigUint,
) -> serde_json::Value {
    let divisor = big_gcd(numerator, denominator);
    let divisor = if divisor == num_bigint::BigUint::ZERO {
        num_bigint::BigUint::from(1_u8)
    } else {
        divisor
    };
    json!({"numerator": big_json(&(numerator / &divisor)), "denominator": big_json(&(denominator / &divisor))})
}

/// D46/D80 `bow_damage` actual, reproducing the frozen certificate exactly: `base =
/// trunc(formula)`, `damage = floor(4·base/5 + base·√R/125000)` with R = dx²+dy² (milli-
/// tiles), decided on the first 2^-bits (64, 128, 256, 512) root interval whose floors agree
/// or whose upper floor is an exact root tie; distances are those bounds × 32/1000.
#[allow(clippy::arithmetic_side_effects)] // Arbitrary-precision integers; divisors are nonzero.
pub(in crate::domain::zone) fn bow_damage_actual(
    base: u64,
    dx: i64,
    dy: i64,
) -> Option<serde_json::Value> {
    use num_bigint::BigUint;
    let radicand =
        BigUint::from(dx.unsigned_abs()).pow(2) + BigUint::from(dy.unsigned_abs()).pow(2);
    let base_big = BigUint::from(base);
    for bits in [64_u32, 128, 256, 512] {
        let scale = BigUint::from(1_u8) << bits;
        let scaled = &radicand * &scale * &scale;
        let lower = scaled.sqrt();
        let upper = if &lower * &lower == scaled {
            lower.clone()
        } else {
            &lower + 1_u8
        };
        // floor(base·(500000·S + 5·root)/(625000·S))
        let floor_at = |root: &BigUint| -> BigUint {
            (&base_big * (BigUint::from(500_000_u32) * &scale + root * 5_u8))
                / (BigUint::from(625_000_u32) * &scale)
        };
        let lo = floor_at(&lower);
        let hi = floor_at(&upper);
        let tie = base > 0 && {
            // ((hi − 4·base/5)·125000/base)² == R, with the quotient nonnegative.
            let five_hi = &hi * 5_u8;
            let four_base = &base_big * 4_u8;
            five_hi >= four_base && {
                let numerator = (five_hi - four_base) * 25_000_u32;
                &numerator * &numerator == &radicand * &base_big * &base_big
            }
        };
        if lo == hi || tie {
            let thousand_scale = BigUint::from(1000_u16) * &scale;
            return Some(json!({"base": base,
                "distance_l2_lower": big_ratio(&(&lower * 32_u8), &thousand_scale),
                "distance_l2_upper": big_ratio(&(&upper * 32_u8), &thousand_scale),
                "damage": big_json(&hi)}));
        }
    }
    None
}
