# Replay evidence (C14)

```sh
cargo run --bin nightfall-replay -- export \
  --file apps/api/fixtures/sessions/phase3-gameplay-v5.nfr \
  --evidence-out /tmp/phase3-gameplay.evidence.jsonl
```

The destination must be fresh. JetStream export also accepts `--evidence-out` alongside its
existing `--out recording.nfr`; evidence always covers the whole recording, even if
`--session` checks that a particular owner occurs in it. File evidence export accepts neither
`--out` nor `--live`. No live character state, current data directory, database, clock or
independent oracle file is used to produce evidence.

The exporter restores the recorded snapshot and runs the recorded commands. Every tick must
reproduce its complete record via `AppliedTickRecord::reproduced_by`, including dispositions,
private events, observer output bytes (or stored digests), and state digest. The watermark,
zone, epoch, contiguous ticks and policy must agree. Evidence is buffered until the entire
recording verifies; divergence produces a nonzero exit and no new evidence file. A capture
watermark proves the recorded prefix, not the end of a still-running epoch. Failed file writes
remove the newly created destination; a consumer must also require the completion trailer.

The opt-in domain recorder is ephemeral. It does not enter snapshots, state digests, replay
record encoding, persistence or public wire output. Legacy policies produce no Phase 3 facts.
An initial snapshot is a baseline; the producer does not invent admission, learning, death,
or other transitions that happened before it. Snapshot cache reconstruction at the first
replayed tick can produce `modifier_reconstruction`.

## JSON Lines schema 1

Each line is one JSON object. Integers retain their exact integer representation; local exact
ratios are strings `numerator/denominator`. Exact root expressions are
`{offset,coefficient,radicand}`, meaning `offset + coefficient * sqrt(radicand)`; each component
is a rational string. Q quantities are integers with Q = 1,000,000
(`apps/api/src/domain/zone/scaled.rs:13`). Consumers must use exact integer/rational arithmetic.
A JSON object is a keyed map, not an ordered sequence. Lists preserve execution order.

The first line has `kind: "manifest"`, `schema_version: 1`, `recording_sha256` of the **exact
input file bytes**, `zone`, `epoch`, `snapshot_tick`, `watermark`, `snapshot_meta`, `rules`,
`classes`, `phase3_rules`, and `scope: "whole_recording"`. The immutable registries are included
once so consumers can inspect source profiles, configured policy, skill metadata and class
growth without loading a different build's data. They retain their recorded provenance.

Fact lines have exactly `kind`, `entity` (UUID), `tick`, `sequence` (zero-based consecutive
integer), and `data`. Multiple facts of the same kind/entity/tick are valid: use `sequence`
for identity and order. A resolution can have multiple outcomes or multiple same-site draws.
The final line is `{"kind":"complete","facts":N,"ticks":T,"last_tick":...}`. No line carries
a producer acceptance/PASS verdict or a guessed oracle-vector index.

| Kind | `data` fields and interpretation |
| --- | --- |
| `regen_pulse` | `cause=scheduled_regeneration`; `inputs` includes player/NPC, exact base HP/MP/CP, level/CON/MEN factors and `posture` (`Standing` or `Moving`) at the resource phase **before movement**. `rules` contains the recorded multipliers; `functions` is the ordered calculator. `before.hp/mp`, nullable `before.cp={current,maximum}`, `max_hp/mp`, `fractions_before`, `amounts.hp_q/mp_q/cp_q`, and `actual.hp/mp/cp/fractions/hp_granted/mp_granted/cp_granted` expose both uncapped formula credit and actual whole credit/remainders. NPC CP is null. Full-resource pulses still count; dead entities have no due pulse. |
| `death_record` | `incarnation`, `pre_death_xp`, `loss` (actual recoverable loss), pre-death `level`, `killer`, `inside_peace`, `victim_classification`, `penalty_before`, nullable `penalty_roll`, and `after` (player policy including penalty level and death record; null for NPC). Null roll means no draw, not a failed chance roll. The death penalty's outcome is the before/after level pair. |
| `modifier_reconstruction` | `before.entity` and `before.word_cursor`; `after` context; `word_cursor_after`; `ordered_sources` is an array of `[install_sequence, {kind,value?}]` in numeric sequence order. Source kinds are `passive`, `effect`, `death_penalty`. `functions` is the actual fold order: stat, priority, install sequence, handler ordinal, function ordinal. Effect source kind/attribution and periodic phase are retained in `after.phase3.effects`. A rebuild can clamp resources; it does not claim that all resource changes are hook credits. |
| `admission_reconstruction` | The recorded `load` (including optional paused checkpoint) and actual reconstructed `after` context. Compare retained relative source order, rebased periodic phase and death record with the load; entity incarnation and effect IDs can change on admission. |
| `resource_hook` | Actual one-time resource hook execution: `skill_id`, `rank`, `handler`, `resource`, `before`, `fraction_before`, exact `amount`, `after`, `whole_credit`, `fraction_after`. This is emitted at the live effect hook, never by reconstruction. |
| `hate_delta` | Subject is NPC; `source` is the contributor, `requested` contains hate/damage, `ledger_before/after` contain complete keyed ledgers, `amount` is actual capped hate increase. `direction=add`; `reason` is `aggro`, `effect_point`, `attacked`, `damage`, `auto_attacked`, `heal`, or `add_hate`. Removals instead have `direction=remove`, `removed_row`, unsigned `amount`, and reason `target_released`, `npc_died`, `return_home`, or `despawn`; the row is deleted, not retained at zero. |
| `hate_selection` | Immediate ledger selection: `ledger`, eligible entity IDs, `current_target`, and nullable `most_hated` life binding. This selection is queued for AI; a later forced intention can supersede it. |
| `formula_resolution` | Subject is caster. `inputs` includes pre-resolution caster/target contexts, full skill definition and rank. `target`, `skill_id`, `rank`, optional `continuous=[landed,duration_seconds]`, ordered `outcomes`, complete `rng_draws` for that recipient resolution, and target context `after`. The slice starts before continuous reflection/success/duration and includes instant handlers and recipient/death consequences. Each outcome preserves rolled damage, CP absorbed, HP removed, restoration and applied IDs. |
| `effect_formula` | Actual continuous success/duration evaluation: `chance_evaluation={inputs,chance}` (null for unconditional success), `landed`, `duration_seconds`, exact `duration_inputs`, and its ordered `rng_draws`; identifies skill/rank/target. |
| `damage_formula` | Actual physical/magical damage intermediates: `formula`, skill/rank/target, power and attack/defence Q inputs, critical flag and rolled damage; physical also retains exact damage, magic retains success rate, level difference, resolution and spread. The enclosing `formula_resolution` supplies the complete caller inputs and post-damage RNG tail. |
| `auto_attack_resolution` | `inputs.source`, target entity baseline and swing; resulting ordered `events` and complete impact `rng_draws`. Canceled impacts can have no draws. |
| `rng_draw` | Every instrumented Phase 3 formula draw, also outside impact (e.g. `AffectLimit`): `callsite`, exclusive `bound`, `value`, target life, decimal-string `word_before/after`. Rejection sampling may advance multiple words. The copies inside resolution slices are references to these same draws, not additional consumption. NPC spawn/AI randomness is not a formula draw and can create gaps between cursors. |
| `resurrection_offer` | `offer` includes ID, life, percent Q and frozen recoverable XP; exact `wit_bonus`, skill reference, recorded `definition`, and death record. |
| `resurrection_refund` | `offer`, `accepted`, `xp_before`, `expected_xp_refund` from the staged mutation, actual `xp_after`, `incarnation_after`, and resulting player policy `after`. A decline has zero refund; a successful acceptance consumes the retained loss once. |
| `learning_commit` | `account`, `request_key`, and the reproduced frozen `result` from the recording's private `SkillLearned` fact. Rejected learning commands do not emit a success. This proves the recorded simulation fact, not an independent database audit. |

A context contains `entity`, canonical `phase3`, derived `effective` (including calculator
functions and traits), and optional NPC `hate`. Function operations retain Q integers or exact
ratios; special builtins retain their typed inputs. Source fields retain their existing domain
serde names. This evidence contains private server facts and is not a replacement public wire
contract.

## Source anchors and consumer responsibilities

Pinned game revision: `abfde0490ac52a2106e884b7242c0276c44cdf76`; datapack revision:
`3ca488dd2bd0bfaca43e378886a3c2e37968153a`. Paths below are relative to each repository:

- Regeneration profile/posture/fold: game `src/main/java/com/l2jserver/gameserver/model/stats/Formulas.java:263`, `:334`, `:351`, `:357`, `:442`; Q accumulation is the binding Nightfall architecture decision D-R12, not a claim about the Java resource representation.
- Modifier insertion order: game `src/main/java/com/l2jserver/gameserver/model/stats/Calculator.java:110` and `:117`; live handler hooks: datapack `src/main/java/com/l2jserver/datapack/handlers/effecthandlers/pump/MaxHp.java:50` and `:83`.
- Damage/RNG callers: game `src/main/java/com/l2jserver/gameserver/model/stats/Formulas.java:746`, `:815`, `:992`, `:1158`, `:1286`; datapack `src/main/java/com/l2jserver/datapack/handlers/effecthandlers/instant/MagicalAttack.java:79` and `:85`, `PhysicalAttack.java:104`.
- Death penalty source draw: game `src/main/java/com/l2jserver/gameserver/model/actor/instance/L2PcInstance.java:9363`; the normalized draw and Phase 3 eligibility/cap follow the binding design. XP loss is the recorded existing Nightfall table via `apps/api/src/domain/zone/progression.rs:42`.
- Hate caller/ledger: game `src/main/java/com/l2jserver/gameserver/model/actor/L2Character.java:4603` and `src/main/java/com/l2jserver/gameserver/model/actor/L2Attackable.java:556`, `:606`, `:666`.
- Resurrection percent: game `src/main/java/com/l2jserver/gameserver/model/stats/Formulas.java:1372`; refund rounding: game `src/main/java/com/l2jserver/gameserver/model/actor/instance/L2PcInstance.java:4702`.

C11 consumers must verify the manifest hash, schema, complete trailer, consecutive sequence,
entity/tick scope and required raw facts, then perform their independent expectations. Missing
facts, unsupported independent oracle inputs, unknown categories or incomplete files must fail
closed. Do not count a draw twice because it also appears in a resolution slice. The older
harness proposal using `trace --evidence-out`, one JSON document and gate-named rows needs an
adapter to this binding C14 export/JSONL schema; this producer deliberately does not manufacture
oracle-vector matches or acceptance flags.

## Phase 4 item evidence (D46)

```sh
nightfall-replay export --file epoch.nfr --items-evidence-out /tmp/items.jsonl
```

This separate JSONL format follows the [frozen E6 contract](../../apps/client-unreal/Scripts/phase4-evidence-contract.md).
It verifies every recorded output and state digest before creating a fresh destination.
`--session` checks that the owner occurs in the recording; facts still cover the whole
recording. Partial/live captures, historical schemas, inconsistent completion watermarks,
RNG gaps (including an omitted first or final draw), and replay divergence refuse export.
RNG cursors are full-width JSON integers. The old `--evidence-out` format remains separate.

The current producer captures initial/admission inventories, accepted item operation
changes before consume aggregation, live grants and starter/backfill materialization,
item/learned-cast payments, exact enchanting inputs/outcomes/refunds, and intervening
combat/effect/NPC randomness. Other formula families remain required work; the consumer
rejects a gate whose formula observations are absent. Successful file creation alone is
not Phase 4 acceptance. Offline grants currently refuse this export because their canonical
incarnation is zero and D46 requires a positive incarnation. Same-tick enchant revisions
also require consumer review: multiple accepted operations share one inventory revision.
The incarnation reset (D142) is evidence for the API ledger only. The item evidence export
does not contain it (D145).
An automatic shot recharge at the start of an attack or cast writes a Use row on the weapon with no
receipt. The export gives it the cause `shot_charge:<resolution>` only when the same tick has the
`auto_shot_payment` consume of that shot with the same resolution, and the row only adds charge
bits. Other Use rows without a cause stop the export (T50).
A pure ToggleAutoShot (no charge row) that steps the inventory revision exports one `item_delta`
with the operation `toggle_auto_shot`. This row has no instance. It has the template, the enabled
flag, the new revision and count delta 0 (D148).
`nightfall-replay coverage` writes schema 4 for V6 recordings. Schema 4 adds the item counters
`item_operations`, `enchant_outcomes`, `refine_outcomes` and `shot_payments`, and the item scalars
`item_ledger_rows`, `item_allocations` and `inventory_baselines` (T48). V5 recordings keep schema 3.

The real engine probe can emit a disposable recording for CLI/consumer verification:

```sh
NIGHTFALL_TEST_ITEM_EVIDENCE_RECORDING=/tmp/items-probe.nfr \
  cargo test -p nightfall-api --lib v6_item_evidence_captures -- --test-threads=2
```
