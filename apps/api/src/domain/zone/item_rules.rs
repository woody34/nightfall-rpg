//! Immutable V6 item rules. Generated values are exact and independent of legacy profiles.

use super::effects::FrozenRatio;
use super::item_angles::AngleRatio;
use super::skill_profiles::ProfileSource;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

macro_rules! record {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(missing_docs)]
        pub struct $name { $(pub $field: $ty),* }
    };
}

record!(ItemCondition {
    tag: String, attributes: BTreeMap<String, String>, text: String,
    xml_ordinal: u32, children: Vec<ItemCondition>
});
record!(ItemFunction {
    stat: String, operation: String, order: i32, xml_ordinal: u32,
    value: FrozenRatio, conditions: Vec<ItemCondition>
});
record!(ItemSkillRef {
    id: u32,
    level: u32,
    kind: String,
    xml_ordinal: u32
});
record!(ItemDefinition {
    template_id: u32, name: String, kind: String, grade: String,
    bodypart: String, bodypart_mask: u64, weapon_type: String, armor_type: String,
    etcitem_type: String, icon: String, handler: String, weight: u32, price: u64,
    crystal_count: u32, enchant_enabled: u32, stackable: bool,
    flags: BTreeMap<String, bool>, properties: BTreeMap<String, String>,
    soulshots: u32, spiritshots: u32, random_damage: u32, attack_range: u32,
    attack_angle: u32, damage_range: String, mp_consume: u32, reuse_delay: u64,
    equip_reuse_delay: u64, shared_reuse_group: i32, reduced_soulshot: Vec<u32>,
    reduced_mp_consume: Vec<u32>, enchant_hp: bool, functions: Vec<ItemFunction>,
    skills: Vec<ItemSkillRef>, enchant4_skills: Vec<ItemSkillRef>, conditions: Vec<ItemCondition>
});
record!(ItemCapability { template_id: u32, admitted: bool, storable: bool,
    blockers: Vec<String>, use_blockers: Vec<String> });
record!(ItemSkillCapability { id: u32, max_level: u32, admitted: bool, blockers: Vec<String> });
record!(OptionCapability { id: u32, admitted: bool, blockers: Vec<String> });
record!(AugmentCapability { kind: String, stone_id: u32, admitted: bool, blockers: Vec<String> });
record!(RecipeCapability { recipe_id: u64, admitted: bool, blockers: Vec<String> });
record!(WeaponTypeRule {
    kind: String,
    ordinal: u32,
    mask: u64,
    r#trait: String
});
record!(ItemGradeRule {
    grade: String,
    id: u8,
    crystal_id: u32,
    armor_bonus: u32,
    weapon_bonus: u32,
    shot_grade: String
});
record!(BlacksmithRule {
    template: String,
    x: i32,
    y: i32,
    radius_millitiles: u32,
    tax_rate: FrozenRatio
});
record!(ItemCapacityRules { inventory: u32, dwarf_inventory: u32, quest_inventory: u32,
    weight_limit: FrozenRatio, load_multiplier: u32, weight_penalty_base: i64,
    overload_thresholds: Vec<u32>, blacksmith: BlacksmithRule, policy: BTreeMap<String, String> });
record!(EnchantChanceRange {
    min: u32,
    max: u32,
    chance: FrozenRatio,
    chance_1e5: u32
});
record!(EnchantRates { name: String, ranges: Vec<EnchantChanceRange> });
record!(EnchantBinding {
    id: u32,
    rules: ItemCondition
});
record!(EnchantScrollRule { template_id: u32, target_grade: String, bonus_rate: FrozenRatio,
    max_enchant: u32, group_id: u32, weapon: bool, blessed: bool, safe: bool, allowed_items: Vec<u32> });
record!(EnchantOptionLevel { level: u32, option_ids: Vec<u32> });
record!(EnchantAttackRule {
    grade: String,
    m_atk: u32,
    p_atk: u32,
    p_atk_two_hand: u32,
    p_atk_bow: u32
});
record!(FuncEnchantRules { attack: Vec<EnchantAttackRule>, safe_levels: u32,
    attack_over_multiplier: u32, defence_over_multiplier: u32, hp_order: i32 });
record!(EnchantHpRule { grade: String, bonuses: Vec<u32> });
record!(ArmorSetRule { id: u32, chest: Vec<u32>, legs: Vec<u32>, head: Vec<u32>,
    gloves: Vec<u32>, feet: Vec<u32>, shield: Vec<u32>, skills: Vec<ItemSkillRef>,
    shield_skills: Vec<ItemSkillRef>, enchant6_skills: Vec<ItemSkillRef>, base_stats: BTreeMap<String, i32> });
record!(AugmentOptionRule { id: u32, functions: Vec<ItemFunction>, skills: Vec<ItemSkillRef> });
record!(AugmentChanceRow {
    kind: String,
    xml_ordinal: u32,
    rules: ItemCondition
});
record!(LifeStoneRule {
    template_id: u32,
    kind: String,
    tier: u32,
    level: u32
});
record!(AugmentCostRule {
    grade: String,
    gemstone_id: u32,
    weapon_count: u32,
    accessory_count: u32
});
record!(CancelRefineFee {
    grade: String,
    crystal_count_below: u64,
    adena: u64
});
record!(AugmentSkillRule {
    option_id: u32,
    id: u32,
    level: u32,
    color: String
});
record!(ItemIngredient {
    template_id: u32,
    count: u64
});
record!(SaRecipeRule { recipe_id: u64, list_id: u32, xml_ordinal: u32, base_template: u32,
    ingredients: Vec<ItemIngredient>, products: Vec<ItemIngredient>, maintain_enchantment: bool, apply_taxes: bool });
record!(SoulCrystalTransition {
    template_id: u32,
    stage: u32,
    leveled_template: u32
});
record!(StarterKitItem {
    template_id: u32,
    count: u64,
    equipped: bool,
    xml_ordinal: u32
});
record!(StarterKitRule { class_id: u32, items: Vec<StarterKitItem> });
record!(CertifiedAngle { degrees: u32, special: String, cos_lower: AngleRatio, cos_upper: AngleRatio,
    cos2_lower: AngleRatio, cos2_upper: AngleRatio, certified: bool, blockers: Vec<String> });
record!(ClassSlotRule {
    base_class_id: u32, key: String,
    p_def_slots: BTreeMap<super::EquipSlot, u32>,
    m_def_slots_xml: BTreeMap<super::EquipSlot, u32>,
    m_def_slots: BTreeMap<super::EquipSlot, u32>
});
record!(ItemRulesParts {
    formula_revision: u32, phase3_hash: [u8; 32], items: BTreeMap<u32, ItemDefinition>,
    capabilities: BTreeMap<u32, ItemCapability>,
    skill_capabilities: BTreeMap<u32, ItemSkillCapability>, option_capabilities: BTreeMap<u32, OptionCapability>,
    augment_capabilities: Vec<AugmentCapability>, recipe_capabilities: BTreeMap<u64, RecipeCapability>,
    weapon_types: Vec<WeaponTypeRule>, grades: Vec<ItemGradeRule>, class_slots: Vec<ClassSlotRule>,
    capacities: ItemCapacityRules, enchant_rates: Vec<EnchantRates>, enchant_bindings: Vec<EnchantBinding>,
    enchant_scrolls: BTreeMap<u32, EnchantScrollRule>, enchant_supports: BTreeMap<u32, EnchantScrollRule>,
    enchant_blacklist: Vec<u32>, enchant_options: BTreeMap<u32, Vec<EnchantOptionLevel>>,
    func_enchant: FuncEnchantRules, enchant_hp: Vec<EnchantHpRule>, enchant_hp_bodyparts: Vec<String>,
    armor_sets: BTreeMap<u32, ArmorSetRule>, augment_options: BTreeMap<u32, AugmentOptionRule>,
    augment_chances: Vec<AugmentChanceRow>, weapon_categories: Vec<u32>, accessory_categories: Vec<u32>,
    augment_selector: String, augment_blacklist: Vec<u32>, inactive_augment_policy: BTreeMap<String, String>, life_stones: BTreeMap<u32, LifeStoneRule>, augment_costs: Vec<AugmentCostRule>,
    cancel_fees: Vec<CancelRefineFee>, augment_skills: Vec<AugmentSkillRule>,
    sa_recipes: BTreeMap<u64, SaRecipeRule>, soul_crystals: BTreeMap<u32, SoulCrystalTransition>,
    soul_crystal_npcs: Vec<ItemCondition>, absorb_policy: String, starter_kits: Vec<StarterKitRule>,
    coordinate_bound_millitiles: u32, angle_proof: String, angles: Vec<CertifiedAngle>,
    sources: Vec<ProfileSource>
});

/// Validated immutable epoch bundle. Decoding repeats validation and verifies the hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Phase4Rules {
    parts: ItemRulesParts,
    hash: [u8; 32],
}
impl Phase4Rules {
    /// Construct after loading and normalizing generated exact literals.
    pub fn try_new(parts: ItemRulesParts) -> Result<Self, String> {
        parts.validate()?;
        let value = serde_json::to_value(&parts).map_err(|error| error.to_string())?;
        let mut bytes = b"nightfall.phase4.rules.1\0".to_vec();
        encode_value(&value, &mut bytes)?;
        Ok(Self {
            parts,
            hash: Sha256::digest(bytes).into(),
        })
    }

    /// Bind the item closure to the actual V6 skill/profile bundle, never a hash supplied
    /// without its content. Legacy V5 registries are not modified by this check.
    pub fn validate_phase3(&self, rules: &super::Phase3Rules) -> Result<(), String> {
        if self.parts.phase3_hash != rules.hash() {
            return Err("Phase4 skill/profile hash mismatch".into());
        }
        for profile in rules.profiles().classes.values() {
            if !self
                .parts
                .class_slots
                .iter()
                .any(|row| row.base_class_id == profile.base_class_id.0)
            {
                return Err("missing class defence slots".into());
            }
        }
        for cap in self
            .parts
            .skill_capabilities
            .values()
            .filter(|cap| cap.admitted)
        {
            let skill = rules
                .skills()
                .get(super::skill::SkillId(cap.id))
                .ok_or_else(|| format!("missing Phase4 skill {}", cap.id))?;
            if skill.max_level != cap.max_level
                || skill.runtime_status != crate::domain::class::RuntimeSkillStatus::Executable
            {
                return Err(format!("Phase4 skill capability disagrees with registry {}", cap.id));
            }
        }
        // Penalties are installed after load/capacity getters, so they cannot feed back
        // into the inputs that select their own rank or the preceding capacity fold.
        for id in [4270, 6209, 6213] {
            let skill = rules
                .skills()
                .get(super::skill::SkillId(id))
                .ok_or_else(|| format!("missing equipment penalty {id}"))?;
            if skill
                .effects
                .iter()
                .flat_map(|handler| &handler.stat_children)
                .any(|modifier| {
                    matches!(
                        modifier.stat.as_str(),
                        "STR"
                            | "DEX"
                            | "CON"
                            | "INT"
                            | "MEN"
                            | "WIT"
                            | "weightLimit"
                            | "weightPenalty"
                            | "inventoryLimit"
                    )
                })
            {
                return Err(format!("cyclic equipment penalty {id}"));
            }
        }
        Ok(())
    }

    /// Full rule contents; mutation requires a newly validated bundle and a new epoch.
    pub const fn parts(&self) -> &ItemRulesParts {
        &self.parts
    }
    /// Separately bound to V6 snapshots and admission.
    pub const fn hash(&self) -> [u8; 32] {
        self.hash
    }
}
impl<'de> Deserialize<'de> for Phase4Rules {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            parts: ItemRulesParts,
            hash: [u8; 32],
        }
        let encoded = Encoded::deserialize(deserializer)?;
        let rules = Self::try_new(encoded.parts).map_err(serde::de::Error::custom)?;
        if rules.hash != encoded.hash {
            return Err(serde::de::Error::custom("Phase4 rules hash mismatch"));
        }
        Ok(rules)
    }
}

// The typed structs above define the grammar. The hash uses explicit value tags, LE
// fixed-width integers and length-delimited strings/containers, independent of JSON text.
fn encode_value(value: &serde_json::Value, bytes: &mut Vec<u8>) -> Result<(), String> {
    use serde_json::Value;
    match value {
        Value::Null => bytes.push(0),
        Value::Bool(value) => bytes.extend_from_slice(&[1, u8::from(*value)]),
        Value::Number(value) => {
            if let Some(value) = value.as_u64() {
                bytes.push(2);
                bytes.extend_from_slice(&value.to_le_bytes());
            } else if let Some(value) = value.as_i64() {
                bytes.push(3);
                bytes.extend_from_slice(&value.to_le_bytes());
            } else {
                return Err("noninteger Phase4 rule scalar".into());
            }
        },
        Value::String(value) => {
            bytes.push(4);
            write_len(value.len(), bytes)?;
            bytes.extend_from_slice(value.as_bytes());
        },
        Value::Array(values) => {
            bytes.push(5);
            write_len(values.len(), bytes)?;
            for value in values {
                encode_value(value, bytes)?;
            }
        },
        Value::Object(values) => {
            bytes.push(6);
            write_len(values.len(), bytes)?;
            // Explicit ordering also holds if serde_json enables preserve_order elsewhere.
            let ordered: BTreeMap<_, _> = values.iter().collect();
            for (key, value) in ordered {
                write_len(key.len(), bytes)?;
                bytes.extend_from_slice(key.as_bytes());
                encode_value(value, bytes)?;
            }
        },
    }
    Ok(())
}
fn write_len(value: usize, bytes: &mut Vec<u8>) -> Result<(), String> {
    bytes.extend_from_slice(
        &u64::try_from(value)
            .map_err(|_| "Phase4 length overflow")?
            .to_le_bytes(),
    );
    Ok(())
}

impl ItemRulesParts {
    fn validate(&self) -> Result<(), String> {
        self.validate_class_slots()?;
        self.validate_items()?;
        self.validate_enchant()?;
        self.validate_references()?;
        self.validate_capabilities()?;
        self.validate_angles()
    }

    fn validate_class_slots(&self) -> Result<(), String> {
        use super::EquipSlot::{
            Chest, Cloak, Feet, Gloves, Head, Lear, Legs, Lfinger, Neck, Rear, Rfinger, Under,
        };
        let physical =
            std::collections::BTreeSet::from([Chest, Legs, Head, Feet, Gloves, Under, Cloak]);
        let magical = std::collections::BTreeSet::from([Rear, Lear, Rfinger, Lfinger, Neck]);
        let mut previous = None;
        if self.class_slots.is_empty() {
            return Err("missing class defence slots".into());
        }
        for row in &self.class_slots {
            let mut expected_magic = row.m_def_slots_xml.clone();
            let finger = *expected_magic
                .get(&Rfinger)
                .ok_or("missing right finger base")?;
            expected_magic.insert(Lfinger, finger);
            if row.key.is_empty()
                || previous.is_some_and(|id| id >= row.base_class_id)
                || row
                    .p_def_slots
                    .keys()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    != physical
                || row
                    .m_def_slots_xml
                    .keys()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    != magical
                || row.m_def_slots != expected_magic
            {
                return Err("invalid class defence slots".into());
            }
            previous = Some(row.base_class_id);
        }
        Ok(())
    }

    fn validate_angles(&self) -> Result<(), String> {
        if self.coordinate_bound_millitiles == 0
            || self.angle_proof.is_empty()
            || self.angles.len() != 361
        {
            return Err("incomplete certified angle table".into());
        }
        for (index, angle) in self.angles.iter().enumerate() {
            if usize::try_from(angle.degrees).ok() != Some(index)
                || angle.cos_lower.compare(&angle.cos_upper).is_gt()
                || angle.cos2_lower.compare(&angle.cos2_upper).is_gt()
                || angle
                    .cos2_lower
                    .compare_integer_ratio(0, 1)
                    .map_err(|error| error.to_string())?
                    .is_lt()
                || angle.certified != angle.blockers.is_empty()
            {
                return Err("invalid certified angle interval".into());
            }
            let squared = match angle.degrees {
                0 | 360 => Some((1, 1)),
                60 | 300 => Some((3, 4)),
                90 | 270 => Some((1, 2)),
                120 | 240 => Some((1, 4)),
                180 => Some((0, 1)),
                _ => None,
            };
            if let Some((n, d)) = squared {
                if !angle
                    .cos2_lower
                    .compare_integer_ratio(n, d)
                    .map_err(|error| error.to_string())?
                    .is_eq()
                    || !angle
                        .cos2_upper
                        .compare_integer_ratio(n, d)
                        .map_err(|error| error.to_string())?
                        .is_eq()
                {
                    return Err("incorrect exact special-angle boundary".into());
                }
            }
        }
        Ok(())
    }

    fn validate_capabilities(&self) -> Result<(), String> {
        let valid = |admitted: bool, blockers: &[String]| admitted == blockers.is_empty();
        for (id, cap) in &self.skill_capabilities {
            if *id == 0
                || *id != cap.id
                || cap.max_level == 0
                || !valid(cap.admitted, &cap.blockers)
            {
                return Err("invalid skill capability".into());
            }
        }
        if self.option_capabilities.len() != self.augment_options.len()
            || self.recipe_capabilities.len() != self.sa_recipes.len()
        {
            return Err("option/recipe capability coverage mismatch".into());
        }
        for (id, cap) in &self.option_capabilities {
            if *id != cap.id
                || !self.augment_options.contains_key(id)
                || !valid(cap.admitted, &cap.blockers)
            {
                return Err("invalid option capability".into());
            }
        }
        for (id, cap) in &self.recipe_capabilities {
            if *id != cap.recipe_id
                || !self.sa_recipes.contains_key(id)
                || !valid(cap.admitted, &cap.blockers)
            {
                return Err("invalid recipe capability".into());
            }
        }
        let mut stones = std::collections::BTreeSet::new();
        for cap in &self.augment_capabilities {
            if !stones.insert((&cap.kind, cap.stone_id))
                || !self.life_stones.contains_key(&cap.stone_id)
                || !valid(cap.admitted, &cap.blockers)
            {
                return Err("invalid augmentation capability".into());
            }
        }
        for (id, item) in &self.items {
            let admitted = self.capabilities.get(id).is_some_and(|cap| cap.admitted);
            for skill in item.skills.iter().chain(&item.enchant4_skills) {
                self.validate_skill_ref(skill, admitted)?;
            }
        }
        for (id, option) in &self.augment_options {
            let admitted = self
                .option_capabilities
                .get(id)
                .is_some_and(|cap| cap.admitted);
            for skill in &option.skills {
                self.validate_skill_ref(skill, admitted)?;
            }
        }
        for set in self.armor_sets.values() {
            for skill in set
                .skills
                .iter()
                .chain(&set.shield_skills)
                .chain(&set.enchant6_skills)
            {
                self.validate_skill_ref(skill, false)?;
            }
        }
        Ok(())
    }

    fn validate_skill_ref(&self, skill: &ItemSkillRef, executable: bool) -> Result<(), String> {
        if skill.id == 0
            || skill.level == 0
            || !self
                .skill_capabilities
                .get(&skill.id)
                .is_some_and(|cap| !executable || (cap.max_level >= skill.level && cap.admitted))
        {
            return Err(format!("invalid skill reference {}:{}", skill.id, skill.level));
        }
        Ok(())
    }

    fn validate_items(&self) -> Result<(), String> {
        if self.formula_revision != 1 {
            return Err("unsupported Phase4 formula revision".into());
        }
        let expected_grades = ["NONE", "D", "C", "B", "A", "S", "S80", "S84"];
        if self.grades.len() != expected_grades.len()
            || self.grades.iter().zip(expected_grades).enumerate().any(
                |(index, (row, expected))| row.grade != expected || usize::from(row.id) != index,
            )
        {
            return Err("Phase4 grade table is incomplete or unordered".into());
        }
        let grades: std::collections::BTreeSet<_> =
            self.grades.iter().map(|row| row.grade.as_str()).collect();
        if self.capabilities.len() != self.items.len() {
            return Err("item capability coverage mismatch".into());
        }
        for (id, item) in &self.items {
            if *id == 0
                || *id > i32::MAX as u32
                || item.template_id != *id
                || !matches!(item.kind.as_str(), "Weapon" | "Armor" | "EtcItem")
                || !grades.contains(item.grade.as_str())
            {
                return Err(format!("invalid item {id}"));
            }
            let capability = self
                .capabilities
                .get(id)
                .ok_or_else(|| format!("missing capability {id}"))?;
            if capability.template_id != *id
                || (capability.admitted
                    && (!capability.storable || !capability.blockers.is_empty()))
                || (!capability.admitted
                    && capability.blockers.is_empty()
                    && capability.use_blockers.is_empty())
            {
                return Err(format!("inconsistent capability {id}"));
            }
            validate_functions(&item.functions)?;
            for reduced in [&item.reduced_soulshot, &item.reduced_mp_consume] {
                if !reduced.is_empty()
                    && (reduced.len() != 2 || reduced.first().is_some_and(|chance| *chance > 100))
                {
                    return Err(format!("invalid item reduction {id}"));
                }
            }
        }
        Ok(())
    }

    fn validate_enchant(&self) -> Result<(), String> {
        let exists = |id: &u32| self.items.contains_key(id);
        let grades: std::collections::BTreeSet<_> =
            self.grades.iter().map(|row| row.grade.as_str()).collect();
        for rates in &self.enchant_rates {
            if rates.ranges.is_empty() {
                return Err("empty enchant rate table".into());
            }
            let mut next = 0;
            for row in &rates.ranges {
                if row.min != next || row.max < row.min || row.chance_1e5 > 100_000 {
                    return Err(format!("invalid enchant range {}", rates.name));
                }
                let chance = row
                    .chance
                    .value()
                    .checked_mul(super::skill_math::ExactRatio::whole(1000))
                    .map_err(|error| error.to_string())?;
                if chance != super::skill_math::ExactRatio::whole(i64::from(row.chance_1e5)) {
                    return Err("enchant integer adaptation disagrees with exact percentage".into());
                }
                next = row.max.checked_add(1).ok_or("enchant range overflow")?;
            }
        }
        for scrolls in [&self.enchant_scrolls, &self.enchant_supports] {
            for (id, row) in scrolls {
                if *id != row.template_id
                    || !exists(id)
                    || !grades.contains(row.target_grade.as_str())
                    || !row.allowed_items.iter().all(exists)
                    || !self
                        .enchant_bindings
                        .iter()
                        .any(|binding| binding.id == row.group_id)
                {
                    return Err(format!("invalid enchant scroll/support {id}"));
                }
            }
        }
        for (id, levels) in &self.enchant_options {
            if !exists(id) {
                return Err(format!("unknown enchant option target {id}"));
            }
            let mut previous = None;
            for row in levels {
                if previous.is_some_and(|level| row.level <= level)
                    || row
                        .option_ids
                        .iter()
                        .any(|id| !self.augment_options.contains_key(id))
                {
                    return Err("invalid enchant option level/reference".into());
                }
                previous = Some(row.level);
            }
        }
        Ok(())
    }

    fn validate_references(&self) -> Result<(), String> {
        let exists = |id: &u32| self.items.contains_key(id);
        for (id, set) in &self.armor_sets {
            if *id == 0
                || *id != set.id
                || set.chest.is_empty()
                || [
                    &set.chest,
                    &set.legs,
                    &set.head,
                    &set.gloves,
                    &set.feet,
                    &set.shield,
                ]
                .into_iter()
                .any(|ids| !ids.iter().all(exists))
            {
                return Err(format!("invalid armour-set references {id}"));
            }
        }
        for (id, option) in &self.augment_options {
            if *id == 0 || *id > u32::from(u16::MAX) || *id != option.id {
                return Err("invalid option identity".into());
            }
            validate_functions(&option.functions)?;
        }
        for (id, stone) in &self.life_stones {
            if *id != stone.template_id || !exists(id) || stone.level == 0 {
                return Err("invalid life stone".into());
            }
        }
        for (id, recipe) in &self.sa_recipes {
            let expected = u64::from(recipe.list_id)
                .checked_mul(1_u64 << 32)
                .and_then(|base| base.checked_add(u64::from(recipe.xml_ordinal)))
                .ok_or("recipe identity overflow")?;
            if *id != recipe.recipe_id
                || *id != expected
                || !exists(&recipe.base_template)
                || recipe.ingredients.is_empty()
                || recipe.products.is_empty()
                || recipe
                    .ingredients
                    .iter()
                    .chain(&recipe.products)
                    .any(|row| {
                        row.count == 0 || row.count > i64::MAX as u64 || !exists(&row.template_id)
                    })
                || recipe
                    .ingredients
                    .iter()
                    .filter(|row| row.template_id == recipe.base_template)
                    .count()
                    != 1
            {
                return Err(format!("invalid SA recipe {id}"));
            }
        }
        for (id, crystal) in &self.soul_crystals {
            if *id != crystal.template_id || !exists(id) || !exists(&crystal.leveled_template) {
                return Err("invalid soul-crystal transition".into());
            }
        }
        let mut classes = std::collections::BTreeSet::new();
        for kit in &self.starter_kits {
            if !classes.insert(kit.class_id) || kit.items.is_empty() {
                return Err("duplicate/empty starter kit".into());
            }
            for (ordinal, item) in kit.items.iter().enumerate() {
                if usize::try_from(item.xml_ordinal).ok() != Some(ordinal)
                    || item.count == 0
                    || item.count > i64::MAX as u64
                    || !exists(&item.template_id)
                {
                    return Err("invalid starter kit row".into());
                }
            }
        }
        if self.capacities.inventory == 0
            || self.capacities.dwarf_inventory == 0
            || self.capacities.quest_inventory == 0
            || self.capacities.load_multiplier == 0
            || self.capacities.overload_thresholds.len() != 4
            || self
                .capacities
                .overload_thresholds
                .windows(2)
                .any(|pair| pair.first() >= pair.last())
            || self.capacities.blacksmith.tax_rate.value()
                != super::skill_math::ExactRatio::whole(0)
        {
            return Err("invalid Phase4 capacity/blacksmith policy".into());
        }
        Ok(())
    }
}

fn validate_functions(functions: &[ItemFunction]) -> Result<(), String> {
    let mut previous = None;
    for function in functions {
        if function.stat.is_empty()
            || previous.is_some_and(|ordinal| ordinal >= function.xml_ordinal)
        {
            return Err("unordered/invalid item functions".into());
        }
        previous = Some(function.xml_ordinal);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // Full pinned E1 bundle, shared once across rule/snapshot tests.
    fn fixture() -> ItemRulesParts {
        crate::infrastructure::item_data::generated_test_rules()
            .parts()
            .clone()
    }

    #[test]
    fn separately_hashed_rules_roundtrip_and_reject_changed_content() {
        let rules = Phase4Rules::try_new(fixture()).unwrap();
        let bytes = serde_json::to_vec(&rules).unwrap();
        assert_eq!(serde_json::from_slice::<Phase4Rules>(&bytes).unwrap(), rules);
        let mut changed = fixture();
        changed.items.get_mut(&2369).unwrap().weight = 1601;
        let other = Phase4Rules::try_new(changed).unwrap();
        assert_ne!(other.hash(), rules.hash());
        let mut forged = serde_json::to_value(other).unwrap();
        forged
            .as_object_mut()
            .unwrap()
            .insert("hash".into(), serde_json::to_value(rules.hash()).unwrap());
        assert!(serde_json::from_value::<Phase4Rules>(forged).is_err());
    }

    #[test]
    fn references_function_order_and_unknown_fields_are_not_normalized() {
        let mut parts = fixture();
        parts.capabilities.clear();
        assert!(Phase4Rules::try_new(parts).is_err());
        let mut parts = fixture();
        parts.items.get_mut(&2369).unwrap().functions.reverse();
        assert!(Phase4Rules::try_new(parts).is_err());
        let mut parts = fixture();
        parts.grades.reverse();
        assert!(Phase4Rules::try_new(parts).is_err());
        let mut value = serde_json::to_value(fixture()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), serde_json::Value::Null);
        assert!(serde_json::from_value::<ItemRulesParts>(value).is_err());
    }

    #[test]
    fn binary_rule_encoding_keeps_types_lengths_and_order() {
        let encode = |text: &str| {
            let mut bytes = Vec::new();
            encode_value(&serde_json::from_str(text).unwrap(), &mut bytes).unwrap();
            bytes
        };
        assert_eq!(encode(r#"{"b":2,"a":1}"#), encode(r#"{"a":1,"b":2}"#));
        assert_ne!(encode(r#"["ab","c"]"#), encode(r#"["a","bc"]"#));
        assert_ne!(encode("1"), encode(r#""1""#));
        assert_ne!(encode("[1,2]"), encode("[2,1]"));
        assert!(encode_value(&serde_json::from_str("0.5").unwrap(), &mut Vec::new()).is_err());
    }
}
