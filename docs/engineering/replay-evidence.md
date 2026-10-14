# Replay evidence (C14)

```sh
cargo run --bin nightfall-replay -- export \
  --file apps/api/fixtures/sessions/phase3-gameplay-v5.nfr \
  --evidence-out /tmp/phase3-gameplay.evidence.jsonl
```

The destination must be new. JetStream export also accepts `--evidence-out` together with its
usual `--out recording.nfr`. The evidence always covers the full recording, also when
`--session` makes sure that a specified owner occurs in it. File evidence export does not accept
`--out` or `--live`. The export uses no live character state, current data directory, database, clock or
independent oracle file to make evidence.

The exporter restores the recorded snapshot and runs the recorded commands. Each tick must
make its complete record again through `AppliedTickRecord::reproduced_by`. This includes the dispositions,
the private events, the observer output bytes (or the stored digests), and the state digest. The watermark,
zone, epoch, contiguous ticks and policy must agree. The exporter keeps the evidence in a buffer until it verifies the full
recording. If a divergence occurs, the exit is nonzero and there is no new evidence file. A capture
watermark proves the recorded prefix. It does not prove the end of an epoch that continues to run. If a file write fails,
the exporter removes the new destination. A consumer must also make sure that the completion trailer is there.

The opt-in domain recorder is ephemeral. It does not go into snapshots, state digests, the encoding of replay
records, persistence or public wire output. Legacy policies make no Phase 3 facts.
An initial snapshot is a baseline. The producer does not make up admission, learning, death,
or other transitions that occurred before it. The reconstruction of the snapshot cache at the first
replayed tick can make `modifier_reconstruction`.

## JSON Lines schema 1

Each line is one JSON object. Integers keep their exact integer representation. Local exact
ratios are strings `numerator/denominator`. Exact root expressions are
`{offset,coefficient,radicand}`, which means `offset + coefficient * sqrt(radicand)`. Each component
is a rational string. Q quantities are integers with Q = 1,000,000
(`apps/api/src/domain/zone/scaled.rs:13`). Consumers must use exact integer/rational arithmetic.
A JSON object is a keyed map, not an ordered sequence. Lists keep the order of execution.

The first line has these fields:

- `kind: "manifest"` and `schema_version: 1`.
- `recording_sha256` of the **exact input file bytes**.
- `zone`, `epoch`, `snapshot_tick`, `watermark` and `snapshot_meta`.
- `rules`, `classes` and `phase3_rules`.
- `scope: "whole_recording"`.

The immutable registries are included
one time. Thus consumers can examine source profiles, configured policy, skill metadata and class
growth. They do not have to load the data of a different build. The registries keep their recorded provenance.

Fact lines have exactly `kind`, `entity` (UUID), `tick`, `sequence` (a zero-based consecutive
integer), and `data`. More than one fact of the same kind/entity/tick is valid. Use `sequence`
for identity and order. A resolution can have more than one outcome or more than one draw at the same site.
The final line is `{"kind":"complete","facts":N,"ticks":T,"last_tick":...}`. No line has
a producer acceptance/PASS verdict or a guessed oracle-vector index.

| Kind | `data` fields and interpretation |
| --- | --- |
| `regen_pulse` | `cause=scheduled_regeneration`. `inputs` includes player/NPC, the exact base HP/MP/CP, the level/CON/MEN factors and `posture` (`Standing` or `Moving`) at the resource phase **before movement**. `rules` has the recorded multipliers. `functions` is the ordered calculator. `before.hp/mp`, the nullable `before.cp={current,maximum}`, `max_hp/mp`, `fractions_before`, `amounts.hp_q/mp_q/cp_q`, and `actual.hp/mp/cp/fractions/hp_granted/mp_granted/cp_granted` show both the formula credit without a cap and the real whole credit/remainders. NPC CP is null. Pulses at full resource still count. Dead entities have no due pulse. |
| `death_record` | `incarnation`, `pre_death_xp`, `loss` (the real recoverable loss), the `level` before death, `killer`, `inside_peace`, `victim_classification`, `penalty_before`, the nullable `penalty_roll`, and `after` (the player policy, which includes the penalty level and the death record; null for NPC). A null roll means that there was no draw. It does not mean a failed chance roll. The outcome of the death penalty is the before/after level pair. |
| `modifier_reconstruction` | `before.entity` and `before.word_cursor`. The `after` context. `word_cursor_after`. `ordered_sources` is an array of `[install_sequence, {kind,value?}]` in numeric sequence order. The source kinds are `passive`, `effect`, `death_penalty`. `functions` is the real fold order: stat, priority, install sequence, handler ordinal, function ordinal. `after.phase3.effects` keeps the effect source kind/attribution and the periodic phase. A rebuild can clamp resources. It does not claim that all resource changes are hook credits. |
| `admission_reconstruction` | The recorded `load` (this includes the optional paused checkpoint) and the real reconstructed `after` context. Compare the kept relative source order, the rebased periodic phase and the death record with the load. The entity incarnation and the effect IDs can change on admission. |
| `resource_hook` | The real one-time run of a resource hook: `skill_id`, `rank`, `handler`, `resource`, `before`, `fraction_before`, the exact `amount`, `after`, `whole_credit`, `fraction_after`. The live effect hook emits this fact. Reconstruction never emits it. |
| `hate_delta` | The subject is the NPC. `source` is the contributor. `requested` has the hate/damage. `ledger_before/after` have the complete keyed ledgers. `amount` is the real hate increase after the cap. `direction=add`. `reason` is `aggro`, `effect_point`, `attacked`, `damage`, `auto_attacked`, `heal`, or `add_hate`. Removals have `direction=remove`, `removed_row`, an unsigned `amount`, and the reason `target_released`, `npc_died`, `return_home`, or `despawn`. The row is removed. It is not kept at zero. |
| `hate_selection` | The immediate ledger selection: `ledger`, the eligible entity IDs, `current_target`, and the nullable `most_hated` life binding. This selection goes into the queue for AI. A later forced intention can replace it. |
| `formula_resolution` | The subject is the caster. `inputs` includes the caster/target contexts before resolution, the full skill definition and the rank. Also: `target`, `skill_id`, `rank`, the optional `continuous=[landed,duration_seconds]`, the ordered `outcomes`, the complete `rng_draws` for the resolution of that recipient, and the target context `after`. The slice starts before the continuous reflection/success/duration. It includes the instant handlers and the recipient/death consequences. Each outcome keeps the rolled damage, the CP absorbed, the HP removed, the restoration and the applied IDs. |
| `effect_formula` | The real evaluation of continuous success/duration: `chance_evaluation={inputs,chance}` (null for unconditional success), `landed`, `duration_seconds`, the exact `duration_inputs`, and its ordered `rng_draws`. It identifies the skill/rank/target. |
| `damage_formula` | The real intermediate values of physical/magical damage: `formula`, skill/rank/target, the power and attack/defence Q inputs, the critical flag and the rolled damage. Physical damage also keeps the exact damage. Magic keeps the success rate, the level difference, the resolution and the spread. The `formula_resolution` that contains it gives the complete caller inputs and the RNG tail after damage. |
| `auto_attack_resolution` | `inputs.source`, the target entity baseline and the swing. The ordered `events` that result, and the complete impact `rng_draws`. Canceled impacts can have no draws. |
| `rng_draw` | Each instrumented Phase 3 formula draw, also outside an impact (for example `AffectLimit`): `callsite`, the exclusive `bound`, `value`, the target life, the decimal-string `word_before/after`. Rejection sampling can advance more than one word. The copies in the resolution slices are references to these same draws. They are not more consumption. NPC spawn/AI randomness is not a formula draw, and can make gaps between cursors. |
| `resurrection_offer` | `offer` includes the ID, the life, the percent Q and the frozen recoverable XP. Also the exact `wit_bonus`, the skill reference, the recorded `definition`, and the death record. |
| `resurrection_refund` | `offer`, `accepted`, `xp_before`, `expected_xp_refund` from the staged mutation, the real `xp_after`, `incarnation_after`, and the player policy `after` that results. A decline has zero refund. A successful acceptance uses the kept loss one time. |
| `learning_commit` | `account`, `request_key`, and the reproduced frozen `result` from the private `SkillLearned` fact of the recording. Rejected learning commands do not emit a success. This fact proves the recorded simulation fact. It is not an independent database audit. |

A context has `entity`, the canonical `phase3`, the derived `effective` (this includes the calculator
functions and traits), and the optional NPC `hate`. Function operations keep Q integers or exact
ratios. Special builtins keep their typed inputs. Source fields keep their domain
serde names. This evidence has private server facts. It does not replace the public wire
contract.

## Source anchors and consumer responsibilities

Pinned game revision: `abfde0490ac52a2106e884b7242c0276c44cdf76`. Datapack revision:
`3ca488dd2bd0bfaca43e378886a3c2e37968153a`. The paths below are relative to each repository:

- Regeneration profile/posture/fold: game `src/main/java/com/l2jserver/gameserver/model/stats/Formulas.java:263`, `:334`, `:351`, `:357`, `:442`. The Q accumulation is the binding Nightfall architecture decision D-R12. It is not a claim about the Java resource representation.
- Modifier insertion order: game `src/main/java/com/l2jserver/gameserver/model/stats/Calculator.java:110` and `:117`. Live handler hooks: datapack `src/main/java/com/l2jserver/datapack/handlers/effecthandlers/pump/MaxHp.java:50` and `:83`.
- Damage/RNG callers: game `src/main/java/com/l2jserver/gameserver/model/stats/Formulas.java:746`, `:815`, `:992`, `:1158`, `:1286`. Datapack `src/main/java/com/l2jserver/datapack/handlers/effecthandlers/instant/MagicalAttack.java:79` and `:85`, `PhysicalAttack.java:104`.
- Death penalty source draw: game `src/main/java/com/l2jserver/gameserver/model/actor/instance/L2PcInstance.java:9363`. The normalized draw and the Phase 3 eligibility/cap follow the binding design. The XP loss is the recorded Nightfall table through `apps/api/src/domain/zone/progression.rs:42`.
- Hate caller/ledger: game `src/main/java/com/l2jserver/gameserver/model/actor/L2Character.java:4603` and `src/main/java/com/l2jserver/gameserver/model/actor/L2Attackable.java:556`, `:606`, `:666`.
- Resurrection percent: game `src/main/java/com/l2jserver/gameserver/model/stats/Formulas.java:1372`. Refund rounding: game `src/main/java/com/l2jserver/gameserver/model/actor/instance/L2PcInstance.java:4702`.

C11 consumers must verify these items first:

- The manifest hash, the schema and the complete trailer.
- The consecutive sequence and the entity/tick scope.
- The necessary raw facts.

Then the consumers do their independent expectations. The consumer must fail closed for these conditions:
missing facts, independent oracle inputs that are not supported, unknown categories, or files that are not complete. Do not count a draw two times
because it also occurs in a resolution slice. The older harness proposal uses `trace --evidence-out`, one JSON document
and rows with gate names. This proposal must have an adapter to this binding C14 export/JSONL schema. This producer intentionally
does not make oracle-vector matches or acceptance flags.
