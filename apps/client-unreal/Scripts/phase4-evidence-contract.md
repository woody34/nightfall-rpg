# E6 item evidence handoff — frozen D46 interface

`phase4-evidence.py` is a strict consumer of the **frozen D46** E2
`nightfall-replay export --file FILE --session OWNER --items-evidence-out PATH`
interface. The producer must replay/check the complete recording before exporting.
The packaged soak uses the same export without a session filter. The consumer
runs the candidate replay binary's `check` first for scenario runs. It never
builds a binary or accepts producer PASS flags. `SIM_PHASE4_RULES_HASH` and
`SOAK_PHASE4_RULES_HASH` are the compiled runtime hash, not the generated data hash.

JSONL contract (schema version 1 of this sidecar):

- First row: `kind=manifest`, `schema_version=1`, `scope=whole_recording`,
  `recording_sha256`, `zone`, `epoch`, `snapshot_tick`,
  `watermark={zone,epoch,records,last_tick,reason}`, `phase4_rules_hash`,
  `snapshot_schema=10`, `record_schema=8`, `digest_version=BinaryV6`.
  Reason is `shutdown` or `epoch_closed`; partial exports refuse.
- Every fact: exactly `kind,entity,incarnation,tick,sequence,data`, plus optional
  top-level `origin` (`live|offline`). Canonical UUID entity, live incarnation ≥1,
  contiguous zero-based sequence and ordered ticks
  within the watermark. Facts cover the complete epoch; owner filtering happens
  in the consumer, so missing or foreign-owner facts cannot satisfy a gate.
  Offline GrantItem, pending-kit and backfill materialization facts explicitly
  carry `origin=offline`, incarnation0 and the character UUID in `entity` (D56).
  Incarnation0 without that explicit origin refuses; it never invents a live owner.
- `item_formula` data: `formula,inputs,actual,resolution_id,baseline=false`.
  The independent E1 Fraction functions recompute `actual` from actual resolution
  inputs; no vector lookup, tolerance, floats, booleans masquerading as integers
  or duplicate resolution identity. `gear_stats` additionally carries the pinned
  class XML path, level, concrete equipment/install sequence, resolved winning
  skill providers and base stat deltas. Source-dependent inputs fail closed.
- `item_rng`: `callsite,word_before,word_after,bound,value,resolution_id`. Preserve the complete
  ordered draw trace, including intervening non-item draws when needed for cursor
  continuity. Draw-free branches have no fabricated draw. Enchant always requires one bound100000
  draw at callsite `enchant`, including guaranteed success (GAME@abfde049
  EnchantScroll.java:149-155). Its residue must equal the outcome input.
- `item_delta`: `operation,instance,before,after,count_delta,operation_ordinal,resolution_id`,
  plus `receipt_refs,causes` for `operation=consume`, otherwise `receipt_ref`.
  Consume is one aggregate row per instance per tick at the final ordinal. Under D62,
  `causes` is a nonempty ordered list of `{cause,count_delta}` contributions.
  `cause` is `auto_shot:<attack resolution_id>`, `arrow:<attack resolution_id>`,
  `item_cast:<cast_id>`, or `receipt:<operation>:<key>`; every identity component
  is nonempty. Per-cause `count_delta` is a strictly negative signed-bigint integer;
  the aggregate's net `count_delta` equals their sum and the endpoint count change.
  `receipt_refs` is exactly the ordered projection of receipt causes, possibly
  empty: it is nonempty iff at least one cause is a client receipt. Automatic
  consumption has no fabricated receipt. System and client contributions may
  interleave within a tick in cause order (D62 amends D57). Other rows have a
  single `receipt_ref` (nullable for receipt-free facts);
  Exchange is represented by `consume` constituents and one `grant` of a fresh
  instance, linked by `receipt:exchange:<key>` (D52/D57). The consumes precede
  the grant within the same owner/incarnation/tick; the old base is exhausted,
  while ingredient stacks may remain. Each consume includes that receipt in its
  causes and ordered receipt projection; the grant carries it in `receipt_ref`.
  The grant's `resolution_id` binds exactly one same-owner/incarnation/tick
  `sa_exchange` formula. Its `recipe_id` must name an E1 `sa_variants.toml`
  recipe verified against the pinned multisell source. The bound
  `inputs.before_instance` must be the exhausted base instance; the fresh grant
  matches `inputs.after_instance`, including the recipe product template and
  enchant/augmentation copied only when `maintain_enchantment` is true.
  Every required ingredient (base, soul crystal, gemstones, adena under tax0)
  contributes its exact recipe count under that Exchange receipt. Ingredient
  stacks may remain or split across instances; shared aggregates count only
  the negative cause belonging to this receipt, once per constituent stack.
  Missing formula/receipt groups, constituents and conserved underpayment refuse.
  Missing/unlinked constituents and monolithic `operation=exchange` refuse.
  The old `receipt` field refuses.
  `before/after` are null or the complete canonical ItemInstance fields from
  `domain/zone/item.rs`, not partial count projections. Exact widths, identity,
  real change, count conservation, duplicate logical rows and the final Consume ordinal
  are checked. Pure toggles/reuse never fabricate a ledger instance (D28).
  D66 amends D64: canonical order per `(entity,incarnation,tick,instance)` is
  ALL positive rows (`grant`, including refunds/Exchange products, `starter`,
  `backfill`) FIRST in cause order, then the single Consume aggregate LAST.
  Ordinals are zero-based in this chain. The first before equals tick-start
  state (null for a fresh allocation); each next before equals the preceding
  after. Consume's before is the post-positive state and its after is tick-end.
  Validate `tick_end_count = tick_start_count + sum(count_delta)` across all rows.
  Internal tick observations are not intermediate ledger endpoints (A3).
  Runtime counts `3→2→5→4` export Grant `3→6, +3` then Consume `6→4, −2`.
  Other controls: start1 exports Grant `1→4, +3` then Consume `4→2, −2`;
  start absent exports Grant `null→3, +3` then Consume `3→2, −1`.
  D69 amends D66: the rows before the Consume aggregate are ALL non-consume
  rows on that instance in cause order — positive (`grant`, `starter`,
  `backfill`) AND count-neutral retained rows (`equip`, `unequip`, `enchant`,
  `refine`, `cancel_refine`) — with the same endpoint continuity; the single
  Consume stays last. Equip-then-shoot on an arrow stack in one tick exports
  Equip `10→10` (inventory→lhand) then Consume `10→7`. Removal rows
  (`destroy`, `crystallize`) never precede Consume, and a Consume followed by
  any row on the same instance and tick refuses.
  A positive row may increase an existing stack; it preserves all canonical
  fields except count/revision. Fresh Exchange products still require null
  before and a new product instance (D37/D52).
  D67 requires every null-before positive row to allocate an ID never seen earlier
  in the whole recording, across owners, ticks and incarnations. Baselines,
  ledger endpoints and canonical observed before bindings establish allocation;
  formula outputs describe a product without allocating it. Retained positives
  preserve every canonical field except count/revision. Removal leaves a tombstone
  with the last canonical metadata: exhausted IDs cannot be regranted, restored
  with a non-null before, or reintroduced by a later baseline. Genuinely fresh
  grants/refunds/Exchange products remain legal.
  D69 extends identity continuity to EVERY observation of an ID: baseline
  instances, enchant `target_before/scroll_before/support_before` bindings and
  the `before` of every ledger operation (consume, equip, unequip, enchant,
  refine, cancel_refine, destroy, crystallize, grant, ...). Each is compared
  with the recorded identity before the record is updated: all canonical fields
  except count/revision must be unchanged, and a live (non-exhausted) ID is
  bound to the entity that first held it, so observing it under a different
  entity refuses (Phase 4 has no transfer operation). The record advances from
  `after` only once `before` has matched, so no row can launder drift.
  D71 amends D69: across a re-admission (the first observation of an ID under a
  later incarnation of its owner) exactly the transient `charges` field is
  exempt from continuity, because charged shots live only in the in-memory
  `L2ItemInstance` and are cleared on relog; the producer emits the cleared
  value. The baseline `auto_shots` set is likewise not carried across
  incarnations (toggles are off after relog). Slot/location (the paperdoll is
  retained, D16) and every other canonical field must still match, and within
  one incarnation `charges` is compared like any other field.
  D72 narrows D71: at that boundary `charges` may only CLEAR — the new value
  is 0 or unchanged; a charge that appears or changes kind refuses. The
  owner's first `item_baseline` of a later incarnation, once an earlier live
  incarnation (≥1) of that entity was observed in the recording, carries
  `auto_shots=[]`. A recording whose first observation of the owner is
  already a later incarnation (snapshot start) is not a re-admission.
  Owner binding and the D68 cap also apply to a malformed (non-canonical)
  enchant before binding whose `id` is a positive integer, whichever owner is
  being evaluated.
  ONE shared operation transition table applies to EVERY delta, independently of
  formula correlation. Revision is governed by D55 below; all other canonical
  fields outside the allowed set must remain identical:

  | Operation | Required endpoints / allowed changes |
  |---|---|
  | consume | Existing before; only count changes by the strictly negative cause sum. Remaining count ≥0; after is null iff exhausted. |
  | equip / unequip | Both endpoints; only slot/location change. Equip enters paperdoll; unequip leaves it for inventory. |
  | enchant | Both endpoints; only enchant changes. Outcome correlation requires the exact source result; unchanged outcomes have no row. Destruction/refund uses separate destroy/grant rows. |
  | refine / cancel_refine | Both endpoints; only augment_id changes. Refine installs a packed option pair; cancel removes it. |
  | destroy / crystallize | Existing before; after is null. Refunds are separate grant rows. |
  | grant | Canonical after; strictly positive count_delta. Before is null for creation, or canonical for a retained stack with only count changing. |
  | starter / backfill | Before is null; after is canonical. |

  D68 bounds every canonical count (baseline instances, enchant bindings and
  both endpoints of every ledger row, which are the only intermediate states
  under D66/D69) by the pinned per-template cap: adena (template 57,
  `Inventory.java:51`) at `MaxAdena = 99900000000` (`character.properties:671`);
  any other E1 item with `stackable=true` at `Integer.MAX_VALUE = 2147483647`
  (`L2ItemInstance.java:422`, the pinned stack ceiling; the E1 tables carry
  `stackable` but no per-template maximum); `stackable=false` items at 1. Python
  integers are unbounded, so sums never wrap (i128-equivalent).
  `count_delta` is an exact signed-bigint integer equal to the endpoint count
  difference for every operation. Revision-only rows and `operation=use` refuse;
  item-use costs belong in Consume aggregates.
  Under D55, correlate deltas by `(resolution_id,instance,operation_ordinal)`
  within the owner/incarnation scope, independent of operation name. Derive ONE
  revision step per `(entity,incarnation,tick)` from the owner's baseline or
  previous changed tick: previous revision → previous revision + 1. Every retained
  post-state, across ALL instances, must use that same new revision. Before-states
  use the previous revision, or the new revision for later same-tick changes;
  equality remains valid. A baseline-free slice binds its initial revision from
  its canonical endpoints. Subsequent changed ticks strictly advance the owner
  step even when every changed instance is removed. Per-instance ordinals strictly
  increase within a tick and may reset on a new tick. Removed instances have no
  fabricated post-state. Never fabricate intermediate revisions.
  Carry complete canonical states forward per owner/incarnation/instance, including
  counts, templates, locations, enchantments and removal. Each next mutation or
  aggregate must start at the established retained state, after recorded intervening
  changes; untouched canonical snapshots advance to the owner's revision at tick
  end. Creation starts from null and removal establishes absence. Incarnation
  boundaries have independent state/revision histories; allocation freshness,
  exhausted-ID tombstones, single-owner binding and canonical identity fields
  (D69, except `charges` at the boundary, D71) are recording-wide.
- `item_baseline`: `inventory_revision,instances,paperdoll,auto_shots`. Complete
  canonical instances and valid paperdoll references bind an owner. A baseline
  cannot satisfy formula gates. Disabled GrantItem is rejected before actor
  enqueue; its client JUnit proves RPC status, and a complete owner-bound log
  proves absence of grants. Do not invent a rejected actor command to attest it.
- Last row: exactly `kind=complete,facts,ticks,last_tick`, agreeing with the
  manifest watermark and number of preceding fact rows.

Revision 2 tightens the D46 interface under the accepted review instructions:
RNG and non-aggregate ledger facts carry the same `resolution_id` as their formula facts.
The key is `(entity,incarnation,resolution_id)`, never tick or owner alone.
An `enchant_outcome` formula additionally requires
`operation={target_before,scroll_before,support_before,receipt_ref}` (support may be null).
The three before-states are canonical instances at resolution time, consistent with D49's audit capture.
`receipt_ref` is the nonempty receipt of this resolution, unique among contributing
enchant resolutions within the owner/incarnation/tick scope. Its non-Consume
target/refund deltas carry this same receipt. Exactly one correlated `enchant_chance`, draw, scroll debit, optional support debit,
and outcome-dependent target change are required. Destruction verifies source
crystal/refund inputs from E1 grades/items and requires the actual positive refund
row when applicable. Success/reset/no-increment/safe branches have no fabricated
refund; unchanged targets have no D28 ledger row. EVERY target binding, including
unchanged outcomes, must match the canonical owner/incarnation/instance state
immediately before that resolution: complete-baseline membership, continuity
after intervening mutations, and the applicable owner-tick revision step. Sources: GAME@abfde049
RequestEnchantItem.java:123-139,154-271; L2Item.java:353,398-419;
CrystalType.java:26-33. Deletion tests repair sequences/trailer counts.
E2 must implement these correlation additions before producer verification.

Consume correlation uses `(entity,incarnation,tick,instance)`, rather than one
resolution ID per debit. For each scroll/support stack, collect contributing
`enchant_outcome.operation.receipt_ref` values in fact sequence order. The single
final-ordinal aggregate's receipt causes must include each enchant contribution exactly
once, in that same order, with `count_delta=-1`. A receipt in the enchant operation
scope without a corresponding outcome refuses. Other client operation scopes and
system causes may interleave; their deltas also contribute to the running count.
Each bound scroll/support before-state must equal the aggregate's canonical initial
state after ALL preceding causes, including automatic costs. Missing, duplicated
or reordered enchant contributions and incorrect combined totals refuse.

The aggregate carries the first contributor's `resolution_id`: the bound enchant
resolution for a first enchant receipt, or the system cause's attack resolution/cast
ID for a first automatic cause. Its after-state is the resulting stack or null when
exhausted. Receipt uniqueness is checked across all enchant outcomes in the
owner/incarnation/tick scope before grouping by stack. Canonical contribution
revisions never decrease; later bindings equal the tick's retained revision step,
and no binding may exceed it. Equal revisions are permitted. Exhausted stacks
have no invented post-state; use the verified owner step for every contribution.

For example, two enchant debits can share a stack with an intervening item-cast cost:
`causes=[{"cause":"receipt:enchant:key1","count_delta":-1},
{"cause":"item_cast:cast1","count_delta":-2},
{"cause":"receipt:enchant:key2","count_delta":-1}]`,
`receipt_refs=["receipt:enchant:key1","receipt:enchant:key2"]`, `count_delta=-4`.
An automatic-only shot aggregate instead has
`causes=[{"cause":"auto_shot:attack1","count_delta":-1}]`, `receipt_refs=[]`.

D55–D57 and D62–D71 revise this same schema-version1 sidecar for the accepted producer:
same-tick enchant/consume/refund rows may share revisions, offline facts must
emit the explicit top-level origin, and Consume aggregates emit ordered
`receipt_refs` and `causes`; Exchange grants emit `receipt_ref` and their consumes
include the Exchange receipt in both lists. Positive controls and
repaired-deletion/mutation probes cover multiple enchant deltas on one instance,
revision/ordinal ordering, offline incarnation0 owner binding and two-receipt
Consume aggregates. These checks do not attest a live producer.

Formula data may also carry `context` with only `weapon_template,skill_id,
target_entity,event,set_id`. IDs are bounded integers; weapon0 means unarmed;
event is equip|unequip|enchant|admission|attack|cast. These are captured source
caller identities, not harness phase labels. `# item-phase: <JSON>` selects a
required owner resolution by formula, context, input/output values, required or
absent skill providers, and a correlated ledger mutation. Missing weapon/provider
phases fail even if the generic gate is already satisfied. `providers` and
`absent_providers` inspect actual `gear_stats.inputs.skill_refs`; they never inject
providers. Mutation selectors require the same owner/incarnation/resolution ID.
Admission phases verify retained history after reconnect.

Revision 3 binds scroll/support templates to E1 `enchant_scrolls.toml`, target
kind/grade/enchant eligibility and blacklist to the item catalogue, and the base
chance to `enchant_groups.toml` (slot, magic-weapon route and current enchant range).
Safe/blessed flags and both bonus contributions must match before deciding which
ledger/refund branch applies. Citations: DP@3ca488dd enchantItemData.xml:4-254 and
enchantItemGroups.xml:4-93; GAME@abfde049 AbstractEnchantItem.java:111-145,
EnchantScroll.java:100-157, EnchantRateItem.java:71-77, ItemTable.java:74-109
(overlapping slot masks and aliases) and L2Item.java:539-540. Formula inputs enforce count/tick
domains, u32 enchant values and draw bounds before calling the oracle; signed
coordinates and additive stat modifiers remain signed. Canonical template IDs must
exist, including dormant starters; auto-shot IDs additionally require an admitted
SHOT with a supported SoulShots/SpiritShot/BlessedSpiritShot handler in E1.

Revision 4 distinguishes inventory count maps from item-record lists; numeric
record fields are bounded individually while grade/kind metadata is preserved.
The full consumer positive control uses bow13, arrow17 ×2 and MP10: a hit consumes
one arrow and leaves MP9. Source: DP@3ca488dd stats/items/00000-00099.xml:274-295,
368-380; E1 item_behavior_oracle.py::bow_definition/arrow_stack/bow_launch.
Negative/overflow counts still fail even with oracle-consistent mutated output.
`resurrection_scroll.restore_hp` is an exact numeric ratio in [0,1], not a boolean.
GAME@abfde049 config/character.properties:41 specifies RespawnRestoreHP=0.65;
scroll737 with max HP101 restores exactly 1313/20 using 13/20. The full consumer
positive probe and invalid ratio mutations exercise the same E1 resurrection oracle.

The `equipment` phase selector maps concrete slots to templates (null means absent).
The active incomplete armour admission requires chest58, shield628, starter legs461,
absent head47 and retained provider3543. All three armour workloads reconnect there
and require unchanged received stats before removing the provider. They retain the
shield→chest inactive checkpoint and complete-set checkpoint. Source: DP@3ca488dd
stats/armorsets/d_grade.xml:4-15; GAME@abfde049 PcInventory.java:1147-1208,1232-1285.
The mutation probe recomputes the sheet with `full_set && shield`: the generic
formula passes, but the incomplete admission provider requirement fails.

`# item-gate: <formula>` declarations use the finite E1 function registry plus
pinned-source gear/SA checks. Enchant's refund gate requires a destruction refund
only if independently verified actual EnchantResult executed destruction; a
successful, blessed or safe attempt has no destruction refund branch. Required
coverage still needs every branch across the suite, not one RNG-dependent unit.

The D46 harness-owned native transition schema4 retains **all** schema3 tables/counters/
scalars and the complete inherited reachability catalog. Add bounded-symbol
counters `item_operations,enchant_outcomes,refine_outcomes,shot_payments` and
scalars `item_ledger_rows,item_allocations,inventory_baselines`. Unknown/missing
fields and mixed-schema merging refuse. `phase4-coverage.required.json` records
future required observations parsed from frozen proto fields; it is not a
historical observed baseline. No old baseline was promoted or loosened.

D26 live/replayed wire bytes, enriched tag41 round trips, legacy tag selection,
Revision 2 producer implementation, live ledger/receipt cause linkage and measurement
proof remain E2/E3 integration gates. Synthetic Python tests prove consumer
rejection and formula behavior, never that a server emitted these facts.
