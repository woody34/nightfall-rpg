# Actual-input item oracle handoff

`packages.data.scripts.item_oracle` exposes Fraction functions, not a lookup of
fixture outcomes. Inject the replay export's actual stats, positions, source
chance, pre-attempt enchant and residues. Results include exact intermediates and
caller truncations. Float inputs reject. Formula callers must retain their source
order; do not combine bow/pole casts or reuse an unbound shot.

- F1–F3: `gear_stats(Source(), class_file, level, equipment, skill_refs,
  base_deltas)`; equipment tuples are `(template_id,enchant,concrete_slot,install_sequence)`.
  Skill refs are `(skill_id,rank,install_sequence)`, already resolved to the winning
  provider. This isolates the equipment/source fold; callers supply mastery,
  penalty, set, augmentation and active overlap providers explicitly. No automatic
  expertise or mastery is inferred. `enchant_increment`, `enchant_hp`,
  `clamp_no_heal`, `set_shield_history` expose individual boundaries/history.
- F4–F7: `enchant_chance`, `enchant_outcome`, `enchant_valid`, `crystal_count`,
  `failure_refund`, `max_load`, `overload`, `grade_penalties`. `Source.chance`
  independently reads the pinned range row. Overload accepts retained prior rank
  and boolean; `maxLoad<=0` returns a source no-op.
- F8–F10: `physical_damage`, `magic_damage`, `cast_time`, `heal`, `shield`,
  `position`, `angle_contains`, `fist_spread`, `bow_reuse`, `bow_damage`,
  `pole_damage`, `pole_sequence`, `blow_success`, `lethal`, `charge_transition`.
  `bow_damage` converts Euclidean milli-tile distance using exact32/1000 L2 units,
  certifying the final integer with root intervals. `pole_damage` first truncates
  the formula and then scales. Shield returns consumed draw sites and skips normal
  on a perfect result. Angle comparisons reject coordinates outside the hashed bound.
- F11–F12: `retail_select`, `retail_distribution`, `category`, `pack_options`,
  `cancel_fee`, `checked_u32`, `validate_allocator`. Retail selector is the pinned
  descending tail. Distribution partitions every integer residue exactly, including
  under-summed final rows; tests also exhaust all10000 samples independently.

`fixtures/item-oracle.json` has source hashes, explicit inputs, rational
numerator/denominator objects, source citations, draw sites and vector hashes.
The 24 named gear vectors are only the minimum isolated matrix. The remaining
vectors enumerate source grade/enchant/HP/chance/load/retail/SA boundaries.

Data foundation does not claim live client/replay agreement, durable mutation
ordering, protobuf carrier byte preservation (F13), or a full encoded snapshot
budget. Those require the engine/API/harness integration and independent review.
Capability `admitted` describes the generated V6 evaluator contract, not a claim
that an unintegrated runtime already implements it. Every named blocker remains
fail-closed; O1-required blocked paths must be completed before release.

`consumable_definition(Source(), template_id)` exposes independently expanded
source item/skill costs, delays, shared reuse, cast-lane/static flags and effect
parameters. `verify_facts` (or `item_oracle.py --facts PATH`) accepts a nonempty
JSON array of `{formula,inputs,expected}` records, evaluates a finite formula
registry on arbitrary actual inputs and rejects missing, unknown, float or
mismatched evidence. Exact inputs/results use `{numerator,denominator}` objects.
Each vector has an identity/hash; the unequal-finger injection proves source
RFINGER duplication, and all four penalty ranks retain the same equipment fold
across living/death/dead-admission/resurrection inputs. This is formula evidence,
not a claim that those runtime lifecycle transitions have already been exercised.

`dual_damage` preserves formula→whole→integer-half, including perfect1→0 and miss
no-draw exits; `dual_timing` preserves attack_ms→timeToHit/2→first/second delays
before tick ceiling. `skill_caller_damage` applies the handler's separate critical
doubling after formula/perfect-block return. The e0 gear-kind matrix uses level84
with sufficient source expertise; level1 remains explicit for the unarmed and kit
cases. All inputs are recorded, never inferred from a finite fixture identity.
