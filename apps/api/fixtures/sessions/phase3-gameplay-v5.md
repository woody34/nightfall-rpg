# V5 gameplay regression recording

`phase3-gameplay-v5.nfr` uses Snapshot9, Record7 and the explicit, hashed runtime revision 2.
Generate it with `bash apps/api/fixtures/sessions/record-phase3-gameplay.sh`. The generator
first verifies each complete tick record against a cold replay; ordinary tests never write
fixtures. `replay-check.sh` checks it alongside all three unchanged earlier recordings.

Its initial snapshot contains a level-20 cleric with a valid class receipt, Resurrection 2
and Wind Strike 5, a player corpse with the retained NPC death loss, and a keltir. Initial
skills/resources/death state are fixture setup, not a claim of a production learning or
persistence transaction. Every subsequent change comes from recorded tick inputs.

The session creates and declines an offer, waits the real reuse deadline, creates and
accepts a second offer, performs player auto-attacks through CP, finishes the PvP kill
with Wind Strike, respawns the victim in town, and kills the NPC for XP/SP. The recording
contains seven completed casts, five auto CP resolutions, four skill CP resolutions,
37 total CP removed, one PK credit/karma increase, and two player revivals. The keltir's
small credited XP is below this killer level's whole-karma decay threshold.

Schema-three coverage counts canonical offer and PvP transitions independently of
observer hydration. Tests also verify digest-only replay and reject altered cast facts.
NPC auto AI, offer expiry, peace entry, the other PvP kill classifications, and unchanged-AOI
revival/hidden-swing hydration are separate live domain tests. This fixture does not claim
packaged-client soak, production bootstrap or durable account transactions.

The session-13 review correction regenerates this new V5 fixture under the architect's
explicit allowance in `inbox/zone-inbox.md` (session 13). Its previous bytes retained
player-owned hate rows after nonlethal Wind Strike, and used the wrong cast/auto/AI
phase order. The corrected recording limits effect-point hate to playable casters
against attackable NPCs, stages attack/offer intents, and refreshes hostility at eligible
starts/impacts independently of damage. The two historical recordings and the earlier
runtime-revision-one cast recording remain unchanged and must still replay byte-exact.
