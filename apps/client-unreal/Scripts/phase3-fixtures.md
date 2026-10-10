Phase 3 uses the same isolated fixture lifecycle as [phase2-fixtures.md](phase2-fixtures.md).
Fixtures migrate and seed **before API startup/admission**. They require `--api start`
(`run-sim-ci.py --fresh-stack`), distinct role accounts, fresh DB/JetStream volumes,
nondefault loopback ports, private token files, and owned teardown. The wrappers
never modify admitted characters. `SIM_PHASE3_SEEDER` selects the published seeder;
the default is `infra/scripts/seed-phase3-fixture.py`. Only explicit dev-token opt-in
plus a loopback `nf_phase3_fixture_*` DB on a non-5432 port allows a seed.

**Integration status:** Schema reconciled to migration 000008 in API commit
`0a37fb3f6c69c9b3dc3f9b9b4d1fe8291262cb5f`. Seeds set timer policy1, zero
fractions/counters/penalty, absent death record, empty modifier order and timer/
effect children. Existing 000007 SP/slots/ranks remain the only learned ledger.
The candidate migrator must contain 000008, now merged into this worktree;
an older candidate binary fails the child-table guard. Transaction failure rolls back the entire seed,
and a failed/mismatched seed never permits API startup. Retained typed payloads,
live admission and gameplay are still pending the engine/API handoff. No live
seed, scenario, Unreal build, API/Compose launch or packaged soak has been run
by this worker.

| Exact header | Roles and persisted inputs |
|---|---|
| `# fixture: phase3-trainer` | Human Mystic class10/base10, level14, SP10000, at126,128. Wind Strike1177 rank1, Ice Bolt1184/Might1068/Shield1040/Curse Weakness1164 rank1. **1177 rank2 is unlearned**, cost240/required level7 from professions/human_mystic.toml. Optional matching A/B observer pair. |
| `# fixture: phase3-pvp` | Two distinct Human Mystic accounts level14 at110,100 and112,100, outside inclusive peace square116..136. Owner SP10000,1177 rank5/1184 rank4/1068 rank1. Peer SP0,1177 rank1,HP100,CP10. |
| `# fixture: phase3-resurrect` | Cleric class15/base10 level30 with1016 rank1 and1069/1068/1216 rank1 at110,100; distinct living peer level14 at112,100,HP100,CP10. Peer dies through ordinary sentinel retaliation; no precreated offer or fabricated death loss. |
| `# fixture: phase3-regen` | Human Mystic level14,SP0,HP/MP/CP1 at126,128;1177 rank1. |

All unconfigured HP/MP are NULL (server derives maxima); CP defaults0. XP comes
from shipped experience.toml at the seed level; active class-slot SP equals
characters.sp. Learned ranks use existing `l2.skill.<id>` keys. The owner character
is `01970000-0000-7000-8000-000000000030`, peer `...0031`. Accounts are random per
unit. The manifest includes exact role inputs, account/character identifiers,
binary/migrator/seeder hashes, every input data hash and the final overlay-zone
hash. Generated overlay inputs stay under the private fixture directory.

The overlay copies shipped data and makes keltirs passive/no-clan-help while
preserving XP/SP rewards. One keltir remains uniquely targetable. `skill_target`
is a passive million-HP combat target; `skill_sentinel` is the Phase2 sentinel
renamed. Template construction reuses token-fixtures.py's audited inputs; neither
is a gameplay formula/oracle. Class Master stays126,128. Numeric fixture inputs
are test choices, not claims about pinned gameplay. Source skill durations/costs
remain unchanged. Oracle assertions require independent, hash-pinned E1 vectors;
those vectors have not landed in this worktree and no oracle pass is claimed.

The 17 matrix files retain their stable names. Two additional `-peer.nfs` files
provide real cooperating clients for the solo-named karma/aggro stories.
`Scenarios/phase3-roles.json` binds each primary to its companion; both reports
have unique filenames/artifact folders and independent accounts. Missing roles,
repeated paths, mismatched fixture headers and duplicate artifact names fail.
Existing A/B units remain atomic. Coordination uses received server state only.

Run the first scenario with `Scripts/run-sim.sh --api start --artifacts <new-dir>
Scenarios/3-cast-skill.nfs`. Run cooperating roles with `run-sim-multi.sh` and
both paths. Source-duration waits remain full length: Might1200s →3600s timeout;
regen30ticks×100ms×3→9s/pulse; flag1200ticks×100ms×3→360s. Wrappers/CI extend
process budgets to at least `nf.Within +30s`, retaining all final evidence gates.
No duration or cost is recomputed from client combat stats in Python/Bash.

The following is a **requested client contract**, pending publication of M3 in
client-progress.md. All predicates must return UNKNOWN before valid received
scope evidence, reset on transport/entity/generation changes, and fence tick,
incarnation, cast/effect/offer IDs. Owner bootstrap/revival exceptions follow
binding design-review2. Neither uninitialized zero nor an Ack alone proves an effect.
Unsupported predicates must fail parsing; do not skip assertions.

| Commands / predicates | Authoritative evidence |
|---|---|
| `nf.Cast <id> [self\|UUID]`, `nf.CancelCast` | WS intents → one Ack/rejection, then received Started/Result/Cancelled. Explicit corpse UUID for1016; no corpse SetTarget. |
| `nf.Learn <id> <rank> [UUID-key]`, `nf.LearnableSkills`, `nf.SkillCatalog` | Owned-character RPC, exact-rank frozen response/idempotency fields; catalogue status/blockers. |
| `nf.AcceptResurrect latest accept` | Captured valid owner offer id, then intent result/lifecycle. Client must retain last consumed id for duplicate-consent negative tests. |
| skill_ready, skill_reuse_remaining, learned_rank | SkillReuse/SkillStateSync ready_tick, received current tick, SkillLearned/frozen receipt; skill-id keyed. |
| cast_state, cast_starts/results/cancellations, observed_cast_results | Scoped lifecycle IDs, not intent count; observer baselines are not starts/results. |
| buff_active/inactive, debuff_active, entity_effect_active/inactive, effect_count | Stable effect-id map from Applied/Removed/hydration; inactive requires a received baseline/removal, not absence of an unknown map. |
| own_sp, sp_gained_total/events, skill_learned_events, learnable_contains, learn_rpc_error, learn_receipt_matches_first | Owner stats/sync/SpGained/SkillLearned and exact frozen RPC; retry creates no second event. Reconnect balances separate from connection-scoped event counters. |
| own_mp_cost_matches, skill_damage/effect_matches_oracle, stat_matches_oracle, regen_matches_oracle, moving_regen_matches_oracle | Received resources/effective stats/results/regen with external independent fixture vectors. **MP costs currently missing from wire; stays UNKNOWN**, per client report. No runtime/client formula as expectation. |
| own_flag/karma/pk/pvp, target_flag, player_cp_damage_total, cp_absorption_matches_received | Public PvP/spawn and private owner stats; CP damage from result field, absolute resources from owner stats. Public data never exposes peer CP/MP/SP totals. |
| own_flag_deadline_matches_received, victim_flag_not_refreshed, pvp_private_stats_coherent | Received flag deadline/state history per incarnation, max-deadline refresh, victim copy, owner-only field scope. |
| last_reject, cancelled_cast_costs_retained, cast_dispositions_once, peace_rejection_has_no_start | Correlated intent seq/reason, start/result/cancel tick and received resource/reuse history. Cancel has no late IntentRejected. |
| owner/observer_skill_baseline_coherent, spawn_before_skill_references, observer_private_stats, public_vitals_coherent | Identity-first and reference hydration trace, no duplicate lifecycle count, exact IDs/incarnations/deadlines and owner-only stats routing. |
| buff_replacements, replacement_remove_before_apply, effect_identity_coherent, effect_remove_reason | Applied/Removed stable IDs and causal order/reason; replacement installs new ID. |
| retained_effect_identity_coherent, restore_did_not_reapply_hooks | Received restored skill/pool/source/install-order/remainder fields and fixture oracle; offline pause, no replayed heal/instant hook. Admission may allocate fresh zone effect IDs. |
| entity_in_view/dead/alive <UUID>, players_attacking_me, npc_target_is_owner, skill_hate_outcome_coherent | Received spawn/lifecycle/AttackStarted/target/AI projections. Ledger arithmetic belongs to engine oracles. |
| own_death_penalty_matches_seed, death_record_matches_fixture/before_disconnect, death_xp_matches_fixture, town_respawn_vitals_match_fixture | Private sync/death/stats, deterministic seed/vector and actual recorded loss; not an assumed20% success. |
| resurrect_offer_matches_death, resurrection_vitals_match_fixture, resurrect_refund_matches_oracle, resurrection_refunds, public_revival_vitals_coherent | Private offer/loss/id/expiry, death incarnation, Respawned then final VitalsChanged, owner vitals and independent refund vector; rank1 power0 is a valid zero-refund case. |
| karma_gain/decay_matches_oracle | Committed PvP/stats/XP events, pre-kill PK count/pre-award level and external source vectors. |

Current files cover core flows, **not the entire matrix's variants**. Pending
client/engine integration includes exact 50/51 and100/101 checkpoint boundaries,
seeded resist/status/stun/physical-skill fixtures, stronger/lower/overflow/dispel
branches, nonlethal player/NPC DoT, real regen dead/full/NPC variants, PvP flagged
kill/PK/CP/peace-impact/expiry-impact cross-products, learning cap/competing-kill
and historical-receipt variants, resurrection decline/expiry/replacement/town
race/crash matrix, and AddHate/heal/GetAgro/tie variants. These require published
runtime/data/vector/DSL contracts; source files and covers headers are not pass
attestations. Actor/adapter precision tests remain the server owners' work.

Schema3 coverage consumers use `player_cast_states` and `effect_states` plus the
maps listed in sim-gates.py. They retain every schema2 field, reject missing or
unknown facts, validate Cartesian reachability, deduplicate canonical recordings,
and reject mixed-schema merges. Schema2 remains readable without invented zero
Phase3 observations. **TODO-RECONCILE producer field names** when Z publishes
coverage.rs. Synthetic schema3 test fixtures are not native replay evidence.

Host checks: `TMPDIR=/tmp python3 Scripts/test/phase3-fixture.test.py`,
`python3 Scripts/test/proto-optionals.test.py`, `python3 Scripts/test/transition-v3.test.py`.
These use command stubs and do not launch Unreal/API/Compose.

Casting soak defaults to eight clients and1200s, all roles casting Wind Strike and
long Might buffs. `soak-fixtures.py --casting` emits mode0600 tokens and an exact
`roles.json` manifest; seed-phase3-soak.py validates canonical inputs/unique UUIDs
and seeds all roles in one fresh transaction before API startup. The casting
NPC overlay has HP1 as an explicit test input; its source rewards are unchanged.
Gauntlet/report filenames stay `1-kill-one-monster` for existing artifact consumers.
Each role must supply positive cast_skill, skill_cast_result and effect_applied
runtime counts, plus verified native schema3 cast/effect transitions. Missing
casting evidence fails. `--legacy-attacks` retains the old attack workload and
cannot count as Phase3 soak acceptance.

**Client-owned NightfallSoak.cs handoff:** consume `-SoakRoleManifest=<roles.json>`,
validate schema_version1/casting/count/order, use each role's scenario and
`-DevTokenFile=<token_file>` (never inline tokens), retain unique UserDir/BotOutDir,
and allow the final120s casting scenario plus boot grace. Do not change client
skill formulas. Harness run-soak.sh refuses casting before infrastructure if the
client source lacks this hook. C# and its build are outside C-HARNESS ownership;
no Gauntlet/packaged soak pass is claimed.
