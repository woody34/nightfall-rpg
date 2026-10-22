# Phase 4 bot and fixture contract (E6 → E5/E3)

`phase4-grammar.py` is the parser-tested argument authority. Commands are case
insensitive; slot/predicate/enum tokens are case sensitive. `<item>` is a decimal
template ID, `template:<id>` or `instance:<positive-bigint>`. Template resolution
uses only the owner's received inventory, selecting the lowest instance ID;
missing baseline/item fails before sending. `entity:<uuid>` resolves a received
entity/incarnation; bare trailing UUID always means request key. Every mutation
accepts optional trailing UUID key; omitted key is fresh per invocation. A retry
with an explicit key reuses the originally resolved instance/target/body (do not
resolve a template anew after destruction/SA). Changed body with same key must
reach the server conflict path. Equip's optional slot maps to preferred_slot.
UseItem defaults to self. Refine/CancelRefine/ExchangeSA default NPC is the unique received noncombat entity named `blacksmith` (the hashed template name);
wait for `blacksmith_known`. An explicit `target` uses the current received target.
Existing nf.Target template lookup filters attackable NPCs, so it cannot select this NPC. GrantItem calls
the authenticated dev RPC for the selected character, never SQL or console GM.

Command forms (optional target is `self|target|entity:<uuid>` for UseItem,
`target|entity:<uuid>` for blacksmith commands):

```
nf.ItemCatalog
nf.MarkItemState
nf.Equip <item> [slot] [key]
nf.Unequip <slot> [key]
nf.UseItem <item> [target] [key]
nf.Destroy <item> <count> [key]
nf.Crystallize <item> [key]
nf.Enchant <scroll> <item> [support:<item>] [key]
nf.AutoShot <template> on|off [key]
nf.Refine <item> <stone> [npc-target] [key]
nf.CancelRefine <item> [npc-target] [key]
nf.ExchangeSA <item> <recipe-id> [npc-target] [key]
nf.GrantItem <template> <count> [key]
```

Slots are the lower-case EquipSlot suffixes in items.proto (rhand/lhand for
lrhand weapon; chest for onepiece). Numeric comparisons support == != >= <= > <.
`equipped` supports ==/!=. These are received projections, never predictions:

| Predicate argument form | Carrier / semantics |
|---|---|
| inventory_has template cmp count | InventorySync/Changed; sum all owner instances, including paperdoll |
| equipped slot == template (0 clears) | Owner paperdoll plus instances; public gear cannot establish owner instance state |
| enchant_level slot cmp level | Exact owner item enchant; cleared slot is UNKNOWN |
| weight_pct cmp decimal-or-ratio | Exact 100*weight/max_load cross multiplication, never rounded display/per_mille; max_load<=0 UNKNOWN |
| overload_level cmp rank | Owner PenaltyChanged/sync, not computed from weight |
| grade_penalty weapon\|armor cmp rank | Owner penalty fields |
| autoshot_on template | Received owner auto-shot set; charge/damage cannot imply toggle |
| arrows_remaining cmp count | Received owner lhand ARROW count, catalogue identifies arrow; empty slot is 0 after baseline |
| inventory_revision cmp revision; item_baseline | Owner revision and accepted live baseline |
| blacksmith_known | Received EntitySpawn identity/name/incarnation for the unique noncombat `blacksmith`, invalidated on despawn/fence reset |
| item_catalog_ready | Authenticated GetItemCatalog version2 and validated content |
| item_operation == operation | Last received keyed ItemOperationResult, operation names in grammar |
| enchant_outcome == outcome | Last received EnchantResult, normalized enum suffix |
| enchant_result_consistent | Join received result to received owner post-revision item state: success+1, no-increment unchanged, safe unchanged, blessed0, destroyed absent; refunds join received counts. No RNG/formula computation |
| refine_outcome == applied\|removed | Last received owner RefineResult (WorldEvent32), normalized enum; never ItemOperationResult33 |
| refine_result_consistent | Last RefineResult joined by owner/incarnation/target_instance_id to owner InventorySync/Changed at its inventory_revision or later; applied requires the received augment_id, removed requires augment_id absent. A stale/lower revision is UNKNOWN |
| inventory_revision_since_mark cmp revision | Owner inventory revision minus revision saved by nf.MarkItemState |
| owner_stat_delta max_hp\|p_def cmp signed-integer | Received StatsChanged.max_hp or EffectiveStats.p_def_q minus saved received scalar; exact integers, p_def uses Q1e6. Require current stats synchronized to the latest mutation revision; otherwise UNKNOWN |
| owner_stats_match_mark | Received effective sheet and whole maxima equal saved sheet/maxima after current identity+inventory baseline and synchronized stats; excludes changing current HP/MP/CP |
| refined item | Received RefineResult + owner item has augment_id |
| item_rpc_error == status | Last completed item RPC status; none only after a successful RPC, not reset default |
| item_receipt_matches_first | Exact received keyed result bytes equal first result in that operation/account scope |

`nf.MarkItemState` takes no arguments/key. It saves a complete received owner
inventory revision, effective sheet and whole maxima; missing carriers fail.
Reset the reference mark on account/character change. Retain this comparison
snapshot across DropSocket/re-admission of the same authenticated character,
including a changed incarnation; it is a comparison reference, never live state. Current projections
remain UNKNOWN until the fresh baseline/stats arrive. It makes no mechanics prediction.
CancelRefine requires `refine_outcome == removed`, a new inventory revision, and
`refine_result_consistent`; the join proves that augment_id is absent.

Armour selection: admitted set3 (DP@3ca488dd stats/armorsets/d_grade.xml:4-15):
chest58, legs59, head47, shield628, shield provider3543 and enchant6 provider3611.
`armor-enchant6` seeds the required parts at+6; `armor-enchant4` uses actual
shirt10491+4/provider3439 (stats/items/10400-10499.xml:1911-1926).
E1 capabilities admits every chosen item/provider; selectors recheck admission.
D20 runs incomplete chest→shield and shield→chest with identical final gear,
compares received maxHP (3543's source MaxHp), and restores inactive/active history
across reconnect. Full-set/+6 installation/removal and shirt+4 installation/removal
require provider-specific correlated gear_stats export facts.

Replay item evidence (E6 → Z, D46/D55–D57/D62):
`nightfall-replay export --items-evidence-out PATH` emits the schema-version1
whole-recording JSONL described in [phase4-evidence-contract.md](phase4-evidence-contract.md).
Every fact has `kind,entity,incarnation,tick,sequence,data` and may carry a
top-level `origin=live|offline`. Offline GrantItem, pending kits and backfill
must emit `origin=offline`, incarnation0 and the character UUID as `entity`.
Without that explicit origin, incarnation must be ≥1; owner filtering still
uses the character identity, so foreign-owner facts cannot satisfy a gate.

Delta data has `operation,instance,before,after,count_delta,operation_ordinal,
resolution_id` and its receipt field (plus `causes` for Consume). Correlation keys are
`(resolution_id,instance,operation_ordinal)` within owner/incarnation scope.
Per-tick batching yields ONE inventory revision step per owner/incarnation/tick,
derived from the owner's baseline or previous changed tick (previous → previous+1).
All retained post-states across all instances use that same new revision;
before-states use the previous step, or the new step for later same-tick changes.
Equal revisions remain valid. A baseline-free slice binds its initial revision
from canonical endpoints. Changed ticks strictly advance the owner step, including
removal-only ticks. Per-instance ordinals strictly increase within a tick and may
reset on a new tick. Removed instances have no fabricated post-state.
Carry complete canonical state forward per owner/incarnation/instance. Successive
target mutations and next-tick aggregates start from the preceding retained state,
including counts and all other canonical fields, after recorded intervening
mutations. Untouched canonical snapshots advance to the owner revision at tick end.
Every enchant target binding, including unchanged safe failures, must match the
established state immediately before that resolution (complete-baseline membership,
canonical continuity after intervening mutations, and the owner-tick revision step).
Creation starts from null; removal records absence; incarnations remain independent.
Do not fabricate intermediate revisions for enchant, consume or refunds.

The per-instance per-tick `operation=consume` aggregate remains ordinal0 and
emits `receipt_refs` plus a nonempty ordered `causes` list (D62 amends D57).
Each cause is `{cause,count_delta}` with a strictly negative signed-bigint delta
and an identity `auto_shot:<attack resolution_id>`, `arrow:<attack resolution_id>`,
`item_cast:<cast_id>`, or `receipt:<operation>:<key>`. Net `count_delta` must equal
the sum of these deltas and the canonical endpoint count change. `receipt_refs`
is exactly the ordered receipt-cause projection, possibly empty, nonempty iff any
cause is a client receipt. Automatic costs have no fabricated receipts. Causes
may interleave automatic and receipt-bound costs within a tick.
Other rows emit the single `receipt_ref`, nullable for receipt-free facts.
Exchange is a linked consume(old base / ingredients) + grant(new instance) audit
plan, not a monolithic `operation=exchange` delta (D52/D57). Consumes include
`receipt:exchange:<key>` in causes/receipt_refs; the fresh grant carries the same
receipt_ref. The old base must be exhausted; ingredients may retain counts.
Constituents share owner/incarnation/tick and consumes precede the grant.
The old `receipt` field and a singular Consume `receipt_ref` refuse.

Every delta uses the shared operation transition table: consume changes only
count; equip/unequip only slot/location; enchant only enchant; refine/cancel_refine
only augment_id. Destroy/crystallize end at null and use separate refund grants.
Grant/starter/backfill start at null and materialize canonical instances. Other
canonical fields remain identical, with revision governed by the owner/tick step.
Counts and causes agree with endpoints; unchanged/revision-only and `use` rows
refuse. See `phase4-evidence-contract.md` for the complete operation rules.

Each enchant outcome's `operation` additionally binds the nonempty `receipt_ref`
of its resolution, alongside canonical target/scroll/support before-states.
For a shared scroll/support stack, group contributions by
`(entity,incarnation,tick,instance)` in outcome fact sequence order. Its receipt
causes contain every contributing enchant exactly once, ordered and each at -1;
unbound receipts in the enchant operation scope refuse. Other client scopes and
system causes may interleave. Bound before-states reflect ALL preceding cause
deltas, not just earlier enchant costs. The aggregate uses the first contributor's
resolution ID (the enchant resolution or automatic attack/cast identity) and its
canonical initial/final states and net count must agree with the cause list.
Bound revisions never decrease; later before-states share the applicable tick
revision. No bound revision may exceed the verified owner step. Retained
scroll/support endpoints and exhausted contributions use this same step; exhausted
stacks have no invented post-state. Receipt uniqueness spans all enchant resolutions
in the scope before stack grouping, including resolutions spending distinct stacks.
Non-Consume target/refund rows retain their resolution ID and its matching receipt.
See `phase4-evidence-contract.md` for exact JSON examples.

Combat phases use nf.Mark/target_hits_since_mark; template15's pole receives
primary108,100 and secondary108,101 /108,99 targets (explicit fixture positions).
The unarmed phase unequips rhand; source-bound phases require actual weapon caller
context. Blow/lethal remain blocked by the current E1 capability manifest: no
admitted FatalBlow/Backstab/Lethal definition exists. Deferred skills are never
seeded or represented as executable. Required blow_success/lethal gates remain.

UNKNOWN until appropriate carrier/catalogue/baseline arrives; UNKNOWN is never
allow-listed. Reset on account/character/session generation change. New incarnation
invalidates inventory, receipts, last results, weight and penalty until identity
then baseline. Tick/baseline/revision gaps fail closed and request resync; accept
same-revision weight/penalty/stats siblings without dropping them. Public-only
observer messages never hydrate private inventory. Reconnect invalidates transient
results but preserves a scoped first-receipt comparison history for explicit retries.

Fixtures: `phase4-packs.json` publishes exact role inputs and source citations;
counts/enchant/level/SP/position are declared test inputs, not production constants.
All kit extras/equipped flags come from starter_kits.toml; capacity/blacksmith
policy comes from capacities.toml. Pack `instance_id`s are pack-local labels.
Provisioning (D112, `infra/scripts/seed-phase4-fixture.py`): before API startup the
Phase 3 character seed plus a re-run of the frozen pending-kit backfill
(m20261012_000011); no item, ledger or allocator rows. After the owned API is
healthy and before any login (`phase2-fixture.py grant`, inside the startup
allowance): one dev GrantItem gRPC per (role, item) via `buf curl`, deterministic
UUIDv5 keys, serial per character; the first grant materializes the kit. Admission
is confirmed from the offline GetInventory baseline (every template/count/enchant/
slot must equal the pack) and the server-allocated ids are recorded as
`instance_map`; the bot runs a scenario copy whose `instance:N` labels are mapped
(`phase2-fixture.py scenario`). D113: enchanted pack instances are granted at their
level through the optional GrantItem `enchant_level` (one constant, ENCHANT_FIELD) and
the level is confirmed from GetInventory; all 39 packs provision. Charges/augment, or
items equipped beyond the kit, still have no transport and refuse before any write.
Zone flags are per unit (`zone_flags` in the unit manifest, checked again by
`phase2-fixture.py exec-api`). D114: every unit runs on V6, so the value is
PHASE3_ENABLED=1 and PHASE4_ENABLED=1 for all Phase 2, 3 and 4 packs. Shared-stack units
(Phase 0b/1, `sim-lib.sh --api start`) start the API with the same two flags. After
health, sim-lib records what the API ATTESTS (review-3 P2-3): `phase2-fixture.py zone-flags`
reads the API's own V6 bootstrap log line ("Phase3 bootstrap bundle validated with the
Phase4 (V6) item extension") and writes the flags with its `phase4_item_rules_hash` and
`phase4_skill_rules_hash` to `<unit>/zone-flags.json` (shared stack) or
`<unit>/<first scenario>/fixture/zone-flags.json` (owned fixture). T15: every API the
harness starts (shared stack, owned fixtures, soak) runs with `TELEMETRY_PRETTY=0`, whatever
the caller's `.env` says (the repo default is 1), so its log is JSON. The parser also strips
ANSI escapes and accepts the pretty and plain tracing layouts. An API the harness started
whose log has no V6 bootstrap line after health fails the unit loudly (`sim_die`). An
attached API (`--api attach`) records nothing, so its units cannot verify E6.3.
Blacksmith is at (124,128), design §7; tax0 is hashed fixture policy.

Live grammar compatibility is separately checked by sim-parser-check.py against
C++ registration. Publication/parser tests cannot attest that E5 has landed.

## V6 everywhere (D114)

All live runs use a V6 zone (PHASE3_ENABLED=1, PHASE4_ENABLED=1; V6 refuses PHASE3_ENABLED=0):

- The scenario suite (`run-sim-ci.py --fresh-stack`): owned fixture units and shared-stack
  units, as above.
- Required-live UE automation (`run-tests.sh --require-live-api`): it attaches to an API
  that the operator or CI starts. Start that API with `PHASE3_ENABLED=1 PHASE4_ENABLED=1`
  (and `AUTH_DEV_TOKENS=1`).
- The soak (`run-soak.sh`, all modes including `--legacy-attacks`): the API is always
  started with `PHASE3_ENABLED=1 PHASE4_ENABLED=1`. The role manifest's `items` flag still
  needs `--items`.

E6.3 regression (`phase4-regression.py`). The committed files
`Scenarios/phase4-regression.pending.json` and `Scenarios/phase4-backfill.pending.json` are
inventories only (every Phase 0b-3 expectation; every profession x alive/dead/never_admitted
backfill case). They hold no verdict. Regenerate them with
`phase4-regression.py --publish Scenarios/phase4-regression.pending.json --backfill
Scenarios/phase4-backfill.pending.json`. The verdict:

    phase4-regression.py --suite <run-sim-ci artifacts> --backfill-results <api results.json> \
        --out verdict.json --require-live

A Phase 0b-3 scenario is verified only when its unit is PASS in `summary.json` (QUARANTINED
does not verify), the unit `pipeline-verdict.json` and the scenario `pipeline-state.json`
are clean, every recording of the scenario (its own and the unit's shared ones) has a
`.replay.log` that reports `match.`, and the unit's API attested V6 (`zone-flags.json` with both
rules hashes; a fixture unit has exactly one `*/fixture/manifest.json`, shared by its paired
scenarios, and its intended `zone_flags` must agree with the attested record). A backfill case is verified only when the API's
`phase4_backfill` results name it (same `class_id` and `state`) with every required gate
`true`. Unknown or duplicate result ids and other schema versions are refused.
D119: third-class (tier 3) professions are not admissible characters (ClassState
refuses tier > 2). The inventory holds 58 professions x alive/dead/never_admitted =
174 cases. A result for a tier-3 id is out of scope (listed as `out_of_scope` in the
verdict) whether it is all-false with a note, marked `not_applicable`, or absent.
`not_applicable` on an admissible case is not verified. Reference results: API
session 17 (`phase4_backfill`, file commit 40ecdab) verify 174/174 with 93 out of scope
(`test/fixtures/phase4-backfill/api-s17-results.json`).
`--require-live` exits 1 unless everything is verified. A scenario that fails is a
zone-lane finding: do not change its expectations to make it pass.

Backfill results shape (agreed in api-inbox.md; written by `apps/api/tests/phase4_backfill.rs`
when `PHASE4_BACKFILL_RESULTS=<path>` is set; extra keys such as `note` or
`before_commit_failure` are ignored):

    {"schema_version": 1, "source": "apps/api/tests/phase4_backfill.rs", "commit": "<sha>",
     "cases": [{"id": "<profession key>-<alive|dead|never_admitted>", "class_id": <int>,
                "state": "<alive|dead|never_admitted>",
                "gates": {"migration_rerun": true, "first_v6_admission": true,
                          "crash_resume": true, "kit_once": true,
                          "allocator_no_collision": true}}]}

Item evidence reads a closed epoch (D122/T23). The item-evidence exporter refuses live or
partial captures, and that refusal stays. For a Phase 4 unit with item gates or phases,
`sim_phase4_evidence` runs after the live capture's replay, trace and coverage gates. It then:
1. stops the unit's owned fixture API gracefully (`sim_api_stop_graceful`, SIGTERM; the API
   drains and closes the epoch with a Shutdown watermark; it fails if the API does not exit
   within `SIM_API_STOP_GRACE`, default 30 s);
2. exports that closed epoch non-live for the report owner to
   `<unit>/<scenario>/closed-epoch/<scenario>.nfr`, retrying up to 3 times because JetStream
   can time out just after the shutdown;
3. runs `phase4-evidence.py` on that recording.

The live `<scenario>.nfr` remains the unit recording for every other gate. Offline fixtures
(`SIM_REPLAY_CMD`) are unchanged. Proven on an owned phase4-armor-enchant4 fixture with the
integration 7262804 binaries. The live capture's item export refuses ("partial/live
captures"). The graceful-stop closed epoch has a Shutdown watermark, replays to a match, and
its item evidence validates.

Accessory refine (D123/T24). Weapon refine stays deferred (D82: no weapon life stone is
admitted, so the zone refuses it with ItemCapabilityDeferred). 4-augment therefore refines an
accessory, with the same assertions (applied outcome, draws/options, cancel fee). The
phase4-blacksmith pack (shared with 4-sa-exchange, whose SA ingredients are unchanged) now
carries:
- the stable minimum admitted C-grade accessory: Necklace of Seal 119;
- the minimum admitted accessory life stone: 12754 (level 46, the owner's level);
- the C-grade accessory gemstones: 2130 × 200;
- the cancel-fee adena.

The weapon life stone 8723 and 2130 × 20 are gone (`accessory_refine()`).

Grant capacity (D125 amended, T27). The dev GrantItem enforces the real inventory slot
capacity on every addition: ordinary slots 80 (dwarf 100) and quest slots 100. Stackables
take one slot per template; other items take one per instance. A violation is refused with
InventoryFull. Dev grants may exceed max load (weight), as an L2J admin-created item can make
a player overweight; 4-overload depends on this (overload level 4, weight > 100%).
`seed-phase4-fixture.py capacity_blockers()` models the slot rule on each pack's final
inventory, so a pack over slot capacity is blocked at plan time, before any write.

Seeded class-transfer history (D124/T25). The API refuses a class-transferred owner that has
no class-transfer receipts (FAILED_PRECONDITION `invalid_class_transfer_receipt_history`).
For each seeded role whose class is not its base class, `seed-phase3-fixture.py`
(`transfer_receipts`) writes one `change_class` receipt per tier in the seed transaction. The
receipts follow the parent chain in `professions/*.toml`, and each one has:
- an `idempotency_keys` row with the frozen transfer result: the owner's identity, the base
  class's own stats, the target's minimum level and zero vitals;
- a `character_transfer_receipts` row with ordinal 0 or 1.

Today the only such role is the phase4-kinds owner (Treasure Hunter: receipts for classes 7
and 8). A level below a target's minimum, or above 85, refuses before any write.

The `characters.class_profile` column of a transferred owner is the profile of its base class
(`human_fighter` for the phase4-kinds owner), not its current profession. The API writes the
base profile at creation and a class transfer does not change it. The zone load check
(`loaded_player`) refuses an owner whose `class_profile` is not the base profile of its
`base_class_id` with `FAILED_PRECONDITION "character state does not match the zone's rules"`
(T25 at 611407e). `phase4-fixtures.py` (`role`) derives the column from the tier-0 profession
of `base_class_id`.

## Pinned enchant outcomes (T52)

The zone has one random source: a `ChaCha12` stream with the key `nightfall.zone.rng.1`,
the zone id and the epoch (`domain/zone/state.rs`, `ZoneSeed`). Every fresh fixture stack
is zone1 epoch1 and starts at word0. The stream is the same in every run.
`phase4-fixtures.py zone_rng_word(position)` computes the stream in Python, and
`zone_roll(position, bound)` applies the zone's `roll_below`. The i3e exports confirm
words40..43: 47096, 26339, 39447 and 89321 (modulo100000).

The ten NPC spawns of the Phase4 overlay draw entity ids (4 words each, words0..39).
An idle NPC with a positive speed also rolls `npc_walk` on its own cadence. In i3e this
roll moved the enchant draw of 4-enchant-blessed from word40 to word41. The enchant
pack overlay (`create_overlay(..., 'phase4-enchant')`, `pin_rng_cursor`) makes every
spawn slot a static zone NPC with speed0 and stops the Wanderer, so the entity count
stays ten and no walk roll occurs. The first enchant of the unit then draws word40,
value47096, in every run.

The outcome follows from the pinned rate groups (`enchant_groups.toml`): a FIGHTER
weapon at +3 has 70000 (success), a MAGE weapon at +3 has 40000 (failure). The pack adds
the stable minimum admitted C mage sword84 at +3 (instance12). 4-enchant-success
enchants the fighter sword at +0 (100000, success). 4-enchant-fail enchants instance12
with the normal scroll951 (destroyed; no exact-key replay, the instance is gone).
4-enchant-safe (22227) gives safe_failed and 4-enchant-blessed (6573) gives
blessed_reset; both keep the item and replay the exact key. Each unit waits for its
outcome (`nf.WaitFor enchant_outcome == <outcome> 3`) and keeps the evidence gates
enchant_chance, enchant_outcome and failure_refund.

`success_no_increment` has a rule (`state_item_enchant.rs`: base chance0 at the item's
level, success branch) but no pinned unit. It needs armour at +20 (ARMOR_GROUP chance0
from +20) with the bonus scroll22011 (+10%) and a draw below10000. The first such word is
57 (4808), seventeen words after the pinned cursor, and the harness has no deterministic
way to consume seventeen words before an enchant (only three 100% enchants exist on the
fighter sword +0..+2). D161: the suite coverage gate (`phase4-coverage.py`,
`Scenarios/phase4-coverage.required.json`) does not require `success_no_increment` in
Phase4. The label stays valid for the producer (`sim-gates.py` V4_LABELS), because the
pinned L2J rule exists. The branch needs armour at +20 and the +10% bonus scroll22011, which
is outside the Phase4 admitted item range (support items stop at max_enchant9), and the
harness has no fixture RNG-cursor control. Phase5 adds that control and returns the label to
the required set. Every other enchant outcome stays required.
