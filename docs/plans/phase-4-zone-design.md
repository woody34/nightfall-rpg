> **BINDING RECORD — Phase 4 design, incorporating architect D1–D26; dispositions closed in §13; independent re-review required.** This is the implementation contract for new Snapshot 10 / Record 8 / Binary V6 epochs. It does not claim that the implementation or acceptance gates already pass.
>
> **ARCHITECT ERRATA:** O1–O4 and A1–A22 in the Phase 4 decision record supersede Phase 3 where expressly amended; D1–D26 amend those decisions. Source corrections and closed dispositions are listed below and in §13. Existing Phase 3 C1–C18 remain binding.

# Phase 4 zone engine: items and equipment

## Source authority and reconciliation

Citations use **Z/** = `apps/api/src/domain/zone/`, **R/** = `apps/api/src/application/replay_log/`, **A/** = `apps/api/src/application/`, **W** = `packages/proto/nightfall/v1/world.proto`, **P** = `packages/proto/nightfall/v1/game.proto`. **G/** = `/home/matt-woodruff/.cache/nightfall/l2j/l2j-game/src/main/java/com/l2jserver/gameserver/`, revision `abfde0490ac52a2106e884b7242c0276c44cdf76`; **GC** = that repository's `src/main/resources/config/character.properties`. **D/** = `/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp/src/main/resources/data/`, **H/** = that repository's `src/main/java/com/l2jserver/datapack/handlers/`, revision `3ca488dd2bd0bfaca43e378886a3c2e37968153a`.

**N/** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase4-codex-2026-10-12/`; **AD** = `N/architecture-decisions.md`; **P3** = `docs/plans/phase-3-zone-design.md`; **C** = `docs/plans/phase-3-architecture-decisions.md`. References describe the inspected base branch, not proposed future source line numbers. Abbreviated bare filenames/`:line` continuations refer to the fully qualified source most recently named in that paragraph/table. Scenario filenames in §12 are under `apps/client-unreal/Scenarios/`. Every **new contract** below is a design decision under AD:15–36,50–76, not a claim that L2J already implements Nightfall's scheduling, IDs, wire or storage. XML decimals are imported as exact rational strings, never binary floating-point values. All executable formulas here use integers, Q, or exact ratios (AD:15).

| Conflict / old assumption | Binding Phase 4 treatment and evidence |
|---|---|
| Universal Squire's Sword and `using SWORD` | V6 reads actual equipment or class fist bases; preserve V5 `StarterWeaponProfile` validation. `Z/skill_profiles.rs:184–266`, `Z/effective_stats.rs:69–138`, `Z/state_runtime.rs:1304`; kits `D/stats/initialEquipment.xml:4–109`. Orcs use Training Gloves and Dwarves a club; “all fighters sword/all mystics wand” is only shorthand. |
| 04's POLE one-hand enchant row | Bodypart controls the row; lrhand pole gets two-hand increment. `G/model/stats/functions/FuncEnchant.java:85–151`. |
| AD A9 calls all soulshot terms ×2 | Blow P.Atk multiplier is **1458/1000**; regular auto/physical-skill multiplier is 2. `G/model/stats/Formulas.java:517–528,684,763–774`. Resolved by AD:51–52 (D1/D2). |
| AD A10 describes shield damage subtraction | Pinned `ShieldBlocks=False` adds sDef to defence; perfect block returns 1. `GC:136–140`; `G/model/stats/Formulas.java:670–678`. Resolved by AD:51–52 (D1/D2). |
| Retail augmentation mixed with old offsets | Retail table branch returns before offset algorithm; `GC:393–405`, `G/datatables/AugmentationData.java:447–559,563–686`. Resolved by AD:53 (D3). |
| “SA costs base weapon + crystal; C stages 7–8” | Actual multisell ingredients include gemstones and Adena; C Stormbringer uses stage 5. `D/multisell/1005.xml:19–41`. |
| “Soul-crystal stages through 16” | Import every explicit transition, including 17→18; no leveling execution in Phase 4. `D/levelUpCrystalData.xml:4–59`; AD:8. |
| Enchant armour “0 beyond +19” | Ordinary armour current +19 still attempts +20; full armour current +19 uses its shifted row; +20 onward has zero **base** chance. Bonus success at base zero must not increment. `D/enchantItemGroups.xml:4–45`; `G/network/clientpackets/RequestEnchantItem.java:163–167`. |
| UUID item IDs, separate inventory stream, DB-first actor, floats/hot reload in 04 | Rejected by AD:15–20. IDs and data are zone-canonical, immutable per epoch, additive existing WS stream, durable critical path. |
| Phase 3 deferred shield/shots/position/FAN | Executable only in V6; Phase 3 and older recordings keep their exact code paths, fields and RNG draws (AD:18,24). |

## 1. Identity, canonical state and hydration

### 1.1 Ownership and identity

**New contract.** Add `item.rs`, `item_rules.rs`, `phase4.rs`, `phase4_checkpoint.rs`, `item_math.rs`, `state_items.rs` and focused item combat helpers to `Z/mod.rs` and its deterministic SOURCES scan. Extend `ZoneState` with `phase4: Option<Phase4Zone>` beside Phase3; do not place default equipment into legacy `Entity`. Reuse the single tick draft, effective-stat cache, schedule and checkpoint lane; no inventory actor, timer task or independently published inventory transaction (AD:15–19; `Z/phase3.rs:270–423`, `Z/state_runtime.rs:534–609`).

`ItemInstanceId(u64)` is opaque and stable across equip, enchant, reconnect and checkpoint restore. Zero is invalid. Creation allocates checked `next_item_instance`, then advances it; consumption to zero destroys that identity permanently. Counts must fit positive signed bigint; no wrapping or saturating grants. Stack merge retains the existing lowest instance ID and allocates nothing. Nonstackables have count=1; stackability comes from the template (`G/model/items/L2Item.java:173`; AD:19,34). SA exchange consumes an old instance and allocates a new one (§7); augmentation/enchant mutate an existing instance.

**Identity rule I1 (D8/D19, AD:58,69).** API reserves disjoint ranges of 2^32 IDs per `(zone_id,epoch)` in `item_id_ranges`, transactionally under a database exclusion/serialized allocation guard. Reserve `[1,2^32)` exclusively for fixtures; production ranges start at `2^32`, use exclusive ends strictly <2^63, and store start/end/next as signed bigint with checked conversions. Reserve no block whose exclusive end reaches 2^63. Snapshot/admission stores `{range_start,next_item_instance,range_end_exclusive}` with `0 < range_start ≤ next_item_instance ≤ range_end_exclusive < 2^63`; next=end denotes exhaustion and is never an allocatable ID. Allocation past the end fails closed without a partial mutation; provisioning a new range requires a recorded ordered allocator-extension admission fact before retry. Never call a DB sequence from simulation, wrap, renumber loaded items or reclaim committed IDs. Range ownership applies to newly created IDs, not carried items from another epoch. Recovery reuses the persisted range/next, not a fresh range. Fixture seed manifest declares its reserved subset and next; concurrent seeders must have disjoint reservations. The block size is a new design allocation policy explicitly delegated by D8, not an L2J constant.

### 1.2 Phase4Rules

**New contract.** `Phase4Rules::try_new` validates all cross-references before calculating a separate hash; Deserialize repeats validation and verifies the supplied hash, as `Phase3Rules` does (`Z/phase3.rs:91–158,230–264`). Canonical hash input starts with a new domain separator `nightfall.phase4.rules.1\0`, then length-delimited, explicitly typed, ordered encodings of:

- Full item definitions: stable template ID/kind/grade/bodypart, weight, counts, price/crystal parameters, flags/conditions, XML-ordered funcs, item/enchant4 skills, shot consumption and reduction chances, bow MP/reuse, attack range/angle/random damage, equipment skill reuse, enchant options. Preserve XML ordinal even when names coincide. Defaults come from `G/model/items/L2Item.java:157–218`, `L2Weapon.java:82–108`, `L2Armor.java:47–80`, `L2EtcItem.java:49–73`.
- Weapon kind/trait masks (`G/model/items/type/WeaponType.java:27–63`), generated template statistics, grade IDs and S-plus mapping, enchant groups/scroll/support tables, FuncEnchant and enchant-HP tables, blacklist, armour sets and base-stat set deltas (`D/enchantItemGroups.xml:4–92`; `G/model/stats/functions/formulas/FuncArmorSet.java:51–67`). WeaponType itself supplies traits/masks, **not** universal attack-speed/critical/stat defaults; item XML supplies those.
- Augmentation retail rows in original XML order, category weights, options/skill closure, life-stone eligibility/costs/cancel fees; SA recipe ingredients/products/maintain flags; soul-crystal transitions and NPC detail data; per-class starter kits in XML order; capacities/weight/expertise policies. AD:21–23,27–28.
- V6 formula revision and integer adaptation policy, positional geometry/facing policy including the certified integer angle table (D21), blacksmith identity/location/radius and explicit fixture tax rate0 (D22), item capability manifest with explicit blockers, and hash of the Phase3 skill/profile bundle used by this epoch. Keep older hashes unchanged; hash normalized semantic values and ordering, not absolute local paths or comments (AD:18,71–72; `Z/phase3.rs:138–158`).

Import all Weapon/Armor rows but separate **catalogued** from **executable**: unsupported item conditions, trigger skills or special modes produce named blockers, never partial stat installation. O1-required kinds, armour sets, augmentation options and consumables must have a complete executable closure before release; blockers are not an excuse to ship a missing owner requirement. Common/shadow/time-limited contexts remain excluded according to AD:12. Include 57 (Adena) and every multisell input in EtcItem closure, not just the short A7 list (§7). Replay embeds immutable rules once, interned by content hash; `GetItemCatalog` projects from the same bundle (AD:18,21).

### 1.3 Phase4Snapshot and invariants

**New contract**, typed and `deny_unknown_fields`:

```rust
struct Phase4Snapshot {
    rules: Arc<Phase4Rules>, rules_hash: [u8; 32],
    range_start: u64, next_item_instance: u64, range_end_exclusive: u64,
    players: BTreeMap<EntityId, Phase4Player>,
    facing: BTreeMap<EntityId, FacingVector>, // every V6 combatant, including NPCs
    item_casts: BTreeMap<CastId, ItemCastState>, // I5; shared Phase3 CastId allocator
    attack_plans: BTreeMap<EntityId, ItemAttackPlan>, // projectile/dual subhit state
    charge_bindings: BTreeMap<(ItemInstanceId, ShotKind), ActionBinding>,
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

`ItemLocation` admits only INVENTORY/PAPERDOLL; quest is a template pool, not a DB-only container. A charge binding reserves a paid charge for one pending attack/cast; concurrent lanes cannot reuse it. Bind only the shot kinds that the source skill consumes, release on cancellation-before-use, and clear on consumption/unequip. Snapshot validates binding/action/weapon ownership; offline checkpoint has no pending binding. Bound is fixed false; duration/mana are absent for excluded shadow/time-limited items. SA is represented by the variant template; never add an independently mutable SA stat field. Charges belong to the weapon instance, not a global character buff (`H/itemhandlers/SoulShots.java:80–98`; `G/model/itemcontainer/PcInventory.java:961–966`). Inventory includes equipped entries exactly once; paperdoll is a checked index, not a second copy. Arrows are the lhand instance and count, with no second arrows counter (`G/model/itemcontainer/Inventory.java:679–713`). Effective sheets and template/slot indexes are rebuilt caches, excluded from digest; compare recomputable weight/penalty/set projections against reconstruction on restore, retaining the explicit overload/set history bits. Inventory revision increments once per changed player's committed tick; delta payloads identify that revision (new wire fence under AD:16–19).

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

**Mandatory implementation gate (D4, AD:54):** generate the full rules and encode the actual worst case (C10's 1,000 combatants/full pools and eight fully equipped players, plus realistic soak), print decoded/encoded bytes and largest contributors. Also test maximum executable inventory-limit modifiers. If bounds fail, compact/intern first; report exact residual excess to architect. No truncation, new storage fetch during replay, or increased NATS limit. This design proves the base inventory component fits; it deliberately does not fabricate a full-envelope result without the generated rules/codec.

### 1.5 Owner baseline and visible equipment

**New contract.** AOI admission/replacement order is: all identity `EntitySpawn`s (including visible equipment), reference-bearing swing/cast/effect hydration, owner `SkillStateSync`, then owner `InventorySync`. InventorySync carries `entity`, `incarnation`, `state_tick`, `baseline=true`, `baseline_tick`, `inventory_revision`, full items/paperdoll, auto-shot toggles, weapon charge bits, item reuse deadlines, capacities/weight and penalties. No owner baseline is filtered because an identical observer state was seen (C2/C4/C12; P3:166–210). Socket replacement preserves live charges/reuse; true fresh admission restores checkpoint state without charging again.

Public `VisibleEquipment` contains `(slot, template_id, glow_tier)` only, including an explicit empty slot/tombstone in deltas. No instance IDs, counts, exact enchant, augmentation or inventory revision in public entries. `EntitySpawn.visible_equipment` and `EquipmentChanged.visible` use identical projection. Owner's exact item mutations arrive separately in InventoryChanged; EquipmentChanged can therefore be safely shared with observers. Glow tiers are a **placeholder presentation policy**, not a claimed L2 client retail threshold; presentation rule I6 under D13 uses tier0 below weapon+7/armour+6, tier1 from those levels, and tier2 for weapon+15; these tiers reuse §4 announcement milestones solely as placeholder art. StatsChanged stays owner-only; effective maximum HP changes additionally use public VitalsChanged (AD:16; C4; C2'). Baseline fences reject prior incarnation/older state_tick and atomically replace the owner inventory; deltas are applied in committed tick/revision order. A missing revision triggers a new owner baseline, never guessed counts.

## 2. V6 in-tick ordering and RNG ledger

### 2.1 Schedule amendment

**New contract.** Add a V6 policy branch; do not edit V5's observable schedule/RNG. The existing driver is `Z/state_runtime.rs:545–645`, with V5 auto resolution in `Z/state_runtime_auto.rs:176–253`. Tick-local ledgers, output coalescers and dirty sets are scratch; canonical pending action state is retained.

| Phase | V6 amendment to P3 §3 |
|---|---|
| 0 — receive-order commands | Authorize/fingerprint/deduplicate first. Resolve equip, unequip, destroy, crystallize, enchant, refine, cancel-refine, SA exchange, grants and ToggleAutoShot in command ordinal order. Manual shot UseItem charges here; potion/scroll UseItem stages an item-origin cast. Reconcile equipment/sets/weight/penalties and rebuild immediately after each accepted mutation, before the next command. Enchant/refine RNG occurs here after complete preflight. Learning item costs run in its existing critical command phase. |
| 1 — spawn | Attach complete V6 sidecars, default facing and starter-kit state before publishing identity. |
| 2 — expiry | Expire ordinary effects/reuse/PvP/offers, including item shared-reuse deadlines; equipment System sources have no duration. Recompute derived stats/weight limit/penalties if a provider expires. |
| 3 — periodic + regen | Preserve V5 handler/regen order; overload sources already affect this tick. |
| 4 — chase/movement | Every actual nonzero displacement updates canonical facing; blocked/no-op moves do not. Resolve all geometry from post-step positions. |
| 5 — cast launches/impacts | Existing total order `(deadline,caster ordinal,caster id,cast id)`. Prepaid item cost is not charged again. Use cast shot context across all handlers/targets. Shield/blow/lethal draws occur at the caller sites in §2.2. On finalization clear consumed charge context; next auto-charge attempt is at next eligible start. Rebuild immediately if any consumption/effect changes a condition. |
| 6 — auto impacts | Attacker-ID order, subhit index order within one attack. Revalidate life/incarnation/range/peace/status before resolution; the stored second dual subhit revalidates life/incarnation/peace/hostility and status (stun/sleep/paralysis/cast, physical mute) but NOT range, as pinned `L2Character.onHitTimer` does not (D107); cancelled actions draw no damage RNG. Projectile costs were paid at accepted start. Use shield/position/shots in §5. Dual planning caveat below. |
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
| Bow accepted start | Optional reduced-MP draw at source condition, preflight resulting MP/arrow, optional shot reduction draw, then target-limit selection if relevant. Pay one arrow regardless of future hit/miss. `G/model/actor/L2Character.java:701–758,1007–1026`. Insufficient randomized MP cost must retain the attempted draw fact; preserve it as a committed rejected-attempt fact while rolling back item/MP changes. |
| Single auto/pole recipient | hit1000 → on hit shield subroutine → critical1000 → spread(2r+1) unless perfect block or r=0 → recipient status/death hooks → attack-break/reflection at the existing auto caller sites. Miss skips shield/critical/spread. `G/model/actor/L2Character.java:1318–1340`; `Z/state_runtime_auto.rs:104–155`. |
| Shield subroutine | Eligibility/angle/rate guards draw none. Perfect100 first, success iff `100-perfect_rate < residue`; on failure normal100, success iff `residue*Q < rate_q`. Perfect skips normal. `G/model/stats/Formulas.java:1077–1105`. |
| PhysicalAttack handler | Existing physical-skill evasion100 → shield → optional skill critical(100×Q) → spread unless perfect → recipient hooks → skill reflection (no direct attack-break call in PhysicalAttack). Source handler order is authoritative, not auto's order (`H/effecthandlers/instant/PhysicalAttack.java:49–104`; P3 §6). |
| FatalBlow | physical-skill evasion100 → on non-evasion blow1000 → on blow success shield → spread unless perfect → optional critical(100×Q) → recipient hooks → attack-break → reflection. `H/effecthandlers/instant/FatalBlow.java:49–102`. Blow uses A8's per-mille adaptation, not an additional normal hit roll. |
| MagicalAttack / range | Magic-critical1000 → shield → first/conditional second magic-success100 → spread; if positive damage, attack-break when eligible → vengeance100 even when rate0 → chosen recipient damage/status/death hooks. Keep Range's handler-specific branches in XML order; do not borrow the auto caller order. Perfect block returns before formula success/spread draws, but does not undo caller draws already made. `H/effecthandlers/instant/MagicalAttack.java:70–92`; `Formulas.java:815–897`. |
| Retail accessory augment | option12(10000) → category(10000; weights5500/3500/900/100) → option34(10000). `G/datatables/AugmentationData.java:689–737`. |
| Heal | No shield/hit draw. Existing eligible magic-critical1000 draw only; shot coefficient determined before it (`H/effecthandlers/instant/Heal.java:68–99`). |
| Lethal handler | Guard magic level/eligibility/invulnerability; full1000 then half1000 only if full fails, even if chance is zero. Execute in XML handler order independently of FatalBlow; no invented shared success flag. `H/effecthandlers/instant/Lethal.java:60–96`. |
| Enchant | One enchant100000 after preflight; draw even at zero or certain chance. Never reroll durable retry. `G/model/items/enchant/EnchantScroll.java:142–157`; AD:22. |
| Retail weapon augment | option12(10000) → category(10000) → option34(10000); no skill/glow/base-stat/offset draws (§7). `G/datatables/AugmentationData.java:463–504,517–558`; AD:22. |

Dual source order is **hit1, hit2, then shield1/crit1/spread1, then shield2/crit2/spread2**, with each miss skipping that hit's later draws; integer damage is halved after truncation (`G/model/actor/L2Character.java:1137–1179`). To retain that order and separate subhit timing, V6 creates a canonical dual resolution plan when the first subhit becomes due: capture both hit tests and both formula results in that order, store the second result until its deadline; recipient damage/status/death hooks run at each actual impact, never twice. If the second target incarnation becomes invalid, discard its stored result without refunding consumed planning RNG. This is numbered rule I3, a V6 scheduling adaptation of source launch-time calculation delegated by D13 (AD:63). No V5 helper gains this behavior.

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

Direct set STR/DEX/CON/INT/MEN/WIT deltas are priority1 **base-stat** inputs, before bonus lookup; they are not additive final P.Atk/HP bonuses (`G/model/stats/functions/formulas/FuncArmorSet.java:46–67`). **D16 (AD:66): equipment providers are V6 System modifier sources reconstructed from retained equipment, independently of ordinary timed `ActiveEffect` lifecycle flags.** Item functions, set/item skills, augmentation passive providers, penalties4270/6209/6213 and equipment-dependent masteries survive death and dead admission regardless of the source skill's `stayAfterDeath` (whose pinned default is false, `G/model/skills/Skill.java:231`). They spend no timed buff slots and have no expiry; retain their provider identity/shared install order and fold each winning contribution exactly once. Death or resurrection does not remove/reinstall them; dead admission reconstructs them from the checkpoint just as living admission does, without onStart, healing or sequence duplication.

Implement a V6 modifier-source path, not synthetic ordinary `ActiveEffect`s with rewritten source flags. Existing validation rejects System-pool effects and passive effect skills, checks source flags and rejects nonpersistent effects on dead admission (`Z/effects.rs:348–360,653–721`; `Z/phase3_checkpoint.rs:184–190`); preserve those V5 validation paths, death cleanup and fold behavior (`Z/state_death.rs:112–122`; `Z/effective_stats.rs:333–356`). V6 separately validates provider ownership, executable source/rank, unique sequence and retained equipment/condition correspondence. Active augmentation/item skills remain granted capabilities using their actual operate type/cast pipeline; ordinary effects created by their casts retain source lifecycle rules, including death removal when `stayAfterDeath=false`. Augmentation triggers require complete effects support before admission (§7).

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

ENCHANTHP is priority40 (`G/enums/StatFunction.java:25–32`; `G/model/stats/functions/FuncTemplate.java:48–54`), and is implicitly attached by the loader to graded armour in CHEST, FEET, GLOVES, HEAD, LEGS, BACK, FULL_ARMOR, UNDERWEAR, LHAND and BELT. It is not inferred by looking only for `<enchanthp>` XML (`G/data/xml/impl/EnchantItemHPBonusData.java:83–105`; `G/model/stats/functions/FuncEnchantHp.java:37–44`). Bonus index is `min(e,table_len)−1`, zero when e=0 or no grade row, onepiece bonus truncates `table_value×3/2`; `EnchantItemHPBonusData.java:50,116–126`. Read values from `D/stats/enchantHPBonus.xml`, never extrapolate beyond last row.

### 3.4 Set detection

Select set by equipped CHEST template, require every nonempty allowed-ID list for legs/head/gloves/feet; an empty list is optional. Fullarmor naturally uses the set's absent legs requirement, not a forged second item. Shield bonus uses its separate list; enchant6 checks required pieces only, excluding optional pieces and shield (`G/model/L2ArmorSet.java:128–170,192–196,215–248`; `G/model/itemcontainer/PcInventory.java:1143–1208`). Install exact full-set, shield and enchant6 skill lists plus direct base-stat deltas. `enchant4_skill` is an item property (including weapons), not “every full set gives a +4 skill”; install only when equipped e≥4 and property exists (`PcInventory.java:969–976,1067–1080`; `G/model/items/L2Weapon.java:108–123`). Reconcile after equip, unequip, enchant success/reset/destruction, load, level/class and any condition-changing effect. D20 (AD:70) requires a per-chest-set-provider `SetActivation.shield_skill_active` flag, retained even for an incomplete set. Transition it in slot-mutation order using the pinned equip/removal branches: completing the set with its shield present installs the shield skill; equipping a matching shield while the chest is present installs it even if the set is incomplete; removing the chest, a required set part or the matching shield removes it (`G/model/itemcontainer/PcInventory.java:1147–1208,1232–1285`). Thus chest→shield (incomplete set) activates, while shield→chest (incomplete set) does not. Persist the flag in snapshot/checkpoint and rebuild its provider from that flag without inventing listener transitions. Full-set/enchant6 activation remains independently derived; ordinary rebuild must not replace history with `full_set && shield`.

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

Source: `D/enchantItemGroups.xml:4–59`. Full armour +19 is **5890**, not 5560: the shifted schedule terminates at +20. Weapon “uncapped” means no low gameplay cap; the finite source table/default maximum still matters. A signed SQL smallint cannot represent the source maximum; D19 requires signed bigint with u32 CHECK constraints (§9.1).

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

Charging consumes inventory immediately and stores charge bits on the active weapon; firing consumes the **charge**, not a second inventory payment. Physical count is template soulshots, magical count spiritshots; matching is `getItemGradeSPlus` so S80/S84 use S shots (`H/itemhandlers/SoulShots.java:61–98`, `SpiritShot.java:60–92`, `BlessedSpiritShot.java:63–95`). Fists/no equipped weapon cannot charge. An already-charged same kind returns a successful no-change receipt with no spend (new idempotent API projection of the source early return). Blessed can be charged over ordinary spiritshot; formula precedence is blessed first, not multiplication of both (`BlessedSpiritShot.java:71–95`; `G/model/stats/Formulas.java:843`).

Toggle stores a template ID, not a particular stack ID. Iterate toggles in template-ID order and choose the lowest matching available instance; this fixes source unordered iteration deterministically. Enabling attempts immediate charge as pinned; later accepted start attempts recharge before cast timing. Source enables the toggle **even on grade mismatch**, reports mismatch, and attempts recharge (`G/network/clientpackets/RequestAutoSoulShot.java:105–124`); numbered rule I2 under D13 preserves that toggle and rejects only the charge attempt. Insufficient stock disables that auto-shot template (`H/itemhandlers/SoulShots.java:91–95`, `SpiritShot.java:84–88`). Unequipping clears all weapon charge bits, without refund (`G/model/itemcontainer/PcInventory.java:961–966`).

Source auto attack clears soulshot only if at least one hit lands; a wholly missed dual/pole attack retains it (`G/model/actor/L2Character.java:889,953–958`). **Numbered rule I2 (D13, AD:63):** one prepaid charge per attack bundle, shared across both dual halves/all pole recipients; discharge on first landed bundle hit, retain on all misses. A cast binds its charge context once and uses it for all eligible handlers/targets; it cannot consume one full shot pack per AoE target. After target and SELF handlers, if useSpiritShot then discharge blessed when present, otherwise ordinary; only the else-if useSoulShot branch clears physical. Do not clear both magical bits or an additional physical charge (`G/model/skills/Skill.java:1144–1157`). Cancel before use leaves weapon charge intact, subject to unequip clearing. This supersedes A9's “per hit” shorthand under the explicit D13 delegation. Record paid inventory deltas separately from discharged charge bits so replay can prove no double charge. All charge/auto-toggle changes are durable even when count delta=0 (AD:17,23). A9/I2 intentionally schedules automatic recharge at accepted start before timing; pinned source also recharges at attack/cast tail sites (`G/model/actor/L2Character.java:4014,4169,4611,4700`). Do not add those tail charges as a second V6 payment.

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

D59 uses the existing whole HP plus `ResourceFractions.hp` representation: `hp_q = whole×Q + remainder_q`; D61 half-kill sets `max(1, floor(hp_q/2))`, retaining one Q-unit at the minimum positive total. Thus 1, 2 and 3 HP become 0.5, 1 and 1.5 HP. V6 liveness is `hp_q > 0`, including whole HP0 with a positive remainder; the public HP display remains the whole part. Snapshot validation, paused admission, casting, ordinary damage and regeneration use that same liveness rule. Regeneration carries the remainder; only reaching total HP0 ends a life. Historical epochs retain their whole-HP damage/death decisions. This is the explicit D59/D61 fixed-point adaptation of `Lethal.java:90`; the lower bound is one Q-unit, not one whole HP.

### 5.3 Shield and deterministic geometry

Eligible secondary item must be L2Armor and not SIGIL; rate=`fold(rShld,0)×DEXbonus`. Rate≤1/Q returns no block/no draws. Angle=`trunc(fold(shieldDefAngle,0))+120` degrees; angle≥360 skips facing guard. Bow attacks multiply rate by13/10. Perfect test runs **before** ordinary rate test: `100−10 < roll100`, using pinned perfect-rate10, hence residues91–99 succeed (**nine**, not ten outcomes). Only on perfect failure roll normal shield. (`G/model/stats/Formulas.java:1077–1105`; `GC:136–140`.) No sDef subtraction from final damage; source ShieldBlocks alternative is inactive.

**New geometry contract under A10.** Store a primitive integer direction vector for every combatant, last nonzero actual movement delta divided by gcd; initial/new-life direction is east `(1,0)` until movement; teleport within the same life preserves existing facing. Facing is not changed by client aim or target selection. This differs from L2J's turn-to-target on attack/cast (`G/model/actor/L2Character.java:890,1554–1557`) as A10 explicitly requires last-movement facing. Include this policy in Phase4Rules hash.

Implementation (T51/D157): the movement phase sets the facing after each step with an actual displacement. Death does not change the facing. A town respawn or an accepted resurrection starts east. An escape teleport keeps the facing. D158: a same-life re-admission (logout, then login) starts east, because the item checkpoint does not keep the facing. Phase 5 follow-up: L2J turns the actor toward its target on attack and cast. Phase 4 does not, because A10 requires last-movement facing.

Classify actual target-relative position, never the mutable selected target of an AoE caster. Given victim facing f and victim→attacker vector v: front if dot≥0 and `4×dot²≥|f|²×|v|²`; back if dot<0 and same squared test; otherwise side. Equality belongs to front/back (source half-angle60°; `L2Character.java:4930–4993`). Coincident positions count front and satisfy facing (new zero-vector policy). Shield/FAN use the D21 certified integer angle table: exact rational intervals bounding `cos(angle/2)`, refined until every reachable bounded integer vector comparison is decided, with exact special-angle boundaries and inclusive-boundary oracle vectors. Hash the table and its coordinate bounds in Phase4Rules (AD:71). No runtime trig or binary floating point. Any imported angle the generator cannot certify is a named blocker for that skill; never approximate it by 8-way sectors.

For hit chance, apply `trunc((80+2×(accuracy−evasion))×10×condition_bonus)` then clamp200..980; hit iff residue≤chance (source miss uses strict `<`). `condition_bonus=max((100+front/side/back+height+dark)/100,0)` using existing hit_condition_bonus table. Flat test_zone has no height modifier, and no invented day/night wall clock (`G/model/stats/Formulas.java:1054–1063`; `packages/data/tables-phase3/hit_condition_bonus.toml:3–10,12–58`).

### 5.4 Bow, dual, fist and pole

Bow start requires compatible lhand arrow (auto-select lowest matching ID if empty), reuse ready and enough MP after BOW_MP_CONSUME_RATE. Pay MP once, consume one arrow even if the shot misses; no-arrow start does not charge/fire. Grade comparison collapses S-plus (`G/model/itemcontainer/Inventory.java:679–713`; `G/model/actor/L2Character.java:711–758,1013–1016`). Use weapon `mp_consume` and reuse_delay, not a grade-derived guessed cost. Reach=`floor(range_L2×1000/32)` milli-tiles (`Z/combat.rs:286–296`), so source500 reach is15,625 milli-tiles. Timing: `attack_ms=trunc(500000/pAtkSpd_getter)`, `reuse_ms=trunc(template.reuse_delay×333/pAtkSpd_getter)`, bow impact at start+ceil(attack_ms/100), next eligible start at start+ceil((attack_ms+reuse_ms)/100). Thus “1500ms bow reuse” is a **base template value**, not fixed15 ticks (`G/model/actor/L2Character.java:1007–1045,4335–4357`).

Bow damage additionally truncates ordinary damage to whole first, then truncates multiplication by `(distance_L2/4000+4/5)` (`L2Character.java:1029–1033`). Compute Euclidean distance with the existing exact-root machinery and units conversion, not Manhattan or squared distance. Crossbow uses its own source branch/template; do not apply bow distance falloff to it without source support. Arrows exhausted to zero clear lhand before weight/penalty rebuild and emit one removal.

Player FIST falls through to DUAL/DUALFIST/DUALDAGGER handling; NPC FIST uses simple hit (`L2Character.java:923–935`). Dual halves are each `trunc(formulaDamage)/2` with integer division; deadlines correspond to half of `timeToHit` and full `timeToHit`, where `timeToHit=attack_ms/2` (`:887,1158–1179`). Preserve second-hit canonical plan and separate public outcomes. An unequipped player uses class fist attack/speed/critical bases; the current constructor still sets `random_damage` from the weapon profile (`Z/effective_stats.rs:105–116`), so V6 must explicitly replace that assignment. **D24:** with no active weapon, random damage radius is `r=5+isqrt(level)`, using `fist_random_damage` (`G/model/actor/L2Character.java:5034–5044`; `Z/stat_sheet.rs:276–281`; AD:74). Equipped weapons use their template radius. Recompute on equip, unequip and level change. The spread draw bound is `2r+1`, with offset `residue−r` and multiplier `(100+offset)/100`; level1 therefore has radius6 and bound13.

D25 pole rounding is per recipient: `base=trunc(calcPhysDam(...))`; primary damage is base; secondary damage is `trunc(base × attackpercent / 100)`. Retain attackpercent as an exact ratio starting at85 and divide by23/20 after every attempted eligible secondary, including misses (`G/model/actor/L2Character.java:1282–1283,1335–1338`; AD:75). A miss does not calculate damage but still advances the percentage. Preserve the formula minimum before these caller truncations; do not reapply a minimum after secondary scaling (perfect-block base1 becomes0 at85%). Range and angle are effective physical attack range/angle; scan secondaries in entity-ordinal then EntityId order (new deterministic replacement for source map iteration), excluding primary, wrong hostility/peace/dead, outside radius/cone. Skill3599 contributes attack-count modifiers; **source increments secondary count and breaks when count > max**, not ≥, so do not equate its comment “three targets” to the executed bound (`G/model/actor/L2Character.java:1202–1206,1237–1287`; `D/stats/skills/03500-03599.xml:1695–1707`). Preserve this executed off-by-one bound; it is an oracle target, not a new cap. FAN skill target collection uses the same geometry and ordinary skill affect-limit semantics (C1), but is not a replacement for the separate pole auto-attack count algorithm.

### 5.5 Rounding ledger

| Boundary | Required operation |
|---|---|
| Imported decimal | Exact base10 numerator/denominator; reject unrepresentable fixed-scale chances; no host numeric parser through a binary float. |
| Stat funcs | Existing Q conversion/priority ordering; checked i128 multiply before division, explicit Java truncation toward zero where source casts. SET/ENCHANT additions happen before STR/INT/LM (§3). |
| Crit/evasion/shield getter | Preserve getter cast/cap before later formula factors; rShld comparison retains Q fractional rate. |
| Enchant chance | Exact percent×1000, clamp upper100000 only after bonuses; strict residue<c. |
| Defence | Slot subtraction and normal shield addition occur before defence multipliers/division. |
| Roots | Carry `RootExpression`/exact bounds through final comparison or rounding; do not floor sqrt(M) before applying power. |
| Damage | Preserve formula minimum and caller truncation separately; bow applies a second truncation; dual truncates then integer-halves; pole truncates each formula result before secondary percentage scaling and truncates again, with no post-scaling minimum (§5.4). |
| Time | Source truncates milliseconds first; convert absolute nonnegative offsets by integer ceil-to100ms, retaining P3 minimum one-tick cast completion. |
| Weight | Truncate signed `(weight−bonus)×1000/maxLoad` toward zero, not floor for negative values. MaxLoad floors pre-fold CON product, then truncates post-fold getter. |
| Restore | Whole resource credit plus Q remainder under existing P3 rules; clamp-no-heal does not manufacture fractional credit. |

Each oracle vector records intermediate terms and casts, not only final damage (P3:447–461; AD:15,21–22).

## 6. Weight, overload and expertise penalties

`weight=sum(template.weight×count)` over all inventory including equipped entries once; weight is a source unit, not grams (`G/model/items/L2Item.java:162`; `G/model/itemcontainer/Inventory.java:670–675`). `maxLoad=trunc(fold(weightLimit,floor(CONbonus×69000×WeightLimit)))`; pinned WeightLimit=1. `bonus=trunc(fold(weightPenalty,1))`, deliberately **one**, not zero (`G/model/actor/instance/L2PcInstance.java:9427–9438`; `GC:27`). No31000/176000 clamp from older chronicles.

For maxLoad>0, `per_mille=trunc((weight−bonus)×1000/maxLoad)`. Penalty levels: <500→0, <666→1, <800→2, <1000→3, else4. All four nonzero ranks halve regHp/regMp; movement multipliers are1,1/2,1/2,0. Weight zero still includes the bonus subtraction; negative per_mille is valid and maps0 (`L2PcInstance.java:1668–1693`; `D/stats/skills/04200-04299.xml:1623–1639`). Level4 and overloaded are distinct: source overload condition is `weight > maxLoad`, strictly greater; at exactly maxLoad it is false. **Numbered rule I4 (D13, AD:63):** preserve the source transition quirk: update overloaded only inside the penalty-rank-change branch. Crossing `weight=maxLoad` while remaining rank4 can retain the previous boolean; snapshot/checkpoint must therefore preserve it, not recompute it from current weight alone. WeightChanged exposes numeric weight and PenaltyChanged exposes this boolean independently. MaxLoad≤0 is source no-op, not division by zero.

Recompute after grant/consume/destroy/refund/exchange, CON/weightLimit/weightPenalty changes, level/class and restore. Equip does not change total weight but can change capacity/CON/maxLoad. Movement must tolerate effective speed zero without violating existing positive-Speed constructors; use movement-disabled state for overload rank4 and preserve base speed, rather than inventing speed1. `can_accept` independently checks slot and weight budgets for ordinary acquisition (`G/model/itemcontainer/PcInventory.java:772–785`); fixture overload states can be reached by seeded load or removing a capacity/CON bonus, not a bypass in production acquisition.

Expertise=`max(learned239 rank,0)`, with source auto-get levels20/40/52/61/76/80/84 for D/C/B/A/S/S80/S84 (`G/model/actor/instance/L2PcInstance.java:8095–8104`; `D/skillTrees/classSkillTree.xml:8–14`). Do not collapse S80/S84 to5 for penalties. Default expertise penalty bonus is zero unless an admitted explicit provider supplies it; no invented “expertise bonus” by level (`L2PcInstance.java:1650–1655,1727–1731`).

Canonical scan is ascending ItemInstanceId, each equipped instance exactly once; skip arrows/bolts. Start weaponGrade=armorGrade=0. For grade>expertise: if weapon **and grade>weaponGrade**, set weaponGrade; **else if** grade>armorGrade set armorGrade. This preserves the weapon-to-armour branch quirk, even though valid one-RHAND states normally cannot expose a second equipped weapon. Each final rank=`clamp(grade−expertise−bonus,0,4)` (`L2PcInstance.java:1702–1753`). Recompute on equip/unequip, level/learn/class and rollback-free restoration.

System6209 rank1–4: accuracy−16; critRate and critDmg×9/10,8/10,7/10,6/10; P.AtkSpd/P.Atk×9/10 at every rank. System6213: evasion−5/2,5,15/2,10; P.AtkSpd/M.AtkSpd/runSpd×8333/10000,6944/10000,5787/10000,4823/10000 (`D/stats/skills/06200-06299.xml:324–359`). Install only nonzero ranks, replace rank under source identity, remove at zero; both plus4270 have P/stayAfterDeath. They are effect-engine inputs, not post-hoc multiplication of a serialized sheet (AD:20,26).

## 7. Augmentation, special abilities and soul crystals

### 7.1 Validation and costs

**New fixture contract under A13:** add noncombat `blacksmith` at (124,128), radius3 tiles, within the existing test-zone town; validate current same-zone NPC identity, position, life and range at command resolution. Coordinates are Nightfall fixture policy (D10, AD:60): within bounds `[0,256]`, inside the inclusive peace square `[116,136]`, and distinct from Gatekeeper(128,128) and Class Master(126,128); safe point distance is √8≤3 tiles (`packages/data/zones/test_zone.toml:8–33`; `packages/data/zones/test_zone_phase3.toml:7–11`). Geometry uses squared distance8≤9, never a runtime root. Refine/Cancel/SA all use it; no silent “any NPC” acceptance.

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

CancelRefine fee depends on **instance crystal count including enchant**: C below1720→95000, below2452→150000, else210000; B below1746→240000 else270000; A below2160→330000, below2824→390000 else420000; S480000; S80/S84920000. Consume Adena57, disarm equipped weapon, remove augmentation with no refund (`G/network/clientpackets/RequestRefineCancel.java:57–118`). Fees are data, not values inferred from grade sale price. D11 (AD:61) explicitly includes Adena57, Ancient Adena5575 and all admitted ingredients as fixture-only inputs.

### 7.2 Retail draws, packing and effect application

Pinned config enables retail weapon and accessory augmentation (`GC:393,405`). Weapon routine filters `retailchances.xml` by mage/warrior and exact stone ID, preserving XML order. Draw option12 residue0..9999; start threshold10000 and visit every row: while previous threshold>residue, subtract row chance×100 and replace selected option. This is a descending-tail selection; a conventional ascending CDF with the same draw reverses the outcomes. Draw category using A8 weights5500/3500/700/300; map to category labels55/35/7/3; filter variation2 rows to that label, then draw/select option34 with the same descending algorithm (`G/datatables/AugmentationData.java:447–558`). Keep table weights as exact ratios if chance×100 is fractional; compare rational thresholds to the integer residue. Validate no reachable residue leaves option0, rather than invent rounding/rebalancing of published weights.

Pack as unsigned32 `(stat34<<16)|stat12`, with both halves bounded to16bits and nonzero valid option IDs. PostgreSQL signed int cannot represent every u32 packed value; D19 requires signed bigint with u32 CHECK constraints (AD:69; `RequestRefine.java:108–113`; `AugmentationData.java:558`). Accessory retail also draws option12→category→option34, but category weights are **5500/3500/900/100** with labels55/35/9/1, hard-coded independently of the weapon config. Preserve its warrior/stone filter and descending-tail selections (`AugmentationData.java:689–737`). Oracle must enumerate every10000 residue per choice and preserve list order.

The old non-retail algorithm is **not combined** with retail: it clamps stone index to9, draws skill/glow, conditional1% base-stat, color and option indices; stat subblock91 and block3640 produce `stoneIndex×91+colorOffset`. Normal stat34 uses `color×910+temp×3640+1`, temp∈{2,3}, then offset..offset+90; stat12 chooses a corresponding permitted block, and packing is identical (`AugmentationData.java:563–686`). The15/30/45/60 skill chances and0/40/70/100 glows in GC:371–389 are inactive in the retail weapon branch. Import for provenance if A7 requests them; do not consume their draws in V6 retail mode. D3 (AD:53) resolves the contradictory “retail plus offsets” brief.

Decode option12/34 through `D/stats/options`; install functions and passive skills through the effective-stat/System source engine, grant active skills, register trigger providers in source XML order. `G/model/options/Options.java:61–74,136–175` explicitly distinguishes those three forms. Unequip/cancel removes only that provider and clamps without onStart replay. A skill with an unsupported trigger/handler blocks its source recipe/augmentation table from execution; do not silently discard the rolled skill or reroll into supported outcomes. D5/D6 (AD:55–56) require an explicit capability census, with Charges/EnergyAttack/skill enchant still deferred. Do not change probabilities to avoid blockers; admission must reject a stone/target combination with any unreachable implementation path. Record the missing closure in the data gate and complete the O1-required execution paths before release.

### 7.3 SA recipes and absorb hook

ExchangeSpecialAbility names `(npc,base_instance,recipe_id)`. Recipe ID is a stable generated pair `(multisell list ID,item XML ordinal)`; all inputs/counts/products/maintain flags come from the pinned row. Validate exact base template and all ingredients together, unequip base, consume ingredients, create product with fresh ID; if maintainEnchantment, carry source enchant and augmentation as pinned. Output stays in inventory, requiring explicit equip. `D/multisell/1005.xml:2`; `G/network/clientpackets/MultiSellChoose.java:247–278,342–363`. Reject ambiguity/multiple base weapons; no client-supplied output template or crystal stage.

Concrete recipes: Stormbringer72 + green stage5 crystal4645 +97 gemstones2131 +291000 Adena57 → Focus4682 (`D/multisell/1005.xml:31–41`). Damascus79 + red stage10 crystal4639 +339 gemstones2132 +3390000 Adena57 → Focus4717 (`:2503–2513`). These disprove a uniform stage-by-grade table and the two-input shorthand. Generate SA closure from relevant blacksmith/Mammon lists, not every unrelated multisell. D22 fixes the fixture blacksmith external tax rate at0, explicitly hashed in Phase4Rules (AD:72); charge every listed ingredient, including Adena/gemstones, in full. The source list declares `applyTaxes=true` (`D/multisell/1005.xml:2`); fixture tax0 is a Nightfall contract, not a universal source zero-tax rule. Castle/tax economy remains outside Phase4.

Soul crystal data includes color/template/current stage/leveled template and every NPC detail condition/chance/level range/absorb type; preserve source missing-field defaults from its loader. Transitions include stages0→18 for all three colors (`D/levelUpCrystalData.xml:4–59`). No generic5% or break probability is assumed. **New hook:** `fn plan_soul_crystal_absorb(context: &AbsorbContext, rules: &SoulCrystalRules) -> AbsorbPlan`, where context holds tick, victim template/life, killer binding, ordered eligible participant bindings, crystal instance/stage and skill-use evidence. Phase4 returns `DeferredPhase6` without RNG, inventory change or XP effect. Phase6 may supply an ordered roll resolver and turn a plan into the ordinary critical item mutation. Hook is called once after verified NPC death credit; it cannot run on replay of a duplicate death (§2; AD:8,27).

## 8. Consumables through the cast pipeline

UseItem resolves the owned item and its exact item_skill list; it does not trust a client skill/rank and does not require a purchased learned rank for the item-granted skill. Add a typed `CastOrigin::Item {instance,template,skill_ordinal,receipt}` beside Learned/Equipment sources. Validate skill capability, item conditions, target/peace, reuse, life and all costs before accepting. Execute through existing target collection, timed stages, effect application and resource/damage pipeline (`Z/state_runtime.rs:1215–1398`; `H/itemhandlers/ItemSkillsTemplate.java:60–119`). Include charges/timing in the start draft, with item-reuse state persisted and hydrated.

ItemConsume is a template/count requirement, paid once at accepted start (source `G/model/actor/L2Character.java:1560–1566`); consume lowest matching owned IDs in order. For item-origin casts, prefer the selected instance when it matches. Do **not** also consume a second clicked scroll: ItemSkillsTemplate's fallback consumption applies only to its applicable CAPSULE/SKILL_REDUCE immediate-effect path with no consuming skill (`H/itemhandlers/ItemSkillsTemplate.java:124–148`). Aggregate a multi-skill item's complete cost plan before mutating. Learning consumes required spellbooks through the same inventory planner atomically with SP/rank/receipt; remove `Z/state_learn.rs:51`'s unconditional item rejection only for admitted prerequisites (AD:28).

Reuse has both skill ID and item template/shared-group keys. Key shared_reuse_group when positive, otherwise template ID; timers survive the consumed instance and block a different stack of the same group. Template reuse_delay is already milliseconds, equip_reuse_delay is seconds converted to milliseconds (`G/model/items/L2Item.java:164,193–194`). Preserve source handler checks for item and skill reuse; no universal potion cooldown (`ItemSkillsTemplate.java:60–95,118–119`). Deadline=`start+ceil(ms/100)`; repeated idempotency key returns original receipt/deadline. Fresh admission pauses/rebases, snapshot restore preserves absolute deadlines (new contract consistent with P3 offline policy).

Escape736→skill2013 has hitTime20000ms, isMagic2, itemConsume736×1, SELF/TOWN; source hitCancelTime500 is unused by existing P3 timing (`D/stats/skills/02000-02099.xml:163–187`). A14 fixes destination(126,126). Use static item timing: animation20000ms, launch19600ms, complete20000ms =200 ticks; interrupt cutoff198 ticks under P3 staging. It is interruptible; “interruptible” does not mean invent100% damage cancel when source cast-break is probabilistic. At completion relocate alive caster, cancel movement/auto, recompute AOI and send existing movement/identity baseline; do not call town **respawn** or reset HP/MP/XP/incarnation. Blessed Escape uses its own item_skill/hitTime, not the20s normal-scroll timing (`D/stats/skills/02000-02099.xml:440–462`).

Resurrection737→skill2014 targets PC_BODY and uses the existing first-valid ResurrectOffer; no instant revival or bypass of offer acceptance/expiry/death incarnation. The scroll is already consumed when offered; decline/expiry is not a refund (`D/stats/skills/02000-02099.xml:189–211`; C12; P3:130–134). Potions/elixirs use their actual restore/periodic handlers and reuse; static item heals must not gain an ordinary magic heal shot bonus. ManaPotion delegates to ItemSkills but is config-gated (`H/itemhandlers/ManaPotion.java:27–36`); O1/A14 explicitly enables this product feature, an override to be recorded in rules, not an implicit always-on source default.

**Numbered rule I5 (D13, AD:63):** source supports simultaneous/immediate potion casts (`ItemSkillsTemplate.java:98–115`), while Phase3 has one cast slot. V6 adds an ordered item-cast lane keyed by CastId (same phase5 deadlines/RNG ordering) exactly when `skill.simultaneousCast || ((item.immediateEffect || item.exImmediateEffect) && skill.isStatic)`; the handler first applies its casting guard for non-potion/non-elixir/non-scroll/non-exImmediate items; ordinary escape/resurrection occupies the main slot. It is bounded by inventory/reuse/input admission, fully snapshotted and absent in V5. Do not silently cancel the main cast or reject all potions during casting. Include its full state and maximum admitted occupancy in the C10 gate.

## 9. Persistence, starter materialization and critical item mutations

### 9.1 Character checkpoint and tables

**New contract under A3/A20/D7/D8 (AD:17,34,57–58).** Add `phase4: Option<Phase4Checkpoint>` to `CharacterCheckpoint`; legacy None omits the key entirely from the existing fingerprint, as Phase3 already does (`A/ports.rs:159–207`). A V6 checkpoint requires both Phase3 and Phase4, and carries the item rules hash. Its strict versioned DTO contains:

| DTO field | Persisted meaning / restore rule |
|---|---|
| `schema_version`, `timer_policy_version`, `rules_hash`, `inventory_revision` | Explicit version1, paused-offline policy1, exact epoch bundle, monotone owner revision. Unknown fields/versions fail closed. |
| `instances: Vec<RetainedItem>` | ID-ordered full scalar records from §1.3, including weapon charge bits. Never template/function bodies. |
| `paperdoll` | Slot→instance, cross-checked against each record's location/slot. |
| `auto_shots` | Ordered template set, including enabled mismatched templates under I2. |
| `item_reuse_remainders` | Ordered `(group-or-template,remaining_ticks)`; zero deadlines dropped. |
| `equipment_install_order`, `active_sets` | Provider identities and ordering interleaved with retained Phase3 passive/effect/System sources; retain each set provider’s `shield_skill_active` history (D20, §3.4). |
| `weight`, `max_load`, `weight_penalty_bonus`, `overload_level`, `overloaded`, `expertise`, `weapon_penalty`, `armor_penalty` | Validate recomputable projections; overloaded's historical transition bit is separately retained under I4. |
| `starter_kit_receipt` | Manifest version/hash, source class, request identity and committed materialization revision. |

Do not persist active casts/auto swings into an offline character checkpoint; checkpoint pause retains reuse/effects/resources by the existing Phase3 policy (`Z/phase3_checkpoint.rs:49–95,131–170`). Full **zone snapshot** recovery retains pending main/item lanes, dual plans, absolute deadlines and allocator exactly. Fresh admission, alive or dead, validates inventory and reconstructs the V6 equipment modifier sources of §3.2 from retained equipment and set history regardless of source `stayAfterDeath`; allocate shared install sequences once in retained relative order. Rebuild once, clamp HP/MP/CP/fractions without healing, then publish identity→SkillStateSync→InventorySync. Ordinary timed effect restoration still enforces its source lifecycle; equipment providers never enter that retained-effect validation lane (D16). It must not “re-equip” every item via command callbacks, clear saved charges, replay onStart, or pay equip reuse again. Rejection leaves admission counters unchanged. The item table and checkpoint are two projections of one transaction; mismatch is corruption, never resolved by silently selecting the fuller list.

Migration000010 contract:

| Table / columns | Constraints and interpretation |
|---|---|
| `item_instances(id bigint PK, character_id FK, location, slot smallint NULL, template_id integer, count bigint, enchant bigint, augment_id bigint NULL, charge_bits smallint, bound boolean, created_tick bigint, revision bigint)` | ID/count positive; id≤2^63−1; template valid; enchant0..2^32−1; augment0..2^32−1 and decoded options valid; bound=false; count1 for nonstackable. Slot present iff PAPERDOLL; unique `(character_id,slot)` where equipped. Quest pool derives from template. Tick/revision conversions checked. No arbitrary SA/mana fields: variant is template, shadow/time-limited usage excluded. **D19 (AD:69) amends A20: enchant/augment use signed bigint with CHECK constraints to the full u32 range.** |
| `item_ledger(id identity PK, character_id, zone_id, epoch, tick, revision, op, instance_id, delta jsonb, receipt_ref NULL)` | Append-only typed delta with before/after scalar values, ordered cause facts and per-instance net count delta; unique logical row key `(zone,epoch,tick,character,instance,op,operation_ordinal)`; consumption uses a fixed tick-batch ordinal. Do not require a live-row FK on instance_id: destruction must retain audit history. |
| `characters.phase4_checkpoint jsonb NULL` | Exact strict DTO above. Existing character revision fences the complete scalar/class/skill/item transaction. Legacy NULL is different from explicit empty V6 inventory. |
| `item_id_ranges(zone_id,epoch,range_start bigint,range_end_exclusive bigint,next_reserved bigint,...)` | Durable global nonoverlap per I1. Allocation high watermark survives restarts; all allocator values use signed bigint with checked bounds `0 < range_start ≤ next_reserved ≤ range_end_exclusive < 2^63` (D19). Refuse reservations whose exclusive end would reach2^63; exhaustion never wraps. |
| `pending_item_requests(request_id,character_id,kind,request_fingerprint,manifest_version,manifest_hash,payload,status,materialized_revision,receipt_ref)` | Durable input intent, **not item ownership**. Uniqueness for starter/backfill per character; command idempotency uniqueness for grant. Final manifest contents admitted into the replay before use. |

The pending table may be introduced in000010 and populated by000011. Migration000011 inserts one pending backfill manifest per existing character with stable request identity derived from `(backfill-version,character-id)`; conflict does nothing after validating manifest identity. It never allocates item IDs or inserts item rows (D7). Pin XML selection to the character's base creation class, never a later class-transfer role; unknown/missing kit class fails migration validation instead of granting a generic sword. Record this selection in the manifest for deterministic first admission. Regenerate SeaORM entities through `entities.sh`; add equivalent in-memory adapter constraints (AD:34; `A/ports.rs:216–217`).

### 9.2 One tick, one durable draft

Extend the existing actor gate, not the public event bus: preflight immutable requests/receipts → build one deterministic tick → acknowledge Record8 in JetStream → revision-fenced DB transaction containing checkpoint, item rows, ledger, request completion and outbox → release WS/RPC outputs. Retry the retained record on failure; never rerun enchant/augment RNG. A crash after log ack but before DB commit recovers from the same record, including allocated IDs and result facts; a crash after commit but before visible output returns the frozen receipt (existing gate `A/zone_actor.rs:1–20,768–833`; new Phase4 scope AD:17).

Maintain tick scratch `BTreeMap<(CharacterId,ItemInstanceId),ConsumptionAccumulator>`. Each shot/arrow/skill-item use appends an ordered cause with command/cast/attack ID, consumed count and charge change; at flush, write **one consume row per instance per tick** with the net count delta. Other mutations in the same tick keep their ordered operation facts so an item consumed then destroyed or granted then consumed is reconstructible. Zero net count with changed charge/equip/reuse state still checkpoints. No per-hit database calls; no batching across ticks. Validate inventory capacity/stack/count overflow and all outputs before committing multi-item plans; consume inputs before evaluating the post-operation slot/weight occupancy. Rejection cannot discard an acknowledged RNG-cost-attempt fact (§2).

Item request fingerprints include operation, authenticated character, canonical instance/template IDs, counts, selected slot/recipe/NPC/target binding and request body; omit transport sequence and idempotency key itself. Same key/body returns original receipt; same key/different body rejects before simulation, matching LearnSkill's existing receipt pattern (`A/ports.rs:367–467`; AD:17). No target-character field on zone item intents: session supplies owner. Enchant/RefineResult contains a receipt reference, not a fresh reroll on reconnect.

Outbox subjects are exactly the A20 item subjects; `crystallize` maps to destroyed plus granted facts inside the same receipt, cancel-refine to refined with removal outcome. Equip displacements emit unequipped then equipped facts. DomainEvent variants are API-owned; use operation/outcome/grade/kind labels only in metrics, never template/instance/character IDs (AD:32,34). Audit payload may contain those IDs; metric labels may not.

### 9.3 Starter kits, fixtures, grants, destroy and crystallize

CreateCharacter records a pending-kit request in its existing transaction. First V6 admission materializes it **before visible output**: enumerate `initialEquipment.xml` rows in XML order, allocate nonstackables one each and merge only identical stackable templates/state, then apply every XML equipped flag in row order using §3 rules. Preserve tutorial/teleport items and exact counts even if their UseItem capability is blocked; D12 forbids dropping extras (`D/stats/initialEquipment.xml:4–109`; AD:57,62). Starter/backfill op is carried in every ledger row; retry returns the committed receipt, never a second kit. Fixtures declare whether kit is already materialized, with its matching receipt, preventing accidental automatic additions to a seeded inventory.

Fixture packs seed rows+ledger+reserved ID manifest before admission, never mutate a live actor or rely on host psql (A22). GrantItem authenticates owner and checks exact `AUTH_DEV_TOKENS=1`; otherwise FAILED_PRECONDITION. Validate template capability/count/capacity and request fingerprint before enqueue. An online zone resolves it normally; an out-of-world character uses the D7 pending-request/admission materialization mechanism and returns success only after zone commit. If no admissible zone exists, UNAVAILABLE; no DB-only grant, success receipt, or orphan owned item. This is a dev path in production code, not a second authoritative inventory service (AD:29,36,57).

DestroyItem validates destroyability, owned quantity and casting binding, unequips if needed, then deletes/decrements without refund. **CrystallizeItem is distinct** (D9, AD:59), consumes one admitted equipment instance, and refunds its **full** instance crystal count (§4), not enchant-failure's reduced refund. Require Crystallize skill248: rank≥1 for D,2 C,3 B,4 A,5 S/S80/S84; positive base crystal count/grade and crystallizable flag; reject hero/shadow/time-limited items. Unequip before destroy; merge/create refund atomically. Source `G/model/skills/CommonSkill.java:47`; `G/network/clientpackets/RequestCrystallizeItem.java:79–118,124–186`. The source logs unusual race/class when skill missing but has no extra universal dwarf-only successful-use guard; check the actual skill. These nonstackable targets make source count-clamping irrelevant; Nightfall rejects excess requested quantities rather than hiding client mistakes.

## 10. Replay, codec and digest coverage

### 10.1 Strict V6 boundary

**New contract under A4 (AD:18).** `Snapshot.version=10` requires Phase3+Phase4, `Record.schema=8`, `StateDigestVersion::BinaryV6`; mismatched/missing sidecars/rules reject. Snapshot≤9 and Record≤7 preserve old codecs, validations and digests; never populate default gear while replaying them. Extend `Z/state.rs:405–460`, `R/codec.rs:637–650`, typed snapshot registry and replay admission `version_two`; do not deserialize arbitrary unknown fields into new defaults. `PHASE4_ENABLED` mirrors `apps/api/src/zone_runtime.rs:35–78`: unset→V6 for production, explicit0→V5,1→V6, other→startup error; recorded version always wins during recovery. V6 requires the Phase3 bundle regardless of a contradictory Phase3 environment switch; refuse contradiction explicitly. Simulation fixtures select V6 deliberately for new C16 runs, while historical binary recordings stay old.

Use separate tag spaces (existing `R/codec.rs:155–226,435–506`). New wire payloads can use strict integer-only canonical DTO bytes in replay, following the existing Phase3 codec pattern, but malformed/unknown enums, unordered duplicate keys, dangling item references and impossible results must reject. Do not embed public protobuf's default-zero semantics as a loophole in canonical state validation.

| Codec allocation | Meaning |
|---|---|
| `PbCommandKind`20..29 | Equip, Unequip, Use, Destroy, Enchant, ToggleAutoShot, Refine, CancelRefine, ExchangeSA, Crystallize respectively. |
| `PbCommandKind`30 | Authenticated internal GrantItem/pending materialization command; client cannot construct it over WS. |
| `PbCommandKind`31 | Recorded allocator-range extension, if an epoch exhausts its range; authenticated API input only. |
| `PbPlayerLoad`10 | Optional strict Phase4 admission DTO, required in V6; absent in all legacy records. Includes loaded inventory, pending manifest and imported allocator/receipt facts, not a DB pointer. |
| `PbOutput`32..38 | InventorySync, InventoryChanged, EquipmentChanged, WeightChanged, PenaltyChanged, EnchantResult, RefineResult. |
| `PbOutput`39 | Owner ItemOperationResult for other accepted mutations/frozen receipts (§11). |
| `PbOutput`40 | Private Phase4Projection: checkpoint, ledger plans, receipts, consumption aggregates and evidence facts; not a WS event. |
| `PbOutput`41 | V6-only Phase4EnrichedEvent: existing EntitySpawn/StatsChanged events with Phase4 payloads, including nested EffectiveStats enrichments; strict carrier below (D26). |

Tag28 in PbOutput remains unused/reserved; do not confuse these numbers with WorldEvent26+ or ClientMessage19+ (AD:18; `R/codec.rs:439–506`). D23 (AD:73) approves the **wire** map in §11; C freezes it in the first proto commit, reviewed by Z; replay tag numbers above are this design's allocation, never reused after recording fixtures.

**D26 enriched-event carrier (AD:76).** Tag41 stores the complete event as canonical integer-only DTO bytes with explicit Phase4 sidecar presence, using strict decode/re-encode byte equality and rejecting unknown fields, duplicate keys, malformed enums and noncanonical ordering. Its permitted outer variants are exactly `EntitySpawn` and `StatsChanged`; EffectiveStats is their nested projection where applicable, never a new top-level WorldEvent. EntitySpawn requires a present Phase4 visible-equipment sidecar (an empty list is valid for unequipped/no public gear); StatsChanged requires a present owner Phase4 sidecar containing weight, penalties and inventory revision, plus all five Phase4 EffectiveStats fields from §11.2 when its effective-stat projection is present. A present nested effective-stat projection must carry its complete Phase4 extension, including explicit zero values where valid; do not infer sidecar presence from nonzero scalars. Existing required identity/combat/class/Phase3 sidecars remain required for the corresponding entity/event shape. Missing/mismatched extensions, a nested Phase4 extension without its parent Phase4 sidecar, and any other outer variant reject.

Select tag41 before older event-carrier dispatch whenever either permitted event has a Phase4 sidecar; require Record8/V6 context. Reject tag41 under Record≤7 and reject Phase4 payloads smuggled through older carriers. When Phase4 is absent, preserve the exact existing tag selection and serialization, including typed legacy, Phase2 and Phase3Projection carriers; do not widen their historical acceptance grammar. Existing carrier branches and typed field limits are `R/codec.rs:520–538,1039–1055,1173–1198,1821–1826,2012–2037`. Tag40 remains private checkpoint/ledger/evidence output and cannot substitute for tag41. Live WS projection and replay projection must preserve identical public/owner fields; RPC EffectiveStats uses the same enrichment projection without inventing a replay output for the RPC itself.

### 10.2 Binary digest and evidence

V6 extends the existing binary entity/Phase3 hash with a domain-separated Phase4 block. Encode fixed-width tagged integers, explicit Option markers and lengths, ordered maps/sets and source XML ordinals; hash rule content/hash, allocator range/next, all item scalars/counts/location/charges, paperdoll, toggles, reuse deadlines, active-set history, provider identities/install ordering, weight/max/bonus/overload boolean and ranks, expertise, starter receipts/inventory revision, every facing vector, pending item lanes/dual plans/prepaid charge bindings and deterministic item result facts. Include RNG seed/counter/position through the existing state hash. Cache layout, DB surrogate ledger IDs, transport bytes, wall time, metric counters and regenerated lookup indexes are excluded. Changing any future-behavior field must change the digest; absence of Phase4 never changes a V5 digest (`Z/state_digest.rs:18–114`; AD:18,22,35).

Record facts are authoritative **inputs to verification**, not a client-provided claim of correctness. Replay recomputes and compares them before exporting: `(tick,phase,ordinal,owner,instance,template,before/after,count delta,cause)`; charge attempt/reduction/consumption/discharge and weapon binding; arrow/MP payment; equip displaced slots/provider sequence; before/after grade/weight penalties and overloaded transition; enchant group/scroll/base chance/bonus/final chance/raw draw/residue/outcome/old-new enchant/refund; refine stone/rows/category/options/draws/packing/costs; SA recipe/ingredients/output/carry fields; shield eligibility/facing/rate/perfect+normal draw and outcome; blow/lethal geometry/rates/draws; item cast consumed IDs/reuse. Persist only draws actually made, with explicit skipped-site reasons in export derived from branch facts. C14/C15 consume these facts to compare the independent oracle, not to accept any successful animation (AD:35).

## 11. Additive wire and API mapping

**Binding D23 schema allocation for the C-owned proto commit, reviewed by Z (AD:16,59,73).** Types live in `nightfall/v1/items.proto`; import into existing envelopes. All integer quantities remain integers. Preserve old fields unchanged, including legacy Position encoding; new inventory coordinates are unnecessary. `u32/u64/i64/bool/string` below mean protobuf uint32/uint64/int64/bool/string. ID bytes follow the established entity/character UUID encoding; item IDs use u64. Tag/type pairs below are field-by-field allocation, not existing generated code.

### 11.1 Intent payloads

Every request has `string idempotency_key=1`; owner/session/command seq remains in ClientMessage. A target binding uses `bytes entity_id=1,u32 incarnation=2`; missing/zero invalid where needed. ClientMessage's existing cast/resurrect ends18 (`W:32–43`).

| Envelope tag / request | Payload after key1 |
|---|---|
|19 EquipItem|`u64 instance_id=2; EquipSlot preferred_slot=3` (UNSPECIFIED uses §3 pair policy)|
|20 UnequipItem|`EquipSlot slot=2`|
|21 UseItem|`u64 instance_id=2; ItemTarget target=3` (absent for self/no-target)|
|22 DestroyItem|`u64 instance_id=2; u64 count=3`|
|23 EnchantItem|`u64 scroll_instance_id=2; u64 target_instance_id=3; optional u64 support_instance_id=4`|
|24 ToggleAutoShot|`u32 template_id=2; bool enabled=3`|
|25 RefineItem|`ItemTarget npc=2; u64 target_instance_id=3; u64 stone_instance_id=4` (gem plan server-selected)|
|26 CancelRefine|`ItemTarget npc=2; u64 target_instance_id=3`|
|27 ExchangeSpecialAbility|`ItemTarget npc=2; u64 target_instance_id=3; u64 recipe_id=4`|
|28 CrystallizeItem|`u64 instance_id=2` (nonstackable equipment only)|

No client supplies count-after, damage, chance, rolled options, desired enchant, gem fee, output template, grade rank or allocator ID. Bool false is a valid toggle; absent target/slot follows only documented defaults. Enchant support must be explicitly admitted in the catalogue; omitted support is not inferred from bags.

`RejectReason` append25 ITEM_NOT_OWNED,26 ITEM_NOT_USABLE,27 INVALID_EQUIP_SLOT,28 INSUFFICIENT_ITEMS,29 INVENTORY_FULL,30 ITEM_GRADE_MISMATCH,31 ITEM_REUSE,32 INVALID_ENCHANT,33 INVALID_REFINEMENT,34 INVALID_RECIPE,35 ITEM_OPERATION_CONFLICT,36 ITEM_CAPABILITY_DEFERRED,37 CRYSTALLIZE_LEVEL_TOO_LOW. Prefix every value `REJECT_REASON_`; preserve1..24 (`W:135–160`). Reuse existing INVALID_TARGET/OUT_OF_RANGE/dead/incapacitated reasons where semantically exact; do not mask ownership errors as missing skill or invent random success/failure for rejected mutations. Ordinary idempotency-key reuse uses the existing conflict transport contract and no mutation.

### 11.2 Shared item records and event fields

`ItemGrade`: UNSPECIFIED0, NG1,D2,C3,B4,A5,S6,S807,S848; prefix `ITEM_GRADE_`. These are wire values, **not** the source's grade0..7, requiring an explicit conversion table. `EquipSlot`: UNSPECIFIED0, then §3's source slot+1 through25, prefix `EQUIP_SLOT_`. `ItemLocation`: UNSPECIFIED0,INVENTORY1,PAPERDOLL2, prefix `ITEM_LOCATION_`. Unknown values reject. Option-bearing fields use presence, not sentinel invalid IDs.

| Shared message | Fields |
|---|---|
|`OwnedItem`|`u64 instance_id=1; u32 template_id=2; u64 count=3; u32 enchant=4; optional u32 augment_id=5; ItemLocation location=6; EquipSlot slot=7; u32 charge_bits=8; u64 revision=9`|
|`PaperdollEntry`|`EquipSlot slot=1; u64 instance_id=2`|
|`VisibleEquipment`|`EquipSlot slot=1; u32 template_id=2; u32 glow_tier=3`; template0 tombstone allowed in delta only|
|`ItemReuseState`|`ItemReuseKind kind=1; u32 key=2; u64 until_tick=3`; kinds TEMPLATE1/SHARED_GROUP2, prefixed `ITEM_REUSE_KIND_`|
|`ItemWeightState`|`i64 weight=1; i64 max_load=2; i64 penalty_bonus=3; i64 per_mille=4`|
|`ItemPenaltyState`|`u32 expertise=1; u32 weapon_level=2; u32 armor_level=3; u32 overload_level=4; bool overloaded=5`|
|`ItemRefund`|`u32 template_id=1; u64 count=2; u64 instance_id=3`|

Every owner event below starts `bytes entity_id=1; u32 incarnation=2; u64 tick=3; u64 inventory_revision=4`. For EquipmentChanged public version, field4 is **absent from its schema** (reserved4); use its public tick/incarnation fence. This prevents leaking owner revision/hidden operations. WorldEvent currently ends25 (`W:163–189`).

| WorldEvent tag / message | Remaining fields and audience |
|---|---|
|26 InventorySync|`u64 baseline_tick=5; bool baseline=6; repeated OwnedItem items=7; repeated PaperdollEntry paperdoll=8; repeated u32 auto_shot_templates=9; ItemWeightState weight=10; ItemPenaltyState penalties=11; repeated ItemReuseState reuse=12; bytes rules_hash=13; u32 ordinary_capacity=14; u32 quest_capacity=15`; owner only, full replacement|
|27 InventoryChanged|`u64 previous_revision=5; repeated OwnedItem added=6; repeated u64 removed_ids=7; repeated OwnedItem updated=8; repeated u32 auto_shot_templates=9; repeated ItemReuseState reuse=10`; owner only, toggle/reuse sets are full replacement projections|
|28 EquipmentChanged|common1..3, reserved4, `repeated VisibleEquipment visible=5`; owner and observers, changed slots including empties|
|29 WeightChanged|`ItemWeightState weight=5`; owner only|
|30 PenaltyChanged|`ItemPenaltyState penalties=5`; owner only|
|31 EnchantResult|`string receipt_id=5; u64 target_instance_id=6; u32 old_enchant=7; u32 new_enchant=8; EnchantOutcome outcome=9; repeated ItemRefund refunds=10; u32 announcement_tier=11`; owner only, no public exact enchant|
|32 RefineResult|`string receipt_id=5; u64 target_instance_id=6; optional u32 augment_id=7; RefineOutcome outcome=8`; owner only|
|33 ItemOperationResult|`string receipt_id=5; ItemOperation operation=6; repeated u64 affected_instance_ids=7; optional u64 recipe_id=8`; owner only; SA/grant/destroy/crystallize/equip/unequip/toggle/use acceptance receipt|

Outcome enums reserve0; EnchantOutcome SUCCESS1,SAFE_FAILED2,BLESSED_RESET3,DESTROYED4,SUCCESS_NO_INCREMENT5. RefineOutcome APPLIED1,REMOVED2. Prefix names with their enum name. ItemOperation ordinals are EQUIP1,UNEQUIP2,USE3,DESTROY4,ENCHANT5,TOGGLE_AUTO_SHOT6,REFINE7,CANCEL_REFINE8,EXCHANGE_SA9,CRYSTALLIZE10,GRANT11,STARTER12,BACKFILL13. Results describe committed plans; “success no increment” is required by §4's source base-zero quirk. Detailed chances/raw draws remain replay evidence, not a requirement for ordinary UI.

Add `EntitySpawn.repeated VisibleEquipment visible_equipment=29` only, never owned records (`W:200–233`). Add owner `StatsChanged.ItemWeightState item_weight=24; ItemPenaltyState item_penalties=25; u64 inventory_revision=26` (`W:320–343`), with same revision as InventorySync/Changed. Add `EffectiveStats.i64 shield_def_q=11; i64 shield_rate_q=12; u32 shield_angle_degrees=13; i64 physical_reach_millitiles=14; u32 physical_angle_degrees=15` (`P:381–392`). Add `Character.u64 inventory_revision=15; bytes item_rules_hash=16`, both absent/default for legacy (`P:132–147`). Full inventory is obtained via the new RPC, not repeated in every character listing.

The existing events carrying these Phase4 enrichments use PbOutput41 under §10.1, including nested EffectiveStats; no new public event tag is allocated for EffectiveStats. The D23 intent/result-event map is unconditional (AD:73).

### 11.3 RPCs, hydration and version visibility

Append GetItemCatalog, GetInventory, GrantItem after GetSkillCatalog. Catalogue request `optional bytes known_rules_hash=1`; response `u32 data_version=1` fixed2, `bytes rules_hash=2`, `repeated ItemDefinition items=3`, `repeated SaRecipe recipes=4`, `repeated ItemCapability capabilities=5`. Each ItemDefinition has `u32 template_id=1; string name=2; ItemKind kind=3; ItemGrade grade=4; u64 bodypart_mask=5; u32 weight=6; bool stackable=7; bool usable=8; repeated string blockers=9; string icon_key=10; repeated ItemStatDescription stats=11; repeated ItemSkillReference skills=12`. Stats encode `string stat=1; string operation=2; i64 numerator=3; i64 denominator=4; u32 order=5`; skills `u32 id=1; u32 rank=2; ItemSkillSource source=3`. SaRecipe has `u64 recipe_id=1; u32 base_template=2; repeated ItemIngredient ingredients=3; u32 product_template=4; bool maintain_enchantment=5`, ingredient `u32 template=1; u64 count=2`. ItemCapability has `u32 template_id=1; repeated string blockers=2`. `ItemKind` uses UNSPECIFIED0/WEAPON1/ARMOR2/ETC3 with prefix `ITEM_KIND_`; `ItemSkillSource` uses UNSPECIFIED0/ITEM1/ENCHANT_FOUR2/SET3/AUGMENT4 with prefix `ITEM_SKILL_SOURCE_`. Unknown values reject. Catalogue is informational, not trusted command input. Unsupported bundle returns UNAVAILABLE like GetSkillCatalog (AD:16,21).

GetInventory request `bytes character_id=1`; response `InventorySync inventory=1`. GetInventory authenticates ownership and returns FAILED_PRECONDITION while the character is admitted in a zone. It returns UNAVAILABLE when the required zone or catalogue port is absent. Only out-of-world retained state is returned, from revision-consistent checkpoint/item projections. D15 (AD:65): the live baseline is InventorySync over the WebSocket; never serve stale DB rows as a live baseline. GrantItem request `bytes character_id=1; u32 template_id=2; u64 count=3; string idempotency_key=4`; response `ItemOperationResult result=1; InventorySync inventory=2`. Do not accept requested enchant/augment through this path; richer fixture state is seeded through A22.

On initial admission/replacement/late observer AOI, finish all identity spawns first; send SkillStateSync then InventorySync to owner before incremental item results. Observers receive only visible paperdoll in identity and deltas, no owner InventorySync/Changed/Weight/Penalty/results. Hydration and deltas use the same incarnation/tick/revision checks (§1.5), and client predicates use only received owner state (C11). No SubscribeInventory, new socket, side actor or speculative count decrement (AD:16,30).

## 12. Independent oracle, regression and acceptance plan

### 12.1 Python Fraction vectors

These are **required implementation tests**, not tests claimed to exist/pass in this design-only change. Extend the independent Python oracle under A7 (AD:21), reading pinned XML/Java-derived constants through a separately reviewed parser. Do not call Rust, copy recorded expected results, or serialize binary floating-point numbers. Emit exact numerator/denominator, getter truncations, final Q/wire values, source file:line and every expected draw site/bound/residue. The replay exporter supplies the random residues; the oracle computes branch outcomes independently.

Minimum effective-stat matrix is24 named vectors, with additional boundary vectors below. Select concrete templates by stable minimum template ID satisfying the predicate in the pinned catalogue, record the ID and source lines in the generated fixture, and fail generation if no eligible template exists; never replace a missing type with a sword:

| Vector IDs | Required input cases |
|---|---|
|01–08|SWORD,BLUNT,DAGGER,BOW,DUAL,DUALFIST,POLE and unequipped class FIST, each at e0; source class/level chosen explicitly with sufficient expertise and no learned equipment mastery so the isolated fold is visible.|
|09–14|D/C/B/A/S/S80 weapon e4; include magic weapon and lrhand predicates across these rows; assert correct P.Atk/M.Atk row and doubled increment beyond3.|
|15–18|S84 weapon e6, onepiece armour e4, shield e6, jewelry e6; include EnchantHP table and no accidental P.Def subtraction for shield.|
|19–21|Complete heavy/light/robe sets with exact set skills; one case all required pieces e6, one loses one piece, one enchanted weapon with enchant4_skill.|
|22–24|Human Mystic kit, Orc Fighter kit, Dwarf kit; exact starters, weapon kind/speed, slot subtraction, MP bonuses, total weight and capacity.|

Separate required vector families (citations in §§3–8 are the authority):

- Every FuncEnchant grade/bodypart branch, e0/3/4/19/20 and source cap, enchantHP first/last/out-of-table index, onepiece3/2, jewelry and shield distinction. P.Def per individual slot/fullarmor; M.Def with deliberately unequal class RFINGER/LFINGER bases proving RFINGER duplication. Install/remove/reinstall source sequence, same-priority set attribute interaction, clamp-no-heal and active-provider overlap.
- Every scroll/group/level, safe+3/fullarmor+4, current `maxEnchant−1/maxEnchant/maxEnchant+1`, zero/certain probabilities, blessed+support incompatibility, bonus-only base-zero success. Exhaust all100000 residues for representative rational chance and compare quantized source inequality; include zero/certain draws. Normal failure crystal refund for e0/3/4 and large e, exact division/subtraction order; crystallization full refund separately. Enumerate weighted modulo bias rather than claim continuous-distribution identity.
- MaxLoad for every admitted CON row and weightLimit modifier; all four threshold boundaries using the actual integer formula; exact `weight=maxLoad` and ±1, bonus1, maxLoad≤0 and I4 transition history. Grade expertise0..7, each equipped slot, max gap/clamp4 and lower-grade weapon→armour branch. Equipment-derived penalty sources4270/6209/6213 apply once under D16, independently of source stayAfterDeath; lifecycle coverage includes every equipment provider below.
- All physical/magical/heal shot caller shapes; manual/auto, ordinary+blessed precedence, wrong-grade persistent toggle, reduction-draw failure, all-miss retention, dual/pole shared charge, clear-on-unequip, canceled cast, multi-target no double charge. Shield no eligible item/SIGIL/rear/rate-zero skip, front angle equality, bow13/10, perfect residues90/91/99 at default10, ordinary draw equality and perfect early-return skips.
- Bow matching arrows/S-plus grade, empty/refill lhand, MP reduction, miss still consuming arrow, speed-dependent reuse, integer damage before distance factor. Dual both miss/each single hit/both hit with stored second resolution; pole max count boundary and85% then divide23/20. Dagger front/side/back, blow cap/rounding and independent lethal full/half draw order.
- Weapon augmentation each stone tier/level/mage/warrior combination and accessory category55/35/9/1; enumerate all10000 residues for each row selector/category, descending-tail boundaries, no non-retail draws. Packed unsigned options≥2^31, install active/passive/trigger effects, cancel cost boundaries1720/2452/1746/2160/2824. Every admitted SA recipe's full ingredient sum and enchant/augmentation transfer. Soul crystal stage0..18 source transitions with no runtime absorb RNG.
- Normal/Blessed Escape distinct timing; resurrection offer/decline/expiry; itemConsume once; shared reuse across multiple stack identities; item-lane and main-lane casts due same tick; regeneration/status interactions and item-consuming learned skill. Dormant unsupported starter extras reject without consuming.

**D24 FIST boundary vectors** extend vector08 and assert both radius and spread bound from §5.4; equipped→unequipped and level-change paths must produce the same values. For each row assert offset endpoints `−r,+r` at residues0 and2r, and the equipped-template radius when a weapon is restored.

| Level | `5+isqrt(level)` radius | `2r+1` draw bound |
|---:|---:|---:|
|1|6|13|
|3|6|13|
|4|7|15|
|8|7|15|
|9|8|17|
|15|8|17|
|16|9|19|

**D25 pole boundary vectors** isolate each recipient's caller rounding (§5.4), using exact rational formula inputs; retain the normal RNG skip rules for misses/perfect blocks.

| Formula result | Secondary attackpercent | Truncated base | Required final damage |
|---|---|---:|---:|
|499/10 (49.9)|85|49|41 (single final truncation would incorrectly give42)|
|999/50 (19.98)|85|19|16|
|20|85|20|17|
|1001/50 (20.02)|85|20|17|
|1 (perfect block)|85|1|0 (no minimum reapplied)|
|499/10|1700/23 (85 divided by23/20)|49|36|

Also assert primary49.9→49 and primary perfect-block1→1, and miss-first-secondary→next-secondary percentage1700/23 with no shield/crit/spread draws for the miss. Rejected/ineligible candidates do not advance that percentage. These tables are specified expected vectors, not evidence of tests executed in this revision.

### 12.2 Rust unit/property, durability and wire targets

**Cross-worker contract (D14/D18, AD:64,68).** E4 combat integration (shots/shield/positional/bows/poles/blows and their shared combat/RNG callers) is Z-owned and C-reviewed. E2.6 depends only on reviewed E3.1 adapters plus E3.3's published pending-kit/materialization interface and isolated adapter tests. E3.3's live Done When follows E2.6 integration; it is not a prerequisite that blocks E2.6.

Unit tests cover every table above and every deterministic rejection with unchanged inventory/allocator/RNG, except documented randomized-cost attempt draws. Proptests generate legal inventories/command sequences and assert: positive counts; identity never reused; paperdoll single occupancy; owned sum preserved by equip; consume/grant ledger conserves counts; destroy never creates crystals; atomic enchant/refine/exchange rollback; charges never provide unpaid duplicate bundles; rebuilt sheets equal full fresh fold **given retained history**; no heal on removal; deterministic record/snapshot round trips; replay from any tick produces identical next state/output/RNG. Include checked overflow and malformed restore inputs, not only realistic small values (AD:15,17–22).

**D16 lifecycle triplet:** separately test (1) death with retained equipment, (2) checkpoint restore/admission while dead, and (3) resurrection. In every case assert item funcs, set/item skills, augmentation passive providers, penalties4270/6209/6213 and equipment-dependent masteries each contribute exactly once even with source `stayAfterDeath=false`. Cover learned/equipment provider overlap and persisted shield history. Assert no onStart replay, no equipment-induced heal and no duplicate install sequence; death/resurrection retain identities and dead admission allocates each retained provider once in relative order. Any explicit resurrection resource restoration is checked separately from the equipment rebuild. Include active item/augment cast effects with both lifecycle flags to prove ordinary removal/retention still works, and retain V5 rejection tests for System/passive/misflagged ordinary effects and nonpersistent effects on dead admission.

**D15 first proto/API contract commit:** include these three owner-authenticated GetInventory cases, with no live DB-baseline fallback:

| Case | Required result |
|---|---|
| Character admitted in an available zone, ports present | FAILED_PRECONDITION; live baseline comes only from WebSocket InventorySync. |
| Required zone port absent or required catalogue port absent (exercise each independently) | UNAVAILABLE; no checkpoint fallback. |
| Character out of world, ports present | Return only retained inventory from checkpoint/item projections in one revision-consistent transaction; assert matching revision, paperdoll, weight and penalties. |

Critical adapter tests must inject failure before append, after append, before DB commit, after DB commit/before output, and after socket replacement. Assert one receipt, one RNG result, one allocator advancement, no visible pre-commit output; revision conflict fences the actor. Backfill migration run twice, first admission crash/retry, CreateCharacter retry, fixture plus kit marker, two epochs reserving ranges, range exhaustion, and concurrent identical/different-key GrantItem all need real disposable-Postgres coverage. JetStream replay/checkpoint tests require owned services under worker-common rules; they are not inferred from unit passes.

Codec tests round-trip every new command/output/PlayerLoad, reject V6 data in V5 and missing V6 sidecars, mutate each digest field, permute insertion order without changing canonical bytes, corrupt owner/slot/option/hash/ID bounds, and reject unknown enum/tags under the strict domain conversion. Golden wire tests prove no old tag moves and no observer exact enchant/count/augment/charge leak. Hydration tests include replacement, late observer, reconnect after enchant destruction, matching tick with different event types, stale incarnation, duplicate receipt, gap resync and public equipment tombstones. Apply all same-revision owner event components; seeing InventoryChanged first must not suppress its WeightChanged/PenaltyChanged/StatsChanged siblings.

**D26 carrier tests:** round-trip PbOutput41 for equipped EntitySpawn hydration, owner StatsChanged with weight/penalties/revision, and StatsChanged with enriched EffectiveStats using nondefault shield/reach/angle values. For each, compare canonical replay output bytes before/after decode and live-versus-replayed serialized WS output bytes, asserting all enrichment fields survive. Include present-empty spawn equipment and valid zero-valued extensions. Reject missing/partial sidecars, wrong outer variants, unknown/duplicate fields, noncanonical bytes, tag41 under Record≤7 and enriched payloads in older carriers. Phase4-absent fixtures must retain their exact old carrier tags/output bytes for typed legacy, Phase2 and Phase3Projection paths; historical recordings remain untouched.

Run `gen_items.py --check`, the Fraction oracle/vector check, domain unit/property tests, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, historical replay-check, proto/client regeneration checks and SOURCES scan for every new deterministic module. Full implementation gate also measures C10 worst-case encoded/decoded envelope (§1.4), pathological per-tick consumption/result volume under deterministic admission limits (P3 §4.5), and tick p99<20ms for8 clients with equipment swaps/autoshots/one enchant per client (AD:33). Catalogue rules/options must be included in both snapshot and tick record admission sizing, never omitted to pass a benchmark.

### 12.3 Phase1–3 expectations under real starter gear

Historical `.nfr` bytes do not change. **Fresh production-policy scenario runs select V6 and materialize real kits**; update oracle evidence/header citations, not allowlists, to keep C15/C16. Current bare class bases and universal weapon are visible at `packages/data/classes/human_fighter.toml:19–38`, `human_mystic.toml:19–38`, `Z/effective_stats.rs:69–138`. Pinned kit functions: wand6 P.Atk5/M.Atk7/crit4/accuracy19/4/speed379 (`D/stats/items/00000-00099.xml:110–134`); fighter2369 P.Atk6/M.Atk5/crit8/speed379 (`D/stats/items/02300-02399.xml:844–866`); Orc2368 P.Atk7/M.Atk5/crit4/accuracy19/4/speed325 (`:819–842`); Dwarf2370 P.Atk6/M.Atk5/crit4/accuracy19/4/speed379 (`:868–891`). Exact kits are per-class XML, not the shorthand archetype rule.

Concrete isolated re-derivations (no learned passives, e0, no shots/effects): human fighter P.Def base80→`80−31−18+33+20=84`; human mystic54→`54−15−8+17+10=58`. Multiply by `(level+89)/100` after slot subtraction. Mystic maxMP gains19+10=29 after the base MEN fold. Source armour `D/stats/items/01100-01199.xml:593–624`, `00400-00499.xml:427–446,1152–1171`; getter/fold `G/model/stats/functions/formulas/FuncPDefMod.java:44–70`; `Z/stat_functions.rs:324–341`. Mystic M.Atk base5→7 gives `base×(121/100)^2×((level+89)/100)^2` with INT41 (`packages/data/tables/stat_bonus.toml:97–118`). Independent Fraction arithmetic yields:

| Level | Fighter P.Def Q old→kit | Mystic P.Def Q old→kit | Mystic M.Atk Q old→wand (truncate at Q projection) |
|---:|---:|---:|---:|
|1|72000000→75600000|48600000→52200000|5929605→8301447|
|20|87200000→91560000|58860000→63220000|8697486→12176480|
|40|103200000→108360000|69660000→74820000|12182044→17054861|

These isolated vectors do not pretend to be the complete fixture's final stat sheet: fixture passives/racial traits/masteries must be folded too. Magic damage must rerun its exact root/getter chain; multiplying the final rounded old damage by7/5 is wrong. V6 also activates positional bonuses and new conditional RNG, so even unchanged gear may alter actual replay outcomes.

| Existing scenarios / numbers | Required V6 derivation and retained assertions |
|---|---|
|`1-kill-one-monster` first hit10s, kill30s; `1-target-attack`, `1-npc-respawn`, `1-late-entry-*`, `1-chase-leash`, `1-social-aggro-*`|Keltir44HP/253speed/28XP are fixture constants (`packages/data/npcs/keltir.toml:11–20`), not changed by kits. Recompute player hit/crit/spread/position and NPC regen sequence, chase/reach, swing deadlines and kill bound from the exact fixture kit. Remove the informal “~7 swings/~3s” justification at `1-kill-one-monster.nfs:23–24`; use the independent seeded/allowance calculation, keeping at least the required safety factor. No guessed new fixed kill count.|
|`1-die-respawn`, `1-delevel`, `3-death-penalty`, PvP/karma/resurrection pairs|P.Def change reduces incoming physical damage; wand magic/Orc dualfist changes outgoing damage and shot-free RNG. Recompute death/time budgets and XP event sequence with the fixture's actual class/gear. Death-penalty caps/XP rules, offer acceptance/recovery and respawn safe point remain the existing contracts; armour must not introduce a heal on reconnect.|
|`2-token-milestone20/40`, `2-token-jump19-40`|Human sword379×DEX110/100 truncates417, so the12-tick interval stays; sentinel253 gives20ticks. Headers'64 player/3 sentinel attempt allowances and300s waits remain **test policy subject to the V6 oracle proof**, not guaranteed RNG limits (`2-token-milestone20.nfs:10–21`; `2-token-milestone40.nfs:10–21`). Recheck P.Atk10000 sentinel still kills through real P.Def and that the hit-floor branch still applies. XP rewards10746/65888 and losses14329/87851 stay governed by the same progression source (`:3–8`), not adjusted to speed up the scenario.|
|`2-racial-traits`, create/class-transfer/reconnect/backfill scenarios|Elf4580 and Human3950 movement values and Human29XP remain if kit weight stays below penalty and no kit speed skill changes the fold (`2-racial-traits.nfs:2–5,17,25,35`). Independently prove weight/rank0 with every kit extra. Orc/Dwarf weapon-kind/crit/speed expectations change as above; identity/class/token assertions stay exact. Add inventory hydration/kit-once evidence around the existing reconnect flows.|
|`3-cast-skill`, observer/reconnect/aggro/debuff variants|Skill1177 oracle now uses wand M.Atk7 and real armour/maxMP; SP9760 and skill-rank assertions stay (`3-cast-skill.nfs:18–38`). Keep MP **cost** exact while changing maxMP; recalculate damage/root/target retaliation and new shield-free skip draw schedule. No starter cast-speed function means timing is unchanged only after verifying inherited passives and penalties.|
|`3-learn-skill`, rejected variants|SP9762 remains (`3-learn-skill.nfs:33–44`); existing rejection fixtures must still lack their blocked prerequisite. Item-consuming learns now succeed only with admitted books; do not silently reinterpret a historical fixture as owning one. Add the new spellbook scenario separately.|
|`3-regen`, buff expiry/cast-low-MP variants|Initial HP/MP/CP1 stays (`3-regen.nfs:18–26`); kit materialization must not refill them. Base CON/MEN and regen cadence stay, while maxMP+29 changes saturation timing. Re-evaluate moving/standing pulses and effect-removal clamps against real maxima. Existing9/27s pulse waits are cadence-derived (`:21–29`), not an excuse to skip the new cap oracle.|

Harness work must generate a machine-readable before/after expectation report for **every** Phase1–3 scenario/variant, including unchanged entries and why, from explicit role fixtures. This design supplies the changed source operands and representative exact numbers; it does not invent final scenario outcomes before the V6 engine/export/oracle exist. C16 required-live runs remain zero-skips (AD:25,31).

### 12.4 Phase4 scenarios and acceptance evidence

Implement the complete A17 list (`AD:31`) with explicit seeded inventory/paperdoll/allocator manifests. Each client-visible story ends in a Simulation scenario: starter once, swap/fists, set/enchant6, shield normal/perfect, bows empty+refill+miss, grade/weight boundaries, enchant success/failure branches, physical/magic shot effects, potion/reuse, interruptible escape, resurrection paired players, augment/cancel, SA exchange, spellbook learn, dev grant denied. Add CrystallizeItem and true DestroyItem no-refund coverage after D9. Clients assert received InventorySync/Changed/operation result/paperdoll/stats; C14 export and C15 oracle assert exact draws/formula/ledger/refunds. Enchant failure is not an allowlisted server rejection. Reconnect each durable operation and prove the same receipt/state without another consume or roll.

## 13. Closed architect dispositions

D1–D26 in `AD:50–76` are incorporated; no listed question remains awaiting approval. Closed under D1–D13: Q1 identity ranges (I1); Q2 migration/zone authority (D7, §9); Q3 per-caller shots/shield (D1/D2); Q4 retail versus offsets/closure/currency (D3/D5/D6/D11); Q5 snapshot arithmetic plus measurement gate (D4); Q6 dual scheduling (I3); Q7 shot toggle/bundles (I2); Q9 overload transition (I4); Q10 glow placeholder (I6); Q13 simultaneous item lane (I5). Blacksmith tile is(124,128) under D10; all starter XML extras retained under D12.

| Closed question | Binding disposition and implementation contract |
|---|---|
| Q8 storage widths | D19: enchant/augment signed bigint with u32 CHECK constraints; allocator exclusive endpoint strictly <2^63, signed bigint throughout (§§1.1,9.1). |
| Q11 set shield history | D20: per-provider `shield_skill_active` transitions in slot-mutation order and survives checkpoint reconstruction (§3.4). |
| Q12 integer angles | D21: certified rational cosine intervals/exact special boundaries, hashed in Phase4Rules; uncertifiable angles are named skill blockers (§§1.2,5.3). |
| Q14 fixture tax | D22: explicit hashed fixture tax0, all listed ingredients charged in full (§7.3). |
| Q15 result event | D23: intents19..28 (Crystallize28), WorldEvent26..33 (owner-only ItemOperationResult33), PbOutput28 reserved; C freezes proto tags, Z reviews (§§10–11). |

D14/D18 fix cross-worker ownership/dependencies (§12.2); D15 fixes GetInventory (§11.3); D16 fixes equipment lifecycle (§§3.2,9.1); D17/D24/D25 fix per-caller bow, fist and pole behavior (§5.4); D26 fixes enriched-event replay carrier41 (§10.1). Independent re-review remains required (`AD:42`); implementation gates in §12 are not claimed as executed by this docs-only revision.
