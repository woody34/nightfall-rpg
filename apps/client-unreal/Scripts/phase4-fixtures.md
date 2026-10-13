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
Zone flags are per unit (`zone_flags` in the unit manifest): Phase 4 packs and the items
soak run the API with PHASE4_ENABLED=1; Phase 0b-3 units leave it unset.
Blacksmith is at (124,128), design §7; tax0 is hashed fixture policy.

Live grammar compatibility is separately checked by sim-parser-check.py against
C++ registration. Publication/parser tests cannot attest that E5 has landed.
