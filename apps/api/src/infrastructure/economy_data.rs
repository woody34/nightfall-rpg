//! Phase 5 economy tables (design §1.10, D27): the generated `packages/data/tables-phase5/`
//! and the captured `jvm-pow-tables.txt`, embedded at compile time and loaded into
//! [`Phase5Rules`] bound to the V6 bundle the epoch runs on. Decimal literals stay strings
//! (exact); floating-point numbers in a table are refused; the loader and the Python
//! generator agree on every table because `Phase5Rules::try_new` recomputes every frozen
//! binary64 value from the decimal strings (E46).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::domain::zone::phase5_rules::{Phase5RulesParts, PowTable};
use crate::domain::zone::{Phase4Rules, Phase5Rules, PHASE5_FORMULA_REVISION};

/// The generated tables, in file-name order.
pub const PHASE5_TABLES: [&str; 18] = [
    "adaptations.toml",
    "auction.toml",
    "catalogue.toml",
    "drops.toml",
    "ground.toml",
    "herbs.toml",
    "items_closure.toml",
    "manor.toml",
    "merchant_fixtures.toml",
    "merchants.toml",
    "multisell.toml",
    "party.toml",
    "rates.toml",
    "recipes.toml",
    "rewards.toml",
    "skill_promotions.toml",
    "sources.toml",
    "storage_mail.toml",
];
/// The `OpenJDK` 21 `Math.pow` capture (E32, B20), beside the tables directory.
pub const POW_TABLE_FILE: &str = "jvm-pow-tables.txt";

/// Where the tables come from: the compiled-in copy, or a directory (the data check).
/// The pow table is read beside a directory (`../jvm-pow-tables.txt`) or inside it.
pub enum Phase5DataSource<'a> {
    /// `packages/data/tables-phase5/` as embedded in this build.
    Embedded,
    /// A `tables-phase5/` directory.
    Directory(&'a Path),
}

impl Phase5DataSource<'_> {
    fn read(&self, file: &str) -> Result<Cow<'static, str>, String> {
        match self {
            Self::Directory(dir) => {
                let path = if file == POW_TABLE_FILE {
                    let beside = dir.parent().map(|parent| parent.join(file));
                    match beside {
                        Some(path) if path.is_file() => path,
                        _ => dir.join(file),
                    }
                } else {
                    dir.join(file)
                };
                std::fs::read_to_string(&path)
                    .map(Cow::Owned)
                    .map_err(|error| format!("{}: {error}", path.display()))
            },
            Self::Embedded => Ok(Cow::Borrowed(match file {
                "adaptations.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/adaptations.toml")
                },
                "auction.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/auction.toml")
                },
                "catalogue.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/catalogue.toml")
                },
                "drops.toml" => include_str!("../../../../packages/data/tables-phase5/drops.toml"),
                "ground.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/ground.toml")
                },
                "herbs.toml" => include_str!("../../../../packages/data/tables-phase5/herbs.toml"),
                "items_closure.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/items_closure.toml")
                },
                "rewards.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/rewards.toml")
                },
                "skill_promotions.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/skill_promotions.toml")
                },
                "manor.toml" => include_str!("../../../../packages/data/tables-phase5/manor.toml"),
                "merchant_fixtures.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/merchant_fixtures.toml")
                },
                "merchants.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/merchants.toml")
                },
                "multisell.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/multisell.toml")
                },
                "party.toml" => include_str!("../../../../packages/data/tables-phase5/party.toml"),
                "rates.toml" => include_str!("../../../../packages/data/tables-phase5/rates.toml"),
                "recipes.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/recipes.toml")
                },
                "sources.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/sources.toml")
                },
                "storage_mail.toml" => {
                    include_str!("../../../../packages/data/tables-phase5/storage_mail.toml")
                },
                POW_TABLE_FILE => include_str!("../../../../packages/data/jvm-pow-tables.txt"),
                _ => return Err(format!("unknown embedded Phase5 table {file}")),
            })),
        }
    }
}

/// Refuse any non-integer number anywhere: every rate, chance and tax is a decimal string.
fn reject_floats(file: &str, value: &Value) -> Result<(), String> {
    match value {
        Value::Number(number) if number.as_u64().is_none() && number.as_i64().is_none() => {
            Err(format!("{file}: floating-point literal {number} (decimals must be strings)"))
        },
        Value::Array(values) => values.iter().try_for_each(|v| reject_floats(file, v)),
        Value::Object(fields) => fields.values().try_for_each(|v| reject_floats(file, v)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

/// One generated table as JSON with its provenance `sources` array removed (provenance is
/// recorded in SOURCES.md and the design; the rule bytes are what the hash covers).
fn table_json(source: &Phase5DataSource<'_>, file: &str) -> Result<Value, String> {
    let text = source.read(file)?;
    let parsed: toml::Value = toml::from_str(&text).map_err(|error| format!("{file}: {error}"))?;
    let mut value = serde_json::to_value(parsed).map_err(|error| format!("{file}: {error}"))?;
    let table = value
        .as_object_mut()
        .ok_or_else(|| format!("{file}: not a table"))?;
    if !table
        .remove("sources")
        .is_some_and(|sources| sources.is_array())
    {
        return Err(format!("{file}: missing provenance sources"));
    }
    reject_floats(file, &value)?;
    Ok(value)
}

fn typed<T: DeserializeOwned>(file: &str, value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("{file}: {error}"))
}

/// Parse `jvm-pow-tables.txt`: a `java.version=` line, then `pow56 k=1..79` and
/// `pow13 d=-120..120` in order, each with 16 lowercase hex digits (E32, B20).
pub fn parse_pow_table(text: &str) -> Result<PowTable, String> {
    let mut lines = text.lines();
    let java_version = lines
        .next()
        .and_then(|line| line.strip_prefix("java.version="))
        .ok_or("pow table: missing java.version line")?
        .to_owned();
    let (mut pow56, mut pow13) = (Vec::new(), Vec::new());
    for line in lines {
        let (prefix, rest) = line
            .split_once(" bits=")
            .ok_or_else(|| format!("pow table: malformed line {line:?}"))?;
        if rest.len() != 16
            || !rest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(format!("pow table: bits {rest:?} are not 16 lowercase hex digits"));
        }
        if let Some(k) = prefix.strip_prefix("pow56 k=") {
            let expected = pow56.len().checked_add(1).ok_or("pow table: length")?;
            if k.parse::<usize>().ok() != Some(expected) || !pow13.is_empty() {
                return Err(format!("pow table: pow56 line out of order: {line:?}"));
            }
            pow56.push(rest.to_owned());
        } else if let Some(d) = prefix.strip_prefix("pow13 d=") {
            let expected = i64::try_from(pow13.len())
                .ok()
                .and_then(|n| n.checked_sub(120))
                .ok_or("pow table: length")?;
            if d.parse::<i64>().ok() != Some(expected) {
                return Err(format!("pow table: pow13 line out of order: {line:?}"));
            }
            pow13.push(rest.to_owned());
        } else {
            return Err(format!("pow table: unknown line {line:?}"));
        }
    }
    if pow56.len() != 79 || pow13.len() != 241 {
        return Err(format!("pow table: {} pow56 and {} pow13 entries", pow56.len(), pow13.len()));
    }
    let mut sha256 = String::with_capacity(64);
    for byte in Sha256::digest(text.as_bytes()) {
        let _ = std::fmt::Write::write_fmt(&mut sha256, format_args!("{byte:02x}"));
    }
    Ok(PowTable {
        java_version,
        sha256,
        pow56,
        pow13,
    })
}

/// Every table as JSON plus the parsed pow table; parsed once per process for the
/// embedded source, since the 20 MiB of TOML dominate load time.
type Loaded = (BTreeMap<&'static str, Value>, PowTable);

fn load_all(source: &Phase5DataSource<'_>) -> Result<Loaded, String> {
    let mut tables = BTreeMap::new();
    for file in PHASE5_TABLES {
        tables.insert(file, table_json(source, file)?);
    }
    let pow = parse_pow_table(&source.read(POW_TABLE_FILE)?)?;
    Ok((tables, pow))
}

fn embedded_loaded() -> Result<&'static Loaded, String> {
    static LOADED: OnceLock<Result<Loaded, String>> = OnceLock::new();
    LOADED
        .get_or_init(|| load_all(&Phase5DataSource::Embedded))
        .as_ref()
        .map_err(Clone::clone)
}

fn parts_from(loaded: &Loaded, phase4: &Phase4Rules) -> Result<Phase5RulesParts, String> {
    let (tables, pow) = loaded;
    let get = |file: &str| -> Result<Value, String> {
        tables
            .get(file)
            .cloned()
            .ok_or_else(|| format!("{file}: not loaded"))
    };
    // D10: closure rows use the V6 row encoding, so they take the V6 normalisation (exact
    // literals as `n/d`, provenance-only `source`/`raw`/`line` keys dropped).
    let closure = super::item_data::normalize_rows(get("items_closure.toml")?)?;
    Ok(Phase5RulesParts {
        drops: typed("drops.toml", get("drops.toml")?)?,
        rewards: typed("rewards.toml", get("rewards.toml")?)?,
        items_closure: typed("items_closure.toml", closure)?,
        skill_promotions: typed("skill_promotions.toml", get("skill_promotions.toml")?)?,
        rates: typed("rates.toml", get("rates.toml")?)?,
        ground: typed("ground.toml", get("ground.toml")?)?,
        party: typed("party.toml", get("party.toml")?)?,
        herbs: typed("herbs.toml", get("herbs.toml")?)?,
        catalogue: typed("catalogue.toml", get("catalogue.toml")?)?,
        merchants: typed("merchants.toml", get("merchants.toml")?)?,
        // P5-D29: required; a missing table refuses the load (no silent "no merchants").
        merchant_fixtures: typed("merchant_fixtures.toml", get("merchant_fixtures.toml")?)?,
        multisell: typed("multisell.toml", get("multisell.toml")?)?,
        recipes: typed("recipes.toml", get("recipes.toml")?)?,
        storage_mail: typed("storage_mail.toml", get("storage_mail.toml")?)?,
        auction: typed("auction.toml", get("auction.toml")?)?,
        manor: typed("manor.toml", get("manor.toml")?)?,
        adaptations: typed("adaptations.toml", get("adaptations.toml")?)?,
        sources: typed("sources.toml", get("sources.toml")?)?,
        pow: pow.clone(),
        formula_revision: PHASE5_FORMULA_REVISION,
        phase3_hash: phase4.parts().phase3_hash,
        phase4_hash: phase4.hash(),
    })
}

/// Load the parts from a source and bind them to the V6 bundle (no validation yet).
pub fn load_phase5_parts(
    source: &Phase5DataSource<'_>,
    phase4: &Phase4Rules,
) -> Result<Phase5RulesParts, String> {
    match source {
        Phase5DataSource::Embedded => parts_from(embedded_loaded()?, phase4),
        Phase5DataSource::Directory(_) => parts_from(&load_all(source)?, phase4),
    }
}

/// Load, validate and hash the rules from a source, bound to the V6 bundle (D27).
pub fn load_phase5_rules(
    source: &Phase5DataSource<'_>,
    phase4: &Phase4Rules,
) -> Result<Phase5Rules, String> {
    let mut rules = Phase5Rules::try_new(load_phase5_parts(source, phase4)?)?;
    rules.bind_items(phase4)?;
    // P5-D19: the V6 skill bundle this item bundle binds (embedded or configured).
    let bundle =
        super::item_data::resolve_phase4_bundle(phase4.hash(), phase4.parts().phase3_hash)?;
    rules.bind_skills(&bundle.skills)?;
    Ok(rules)
}

/// The embedded Phase 5 rules bound to a V6 bundle, composed once per Phase 4 hash: the
/// tables are the embedded ones (D27); only the binding differs across `RULES_DIR` overlays.
pub fn phase5_rules_for(phase4: &Phase4Rules) -> Result<Arc<Phase5Rules>, String> {
    static BOUND: parking_lot::Mutex<Vec<Arc<Phase5Rules>>> = parking_lot::Mutex::new(Vec::new());
    if let Some(known) = BOUND
        .lock()
        .iter()
        .find(|rules| rules.parts().phase4_hash == phase4.hash())
    {
        return Ok(known.clone());
    }
    let rules = Arc::new(load_phase5_rules(&Phase5DataSource::Embedded, phase4)?);
    let mut bound = BOUND.lock();
    if let Some(known) = bound
        .iter()
        .find(|r| r.parts().phase4_hash == phase4.hash())
    {
        return Ok(known.clone());
    }
    bound.push(rules.clone());
    Ok(rules)
}

/// The embedded Phase 5 rules bound to the embedded D27 V6 bundle.
pub fn embedded_phase5_rules() -> Result<Arc<Phase5Rules>, String> {
    let bundle = super::item_data::embedded_phase4_bundle()?;
    phase5_rules_for(&bundle.items)
}

/// The data check: load a `tables-phase5/` directory (or the embedded copy) against the
/// embedded V6 bundle; every validation failure is the error text.
pub fn check(directory: Option<&Path>) -> Result<Arc<Phase5Rules>, String> {
    match directory {
        None => embedded_phase5_rules(),
        Some(dir) => {
            let bundle = super::item_data::embedded_phase4_bundle()?;
            load_phase5_rules(&Phase5DataSource::Directory(dir), &bundle.items).map(Arc::new)
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::domain::zone::binary64::vendor_unit_price;
    use crate::domain::zone::phase5_event::{Phase5Checkpoint, RecipeBook};
    use crate::domain::zone::phase5_rules::DropNode;

    fn rules() -> Arc<Phase5Rules> {
        embedded_phase5_rules().unwrap()
    }

    fn parts() -> Phase5RulesParts {
        rules().parts().clone()
    }

    #[test]
    fn embedded_tables_load_validate_and_match_the_design_counts() {
        let rules = rules();
        let inv = rules.inventory();
        // Design §1.10 items 1–10 totals, re-counted by the loader.
        assert_eq!(inv.npcs, 3246);
        assert_eq!(inv.death_npcs, 3245);
        assert_eq!(inv.corpse_npcs, 2260);
        assert_eq!(inv.drop_groups, 14_589);
        assert_eq!(inv.single_item_groups, 3032);
        assert_eq!(inv.ungrouped_death_items, 744);
        assert_eq!(inv.ungrouped_corpse_items, 7335);
        assert_eq!(inv.item_rows, 63_309);
        assert_eq!(inv.min_equals_max_rows, 56_117);
        assert_eq!(inv.corpse_time_overrides, 234);
        assert_eq!(inv.sp_only_templates, 36);
        assert_eq!(inv.g_multiplied_rates, 7);
        assert_eq!((inv.herbs_handled, inv.herbs_unhandled, inv.herb_skill_ids), (59, 9, 48));
        assert_eq!(inv.catalogue_templates, 9338);
        assert_eq!(
            (inv.merchant_entries, inv.merchant_pairs, inv.merchant_limited, inv.buylists),
            (20_236, 5803, 2145, 583)
        );
        assert_eq!(inv.unit_price_cells, 5803 * 4);
        assert_eq!(inv.multisell_lists, 195);
        assert_eq!((inv.recipes, inv.rare_recipes), (1000, 582));
        assert_eq!(inv.auction_items, 3 * 82);
        assert_eq!((inv.crops, inv.gludio_fixture_rows), (258, 64));
        assert_eq!((inv.adaptations, inv.l_table_rows, inv.pow_entries), (37, 22, 320));
        assert_eq!(rules.global_multiplier_string(), "1");
        assert_eq!(rules.global_multiplier().hex(), "3ff0000000000000");
        for name in crate::domain::zone::phase5_rules::G_MULTIPLIED_RATES {
            assert_eq!(rules.rate_prime(name).unwrap().hex(), "3ff0000000000000", "{name}");
        }
        assert!(rules.rate_prime("DeathDropAmountMultiplier").is_none());
        assert_eq!(
            rules.parts().pow.sha256,
            "4386eed8ca269e85885dabaf9bae02c5327cf425013221dfbf7df962136201f6"
        );
    }

    #[test]
    fn hash_is_stable_bound_and_referenced_not_embedded() {
        let a = rules();
        let b = Phase5Rules::try_new(parts()).unwrap();
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.hash_hex().len(), 64);
        assert!(Arc::ptr_eq(&a, &embedded_phase5_rules().unwrap()), "composed once per binding");
        let bundle = super::super::item_data::embedded_phase4_bundle().unwrap();
        a.validate_phase4(&bundle.items).unwrap();
        assert_eq!(a.parts().phase4_hash, bundle.items.hash());
        assert_eq!(a.parts().phase3_hash, bundle.skills.hash());
        // A different V6 binding is a different Phase 5 hash (D27).
        let mut rebound = parts();
        rebound.phase4_hash[0] ^= 1;
        let other = Phase5Rules::try_new(rebound).unwrap();
        assert_ne!(other.hash(), a.hash());
        assert!(other.validate_phase4(&bundle.items).is_err());
        // The checkpoint carries only the hex hash; a mismatch refuses admission.
        let checkpoint = Phase5Checkpoint {
            schema_version: 1,
            rules_hash: a.hash_hex(),
            recipes: RecipeBook::default(),
        };
        checkpoint.validate_rules(&a).unwrap();
        assert!(checkpoint.validate_rules(&other).is_err());
        let mut stale = checkpoint;
        stale.rules_hash = other.hash_hex();
        assert!(stale.validate_rules(&a).is_err());
    }

    /// D10 and P5-D12: the closure rows and the rewards table are part of the Phase 5 hash;
    /// the V7 item view is V6 plus the closure under the unchanged V6 hash; rewards cover
    /// every pinned template, including NPCs without a drop list.
    #[test]
    fn closure_items_and_rewards_bind_to_the_v6_bundle_and_fail_closed() {
        let rules = rules();
        let bundle = super::super::item_data::embedded_phase4_bundle().unwrap();
        let inv = rules.inventory();
        assert_eq!((inv.closure_items, inv.reward_npcs), (4264, 10_461));
        // The V6 bundle is unchanged; the V7 view adds exactly the closure under its hash.
        let v6_hex = rules.parts().items_closure.v6_items_sha256.clone();
        assert_eq!(rules.parts().phase4_hash, bundle.items.hash());
        assert_eq!(v6_hex, "bfee2e7d9fb73f53706c7a4afc19f5d095f4d630ca90e9a5747895c0e5472aad");
        let view = rules.v7_items().unwrap();
        assert_eq!(view.hash(), bundle.items.hash());
        assert!(!bundle.items.parts().items.contains_key(&1048));
        assert!(view.parts().items.contains_key(&1048));
        assert_eq!(view.parts().items.len(), bundle.items.parts().items.len() + 4264);
        for template in &rules.parts().catalogue.templates {
            assert!(view.parts().items.contains_key(&template.id), "{}", template.id);
            assert!(view.parts().capabilities.contains_key(&template.id), "{}", template.id);
        }
        // Binding is idempotent on the V7 view itself (a V7 restore passes it back).
        let mut again = Phase5Rules::try_new(parts()).unwrap();
        again.bind_items(view).unwrap();
        assert_eq!(again.v7_items().unwrap().parts(), view.parts());
        // A closure bound to another V6 bundle, a closure row that shadows a V6 item, a row
        // missing, a non-storable row: refused.
        let mut other_bundle = parts();
        other_bundle.items_closure.v6_items_sha256 = "00".repeat(32);
        let mut other_bundle = Phase5Rules::try_new(other_bundle).unwrap();
        assert!(other_bundle
            .bind_items(&bundle.items)
            .unwrap_err()
            .contains("v6_items_sha256"));
        let mut shadow = parts();
        let v6_row = bundle.items.parts().items.values().next().unwrap().clone();
        shadow.items_closure.items[0] = v6_row;
        assert!(Phase5Rules::try_new(shadow).is_err());
        let mut missing = parts();
        missing.items_closure.items.pop();
        assert!(Phase5Rules::try_new(missing)
            .unwrap_err()
            .contains("items_closure"));
        let mut stored = parts();
        stored.items_closure.capabilities[0].storable = false;
        assert!(Phase5Rules::try_new(stored)
            .unwrap_err()
            .contains("storable"));
        let mut closure_value = parts();
        closure_value.items_closure.items[0].weight += 1;
        assert_ne!(Phase5Rules::try_new(closure_value).unwrap().hash(), rules.hash());
        // Rewards: every sp-only template and a template without a drop list have a row.
        for id in &rules.parts().drops.sp_only_templates {
            assert!(rules.reward_npc(*id).is_some(), "{id}");
        }
        let row = rules.reward_npc(102).unwrap();
        assert_eq!((row.level, row.exp_rate.as_str(), row.sp.as_str()), (70, "0.1", "10"));
        assert!(rules.drop_npc(102).is_none());
        let mut disagree = parts();
        let first = disagree.drops.npc[0].id;
        disagree
            .rewards
            .npc
            .iter_mut()
            .find(|row| row.id == first)
            .unwrap()
            .level += 1;
        assert!(Phase5Rules::try_new(disagree)
            .unwrap_err()
            .contains("disagrees"));
        let mut count = parts();
        count.rewards.npc_count += 1;
        assert!(Phase5Rules::try_new(count).is_err());
        let mut reward_value = parts();
        reward_value
            .rewards
            .npc
            .last_mut()
            .unwrap()
            .collision_radius += 1;
        assert_ne!(Phase5Rules::try_new(reward_value).unwrap().hash(), rules.hash());
    }

    /// P5-D29: the bound fixture merchants resolve by name and exact position; nothing else
    /// does; a duplicate npc or an npc that is neither a buylist nor a multisell NPC refuses.
    #[test]
    fn merchant_fixtures_bind_by_name_and_position() {
        use crate::domain::zone::Vec2Fixed;
        let rules = rules();
        assert_eq!(rules.parts().merchant_fixtures.merchants.len(), 3);
        assert_eq!(
            rules.merchant_fixture("restock_merchant", Vec2Fixed::from_tiles(99, 102)),
            Some(35_363)
        );
        assert_eq!(
            rules.merchant_fixture("exchanger", Vec2Fixed::from_tiles(101, 98)),
            Some(32_347)
        );
        assert_eq!(
            rules.merchant_fixture("merchant", Vec2Fixed::from_tiles(101, 102)),
            Some(31_413)
        );
        assert_eq!(rules.merchant_fixture("merchant", Vec2Fixed::from_tiles(101, 103)), None);
        assert_eq!(rules.merchant_fixture("Gatekeeper", Vec2Fixed::from_tiles(128, 128)), None);
        let mut duplicate = parts();
        let first = duplicate.merchant_fixtures.merchants[0].clone();
        duplicate.merchant_fixtures.merchants.insert(0, first);
        assert!(Phase5Rules::try_new(duplicate).is_err());
        let mut unknown = parts();
        unknown.merchant_fixtures.merchants[0].npc = 1;
        assert!(Phase5Rules::try_new(unknown).is_err());
    }

    /// P5-D19: the promotions are hashed, refuse blockers outside the engine constant, bind
    /// to the V6 skill bundle, and give a V7 skill view with Spoil/Sweeper executable and
    /// Sowing/Harvesting added, under the unchanged Phase 3 hash.
    #[test]
    fn skill_promotions_bind_to_the_v6_skill_bundle_and_fail_closed() {
        use crate::domain::class::RuntimeSkillStatus;
        use crate::domain::zone::skill::SkillId;
        let rules = rules();
        let bundle = super::super::item_data::embedded_phase4_bundle().unwrap();
        // 2 promotions + Sowing/Harvesting + the 27 clean herb skills (C lane, HERBS_BOUND).
        assert_eq!(rules.inventory().skill_promotions, 31);
        let view = rules.v7_skills().unwrap();
        assert_eq!(view.hash(), bundle.skills.hash());
        for id in [254, 42, 2097, 2098, 2244, 2278, 22128] {
            let skill = view.skills().get(SkillId(id)).unwrap();
            assert_eq!(skill.runtime_status, RuntimeSkillStatus::Executable, "{id}");
            assert!(skill.blockers.is_empty(), "{id}");
        }
        for id in [254, 42] {
            let v6 = bundle.skills.skills().get(SkillId(id)).unwrap();
            assert_eq!(v6.runtime_status, RuntimeSkillStatus::Deferred, "V6 unchanged {id}");
        }
        assert!(bundle.skills.skills().get(SkillId(2097)).is_none());
        // Idempotent on the view itself.
        let mut again = Phase5Rules::try_new(parts()).unwrap();
        again.bind_skills(view).unwrap();
        assert_eq!(again.v7_skills().unwrap().skills().skills(), view.skills().skills());
        // Fail closed: an unimplemented blocker, another formula revision, another bundle.
        let mut unimplemented = parts();
        unimplemented.skill_promotions.promotions[0]
            .clears
            .push("effect:Teleport".into());
        assert!(Phase5Rules::try_new(unimplemented).is_err());
        let mut revision = parts();
        revision.skill_promotions.formula_revision = 2;
        assert!(Phase5Rules::try_new(revision).is_err());
        let mut other = parts();
        other.skill_promotions.v6_skills_sha256 = "00".repeat(32);
        let mut other = Phase5Rules::try_new(other).unwrap();
        assert!(other
            .bind_skills(&bundle.skills)
            .unwrap_err()
            .contains("v6_skills_sha256"));
        // A clean addition (herbs, P5-D19): executable, no blockers, empty clears.
        let executable = bundle
            .skills
            .skills()
            .skills()
            .values()
            .find(|s| s.runtime_status == RuntimeSkillStatus::Executable && s.operate_type == "A1")
            .unwrap()
            .clone();
        let mut clean = parts();
        let mut row = executable.clone();
        row.id = SkillId(65_000);
        clean
            .skill_promotions
            .additions
            .push(crate::domain::zone::phase5_rules::SkillAddition {
                clears: Vec::new(),
                skill: row.clone(),
            });
        let mut clean = Phase5Rules::try_new(clean).unwrap();
        clean.bind_skills(&bundle.skills).unwrap();
        assert!(clean
            .v7_skills()
            .unwrap()
            .skills()
            .get(SkillId(65_000))
            .is_some());
        let mut deferred = parts();
        let mut bad = row;
        bad.runtime_status = RuntimeSkillStatus::Deferred;
        deferred.skill_promotions.additions.push(
            crate::domain::zone::phase5_rules::SkillAddition {
                clears: Vec::new(),
                skill: bad,
            },
        );
        assert!(Phase5Rules::try_new(deferred).is_err(), "empty clears needs a clean row");
        let mut changed = parts();
        changed.skill_promotions.promotions[1].max_level = 10;
        let mut changed = Phase5Rules::try_new(changed).unwrap();
        assert_ne!(changed.hash(), rules.hash());
        assert!(changed.bind_skills(&bundle.skills).is_err());
    }

    #[test]
    fn any_field_mutation_changes_the_hash_or_fails_closed() {
        let base = rules();
        // The G decimal string is hashed verbatim: "1.0" is the same binary64 but a new hash.
        let mut g_text = parts();
        g_text.rates.global_multiplier_g = "1.0".into();
        let same_value = Phase5Rules::try_new(g_text).unwrap();
        assert_eq!(same_value.global_multiplier(), base.global_multiplier());
        assert_ne!(same_value.hash(), base.hash());
        // A G whose fold changes a rate' without the table following fails closed.
        let mut g_value = parts();
        g_value.rates.global_multiplier_g = "1.5".into();
        assert!(Phase5Rules::try_new(g_value).unwrap_err().contains("rate"));
        // G outside [2^-32, 2^32] fails closed even with consistent bits.
        let mut g_range = parts();
        g_range.rates.global_multiplier_g = "4294967297".into();
        g_range.rates.global_multiplier_g_bits = "41f0000000100000".into();
        assert!(Phase5Rules::try_new(g_range).unwrap_err().contains("2^32"));
        // Nonzero ExponentXp/Sp fails closed (E42).
        let mut exponent = parts();
        exponent.party.party.exponent_sp = 1;
        assert!(Phase5Rules::try_new(exponent)
            .unwrap_err()
            .contains("Exponent"));
        // A flipped frozen chance bit, unit price, pow entry or L bit is refused.
        let mut chance = parts();
        if let Some(DropNode::Group { chance_bits, .. }) = chance.drops.npc[0].death.first_mut() {
            *chance_bits = "4059000000000001".into();
        }
        assert!(Phase5Rules::try_new(chance).unwrap_err().contains("fl("));
        let mut price = parts();
        price.merchants.unit_prices[0].castle_15 += 1;
        assert!(Phase5Rules::try_new(price)
            .unwrap_err()
            .contains("unit price"));
        let mut pow = parts();
        pow.pow.pow13[45 + 120] = "3ff0000000000000".into();
        assert!(Phase5Rules::try_new(pow)
            .unwrap_err()
            .contains("pow13 d=45"));
        let mut l_table = parts();
        l_table.adaptations.l_table[7].bits = "3feb333333333334".into();
        assert!(Phase5Rules::try_new(l_table)
            .unwrap_err()
            .contains("L raid_clamp -1"));
        // A template dropped from the closure, or a tax literal changed, is refused.
        let mut closure = parts();
        closure.catalogue.templates.pop();
        assert!(Phase5Rules::try_new(closure).is_err());
        let mut tax = parts();
        tax.merchants.entries[0].tax = "25".into();
        assert!(Phase5Rules::try_new(tax).is_err());
        // Any other field: a changed constant is a new hash.
        let mut fee = parts();
        fee.storage_mail.warehouse.deposit_fee_per_entry = 31;
        assert_ne!(Phase5Rules::try_new(fee).unwrap().hash(), base.hash());
        let mut interval = parts();
        interval.storage_mail.flood_intervals_ticks.server_bypass = 6;
        assert!(Phase5Rules::try_new(interval).is_err());
    }

    #[test]
    fn oracle_goldens_agree_with_the_loader_where_published() {
        // V-P: the 362 pairs where naive truncation differs (economy-oracle-vendor.json).
        let oracle: Value = serde_json::from_str(include_str!(
            "../../../../packages/data/fixtures/economy-oracle-vendor.json"
        ))
        .unwrap();
        let diffs = oracle["vp_naive_differences"].as_array().unwrap();
        assert_eq!(diffs.len(), 362);
        let rules = rules();
        let table: BTreeMap<(u32, &str), u64> = rules
            .parts()
            .merchants
            .unit_prices
            .iter()
            .map(|row| ((row.item, row.tax.as_str()), row.castle_0))
            .collect();
        for row in diffs {
            let item = u32::try_from(row[0].as_u64().unwrap()).unwrap();
            let tax = row[1].as_str().unwrap();
            let price = row[2].as_u64().unwrap();
            let unit = row[3].as_u64().unwrap();
            assert_eq!(vendor_unit_price(price, tax, 0).unwrap(), unit, "{item} {tax}");
            assert_eq!(table[&(item, tax)], unit, "{item} {tax}");
            assert_ne!(price * (100 + tax.parse::<u64>().unwrap()) / 100, unit);
        }
        assert_eq!(oracle["pw"]["entries"].as_u64(), Some(320));
        assert_eq!(oracle["lt"]["rows"].as_u64(), Some(22));
        assert_eq!(oracle["lt"]["distinct_bits"].as_u64(), Some(19));
        // D-M: every branch × G string folds to the oracle's m_bits (rate 1 × G, E37).
        let drops: Value = serde_json::from_str(include_str!(
            "../../../../packages/data/fixtures/economy-oracle-drops.json"
        ))
        .unwrap();
        let vectors = drops["dm_vectors"].as_array().unwrap();
        let mut checked = 0;
        for vector in vectors {
            // Chance-branch vectors carry a G string and m_bits; amount vectors do not fold.
            let (Some(g_text), Some(m_bits)) = (vector["G"].as_str(), vector["m_bits"].as_str())
            else {
                continue;
            };
            let g = crate::domain::zone::binary64::fl_decimal(g_text).unwrap();
            let folded = crate::domain::zone::binary64::g_fold(
                crate::domain::zone::binary64::Binary64::ONE,
                g,
            )
            .unwrap();
            assert_eq!(folded.hex(), m_bits, "{vector}");
            checked += 1;
        }
        assert!(checked >= 15, "{checked} D-M vectors checked");
        // D-L: the killer modifier by strategy and d equals the frozen L table lookup.
        for (strategy, rows) in drops["dl_vectors"].as_object().unwrap() {
            let Some(rows) = rows.as_object() else {
                continue;
            };
            for (d, bits) in rows {
                let d: i64 = d.parse().unwrap();
                assert_eq!(
                    rules.killer_modifier(strategy, d).unwrap().hex(),
                    bits.as_str().unwrap(),
                    "{strategy} {d}"
                );
            }
        }
        assert!(rules.killer_modifier("unknown", 0).is_none());
    }

    #[test]
    fn directory_source_equals_the_embedded_tables_and_refuses_floats() {
        let dir =
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/data/tables-phase5"));
        let from_dir = check(Some(dir)).unwrap();
        assert_eq!(from_dir.hash(), rules().hash());
        assert!(check(Some(Path::new("/nonexistent/tables-phase5"))).is_err());
        let float = serde_json::json!({"a": [1, {"b": 1.5}]});
        assert!(reject_floats("x", &float)
            .unwrap_err()
            .contains("floating-point"));
        assert!(parse_pow_table("java.version=21\npow56 k=2 bits=3feaaaaaaaaaaaab\n").is_err());
        assert!(parse_pow_table("pow56 k=1 bits=3feaaaaaaaaaaaab\n").is_err());
    }
}
