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
                if origin.count == 0
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
                        && source.id == lane.cast.skill.id.0
                        && source.level == lane.cast.skill.rank
                }) || lane.simultaneous != simultaneous(definition, skill, lane.cast.skill.rank)?
                {
                    return Err(ItemError("item cast origin does not grant this skill/lane"));
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
