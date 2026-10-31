//! `Phase5Rules` (design §1.10): the validated, hash-referenced economy rule set loaded from
//! the generated `tables-phase5/` (D27 pattern). `try_new` validates every cross-reference,
//! recomputes every frozen binary64 value (drop chances, the G fold, the L table, the vendor
//! unit-price table, the `OpenJDK` pow table) with the exact emulation of [`super::binary64`],
//! fails closed on a nonzero `ExponentXp`/`ExponentSp` or an out-of-range G, and hashes the eleven
//! items in design order under the domain separator `nightfall.phase5.rules.1\0`.
//!
//! Snapshots and admission carry only the hash; the bytes come from the embedded tables.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::binary64::{
    fl32_decimal, fl_decimal, g_fold, pow13, pow56, vendor_unit_price, Binary64, Rational,
};
use super::item_rules::encode_value;

macro_rules! record {
    ($name:ident { $($(#[$meta:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(missing_docs)]
        pub struct $name { $($(#[$meta])* pub $field: $ty),* }
    };
}

// ---- item 1: drop and corpse lists ------------------------------------------------------

record!(DropItemRow {
    id: u32,
    min: u64,
    max: u64,
    chance: String,
    chance_bits: String
});

/// One `<death>`/`<corpse>` node in document order: an ungrouped item or a `<group>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum DropNode {
    Item {
        id: u32,
        min: u64,
        max: u64,
        chance: String,
        chance_bits: String,
    },
    Group {
        chance: String,
        chance_bits: String,
        items: Vec<DropItemRow>,
    },
}

record!(DropNpc {
    id: u32, level: u32, is_raid: bool, corpse_time: u64, can_be_sown: bool,
    harvest_skills: Vec<u32>, exp_rate: String, sp: String, collision_radius: u32,
    death: Vec<DropNode>, corpse: Vec<DropNode>
});
record!(CorpseTimeOverride {
    id: u32,
    corpse_time: u64
});
record!(DropsTable {
    schema_version: u32, corpse_time_overrides_count: u64, sp_only_templates_count: u64,
    corpse_time_overrides: Vec<CorpseTimeOverride>, sp_only_templates: Vec<u32>, npc: Vec<DropNpc>
});

// ---- item 2: rate chain --------------------------------------------------------------------

record!(RateConfig {
    kind: String, value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] rate_prime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] rate_prime_bits: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] item_id: Option<u32>
});
record!(DropStrategy {
    #[serde(rename = "PreciseDropCalculation")]
    precise_drop_calculation: bool,
    #[serde(rename = "PreciseDropMultipleGroupRolls")]
    precise_drop_multiple_group_rolls: bool,
    #[serde(rename = "PreciseDropMultipleRollsAggregateDrops")]
    precise_drop_multiple_rolls_aggregate_drops: bool
});
record!(DeepBlue {
    #[serde(rename = "UseDeepBlueDropRules")]
    use_deep_blue_drop_rules: bool,
    #[serde(rename = "UseDeepBlueDropRulesRaid")]
    use_deep_blue_drop_rules_raid: bool,
    #[serde(rename = "DropAdenaMinLevelDifference")]
    drop_adena_min_level_difference: i64,
    #[serde(rename = "DropAdenaMaxLevelDifference")]
    drop_adena_max_level_difference: i64,
    #[serde(rename = "DropAdenaMinLevelGapChance")]
    drop_adena_min_level_gap_chance: i64,
    #[serde(rename = "DropItemMinLevelDifference")]
    drop_item_min_level_difference: i64,
    #[serde(rename = "DropItemMaxLevelDifference")]
    drop_item_max_level_difference: i64,
    #[serde(rename = "DropItemMinLevelGapChance")]
    drop_item_min_level_gap_chance: i64,
    #[serde(rename = "ChampionEnable")]
    champion_enable: bool
});
record!(RatesTable {
    schema_version: u32, global_multiplier_g: String, global_multiplier_g_bits: String,
    double_rates: BTreeMap<String, RateConfig>, float_rates: BTreeMap<String, RateConfig>,
    int_rates: BTreeMap<String, RateConfig>, item_overrides: BTreeMap<String, RateConfig>,
    drop_strategy: DropStrategy, deep_blue: DeepBlue
});

// ---- item 3: ground constants and PK policy -----------------------------------------------

record!(AutoDestroyRow {
    item_id: u32,
    seconds: u64
});
record!(GroundConstants {
    loot_owner_ms: u64, drop_protection_ms: u64, raid_loot_rights_seconds: u64,
    raid_loot_rights_cc_size: u64, auto_loot: bool, auto_loot_raids: bool, auto_loot_herbs: bool,
    decay_seconds: u64, herb_decay_seconds: u64, auto_destroy_time: Vec<AutoDestroyRow>,
    protected_items: Vec<u32>, destroy_player_dropped_item: bool, destroy_equipable_item: bool,
    save_dropped_item: bool, multiple_item_drop: bool, scanner_ms: u64, offset_npc: u64,
    offset_player: u64, offset_player_range: u64
});
record!(PkPolicy {
    karma_rate_drop: u64, karma_rate_drop_item: u64, karma_rate_drop_equip: u64,
    karma_rate_drop_equip_weapon: u64, karma_drop_limit: u64, player_rate_drop: u64,
    minimum_pk_required_to_drop: u64, pk_drop_protection: String, non_droppable_items: Vec<u32>,
    pet_items: Vec<u32>
});
record!(GroundTable {
    schema_version: u32,
    ground: GroundConstants,
    pk: PkPolicy
});

// ---- item 4: party constants -----------------------------------------------------------------

record!(EvenDistribute { all_stackable_items: bool, all_other_items: bool, items: Vec<u32> });
record!(CutoffGap {
    min: u32,
    max: u32,
    percent: u32
});
record!(XpCutoff {
    party_xp_cutoff_method: String, party_xp_cutoff_percent: String, party_xp_cutoff_level: u32,
    cutoff_gaps: Vec<CutoffGap>
});
record!(PartyConstants {
    size_cap: u32, party_range: u64, party_range2: u64, leave_party_leader: bool,
    exponent_xp: i64, exponent_sp: i64, vote_timeout_seconds: u64, bonus_exp_sp: Vec<String>,
    bonus_exp_sp_bits: Vec<String>, even_distribute: EvenDistribute, xp_cutoff: XpCutoff
});
record!(PartyTable {
    schema_version: u32,
    party: PartyConstants
});

// ---- item 5: herbs and the catalogue closure -------------------------------------------------

record!(HerbSkill {
    id: u32,
    level: u32
});
record!(HandledHerb { id: u32, name: String, r#type: String, skills: Vec<HerbSkill> });
record!(UnhandledHerb {
    id: u32,
    name: String,
    r#type: String
});
record!(HerbsTable {
    schema_version: u32, total_herbs: u64, handled_herbs_count: u64, unhandled_herbs_count: u64,
    distinct_skill_ids: Vec<u32>, handled: Vec<HandledHerb>, unhandled: Vec<UnhandledHerb>
});
record!(BlockedTemplate { id: u32, name: String, blockers: Vec<String> });
record!(SharedRecipeItem { recipe_item: u32, recipes: Vec<u32> });
record!(SpecialIngredient {
    id: i64, name: String, lists: Vec<u32>, entries: u64, ingredient_rows: u64, product_rows: u64,
    lists_admitted: bool, entry_capability: String
});
record!(CatalogueTemplate {
    id: u32, name: String, r#type: String, roles: Vec<String>, in_phase4: bool, storable: bool,
    storable_status: String, phase4_contexts: Vec<String>, tags: Vec<String>, executable: String,
    capability: String, blockers: Vec<String>
});
record!(CatalogueCounts {
    referenced_templates: u64,
    imported: u64,
    in_phase4_catalogue: u64,
    phase4_closure_additions: u64,
    storable: u64,
    not_storable: u64,
    storable_pending_d97: u64,
    lifetime_deferred: u64,
    executable_yes: u64,
    executable_no: u64,
    executable_not_required: u64
});
record!(CatalogueTable {
    schema_version: u32, phase4_closure_additions: Vec<u32>, non_executable_herbs: Vec<BlockedTemplate>,
    not_storable: Vec<u32>, pending_d97: Vec<u32>, non_executable_recipe_items: Vec<BlockedTemplate>,
    shared_recipe_items: Vec<SharedRecipeItem>, special_multisell_ingredients: Vec<SpecialIngredient>,
    templates: Vec<CatalogueTemplate>, counts: CatalogueCounts
});

// ---- item 6: merchants -------------------------------------------------------------------------

record!(UnitPriceRow {
    item: u32,
    tax: String,
    castle_0: u64,
    castle_5: u64,
    castle_15: u64,
    castle_25: u64
});
record!(MerchantEntry {
    script: String, array: String, list_ids: Vec<u64>, npcs: Vec<u32>, item: u32, ref_price: u64,
    tax: String, delay: String, count: i64, restock_min: i64, limited: bool, zero_price: bool
});
record!(Buylist {
    list_id: u64,
    npc: u32,
    script: String,
    array: String,
    entries: u64
});
record!(CastleTax {
    id: u32,
    name: String,
    tax_percent: u32
});
record!(CastleTaxes { castles: Vec<CastleTax>, no_owner_ceiling: u32, dawn_ceiling: u32, dusk_ceiling: u32 });
record!(MerchantsTable {
    schema_version: u32, total_entries: u64, total_pairs: u64, limited_entries: u64, buylists_count: u64,
    zero_price_items: Vec<u32>, naive_truncation_differences_castle_0: u64, castle_percents: Vec<u32>,
    unit_prices: Vec<UnitPriceRow>, entries: Vec<MerchantEntry>, buylists: Vec<Buylist>,
    castle_taxes: CastleTaxes
});

// ---- item 7: multisell -------------------------------------------------------------------------

record!(MultisellIngredient {
    id: i64,
    count: u64,
    is_tax: bool,
    maintain: bool
});
record!(IdCount {
    id: i64,
    count: u64
});
record!(MultisellEntry { ingredients: Vec<MultisellIngredient>, products: Vec<IdCount> });
record!(MultisellList {
    id: u32, source_files: Vec<String>, apply_taxes: bool, maintain_enchantment: bool, npcs: Vec<u32>,
    entries: Vec<MultisellEntry>
});
record!(MultisellDeviation {
    list_id: u32,
    kept: String,
    kept_entries: u64,
    dropped: String,
    dropped_entries: u64
});
record!(MultisellTable {
    schema_version: u32, total_files: u64, total_lists: u64, apply_taxes_lists: u64,
    maintain_enchantment_lists: u64, maintain_ingredient_ingredients: u64,
    deviations: Vec<MultisellDeviation>, list: Vec<MultisellList>
});

// ---- item 8: recipes ----------------------------------------------------------------------------

record!(RecipeConfig {
    craft_masterwork: bool,
    dwarf_recipe_limit: u32,
    common_recipe_limit: u32,
    alternative_crafting: bool
});
record!(CraftRank {
    rank: u32,
    max_craft_level: u32,
    magic_level: u32
});
record!(RankTable { skill_id: u32, ranks: Vec<CraftRank> });
record!(StatUse {
    name: String,
    value: u64
});
record!(RareProduct {
    id: u32,
    count: u64,
    rarity: u32
});
record!(RecipeRow {
    id: u32, recipe_id: u32, name: String, craft_level: u32, r#type: String, success_rate: u32,
    stat_use: Vec<StatUse>, alt_stat_change: Vec<StatUse>, ingredients: Vec<IdCount>,
    production: IdCount,
    #[serde(default, skip_serializing_if = "Option::is_none")] production_rare: Option<RareProduct>
});
record!(RecipesTable {
    schema_version: u32, total_recipes: u64, total_rare_recipes: u64, config: RecipeConfig,
    create_item: RankTable, create_common_item: RankTable, recipe: Vec<RecipeRow>
});

// ---- item 9: storage, mail and flood ---------------------------------------------------------

record!(WarehouseConstants {
    slots_dwarf: u32,
    slots_no_dwarf: u32,
    deposit_fee_per_entry: u64,
    karma_player_can_use: bool
});
record!(FreightConstants {
    slots: u32,
    fee_per_entry: u64
});
record!(MailConstants {
    max_recv_length: u32,
    max_subj_length: u32,
    max_text_length: u32,
    max_attachments: u32,
    inbox_size: u32,
    outbox_size: u32,
    message_fee: u64,
    message_fee_per_slot: u64,
    expiration_hours: u64,
    cod_expiration_hours: u64
});
record!(FloodIntervals {
    multisell: u64,
    transaction: u64,
    manufacture: u64,
    manor: u64,
    send_mail: u64,
    server_bypass: u64,
    item_auction: u64
});
record!(StorageMailTable {
    schema_version: u32,
    warehouse: WarehouseConstants,
    freight: FreightConstants,
    mail: MailConstants,
    flood_intervals_ticks: FloodIntervals
});

// ---- item 10: item auction and manor ----------------------------------------------------------

record!(AuctionConfig {
    expired_after_days: u64,
    time_extends_on_bid: u64,
    start_time_space_minutes: u64,
    finish_time_space_minutes: u64
});
record!(ScheduledAuction {
    auction_id: u64,
    instance: u32,
    round: u32,
    start: String,
    end: String,
    start_epoch_ms: i64,
    end_epoch_ms: i64,
    item: String
});
record!(FixtureSchedule { anchor: String, rounds_per_instance: u32, auctions: Vec<ScheduledAuction> });
record!(AuctionItem {
    auction_id: u64,
    item_id: u32,
    count: u64,
    init_bid: u64,
    length_minutes: u64
});
record!(AuctionInstance { id: u32, day_of_week: u32, hour_of_day: u32, minute_of_hour: u32, items: Vec<AuctionItem> });
record!(AuctionTable {
    schema_version: u32, total_instances: u64, config: AuctionConfig, fixture_schedule: FixtureSchedule,
    instance: Vec<AuctionInstance>
});
record!(ManorCrop {
    castle: u32,
    crop: u32,
    seed: u32,
    mature: u32,
    rew1: u32,
    rew2: u32,
    alternative: bool,
    level: u32,
    limit_seed: u64,
    limit_crops: u64
});
record!(ManorSkill {
    id: u32,
    level: u32,
    levels: u32
});
record!(StrongTypeMultiplier {
    skill: u32,
    multiplier: u32
});
record!(ManorSkills {
    sow: Vec<ManorSkill>, harvester_items: Vec<u32>, harvest: Vec<ManorSkill>,
    strong_type_multipliers: Vec<StrongTypeMultiplier>, rate_drop_manor: u64
});
record!(SeedProduction {
    seed_id: u32,
    price: u64,
    amount: u64
});
record!(CropProcurement {
    crop_id: u32,
    price: u64,
    amount: u64,
    reward_type: u32
});
record!(FixtureGludio { castle_id: u32, castle_area: u32, seed_production: Vec<SeedProduction>, crop_procurement: Vec<CropProcurement> });
record!(ManorTable {
    schema_version: u32, total_crops: u64, total_alternative_crops: u64, gludio_crops_count: u64,
    crops: Vec<ManorCrop>, skills: ManorSkills, fixture_gludio: FixtureGludio
});

// ---- item 11: adaptations, the frozen tables and the bindings --------------------------------

record!(Adaptation {
    label: String,
    withdrawn: bool,
    section: String
});
record!(OracleTableRef {
    path: String,
    sha256: String
});
record!(LTableRow {
    strategy: String,
    d_condition: String,
    value: String,
    bits: String
});
record!(AdaptationsTable {
    schema_version: u32, l_table_rows: u64, l_table_distinct_bits: u64, adaptations: Vec<Adaptation>,
    oracle_table: OracleTableRef, l_table: Vec<LTableRow>
});
record!(Repository {
    url: String,
    revision: String,
    branch: String
});
record!(DesignRef {
    revision: String,
    path: String
});
record!(RevisionRef { revision: String });
record!(Repositories {
    game: Repository,
    datapack: Repository,
    design: DesignRef,
    phase4_data: RevisionRef
});
record!(PowTableRef {
    path: String,
    sha256: String,
    pow56_entries: u64,
    pow13_entries: u64,
    total_entries: u64,
    java_version: String
});
record!(OracleTables {
    jvm_pow_tables: PowTableRef
});
record!(SourcesTable {
    schema_version: u32,
    domain: String,
    repositories: Repositories,
    oracle_tables: OracleTables
});
// The captured OpenJDK 21 `Math.pow` raw-bits table (E32, B20): `pow56[k − 1]` for
// `k = 1..79` and `pow13[d + 120]` for `d = −120..120`, plus the file's sha256.
record!(PowTable { java_version: String, sha256: String, pow56: Vec<String>, pow13: Vec<String> });

// Everything `Phase5Rules` hashes, in design §1.10 item order.
record!(Phase5RulesParts {
    drops: DropsTable,
    rates: RatesTable,
    ground: GroundTable,
    party: PartyTable,
    herbs: HerbsTable,
    catalogue: CatalogueTable,
    merchants: MerchantsTable,
    multisell: MultisellTable,
    recipes: RecipesTable,
    storage_mail: StorageMailTable,
    auction: AuctionTable,
    manor: ManorTable,
    adaptations: AdaptationsTable,
    sources: SourcesTable,
    pow: PowTable,
    /// The V7 formula revision (design §1.10 item 11).
    formula_revision: u32,
    /// The Phase 3 rules hash this epoch is bound to.
    phase3_hash: [u8; 32],
    /// The Phase 4 rules hash this epoch is bound to.
    phase4_hash: [u8; 32]
});

/// Counts the loader reports and the tests assert (design §1.10 totals).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[allow(missing_docs)]
pub struct Phase5Inventory {
    pub npcs: u64,
    pub death_npcs: u64,
    pub corpse_npcs: u64,
    pub drop_groups: u64,
    pub single_item_groups: u64,
    pub ungrouped_death_items: u64,
    pub ungrouped_corpse_items: u64,
    pub item_rows: u64,
    pub min_equals_max_rows: u64,
    pub corpse_time_overrides: u64,
    pub sp_only_templates: u64,
    pub g_multiplied_rates: u64,
    pub herbs_handled: u64,
    pub herbs_unhandled: u64,
    pub herb_skill_ids: u64,
    pub catalogue_templates: u64,
    pub merchant_entries: u64,
    pub merchant_pairs: u64,
    pub merchant_limited: u64,
    pub buylists: u64,
    pub unit_price_cells: u64,
    pub multisell_lists: u64,
    pub multisell_entries: u64,
    pub recipes: u64,
    pub rare_recipes: u64,
    pub auction_items: u64,
    pub crops: u64,
    pub gludio_fixture_rows: u64,
    pub adaptations: u64,
    pub l_table_rows: u64,
    pub pow_entries: u64,
}

/// Validated and hashed Phase 5 rules (design §1.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase5Rules {
    parts: Phase5RulesParts,
    inventory: Phase5Inventory,
    g: Binary64,
    hash: [u8; 32],
}

/// The hash domain separator (design §1.10).
pub const PHASE5_RULES_DOMAIN: &[u8] = b"nightfall.phase5.rules.1\0";
/// The only V7 formula revision.
pub const PHASE5_FORMULA_REVISION: u32 = 1;
/// The rate configs G multiplies (design §3.4): chance branches and the XP/SP base rates.
pub const G_MULTIPLIED_RATES: [&str; 7] = [
    "DeathDropChanceMultiplier",
    "CorpseDropChanceMultiplier",
    "HerbDropChanceMultiplier",
    "RaidDropChanceMultiplier",
    "RateXp",
    "RateSp",
    "DropChanceMultiplier_57",
];

type Check = Result<(), String>;

fn ensure(condition: bool, message: impl FnOnce() -> String) -> Check {
    if condition {
        Ok(())
    } else {
        Err(message())
    }
}

fn len(items: &[impl Sized]) -> u64 {
    u64::try_from(items.len()).unwrap_or(u64::MAX)
}

fn hex(hash: [u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in hash {
        let _ = std::fmt::Write::write_fmt(&mut text, format_args!("{byte:02x}"));
    }
    text
}

/// A plain decimal whose binary64 is the frozen bits.
fn check_bits(label: &str, decimal: &str, bits: &str) -> Check {
    let value = fl_decimal(decimal).map_err(|e| format!("{label}: {e}"))?;
    let frozen = Binary64::parse_hex(bits).map_err(|e| format!("{label}: {e}"))?;
    ensure(value == frozen, || {
        format!("{label}: fl({decimal}) = {} but the table froze {bits}", value.hex())
    })
}

impl Phase5Rules {
    /// Construct after loading the generated tables; validates and hashes.
    pub fn try_new(parts: Phase5RulesParts) -> Result<Self, String> {
        let (inventory, g) = parts.validate()?;
        let mut bytes = PHASE5_RULES_DOMAIN.to_vec();
        let items: [(&str, serde_json::Value); 11] = [
            ("1.drops", json(&parts.drops)?),
            ("2.rates", json(&parts.rates)?),
            ("3.ground", json(&parts.ground)?),
            ("4.party", json(&parts.party)?),
            ("5.herbs", json(&(&parts.herbs, &parts.catalogue))?),
            ("6.merchants", json(&parts.merchants)?),
            ("7.multisell", json(&parts.multisell)?),
            ("8.recipes", json(&parts.recipes)?),
            ("9.storage_mail", json(&parts.storage_mail)?),
            ("10.auction_manor", json(&(&parts.auction, &parts.manor))?),
            (
                "11.policy",
                json(&(
                    &parts.adaptations,
                    &parts.sources,
                    &parts.pow,
                    parts.formula_revision,
                    hex(parts.phase3_hash),
                    hex(parts.phase4_hash),
                ))?,
            ),
        ];
        for (label, value) in &items {
            encode_value(&serde_json::Value::String((*label).to_owned()), &mut bytes)?;
            encode_value(value, &mut bytes)?;
        }
        Ok(Self {
            parts,
            inventory,
            g,
            hash: Sha256::digest(bytes).into(),
        })
    }

    /// The validated tables.
    #[must_use]
    pub const fn parts(&self) -> &Phase5RulesParts {
        &self.parts
    }

    /// The design §1.10 hash.
    #[must_use]
    pub const fn hash(&self) -> [u8; 32] {
        self.hash
    }

    /// Lowercase hex of [`Self::hash`], the `Phase5Checkpoint.rules_hash` form.
    #[must_use]
    pub fn hash_hex(&self) -> String {
        hex(self.hash)
    }

    /// The counts established at validation.
    #[must_use]
    pub const fn inventory(&self) -> &Phase5Inventory {
        &self.inventory
    }

    /// `G = fl(G_string)`, the derived binary64 of the owner's decimal string (E42).
    #[must_use]
    pub const fn global_multiplier(&self) -> Binary64 {
        self.g
    }

    /// The owner's G decimal string as hashed.
    #[must_use]
    pub fn global_multiplier_string(&self) -> &str {
        &self.parts.rates.global_multiplier_g
    }

    /// `rate'` of a G-multiplied config (design §3.4), already folded at load.
    #[must_use]
    pub fn rate_prime(&self, name: &str) -> Option<Binary64> {
        let config = self
            .parts
            .rates
            .double_rates
            .get(name)
            .or_else(|| self.parts.rates.float_rates.get(name))
            .or_else(|| self.parts.rates.item_overrides.get(name))?;
        Binary64::parse_hex(config.rate_prime_bits.as_deref()?).ok()
    }

    /// The frozen killer (deep-blue) modifier `L` of design §6.2 for a strategy and level
    /// difference `d`, read from the validated table (E36). `None` for an unknown strategy.
    #[must_use]
    pub fn killer_modifier(&self, strategy: &str, d: i64) -> Option<Binary64> {
        self.parts
            .adaptations
            .l_table
            .iter()
            .filter(|row| row.strategy == strategy)
            .find(|row| {
                let (op, bound) = row
                    .d_condition
                    .split_once(' ')
                    .map_or(("", row.d_condition.as_str()), |(op, bound)| (op, bound));
                let bound: i64 = bound.parse().unwrap_or(i64::MIN);
                match op {
                    ">=" => d >= bound,
                    "<=" => d <= bound,
                    _ => d == bound,
                }
            })
            .and_then(|row| Binary64::parse_hex(&row.bits).ok())
    }

    /// Bind to the actual V6 bundle this epoch runs on, never a hash supplied without its
    /// content (D27).
    pub fn validate_phase4(&self, rules: &super::Phase4Rules) -> Result<(), String> {
        if self.parts.phase4_hash != rules.hash() {
            return Err("Phase5 item rules hash mismatch".into());
        }
        if self.parts.phase3_hash != rules.parts().phase3_hash {
            return Err("Phase5 skill/profile hash mismatch".into());
        }
        Ok(())
    }
}

fn json<T: Serialize>(value: &T) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

impl super::phase5_event::Phase5Checkpoint {
    /// D27: a checkpoint is admitted only against the rules it was validated against.
    pub fn validate_rules(&self, rules: &Phase5Rules) -> Result<(), super::ItemError> {
        self.validate()?;
        if self.rules_hash != rules.hash_hex() {
            return Err(super::ItemError("Phase 5 checkpoint rules hash mismatch"));
        }
        Ok(())
    }
}

/// `Util.map` in double, left to right, after `constrain` (the generator's `_umap`).
fn util_map(
    value: i64,
    in_min: i64,
    in_max: i64,
    out_min: i64,
    out_max: &Rational,
) -> Result<Rational, String> {
    let e = |error: super::binary64::Binary64Error| error.to_string();
    let x = Rational::integer(value.clamp(in_min, in_max));
    let span_in = Rational::integer(in_max.checked_sub(in_min).ok_or("level window overflow")?)
        .fl()
        .map_err(e)?;
    let span_out = out_max.sub(&Rational::integer(out_min)).fl().map_err(e)?;
    let offset = x.sub(&Rational::integer(in_min)).fl().map_err(e)?;
    let product = offset
        .rational()
        .map_err(e)?
        .mul(&span_out.rational().map_err(e)?)
        .fl()
        .map_err(e)?;
    let quotient = product
        .rational()
        .map_err(e)?
        .div(&span_in.rational().map_err(e)?)
        .map_err(e)?
        .fl()
        .map_err(e)?;
    quotient
        .rational()
        .map_err(e)?
        .add(&Rational::integer(out_min))
        .fl()
        .map_err(e)?
        .rational()
        .map_err(e)
}

/// One deep-blue window of the L table: `d` from `−lo` down to `−hi`, `Util.map(...) / 100`.
fn l_window(
    rows: &mut Vec<(String, String, Binary64)>,
    hundred: &Rational,
    name: &str,
    lo: i64,
    hi: i64,
    gap: i64,
) -> Check {
    let e = |error: super::binary64::Binary64Error| error.to_string();
    let (neg_lo, neg_hi) = (0_i64.saturating_sub(lo), 0_i64.saturating_sub(hi));
    let mut d = neg_lo;
    while d >= neg_hi {
        let mapped = util_map(d, neg_hi, neg_lo, gap, hundred)?;
        let value = mapped
            .div(&Rational::integer(100))
            .map_err(e)?
            .fl()
            .map_err(e)?;
        let condition = if d == neg_lo {
            format!(">= {d}")
        } else if d == neg_hi {
            format!("<= {d}")
        } else {
            d.to_string()
        };
        rows.push((name.to_owned(), condition, value));
        d = d.checked_sub(1).ok_or("level window underflow")?;
    }
    Ok(())
}

/// The design §6.2 L table recomputed from the deep-blue windows and `0.15` (§3.3).
fn expected_l_table(deep_blue: &DeepBlue) -> Result<Vec<(String, String, Binary64)>, String> {
    let e = |error: super::binary64::Binary64Error| error.to_string();
    let hundred = fl_decimal("100.0").map_err(e)?.rational().map_err(e)?;
    let mut rows = Vec::new();
    l_window(
        &mut rows,
        &hundred,
        "item_window",
        deep_blue.drop_item_min_level_difference,
        deep_blue.drop_item_max_level_difference,
        deep_blue.drop_item_min_level_gap_chance,
    )?;
    let factor = fl_decimal("0.15").map_err(e)?.rational().map_err(e)?;
    let mut d = 0_i64;
    loop {
        let raw = Rational::integer(d)
            .mul(&factor)
            .fl()
            .map_err(e)?
            .rational()
            .map_err(e)?
            .add(&Rational::integer(1))
            .fl()
            .map_err(e)?;
        let clamped = if raw
            .rational()
            .map_err(e)?
            .compare(&Rational::integer(0))
            .is_le()
        {
            Binary64::ZERO
        } else if raw
            .rational()
            .map_err(e)?
            .compare(&Rational::integer(1))
            .is_ge()
        {
            Binary64::ONE
        } else {
            raw
        };
        if clamped == Binary64::ZERO {
            rows.push(("raid_clamp".to_owned(), format!("<= {d}"), clamped));
            break;
        }
        let condition = if d == 0 {
            ">= 0".to_owned()
        } else {
            d.to_string()
        };
        rows.push(("raid_clamp".to_owned(), condition, clamped));
        d = d.checked_sub(1).ok_or("raid clamp underflow")?;
        if d < -64 {
            return Err("raid clamp never reaches zero".into());
        }
    }
    l_window(
        &mut rows,
        &hundred,
        "adena_window",
        deep_blue.drop_adena_min_level_difference,
        deep_blue.drop_adena_max_level_difference,
        deep_blue.drop_adena_min_level_gap_chance,
    )?;
    Ok(rows)
}

/// The catalogue closure: every template a table references must be a storable catalogue row,
/// and the catalogue must contain nothing else.
struct Closure<'a> {
    templates: &'a BTreeMap<u32, &'a CatalogueTemplate>,
    referenced: BTreeSet<u32>,
}

impl Closure<'_> {
    fn require(&mut self, table: &str, id: u32) -> Check {
        self.referenced.insert(id);
        ensure(self.templates.contains_key(&id), || {
            format!("{table}: template {id} is outside the catalogue closure")
        })
    }
}

/// One drop or corpse item row: count range, decimal precision, frozen bits, closure.
fn drop_item_row(
    inv: &mut Phase5Inventory,
    closure: &mut Closure<'_>,
    npc_id: u32,
    scope: &str,
    row: (&u32, &u64, &u64, &str, &str),
) -> Check {
    let (id, min, max, chance, bits) = row;
    let label = format!("drops npc {npc_id} {scope} item {id}");
    ensure(*min >= 1 && *max >= 1 && *min <= 18_000_000 && *max <= 18_000_000, || {
        format!("{label}: count range {min}..{max}")
    })?;
    ensure(
        chance
            .split_once('.')
            .is_none_or(|(_, frac)| frac.len() <= 20),
        || format!("{label}: chance precision"),
    )?;
    check_bits(&label, chance, bits)?;
    closure.require("drops", *id)?;
    inv.item_rows = inv.item_rows.saturating_add(1);
    if min == max {
        inv.min_equals_max_rows = inv.min_equals_max_rows.saturating_add(1);
    }
    Ok(())
}

impl Phase5RulesParts {
    /// Every design §1.10 cross-reference and frozen-value check; returns the inventory and G.
    #[allow(clippy::too_many_lines)] // One pass over eleven tables; splitting would hide the order.
    fn validate(&self) -> Result<(Phase5Inventory, Binary64), String> {
        let mut inv = Phase5Inventory::default();
        let e = |error: super::binary64::Binary64Error| error.to_string();
        for (name, version) in [
            ("drops", self.drops.schema_version),
            ("rates", self.rates.schema_version),
            ("ground", self.ground.schema_version),
            ("party", self.party.schema_version),
            ("herbs", self.herbs.schema_version),
            ("catalogue", self.catalogue.schema_version),
            ("merchants", self.merchants.schema_version),
            ("multisell", self.multisell.schema_version),
            ("recipes", self.recipes.schema_version),
            ("storage_mail", self.storage_mail.schema_version),
            ("auction", self.auction.schema_version),
            ("manor", self.manor.schema_version),
            ("adaptations", self.adaptations.schema_version),
            ("sources", self.sources.schema_version),
        ] {
            ensure(version == 1, || format!("{name}: schema_version {version} is not 1"))?;
        }
        ensure(self.formula_revision == PHASE5_FORMULA_REVISION, || {
            "unknown V7 formula revision".into()
        })?;
        ensure(
            self.sources.domain.as_bytes()
                == PHASE5_RULES_DOMAIN.strip_suffix(b"\0").unwrap_or_default(),
            || "sources.toml domain differs from the hash domain".into(),
        )?;

        // -- item 5 first: the catalogue closure every other table references.
        let mut templates: BTreeMap<u32, &CatalogueTemplate> = BTreeMap::new();
        for template in &self.catalogue.templates {
            ensure(template.id > 0 && templates.insert(template.id, template).is_none(), || {
                format!("catalogue: duplicate or zero template {}", template.id)
            })?;
            ensure(template.storable && template.storable_status != "not_storable", || {
                format!("catalogue: template {} is not storable (D5)", template.id)
            })?;
        }
        ensure(
            self.catalogue.not_storable.is_empty() && self.catalogue.counts.not_storable == 0,
            || "catalogue: non-storable templates remain".into(),
        )?;
        let counts = &self.catalogue.counts;
        ensure(
            counts.referenced_templates == len(&self.catalogue.templates)
                && counts.imported == counts.referenced_templates
                && counts.storable == counts.referenced_templates
                && counts.phase4_closure_additions == len(&self.catalogue.phase4_closure_additions)
                && counts.storable_pending_d97 == len(&self.catalogue.pending_d97)
                && counts
                    .executable_yes
                    .checked_add(counts.executable_no)
                    .and_then(|n| n.checked_add(counts.executable_not_required))
                    == Some(counts.referenced_templates),
            || "catalogue: counts disagree with the rows".into(),
        )?;
        let mut closure = Closure {
            templates: &templates,
            referenced: BTreeSet::new(),
        };
        inv.catalogue_templates = len(&self.catalogue.templates);

        // -- item 1: drop and corpse lists.
        let mut npc_ids = BTreeSet::new();
        for npc in &self.drops.npc {
            ensure(npc.id > 0 && npc_ids.insert(npc.id), || {
                format!("drops: duplicate npc {}", npc.id)
            })?;
            Rational::parse_decimal(&npc.exp_rate)
                .map_err(|err| format!("drops npc {}: exp_rate {err}", npc.id))?;
            Rational::parse_decimal(&npc.sp)
                .map_err(|err| format!("drops npc {}: sp {err}", npc.id))?;
            if !npc.death.is_empty() {
                inv.death_npcs = inv.death_npcs.saturating_add(1);
            }
            if !npc.corpse.is_empty() {
                inv.corpse_npcs = inv.corpse_npcs.saturating_add(1);
            }
            for (scope, nodes) in [("death", &npc.death), ("corpse", &npc.corpse)] {
                for node in nodes {
                    match node {
                        DropNode::Item {
                            id,
                            min,
                            max,
                            chance,
                            chance_bits,
                        } => {
                            drop_item_row(
                                &mut inv,
                                &mut closure,
                                npc.id,
                                scope,
                                (id, min, max, chance, chance_bits),
                            )?;
                            if scope == "death" {
                                inv.ungrouped_death_items =
                                    inv.ungrouped_death_items.saturating_add(1);
                            } else {
                                inv.ungrouped_corpse_items =
                                    inv.ungrouped_corpse_items.saturating_add(1);
                            }
                        },
                        DropNode::Group {
                            chance,
                            chance_bits,
                            items,
                        } => {
                            ensure(scope == "death", || {
                                format!("drops npc {}: a corpse group", npc.id)
                            })?;
                            ensure(!items.is_empty(), || {
                                format!("drops npc {}: empty group", npc.id)
                            })?;
                            check_bits(
                                &format!("drops npc {} group", npc.id),
                                chance,
                                chance_bits,
                            )?;
                            inv.drop_groups = inv.drop_groups.saturating_add(1);
                            if items.len() == 1 {
                                inv.single_item_groups = inv.single_item_groups.saturating_add(1);
                            }
                            for item in items {
                                drop_item_row(
                                    &mut inv,
                                    &mut closure,
                                    npc.id,
                                    "group",
                                    (
                                        &item.id,
                                        &item.min,
                                        &item.max,
                                        &item.chance,
                                        &item.chance_bits,
                                    ),
                                )?;
                            }
                        },
                    }
                }
            }
        }
        inv.npcs = len(&self.drops.npc);
        // corpseTime overrides span every NPC template (234), SP-only templates are the 36
        // NPCs without an exp reward: both are id lists, unique and positive.
        let mut override_ids = BTreeSet::new();
        for row in &self.drops.corpse_time_overrides {
            ensure(row.id > 0 && override_ids.insert(row.id), || {
                format!("drops: duplicate corpse_time override {}", row.id)
            })?;
        }
        let mut sp_only = BTreeSet::new();
        for id in &self.drops.sp_only_templates {
            ensure(*id > 0 && sp_only.insert(*id), || {
                format!("drops: duplicate SP-only template {id}")
            })?;
        }
        inv.corpse_time_overrides = len(&self.drops.corpse_time_overrides);
        inv.sp_only_templates = len(&self.drops.sp_only_templates);
        ensure(
            self.drops.corpse_time_overrides_count == inv.corpse_time_overrides
                && self.drops.sp_only_templates_count == inv.sp_only_templates,
            || "drops: declared counts disagree with the rows".into(),
        )?;

        // -- item 2: rate chain and G.
        let g_rational = Rational::parse_decimal(&self.rates.global_multiplier_g)
            .map_err(|err| format!("rates: G {err}"))?;
        let g = g_rational.fl().map_err(|err| format!("rates: G {err}"))?;
        let lower = Rational::integer(1)
            .div(&Rational::integer(1_i64 << 32))
            .map_err(e)?;
        let upper = Rational::integer(1_i64 << 32);
        ensure(
            g.rational().map_err(e)?.compare(&lower).is_ge()
                && g.rational().map_err(e)?.compare(&upper).is_le(),
            || format!("rates: G = {} is outside [2^-32, 2^32]", self.rates.global_multiplier_g),
        )?;
        ensure(
            Binary64::parse_hex(&self.rates.global_multiplier_g_bits).map_err(e)? == g,
            || "rates: G bits differ from fl(G)".into(),
        )?;
        let configs = self
            .rates
            .double_rates
            .iter()
            .chain(&self.rates.float_rates)
            .chain(&self.rates.int_rates)
            .chain(&self.rates.item_overrides);
        for (name, config) in configs {
            let label = format!("rates {name}");
            let kinds = match config.kind.as_str() {
                "double" => &self.rates.double_rates,
                "float" => &self.rates.float_rates,
                "int" => &self.rates.int_rates,
                other => return Err(format!("{label}: unknown kind {other}")),
            };
            ensure(
                kinds.contains_key(name)
                    || (config.kind == "float" && self.rates.item_overrides.contains_key(name)),
                || format!("{label}: kind {} in the wrong table", config.kind),
            )?;
            ensure(
                config.item_id.is_some() == self.rates.item_overrides.contains_key(name),
                || format!("{label}: item_id placement"),
            )?;
            let parsed = if config.kind == "float" {
                fl32_decimal(&config.value)
            } else {
                fl_decimal(&config.value)
            }
            .map_err(|err| format!("{label}: value {err}"))?;
            if config.kind == "int" {
                ensure(!config.value.contains('.'), || format!("{label}: int with a fraction"))?;
            }
            let multiplied = G_MULTIPLIED_RATES.contains(&name.as_str());
            ensure(
                multiplied == config.rate_prime.is_some()
                    && multiplied == config.rate_prime_bits.is_some(),
                || format!("{label}: rate_prime presence disagrees with the G-multiplied set"),
            )?;
            if let (Some(prime), Some(bits)) = (&config.rate_prime, &config.rate_prime_bits) {
                let expected = g_fold(parsed, g).map_err(|err| format!("{label}: {err}"))?;
                ensure(Binary64::parse_hex(bits).map_err(e)? == expected, || {
                    format!(
                        "{label}: rate_prime_bits {bits} but fl32(fl(rate × G)) = {}",
                        expected.hex()
                    )
                })?;
                let decimal = Rational::parse_decimal(prime)
                    .map_err(|err| format!("{label}: rate_prime {err}"))?;
                ensure(
                    decimal
                        .compare(&expected.rational().unwrap_or_else(|_| Rational::integer(0)))
                        .is_eq(),
                    || {
                        format!(
                            "{label}: rate_prime {prime} is not the exact value of {}",
                            expected.hex()
                        )
                    },
                )?;
                inv.g_multiplied_rates = inv.g_multiplied_rates.saturating_add(1);
            }
        }
        ensure(inv.g_multiplied_rates == len(&G_MULTIPLIED_RATES), || {
            "rates: a G-multiplied config is missing".into()
        })?;
        ensure(!self.rates.deep_blue.champion_enable, || {
            "rates: ChampionEnable must be false (no champion value is executable)".into()
        })?;
        ensure(self.rates.drop_strategy.precise_drop_calculation, || {
            "rates: PreciseDropCalculation must be true".into()
        })?;

        // -- item 3: ground constants and PK policy.
        for row in &self.ground.ground.auto_destroy_time {
            closure.require("ground", row.item_id)?;
        }
        ensure(self.ground.pk.pk_drop_protection == "None", || {
            "ground: pk_drop_protection must be None (E24)".into()
        })?;
        let distinct: BTreeSet<_> = self.ground.pk.non_droppable_items.iter().collect();
        ensure(self.ground.pk.non_droppable_items.len() == 30 && distinct.len() == 29, || {
            "ground: NonDroppableItems is 30 entries / 29 ids".into()
        })?;
        ensure(self.ground.pk.pet_items.len() == 11, || "ground: PetItems has 11 ids".into())?;
        ensure(
            self.ground.ground.scanner_ms > 0
                && self.ground.ground.decay_seconds > 0
                && self.ground.ground.herb_decay_seconds > 0,
            || "ground: zero timer".into(),
        )?;

        // -- item 4: party constants (ExponentXp/Sp nonzero fails closed, E42).
        let party = &self.party.party;
        ensure(party.exponent_xp == 0 && party.exponent_sp == 0, || {
            "party: ExponentXp/ExponentSp must be 0 (E42)".into()
        })?;
        ensure(party.size_cap == 9, || "party: size cap is 9".into())?;
        ensure(party.bonus_exp_sp.len() == 9 && party.bonus_exp_sp_bits.len() == 9, || {
            "party: BONUS_EXP_SP has 9 entries".into()
        })?;
        for (decimal, bits) in party.bonus_exp_sp.iter().zip(&party.bonus_exp_sp_bits) {
            check_bits("party bonus", decimal, bits)?;
        }
        Rational::parse_decimal(&party.xp_cutoff.party_xp_cutoff_percent)
            .map_err(|err| format!("party: cutoff percent {err}"))?;
        ensure(party.xp_cutoff.party_xp_cutoff_method == "highfive", || {
            "party: cutoff method".into()
        })?;
        let mut next = 0_u32;
        for gap in &party.xp_cutoff.cutoff_gaps {
            ensure(gap.min == next && gap.max >= gap.min && gap.percent <= 100, || {
                "party: cutoff gaps are not contiguous".into()
            })?;
            next = gap.max.saturating_add(1);
        }

        // -- item 5: herbs.
        let mut herb_ids = BTreeSet::new();
        let mut skill_ids = BTreeSet::new();
        for herb in &self.herbs.handled {
            ensure(herb_ids.insert(herb.id) && !herb.skills.is_empty(), || {
                format!("herbs: duplicate or skill-less handled herb {}", herb.id)
            })?;
            closure.require("herbs", herb.id)?;
            skill_ids.extend(herb.skills.iter().map(|skill| skill.id));
        }
        for herb in &self.herbs.unhandled {
            ensure(herb_ids.insert(herb.id), || {
                format!("herbs: duplicate unhandled herb {}", herb.id)
            })?;
            closure.require("herbs", herb.id)?;
        }
        inv.herbs_handled = len(&self.herbs.handled);
        inv.herbs_unhandled = len(&self.herbs.unhandled);
        inv.herb_skill_ids = u64::try_from(skill_ids.len()).unwrap_or(u64::MAX);
        ensure(
            self.herbs.total_herbs == inv.herbs_handled.saturating_add(inv.herbs_unhandled)
                && self.herbs.handled_herbs_count == inv.herbs_handled
                && self.herbs.unhandled_herbs_count == inv.herbs_unhandled
                && self
                    .herbs
                    .distinct_skill_ids
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    == skill_ids,
            || "herbs: declared counts or skill ids disagree with the rows".into(),
        )?;
        for blocked in &self.catalogue.non_executable_herbs {
            ensure(self.herbs.handled.iter().any(|herb| herb.id == blocked.id), || {
                format!("catalogue: non-executable herb {} is not a handled herb", blocked.id)
            })?;
        }

        // -- item 6: merchants and the unit-price table.
        let mut reference: BTreeMap<u32, u64> = BTreeMap::new();
        let mut pairs: BTreeSet<(u32, &str)> = BTreeSet::new();
        let mut per_list: BTreeMap<u64, u64> = BTreeMap::new();
        for entry in &self.merchants.entries {
            closure.require("merchants", entry.item)?;
            ensure(
                *reference.entry(entry.item).or_insert(entry.ref_price) == entry.ref_price,
                || format!("merchants: template {} has two reference prices", entry.item),
            )?;
            Rational::parse_decimal(&entry.tax).map_err(|err| format!("merchants: tax {err}"))?;
            Rational::parse_decimal(&entry.delay)
                .map_err(|err| format!("merchants: delay {err}"))?;
            ensure(entry.zero_price == (entry.ref_price == 0), || {
                format!("merchants: zero_price flag on {}", entry.item)
            })?;
            ensure(entry.limited == (entry.count > 0), || {
                format!("merchants: limited flag on {}", entry.item)
            })?;
            ensure(!entry.list_ids.is_empty() && entry.list_ids.len() == entry.npcs.len(), || {
                "merchants: entry without lists".into()
            })?;
            for list in &entry.list_ids {
                let seen: &mut u64 = per_list.entry(*list).or_default();
                *seen = seen.saturating_add(1);
            }
            pairs.insert((entry.item, entry.tax.as_str()));
            if entry.limited {
                inv.merchant_limited = inv.merchant_limited.saturating_add(1);
            }
        }
        inv.merchant_entries = len(&self.merchants.entries);
        inv.merchant_pairs = u64::try_from(pairs.len()).unwrap_or(u64::MAX);
        inv.buylists = len(&self.merchants.buylists);
        let zero_priced: BTreeSet<u32> = self
            .merchants
            .entries
            .iter()
            .filter(|entry| entry.zero_price)
            .map(|entry| entry.item)
            .collect();
        ensure(
            self.merchants.total_entries == inv.merchant_entries
                && self.merchants.total_pairs == inv.merchant_pairs
                && self.merchants.limited_entries == inv.merchant_limited
                && self.merchants.buylists_count == inv.buylists
                && self
                    .merchants
                    .zero_price_items
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    == zero_priced
                && self.merchants.castle_percents == [0, 5, 15, 25],
            || "merchants: declared counts disagree with the rows".into(),
        )?;
        let mut buylist_ids = BTreeSet::new();
        for buylist in &self.merchants.buylists {
            ensure(buylist_ids.insert(buylist.list_id), || {
                format!("merchants: duplicate buylist {}", buylist.list_id)
            })?;
            ensure(per_list.get(&buylist.list_id) == Some(&buylist.entries), || {
                format!("merchants: buylist {} entry count", buylist.list_id)
            })?;
        }
        ensure(buylist_ids.len() == per_list.len(), || {
            "merchants: entries reference an undeclared buylist".into()
        })?;
        let mut table_pairs = BTreeSet::new();
        let mut naive_differences = 0_u64;
        let mut previous: Option<(u32, &str)> = None;
        for row in &self.merchants.unit_prices {
            let key = (row.item, row.tax.as_str());
            ensure(previous.is_none_or(|p| p < key), || {
                "merchants: unit_prices are not sorted".into()
            })?;
            previous = Some(key);
            ensure(table_pairs.insert(key) && pairs.contains(&key), || {
                format!("merchants: unit price row {key:?} without an entry")
            })?;
            let price = *reference
                .get(&row.item)
                .ok_or_else(|| format!("merchants: unit price for unknown {}", row.item))?;
            for (castle, frozen) in [
                (0, row.castle_0),
                (5, row.castle_5),
                (15, row.castle_15),
                (25, row.castle_25),
            ] {
                let computed = vendor_unit_price(price, &row.tax, castle)
                    .map_err(|err| format!("merchants: unit price {key:?}: {err}"))?;
                ensure(computed == frozen, || {
                    format!("merchants: unit price {key:?} castle {castle}: table {frozen}, fl chain {computed}")
                })?;
                inv.unit_price_cells = inv.unit_price_cells.saturating_add(1);
            }
            let tax: u64 = row
                .tax
                .parse()
                .map_err(|_| format!("merchants: tax literal {} is not an integer", row.tax))?;
            let naive = price
                .checked_mul(tax.checked_add(100).ok_or("tax overflow")?)
                .ok_or("naive price overflow")?
                / 100;
            if naive != row.castle_0 {
                naive_differences = naive_differences.saturating_add(1);
            }
        }
        ensure(table_pairs == pairs, || {
            "merchants: unit_prices do not cover every (template, tax) pair exactly once".into()
        })?;
        ensure(
            self.merchants.naive_truncation_differences_castle_0 == naive_differences,
            || format!("merchants: {naive_differences} naive-truncation differences at castle 0"),
        )?;
        ensure(
            self.merchants
                .castle_taxes
                .castles
                .iter()
                .all(|castle| castle.tax_percent == 0),
            || "merchants: pinned castle tax is 0 (E3)".into(),
        )?;

        // -- item 7: multisell.
        let mut list_ids = BTreeSet::new();
        let special: BTreeSet<i64> = self
            .catalogue
            .special_multisell_ingredients
            .iter()
            .map(|row| row.id)
            .collect();
        let (mut taxed, mut enchant, mut maintain) = (0_u64, 0_u64, 0_u64);
        for list in &self.multisell.list {
            ensure(list_ids.insert(list.id) && !list.entries.is_empty(), || {
                format!("multisell: duplicate or empty list {}", list.id)
            })?;
            taxed = taxed.saturating_add(u64::from(list.apply_taxes));
            enchant = enchant.saturating_add(u64::from(list.maintain_enchantment));
            for entry in &list.entries {
                ensure(!entry.ingredients.is_empty() && !entry.products.is_empty(), || {
                    format!("multisell list {}: empty entry", list.id)
                })?;
                for ingredient in &entry.ingredients {
                    maintain = maintain.saturating_add(u64::from(ingredient.maintain));
                    ensure(ingredient.count > 0, || {
                        format!("multisell list {}: zero ingredient count", list.id)
                    })?;
                    if !special.contains(&ingredient.id) {
                        closure.require(
                            "multisell",
                            u32::try_from(ingredient.id).map_err(|_| {
                                format!("multisell list {}: ingredient id", list.id)
                            })?,
                        )?;
                    }
                }
                for product in &entry.products {
                    ensure(product.count > 0, || {
                        format!("multisell list {}: zero product count", list.id)
                    })?;
                    if !special.contains(&product.id) {
                        closure.require(
                            "multisell",
                            u32::try_from(product.id)
                                .map_err(|_| format!("multisell list {}: product id", list.id))?,
                        )?;
                    }
                }
                inv.multisell_entries = inv.multisell_entries.saturating_add(1);
            }
        }
        inv.multisell_lists = len(&self.multisell.list);
        ensure(
            self.multisell.total_lists == inv.multisell_lists
                && self.multisell.apply_taxes_lists == taxed
                && self.multisell.maintain_enchantment_lists == enchant
                && self.multisell.maintain_ingredient_ingredients == maintain
                && self.multisell.total_files
                    == inv
                        .multisell_lists
                        .saturating_add(len(&self.multisell.deviations)),
            || "multisell: declared counts disagree with the rows".into(),
        )?;
        for row in &self.catalogue.special_multisell_ingredients {
            ensure(row.lists_admitted && row.lists.iter().all(|id| list_ids.contains(id)), || {
                format!("catalogue: special ingredient {} names an unknown list", row.id)
            })?;
        }

        // -- item 8: recipes.
        let mut recipe_ids = BTreeSet::new();
        let mut recipe_items: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let (mut dwarven, mut common) = (0_u64, 0_u64);
        for recipe in &self.recipes.recipe {
            ensure(recipe_ids.insert(recipe.id), || {
                format!("recipes: duplicate recipe {}", recipe.id)
            })?;
            recipe_items
                .entry(recipe.recipe_id)
                .or_default()
                .push(recipe.id);
            closure.require("recipes", recipe.recipe_id)?;
            closure.require(
                "recipes",
                u32::try_from(recipe.production.id).map_err(|_| "recipes: product id")?,
            )?;
            ensure(
                recipe.production.count > 0
                    && (1..=100).contains(&recipe.success_rate)
                    && recipe.craft_level >= 1,
                || format!("recipes: recipe {} bounds", recipe.id),
            )?;
            for ingredient in &recipe.ingredients {
                ensure(ingredient.count > 0, || {
                    format!("recipes: recipe {} zero ingredient", recipe.id)
                })?;
                closure.require(
                    "recipes",
                    u32::try_from(ingredient.id).map_err(|_| "recipes: ingredient id")?,
                )?;
            }
            match recipe.r#type.as_str() {
                "dwarven" => dwarven = dwarven.saturating_add(1),
                "common" => common = common.saturating_add(1),
                other => return Err(format!("recipes: recipe {} type {other}", recipe.id)),
            }
            if let Some(rare) = &recipe.production_rare {
                closure.require("recipes", rare.id)?;
                ensure(matches!(rare.rarity, 1 | 4 | 20) && rare.count > 0, || {
                    format!("recipes: recipe {} rare product", recipe.id)
                })?;
                inv.rare_recipes = inv.rare_recipes.saturating_add(1);
            }
        }
        inv.recipes = len(&self.recipes.recipe);
        ensure(
            self.recipes.total_recipes == inv.recipes
                && self.recipes.total_rare_recipes == inv.rare_recipes,
            || "recipes: declared counts disagree with the rows".into(),
        )?;
        ensure(dwarven > 0 && common > 0, || "recipes: a recipe type is empty".into())?;
        let shared: BTreeMap<u32, Vec<u32>> = recipe_items
            .iter()
            .filter(|(_, ids)| ids.len() > 1)
            .map(|(item, ids)| (*item, ids.clone()))
            .collect();
        let declared: BTreeMap<u32, Vec<u32>> = self
            .catalogue
            .shared_recipe_items
            .iter()
            .map(|row| (row.recipe_item, row.recipes.clone()))
            .collect();
        ensure(shared == declared, || {
            "recipes: shared recipe items disagree with the catalogue".into()
        })?;
        ensure(
            self.recipes.config.craft_masterwork && !self.recipes.config.alternative_crafting,
            || "recipes: CraftMasterwork true, AlternativeCrafting false".into(),
        )?;
        ensure(
            self.recipes.config.dwarf_recipe_limit == 50
                && self.recipes.config.common_recipe_limit == 50,
            || "recipes: limits 50/50".into(),
        )?;
        for table in [&self.recipes.create_item, &self.recipes.create_common_item] {
            let mut last = 0;
            for rank in &table.ranks {
                ensure(rank.rank > last && rank.max_craft_level >= rank.rank, || {
                    format!("recipes: rank table {}", table.skill_id)
                })?;
                last = rank.rank;
            }
        }

        // -- item 9: storage, mail and flood (every interval present, ServerBypass 5).
        let flood = &self.storage_mail.flood_intervals_ticks;
        ensure(
            [
                flood.multisell,
                flood.transaction,
                flood.manufacture,
                flood.manor,
                flood.send_mail,
                flood.server_bypass,
                flood.item_auction,
            ]
            .iter()
            .all(|interval| *interval > 0)
                && flood.server_bypass == 5,
            || "storage_mail: a flood interval is zero or ServerBypass is not 5".into(),
        )?;
        ensure(
            self.storage_mail.warehouse.slots_dwarf >= self.storage_mail.warehouse.slots_no_dwarf,
            || "storage_mail: warehouse slots".into(),
        )?;
        ensure(
            self.storage_mail.freight.slots > 0 && self.storage_mail.mail.max_attachments > 0,
            || "storage_mail: zero capacity".into(),
        )?;

        // -- item 10: item auction and manor.
        let mut instance_ids = BTreeSet::new();
        for instance in &self.auction.instance {
            ensure(instance_ids.insert(instance.id), || {
                format!("auction: duplicate instance {}", instance.id)
            })?;
            let mut auction_ids = BTreeSet::new();
            for item in &instance.items {
                ensure(
                    auction_ids.insert(item.auction_id)
                        && item.count > 0
                        && item.init_bid > 0
                        && item.length_minutes > 0,
                    || format!("auction: instance {} item {}", instance.id, item.auction_id),
                )?;
                closure.require("auction", item.item_id)?;
                inv.auction_items = inv.auction_items.saturating_add(1);
            }
        }
        ensure(self.auction.total_instances == len(&self.auction.instance), || {
            "auction: declared instance count".into()
        })?;
        let mut scheduled = BTreeSet::new();
        for row in &self.auction.fixture_schedule.auctions {
            ensure(
                scheduled.insert(row.auction_id)
                    && instance_ids.contains(&row.instance)
                    && row.end_epoch_ms > row.start_epoch_ms,
                || format!("auction: schedule row {}", row.auction_id),
            )?;
            ensure(row.round < self.auction.fixture_schedule.rounds_per_instance, || {
                format!("auction: round of {}", row.auction_id)
            })?;
        }
        let (mut alternative, mut gludio) = (0_u64, 0_u64);
        let mut castle_crops: BTreeSet<(u32, u32)> = BTreeSet::new();
        for crop in &self.manor.crops {
            ensure(castle_crops.insert((crop.castle, crop.crop)), || {
                format!("manor: duplicate crop {} at castle {}", crop.crop, crop.castle)
            })?;
            for id in [crop.crop, crop.seed, crop.mature, crop.rew1, crop.rew2] {
                closure.require("manor", id)?;
            }
            alternative = alternative.saturating_add(u64::from(crop.alternative));
            if crop.castle == self.manor.fixture_gludio.castle_id {
                gludio = gludio.saturating_add(1);
            }
        }
        inv.crops = len(&self.manor.crops);
        ensure(
            self.manor.total_crops == inv.crops
                && self.manor.total_alternative_crops == alternative
                && self.manor.gludio_crops_count == gludio,
            || "manor: declared counts disagree with the rows".into(),
        )?;
        let fixture = &self.manor.fixture_gludio;
        ensure(fixture.castle_id == 1 && fixture.castle_area == 1, || {
            "manor: fixture lists are Gludio-only (E28)".into()
        })?;
        for row in &fixture.seed_production {
            ensure(
                self.manor
                    .crops
                    .iter()
                    .any(|crop| crop.castle == 1 && crop.seed == row.seed_id),
                || format!("manor: fixture seed {}", row.seed_id),
            )?;
            inv.gludio_fixture_rows = inv.gludio_fixture_rows.saturating_add(1);
        }
        for row in &fixture.crop_procurement {
            ensure(
                self.manor
                    .crops
                    .iter()
                    .any(|crop| crop.castle == 1 && crop.crop == row.crop_id),
                || format!("manor: fixture crop {}", row.crop_id),
            )?;
            inv.gludio_fixture_rows = inv.gludio_fixture_rows.saturating_add(1);
        }
        for id in &self.manor.skills.harvester_items {
            closure.require("manor", *id)?;
        }
        ensure(
            self.manor.skills.rate_drop_manor
                == self
                    .rates
                    .int_rates
                    .get("RateDropManor")
                    .and_then(|r| r.value.parse().ok())
                    .unwrap_or(0),
            || "manor: RateDropManor disagrees with rates.toml".into(),
        )?;

        // -- item 11: adaptations, the L table, the pow table and the bindings.
        let labels: BTreeSet<&str> = self
            .adaptations
            .adaptations
            .iter()
            .map(|row| row.label.as_str())
            .collect();
        ensure(labels.len() == self.adaptations.adaptations.len() && labels.len() == 37, || {
            "adaptations: 37 distinct labels X1–X14, P1–P21 (with P7b), T1".into()
        })?;
        ensure(
            self.adaptations
                .adaptations
                .iter()
                .filter(|row| row.withdrawn)
                .map(|row| row.label.as_str())
                .collect::<BTreeSet<_>>()
                == ["X5", "X6"].into_iter().collect(),
            || "adaptations: exactly X5 and X6 are withdrawn".into(),
        )?;
        inv.adaptations = len(&self.adaptations.adaptations);
        let expected = expected_l_table(&self.rates.deep_blue)?;
        ensure(expected.len() == self.adaptations.l_table.len(), || {
            format!(
                "adaptations: L table has {} rows, expected {}",
                self.adaptations.l_table.len(),
                expected.len()
            )
        })?;
        let mut distinct_bits = BTreeSet::new();
        for (row, (strategy, condition, value)) in self.adaptations.l_table.iter().zip(&expected) {
            ensure(row.strategy == *strategy && row.d_condition == *condition, || {
                format!("adaptations: L row {strategy} {condition} order")
            })?;
            let frozen = Binary64::parse_hex(&row.bits).map_err(e)?;
            ensure(frozen == *value, || {
                format!(
                    "adaptations: L {strategy} {condition} froze {} but §3.3 gives {}",
                    row.bits,
                    value.hex()
                )
            })?;
            check_bits(&format!("adaptations L {strategy} {condition}"), &row.value, &row.bits)?;
            distinct_bits.insert(row.bits.as_str());
        }
        inv.l_table_rows = len(&self.adaptations.l_table);
        ensure(
            self.adaptations.l_table_rows == inv.l_table_rows
                && self.adaptations.l_table_distinct_bits
                    == u64::try_from(distinct_bits.len()).unwrap_or(u64::MAX),
            || "adaptations: declared L table counts".into(),
        )?;
        let pow_ref = &self.sources.oracle_tables.jvm_pow_tables;
        ensure(
            self.adaptations.oracle_table.sha256 == pow_ref.sha256
                && self.pow.sha256 == pow_ref.sha256
                && self.adaptations.oracle_table.path == pow_ref.path,
            || {
                "pow table: sha256 or path disagrees between adaptations.toml, sources.toml and the file".into()
            },
        )?;
        ensure(
            pow_ref.pow56_entries == len(&self.pow.pow56)
                && pow_ref.pow13_entries == len(&self.pow.pow13)
                && pow_ref.total_entries
                    == len(&self.pow.pow56).saturating_add(len(&self.pow.pow13)),
            || "pow table: entry counts".into(),
        )?;
        ensure(
            self.pow.pow56.len() == 79
                && self.pow.pow13.len() == 241
                && self.pow.java_version == pow_ref.java_version,
            || "pow table: 79 + 241 entries from the pinned JVM".into(),
        )?;
        for (index, bits) in self.pow.pow56.iter().enumerate() {
            let k = i32::try_from(index)
                .map_err(|_| "pow56 index")?
                .checked_add(1)
                .ok_or("pow56 index")?;
            let computed = pow56(k).map_err(e)?;
            ensure(Binary64::parse_hex(bits).map_err(e)? == computed, || {
                format!("pow56 k={k}: captured {bits}, exact power rounded once {}", computed.hex())
            })?;
        }
        for (index, bits) in self.pow.pow13.iter().enumerate() {
            let d = i32::try_from(index)
                .map_err(|_| "pow13 index")?
                .checked_sub(120)
                .ok_or("pow13 index")?;
            let computed = pow13(d).map_err(e)?;
            ensure(Binary64::parse_hex(bits).map_err(e)? == computed, || {
                format!("pow13 d={d}: captured {bits}, exact power rounded once {}", computed.hex())
            })?;
        }
        inv.pow_entries = len(&self.pow.pow56).saturating_add(len(&self.pow.pow13));

        // Closure: the catalogue is exactly the set of referenced templates.
        ensure(closure.referenced.len() == templates.len(), || {
            format!(
                "catalogue: {} templates but {} referenced by the tables",
                templates.len(),
                closure.referenced.len()
            )
        })?;
        Ok((inv, g))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn l_table_recomputation_matches_the_design_table() {
        let deep_blue = DeepBlue {
            use_deep_blue_drop_rules: true,
            use_deep_blue_drop_rules_raid: true,
            drop_adena_min_level_difference: 8,
            drop_adena_max_level_difference: 15,
            drop_adena_min_level_gap_chance: 10,
            drop_item_min_level_difference: 5,
            drop_item_max_level_difference: 10,
            drop_item_min_level_gap_chance: 10,
            champion_enable: false,
        };
        let rows = expected_l_table(&deep_blue).unwrap();
        assert_eq!(rows.len(), 22);
        let hex: Vec<String> = rows.iter().map(|(_, _, v)| v.hex()).collect();
        assert_eq!(
            hex,
            [
                "3ff0000000000000",
                "3fea3d70a3d70a3d",
                "3fe47ae147ae147b",
                "3fdd70a3d70a3d71",
                "3fd1eb851eb851ec",
                "3fb999999999999a",
                "3ff0000000000000",
                "3feb333333333333",
                "3fe6666666666666",
                "3fe199999999999a",
                "3fd999999999999a",
                "3fd0000000000000",
                "3fb99999999999a0",
                "0000000000000000",
                "3ff0000000000000",
                "3febe2be2be2be2c",
                "3fe7c57c57c57c58",
                "3fe3a83a83a83a84",
                "3fdf15f15f15f15f",
                "3fd6db6db6db6db7",
                "3fcd41d41d41d41d",
                "3fb999999999999a",
            ]
        );
        assert_eq!(
            rows.iter()
                .map(|(_, _, v)| v.hex())
                .collect::<BTreeSet<_>>()
                .len(),
            19
        );
        assert_eq!(rows.get(6).map(|r| (r.0.as_str(), r.1.as_str())), Some(("raid_clamp", ">= 0")));
        assert_eq!(rows.get(13).map(|r| r.1.as_str()), Some("<= -7"));
        assert_eq!(rows.get(21).map(|r| r.1.as_str()), Some("<= -15"));
    }
}
