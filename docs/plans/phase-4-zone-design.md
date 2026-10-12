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

Weapon mutation cancels a pending auto swing and clears weapon charges when unequipped (`G/model/itemcontainer/PcInventory.java:961–966`); **new contract**: reject equip/refine/exchange/destruction of the bound weapon while an item-origin or weapon-conditioned cast is active, rather than let a cast exploit changing shot/weapon ownership. Ordinary cast damage continues to read current effective stats at impact as P3 requires; frozen timing and shot context do not rescale. Deterministically rejected operations must not allocate IDs, consume RNG/items, or disturb a staged cast; source randomized cost checks retain their draw even on a later resource rejection (§2.2). Manual shot charging is itself critical even if no attack follows (AD:17; `H/itemhandlers/SoulShots.java:85–98`).

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

ENCHANTHP is priority40, and is implicitly attached by the loader to graded armour in CHEST, FEET, GLOVES, HEAD, LEGS, BACK, FULL_ARMOR, UNDERWEAR, LHAND and BELT. It is not inferred by looking only for `<enchanthp>` XML (`G/data/xml/impl/EnchantItemHPBonusData.java:83–105`; `G/model/stats/functions/FuncEnchantHp.java:37–44`). Bonus index is `min(e,table_len)−1`, zero when e=0 or no grade row, onepiece bonus truncates `table_value×3/2`; `EnchantItemHPBonusData.java:50,116–126`. Read values from `D/stats/enchantHPBonus.xml`, never extrapolate beyond last row.

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

## 5. Shots, projectiles, weapon kinds, shield and position

### 5.1 Charge lifecycle

Charging consumes inventory immediately and stores charge bits on the active weapon; firing consumes the **charge**, not a second inventory payment. Physical count is template soulshots, magical count spiritshots; matching is `getItemGradeSPlus` so S80/S84 use S shots (`H/itemhandlers/SoulShots.java:61–98`, `SpiritShot.java:60–92`, `BlessedSpiritShot.java:63–95`). Fists/no equipped weapon cannot charge. A already-charged same kind is a no-op/rejected use with no spend. Blessed can be charged over ordinary spiritshot; formula precedence is blessed first, not multiplication of both (`BlessedSpiritShot.java:71–95`; `G/model/stats/Formulas.java:843`).

Toggle stores a template ID, not a particular stack ID. Iterate toggles in template-ID order and choose the lowest matching available instance; this fixes source unordered iteration deterministically. Enabling attempts immediate charge as pinned; later accepted start attempts recharge before cast timing. Source enables the toggle **even on grade mismatch**, reports mismatch, and attempts recharge (`G/network/clientpackets/RequestAutoSoulShot.java:105–124`); Q7 asks whether A9 intended to reject toggling or only reject charging. Insufficient stock disables that auto-shot template (`H/itemhandlers/SoulShots.java:91–95`, `SpiritShot.java:84–88`). Unequipping clears all weapon charge bits, without refund (`G/model/itemcontainer/PcInventory.java:961–966`).

Source auto attack clears soulshot only if at least one hit lands; a wholly missed dual/pole attack retains it (`G/model/actor/L2Character.java:889,953–958`). **Proposed V6 contract pending Q7:** one prepaid charge per attack bundle, shared across both dual halves/all pole recipients; discharge on first landed bundle hit, retain on all misses. A cast binds its charge context once and uses it for all eligible handlers/targets; it cannot consume one full shot pack per AoE target. On completion discharge the types requested by the skill; cancel before use leaves weapon charge intact, subject to unequip clearing. AD's “per hit” wording must be reconciled with these source bundle semantics before engine freeze. Record paid inventory deltas separately from discharged charge bits so replay can prove no double charge. All charge/auto-toggle changes are durable even when count delta=0 (AD:17,23).

### 5.2 Formula deltas from P3 §6.1

Use `ExactRatio`/`RootExpression` over checked i128, with Q=1,000,000 for stored stats and modifiers; the existing exact-root boundary is `Z/skill_math.rs` and `Z/stat_functions.rs:55–94`. In expressions below, symbols are **stat getter values expressed as exact ratios**, not raw Q integers. `P`, `M`, `D` mean P.Atk, M.Atk and appropriate defence. `S` is soulshot factor, `X` spread, `T` source trait product, `A` attribute multiplier (still neutral for absent weapon attributes), `V` PvP multiplier, `N` source NPC penalty. Do not combine multiplications across explicit source casts. All unchanged condition/attribute/penalty branches retain P3 §6.1.

Position damage multiplier `B=1(front),11/10(side),6/5(back)`; source labels these values unconfirmed/custom, but they are the pinned behavior (`G/model/stats/Formulas.java:516,682,761`). Hit-condition percentage is a **different** term from this damage multiplier.

| Path | Exact V6 expression / evaluation shape |
|---|---|
| Ordinary auto | `D'=D+sDef` on normal block, `D'=D` otherwise; apply PvP defence modifier to D'. `P'=P×(charged?2:1)`. Noncrit base `76×P'×B/D'`; crit base `2×critDmg×critDmgPos×defCritDmg×(76×P'×B/D') + critDmgAdd×77/D' + defCritDmgAdd`. Then trait→spread→PvP damage→attribute→NPC penalty; max(result,1), truncate at caller. `Formulas.java:667–733`. |
| PhysicalAttack | `77×(power+P×S)/D'`, then B→PvP damage→attack trait→attribute→spread→NPC penalties→PHYSICAL_SKILL_POWER; min1. Handler passes `crit=false` into formula and doubles afterwards if its separate STR-based skill-crit succeeds; retain that quirk. `Formulas.java:746–808`; `H/effecthandlers/instant/PhysicalAttack.java:86–104`. |
| FatalBlow | `S=1458/1000` when charged, else1. `base=77×(power+P×S)/D'`; `posCrit=(critDmgPos−1)/2+1`; `add=critDmgAdd×61/10×77/D'`. Result `(base×critDmg×posCrit×defCritDmg×B×V + add + defCritDmgAdd) × weaponTrait × generalTrait × A × X × N`. Optional handler critical doubles afterwards. `Formulas.java:502–555`; `FatalBlow.java:74–87`. |
| MagicalAttack | Add shield defence to M.Def (or `sDef×shieldDefensePercentage/100` for the range handler's positive configured percentage); apply PvP defence. `91×sqrt(M×(blessed?4:spirit?2:1))×power/MDef'`; keep existing magic-failure/critical/spread/PvP/attribute/NPC branches. `Formulas.java:815–897`. |
| Perfect shield | Formula returns1 before its internal spread/success branches; caller critical, dual halve or bow distance cast can still alter this return. Never globally clamp all final shielded damage to1. `Formulas.java:505–511,676–678,755–757,825–827`; `L2Character.java:1033,1158–1172`. |
| Magic cast time | Existing speed scaling then ×3/5 if charged spirit/blessed, then source500ms floor if original hitTime>500, then P3 launch/impact/interrupt staging. Shot state must be read before freezing timing. `Formulas.java:1027–1045`; P3:122–134. |
| Heal | Mage player with spirit/blessed: coefficient2/4 and static bonus `mpConsume2×(1 or12/5)`; charged NPC: coefficient4 and bonus `mpConsume2×12/5`; summon branch remains deferred. Otherwise weapon grade coefficient S84=4,S80=2,other/none=1, then `bss?coefficient×4:coefficient+1`. Nonstatic amount=`power+staticBonus+sqrt(coefficient×M)` then HEAL_EFFECT then eligible magic critical×3; static ignores this addition/crit. Recoverable HP clamp and Q remainder unchanged. `H/effecthandlers/instant/Heal.java:68–104`. |

Blow success rate starts `blowChance×DEXbonus×position(1,3/2,2)`, then BLOW_RATE funcs; compare `roll1000×Q < rate_q×10`. Source uses integer100; A8 explicitly changes resolution to per-mille. This preserves inequalities on shared samples but introduces intermediate tenth-percent residues; oracle must document that adaptation, not claim byte-exact source RNG (`Formulas.java:1621–1640`; AD:22). Lethal eligibility requires `skill.magicLvl ≥ target.level−6`, lethalable and not invulnerable; full/half chances multiply attribute and general trait. Full sets player HP/CP=1, monster HP=1; half sets player CP=1 or halves monster current HP; never causes a death award. Apply full then half draw with the same per-mille adaptation (`H/effecthandlers/instant/Lethal.java:60–96`).

### 5.3 Shield and deterministic geometry

Eligible secondary item must be L2Armor and not SIGIL; rate=`fold(rShld,0)×DEXbonus`. Rate≤1/Q returns no block/no draws. Angle=`trunc(fold(shieldDefAngle,0))+120` degrees; angle≥360 skips facing guard. Bow attacks multiply rate by13/10. Perfect test runs **before** ordinary rate test: `100−10 < roll100`, using pinned perfect-rate10, hence residues91–99 succeed (**nine**, not ten outcomes). Only on perfect failure roll normal shield. (`G/model/stats/Formulas.java:1077–1105`; `GC:136–140`.) No sDef subtraction from final damage; source ShieldBlocks alternative is inactive.

**New geometry contract under A10.** Store a primitive integer direction vector for every combatant, last nonzero actual movement delta divided by gcd; initial/teleport direction is east `(1,0)` until movement, and teleport preserves existing facing unless creating a new life. Facing is not changed by client aim or target selection. This differs from L2J's turn-to-target on attack/cast (`G/model/actor/L2Character.java:890,1554–1557`) as A10 explicitly requires last-movement facing. Include this policy in Phase4Rules hash.

Classify actual target-relative position, never the mutable selected target of an AoE caster. Given victim facing f and victim→attacker vector v: front if dot≥0 and `4×dot²≥|f|²×|v|²`; back if dot<0 and same squared test; otherwise side. Equality belongs to front/back (source half-angle60°; `L2Character.java:4930–4993`). Coincident positions count front and satisfy facing (new zero-vector policy). Shield/FAN use cone half-angle comparisons: exact rational squared comparisons for supported angles, checked integer CORDIC-generated rational bounds for other imported integer angles, with explicit inclusive boundary vectors and oracle certification; no runtime trig or binary floating point. **QUESTION Q12:** architect should freeze supported angle representation/precision before data/code gates; do not approximate unknown angles by 8-way sectors.

For hit chance, apply `trunc((80+2×(accuracy−evasion))×10×condition_bonus)` then clamp200..980; hit iff residue≤chance (source miss uses strict `<`). `condition_bonus=max((100+front/side/back+height+dark)/100,0)` using existing hit_condition_bonus table. Flat test_zone has no height modifier, and no invented day/night wall clock (`G/model/stats/Formulas.java:1054–1063`; `packages/data/tables-phase3/hit_condition_bonus.toml:3–10,12–58`).

### 5.4 Bow, dual, fist and pole

Bow start requires compatible lhand arrow (auto-select lowest matching ID if empty), reuse ready and enough MP after BOW_MP_CONSUME_RATE. Pay MP once, consume one arrow even if the shot misses; no-arrow start does not charge/fire. Grade comparison collapses S-plus (`G/model/itemcontainer/Inventory.java:679–713`; `G/model/actor/L2Character.java:711–758,1013–1016`). Use weapon `mp_consume` and reuse_delay, not a grade-derived guessed cost. Reach=`floor(range_L2×1000/32)` milli-tiles (`Z/combat.rs:286–296`), so source500 reach is15,625 milli-tiles. Timing: `attack_ms=trunc(500000/pAtkSpd_getter)`, `reuse_ms=trunc(template.reuse_delay×333/pAtkSpd_getter)`, bow impact at start+ceil(attack_ms/100), next eligible start at start+ceil((attack_ms+reuse_ms)/100). Thus “1500ms bow reuse” is a **base template value**, not fixed15 ticks (`G/model/actor/L2Character.java:1007–1045,4335–4357`).

Bow damage additionally truncates ordinary damage to whole first, then truncates multiplication by `(distance_L2/4000+4/5)` (`L2Character.java:1029–1033`). Compute Euclidean distance with the existing exact-root machinery and units conversion, not Manhattan or squared distance. Crossbow uses its own source branch/template; do not apply bow distance falloff to it without source support. Arrows exhausted to zero clear lhand before weight/penalty rebuild and emit one removal.

Player FIST falls through to DUAL/DUALFIST/DUALDAGGER handling; NPC FIST uses simple hit (`L2Character.java:923–935`). Dual halves are each `trunc(formulaDamage)/2` with integer division; deadlines correspond to half of `timeToHit` and full `timeToHit`, where `timeToHit=attack_ms/2` (`:887,1158–1179`). Preserve second-hit canonical plan and separate public outcomes. An unequipped player uses class fist bases from `Z/effective_stats.rs:105–116`, not the starter sword profile and not invented universal fist stats.

Pole attacks primary at100%, then eligible secondaries starting85%, dividing successive percentage by23/20. Range and angle are effective physical attack range/angle; scan secondaries in entity-ordinal then EntityId order (new deterministic replacement for source map iteration), excluding primary, wrong hostility/peace/dead, outside radius/cone. Skill3599 contributes attack-count modifiers; **source increments secondary count and breaks when count > max**, not ≥, so do not equate its comment “three targets” to the executed bound (`G/model/actor/L2Character.java:1202–1206,1237–1287`; `D/stats/skills/03500-03599.xml:1695–1708`). Q11 records this quirk. FAN skill target collection uses the same geometry and ordinary skill affect-limit semantics (C1), but is not a replacement for the separate pole auto-attack count algorithm.

### 5.5 Rounding ledger

| Boundary | Required operation |
|---|---|
| Imported decimal | Exact base10 numerator/denominator; reject unrepresentable fixed-scale chances; no host numeric parser through a binary float. |
| Stat funcs | Existing Q conversion/priority ordering; checked i128 multiply before division, explicit Java truncation toward zero where source casts. SET/ENCHANT additions happen before STR/INT/LM (§3). |
| Crit/evasion/shield getter | Preserve getter cast/cap before later formula factors; rShld comparison retains Q fractional rate. |
| Enchant chance | Exact percent×1000, clamp upper100000 only after bonuses; strict residue<c. |
| Defence | Slot subtraction and normal shield addition occur before defence multipliers/division. |
| Roots | Carry `RootExpression`/exact bounds through final comparison or rounding; do not floor sqrt(M) before applying power. |
| Damage | Preserve formula minimum and caller truncation separately; bow applies a second truncation; dual truncates then integer-halves. |
| Time | Source truncates milliseconds first; convert absolute nonnegative offsets by integer ceil-to100ms, retaining P3 minimum one-tick cast completion. |
| Weight | Truncate signed `(weight−bonus)×1000/maxLoad` toward zero, not floor for negative values. MaxLoad floors pre-fold CON product, then truncates post-fold getter. |
| Restore | Whole resource credit plus Q remainder under existing P3 rules; clamp-no-heal does not manufacture fractional credit. |

Each oracle vector records intermediate terms and casts, not only final damage (P3:447–461; AD:15,21–22).

## 6. Weight, overload and expertise penalties

`weight=sum(template.weight×count)` over all inventory including equipped entries once; weight is a source unit, not grams (`G/model/items/L2Item.java:162`; `G/model/itemcontainer/Inventory.java:670–675`). `maxLoad=trunc(fold(weightLimit,floor(CONbonus×69000×WeightLimit)))`; pinned WeightLimit=1. `bonus=trunc(fold(weightPenalty,1))`, deliberately **one**, not zero (`G/model/actor/instance/L2PcInstance.java:9427–9438`; `GC:27`). No31000/176000 clamp from older chronicles.

For maxLoad>0, `per_mille=trunc((weight−bonus)×1000/maxLoad)`. Penalty levels: <500→0, <666→1, <800→2, <1000→3, else4. All four nonzero ranks halve regHp/regMp; movement multipliers are1,1/2,1/2,0. Weight zero still includes the bonus subtraction; negative per_mille is valid and maps0 (`L2PcInstance.java:1668–1693`; `D/stats/skills/04200-04299.xml:1623–1639`). Level4 and overloaded are distinct: source overload condition is `weight > maxLoad`, strictly greater; at exactly maxLoad it is false. **QUESTION Q9:** source updates that bool only when penalty rank changes, so crossing within rank4 can retain stale overload state. Recommended A12 interpretation recomputes bool on every change while preserving strict inequality; mark this explicit correction if approved, otherwise retain previous bool and source transition guard. MaxLoad≤0 is also source no-op, not division by zero.

Recompute after grant/consume/destroy/refund/exchange, CON/weightLimit/weightPenalty changes, level/class and restore. Equip does not change total weight but can change capacity/CON/maxLoad. Movement must tolerate effective speed zero without violating existing positive-Speed constructors; use movement-disabled state for overload rank4 and preserve base speed, rather than inventing speed1. `can_accept` independently checks slot and weight budgets for ordinary acquisition (`G/model/itemcontainer/PcInventory.java:772–785`); fixture overload states can be reached by seeded load or removing a capacity/CON bonus, not a bypass in production acquisition.

Expertise=`max(learned239 rank,0)`, with source auto-get levels20/40/52/61/76/80/84 for D/C/B/A/S/S80/S84 (`G/model/actor/instance/L2PcInstance.java:8095–8104`; `D/skillTrees/classSkillTree.xml:8–14`). Do not collapse S80/S84 to5 for penalties. Default expertise penalty bonus is zero unless an admitted explicit provider supplies it; no invented “expertise bonus” by level (`L2PcInstance.java:1650–1655,1727–1731`).

Canonical scan is ascending ItemInstanceId, each equipped instance exactly once; skip arrows/bolts. Start weaponGrade=armorGrade=0. For grade>expertise: if weapon **and grade>weaponGrade**, set weaponGrade; **else if** grade>armorGrade set armorGrade. This preserves the weapon-to-armour branch quirk, even though valid one-RHAND states normally cannot expose a second equipped weapon. Each final rank=`clamp(grade−expertise−bonus,0,4)` (`L2PcInstance.java:1702–1753`). Recompute on equip/unequip, level/learn/class and rollback-free restoration.

System6209 rank1–4: accuracy−16; critRate and critDmg×9/10,8/10,7/10,6/10; P.AtkSpd/P.Atk×9/10 at every rank. System6213: evasion−5/2,5,15/2,10; P.AtkSpd/M.AtkSpd/runSpd×8333/10000,6944/10000,5787/10000,4823/10000 (`D/stats/skills/06200-06299.xml:324–359`). Install only nonzero ranks, replace rank under source identity, remove at zero; both plus4270 have P/stayAfterDeath. They are effect-engine inputs, not post-hoc multiplication of a serialized sheet (AD:20,26).

## 7. Augmentation, special abilities and soul crystals

### 7.1 Validation and costs

**New fixture contract under A13:** add noncombat `blacksmith` at (128,128), radius3 tiles, within the existing test-zone town; validate current same-zone NPC identity, position, life and range at command resolution. Coordinates are Nightfall fixture policy, not L2J map coordinates (AD:27; P3:126). Refine/Cancel/SA all use it; no silent “any NPC” acceptance.

Refine validates target ownership/location, not already augmented, not hero/shadow/common/timed/PvP-disallowed/blacklisted, grade≥C, weapon except NONE/FISHINGROD or jewelry in combined finger/ear/neck slots; stone category must match weapon/accessory and player level must reach the stone level (`G/network/clientpackets/AbstractRefinePacket.java:235–337`). Dead, stunned, sitting, transaction/store/trade/fishing/cursed states fail where supported (`:345–373`). Stone mapping includes levels46,49,52,55,58,61,64,67,70,76,80,82,84,85; import IDs rather than computing from an assumed contiguous range (`:51–65,88–188`).

| Target grade | Weapon gem cost | Accessory gem cost |
|---|---|---|
| C | 20 ×2130 (D gem) | 200 ×2130 |
| B | 30 ×2130 | 300 ×2130 |
| A | 20 ×2131 (C gem) | 200 ×2131 |
| S | 25 ×2131 | 250 ×2131 |
| S80 | 36 ×2132 (B gem) | 360 ×2132 |
| S84 | 36 ×2132 | 480 ×2132 |

Source: `AbstractRefinePacket.java:43–45,380–414`. One stone is consumed; no extra “life stone purchase price” is charged by RequestRefine. Validate all inputs, unequip target, consume stone/gems, generate options, retain same target identity in inventory, commit atomically (`G/network/clientpackets/RequestRefine.java:79–113`). There is no success/failure draw after valid inputs; invalid input has no outcome roll.

CancelRefine fee depends on **instance crystal count including enchant**: C below1720→95000, below2452→150000, else210000; B below1746→240000 else270000; A below2160→330000, below2824→390000 else420000; S480000; S80/S84920000. Consume Adena57, disarm equipped weapon, remove augmentation with no refund (`G/network/clientpackets/RequestRefineCancel.java:57–118`). Fees are data, not values inferred from grade sale price. AD:21 closure must include currency57; Q4 flags this omission.

### 7.2 Retail draws, packing and effect application

Pinned config enables retail weapon and accessory augmentation (`GC:393,405`). Weapon routine filters `retailchances.xml` by mage/warrior and exact stone ID, preserving XML order. Draw option12 residue0..9999; start threshold10000 and visit every row: while previous threshold>residue, subtract row chance×100 and replace selected option. This is a descending-tail selection; a conventional ascending CDF with the same draw reverses the outcomes. Draw category using A8 weights5500/3500/700/300; map to category labels55/35/7/3; filter variation2 rows to that label, then draw/select option34 with the same descending algorithm (`G/datatables/AugmentationData.java:447–558`). Keep table weights as exact ratios if chance×100 is fractional; compare rational thresholds to the integer residue. Validate no reachable residue leaves option0, rather than invent rounding/rebalancing of published weights.

Pack as unsigned32 `(stat34<<16)|stat12`, with both halves bounded to16bits and nonzero valid option IDs. PostgreSQL signed int cannot represent every u32 packed value; use bigint with u32 bounds or store two constrained ints (Q8; `RequestRefine.java:108–113`; `AugmentationData.java:558`). Accessory retail uses its own rows/algorithm; do not draw a weapon category absent in that source branch (`AugmentationData.java:689–740`). Oracle must enumerate every10000 residue per choice and preserve list order.

The old non-retail algorithm is **not combined** with retail: it clamps stone index to9, draws skill/glow, conditional1% base-stat, color and option indices; stat subblock91 and block3640 produce `stoneIndex×91+colorOffset`. Normal stat34 uses `color×910+temp×3640+1`, temp∈{2,3}, then offset..offset+90; stat12 chooses a corresponding permitted block, and packing is identical (`AugmentationData.java:563–686`). The15/30/45/60 skill chances and0/40/70/100 glows in GC:371–389 are inactive in the retail weapon branch. Import for provenance if A7 requests them; do not consume their draws in V6 retail mode. Q4 resolves the contradictory “retail plus offsets” brief.

Decode option12/34 through `D/stats/options`; install functions and passive skills through the effective-stat/System source engine, grant active skills, register trigger providers in source XML order. `G/model/options/Options.java:61–74,136–175` explicitly distinguishes those three forms. Unequip/cancel removes only that provider and clamps without onStart replay. A skill with an unsupported trigger/handler blocks its source recipe/augmentation table from execution; do not silently discard the rolled skill or reroll into supported outcomes. **QUESTION Q4:** closure may require abilities beyond Phase3 Tier B; architect must assign their implementation or explicitly adjust admitted augmentation scope, with probabilities unchanged. This is a release gate for O1.

### 7.3 SA recipes and absorb hook

ExchangeSpecialAbility names `(npc,base_instance,recipe_id)`. Recipe ID is a stable generated pair `(multisell list ID,item XML ordinal)`; all inputs/counts/products/maintain flags come from the pinned row. Validate exact base template and all ingredients together, unequip base, consume ingredients, create product with fresh ID; if maintainEnchantment, carry source enchant and augmentation as pinned. Output stays in inventory, requiring explicit equip. `D/multisell/1005.xml:2`; `G/network/clientpackets/MultiSellChoose.java:247–278,342–363`. Reject ambiguity/multiple base weapons; no client-supplied output template or crystal stage.

Concrete recipes: Stormbringer72 + green stage5 crystal4645 +97 gemstones2131 +291000 Adena57 → Focus4682 (`D/multisell/1005.xml:31–41`). Damascus79 + red stage10 crystal4639 +339 gemstones2132 +3390000 Adena57 → Focus4717 (`:2503–2513`). These disprove a uniform stage-by-grade table and the two-input shorthand. Generate SA closure from relevant blacksmith/Mammon lists, not every unrelated multisell. Tax/discount context is not implemented in Phase4; **QUESTION Q4** asks architect to approve zero external tax for fixture blacksmith while preserving listed base ingredients rather than inventing a castle owner/tax rate.

Soul crystal data includes color/template/current stage/leveled template and every NPC detail condition/chance/level range/absorb type; preserve source missing-field defaults from its loader. Transitions include stages0→18 for all three colors (`D/levelUpCrystalData.xml:4–59`). No generic5% or break probability is assumed. **New hook:** `fn plan_soul_crystal_absorb(context: &AbsorbContext, rules: &SoulCrystalRules) -> AbsorbPlan`, where context holds tick, victim template/life, killer binding, ordered eligible participant bindings, crystal instance/stage and skill-use evidence. Phase4 returns `DeferredPhase6` without RNG, inventory change or XP effect. Phase6 may supply an ordered roll resolver and turn a plan into the ordinary critical item mutation. Hook is called once after verified NPC death credit; it cannot run on replay of a duplicate death (§2; AD:8,27).

## 8. Consumables through the cast pipeline

UseItem resolves the owned item and its exact item_skill list; it does not trust a client skill/rank and does not require a purchased learned rank for the item-granted skill. Add a typed `CastOrigin::Item {instance,template,skill_ordinal,receipt}` beside Learned/Equipment sources. Validate skill capability, item conditions, target/peace, reuse, life and all costs before accepting. Execute through existing target collection, timed stages, effect application and resource/damage pipeline (`Z/state_runtime.rs:1215–1398`; `H/itemhandlers/ItemSkillsTemplate.java:60–119`). Include charges/timing in the start draft, with item-reuse state persisted and hydrated.

ItemConsume is a template/count requirement, paid once at accepted start (source `G/model/actor/L2Character.java:1560–1566`); consume lowest matching owned IDs in order. For item-origin casts, prefer the selected instance when it matches. Do **not** also consume a second clicked scroll: ItemSkillsTemplate's fallback consumption applies only to its applicable CAPSULE/SKILL_REDUCE immediate-effect path with no consuming skill (`H/itemhandlers/ItemSkillsTemplate.java:124–148`). Aggregate a multi-skill item's complete cost plan before mutating. Learning consumes required spellbooks through the same inventory planner atomically with SP/rank/receipt; remove `Z/state_learn.rs:51`'s unconditional item rejection only for admitted prerequisites (AD:28).

Reuse has both skill ID and item template/shared-group keys. Key shared_reuse_group when positive, otherwise template ID; timers survive the consumed instance and block a different stack of the same group. Template reuse_delay is already milliseconds, equip_reuse_delay is seconds converted to milliseconds (`G/model/items/L2Item.java:164,193–194`). Preserve source handler checks for item and skill reuse; no universal potion cooldown (`ItemSkillsTemplate.java:60–95,118–119`). Deadline=`start+ceil(ms/100)`; repeated idempotency key returns original receipt/deadline. Fresh admission pauses/rebases, snapshot restore preserves absolute deadlines (new contract consistent with P3 offline policy).

Escape736→skill2013 has hitTime20000ms, isMagic2, itemConsume736×1, SELF/TOWN; source hitCancelTime500 is unused by existing P3 timing (`D/stats/skills/02000-02099.xml:163–187`). A14 fixes destination(126,126). Use static item timing: animation20000ms, launch19600ms, complete20000ms =200 ticks; interrupt cutoff198 ticks under P3 staging. It is interruptible; “interruptible” does not mean invent100% damage cancel when source cast-break is probabilistic. At completion relocate alive caster, cancel movement/auto, recompute AOI and send existing movement/identity baseline; do not call town **respawn** or reset HP/MP/XP/incarnation. Blessed Escape uses its own item_skill/hitTime, not the20s normal-scroll timing (`D/stats/skills/02000-02099.xml:440–462`).

Resurrection737→skill2014 targets PC_BODY and uses the existing first-valid ResurrectOffer; no instant revival or bypass of offer acceptance/expiry/death incarnation. The scroll is already consumed when offered; decline/expiry is not a refund (`D/stats/skills/02000-02099.xml:189–211`; C12; P3:130–134). Potions/elixirs use their actual restore/periodic handlers and reuse; static item heals must not gain an ordinary magic heal shot bonus. ManaPotion delegates to ItemSkills but is config-gated (`H/itemhandlers/ManaPotion.java:27–36`); O1/A14 explicitly enables this product feature, an override to be recorded in rules, not an implicit always-on source default.

**QUESTION Q13:** source supports simultaneous/immediate potion casts (`ItemSkillsTemplate.java:98–115`), while Phase3 has one cast slot. Proposed V6 adds an ordered item-cast lane keyed by CastId (same phase5 deadlines/RNG ordering) for simultaneous/static immediate items; ordinary escape/resurrection occupies the main slot. It is bounded by inventory/reuse/input admission, fully snapshotted and absent in V5. Do not silently cancel the main cast or reject all potions during casting. Architect must approve this state extension and its C10 contribution before schema freeze.
