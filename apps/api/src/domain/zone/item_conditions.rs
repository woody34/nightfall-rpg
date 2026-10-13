//! Item admission conditions from DocumentBase.java:372–611 and the corresponding
//! `ConditionPlayer` predicates. Unsupported contexts are errors, including under NOT/OR.
use super::item_rules::ItemCondition;
use super::{ItemError, Phase4Player, Phase4Rules};
use crate::domain::character_progression::CharacterIdentity;
use crate::domain::class::ClassId;
use crate::domain::subclass::Sex;
use crate::domain::Race;

/// Authoritative inputs before spending or changing equipment. Flying mounts and GM diet
/// mode are not admitted; their source branches cannot be selected by a client flag.
pub struct ItemConditionContext<'a> {
    /// Retained race/appearance, independent of equipment grade.
    pub identity: &'a CharacterIdentity,
    /// Current profession, not the immutable starting class.
    pub class_id: ClassId,
    /// Effective character level.
    pub level: u32,
    /// Current inventory and already-folded capacity/load getters.
    pub player: &'a Phase4Player,
    /// Epoch-bound item definitions.
    pub rules: &'a Phase4Rules,
    /// Existing resurrection eligibility, supplied by the cast caller.
    pub can_resurrect: bool,
}
impl ItemConditionContext<'_> {
    /// Every attached condition must hold; a bare item has no implicit restriction.
    pub fn matches(&self, conditions: &[ItemCondition]) -> Result<bool, ItemError> {
        let mut matched = true;
        for condition in conditions {
            matched &= self.condition(condition)?;
        }
        Ok(matched)
    }
    fn condition(&self, condition: &ItemCondition) -> Result<bool, ItemError> {
        if !condition.text.is_empty() {
            return Err(ItemError("unexpected item condition text"));
        }
        match condition.tag.as_str() {
            "cond" => {
                if condition
                    .attributes
                    .keys()
                    .any(|key| !matches!(key.as_str(), "msgId" | "msg" | "addName"))
                {
                    return Err(ItemError("unsupported item condition wrapper"));
                }
                self.matches(&condition.children)
            },
            "and" | "or" | "not" => {
                if !condition.attributes.is_empty() || condition.children.is_empty() {
                    return Err(ItemError("invalid item condition expression"));
                }
                if condition.tag == "not" {
                    let [child] = condition.children.as_slice() else {
                        return Err(ItemError("item NOT requires exactly one child"));
                    };
                    return Ok(!self.condition(child)?);
                }
                let mut result = condition.tag == "and";
                for child in &condition.children {
                    let matched = self.condition(child)?;
                    if condition.tag == "and" {
                        result &= matched;
                    } else {
                        result |= matched;
                    }
                }
                Ok(result)
            },
            "player" => {
                if condition.attributes.is_empty() || !condition.children.is_empty() {
                    return Err(ItemError("invalid item player condition"));
                }
                let mut result = true;
                for (name, value) in &condition.attributes {
                    result &= self.player_condition(name, value)?;
                }
                Ok(result)
            },
            _ => Err(ItemError("unsupported item condition")),
        }
    }
    fn player_condition(&self, name: &str, value: &str) -> Result<bool, ItemError> {
        match name {
            "races" => {
                let current = match self.identity.race {
                    Race::Human => "HUMAN",
                    Race::Elf => "ELF",
                    Race::DarkElf => "DARK_ELF",
                    Race::Orc => "ORC",
                    Race::Dwarf => "DWARF",
                };
                let mut matches = false;
                for race in value.split(',') {
                    if !matches!(race, "HUMAN" | "ELF" | "DARK_ELF" | "ORC" | "DWARF" | "KAMAEL") {
                        return Err(ItemError("unknown item race condition"));
                    }
                    matches |= race == current;
                }
                Ok(matches)
            },
            "sex" => Ok(number(value)?
                == match self.identity.appearance.sex {
                    Sex::Male => 0,
                    Sex::Female => 1,
                }),
            "level" => Ok(i64::from(self.level) >= number(value)?),
            "class_id_restriction" => {
                let mut matches = false;
                for class in value.split(',') {
                    matches |= i64::from(self.class_id.0) == number(class.trim())?;
                }
                Ok(matches)
            },
            "flyMounted" => match value {
                "false" => Ok(true),
                "true" => Ok(false),
                _ => Err(ItemError("invalid item mount condition")),
            },
            // DocumentBase.java:611 ignores the XML attribute value.
            "canResurrect" => Ok(self.can_resurrect),
            "weight" => {
                let threshold = number(value)?;
                if self.player.max_load <= 0 {
                    return Ok(true);
                }
                let percent = i128::from(self.player.weight)
                    .checked_sub(i128::from(self.player.weight_penalty_bonus))
                    .and_then(|weight| weight.checked_mul(100))
                    .and_then(|weight| weight.checked_div(i128::from(self.player.max_load)))
                    .ok_or(ItemError("item condition weight overflow"))?;
                Ok(percent < i128::from(threshold))
            },
            "invSize" => {
                let (ordinary, _, _) =
                    super::item_inventory::inventory_totals(&self.player.inventory, self.rules)?;
                let available = i128::from(self.player.ordinary_capacity)
                    .checked_sub(i128::from(number(value)?))
                    .ok_or(ItemError("item condition capacity overflow"))?;
                Ok(i128::from(ordinary) <= available)
            },
            _ => Err(ItemError("unsupported item player predicate")),
        }
    }
}
fn number(value: &str) -> Result<i64, ItemError> {
    value
        .parse::<i32>()
        .map(i64::from)
        .map_err(|_| ItemError("invalid item condition integer"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn predicate(name: &str, value: &str) -> ItemCondition {
        ItemCondition {
            tag: "player".into(),
            attributes: [(name.into(), value.into())].into(),
            text: String::new(),
            xml_ordinal: 0,
            children: Vec::new(),
        }
    }
    #[test]
    fn every_admitted_item_condition_is_executable_for_each_playable_identity() {
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        let player = super::super::item_functions::tests::player();
        let mut identity = crate::domain::character_progression::CharacterIdentity {
            account_id: crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(1)),
            race: Race::Human,
            base_class_id: ClassId(0),
            appearance: crate::domain::character_progression::CharacterAppearance::default(),
        };
        for race in [
            Race::Human,
            Race::Elf,
            Race::DarkElf,
            Race::Orc,
            Race::Dwarf,
        ] {
            identity.race = race;
            for sex in [Sex::Male, Sex::Female] {
                identity.appearance.sex = sex;
                let context = ItemConditionContext {
                    identity: &identity,
                    class_id: ClassId(0),
                    level: 1,
                    player: &player,
                    rules: &bundle.items,
                    can_resurrect: false,
                };
                for cap in bundle
                    .items
                    .parts()
                    .capabilities
                    .values()
                    .filter(|cap| cap.admitted)
                {
                    let item = bundle.items.parts().items.get(&cap.template_id).unwrap();
                    assert!(
                        context.matches(&item.conditions).is_ok(),
                        "{}: {:?}",
                        cap.template_id,
                        item.conditions
                    );
                }
            }
        }
    }
    #[test]
    fn conditions_use_source_boundaries_and_never_invert_unsupported_contexts() {
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        let mut player = super::super::item_functions::tests::player();
        player.max_load = 1000;
        player.weight = 800;
        player.weight_penalty_bonus = 1;
        let identity = crate::domain::character_progression::CharacterIdentity {
            account_id: crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(1)),
            race: Race::Human,
            base_class_id: ClassId(0),
            appearance: crate::domain::character_progression::CharacterAppearance::default(),
        };
        let context = ItemConditionContext {
            identity: &identity,
            class_id: ClassId(1),
            level: 20,
            player: &player,
            rules: &bundle.items,
            can_resurrect: false,
        };
        for (name, value, expected) in [
            ("races", "KAMAEL", false),
            ("races", "HUMAN,ELF", true),
            ("sex", "0", true),
            ("sex", "1", false),
            ("class_id_restriction", "0,1", true),
            ("class_id_restriction", "0", false),
            ("level", "20", true),
            ("level", "21", false),
            ("flyMounted", "false", true),
            ("flyMounted", "true", false),
            ("weight", "79", false),
            ("weight", "80", true),
            ("invSize", "80", true),
            ("invSize", "81", false),
            ("canResurrect", "false", false),
        ] {
            assert_eq!(
                context.matches(&[predicate(name, value)]).unwrap(),
                expected,
                "{name}={value}"
            );
        }
        assert!(context.matches(&[]).unwrap());
        let unsupported = predicate("olympiad", "false");
        for tag in ["and", "or", "not"] {
            let mut children = vec![unsupported.clone()];
            if tag != "not" {
                children.insert(0, predicate("level", "1"));
            }
            assert!(context
                .matches(&[ItemCondition {
                    tag: tag.into(),
                    attributes: BTreeMap::new(),
                    text: String::new(),
                    xml_ordinal: 0,
                    children
                }])
                .is_err());
        }
    }
}
