# Phase 3 received-state predicate evidence

All observations apply only to the current transport, the current entity incarnation and the current identity baseline. The client ignores ordinary facts at or before the baseline. The client accepts hydration that has an explicit mark at the identity tick. Reconnect comparisons keep only a previous-owner sample that the client captured explicitly. Unknown evidence fails all comparisons, and inequality also. Counts use whole integers, with no floating-point conversion.

| Predicate | Authoritative input and necessary evidence |
|---|---|
| `cast_dispositions_once` | SkillCastStarted caster/incarnation/cast_id, then one SkillCastResult or SkillCastCancelled. Exact duplicate terminal envelopes have no effect. Contradictory terminals, or a rejection of an Acked sequence, fail. A nonbaseline start is necessary, and all witnessed casts must be terminated. |
| `spawn_before_skill_references` | EntitySpawn/EntityRespawned identities come before: SkillCastStarted/Result/Cancelled subjects and result rows, EffectApplied/Removed subjects and sources, AttackStarted/Result/Cancelled subjects and endpoints, and ResurrectOffer subjects and sources. The audit occurs before the projection, cast-history and offer eligibility filters. The client ignores known stale lives/baselines. If there are optional hidden ID/incarnation pairs, the two values must both be absent. Historical attacks without a subject incarnation use the admitted identity. A witnessed audited message is necessary. |
| `owner_skill_baseline_coherent` | Same-identity baseline StatsChanged and SkillStateSync; the public/private PvP copies and the reference checks agree. |
| `observer_skill_baseline_coherent` | Public baseline EffectApplied/SkillCastStarted/PvpStateChanged after the admitted identities, with valid nested references. |
| `public_vitals_coherent` | The accepted VitalsChanged HP/max HP agrees with the combat projection after dispatch. Final values replace intermediate results; the client does not subtract damage two times. A public vitals event is necessary. |
| `public_revival_vitals_coherent` | EntityRespawned, then baseline VitalsChanged at its tick/incarnation; the final max HP is supplied and projected. |
| `pvp_private_stats_coherent` | Owner StatsChanged/SkillStateSync PvP fields agree with public EntitySpawn/PvpStateChanged at the same tick/incarnation. Private messages to a nonowner fail the audit. |
| `own_flag_deadline_matches_received` | Owner StatsChanged.flag_until_tick and the public PvP deadline at the same tick. |
| `own_cp_damage_total op n` | Deduplicated received player-sourced AttackResult.cp_damage and SkillCastResult target.cp_absorbed that target the owner. |
| `player_cp_damage_total op n` | The same received fields from owner-sourced impacts that target visible players. |
| `cp_absorption_matches_received` | Positive witnessed own CP absorption between two owner StatsChanged samples agrees with the received CP/HP deltas and the final HP. |
| `players_attacking_me op n` | C18: "number of players with a currently received, unexpired swing/recovery against me". Counts the different living visible players whose accepted AttackStarted targets the current owner incarnation, and whose ReadyTick is later than the latest received server tick. AttackCancelled, death, despawn and transport reset clear the evidence. The wire has no 'auto-attack enabled' fact. Thus, the client cannot see server-side pursuit without a new swing, and by definition the result is 0 after the recovery expires. |
| `npc_target_is_owner` | The witnessed AttackStarted target of a living visible NPC is the owner. This makes no claim about a hate ledger that the client did not see. |
| `own_flag_expired_at_last_impact`, `target_flag_expired_at_last_impact` | A witnessed start before, and an impact at or after, a received target flag deadline. The own variant must have the owner as victim. The target variant must have the owner as attacker. If the prior flag/start is missing, the result is unknown. |
| `victim_flag_not_refreshed` | After the stream goes beyond the newest impact, the held victim deadline agrees with the received pre-impact copy. The client saw no refresh in that stream window. |
| `peace_rejection_has_no_start` | The sent cast sequence receives PEACE_ZONE IntentRejected. It has no Ack and no own SkillCastStarted between them in the serialized command window. |
| `effect_identity_coherent` | Accepted EffectApplied stable identity/rank/pool/applied_tick and a known visible source, with EffectRemoved tombstones. |
| `death_penalty_matches_before_disconnect` | The captured owner StatsChanged/SkillStateSync penalty before disconnect is equal to the received penalty of that same owner after reconnect. |
| `resurrect_offer_matches_death` | Owner EntityDied and ResurrectOffer have the same target/incarnation; the received expiry is after the offer tick. |
| `resurrect_offer_state == offered\|declined\|expired\|accepted` | The last witnessed owner ResurrectOffer.state. An offered duplicate cannot undo terminal history. The client does not infer a state from wall clock. |
| `resurrection_refunds op n` | Different accepted offer IDs, each followed by owner revival and a received XP increase from the dead StatsChanged sample. This is the count of observed positive refunds. It is not an assertion of hidden checkpoint persistence. |

The client progress handoff records the core Phase 3 commands and predicates that the team implemented before round 2. This file records more exact-cost and oracle predicates when they merge. Live scenario execution and independent server precision evidence are separate verification gates.

| Cost predicate | Authoritative input and necessary evidence |
|---|---|
| `own_mp_cost_matches <skill_id>` | The newest completed owner SkillCastStarted/Result at a received exact rank, the GetSkillCatalog arrays `mp_consume_start/end[rank-1]`, and owner StatsChanged MP samples immediately before/at each stage. MP gains that the client received separately between stages are not consumption. If the catalogue has an explicit MP1=0, an unchanged start resource sample can be absent. Missing/short arrays, or a missing rank, terminal or necessary sample, give UNKNOWN. This compares received net stage deltas. Coalesced unrelated changes and actor consumption modifiers must have independent precision evidence. |
| `cancelled_cast_costs_retained` | Owner SkillCastStarted/Cancelled exact rank plus catalogue MP1, the received start StatsChanged delta, and SkillReuse at start whose deadline stays after cancellation. If MP goes back to a higher value, the received retention comparison is not satisfied. The catalogue must be loaded; the client does not assume zero for a missing cost. |

C6 catalogue tags8–12 and the generated arrays come from API95d2738. The metadata is validated base data before actor modifiers. None of these checks runs a gameplay formula or changes resources.

| Observer predicate | Authoritative input and necessary evidence |
|---|---|
| `cast_result_for <entity> <id>` | The newest witnessed Started for that current visible caster/incarnation/skill/cast ID must receive a Result with the same caster/incarnation/skill/rank/cast ID. An unrelated prior result cannot satisfy it. |
| `cast_terminal_once_for <entity> <id>` | The same identity must have one matching Result. Identical duplicates have no effect, and contradictory terminals fail. |
| `observed_cast_baseline_for <entity> <id>` | An explicit baseline Started after identity admission, with the received original `start_tick < baseline_tick < complete_tick` and matched source/target incarnations. Ordinary or completed casts cannot satisfy it. |
| `entity_not_in_view <entity>` | A witnessed current-transport identity had a despawn after that. If the identity was absent at the start, the result is UNKNOWN. Re-entry and transport reset make the departure sample not valid. |

`Nightfall.Skill.ObserverCastIdentity` uses the harness-authored `Scenarios/fixtures/phase3-observer-mutual.json` and runs its six negative cases. This is evidence of native client dispatch. It is not a recording that the engine made, and it is not an attestation of live gameplay.

| Remaining received-history predicate | Authoritative input and expected-value source |
|---|---|
| `impact_counter_uses_received_flag` | Player impact, the matching target EntityDied/killer/incarnation/tick, the public pre-impact target flag/karma and the attacker PvpStateChanged at the same tick. Compares the received PK/PvP counter changes for neutral unflagged, flagged and chaotic unflagged victims. Binding D-R4; pinned L2PcInstance.java:4592–4612,4624,4647. |
| `flagged_kill_matches_received` | The same correlation. The received flag deadline of the victim must go past the impact. PvP increases one time, and PK/karma do not change. |
| `karma_gain_matches_oracle` | Neutral unflagged kill correlation, plus owner PvpStateChanged before and after. The PK count before the increment selects the exact frozen E1 `calculateKarmaGain` input. It never selects from an output or from the nearest vector (L2PcInstance.java:4640–4647). |
| `karma_decay_matches_oracle` | Owner PvpStateChanged decrease and the XpGained amount related to the latest received tick, with the received StatsChanged level before them. Only the exact E1 `calculateKarmaLost` level/credit vector. XpGained has no wire tick: the stream correlation does not prove the internal XP credit order of the actor. |
| `death_xp_matches_fixture` | Matching EntityDied and before/dead owner StatsChanged XP/incarnation/level. Uses the existing Phase 1 test-only `FBotFixtureData::DeathXpLoss` table expectation, including received XP saturation. No changes to presentation or to gameplay resource calculation. |
| `town_respawn_vitals_match_fixture` | Sent Respawn sequence/Ack, dead owner StatsChanged and the first revival StatsChanged of a newer incarnation. Uses the existing Phase 1 test-only town table expectation. CP stays at the received dead CP. Regen after this cannot overwrite that first sample. |
| `resurrection_vitals_match_fixture` | Accepted ResurrectOffer bound to the dead incarnation, and the first revival StatsChanged. All six dead HP/MP/CP current/max inputs must agree with one frozen E1 `doRevive` vector. If the exact vector is missing, the result is UNKNOWN. |
| `retained_effect_identity_coherent` | Active owner EffectApplied captured before disconnect, a new owner SkillStateSync baseline, and a baseline EffectApplied with the same stable ID, skill/rank/pool/applied tick/abnormal/status and visible caster identity. It does not assert the modifier order, the checkpoint remainder or the periodic phase. |
| `peace_impact_has_no_damage` | Matched peace-zone SkillCastCancelled, plus complete same-life owner HP/CP samples through the first sample after cancellation. In that interval there must be no resource decreases and no positive received AttackResult/SkillCastResult HP/CP removal. Contradictory cast terminals return false. Victim-owner private CP is necessary. A caster that observes a different player cannot prove the CP of that player, and gets UNKNOWN. |

The E1 adapter examines four items before it looks up an expected value. The items are the full oracle SHA-256, the pinned L2J revisions, the schema and all six declared input-file hashes. It does not interpolate. It does not calculate damage/regen/karma. It does not select by observed result, and it does not install modifiers. Missing vectors and changed inputs fail closed. The two Phase 1 table checks use the established test-only helper again. They are received-state assertions, not client gameplay execution.

| Registered but evidence-blocked predicate | Missing authoritative evidence (always UNKNOWN) |
|---|---|
| `regen_pulses op n`, `moving_regen_pulses op n` | A regen pulse/cause marker. A resource delta can also be damage, healing, costs or replacement. |
| `regen_matches_oracle`, `moving_regen_matches_oracle` | Pulse cause, exact regen inputs and movement branch. |
| `death_record_matches_fixture`, `death_record_matches_before_disconnect` | Complete persisted DeathRecord and stored death loss. |
| `restore_did_not_reapply_hooks` | The installation order of actor modifiers, and the periodic phase. |
| `skill_hate_outcome_coherent` | Full hate ledger and credit inputs. `npc_target_is_owner` proves only the observed target selection. |
| `skill_damage_matches_oracle <id>`, `skill_effect_matches_oracle <id>` | Exact base/modifier input bundle and the ordered RNG draw trace. |
| `stat_matches_oracle` | Ordered modifier/base-stat inputs. EffectiveStats alone cannot identify their source. |
| `resurrect_refund_matches_oracle` | Stored death loss and restore-percent inputs. Offer RecoverableXp is not an independent oracle for itself. |

These twelve names have strict argument binders. Thus, all canonical/nested Phase 3 sources parse through the native registry. Their false/UNKNOWN evaluations intentionally prevent scenario acceptance. The stricter source-compatibility audit of the harness must also reject their uses. A parse does not show native scenario gameplay, engine precision reports or a real API pass.

| Core Phase 3 predicate | Authoritative message mapping |
|---|---|
| `skill_ready <id>`, `skill_reuse_remaining <id> op n`, `learned_rank <id> op n` | SkillStateSync learned ranks/reuse deadlines, SkillLearned and SkillReuse. The remaining ticks use the received server tick. |
| `buff_active <id>`, `buff_inactive <id>`, `debuff_active <id>`, `effect_count op n` | Accepted owner EffectApplied/EffectRemoved; the received pool and expiry tick, with known owner hydration. |
| `entity_effect_active <entity> <id>`, `entity_effect_inactive <entity> <id>` | The same accepted public effects on an admitted identity that is visible now. |
| `cast_state == idle\|casting`, `entity_casting <entity> <id>` | Accepted SkillCastStarted (including hydration), SkillCastCancelled and SkillCastResult. Death and despawn clear the scoped state. |
| `own_flag == none\|pvp\|chaotic`, `target_flag == none\|pvp\|chaotic`, `target_flag_remaining op n` | EntitySpawn/PvpStateChanged karma and flag deadline; owner StatsChanged/SkillStateSync private copies. |
| `own_karma op n`, `own_pk op n`, `own_pvp op n` | Received owner PvP fields. The client does not give counters locally. |
| `own_sp op n`, `sp_gained_total op n`, `sp_gained_events op n` | Owner SkillStateSync/StatsChanged SP and the actual SpGained.delta. The client does not infer an XP-to-SP conversion. |
| `learnable_contains <id> <rank>` | The exact ID/rank row of a successful ListLearnableSkills. |
| `learn_rpc_error == <code>`, `skill_rpc_done`, `learn_receipt_matches_first` | The canonical gRPC status of a completed LearnSkill, and the frozen replies for the same sent idempotency key. |
| `catalogue_skill_status <id> == executable\|deferred` | GetSkillCatalog.effect_implemented/deferred_reasons. |
| `resurrect_offer_pending`, `own_death_penalty op n` | Received owner ResurrectOffer state/deadline and StatsChanged/SkillStateSync.death_penalty_level. |
| `vitals_max_hp <entity> op n` | EntitySpawn and absolute VitalsChanged.max_hp, only for the visible identity/incarnation. |
| `initial_hp/mp/cp op n` | Owner EntitySpawn HP, and the first accepted owner StatsChanged MP/CP after admission. |
| `cast_starts/results/cancellations op n`, `observed_cast_results op n`, `cast_damage_results op n`, `skill_learned_events op n` | Accepted typed cast/SkillLearned history. The cast damage count is the count of actual damage-result rows. |
| `cast_cancel_reason == <reason>`, `observed_cast_cancel_reason == <reason>` | Received SkillCastCancelled.reason, from the owner history or the visible observer history. |
| `buff_replacements op n`, `replacement_remove_before_apply`, `effect_remove_reason == <reason>` | EffectRemoved reason, and the ordered received replacement apply with a new stable ID. |
| `entity_alive/dead/in_view <entity>`, `entity_at <entity> <x> <y> <tol>` | EntitySpawn, EntityDied/Respawned, EntityDespawn and received position samples. |

The commands send the actual registered transport operations: `nf.Cast`, `nf.CancelCast`, `nf.Learn <id> <rank> [idempotency_uuid]`, `nf.LearnableSkills`, `nf.SkillCatalog`, and `nf.AcceptResurrect [offer_id|latest] [accept|decline]`. RPC results and world events set the observations. The commands do not make a false success.

Round 3 evidence fences:

- `buff_inactive`, `entity_effect_inactive`, `effect_count` and `cast_state` must have a received stream tick that is later than the subject identity baseline.
- V5 has no hydration-end marker. Thus, an identity or a SkillStateSync alone never proves empty effects or idle casting.
- `cancelled_cast_costs_retained` and `peace_impact_has_no_damage` wait for a tick that is later than the terminal tick. Then they compare the current, incarnation-matched resources/reuse. A refund after that cannot satisfy them.
- `spawn_before_skill_references` audits the subject and endpoint identities before it projects hydration. It ignores known stale lives/baselines. It keeps a violation for subjects that it never admitted, also after valid frames that come after that.

Round 4 C7 catalogue integration: `catalogue_skill_status` continues to report the received
`effect_implemented` capability (an executable passive is not a cast action). Hotbar
slots and the trainer assignment picker also must have a received learned rank
and a `GetSkillCatalog.operate_type` of ACTIVE_INSTANT, ACTIVE_INSTANT_CONTINUOUS or
ACTIVE_CONTINUOUS. Passive/toggle/other/unspecified/unknown types cannot authorize
slot casts. Trainer learning eligibility continues to be the received ListLearnableSkills
row, including passives. The client keeps the target/scope arrays exactly as it received them.
The client does not infer capabilities or formulas from their strings. The predicate
grammar and the command grammar did not change.

`peace_impact_has_no_damage` has these necessary conditions:

- One received peace cancellation that targets the owner.
- Complete same-life HP/CP samples with no decrease, from the cast start through the first sample after cancellation.
- No contradictory cast terminal.

If there is a positive received `AttackResult.damage`/`cp_damage` or `SkillCastResult` target `hp_removed`/`cp_absorbed` in that inclusive interval, the result is false. This is also true when regeneration hides the removal in the absolute samples. Damage evidence uses the target incarnation and tick, also without a received attacker/cast start. Other targets, stale lives, zero removals and later damage do not make the witnessed interval not valid. Contradictory terminals return an explicit false. An ACK without a resource sample after the cancellation stays UNKNOWN. The client keeps positive received regeneration. The paired `peace_cancel_no_damage` replay gate compares the exact HP/CP changes with independently calculated recorded regeneration pulses. If the start, cancellation, or resource samples are missing, the gate fails closed.

`target_swing_impact_remaining op n` reads the accepted AttackStarted impact deadline of the selected entity, minus the latest received tick, with zero as the minimum. If the swing is missing, the endpoint is unknown or the incarnation does not agree, the result is UNKNOWN. It never calculates the attack speed again, and it does not use a local timer.
