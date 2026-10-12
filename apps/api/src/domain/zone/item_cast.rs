//! Item-origin cast authorization is separate from the permanent learned-skill ledger.
use super::item_rules::ItemDefinition;
use super::skill::{SkillDef, SkillRef};
use super::{
    EntityId, EquipSlot, ItemError, Phase3EntityContext, Phase3Snapshot, Phase4Rules,
    Phase4Snapshot,
};
use std::collections::BTreeMap;

/// ItemSkillsTemplate.java:112: explicit skill lane, or immediate static item effect.
pub(super) fn simultaneous(
    definition: &ItemDefinition,
    skill: &SkillDef,
    rank: u32,
) -> Result<bool, ItemError> {
    let explicit = match skill.source_sets.get("simultaneousCast") {
        None => false,
        Some(values) => match values.at(rank, skill.max_level).map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            _ => return Err(ItemError("invalid simultaneous item skill predicate")),
        },
    };
    Ok(explicit
        || (skill.is_magic == 2
            && ["immediate_effect", "ex_immediate_effect"]
                .iter()
                .any(|key| definition.flags.get(*key) == Some(&true))))
}

/// ItemSkillsTemplate.java:82–148 and L2Character.java:1560–1566: one whole-item
/// cost plan, with selected-stack preference and a single source fallback debit.
pub(super) fn payment_edits(
    definition: &ItemDefinition,
    registry: &super::skill::SkillRegistry,
    instance: super::ItemInstanceId,
) -> Result<Vec<super::item_inventory::InventoryEdit>, ItemError> {
    use super::item_inventory::InventoryEdit;
    let mut edits = Vec::new();
    for source in definition
        .skills
        .iter()
        .filter(|source| source.kind == "item")
    {
        let skill = registry
            .get(super::skill::SkillId(source.id))
            .ok_or(ItemError("unknown item skill cost"))?;
        for cost in &skill.item_consume {
            let template = *cost
                .item_id
                .at(source.level, skill.max_level)
                .ok_or(ItemError("invalid item cost rank"))?;
            let count = *cost
                .count
                .at(source.level, skill.max_level)
                .ok_or(ItemError("invalid item cost rank"))?;
            if template == 0 || count == 0 {
                return Err(ItemError("invalid item skill cost"));
            }
            edits.push(InventoryEdit::TakeTemplate {
                template,
                count: u64::from(count),
                preferred: (template == definition.template_id).then_some(instance),
            });
        }
    }
    if edits.is_empty()
        && definition.flags.get("immediate_effect") == Some(&true)
        && definition
            .properties
            .get("default_action")
            .is_some_and(|action| matches!(action.as_str(), "CAPSULE" | "SKILL_REDUCE"))
    {
        edits.push(InventoryEdit::TakeInstance { instance, count: 1 });
    }
    Ok(edits)
}

impl Phase4Snapshot {
    /// Validate both lanes before allowing an unlearned main cast through Phase3 checks.
    /// A consumed last unit remains an origin fact; a still-held origin must match its owner.
    pub(super) fn validate_casts(
        &self,
        rules: &Phase4Rules,
        phase3: &Phase3Snapshot,
        contexts: &BTreeMap<EntityId, Phase3EntityContext<'_>>,
    ) -> Result<BTreeMap<EntityId, SkillRef>, ItemError> {
        self.validate_allocations(rules, phase3)?;
        let mut main = BTreeMap::new();
        let (start, next, end) = self.allocator.bounds();
        for lane in self.item_casts.values() {
            let context = contexts
                .get(&lane.caster.entity)
                .copied()
                .ok_or(ItemError("item cast has no caster"))?;
            let player = self
                .players
                .get(&lane.caster.entity)
                .ok_or(ItemError("item cast has no owner"))?;
            if !context.player
                || lane.caster.incarnation != context.incarnation
                || lane.cast.caster_incarnation != context.incarnation
                || lane.shots & !7 != 0
                || (lane.shots != 0 && lane.weapon.is_none())
            {
                return Err(ItemError("invalid item cast identity/charges"));
            }
            if let Some(weapon) = lane.weapon {
                if player.paperdoll.get(&EquipSlot::Rhand) != Some(&weapon)
                    || player
                        .inventory
                        .get(&weapon)
                        .and_then(|item| rules.parts().items.get(&item.template_id))
                        .is_none_or(|item| item.kind != "Weapon")
                {
                    return Err(ItemError("item cast weapon is not held by caster"));
                }
            }
            let authority = if let Some(origin) = &lane.origin {
                if (origin.count == 0 && !player.inventory.contains_key(&origin.instance))
                    || origin.receipt.key.is_nil()
                    || origin.receipt.operation != super::phase4_projection::ItemOperation::Use
                    || i64::try_from(origin.count).is_err()
                    || (origin.instance.get() >= start
                        && origin.instance.get() < end
                        && origin.instance.get() >= next)
                    || self.players.iter().any(|(owner, player)| {
                        player.inventory.get(&origin.instance).is_some_and(|item| {
                            *owner != lane.caster.entity || item.template_id != origin.template
                        })
                    })
                    || rules
                        .parts()
                        .capabilities
                        .get(&origin.template)
                        .is_none_or(|cap| !cap.admitted || !cap.use_blockers.is_empty())
                {
                    return Err(ItemError("invalid item cast origin"));
                }
                let definition = rules
                    .parts()
                    .items
                    .get(&origin.template)
                    .ok_or(ItemError("unknown item cast origin template"))?;
                let skill = phase3
                    .rules
                    .skills()
                    .get(lane.cast.skill.id)
                    .ok_or(ItemError("unknown item cast skill"))?;
                if !definition.skills.iter().any(|source| {
                    source.kind == "item"
                        && source.xml_ordinal == origin.skill_ordinal
                        && source.id == lane.cast.skill.id.0
                        && source.level == lane.cast.skill.rank
                }) || lane.simultaneous != simultaneous(definition, skill, lane.cast.skill.rank)?
                {
                    return Err(ItemError("item cast origin does not grant this skill/lane"));
                }
                let mut clicked_cost = 0_u64;
                for edit in payment_edits(definition, phase3.rules.skills(), origin.instance)? {
                    use super::item_inventory::InventoryEdit;
                    let count = match edit {
                        InventoryEdit::TakeTemplate {
                            template, count, ..
                        } if template == origin.template => count,
                        InventoryEdit::TakeInstance { count, .. } => count,
                        InventoryEdit::TakeTemplate { .. } | InventoryEdit::Add { .. } => 0,
                    };
                    clicked_cost = clicked_cost
                        .checked_add(count)
                        .ok_or(ItemError("item origin cost overflow"))?;
                }
                if (clicked_cost == 0 && origin.count != 0)
                    || (clicked_cost > 0
                        && (origin.count == 0
                            || origin.count > clicked_cost
                            || (player.inventory.contains_key(&origin.instance)
                                && origin.count != clicked_cost)))
                {
                    return Err(ItemError("item origin does not match its source payment"));
                }
                Some(lane.cast.skill)
            } else {
                None
            };
            lane.cast
                .validate(&phase3.rules, context, authority)
                .map_err(|_| ItemError("invalid live item cast"))?;
            if !lane.simultaneous {
                if let Some(skill) = authority {
                    if main.insert(lane.caster.entity, skill).is_some() {
                        return Err(ItemError("duplicate main item cast authority"));
                    }
                }
            }
        }
        Ok(main)
    }
}
