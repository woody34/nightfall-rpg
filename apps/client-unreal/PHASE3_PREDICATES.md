# Phase 3 received-state predicate evidence

All observations are scoped to the current transport, entity incarnation and identity baseline. Ordinary facts at or before the baseline are ignored; explicitly marked hydration is accepted at the identity tick. Reconnect comparisons preserve only an explicitly captured previous-owner sample. Unknown evidence fails every comparison, including inequality. Counts use whole integers, without floating-point conversion.

| Predicate | Authoritative input and evidence required |
|---|---|
| `cast_dispositions_once` | SkillCastStarted caster/incarnation/cast_id, followed by one SkillCastResult or SkillCastCancelled. Exact duplicate terminal envelopes are no-ops; contradictory terminals or rejection of an Acked sequence fail. Requires a nonbaseline start and all witnessed casts to have terminated. |
| `spawn_before_skill_references` | EntitySpawn/EntityRespawned identity before every SkillCastStarted target, SkillCastResult target, EffectApplied caster, AttackStarted target and ResurrectOffer caster. Optional hidden ID/incarnation pairs must both be absent. Requires a witnessed reference-bearing message. |
| `owner_skill_baseline_coherent` | Same-identity baseline StatsChanged and SkillStateSync; public/private PvP copies and reference checks agree. |
| `observer_skill_baseline_coherent` | Public baseline EffectApplied/SkillCastStarted/PvpStateChanged after admitted identities, with valid nested references. |
| `public_vitals_coherent` | Accepted VitalsChanged HP/max HP agrees with the combat projection after dispatch. Final values supersede intermediate results; no repeated damage subtraction. Requires a public vitals event. |
| `public_revival_vitals_coherent` | EntityRespawned then baseline VitalsChanged at its tick/incarnation; final max HP supplied and projected. |
| `pvp_private_stats_coherent` | Owner StatsChanged/SkillStateSync PvP fields agree with public EntitySpawn/PvpStateChanged at equal tick/incarnation. Nonowner private messages fail the audit. |
| `own_flag_deadline_matches_received` | Equal-tick owner StatsChanged.flag_until_tick and public PvP deadline. |
| `own_cp_damage_total op n` | Deduplicated received player-sourced AttackResult.cp_damage and SkillCastResult target.cp_absorbed targeting the owner. |
| `player_cp_damage_total op n` | Same received fields from owner-sourced impacts targeting visible players. |
| `cp_absorption_matches_received` | Positive witnessed own CP absorption between two owner StatsChanged samples agrees with received CP/HP deltas and final HP. |
| `players_attacking_me op n` | Distinct living visible player AttackStarted attackers targeting the owner, cleared by AttackCancelled/death/despawn/transport reset. |
| `npc_target_is_owner` | A living visible NPC's witnessed AttackStarted target is the owner. No claim about an unobserved hate ledger. |
| `own_flag_expired_at_last_impact`, `target_flag_expired_at_last_impact` | Witnessed start-before and impact-at-or-after a received target flag deadline. Own variant requires owner victim; target variant requires owner attacker. Missing prior flag/start is unknown. |
| `victim_flag_not_refreshed` | After the stream advances beyond the newest impact, the held victim deadline agrees with the received pre-impact copy; no refresh was witnessed in that stream window. |
| `peace_rejection_has_no_start` | Sent cast sequence receives PEACE_ZONE IntentRejected, has no Ack and no intervening own SkillCastStarted in the serialized command window. |
| `effect_identity_coherent` | Accepted EffectApplied stable identity/rank/pool/applied_tick and known visible source, with EffectRemoved tombstones. |
| `death_penalty_matches_before_disconnect` | Captured owner StatsChanged/SkillStateSync penalty before disconnect equals that same owner's received penalty after reconnect. |
| `resurrect_offer_matches_death` | Owner EntityDied and ResurrectOffer share target/incarnation; the received expiry is after the offer tick. |
| `resurrect_offer_state == offered\|declined\|expired\|accepted` | Last witnessed owner ResurrectOffer.state; terminal history cannot be undone by an offered duplicate. No state inferred from wall clock. |
| `resurrection_refunds op n` | Distinct accepted offer IDs followed by owner revival and a received XP increase from the dead StatsChanged sample. This is observed positive refund count, not an assertion of hidden checkpoint persistence. |

The core Phase 3 commands and predicates implemented before round 2 are documented in the client progress handoff. Additional exact-cost and oracle predicates are recorded here as they land. Live scenario execution and independent server precision evidence remain separate verification gates.

| Cost predicate | Authoritative input and evidence required |
|---|---|
| `own_mp_cost_matches <skill_id>` | Newest completed owner SkillCastStarted/Result at a received exact rank, GetSkillCatalog arrays `mp_consume_start/end[rank-1]`, and owner StatsChanged MP samples immediately before/at each stage. Separately received MP gains between stages do not count as consumption. Explicit catalogue MP1=0 may omit an unchanged start resource sample. Missing/short arrays, rank, terminal or required samples are UNKNOWN. This compares received net stage deltas; coalesced unrelated changes and actor consumption modifiers require independent precision evidence. |
| `cancelled_cast_costs_retained` | Owner SkillCastStarted/Cancelled exact rank plus catalogue MP1, received start StatsChanged delta and SkillReuse at start whose deadline remains beyond cancellation. MP returns to a higher value cannot satisfy the received retention comparison. Requires the catalogue to have been loaded; no missing-cost zero assumption. |

C6 catalogue tags8–12 and generated arrays are integrated from API95d2738; the metadata is validated base data before actor modifiers. None of these checks executes a gameplay formula or mutates resources.

| Observer predicate | Authoritative input and evidence required |
|---|---|
| `cast_result_for <entity> <id>` | Newest witnessed Started for that current visible caster/incarnation/skill/cast ID must receive a Result matching caster/incarnation/skill/rank/cast ID. An unrelated prior result cannot satisfy it. |
| `cast_terminal_once_for <entity> <id>` | The same identity must have one matching Result; identical duplicates are no-ops and contradictory terminals fail. |
| `observed_cast_baseline_for <entity> <id>` | Explicit baseline Started after identity admission, with received original `start_tick < baseline_tick < complete_tick` and matched source/target incarnations. Ordinary or completed casts cannot satisfy it. |
| `entity_not_in_view <entity>` | A witnessed current-transport identity was subsequently despawned; initial absence is UNKNOWN. Re-entry and transport reset invalidate the departure sample. |

`Nightfall.Skill.ObserverCastIdentity` consumes the harness-authored `Scenarios/fixtures/phase3-observer-mutual.json` and executes its six negative cases. This is native client dispatch evidence, not an engine-produced recording or live gameplay attestation.

| Remaining received-history predicate | Authoritative input and expected-value source |
|---|---|
| `impact_counter_uses_received_flag` | Player impact, matching target EntityDied/killer/incarnation/tick, public pre-impact target flag/karma and same-tick attacker PvpStateChanged. Compares received PK/PvP counter changes for neutral unflagged, flagged and chaotic unflagged victims. Binding D-R4; pinned L2PcInstance.java:4592–4612,4624,4647. |
| `flagged_kill_matches_received` | The same correlation, requiring the victim's received flag deadline to extend past impact. PvP increases once while PK/karma remain unchanged. |
| `karma_gain_matches_oracle` | Neutral unflagged kill correlation plus before/after owner PvpStateChanged. Pre-increment PK count selects the exact frozen E1 `calculateKarmaGain` input, never an output or nearest vector (L2PcInstance.java:4640–4647). |
| `karma_decay_matches_oracle` | Owner PvpStateChanged decrease and XpGained amount associated with the latest received tick, with the preceding received StatsChanged level. Exact E1 `calculateKarmaLost` level/credit vector only. XpGained has no wire tick: stream correlation does not prove the actor's internal XP credit ordering. |
| `death_xp_matches_fixture` | Matching EntityDied and before/dead owner StatsChanged XP/incarnation/level. Uses the existing Phase 1 test-only `FBotFixtureData::DeathXpLoss` table expectation, including received XP saturation; no presentation or gameplay resource calculation changes. |
| `town_respawn_vitals_match_fixture` | Sent Respawn sequence/Ack, dead owner StatsChanged and first newer-incarnation revival StatsChanged. Uses the existing Phase 1 test-only town table expectation; CP remains the received dead CP. Later regen cannot overwrite that first sample. |
| `resurrection_vitals_match_fixture` | Accepted ResurrectOffer bound to dead incarnation and first revival StatsChanged. All six dead HP/MP/CP current/max inputs must match one frozen E1 `doRevive` vector. Missing exact vector is UNKNOWN. |
| `retained_effect_identity_coherent` | Active owner EffectApplied captured before disconnect, new owner SkillStateSync baseline and baseline EffectApplied with the same stable ID, skill/rank/pool/applied tick/abnormal/status and visible caster identity. Does not assert modifier order, checkpoint remainder or periodic phase. |
| `peace_impact_has_no_damage` | Matched peace-zone SkillCastCancelled plus witnessed target HP/CP at start and unchanged received owner CP/public HP. Requires victim-owner private CP; a caster observing another player cannot prove peer CP and gets UNKNOWN. |

The E1 adapter verifies the entire oracle SHA-256, pinned L2J revisions, schema and all six declared input-file hashes before looking up any expected value. It does not interpolate, calculate damage/regen/karma, choose by observed result or install modifiers. Missing vectors and changed inputs fail closed. The two Phase 1 table checks reuse the established test-only helper; they are received-state assertions, not client gameplay execution.

| Registered but evidence-blocked predicate | Missing authoritative evidence (always UNKNOWN) |
|---|---|
| `regen_pulses op n`, `moving_regen_pulses op n` | A regen pulse/cause marker; a resource delta can also be damage, healing, costs or replacement. |
| `regen_matches_oracle`, `moving_regen_matches_oracle` | Pulse cause, exact regen inputs and movement branch. |
| `death_record_matches_fixture`, `death_record_matches_before_disconnect` | Complete persisted DeathRecord and stored death loss. |
| `restore_did_not_reapply_hooks` | Actor modifier installation order and periodic phase. |
| `skill_hate_outcome_coherent` | Full hate ledger and credit inputs. `npc_target_is_owner` proves only observed target choice. |
| `skill_damage_matches_oracle <id>`, `skill_effect_matches_oracle <id>` | Exact base/modifier input bundle and ordered RNG draw trace. |
| `stat_matches_oracle` | Ordered modifier/base-stat inputs. EffectiveStats alone cannot identify their source. |
| `resurrect_refund_matches_oracle` | Stored death loss and restore-percent inputs. Offer RecoverableXp is not an independent oracle for itself. |

These twelve names have strict argument binders so all canonical/nested Phase 3 sources parse through the native registry. Their false/UNKNOWN evaluations deliberately prevent scenario acceptance. The harness's stricter source-compatibility audit must still reject their uses. No native scenario gameplay, engine precision reports, or real API pass is implied by parsing.

| Core Phase 3 predicate | Authoritative message mapping |
|---|---|
| `skill_ready <id>`, `skill_reuse_remaining <id> op n`, `learned_rank <id> op n` | SkillStateSync learned ranks/reuse deadlines, SkillLearned and SkillReuse; remaining ticks use the received server tick. |
| `buff_active <id>`, `buff_inactive <id>`, `debuff_active <id>`, `effect_count op n` | Accepted owner EffectApplied/EffectRemoved; received pool and expiry tick, with known owner hydration. |
| `entity_effect_active <entity> <id>`, `entity_effect_inactive <entity> <id>` | The same accepted public effects on a currently visible admitted identity. |
| `cast_state == idle\|casting`, `entity_casting <entity> <id>` | Accepted SkillCastStarted (including hydration), SkillCastCancelled and SkillCastResult; death/despawn clear the scoped state. |
| `own_flag == none\|pvp\|chaotic`, `target_flag == none\|pvp\|chaotic`, `target_flag_remaining op n` | EntitySpawn/PvpStateChanged karma and flag deadline; owner StatsChanged/SkillStateSync private copies. |
| `own_karma op n`, `own_pk op n`, `own_pvp op n` | Received owner PvP fields, without locally awarding counters. |
| `own_sp op n`, `sp_gained_total op n`, `sp_gained_events op n` | Owner SkillStateSync/StatsChanged SP and actual SpGained.delta; no inferred XP-to-SP conversion. |
| `learnable_contains <id> <rank>` | Successful ListLearnableSkills exact ID/rank row. |
| `learn_rpc_error == <code>`, `skill_rpc_done`, `learn_receipt_matches_first` | Completed LearnSkill canonical gRPC status and frozen replies for the same sent idempotency key. |
| `catalogue_skill_status <id> == executable\|deferred` | GetSkillCatalog.effect_implemented/deferred_reasons. |
| `resurrect_offer_pending`, `own_death_penalty op n` | Received owner ResurrectOffer state/deadline and StatsChanged/SkillStateSync.death_penalty_level. |
| `vitals_max_hp <entity> op n` | EntitySpawn and absolute VitalsChanged.max_hp, scoped to visible identity/incarnation. |
| `initial_hp/mp/cp op n` | Owner EntitySpawn HP and first accepted owner StatsChanged MP/CP after admission. |
| `cast_starts/results/cancellations op n`, `observed_cast_results op n`, `cast_damage_results op n`, `skill_learned_events op n` | Accepted typed cast/SkillLearned history; cast damage counts actual damage-result rows. |
| `cast_cancel_reason == <reason>`, `observed_cast_cancel_reason == <reason>` | Received SkillCastCancelled.reason, owner or visible observer history. |
| `buff_replacements op n`, `replacement_remove_before_apply`, `effect_remove_reason == <reason>` | EffectRemoved reason and ordered received replacement apply with a new stable ID. |
| `entity_alive/dead/in_view <entity>`, `entity_at <entity> <x> <y> <tol>` | EntitySpawn, EntityDied/Respawned, EntityDespawn and received position samples. |

Commands send the actual registered transport operations: `nf.Cast`, `nf.CancelCast`, `nf.Learn <id> <rank> [idempotency_uuid]`, `nf.LearnableSkills`, `nf.SkillCatalog`, and `nf.AcceptResurrect [offer_id|latest] [accept|decline]`. RPC results and world events determine the observations; commands do not synthesize success.

Round 3 evidence fences: `buff_inactive`, `entity_effect_inactive`, `effect_count` and `cast_state` require a received stream tick later than the subject identity baseline. V5 has no hydration-end marker, so identity or SkillStateSync alone never proves empty effects/idle casting. `cancelled_cast_costs_retained` and `peace_impact_has_no_damage` wait for a tick later than the terminal tick and compare current, incarnation-matched resources/reuse; a later refund cannot satisfy them. `spawn_before_skill_references` audits subject and endpoint identities before projecting hydration, ignores known stale lives/baselines, and retains a violation for never-admitted subjects even after subsequent valid frames.

Round 4 C7 catalogue integration: `catalogue_skill_status` still reports the received
`effect_implemented` capability (an executable passive is not a cast action). Hotbar
slots and the trainer assignment picker additionally require a received learned rank
and `GetSkillCatalog.operate_type` ACTIVE_INSTANT, ACTIVE_INSTANT_CONTINUOUS or
ACTIVE_CONTINUOUS. Passive/toggle/other/unspecified/unknown types cannot authorize
slot casts. Trainer learning eligibility remains the received ListLearnableSkills
row, including passives. Target/scope arrays are retained exactly as received;
the client does not infer capabilities or formulas from their strings. No predicate
or command grammar changed.
