//! Explicit V6 construction. Historical constructors never acquire an item sidecar.
use super::{SnapshotError, StateDigestVersion, ZoneBounds, ZoneSeed, ZoneState};
use crate::domain::zone::{
    ItemInstanceRange, Phase3AdmissionPolicy, Phase3Rules, Phase4Rules, Phase4Snapshot, StatRules,
};
use std::sync::Arc;

impl ZoneState {
    /// V6 callers use the current paperdoll; older callers retain their historical context.
    pub(super) fn item_weapon_kind<'a>(
        &'a self,
        entity: crate::domain::zone::EntityId,
        legacy: &'a str,
    ) -> Result<&'a str, super::TickError> {
        let Some(items) = &self.phase4 else {
            return Ok(legacy);
        };
        let Some(player) = items.canonical.players.get(&entity) else {
            return Ok("NONE");
        };
        crate::domain::zone::effective_stats::EquipmentStatContext {
            player,
            rules: &items.rules,
        }
        .weapon_kind()
        .map_err(|_| super::TickError::Phase3Invariant)
    }

    pub(super) fn remove_items_v6(&mut self, entity: crate::domain::zone::EntityId) {
        if let Some(items) = &mut self.phase4 {
            let weapons: std::collections::BTreeSet<_> = items
                .canonical
                .players
                .get(&entity)
                .into_iter()
                .flat_map(|player| player.inventory.keys().copied())
                .collect();
            items
                .canonical
                .charge_bindings
                .retain(|binding| !weapons.contains(&binding.weapon));
            items
                .canonical
                .item_casts
                .retain(|_, cast| cast.caster.entity != entity);
            items.canonical.attack_plans.remove(&entity);
            items.canonical.players.remove(&entity);
            items.canonical.facing.remove(&entity);
        }
    }
    /// Construct a V6 epoch from validated, explicitly supplied immutable bundles and a
    /// recorded allocator reservation. Service startup supplies the compiled hash bundle.
    #[allow(clippy::too_many_arguments)] // Independent epoch inputs must remain explicit.
    pub fn new_phase4(
        seed: ZoneSeed,
        bounds: ZoneBounds,
        time_origin_ms: i64,
        stat_rules: Arc<StatRules>,
        classes: Arc<crate::domain::class::ClassRegistry>,
        skills: Phase3Rules,
        items: Arc<Phase4Rules>,
        allocator: ItemInstanceRange,
    ) -> Result<Self, SnapshotError> {
        if skills.runtime_revision() != 3 {
            return Err(SnapshotError::Rules("V6 requires the V6 skill capability bundle".into()));
        }
        items
            .validate_phase3(&skills)
            .map_err(SnapshotError::Rules)?;
        let canonical = Phase4Snapshot::new(&items, allocator);
        let mut state =
            Self::new_skill_epoch(seed, bounds, time_origin_ms, stat_rules, classes, skills)?;
        state.phase4 = Some(crate::domain::zone::phase4::Phase4Zone {
            canonical,
            rules: items,
        });
        state.meta.schema_version = 10;
        state.meta.digest_version = StateDigestVersion::BinaryV6;
        state.meta.phase3_admission_policy = Some(Phase3AdmissionPolicy::version_two());
        state.snapshot().validate_version()?;
        Ok(state)
    }

    /// Construct a V7 epoch (design §12.1): the V6 inputs plus the bound Phase 5 rules,
    /// an empty Phase 5 block and the `version_three` admission policy.
    #[allow(clippy::too_many_arguments)]
    // Independent epoch inputs must remain explicit.
    // The V6 bundle is taken by value like `new_phase4`; a V7 epoch binds it and then runs
    // on the economy rules' V7 item view (D10), so the argument itself is only checked.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new_phase5(
        seed: ZoneSeed,
        bounds: ZoneBounds,
        time_origin_ms: i64,
        stat_rules: Arc<StatRules>,
        classes: Arc<crate::domain::class::ClassRegistry>,
        skills: Phase3Rules,
        items: Arc<Phase4Rules>,
        allocator: ItemInstanceRange,
        economy: Arc<crate::domain::zone::Phase5Rules>,
    ) -> Result<Self, SnapshotError> {
        economy
            .validate_phase4(&items)
            .map_err(SnapshotError::Rules)?;
        // D10: a V7 epoch runs on the V7 item view (V6 ∪ closure, under the V6 hash).
        let items = Arc::clone(economy.v7_items().ok_or_else(|| {
            SnapshotError::Rules("V7 economy rules without the bound closure items".into())
        })?);
        // P5-D19: and on the V7 skill view (V6 skills ∪ promotions, same Phase 3 hash).
        let view = economy.v7_skills().ok_or_else(|| {
            SnapshotError::Rules("V7 economy rules without the bound skill promotions".into())
        })?;
        if view.hash() != skills.hash() {
            return Err(SnapshotError::Rules("V7 skill view of another bundle".into()));
        }
        let skills = (**view).clone();
        let mut state = Self::new_phase4(
            seed,
            bounds,
            time_origin_ms,
            stat_rules,
            classes,
            skills,
            items,
            allocator,
        )?;
        state.phase5 = Some(crate::domain::zone::Phase5Zone {
            canonical: crate::domain::zone::Phase5Snapshot::new(&economy),
            rules: economy,
            fault: None,
            rewards: None,
            ledger: Vec::new(),
        });
        state.meta.schema_version = 11;
        state.meta.digest_version = StateDigestVersion::BinaryV7;
        state.meta.phase3_admission_policy = Some(Phase3AdmissionPolicy::version_three());
        state.snapshot().validate_version()?;
        Ok(state)
    }

    /// The design §1.11 invariants of the Phase 5 block against the rest of the zone.
    pub(super) fn validate_phase5_block(&self) -> Result<(), SnapshotError> {
        let Some(phase5) = &self.phase5 else {
            return Ok(());
        };
        let items = self.phase4.as_ref().ok_or(SnapshotError::CombatMismatch)?;
        phase5
            .canonical
            .validate(
                &phase5.rules,
                &crate::domain::zone::Phase5Context {
                    items: &items.rules,
                    phase4: &items.canonical,
                    entities: &self.entities,
                    seed: self.seed,
                    tick: self.next_tick,
                },
            )
            .map_err(|error| SnapshotError::Rules(error.to_string()))?;
        self.validate_pinned_slots()
    }

    /// z-review-3 P2-1: on a V7 zone every slot's `pinned_npc_id` names a pinned template
    /// (one with a `rewards.toml` row, P5-D12), checked when the slots are installed and at
    /// restore, so a bad overlay refuses boot instead of halting the zone at a kill.
    pub(super) fn validate_pinned_slots(&self) -> Result<(), SnapshotError> {
        let Some(phase5) = &self.phase5 else {
            return Ok(());
        };
        for spec in &self.slots {
            if let Some(id) = spec.pinned_npc_id {
                if phase5.rules.reward_npc(id).is_none() {
                    return Err(SnapshotError::Rules(format!(
                        "spawn slot pins npc {id}, which is not a pinned template"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Admit a player's V7 record from its load (design §1.11): a V7 load requires the
    /// Phase 5 checkpoint, validated against the epoch's rules hash and recipe limits.
    pub(super) fn admit_phase5_player(
        &mut self,
        entity: crate::domain::zone::EntityId,
        load: &crate::domain::zone::PlayerLoad,
    ) -> Result<(), super::RejectReason> {
        let Some(phase5) = &mut self.phase5 else {
            return Ok(());
        };
        let checkpoint = load
            .phase5
            .as_ref()
            .ok_or(super::RejectReason::InvalidLoad)?;
        checkpoint
            .validate_rules(&phase5.rules)
            .map_err(|_| super::RejectReason::InvalidLoad)?;
        let limits = &phase5.rules.parts().recipes.config;
        let known: std::collections::BTreeSet<u32> = phase5
            .rules
            .parts()
            .recipes
            .recipe
            .iter()
            .map(|recipe| recipe.id)
            .collect();
        let recipes = &checkpoint.recipes;
        if recipes.dwarven.len() > usize::try_from(limits.dwarf_recipe_limit).unwrap_or(usize::MAX)
            || recipes.common.len()
                > usize::try_from(limits.common_recipe_limit).unwrap_or(usize::MAX)
            || !recipes.dwarven.is_disjoint(&recipes.common)
            || recipes
                .dwarven
                .iter()
                .chain(&recipes.common)
                .any(|recipe| !known.contains(recipe))
        {
            return Err(super::RejectReason::InvalidLoad);
        }
        phase5.canonical.players.insert(
            entity,
            crate::domain::zone::Phase5Player {
                recipes: recipes.clone(),
                ..crate::domain::zone::Phase5Player::default()
            },
        );
        Ok(())
    }

    /// Drop a departed player's V7 record. The departure hooks (store close, party leave,
    /// trade cancel, refund destroy) are stories 3.5 and 3.9; nothing creates those yet.
    pub(super) fn remove_phase5_player(&mut self, entity: crate::domain::zone::EntityId) {
        if let Some(phase5) = &mut self.phase5 {
            phase5.canonical.players.remove(&entity);
        }
    }

    /// Recorded item policy; environment switches cannot alter an existing epoch.
    pub fn phase4_enabled(&self) -> bool {
        matches!(
            (self.meta.schema_version, self.meta.digest_version),
            (10, StateDigestVersion::BinaryV6) | (11, StateDigestVersion::BinaryV7)
        )
    }

    /// V7 (Phase 5) epochs only; selected by the validated epoch version, never by state.
    pub fn phase5_enabled(&self) -> bool {
        self.meta.schema_version == 11 && self.meta.digest_version == StateDigestVersion::BinaryV7
    }

    pub(crate) fn item_instance_range(&self) -> Option<ItemInstanceRange> {
        self.phase4.as_ref().map(|phase| phase.canonical.allocator)
    }
}
