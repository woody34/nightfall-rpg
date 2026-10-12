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
- Every fact: exactly `kind,entity,incarnation,tick,sequence,data`. Canonical UUID
  entity, positive incarnation, contiguous zero-based sequence and ordered ticks
  within the watermark. Facts cover the complete epoch; owner filtering happens
  in the consumer, so missing or foreign-owner facts cannot satisfy a gate.
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
- `item_delta`: `operation,instance,before,after,count_delta,operation_ordinal,receipt,resolution_id`.
  `before/after` are null or the complete canonical ItemInstance fields from
  `domain/zone/item.rs`, not partial count projections. Exact widths, identity,
  real change, count conservation, duplicate logical rows and consume ordinal0
  are checked. Pure toggles/reuse never fabricate a ledger instance (D28).
- `item_baseline`: `inventory_revision,instances,paperdoll,auto_shots`. Complete
  canonical instances and valid paperdoll references bind an owner. A baseline
  cannot satisfy formula gates. Disabled GrantItem is rejected before actor
  enqueue; its client JUnit proves RPC status, and a complete owner-bound log
  proves absence of grants. Do not invent a rejected actor command to attest it.
- Last row: exactly `kind=complete,facts,ticks,last_tick`, agreeing with the
  manifest watermark and number of preceding fact rows.

Revision 2 tightens the D46 interface under the accepted review instructions:
RNG and ledger facts now carry the same `resolution_id` as their formula facts.
The key is `(entity,incarnation,resolution_id)`, never tick or owner alone.
An `enchant_outcome` formula additionally requires
`operation={target_before,scroll_before,support_before}` (support may be null),
each a canonical instance at resolution time, consistent with D49's audit capture.
Exactly one correlated `enchant_chance`, draw, scroll debit, optional support debit,
and outcome-dependent target change are required. Destruction verifies source
crystal/refund inputs from E1 grades/items and requires the actual positive refund
row when applicable. Success/reset/no-increment/safe branches have no fabricated
refund; unchanged targets have no D28 ledger row. Sources: GAME@abfde049
RequestEnchantItem.java:123-139,154-271; L2Item.java:353,398-419;
CrystalType.java:26-33. Deletion tests repair sequences/trailer counts.
E2 must implement these correlation additions before producer verification.

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
