> **BINDING RECORD — Phase 4 design, pending architect dispositions in §13 and independent review.** This is the implementation contract for new Snapshot 10 / Record 8 / Binary V6 epochs. It does not claim that the implementation or acceptance gates already pass.
>
> **ARCHITECT ERRATA:** O1–O4 and A1–A22 in the Phase 4 decision record supersede Phase 3 where expressly amended. Source corrections and unresolved conflicts are listed below and in §13; a proposal marked QUESTION is not an approved architecture amendment. Existing Phase 3 C1–C18 remain binding.

# Phase 4 zone engine: items and equipment

## Source authority and reconciliation

Citations use **Z/** = `apps/api/src/domain/zone/`, **R/** = `apps/api/src/application/replay_log/`, **A/** = `apps/api/src/application/`, **W** = `packages/proto/nightfall/v1/world.proto`, **P** = `packages/proto/nightfall/v1/game.proto`. **G/** = `/home/matt-woodruff/.cache/nightfall/l2j/l2j-game/src/main/java/com/l2jserver/gameserver/`, revision `abfde0490ac52a2106e884b7242c0276c44cdf76`; **GC** = that repository's `src/main/resources/config/character.properties`. **D/** = `/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp/src/main/resources/data/`, **H/** = that repository's `src/main/java/com/l2jserver/datapack/handlers/`, revision `3ca488dd2bd0bfaca43e378886a3c2e37968153a`.

**N/** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase4-codex-2026-10-12/`; **AD** = `N/architecture-decisions.md`; **P3** = `docs/plans/phase-3-zone-design.md`; **C** = `docs/plans/phase-3-architecture-decisions.md`. References describe the inspected base branch, not proposed future source line numbers. Every **new contract** below is a design decision under AD:15–36, not a claim that L2J already implements Nightfall's scheduling, IDs, wire or storage. XML decimals are imported as exact rational strings, never binary floating-point values. All executable formulas here use integers, Q, or exact ratios (AD:15).

| Conflict / old assumption | Binding Phase 4 treatment and evidence |
|---|---|
| Universal Squire's Sword and `using SWORD` | V6 reads actual equipment or class fist bases; preserve V5 `StarterWeaponProfile` validation. `Z/skill_profiles.rs:184–266`, `Z/effective_stats.rs:69–138`, `Z/state_runtime.rs:1304`; kits `D/stats/initialEquipment.xml:4–109`. Orcs use Training Gloves and Dwarves a club; “all fighters sword/all mystics wand” is only shorthand. |
| 04's POLE one-hand enchant row | Bodypart controls the row; lrhand pole gets two-hand increment. `G/model/stats/functions/FuncEnchant.java:85–151`. |
| AD A9 calls all soulshot terms ×2 | Blow P.Atk multiplier is **1458/1000**; regular auto/physical-skill multiplier is 2. `G/model/stats/Formulas.java:517–528,684,763–774`. QUESTION Q3. |
| AD A10 describes shield damage subtraction | Pinned `ShieldBlocks=False` adds sDef to defence; perfect block returns 1. `GC:136–140`; `G/model/stats/Formulas.java:670–678`. QUESTION Q3. |
| Retail augmentation mixed with old offsets | Retail table branch returns before offset algorithm; `GC:393–405`, `G/datatables/AugmentationData.java:447–559,563–686`. QUESTION Q4. |
| “SA costs base weapon + crystal; C stages 7–8” | Actual multisell ingredients include gemstones and Adena; C Stormbringer uses stage 5. `D/multisell/1005.xml:19–41`. |
| “Soul-crystal stages through 16” | Import every explicit transition, including 17→18; no leveling execution in Phase 4. `D/levelUpCrystalData.xml:4–59`; AD:8. |
| Enchant armour “0 beyond +19” | Ordinary armour current +19 still attempts +20; full armour current +19 uses its shifted row; +20 onward has zero **base** chance. Bonus success at base zero must not increment. `D/enchantItemGroups.xml:4–45`; `G/network/clientpackets/RequestEnchantItem.java:163–167`. |
| UUID item IDs, separate inventory stream, DB-first actor, floats/hot reload in 04 | Rejected by AD:15–20. IDs and data are zone-canonical, immutable per epoch, additive existing WS stream, durable critical path. |
| Phase 3 deferred shield/shots/position/FAN | Executable only in V6; Phase 3 and older recordings keep their exact code paths, fields and RNG draws (AD:18,24). |

## 1. Identity, canonical state and hydration

### 1.1 Ownership and identity

**New contract.** Add `item.rs`, `item_rules.rs`, `phase4.rs`, `phase4_checkpoint.rs`, `item_math.rs`, `state_items.rs` and focused item combat helpers to `Z/mod.rs` and its deterministic SOURCES scan. Extend `ZoneState` with `phase4: Option<Phase4Zone>` beside Phase3; do not place default equipment into legacy `Entity`. Reuse the single tick draft, effective-stat cache, schedule and checkpoint lane; no inventory actor, timer task or independently published inventory transaction (AD:15–19; `Z/phase3.rs:270–423`, `Z/state_runtime.rs:534–609`).

`ItemInstanceId(u64)` is opaque and stable across equip, enchant, reconnect and checkpoint restore. Zero is invalid. Creation allocates checked `next_item_instance`, then advances it; consumption to zero destroys that identity permanently. Counts must fit positive signed bigint; no wrapping or saturating grants. Stack merge retains the existing lowest instance ID and allocates nothing. Nonstackables have count=1; stackability comes from the template (`G/model/items/L2Item.java:173`; AD:19,34). SA exchange consumes an old instance and allocates a new one (§7); augmentation/enchant mutate an existing instance.

**QUESTION Q1:** A zone-local counter starting at one cannot also be a global database primary key across epochs/zones. Until architect resolution, implementation must fail admission on overlapping identity ownership; it must not silently renumber loaded items. Proposed solution: persist disjoint positive-bigint allocator ranges, lease them outside simulation, record `{range_start, next_item_instance, range_end_exclusive}` in admission/snapshot, and consume monotonically within them. Record replenishment as an ordered external input, never fetch a DB sequence in the domain. No bit partition, block size or identifier budget is invented here. Imported fixture IDs are globally reserved before admission, with `next_item_instance > max(seed ids)` (AD:19,36). This question blocks allocator/DB schema freeze, not pure mechanics work.

### 1.2 Phase4Rules

**New contract.** `Phase4Rules::try_new` validates all cross-references before calculating a separate hash; Deserialize repeats validation and verifies the supplied hash, as `Phase3Rules` does (`Z/phase3.rs:91–158,230–264`). Canonical hash input starts with a new domain separator `nightfall.phase4.rules.1\0`, then length-delimited, explicitly typed, ordered encodings of:

- Full item definitions: stable template ID/kind/grade/bodypart, weight, counts, price/crystal parameters, flags/conditions, XML-ordered funcs, item/enchant4 skills, shot consumption and reduction chances, bow MP/reuse, attack range/angle/random damage, equipment skill reuse, enchant options. Preserve XML ordinal even when names coincide. Defaults come from `G/model/items/L2Item.java:157–218`, `L2Weapon.java:82–108`, `L2Armor.java:47–80`, `L2EtcItem.java:49–73`.
- Weapon kind/trait masks (`G/model/items/type/WeaponType.java:27–63`), generated template statistics, grade IDs and S-plus mapping, enchant groups/scroll/support tables, FuncEnchant and enchant-HP tables, blacklist, armour sets and base-stat set deltas (`D/enchantItemGroups.xml:4–92`; `G/model/stats/functions/formulas/FuncArmorSet.java:51–67`). WeaponType itself supplies traits/masks, **not** universal attack-speed/critical/stat defaults; item XML supplies those.
- Augmentation retail rows in original XML order, category weights, options/skill closure, life-stone eligibility/costs/cancel fees; SA recipe ingredients/products/maintain flags; soul-crystal transitions and NPC detail data; per-class starter kits in XML order; capacities/weight/expertise policies. AD:21–23,27–28.
- V6 formula revision and integer adaptation policy, positional geometry/facing policy, blacksmith identity/location/radius, item capability manifest with explicit blockers, and hash of the Phase3 skill/profile bundle used by this epoch. Keep older hashes unchanged; hash normalized semantic values and ordering, not absolute local paths or comments (AD:18; `Z/phase3.rs:138–158`).

Import all Weapon/Armor rows but separate **catalogued** from **executable**: unsupported item conditions, trigger skills or special modes produce named blockers, never partial stat installation. O1-required kinds, armour sets, augmentation options and consumables must have a complete executable closure before release; blockers are not an excuse to ship a missing owner requirement. Common/shadow/time-limited contexts remain excluded according to AD:12. Include 57 (Adena) and every multisell input in EtcItem closure, not just the short A7 list (§7). Replay embeds immutable rules once, interned by content hash; `GetItemCatalog` projects from the same bundle (AD:18,21).

### 1.3 Phase4Snapshot and invariants

**New contract**, typed and `deny_unknown_fields`:

```rust
struct Phase4Snapshot {
    rules: Arc<Phase4Rules>, rules_hash: [u8; 32],
    next_item_instance: u64, // range ownership subject to Q1
    players: BTreeMap<EntityId, Phase4Player>,
    facing: BTreeMap<EntityId, FacingVector>, // every V6 combatant, including NPCs
}
struct Phase4Player {
    inventory: BTreeMap<ItemInstanceId, ItemInstance>,
    paperdoll: BTreeMap<EquipSlot, ItemInstanceId>,
    auto_shots: BTreeSet<ItemTemplateId>,
    item_reuse: BTreeMap<ItemReuseKey, Tick>,
    equipment_sources: BTreeMap<EquipmentSourceKey, u64>,
    active_sets: BTreeMap<SetId, SetActivation>,
    weight: i64, max_load: i64, weight_penalty_bonus: i64,
    overload_level: u8, overloaded: bool,
    expertise: u8, weapon_penalty: u8, armor_penalty: u8,
    starter_kit_receipt: StarterKitReceipt,
    inventory_revision: u64,
}
struct ItemInstance {
    id: ItemInstanceId, template_id: ItemTemplateId, count: u64,
    location: ItemLocation, slot: Option<EquipSlot>, enchant: u32,
    augment_id: Option<u32>, charges: ShotChargeBits,
    created_tick: Tick, revision: u64,
}
```

`ItemLocation` admits only INVENTORY/PAPERDOLL; quest is a template pool, not a DB-only container. Bound is fixed false; duration/mana are absent for excluded shadow/time-limited items. SA is represented by the variant template; never add an independently mutable SA stat field. Charges belong to the weapon instance, not a global character buff (`H/itemhandlers/SoulShots.java:80–98`; `G/model/itemcontainer/PcInventory.java:961–966`). Inventory includes equipped entries exactly once; paperdoll is a checked index, not a second copy. Arrows are the lhand instance and count, with no second arrows counter (`G/model/itemcontainer/Inventory.java:679–713`). Effective sheets and template/slot indexes are rebuilt caches, excluded from digest; compare the serialized weight/penalty/set projections against recomputation on restore. Inventory revision increments once per changed player's committed tick; delta payloads identify that revision (new wire fence under AD:16–19).

Validation requires a sidecar for every admitted player and no unknown owner, unique global instance IDs, valid positive counts and scalar bounds, compatible slot occupancy, nonstackable count=1, no orphan charges or equipment sources, correct recipe/option references, legal reuse keys, allocator ownership, and exact rules hashes. Item skill providers use the existing shared `next_install_sequence`; duplicate IDs/sequence keys are corruption (`Z/phase3.rs:321–355`). Pending V6 swings/casts carry weapon identity, prepaid shot state and projectile/dual hit state (§2); checkpoint recovery must not infer them from the weapon currently equipped. Dead actors retain inventory but cannot restore an active swing/cast. Snapshot recovery preserves all absolute ticks; fresh character admission rebases item-reuse remainders and allocates modifier identities in retained relative order (§9; `Z/phase3_checkpoint.rs:134–170`).

### 1.4 Snapshot budget against C10

The **existing enforced encoded limit is 1,047,552 bytes**, not the full NATS 1,048,576; decoded limit is 33,554,432 (`R/record.rs:221–223`). Keep typed prost state plus interned rule dictionaries from `R/snapshot_phase3.rs:209–241` / `R/snapshot_registry.rs:1–100`. Never repeat item templates, options or installed function bodies per item. The following is conservative arithmetic for the proposed mutable item block, not a compression measurement:

| Component | Bound derived from proposed fixed scalar encoding |
|---|---:|
| Instance: id/count/created_tick/revision/install-reference, template/enchant/augment, enums/flags, nested framing | ≤128 bytes (five worst-case 11-byte u64 fields + three 6-byte u32 fields + flags/slot/location/framing ≤55) |
| Requested eight ordinary inventories | 8 ×180 ×128 =184,320 bytes |
| Dwarf plus quest correction (`GC:317–320`) | 8 ×200 ×128 =204,800 bytes |
| Paperdoll with framing | 8 ×25 ×16 =3,200 bytes (`Inventory.java:54–79`) |
| Per-player scalar/header fences | 8 ×256 =2,048 bytes |
| Charge bits | Included per instance; no extra repeated source data |
| Auto-shot/reuse/provider lists | Variable: bound from admitted catalogue cardinalities; not silently limited to occupied item slots |

Thus fixed mutable Phase4 player data is at most **210,048 bytes** for eight Dwarves under the base capacity, leaving **837,504 encoded bytes before compression** for everything else; ordinary inventories use 189,568 and leave 857,984. These are component arithmetic, **not a proof that the full snapshot fits**. Capacity modifiers can raise 80/100 and item reuse can survive consumption; those maxima must be included. All rules, V5 effects/NPCs/casts, V6 providers/facing, checkpoint receipts/outbox and codec overhead consume the remainder. Compressibility cannot be assumed for adversarial IDs or option sequences.

**QUESTION Q5 / mandatory implementation gate:** generate the full rules and encode the actual worst case (C10's 1,000 combatants/full pools and eight fully equipped players, plus realistic soak), print decoded/encoded bytes and largest contributors. Also test maximum executable inventory-limit modifiers. If bounds fail, compact/intern first; report exact residual excess to architect. No truncation, new storage fetch during replay, or increased NATS limit. This design proves the base inventory component fits; it deliberately does not fabricate a full-envelope result without the generated rules/codec.

### 1.5 Owner baseline and visible equipment

**New contract.** AOI admission/replacement order is: all identity `EntitySpawn`s (including visible equipment), reference-bearing swing/cast/effect hydration, owner `SkillStateSync`, then owner `InventorySync`. InventorySync carries `entity`, `incarnation`, `state_tick`, `baseline=true`, `baseline_tick`, `inventory_revision`, full items/paperdoll, auto-shot toggles, weapon charge bits, item reuse deadlines, capacities/weight and penalties. No owner baseline is filtered because an identical observer state was seen (C2/C4/C12; P3:166–210). Socket replacement preserves live charges/reuse; true fresh admission restores checkpoint state without charging again.

Public `VisibleEquipment` contains `(slot, template_id, glow_tier)` only, including an explicit empty slot/tombstone in deltas. No instance IDs, counts, exact enchant, augmentation or inventory revision in public entries. `EntitySpawn.visible_equipment` and `EquipmentChanged.visible` use identical projection. Owner's exact item mutations arrive separately in InventoryChanged; EquipmentChanged can therefore be safely shared with observers. Glow tiers are a **placeholder presentation policy**, not a claimed L2 client retail threshold; proposed thresholds reuse announcement milestones in §4 and require Q10 approval. StatsChanged stays owner-only; effective maximum HP changes additionally use public VitalsChanged (AD:16; C4; C2'). Baseline fences reject prior incarnation/older state_tick and atomically replace the owner inventory; deltas are applied in committed tick/revision order. A missing revision triggers a new owner baseline, never guessed counts.

## 2. V6 in-tick ordering and RNG ledger

### 2.1 Schedule amendment

**New contract.** Add a V6 policy branch; do not edit V5's observable schedule/RNG. The existing driver is `Z/state_runtime.rs:545–645`, with V5 auto resolution in `Z/state_runtime_auto.rs:176–253`. Tick-local ledgers, output coalescers and dirty sets are scratch; canonical pending action state is retained.

| Phase | V6 amendment to P3 §3 |
|---|---|
| 0 — receive-order commands | Authorize/fingerprint/deduplicate first. Resolve equip, unequip, destroy, enchant, refine, cancel-refine, SA exchange, grants and ToggleAutoShot in command ordinal order. Manual shot UseItem charges here; potion/scroll UseItem stages an item-origin cast. Reconcile equipment/sets/weight/penalties and rebuild immediately after each accepted mutation, before the next command. Enchant/refine RNG occurs here after complete preflight. Learning item costs run in its existing critical command phase. |
| 1 — spawn | Attach complete V6 sidecars, default facing and starter-kit state before publishing identity. |
| 2 — expiry | Expire ordinary effects/reuse/PvP/offers, including item shared-reuse deadlines; equipment System sources have no duration. Recompute derived stats/weight limit/penalties if a provider expires. |
| 3 — periodic + regen | Preserve V5 handler/regen order; overload sources already affect this tick. |
| 4 — chase/movement | Every actual nonzero displacement updates canonical facing; blocked/no-op moves do not. Resolve all geometry from post-step positions. |
| 5 — cast launches/impacts | Existing total order `(deadline,caster ordinal,caster id,cast id)`. Prepaid item cost is not charged again. Use cast shot context across all handlers/targets. Shield/blow/lethal draws occur at the caller sites in §2.2. On finalization clear consumed charge context; next auto-charge attempt is at next eligible start. Rebuild immediately if any consumption/effect changes a condition. |
| 6 — auto impacts | Attacker-ID order, subhit index order within one attack. Revalidate life/incarnation/range/peace/status before resolution; cancelled actions draw no damage RNG. Projectile costs were paid at accepted start. Use shield/position/shots in §5. Dual planning caveat below. |
| 7 — hit bookkeeping | Existing unified landed-damage facts; pole secondary targets each contribute their own actual damage once. |
| 8 — new starts | Stage order remains receive ordinal, then auto-start attacker-ID order. Preflight all costs/targets; attempt auto-charge **before timing calculation**, consume bow arrow/MP at accepted launch, freeze weapon/charge/timing/target bindings; start item-skill reuse and consume its items atomically with MP1. Rebuild weight before a later action reads it. |
| 9 — AI | Existing ordered hate/forced-intention handling. No second movement/start pass. |
| 10 — durable flush/observe | One net consumption row per item per tick, one checkpoint per affected player, ordered operation facts/receipts/outbox. Publish InventoryChanged→EquipmentChanged→WeightChanged→PenaltyChanged→final StatsChanged/VitalsChanged, then operation results, only after log ack/checkpoint. Lifecycle identity/hydration ordering takes precedence. |

Weapon mutation cancels a pending auto swing and clears weapon charges when unequipped (`G/model/itemcontainer/PcInventory.java:961–966`); **new contract**: reject equip/refine/exchange/destruction of the bound weapon while an item-origin or weapon-conditioned cast is active, rather than let a cast exploit changing shot/weapon ownership. Ordinary cast damage continues to read current effective stats at impact as P3 requires; frozen timing and shot context do not rescale. Rejected operations must not allocate IDs, consume RNG/items, or disturb a staged cast. Manual shot charging is itself critical even if no attack follows (AD:17; `H/itemhandlers/SoulShots.java:85–98`).

### 2.2 Exact random caller order

**New contract.** Every draw is a single `next_u32() % bound` in V6, with ordered evidence `(phase, command/action, target, handler ordinal, site, bound, raw_u32, residue)`. Preserve the existing per-zone ChaCha12 stream and digest its position. Rejection, deterministic folding, sets, grade/weight, slot choice and SA exchange use no RNG. All inherited V5 status/death/respawn draws retain P3 §6's caller order after the new formula calls. Do not create an enchant-specific seeded generator (AD:15,22; `Z/state_runtime.rs:52–64`).

| Resolution | Ordered calls and conditional skips |
|---|---|
| Charge soulshot | Optional reduced-soulshot draw bound100 **only** when reduced count >0; select reduced count if residue < reduction chance, then check inventory quantity. No draw for normal counts, already charged, wrong grade or no weapon. `H/itemhandlers/SoulShots.java:61–98`. Spirit/blessed charge draws none. |
| Bow accepted start | Optional reduced-MP draw at source condition, preflight resulting MP/arrow, optional shot reduction draw, then target-limit selection if relevant. Pay one arrow regardless of future hit/miss. `G/model/actor/L2Character.java:701–758,1007–1026`. Insufficient randomized MP cost must retain the attempted draw fact; see Q6 for all-cost preflight consistency. |
| Single auto/pole recipient | hit1000 → on hit shield subroutine → critical1000 → spread(2r+1) unless perfect block or r=0 → recipient status/death hooks → attack-break/reflection at the existing auto caller sites. Miss skips shield/critical/spread. `G/model/actor/L2Character.java:1318–1340`; `Z/state_runtime_auto.rs:104–155`. |
| Shield subroutine | Eligibility/angle/rate guards draw none. Perfect100 first, success iff `100-perfect_rate < residue`; on failure normal100, success iff `residue*Q < rate_q`. Perfect skips normal. `G/model/stats/Formulas.java:1077–1105`. |
| PhysicalAttack handler | Existing physical-skill evasion → shield → optional skill critical → spread unless perfect → recipient hooks → skill reflection (no direct attack-break call in PhysicalAttack). Source handler order is authoritative, not auto's order (`H/effecthandlers/instant/PhysicalAttack.java:49–104`; P3 §6). |
| FatalBlow | physical-skill evasion → on non-evasion blow1000 → on blow success shield → spread unless perfect → optional critical → recipient hooks → attack-break → reflection. `H/effecthandlers/instant/FatalBlow.java:49–102`. Blow uses A8's per-mille adaptation, not an additional normal hit roll. |
| MagicalAttack / range | Insert shield at the source handler's pre-formula site; preserve magic-critical, first/conditional second magic-success, spread, attack-break, reflection and recipient hooks in P3 §6 caller order. Perfect block returns before formula success/spread draws, but does not undo caller draws already made. `H/effecthandlers/instant/MagicalAttack.java:70–110`; `Formulas.java:815–897`. |
| Heal | No shield/hit draw. Existing eligible magic-critical draw only; shot coefficient determined before it (`H/effecthandlers/instant/Heal.java:68–99`). |
| Lethal handler | Guard magic level/eligibility/invulnerability; full1000 then half1000 only if full fails, even if chance is zero. Execute in XML handler order independently of FatalBlow; no invented shared success flag. `H/effecthandlers/instant/Lethal.java:60–96`. |
| Enchant | One enchant100000 after preflight; draw even at zero or certain chance. Never reroll durable retry. `G/model/items/enchant/EnchantScroll.java:142–157`; AD:22. |
| Retail weapon augment | option12(10000) → category(10000) → option34(10000); no skill/glow/base-stat/offset draws (§7). `G/datatables/AugmentationData.java:463–504,517–558`; AD:22. |

Dual source order is **hit1, hit2, then shield1/crit1/spread1, then shield2/crit2/spread2**, with each miss skipping that hit's later draws; integer damage is halved after truncation (`G/model/actor/L2Character.java:1137–1179`). To retain that order and separate subhit timing, V6 creates a canonical dual resolution plan when the first subhit becomes due: capture both hit tests and both formula results in that order, store the second result until its deadline; recipient damage/status/death hooks run at each actual impact, never twice. If the second target incarnation becomes invalid, discard its stored result without refunding consumed planning RNG. This is an explicit V6 scheduling adaptation of source launch-time calculation and requires Q6 review. No V5 helper gains this behavior.

## 3. Equip/unequip, installed functions and sets

### 3.1 Slots and compatibility

Paperdoll uses the following source slot IDs internally; wire enum zero remains UNSPECIFIED, so wire values are internal ID+1. A two-hand weapon occupies **one stored RHAND entry**; onepiece occupies **one CHEST entry**, with the other slot excluded by validation. This prevents duplicate functions, weight and grade scans (`G/model/itemcontainer/Inventory.java:54–79,555–605`).

| Internal IDs / slots | Template bodypart and replacement rule |
|---|---|
| 0 UNDER, 1 HEAD | underwear/shirt, head respectively; replace same slot. |
| 2 HAIR, 3 HAIR2 | hair/hair2; hairall occupies HAIR and clears HAIR2; equipping either single removes conflicting hairall (`Inventory.java:609–630`). |
| 4 NECK | neck. |
| 5 RHAND, 7 LHAND | rhand replaces RHAND. lrhand clears LHAND then replaces RHAND; bow auto-equips matching arrows, crossbow bolts. Equipping lhand shield/sigil removes conflicting lrhand weapon; arrow+bow / bolt+crossbow exceptions preserve it. `Inventory.java:556–571`; `PcInventory.java:895–938`. |
| 6 CHEST, 11 LEGS | chest/legs individually; fullarmor/onepiece clears LEGS and occupies CHEST. Legs removes fullarmor. `Inventory.java:592–605`. |
| 8 REAR, 9 LEAR | source chooses empty LEAR first, then REAR, otherwise replaces LEAR, including combined ear masks. |
| 10 GLOVES, 12 FEET | gloves/feet. |
| 13 RFINGER, 14 LFINGER | empty LFINGER, then RFINGER, otherwise replace LFINGER, including combined finger masks. `Inventory.java:573–588`. |
| 15 LBRACELET, 16 RBRACELET | Corresponding bodypart; summon/agathion contexts still subject to executable capability manifest. |
| 17–22 DECO1–DECO6 | Source talisman slots; catalogued, capability-gated until their admitted funcs/slot conditions execute fully. |
| 23 CLOAK, 24 BELT | back/belt. `Inventory.java:631–636`. |

`EquipItem` takes an instance, with optional explicit compatible paired slot for UI drag/drop. Omitted slot uses the source preference above; explicit slot is a **new UI contract** and cannot bypass masks. Do not derive masks by matching display names. Formalwear behavior must be modeled for admitted ALLDRESS: blocks conflicting equips except the source bouquet exception, and clears conflicting slots when equipped (`Inventory.java:539–552,637–645`). Unsupported special templates remain catalogued with blockers, not treated as ordinary chest armour.

Before mutation: owner/session/incarnation, positive count, allowed location, supported item, alive/not disabled, valid slot, explicit template conditions, and pending-action guard (§2). **No blanket class/race/level requirement is inferred from grade.** L2Item returns true without attached conditions; otherwise evaluates every condition (`G/model/items/L2Item.java:655–702`). Import and execute class/race/sex/level/using/slot conditions where present; unsupported world modes fail closed. Grade mismatch equips successfully and creates a penalty. No GM bypass via client flags. Equip and implicit unequips are one atomic mutation; no transient externally visible half-equipment state (AD:17,20,26).

### 3.2 Fold installation and reconstruction

Retain `FunctionKey(stat, priority, source_install_sequence, handler_ordinal, function_ordinal)` and duplicate-key validation (`Z/stat_functions.rs:14–22,161–202`). Each equipped instance gets one fresh shared sequence when it is newly equipped; its funcs retain XML ordinals. SET and ENCHANT are priority0 in their source order, before priority1 class formulas and before ordinary passive/effect priorities. **Do not split all SETs and all ENCHANTs across separate global passes** or sort funcs by their names: an item SET then its ENCHANT must stay ordered. A replacement removes every old provider before installing the new one. In-place enchant refreshes the value under its existing sequence; removal/re-equip allocates a new sequence (AD:20; `Z/skill_profiles.rs:230–245`).

`EquipmentSourceKey` distinguishes `Item(instance)`, `ItemSkill(instance,xml_ordinal)`, `Enchant4(instance)`, `EnchantOption(instance,option)`, `Augment(instance,half)`, `ArmorSet(chest,set,kind,ordinal)`, `GradePenalty(kind)`, `Overload`, and `Polearm`. Source key identifies provider ownership even where multiple providers supply the same skill. Skill resolution uses one winning rank per SkillId (highest rank, deterministic provider-key tie); removing one provider reveals remaining providers without deleting a learned skill. Provider metadata is checkpointed in shared relative install order; no duplicate passive application of the same skill (new contract extending `Z/phase3_checkpoint.rs:54–85`).

Direct set STR/DEX/CON/INT/MEN/WIT deltas are priority1 **base-stat** inputs, before bonus lookup; they are not additive final P.Atk/HP bonuses (`G/model/stats/functions/formulas/FuncArmorSet.java:46–67`). P-type equipment/penalty skills use the Phase3 System pool and stayAfterDeath; passive sources do not spend timed buff slots. Active augmentation/item skills are granted capabilities, not forcibly converted to passive skills; use their actual operate type and cast pipeline. Augmentation triggers require complete effects support before admission (§7).

Dirty rebuild order: equipment and set membership → effective base attributes → class/weapon/item functions and passives → weightLimit/weightPenalty → scalar penalties/System providers → final sheet → clamp HP/MP/CP to maxima, clear fractions when clamped. Rebuild before any subsequent read; never run one-time onStart hooks or heal on equip. Preserve current resources when maxima rise. Detect dependency cycles at rules validation (e.g. penalty providing its own weightLimit); do not run an unbounded fixed-point loop. Existing V5 clamp/cache integration is `Z/state_runtime.rs:187–255`; base assembly is `Z/effective_stats.rs:69–138`; System fold lifecycle is P3:136–164.

### 3.3 Defence subtraction and enchant functions

At priority1, player P.Def is `(sum of base+item priority0 terms − occupied class slot bases) × LM`, `LM=(level+89)/100`. Subtract CHEST, LEGS (also if CHEST is fullarmor), HEAD, FEET, GLOVES, UNDER, CLOAK. **Do not subtract shield or BELT** (`G/model/stats/functions/formulas/FuncPDefMod.java:44–70`; `G/model/actor/L2Character.java:5005–5008`). M.Def subtracts occupied LFINGER, RFINGER, LEAR, REAR, NECK, then multiplies MEN bonus and LM (`FuncMDefMod.java:44–66`). Both LFINGER and RFINGER class bases deliberately use `baseMDefrfinger`; do not repair that pinned template quirk (`G/model/actor/templates/L2PcTemplate.java:71–75`). Transformation-specific nested slot bugs are not generalized into ordinary players; transforms remain an explicit unsupported context.

For enchant level `e`, let `a=min(e,3)`, `b=max(e−3,0)`. An actual ENCHANT function adds `unit×(a+2b)` for attack, and `a+3b` for P.Def/M.Def. Do not add enchant to templates that have no such func (`G/model/stats/functions/FuncEnchant.java:43–80,85–155`).

| Stat/weapon row | NONE/D unit | C/B unit | A unit | S/S80/S84 unit |
|---|---:|---:|---:|---:|
| M.Atk, all weapons | 2 | 3 | 3 | 4 |
| P.Atk rhand/non-lrhand | 2 | 3 | 4 | 5 |
| P.Atk lrhand non-bow, including pole/dual | 2 | 4 | 5 | 6 |
| P.Atk bow/crossbow | 4 | 6 | 8 | 10 |

Source: `G/model/stats/functions/FuncEnchant.java:77–151`. S-plus collapse is for enchant coefficients/shot compatibility, **not** expertise grade IDs. No Olympiad limit runs in Phase4 (`FuncEnchant.java:55–63`; AD:8,12).

ENCHANTHP is priority40, and is implicitly attached by the loader to graded armour in CHEST, FEET, GLOVES, HEAD, LEGS, BACK, FULL_ARMOR, UNDERWEAR, LHAND and BELT. It is not inferred by looking only for `<enchanthp>` XML (`G/data/xml/impl/EnchantItemHPBonusData.java:83–105`; `G/model/stats/functions/FuncEnchantHp.java:37–44`). Bonus index is `min(e,table_len)−1`, zero when e=0 or no grade row, onepiece bonus truncates `table_value×3/2`; `EnchantItemHPBonusData.java:48,116–126`. Read values from `D/stats/enchantHPBonus.xml`, never extrapolate beyond last row.

### 3.4 Set detection

Select set by equipped CHEST template, require every nonempty allowed-ID list for legs/head/gloves/feet; an empty list is optional. Fullarmor naturally uses the set's absent legs requirement, not a forged second item. Shield bonus uses its separate list; enchant6 checks required pieces only, excluding optional pieces and shield (`G/model/L2ArmorSet.java:128–170,192–196,215–248`; `G/model/itemcontainer/PcInventory.java:1143–1208`). Install exact full-set, shield and enchant6 skill lists plus direct base-stat deltas. `enchant4_skill` is an item property (including weapons), not “every full set gives a +4 skill”; install only when equipped e≥4 and property exists (`PcInventory.java:969–976,1067–1080`; `G/model/items/L2Weapon.java:108–123`). Reconcile after equip, unequip, enchant success/reset/destruction, load, level/class and any condition-changing effect. Q11 records the pinned shield-listener history quirk rather than silently claiming that a stateless set test reproduces it.

## 4. Item enchanting

### 4.1 Validation, groups and scroll semantics

`EnchantItem` names target, scroll and optional support instance. Complete preflight validates ownership, locations, distinct IDs, count, enchantability/blacklist, weapon versus armour/accessory kind, S-plus target grade, allowed target ID list and scroll maximum; unsupported group is an error before spending. `maxEnchant` defaults to 65535, zero disables the limit, and validation rejects **current e > max**, not e≥max (`G/model/items/enchant/AbstractEnchantItem.java:52–61,109–130`). Hence max16 admits an attempt at current +16. `EnchantScroll` derives blessed/safe/type from EtcItemType, not names; blessed+support is invalid; support must match target/kind (`G/model/items/enchant/EnchantScroll.java:49–56,102–114`). Import live XML elements only: the old crystal scrolls in `D/enchantItemData.xml:32–42` are comments.

Default group0 chooses ARMOR for listed normal armour/jewelry/shield/sigil/belt/shirt slots, FULL_ARMOR for fullarmor, and fighter/mage by `is_magic_weapon` for rhand/lrhand (`D/enchantItemGroups.xml:63–90`). Safe “+3/+4” means the **attempt from** current0–2/current0–3 is guaranteed, not a separate hardcoded bypass that skips RNG.

| Group | Current enchant → base chance in units of 1/100000 |
|---|---|
| Fighter weapon | 0–2:100000; 3–14:70000; 15–65535:35000 |
| Mage weapon | 0–2:100000; 3–14:40000; 15–65535:20000 |
| Armour | 0–2:100000; 3:66670; 4:33340; 5:25000; 6:20000; 7:16670; 8:14290; 9:12500; 10:11120; 11:10000; 12:9100; 13:8340; 14:7700; 15:7150; 16:6670; 17:6250; 18:5890; 19:5560; 20–65535:0 |
| Full armour | 0–3:100000; 4–19 use ordinary armour's row at e−1; 20–65535:0 |

Source: `D/enchantItemGroups.xml:4–59`. Full armour +19 is **5890**, not 5560: the shifted schedule terminates at +20. Weapon “uncapped” means no low gameplay cap; the finite source table/default maximum still matters. A signed SQL smallint cannot represent the source maximum; Q8 requires an integer column.

Normal scrolls: weapon A/B/C/D/S =729/947/951/955/959; armour =730/948/952/956/960. Blessed weapon =6569/6571/6573/6575/6577, armour =6570/6572/6574/6576/6578 (`D/enchantItemData.xml:4–28`). Preserve each later row's bonusRate, maxEnchant and allowed-items semantics; ancient entries add 10 percentage points with max16, divine entries add100 (`:45–73`). Do not interpret a bonus as a multiplier or safe success as blessed protection.

### 4.2 Integer roll and proof

Let exact source percentage be `p`, scroll bonus `b`, support bonus `s`; `c=min(100,p+b+s)×1000` is an integer for admitted data. Require integral `c` during generation; never round an unknown precision silently. Draw `r=next_u32()%100000`, success iff `r<c` (AD:22; `G/model/items/enchant/EnchantScroll.java:147–157`). For the source comparison on the **same quantized sample** `u=r/100000`, `100u < p+b+s` iff `r<c`. Exactly residues 0 through c−1 succeed for `0≤c≤100000`; equality fails. This proves equal success sets on the stated lattice, **not identical distributions to an unrestricted real-valued draw**. Modulo bias is explicitly retained by A8: `2^32 =42949×100000+67296`, so the first 67296 residues have one extra preimage. Oracle tests exhaust the residue success set and the boundary; no false claim of perfect uniformity.

After a successful draw, increase e only when **base group chance >0**. Bonus-only success with base zero consumes the scroll but retains e (`G/network/clientpackets/RequestEnchantItem.java:163–168`). All accepted attempts consume one scroll and support if supplied; use one critical transaction for input consumption/outcome/refund/equipment changes, not source-style partial SQL operations (`:122–154`; AD:17).

### 4.3 Failure, refunds and visibility

Safe failure: retain target/equipment/e. Blessed failure: unequip target if equipped, clear its providers/charges, reset e=0 and leave it in inventory. Normal failure: unequip, destroy target, refund crystals only if crystal ID exists and target is crystallizable (`G/network/clientpackets/RequestEnchantItem.java:199–271`). No auto-re-equip after blessed failure.

Let B be template crystal_count, g the grade coefficient, e pre-attempt enchant. Crystal count C(e) is B for e=0; B+g×e for e≤3; B+g×(2e−3) for weapons above3; B+g×(3e−6) for armour/accessories above3. Refund `max(1,C(e)−floor((B+1)/2))`. Coefficients `(armour,weapon)` D=(11,90), C=(6,45), B=(11,67), A=(20,145), S/S80/S84=(25,250); crystal IDs1458–1462 (`G/model/items/L2Item.java:405–420`; `G/model/items/type/CrystalType.java:26–33`; `RequestEnchantItem.java:259–269`). Use checked wide arithmetic; preflight refund capacity for every possible branch before drawing; freeing destroyed target permits a new crystal stack. Existing crystal stack keeps its ID. Do not include the attempted new enchant in refund arithmetic.

Success announcements occur exactly at resulting armour+6 or weapon+7/+15, not on every subsequent enchant (`RequestEnchantItem.java:174–187`). Owner EnchantResult includes outcome, before/after e, destroyed flag and refund instance/template/count; replay evidence additionally includes base/bonus/support/final chance and raw draw. Public equipment only exposes the approved glow tier, never exact levels via a generally broadcast owner result (AD:16). Source fireworks/messages are presentation evidence for milestones, not a mandate for a new stream.
