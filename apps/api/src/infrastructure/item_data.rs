//! Phase 4 data boundary. Generated TOML retains source diagnostics; runtime rules retain
//! typed semantic values only. Historical data loaders never consult this directory.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value};

use crate::domain::zone::item_rules::{ItemRulesParts, Phase4Rules};
use crate::domain::zone::skill_math::ExactRatio;

/// D27/D31 immutable build inputs resolved together. Only these hashes enter V6 records.
#[derive(Debug, Clone)]
pub struct Phase4RuleBundle {
    /// Item definitions and every item-policy input.
    pub items: Arc<Phase4Rules>,
    /// Full V6 skills and their physical/magic/profile/policy inputs.
    pub skills: Arc<crate::domain::zone::Phase3Rules>,
}

/// Compile once from embedded sources; no filesystem, database or current-data fallback
/// is permitted when a recorded hash differs from this executable's bundle.
pub fn embedded_phase4_bundle() -> Result<Phase4RuleBundle, String> {
    static BUNDLE: OnceLock<Result<Phase4RuleBundle, String>> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let skills = embedded_phase4_skills()?;
            let items = load_embedded_phase4_rules(skills.hash())?;
            items.validate_phase3(&skills)?;
            Ok(Phase4RuleBundle {
                items: Arc::new(items),
                skills,
            })
        })
        .clone()
}

/// The embedded V6 skill composition alone. Comparing a skill hash never assembles the
/// item tables (the expensive part of a bundle).
fn embedded_phase4_skills() -> Result<Arc<crate::domain::zone::Phase3Rules>, String> {
    static SKILLS: OnceLock<Result<Arc<crate::domain::zone::Phase3Rules>, String>> =
        OnceLock::new();
    SKILLS
        .get_or_init(|| {
            compose_phase4_skills(&super::skill_data::Phase3DataSource::embedded()).map(Arc::new)
        })
        .clone()
}

fn compose_phase4_skills(
    source: &super::skill_data::Phase3DataSource,
) -> Result<crate::domain::zone::Phase3Rules, String> {
    use super::skill_data::{load_phase3_rules, load_phase4_skills, SkillSource};
    let base = load_phase3_rules(source, &[]).map_err(|error| error.to_string())?;
    let skills = load_phase4_skills(&SkillSource::phase4_embedded(), &[])
        .map_err(|error| error.to_string())?;
    base.with_phase4_skills(skills)
        .map_err(|errors| errors.join("; "))
}

/// V6 bundles composed at startup over a configured Phase 3 data source (`RULES_DIR`
/// overlays, simulation fixtures), keyed by their exact (item, skill) hashes.
static INSTALLED: parking_lot::Mutex<Vec<Phase4RuleBundle>> = parking_lot::Mutex::new(Vec::new());

/// Compose the V6 bundle over `source` exactly as [`embedded_phase4_bundle`] composes it over
/// the embedded Phase 3 data: the configured Phase 3 tables (skills, profiles incl. NPC
/// profiles, PvP/regen/constants, geometry, NPC skills) take the embedded Phase 4 skill closure
/// (D31) and the embedded item tables (D27). The skill hash is the Phase 3 rules hash of that
/// composition (V5 hashes an overlay the same way) and the item hash binds it through the
/// manifest's `phase3_hash`; with the embedded source both equal the embedded bundle's.
/// A non-embedded composition is installed so this process can record and recover it.
pub fn configured_phase4_bundle(
    source: &super::skill_data::Phase3DataSource,
) -> Result<Phase4RuleBundle, String> {
    let skills = compose_phase4_skills(source)?;
    if skills.hash() == embedded_phase4_skills()?.hash() {
        return embedded_phase4_bundle();
    }
    if let Some(known) = INSTALLED
        .lock()
        .iter()
        .find(|known| known.skills.hash() == skills.hash())
    {
        return Ok(known.clone());
    }
    // The item tables are the embedded ones (D27); only their Phase 3 binding differs.
    let items = load_embedded_phase4_rules(skills.hash())?;
    items.validate_phase3(&skills)?;
    let bundle = Phase4RuleBundle {
        items: Arc::new(items),
        skills: Arc::new(skills),
    };
    let mut installed = INSTALLED.lock();
    if !installed.iter().any(|known| {
        known.items.hash() == bundle.items.hash() && known.skills.hash() == bundle.skills.hash()
    }) {
        installed.push(bundle.clone());
    }
    Ok(bundle)
}

/// Configured Phase 3 sources composed only when a recorded hash misses (replay tools):
/// registering costs nothing until a V6 epoch actually references the composition.
static DEFERRED: parking_lot::Mutex<Vec<super::skill_data::Phase3DataSource>> =
    parking_lot::Mutex::new(Vec::new());

/// Register a configured source (for example `RULES_DIR`) for lazy composition by
/// [`resolve_phase4_bundle`]. Composition errors surface on the lookup that needs it.
pub fn register_phase4_source(source: super::skill_data::Phase3DataSource) {
    DEFERRED.lock().push(source);
}

/// Resolve both exact recorded hashes before constructing a V6 zone or decoding its
/// dynamic sidecars: the embedded bundle, or one this process composed from its configured
/// data (at startup, or lazily from a registered source on the first miss). A later build
/// must never substitute a different capability closure.
pub fn resolve_phase4_bundle(
    item_hash: [u8; 32],
    skill_hash: [u8; 32],
) -> Result<Phase4RuleBundle, String> {
    let find = || {
        INSTALLED
            .lock()
            .iter()
            .find(|known| known.items.hash() == item_hash && known.skills.hash() == skill_hash)
            .cloned()
    };
    if embedded_phase4_skills()?.hash() == skill_hash {
        let bundle = embedded_phase4_bundle()?;
        if bundle.items.hash() == item_hash {
            return Ok(bundle);
        }
    } else if let Some(bundle) = find() {
        return Ok(bundle);
    }
    let pending = std::mem::take(&mut *DEFERRED.lock());
    let mut failures = Vec::new();
    for source in pending {
        if let Err(error) = configured_phase4_bundle(&source) {
            failures.push(error);
        }
    }
    find().ok_or_else(|| {
        let mut message = String::from(
            "recorded V6 rules are unavailable in this process: an epoch recorded over a \
             RULES_DIR overlay resolves only with PHASE4_ENABLED=1 and the RULES_DIR it was \
             recorded with (replay tools: set that RULES_DIR)",
        );
        if !failures.is_empty() {
            message.push_str("; configured source: ");
            message.push_str(&failures.join("; "));
        }
        message
    })
}

/// D27 item tables parsed once per process; every Phase 3 binding reuses them.
fn embedded_item_sources() -> Result<Map<String, Value>, String> {
    static SOURCES: OnceLock<Result<Map<String, Value>, String>> = OnceLock::new();
    SOURCES
        .get_or_init(|| load_sources(&ItemDataSource::Embedded))
        .clone()
}

/// Read one immutable generated bundle. This path is not enabled by legacy bootstrap.
/// The caller supplies the hash of the exact Phase3 skill/profile extension for the epoch.
pub fn load_phase4_rules(directory: &Path, phase3_hash: [u8; 32]) -> Result<Phase4Rules, String> {
    assemble(&ItemDataSource::Directory(directory), phase3_hash)
}

/// D27: replay resolves only the compiled generated bundle, then verifies its full hash.
pub fn load_embedded_phase4_rules(phase3_hash: [u8; 32]) -> Result<Phase4Rules, String> {
    bind(embedded_item_sources()?, phase3_hash)
}

fn assemble(directory: &ItemDataSource<'_>, phase3_hash: [u8; 32]) -> Result<Phase4Rules, String> {
    bind(load_sources(directory)?, phase3_hash)
}

fn bind(mut parts: Map<String, Value>, phase3_hash: [u8; 32]) -> Result<Phase4Rules, String> {
    parts.insert(
        "phase3_hash".into(),
        serde_json::to_value(phase3_hash).map_err(|e| e.to_string())?,
    );
    let normalized = normalize(Value::Object(parts))?;
    let parts: ItemRulesParts = serde_json::from_value(normalized)
        .map_err(|error| format!("Phase4 typed rules: {error}"))?;
    Phase4Rules::try_new(parts)
}

fn load_sources(directory: &ItemDataSource<'_>) -> Result<Map<String, Value>, String> {
    let mut parts = Map::new();
    let mut manifest = read_table(
        directory,
        "rules_manifest.toml",
        &[
            "domain_separator",
            "formula_revision",
            "files",
            "skill_bundle_sha256",
            "profile_bundle_sha256",
            "data_sha256",
        ],
    )?;
    if take(&mut manifest, "domain_separator")?.as_str() != Some("nightfall.phase4.rules.1\0") {
        return Err("invalid Phase4 manifest domain".into());
    }
    parts.insert("formula_revision".into(), take(&mut manifest, "formula_revision")?);
    parts.insert("sources".into(), Value::Array(Vec::new()));
    load_items(directory, &mut parts)?;
    load_enchantment(directory, &mut parts)?;
    load_equipment_extras(directory, &mut parts)?;
    Ok(parts)
}

fn load_items(
    directory: &ItemDataSource<'_>,
    parts: &mut Map<String, Value>,
) -> Result<(), String> {
    let mut items = Vec::new();
    for file in ["weapons.toml", "armors.toml", "etc_items.toml"] {
        let mut document = read_table(directory, file, &["items"])?;
        items.extend(take_rows(&mut document, "items")?);
    }
    insert_index(parts, "items", items, "template_id")?;
    let mut capabilities = read_table(
        directory,
        "capabilities.toml",
        &["items", "skills", "options", "augmentation", "recipes"],
    )?;
    insert_index(parts, "capabilities", take_rows(&mut capabilities, "items")?, "template_id")?;
    for (source, target, key) in [
        ("skills", "skill_capabilities", "id"),
        ("options", "option_capabilities", "id"),
        ("recipes", "recipe_capabilities", "recipe_id"),
    ] {
        insert_index(parts, target, take_rows(&mut capabilities, source)?, key)?;
    }
    parts.insert("augment_capabilities".into(), take(&mut capabilities, "augmentation")?);
    for (file, source, target) in [
        ("weapon_types.toml", "types", "weapon_types"),
        ("grades.toml", "grades", "grades"),
        ("class_slots.toml", "classes", "class_slots"),
        ("starter_kits.toml", "kits", "starter_kits"),
    ] {
        let mut table = read_table(directory, file, &[source])?;
        parts.insert(target.into(), take(&mut table, source)?);
    }
    // D75: hash-bound unarmed bases (class_combat.toml), keyed by base class.
    let mut combat = read_table(directory, "class_combat.toml", &["unarmed", "classes", "fists"])?;
    parts.insert("class_combat".into(), take(&mut combat, "classes")?);
    parts.insert("fists_weapons".into(), take(&mut combat, "fists")?);
    parts.insert("unarmed_combat".into(), take(&mut combat, "unarmed")?);
    let mut capacity = read_table(
        directory,
        "capacities.toml",
        &[
            "inventory",
            "dwarf_inventory",
            "quest_inventory",
            "weight_limit",
            "load_multiplier",
            "weight_penalty_base",
            "overload_thresholds",
            "blacksmith",
            "policy",
        ],
    )?;
    capacity.remove("schema");
    capacity.remove("sources");
    parts.insert("capacities".into(), Value::Object(capacity));
    Ok(())
}

fn load_enchantment(
    directory: &ItemDataSource<'_>,
    parts: &mut Map<String, Value>,
) -> Result<(), String> {
    let mut groups = read_table(directory, "enchant_groups.toml", &["rates", "bindings"])?;
    parts.insert("enchant_rates".into(), take(&mut groups, "rates")?);
    let bindings = take_rows(&mut groups, "bindings")?
        .into_iter()
        .map(|mut row| {
            rename_raw(&mut row)?;
            Ok(row)
        })
        .collect::<Result<Vec<_>, String>>()?;
    parts.insert("enchant_bindings".into(), Value::Array(bindings));
    let mut scrolls =
        read_table(directory, "enchant_scrolls.toml", &["scrolls", "supports", "blacklist"])?;
    for (source, target) in [
        ("scrolls", "enchant_scrolls"),
        ("supports", "enchant_supports"),
    ] {
        insert_index(parts, target, take_rows(&mut scrolls, source)?, "template_id")?;
    }
    parts.insert("enchant_blacklist".into(), take(&mut scrolls, "blacklist")?);
    let mut options = read_table(directory, "enchant_options.toml", &["items"])?;
    let mut enchant_options = Map::new();
    for row in take_rows(&mut options, "items")? {
        let mut row = object(row)?;
        let id = take(&mut row, "template_id")?
            .as_u64()
            .ok_or("invalid enchant option template")?
            .to_string();
        if enchant_options
            .insert(id, take(&mut row, "levels")?)
            .is_some()
        {
            return Err("duplicate enchant option target".into());
        }
    }
    parts.insert("enchant_options".into(), Value::Object(enchant_options));
    let mut functions = read_table(
        directory,
        "func_enchant.toml",
        &[
            "attack",
            "safe_levels",
            "attack_over_multiplier",
            "defence_over_multiplier",
            "hp_order",
        ],
    )?;
    functions.remove("schema");
    functions.remove("sources");
    parts.insert("func_enchant".into(), Value::Object(functions));
    let mut hp = read_table(directory, "enchant_hp.toml", &["grades", "eligible_bodyparts"])?;
    parts.insert("enchant_hp".into(), take(&mut hp, "grades")?);
    parts.insert("enchant_hp_bodyparts".into(), take(&mut hp, "eligible_bodyparts")?);
    Ok(())
}

fn load_equipment_extras(
    directory: &ItemDataSource<'_>,
    parts: &mut Map<String, Value>,
) -> Result<(), String> {
    for (file, source, target, key) in [
        ("armor_sets.toml", "sets", "armor_sets", "id"),
        ("augmentation_options.toml", "options", "augment_options", "id"),
        ("sa_variants.toml", "recipes", "sa_recipes", "recipe_id"),
    ] {
        let mut table = read_table(directory, file, &[source])?;
        let mut rows = take_rows(&mut table, source)?;
        if target == "augment_options" {
            for row in &mut rows {
                object_mut(row)?.remove("name");
            }
        }
        insert_index(parts, target, rows, key)?;
    }
    let mut chances = read_table(
        directory,
        "augmentation_chances.toml",
        &[
            "rows",
            "weapon_categories",
            "accessory_categories",
            "selector",
            "stones",
            "costs",
            "cancel_fees",
            "inactive_policy",
            "blacklist",
        ],
    )?;
    let rows = take_rows(&mut chances, "rows")?
        .into_iter()
        .map(|mut row| {
            rename_raw(&mut row)?;
            Ok(row)
        })
        .collect::<Result<Vec<_>, String>>()?;
    parts.insert("augment_chances".into(), Value::Array(rows));
    for (source, target) in [
        ("weapon_categories", "weapon_categories"),
        ("accessory_categories", "accessory_categories"),
        ("selector", "augment_selector"),
        ("blacklist", "augment_blacklist"),
        ("inactive_policy", "inactive_augment_policy"),
        ("costs", "augment_costs"),
        ("cancel_fees", "cancel_fees"),
    ] {
        parts.insert(target.into(), take(&mut chances, source)?);
    }
    insert_index(parts, "life_stones", take_rows(&mut chances, "stones")?, "template_id")?;
    let mut skills = read_table(directory, "augmentation_skills.toml", &["skills"])?;
    parts.insert("augment_skills".into(), take(&mut skills, "skills")?);
    let mut crystals =
        read_table(directory, "soul_crystals.toml", &["transitions", "npcs", "absorb_policy"])?;
    insert_index(parts, "soul_crystals", take_rows(&mut crystals, "transitions")?, "template_id")?;
    let npcs = take_rows(&mut crystals, "npcs")?
        .into_iter()
        .map(|row| take(&mut object(row)?, "raw"))
        .collect::<Result<Vec<_>, _>>()?;
    parts.insert("soul_crystal_npcs".into(), Value::Array(npcs));
    parts.insert("absorb_policy".into(), take(&mut crystals, "absorb_policy")?);
    let mut angles = read_table(
        directory,
        "angle_table.toml",
        &["coordinate_bound_millitiles", "proof", "angles"],
    )?;
    for (source, target) in [
        ("coordinate_bound_millitiles", "coordinate_bound_millitiles"),
        ("proof", "angle_proof"),
        ("angles", "angles"),
    ] {
        parts.insert(target.into(), take(&mut angles, source)?);
    }
    Ok(())
}

enum ItemDataSource<'a> {
    Embedded,
    Directory(&'a Path),
}
impl ItemDataSource<'_> {
    fn read(&self, file: &str) -> Result<Cow<'static, str>, String> {
        match self {
            Self::Directory(path) => std::fs::read_to_string(path.join(file))
                .map(Cow::Owned)
                .map_err(|error| format!("{file}: {error}")),
            Self::Embedded => Ok(Cow::Borrowed(match file {
                "angle_table.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/angle_table.toml")
                },
                "armor_sets.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/armor_sets.toml")
                },
                "armors.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/armors.toml")
                },
                "augmentation_chances.toml" => include_str!(
                    "../../../../packages/data/tables-phase4/augmentation_chances.toml"
                ),
                "augmentation_options.toml" => include_str!(
                    "../../../../packages/data/tables-phase4/augmentation_options.toml"
                ),
                "augmentation_skills.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/augmentation_skills.toml")
                },
                "capabilities.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/capabilities.toml")
                },
                "class_combat.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/class_combat.toml")
                },
                "class_slots.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/class_slots.toml")
                },
                "capacities.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/capacities.toml")
                },
                "enchant_groups.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/enchant_groups.toml")
                },
                "enchant_hp.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/enchant_hp.toml")
                },
                "enchant_options.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/enchant_options.toml")
                },
                "enchant_scrolls.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/enchant_scrolls.toml")
                },
                "etc_items.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/etc_items.toml")
                },
                "func_enchant.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/func_enchant.toml")
                },
                "grades.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/grades.toml")
                },
                "rules_manifest.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/rules_manifest.toml")
                },
                "sa_variants.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/sa_variants.toml")
                },
                "soul_crystals.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/soul_crystals.toml")
                },
                "starter_kits.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/starter_kits.toml")
                },
                "weapon_types.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/weapon_types.toml")
                },
                "weapons.toml" => {
                    include_str!("../../../../packages/data/tables-phase4/weapons.toml")
                },
                _ => return Err(format!("unknown embedded Phase4 table {file}")),
            })),
        }
    }
}

fn read_table(
    directory: &ItemDataSource<'_>,
    file: &str,
    fields: &[&str],
) -> Result<Map<String, Value>, String> {
    let text = directory.read(file)?;
    let parsed: toml::Value = toml::from_str(&text).map_err(|error| format!("{file}: {error}"))?;
    let table = object(serde_json::to_value(parsed).map_err(|error| error.to_string())?)?;
    if table.get("schema").and_then(Value::as_u64) != Some(1)
        || !table.get("sources").is_some_and(Value::is_array)
        || fields.iter().any(|field| !table.contains_key(*field))
        || table.keys().any(|field| {
            field != "schema" && field != "sources" && !fields.contains(&field.as_str())
        })
    {
        return Err(format!("{file}: invalid schema or fields"));
    }
    Ok(table)
}
fn object(value: Value) -> Result<Map<String, Value>, String> {
    match value {
        Value::Object(value) => Ok(value),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err("expected Phase4 object".into())
        },
    }
}
fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, String> {
    value
        .as_object_mut()
        .ok_or_else(|| "expected Phase4 object".into())
}
fn take(table: &mut Map<String, Value>, key: &str) -> Result<Value, String> {
    table
        .remove(key)
        .ok_or_else(|| format!("missing Phase4 field {key}"))
}
fn take_rows(table: &mut Map<String, Value>, key: &str) -> Result<Vec<Value>, String> {
    match take(table, key)? {
        Value::Array(rows) => Ok(rows),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            Err(format!("{key}: expected array"))
        },
    }
}
fn rename_raw(row: &mut Value) -> Result<(), String> {
    let row = object_mut(row)?;
    let raw = take(row, "raw")?;
    row.insert("rules".into(), raw);
    Ok(())
}
fn insert_index(
    parts: &mut Map<String, Value>,
    name: &str,
    rows: Vec<Value>,
    key: &str,
) -> Result<(), String> {
    let mut index = BTreeMap::new();
    for row in rows {
        let id = row
            .get(key)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{name}: missing numeric {key}"))?;
        if index.insert(id.to_string(), row).is_some() {
            return Err(format!("{name}: duplicate {key}"));
        }
    }
    parts.insert(name.into(), serde_json::to_value(index).map_err(|error| error.to_string())?);
    Ok(())
}
fn normalize(value: Value) -> Result<Value, String> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(normalize)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => {
            let mut result = Map::new();
            for (key, value) in values {
                if matches!(key.as_str(), "source" | "raw" | "line") {
                    continue;
                }
                let value = if matches!(
                    key.as_str(),
                    "value" | "chance" | "bonus_rate" | "weight_limit" | "tax_rate"
                ) && value.is_string()
                {
                    let ratio =
                        ExactRatio::parse_literal(value.as_str().ok_or("invalid exact literal")?)
                            .map_err(|error| error.to_string())?;
                    let (n, d) = ratio.parts();
                    Value::String(format!("{n}/{d}"))
                } else {
                    normalize(value)?
                };
                result.insert(key, value);
            }
            Ok(Value::Object(result))
        },
        Value::Number(ref n) if n.as_u64().is_none() && n.as_i64().is_none() => {
            Err("noninteger Phase4 numeric literal".into())
        },
        value @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => Ok(value),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) fn generated_test_rules() -> &'static Phase4Rules {
    static RULES: std::sync::OnceLock<Phase4Rules> = std::sync::OnceLock::new();
    RULES.get_or_init(|| load_embedded_phase4_rules([1; 32]).unwrap())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn replay_resolves_exact_compiled_item_and_skill_hashes_without_fallback() {
        let bundle = embedded_phase4_bundle().unwrap();
        assert_eq!(bundle.skills.runtime_revision(), 3);
        assert_eq!(bundle.skills.skills().capability_revision(), 2);
        let restored = resolve_phase4_bundle(bundle.items.hash(), bundle.skills.hash()).unwrap();
        assert!(Arc::ptr_eq(&bundle.items, &restored.items));
        assert!(Arc::ptr_eq(&bundle.skills, &restored.skills));
        let mut wrong = bundle.items.hash();
        wrong[0] ^= 1;
        assert!(resolve_phase4_bundle(wrong, bundle.skills.hash()).is_err());
        let mut wrong = bundle.skills.hash();
        wrong[0] ^= 1;
        assert!(resolve_phase4_bundle(bundle.items.hash(), wrong).is_err());
    }

    #[test]
    fn every_manifest_table_is_embedded_byte_exactly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/data/tables-phase4");
        let disk = ItemDataSource::Directory(&root);
        let embedded = ItemDataSource::Embedded;
        let manifest: toml::Value =
            toml::from_str(&embedded.read("rules_manifest.toml").unwrap()).unwrap();
        for entry in manifest.get("files").unwrap().as_array().unwrap() {
            let file = entry.get("file").unwrap().as_str().unwrap();
            assert_eq!(disk.read(file).unwrap(), embedded.read(file).unwrap(), "{file}");
        }
        assert_eq!(
            disk.read("rules_manifest.toml").unwrap(),
            embedded.read("rules_manifest.toml").unwrap()
        );
        assert!(embedded.read("not-a-table.toml").is_err());
    }

    #[test]
    fn literals_are_exact_and_diagnostics_do_not_become_runtime_values() {
        let value = serde_json::json!({"value":"66.670", "raw":{"ignored":true}, "conditions":[]});
        assert_eq!(
            normalize(value).unwrap(),
            serde_json::json!({"value":"6667/100", "conditions":[]})
        );
        assert!(normalize(serde_json::from_str("0.5").unwrap()).is_err());
        assert!(normalize(serde_json::json!({"value":"NaN"})).is_err());
        let mut index = Map::new();
        assert!(insert_index(
            &mut index,
            "items",
            vec![serde_json::json!({"id":1}), serde_json::json!({"id":1})],
            "id"
        )
        .is_err());
    }
}
