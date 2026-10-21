# E6 item evidence handoff — frozen D46 interface

`phase4-evidence.py` is a strict consumer of the **frozen D46** E2
`nightfall-replay export --file FILE --session OWNER --items-evidence-out PATH`
interface. The producer must replay/check the complete recording before exporting.
The packaged soak uses the same export without a session filter. The consumer
runs the candidate replay binary's `check` first for scenario runs. It never
builds a binary or accepts producer PASS flags. `SIM_PHASE4_RULES_HASH` and
`SOAK_PHASE4_RULES_HASH` are the compiled runtime hash, not the generated data hash.

JSONL contract (schema version 1 of this sidecar):

- Integers (D80): every integer whose value lies outside ±2^53 is encoded as
  a canonical decimal string (`-?(0|[1-9][0-9]*)`, no sign `+`, no leading
  zeros, no exponent); integers inside (−2^53, 2^53) stay JSON numbers, and a
  decimal string for such a value refuses. Exactly ±2^53 may use either form.
  The consumer parses both forms for out-of-range values in EVERY numeric
  field — manifest/watermark/trailer counters, fact `incarnation/tick/sequence`,
  RNG `word_before/word_after/bound/value`, delta `instance/count_delta/
  operation_ordinal` and per-cause `count_delta`, every canonical ItemInstance
  integer (`id,template_id,count,enchant,augment_id,charges,created_tick,
  revision`) in baselines, deltas and enchant bindings, baseline
  `inventory_revision`, paperdoll ids and `auto_shots`, context ids, formula
  `inputs` integer and `{numerator,denominator}` positions (schema-directed by
  the input schema) and formula `actual` values (schema-free: only
  out-of-range decimal strings convert, so text such as an outcome name never
  does). Text fields (`resolution_id`, receipts, causes, UUIDs) are never
  converted. Domain bounds apply after decoding (formula inputs stay
  signed-bigint; instance ids < 2^63).
  Exact-numeric formula inputs (rationals) follow the same rule: a JSON
  integer, a `{numerator,denominator}` object, a canonical rational string
  `-?(0|[1-9][0-9]*)/[1-9][0-9]*`, or a wide decimal string (|n| ≥ 2^53). An
  in-range integer string (`"100"`), decimal-point or exponent forms
  (`"1.25"`, `"1e2"`), signs `+`, leading zeros and zero denominators refuse
  (review-2 P3-3); E1 decimal-point vectors are exported as `n/d`.
  `bow_damage.actual.distance_l2_lower/upper` (certified 2^-bits √ bounds) is
  the motivating case.
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
  D81: `lethal.inputs.full_residue` and `half_residue` are nullable, with the
  rule "null iff the corresponding draw was not taken": the full draw is taken
  only when `eligible`, the half draw only when the full draw missed. A
  sentinel (the withdrawn 999, or any value) for an untaken draw refuses, and
  null for a taken draw refuses; 999 remains a legitimate taken residue
  (bound 1000).
- `item_rng`: `callsite,word_before,word_after,bound,value,resolution_id`. Preserve the complete
  ordered draw trace, including intervening non-item draws when needed for cursor
  continuity. Draw-free branches have no fabricated draw. Enchant always requires one bound100000
  draw at callsite `enchant`, including guaranteed success (GAME@abfde049
  EnchantScroll.java:149-155). Its residue must equal the outcome input.
  T10/B3 (zone 844e541): with item evidence on, EVERY generator call is an
  `item_rng` fact, so the cursor is gap-free from the first fact to the last
  (each `word_before` equals the previous fact's `word_after`; the rule is
  unchanged). A caller's own fact covers the draws it tabulates. Every other
  draw is a zone fact: `entity` = the nil UUID, `origin=live`, `incarnation=1`,
  `callsite` = `zone_roll`, `entity_id` or the caller's own site
  (`[a-z][a-z0-9_]*`), `resolution_id` = `zone:<callsite>:<word_before>`
  (decimal). An `entity_id` draw is one 128-bit id: exactly 4 words, `bound=1`,
  `value=0`. A `zone_roll` or caller-site draw has `value < bound`, may span
  several words (rejection resampling), and a caller site with a pinned bound
  (D109 list) keeps that bound. The nil subject carries no other fact kind,
  and a non-nil subject may not use the `zone_roll`/`entity_id` callsites or a
  `zone:` resolution. Zone facts never correlate with an owner's resolution.
  The exporter drops D65 `creation_origin` from published instance rows; the
  canonical instance scalars are unchanged.
  B4 (E3.4/D47, zone b63f0ec): a V6 skill-book learn pays through the item
  ledger. The API payment family `skill_learn_payment {receipt, skill_id,
  skill_level, count_delta}` exports as the Consume cause
  `item_cast:learn:<request key>` (canonical lowercase UUID). Each paid instance is one
  Consume aggregate (D66/D94 shape) with `resolution_id = learn:<key>` and no
  item receipt (`receipt_refs` lists only other receipt-bearing causes). On the
  same tick, exactly one `learned_item_cost` fact resolves as `learn:<key>`, with
  `inputs.phase = actual.phase = complete` and `repeat=false`. When accepted,
  `actual.consumed` (instance id → count) equals the summed learn-cause debits,
  every paid instance has template `consume_id`, and the total is
  `consume_count`. A refused learn pays nothing. A `learn:` resolution belongs
  only to that family and key; a learn payment without its fact, a fact without
  its payment, another key, or another tick all refuse. Book use (8618
  `use:no_handler`) stays D12 fail-closed. `4-item-skill-learn.nfs` selects the
  fact with the phase `book-learn` (inputs plus the Consume of 8618).
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
  D76/D83: `operation_ordinal` is the row POSITION 0..n−1 in this chain, for
  EVERY chain (with or without a Consume aggregate; the aggregate therefore
  has the highest ordinal). One logical operation producing two rows on one
  instance takes consecutive ordinals. The first before equals tick-start
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
  Equip `10→10` (inventory→lhand) then Consume `10→7`.
  D94 amends the tail: the chain shape is [non-terminal rows in position
  order…, Consume aggregate, at most ONE terminal row]. Terminal rows are
  `destroy`/`crystallize` (Phase 5: whole transfer-out); a terminal row is
  always last and, with a Consume aggregate present, immediately follows it
  (refine payment then Crystallize; Consume then Destroy of the remainder).
  A terminal row before Consume, a non-terminal row after Consume, a second
  Consume or two terminal rows refuse.
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
  D144 names the re-admission boundary: the exported `incarnation` is the
  resurrection counter (1 on every admission), not an admission counter. An
  `item_baseline` of an owner that already has a life in the recording (a
  logout and login, or the adopt baseline of a paused owner) is the D71
  boundary: `charges` may clear to 0 under the D72 clear-only rule, and the
  baseline carries `auto_shots=[]` (the restore drops the toggles; a
  surviving toggle refuses). A `charges` change that any other observation
  shows, with no such baseline between the two observations, refuses as
  drift (engine-review-11 P1-2).
  The `inventory_history` fold at that baseline (D142): when the baseline's
  `inventory_revision` is the retained revision plus one, the admission took
  the reset step; every retained instance with charges is expected at that
  revision with charges 0, and every other instance unchanged. A toggles-only
  reset steps with no instance change, which the consumer cannot tell from a
  charged reset, so the fold applies to every plus-one baseline. Any other
  revision must equal the retained one, with the retained state exactly. A
  clear without the step, two steps, or a re-stamped uncharged instance
  refuses (harness-review-7 P1-1).
  Owner binding and the D68 cap also apply to a malformed (non-canonical)
  enchant before binding whose `id` is a positive integer, whichever owner is
  being evaluated.
  ONE shared operation transition table applies to EVERY delta, independently of
  formula correlation. Revision is governed by D55 below; all other canonical
  fields outside the allowed set must remain identical:

  | Operation | Required endpoints / allowed changes |
  |---|---|
  | consume | Existing before; only count changes by the strictly negative cause sum. Remaining count ≥0; after is null iff exhausted. |
  | equip / unequip | Both endpoints; only slot/location change. Equip enters paperdoll; unequip leaves it for inventory. D147: unequip may also change `charges`, only to 0 (the zone's Unequip clears weapon charges, L2J); any other charges change refuses. |
  | toggle_auto_shot | D148, export-only. No instance, no endpoints: `{resolution_id: the toggle request key, template, enabled: bool, inventory_revision: the post-step revision, count_delta: 0}`; `instance`/`before`/`after`/`receipt_ref` are absent or null. It is the tick's owner revision step for a pure toggle (toggle-on with no charge, or toggle-off) and changes no instance. A row that names an instance or is not count-neutral refuses. |
  | enchant | Both endpoints; only enchant changes. Outcome correlation requires the exact source result; unchanged outcomes have no row. Destruction/refund uses separate destroy/grant rows. |
  | refine / cancel_refine | Both endpoints; only augment_id changes. Refine installs a packed option pair; cancel removes it. |
  | destroy | Existing before; strictly negative count_delta. After is null (whole, terminal) or, D100, a partial Destroy: only count changes, to a positive remainder (non-terminal debit). Refunds are separate grant rows. |
  | crystallize | Existing before; after is null. Refunds are separate grant rows. |
  | grant | Canonical after; strictly positive count_delta. Before is null for creation, or canonical for a retained stack with only count changing. |
  | starter / backfill | Before is null; after is canonical. |
  | use (D73) | Both endpoints; only `charges` changes; count unchanged (count_delta 0). |

  D68 bounds every canonical count (baseline instances, enchant bindings and
  both endpoints of every ledger row, which are the only intermediate states
  under D66/D69) by the pinned per-template cap: adena (template 57,
  `Inventory.java:51`) at `MaxAdena = 99900000000` (`character.properties:671`);
  any other E1 item with `stackable=true` at `Integer.MAX_VALUE = 2147483647`
  (`L2ItemInstance.java:422`, the pinned stack ceiling; the E1 tables carry
  `stackable` but no per-template maximum); `stackable=false` items at 1. Python
  integers are unbounded, so sums never wrap (i128-equivalent).
  D74 amends D68 for ledger endpoints: caps are enforced on the live acceptance
  path, so the consumer checks them at the tick-start (first `before`) and
  tick-end (last `after`) endpoints of each `(entity,incarnation,tick,instance)`
  chain, and for every earlier row requires `after.count − (Consume debits
  later in the same chain) ≤ cap`. The D69 export order may therefore show an
  intermediate `after` above the cap only when same-tick consumption brings the
  tick end back under it (C → C+1 → C is legal). Baselines and enchant
  before-bindings are live states and stay capped directly.
  `count_delta` is an exact signed-bigint integer equal to the endpoint count
  difference for every operation. Revision-only rows refuse; item-use COSTS
  still belong in Consume aggregates. D73: a count-neutral `use` row records a
  shot discharge (or any use that changes only the instance's `charges`). It
  carries `receipt_ref` (null for automatic discharges) AND a nonempty ordered
  `causes` list of `{cause,count_delta}` with every `count_delta` exactly 0 and
  the discharge cause grammar of D93 (`shot_discharge:<attack resolution_id>`,
  `cast_discharge:<skill_id>`, or a `receipt:` cause). `receipt_ref` equals the single `receipt:` cause when one
  exists and is null otherwise. `use` rows are count-neutral retained rows, so
  under D69 they may precede a Consume aggregate on the same instance; several
  use rows on one instance in one tick take consecutive positional ordinals.
  A pure toggle fabricates no ledger instance (D28); when it steps the owner
  revision it is exported as a `toggle_auto_shot` row (D148).
  Under D55, correlate deltas by `(resolution_id,instance,operation_ordinal)`
  within the owner/incarnation scope, independent of operation name, where the
  ordinal is the D83 chain position of the row within its tick (so the join is
  `(entity,incarnation,tick,instance,position)` with the resolution identity
  binding it to its formula/RNG facts). Derive ONE
  revision step per `(entity,incarnation,tick)` from the owner's baseline or
  previous changed tick: previous revision → previous revision + 1. Every retained
  post-state written in that tick, across ALL changed instances, must use that
  same new revision. D79: an item's `revision` is its LAST-CHANGE revision —
  untouched items do not advance per tick. A before-state equals the
  instance's retained state exactly, including that last-change revision; on
  the first touch of an instance in a tick it is strictly below the tick's
  step (any earlier step, not necessarily the previous one), and later
  same-tick rows on that instance see the step itself. Baseline instances
  carry revisions ≤ `inventory_revision`; enchant bindings never exceed the
  tick's step. A baseline-free slice binds its step from the tick's
  post-states (befores cannot bind it). Subsequent changed ticks strictly advance the owner
  step even when every changed instance is removed. Per-instance ordinals are
  exactly 0,1,…,n−1 within a tick (D83) and restart at 0 on every tick. Removed instances have no
  fabricated post-state. Never fabricate intermediate revisions.
  Carry complete canonical states forward per owner/incarnation/instance, including
  counts, templates, locations, enchantments and removal. Each next mutation or
  aggregate must start at the established retained state, after recorded intervening
  changes; untouched instances keep their last-change revision (D79 withdraws
  the former tick-end re-stamp). Creation starts from null and removal establishes absence. D149: the
  offline (pre-admission, incarnation 0) rows and the live incarnation-1 rows of one
  entity are ONE state/revision chain through the adopt/admission baseline, in
  either order (offline grant then admission; live session, logout, offline grant,
  re-admission). Later incarnations (resurrections) keep independent
  state/revision histories; allocation freshness,
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

## Next-tranche formula families (zone session 8) — schema revision 17

The zone exports these 14 `item_formula` families next (zone-progress session 7
gap list): `bow_launch`, `bow_reuse`, `magic_damage`, `heal`, `gear_stats`,
`set_shield_history`, `clamp_no_heal`, `charge_transition`, `consumable_use`,
`escape_timing`, `resurrection_scroll`, `learned_item_cost`, `pole_sequence`,
`dual_plan`; `bow_damage` (D80) and `lethal` (D81) are revised with them. Each
fact keeps the frozen shape `formula,inputs,actual,resolution_id,baseline=false`
plus optional `context`. `inputs` must match `INPUT_SCHEMAS[family]` exactly
(required = the E1 oracle function's parameters without defaults, optional =
the rest, absent means default, never null unless the schema says nullable).
`actual` must match `ACTUAL_SCHEMAS[family]` (new in this revision: the shape
of the E1 oracle's return value after `rational()`, checked before exact
oracle equality; numbers are exact integers or `{numerator,denominator}`,
D80 wide integers as decimal strings). Sources: E1 `item_oracle.py` /
`item_behavior_oracle.py` at the rev 8 import (zone 80b8077 → 89df126),
design `docs/plans/phase-4-zone-design.md` §12.1 and the producer in
`state_item_evidence.rs` (`item_formula_v6`, `attack_context_v6`).

| Family | Producer caller / `context.event` | `inputs` (beyond the oracle signature) | `actual` |
|---|---|---|---|
| `bow_launch` | bow attack start, `attack` (+`weapon_template`, `target_entity`) | `weapon` = E1 bow_definition record; `inventory` = ITEM records of candidate arrows; `lhand` occupied arrow id or null; `mp`, `mp_modifier` exact; `reduction_residue` null iff no reduced-MP draw; `miss`; `reuse_remaining` TICKS (D88); `hit_chance` per-mille clamped to [200, 980], hit iff `hit_residue <= hit_chance` (calcHitMiss; D88's `<` revoked); `hit_residue`, `critical_residue`, `spread_residue` (D87, required, null iff the draw was not taken, D89); `blocked` required boolean, true iff perfect shield block, then `spread_residue` is null (D108); RNG callsite `bow_reduced_mp` bound 100 for reduced_mp_consume bows (D109) | `{accepted, reason∈arrows/reuse/mp/launched, inventory, lhand, equipped_lhand, mp, mp_q, mp_paid, arrows_consumed∈0..1, draws, hit, mp_cost?}`; the launch's arrow debit is an `arrow:<resolution_id>` Consume cause |
| `bow_reuse` | bow attack accepted, `attack` | `reuse_delay` (template ms), `attack_speed` exact P.Atk.Spd, `weapon_reuse_modifier` | whole ms `trunc(reuse·333/speed)` (D17) |
| `bow_damage` | bow hit, `attack` | `formula` exact calcPhysDam, `dx_milli`, `dy_milli` signed milli-tiles | `{base, distance_l2_lower, distance_l2_upper, damage}`; bounds are certified 2^-bits rationals, D80 strings |
| `pole_sequence` | pole swing, `attack` | `candidates` in attempt order `{eligible, hit, formula}`, `maximum` secondary count | per eligible attempt `{percent, hit, draws, base?, damage?}` (D25 85% ÷23/20) |
| `dual_plan` | dual swing, `attack` | `hits` pair (hit iff residue <= `hit_chance`, chance in [200, 980], calcHitMiss), `formulas` pair, `second_valid`, `hit_residues` pair, `hit_chance`, `spread_radius`, `critical_residues` pair and `spread_residues` pair (D87, required, entry null iff that subhit's draw was not taken, D89); `blocked` pair of booleans per subhit, true iff perfect block, then that subhit's spread entry is null (D108) | `{plan, stored_second, applied=[first, second|null], second_recomputed, draws, refunded_draws}` |
| `magic_damage` | magic skill hit, `cast` (+`skill_id`, `target_entity`) | exact stats + shot flags + branch inputs; `first_success`/`second_success` null iff their draw was not taken; `is_pvp` required boolean (D89) | `{lower, upper, damage, damage_q, branch}` |
| `heal` | heal effect, `cast` | power/m_atk/shot/grade inputs; `current_hp`, `remainder_q` before; `recoverable_hp` null = uncapped | `{amount, restoration_q, applied_q, current_hp, remainder_q, whole_credit, coefficient, static_bonus, root_lower, root_upper}` |
| `gear_stats` | dirty rebuild, `equip`/`unequip`/`enchant`/`admission` (+`set_id`) | `class_file` pinned path, `level`, `equipment` `[template, enchant, slot, install_seq]`, `skill_refs` `[skill, rank, seq]` winning providers, `base_deltas`, `base_delta_sequence` | `{q, getters, base_stats}`: q/getters = exactly the 11 folded stats (pAtk, mAtk, pDef, mDef, critRate, pAtkSpd, maxHp, maxMp, maxCp, mAtkSpd, runSpd), getters = trunc(q/Q); base_stats STR..MEN after deltas (D103) |
| `set_shield_history` | set listener on slot mutation, `equip`/`unequip` + `set_id` | `previous`, `mutation∈remove/equip_shield/equip_chest`, `chest_present`, `matching_shield`, `complete`, `required_part` | boolean `shield_skill_active` after the mutation (D20) |
| `clamp_no_heal` | dirty-rebuild clamp, event of the rebuild | `current`, `new_max` exact | `min(current,new_max)`; one fact per resource, `resolution_id` suffix `:hp|:mp|:cp` (D91) |
| `charge_transition` | discharge/unequip, `attack`/`cast`/`unequip` | `charges` set before, `use_spirit`, `use_soul`, `landed`, `unequip` | sorted charges after; correlates with the D73 `use` row on the weapon |
| `consumable_use` | UseItem acceptance, `cast` | `definition` (E1 item definition + skills), `tick`, `available` count map, reuse deadline maps, `main_casting`, `condition_context` | `{accepted, consumed, lanes, reason?, item_deadlines?, skill_deadlines?, main_cast_preserved?}`; accepted costs appear as Consume causes `receipt:use:<key>` |
| `escape_timing` | Escape/Blessed Escape cast start, `cast` | `definition`, `start_tick`, `casting_speed`, `available`, `resources`, `incarnation` | `{accepted, inventory, consumed, draws, animation_ms?, launch_ms?, launch_tick?, completion_tick?, interrupt_cutoff_tick?, completion_consumed?, destination?, resources?, incarnation?, movement_canceled?, auto_canceled?, town_respawn?}` |
| `resurrection_scroll` | scroll 737 cast + offer response, `cast` | `definition`, `start_tick`, `available`, death/current incarnation, `dead`, `response∈offer/accept/decline/expire`, `response_tick`, `response_offer_id`, `existing_offer`, `repeat`, resources, `restore_percent`, `restore_hp` ratio | `{accepted, consumed, inventory, draws, reason?, completion_consumed?, offer?, trace?, dead?, incarnation?, hp?, hp_q?, mp?, cp?, xp_refund?, recoverable_xp?}`; one fact on the resurrection-applied tick under the intent's resolution id (D92); HP Q-floor, recorded offer_id, MP preserved or the engine respawn floor as a recorded deviation, optional `max_mp`/`restore_mp` inputs (D104) |
| `learned_item_cost` | learned skill with itemConsume, `cast` | `skill_id`, `rank`, `consume_id`, `consume_count`, `inventory` ITEM records, `phase∈complete/cancel`, `repeat` | `{accepted, inventory, consumed, draws, completion_consumed?, repeated_consumed?, phase?}`; debits are `item_cast:cast:<id>` Consume causes |
| `learned_item_cost` (B4 skill-book learn) | V6 LearnSkill of a rank with `required_items` | same inputs, `phase=complete`, `repeat=false`, `consume_id`/`consume_count` = the book | `{accepted, inventory, consumed, completion_consumed:{}, repeated_consumed:{}, phase:complete, draws:[]}`; resolution `learn:<request key>`; debits are `item_cast:learn:<request key>` Consume causes |
| `lethal` | blow skill, `cast` | D81 nullable residues | `{outcome∈none/full/half, draws}` |

Draw lists in `actual` name `{site,bound,residue}`; when the family also has
`item_rng` facts they share the family's `resolution_id`. The revision 17
producer questions were decided as D87–D93 (below); every family is still
verified by exact oracle equality on the producer's actual inputs.

### Revision 18 — architect decisions D87–D93 on Q-F1..Q-F7

- D87 (Q-F1): `bow_launch.inputs` gains REQUIRED `critical_residue` and
  `spread_residue`; `dual_plan.inputs` gains REQUIRED `critical_residues` and
  `spread_residues` (pairs, one per subhit). Each is nullable only under the
  D89 rule (null iff that draw was not taken: no landed hit, or no spread draw
  for a zero-radius bow). The producer emits the live residues; neutral-case-
  only export is rejected. The E1 rev 8 oracle still hard-codes 999 / radius,
  so the consumer (`residue_family`) runs it and substitutes the fact's
  residues into `draws` by site (`critical`/`spread`, `critical1/2`/`spread1/2`),
  checking each against its draw bound; neither residue alters any other
  output. When E1 adds these parameters the consumer passes them through.
- D88 (Q-F2), as corrected: every duration in evidence is in TICKS
  (`bow_launch.reuse_remaining` included). D88's strict inequality is
  REVOKED (architect-verified GAME@abfde049 `Formulas.calcHitMiss`: miss iff
  `chance < Rnd.get(1000)`): a bow/dual/pole hit happens iff
  `residue <= hit_chance`, with the chance clamped to [200, 980], which is the
  `hit_chance` input domain. The consumer checks this rule itself
  (`hit_rule`) before the oracle; there is no inequality adapter or probe.
  Rev 9 import path (D96): when the E1 oracle signature takes the D87 residue
  parameters, the consumer passes them through; the residue-substitution
  adapter runs only on the pre-D87 data revisions (rules_manifest
  `data_sha256` rev 7 `5b77911e…` or rev 8 `8e27de15…`), and newer data with
  an old-shaped oracle is a ValueError gate verdict. Zone rev 9 (dd3c1be,
  still at codex/phase4-zone-engine 2dc9176) tests hits with `<`; until its
  follow-up restores `<=`, a fact with residue == chance passes the
  consumer's rule but is refused by that oracle (fail closed).
- D89 (Q-F3): D81's rule generalises to every draw-derived field whose draw may
  not occur: the field is null (or absent) iff its draw was not taken, and a
  value for an untaken draw (any sentinel) or a null for a taken draw refuses.
  Covered fields: `lethal.full_residue/half_residue`, `shield.perfect_residue/
  normal_residue`, `shot_payment.residue` (soulshot reduction),
  `bow_launch.reduction_residue/hit_residue/critical_residue/spread_residue`,
  `dual_plan.critical_residues[i]/spread_residues[i]`, and
  `magic_damage.first_success` (taken iff not perfect and magic failures
  enabled) / `second_success` (taken iff the first failed for a player
  attacker). Booleans that are always evaluated are never null:
  `magic_damage.is_pvp` is a plain boolean and REQUIRED (review-2 P3-4), even
  though the oracle has a default for it.
- D90 (Q-F4), amended by D103: a `gear_stats` fact's `actual` is exactly
  `{q, getters, base_stats}`. `q` and `getters` (trunc q/Q) each carry exactly
  the 11 engine-folded stats `pAtk, mAtk, pDef, mDef, critRate, pAtkSpd,
  maxHp, maxMp, maxCp, mAtkSpd, runSpd`; `base_stats` carries `STR, DEX, CON,
  INT, WIT, MEN` after set/skill deltas. The consumer compares exactly these
  keys with the oracle's `q`/`getters`/`base_stats`. `exact`, the fold `trace`
  and every other oracle key are not part of the fact and refuse if present.
- D91 (Q-F5): every `clamp_no_heal` fact's `resolution_id` is
  `<rebuild resolution>:hp|:mp|:cp` (lowercase, nonempty prefix), so the HP,
  MP and CP clamps of one rebuild are distinct formula identities.
- D92 (Q-F6): `consumable_use`, `escape_timing`, `resurrection_scroll` and
  `learned_item_cost` carry exactly ONE fact per intent, on the tick where the
  terminal effect applies (effect application / teleport landing /
  resurrection applied / learn committed), with `resolution_id` = the
  originating intent's receipt or resolution id. The consumer refuses a
  second terminal fact for the same `(entity,incarnation,resolution_id)` and
  any fact of that intent (prepay Consume, draws, other formulas) on a later
  tick than the terminal fact. A fact belongs to the intent when its
  `resolution_id`, or (for ledger rows) its `receipt_ref` / any `receipt_refs`
  entry, equals the terminal fact's `resolution_id`: a receipt-first prepay
  Consume is bound by its receipt cause, not its resolution (D62). Facts are
  correlated within one entity; the entity carrying the resurrection_scroll
  terminal fact (caster vs resurrected target) is to be pinned with Z. The
  tick of the effect itself is not otherwise observable in the sidecar.
- D93 (Q-F7): the exporter attaches `causes` to `use` rows. Use causes are
  `shot_discharge:<attack resolution_id>` (new family), `cast_discharge:<skill_id>`
  (decimal skill id) or `receipt:<operation>:<key>`; Consume payment causes
  (`auto_shot`, `arrow`, `item_cast`) refuse on use rows and discharge causes
  refuse on Consume. An automatic attack discharge row's `resolution_id`
  equals its first cause's resolution id. A `cast_discharge` cause names a
  skill, not a resolution; D96: such a row's `resolution_id` is
  `cast:<cast_id>` and the consumer checks the `cast:` prefix only.
  A use row with any discharge cause may only CLEAR charge bits
  (`after.charges & ~before.charges == 0`); only a receipt-bound use (manual
  shot charge) may add them.
- D104 (Q-Z8-2): `resurrection_scroll` follows the engine's accepted Phase 3
  resurrection semantics. On acceptance HP = max(current, floor at Q resolution
  of `max_hp × restore_hp`); the exact rational and any ceiling refuse.
  `offer_id` comes from the recording (the `offered` trace event; a positive
  integer) and the response is matched against it, never against a
  hard-coded 1. MP: the Phase 3 zone design §1.2 (pinned
  `L2PcInstance.doRevive`, `RespawnRestoreMP` default 0) specifies NO MP
  restore — current MP is preserved. The engine raises MP to its respawn
  floor; that is recorded as a KNOWN Phase 3 DEVIATION (to be re-examined, not
  changed in Phase 4) and accepted as exactly `max(mp, floor(max_mp ×
  restore_mp))` when the optional inputs `max_mp`/`restore_mp` are emitted,
  otherwise as any raise; an MP decrease refuses.
- D108 (engine-review-4): on a perfect shield block the spread draw is not
  taken. `bow_launch.inputs.blocked` (boolean) and `dual_plan.inputs.blocked`
  (one boolean per subhit — harness reading of D108 for the two-subhit
  family) are REQUIRED; blocked is true iff that hit was perfectly blocked,
  requires a landed hit, and then the corresponding spread residue is null
  (D89). The consumer passes `blocked` to the oracle when its signature takes
  it; otherwise it gives the oracle a placeholder spread residue and drops the
  blocked spread draw from the expected `draws`.
- D109: item_rng callsite `bow_reduced_mp` (pinned `Rnd.get(100)` of the 21
  reduced_mp_consume bows) is accepted. Callsites with pinned bounds are
  checked (`enchant` 100000, `bow_reduced_mp` 100); other callsites remain
  accepted as before.
- D104 at the zone final head (82e1ef6): `resurrection_scroll.inputs.offer_id`
  is REQUIRED (nullable positive integer): the id of the actual offer, which
  the E1 oracle now takes. The oracle encodes the engine revive: whole HP =
  max(1, floor(max_hp × restore_hp)) with the Q remainder cleared (this
  supersedes the "Q-resolution floor" wording above), XP refund rounded half
  up and capped by the loss, MP/CP unchanged. The consumer passes oracles that
  take `offer_id` straight through and applies the same rule over older
  oracles (rev 7/8). The MP respawn-floor deviation acceptance is unchanged.
- D100: non-Consume debits are not ordered against credits. Before the
  Consume aggregate a chain may hold any non-terminal rows — credits,
  count-neutral rows and non-Consume debits such as a partial Destroy — in
  either order (e.g. the zone's [partial Destroy, Consume] and [partial
  Destroy, Grant] ticks); only the D94 shape (Consume last before at most one
  terminal row, terminal = a whole Destroy/Crystallize with after null) and
  the D74 per-chain caps constrain order, with endpoint continuity and
  conservation as before.
- D111: `gear_stats` facts are emitted only inside the D110 export guard
  (base-class player; no timed effect or death-penalty source; no grade or
  overload penalty; no augment, enchant option or armor set with base-stat
  deltas) and carry `context {event: equip|unequip|enchant}` under the
  command's resolution id; there is no `set_id` and no admission gear fact.
  Scenario phases therefore select gear facts by `context.event`, the
  `equipment` map and the correlated ledger `mutation`; the phase4-armor owner
  is a level-20 Human Mystic with Expertise D (skill 239 rank 1).
