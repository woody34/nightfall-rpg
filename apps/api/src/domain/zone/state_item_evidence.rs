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
            json!({"formula":"pack_options","inputs":{"first":selection.option12,"second":selection.option34},
                "actual":selection.packed().map_err(|_| invalid())?,"resolution_id":resolution,"baseline":false}),
        ];
        for fact in facts {
            self.item_evidence(subject, ItemEvidenceData::Formula(fact));
        }
        Ok(())
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
