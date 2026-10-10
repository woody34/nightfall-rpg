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
| `victim_flag_not_refreshed` | Received victim deadline at the newest impact agrees with a public PvP copy at or after that impact. |
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
