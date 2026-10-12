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
  plus `receipt_refs` for `operation=consume`, otherwise `receipt_ref`.
  Consume is one aggregate row per instance per tick at ordinal0; `receipt_refs`
  is an ordered list of ≥1 nonempty receipt references, in contribution order
  (D57). Other rows have a single `receipt_ref` (nullable for receipt-free facts);
  Exchange rows require the single nonempty Exchange receipt (D52/D57), including
  constituent rows whose operation is `exchange`. The old `receipt` field refuses.
  `before/after` are null or the complete canonical ItemInstance fields from
  `domain/zone/item.rs`, not partial count projections. Exact widths, identity,
  real change, count conservation, duplicate logical rows and consume ordinal0
  are checked. Pure toggles/reuse never fabricate a ledger instance (D28).
  Under D55, correlate deltas by `(resolution_id,instance,operation_ordinal)`
  within the owner/incarnation scope, independent of operation name. A tick has
  one inventory revision step: `before.revision ≤ after.revision` when both
  instances exist; equality is valid for later same-tick changes. Per instance,
  ordinals strictly increase within a tick and retained post-revisions strictly
  increase across ticks; every retained post-state of an instance in a tick
  shares that tick's revision step and exceeds the previous tick's retained
  post-revision, including later rows. Ordinals may reset on a new tick. A removed instance
  has no post-revision to compare. Never fabricate intermediate revisions.
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
refund; unchanged targets have no D28 ledger row. Sources: GAME@abfde049
RequestEnchantItem.java:123-139,154-271; L2Item.java:353,398-419;
CrystalType.java:26-33. Deletion tests repair sequences/trailer counts.
E2 must implement these correlation additions before producer verification.

Consume correlation uses `(entity,incarnation,tick,instance)`, rather than one
resolution ID per debit. For each scroll/support stack, collect contributing
`enchant_outcome.operation.receipt_ref` values in fact sequence order. The single
ordinal0 aggregate carries the first contributor's `resolution_id` and exactly
that ordered `receipt_refs` list. Its canonical before-state equals the first
bound scroll/support before-state; later bindings retain the same instance fields
and reflect each preceding one-item debit. Receipt uniqueness is checked across
all enchant outcomes in the owner/incarnation/tick scope before grouping by stack.
Canonical contribution revisions never decrease; later bindings equal the tick's
retained revision step, and no binding may exceed the aggregate's post-revision.
Equal revisions are permitted. Exhausted stacks have no invented post-state;
use other retained deltas in the tick to check the step when available, otherwise
require ordered bindings with one shared later revision.
The aggregate's `count_delta` is minus the number of contributions; its after-state
is the resulting stack or null when exhausted. Two same-tick resolutions may thus
spend one shared stack via one `count_delta=-2` row with
`receipt_refs=["receipt:1","receipt:2"]`. Missing, duplicated or reordered
contributions, repaired deletion, and incorrect combined totals refuse.

D55–D57 revise this same schema-version1 sidecar for the accepted producer:
same-tick enchant/consume/refund rows may share revisions, offline facts must
emit the explicit top-level origin, and Consume aggregates emit ordered
`receipt_refs` while Exchange rows emit `receipt_ref`. Positive controls and
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
