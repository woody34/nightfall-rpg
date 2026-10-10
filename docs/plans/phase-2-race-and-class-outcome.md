# Phase 2 race and class: verification outcome

Status: integration and acceptance in progress, based on candidate `9f3927d`, with coordinated follow-ups through `c74f4b2` on 2026-10-09.
The data/server implementation has passed its scoped reviews and functional checks. Client and
harness follow-ups, live acceptance, and the required token-supply decision remain open.
See the [executable plan](phase-2-race-and-class.md) and
[reference design](../planning/02-race-and-class.md).

## Implemented contracts

Five races and 89 classes retain the 9/18/31/31 base/first/second/third distribution and a
170-point base-stat budget. The authoritative level cap remains **85**; playable transfers are
at levels **20/40**, with a maximum tier of **2**. Third classes are metadata only, even at 85.
Production creation starts at **(0,0)**. Players reach the noncombat Class Master at
**(126,128)**, radius **3**, through ordinary movement.

Pinned L2J datapack `3ca488dd2bd0bfaca43e378886a3c2e37968153a` supplies 7,565 exact growth
rows (22,695 HP/MP/CP values), 39 direct learning trees with 6,927 entries, 3,521 proficiency
rows, and 378 known skill references. Fifty other direct trees are deferred; ancestor metadata
remains available. Inherited auto-get metadata persists at the maximum learned rank without SP
charges and survives deleveling. Skill execution/manual learning and equipment effects remain
future work. See [data provenance](../../packages/data/SOURCES.md).

Transfers consume an existing typed token, preserve resource percentages with one floor, and
retain at least one HP for a living character. CP starts at zero, remains owner-private, and
does not absorb damage. The durability order is **log acknowledgement → atomic class/token/
receipt/outbox checkpoint → RPC success and world output**. At most two successful receipts
per character retain the original full response, including offline retries.

## Verified evidence

| Gate | Result and scope |
|---|---|
| Data and eligibility | 13 Rust catalogue tests and 3 Python provenance tests passed. Independent source audits matched every growth, learning and proficiency tuple; final admission review passed at `4adde1e`. Subclass eligibility/reserved data passed; runtime acquisition, switching and certification effects are absent. |
| Server functional checks | 38 suites reported **642 passed, 0 failed, 3 ignored**, using real isolated PostgreSQL on 26432 and NATS on 25422. The external Keycloak case returned early without its service; this count does **not** establish a Keycloak run. A separate real-provider run at the same server source is recorded below. |
| Creation/migration | Nine base paths, explicit class 0 versus omission, appearance/sex/name validation, seven-slot concurrency and retry behavior passed. Migration fixtures for all nine legacy profiles preserved level/XP/resources/position/revision and rejected invalid profiles. No token backfill. |
| Transfer/progression | All 18 first and 31 second branches passed; all 31 third branches were refused at 85. Exact resource/CP arithmetic, racial XP/run/evasion/critical-damage vectors, learned metadata, immutable retries, privacy, fencing and failure/recovery tests passed. Scoped final runtime review passed at `8edffd1`. |
| Replay and size | Snapshot 7 / record 5 / BinaryV3 retain old snapshot 4/5/6 and record 3/4 support. The old 467-tick v4 recording retained exact replay bytes/digests. Codec/hash review `a8ac679` and outer NFR review `5b6861b` passed. Full catalogue plus one player measured **2,060,904 raw → 152,362 compressed bytes**, below the 1 MiB minus 1 KiB snapshot budget. Catalogue protobuf measured **455,324 / 4,194,304 bytes**. |
| Real Keycloak authentication | At server source `7ceb323`, the existing device-flow/authenticated character-and-ticket RPC test passed **1 / 0 / 0** in **4.06 s**, with explicit `KEYCLOAK_URL` against isolated pinned Keycloak **26.8.0** on **28080**. The provider and port were cleaned up after the run. |
| Static checks | Strict all-target Rust Clippy, formatting, generated entity drift check, protobuf build and breaking checks passed. Four pre-existing Buf lint findings remain; lint is not claimed clean. |
| Client builds and limited native evidence | Initial client `151ba16` + `1e09335` Game/Editor builds and **62 editor tests** passed with **4 offline cases skipped**. Later client `889af68` required-live automation passed **62/62 with zero skips** against server `7ceb323`. Rendered transfer **12.48 s** and persisted second launch **10.39 s** also passed. Additional historical-retry/catalogue-promotion cases are being built; final automation count and ordinary wrapper/full scenario acceptance remain pending. |

The three ignored server tests are distinct from the conditional Keycloak case:
`full_registry_tick_and_digest_measurement` was separately measured with 200 players;
`combat_tick_p99_simulation` awaits the release `api:perf-check` gate; the optional JetStream
ack-latency benchmark was not run. The 200-player debug idle tick plus snapshot projection had
p99 **2.600 ms**; it is idle overhead evidence, rather than a combat soak.

The all-target Cargo command **failed** when its standalone debug `zone_step` benchmark reached
NPC mean **3.090733 ms** (p99 **4.030963 ms**) against the unchanged **2 ms** budget.
A separate quiescent run of the documented optimized
`cargo bench -p nightfall-api --bench zone_step` **passed** at server source `7ceb323`:

| Unchanged optimized budget | Mean | p99 |
|---|---:|---:|
| 1,000 moving NPCs: mean < 2 ms | 0.143480 ms | 0.170123 ms |
| 1,000 moving players with all observers: mean < 10 ms | 5.087414 ms | 7.913913 ms |

The optimized result and 642 functional passes are separate from the debug command failure.

## Open acceptance and limits

- **Token supply/backfill requires the user's decision.** Ledger consumption is implemented;
  production grants/backfill are absent. Pre-admission fixture balances prove consumption only.
- Final automation follow-ups, ordinary wrapper execution of the Phase 2 scenarios, fresh live
  coverage of all 19 Phase 1a cases, and a current-source packaged **8-client / 20-minute combat
  soak** remain pending. Eleven Phase 2 scenario files existed at `9f3927d`; an explicit-key
  historical retry case is being added to prove that an old frozen RPC response cannot roll a
  newer live world/UI back. Final scenario count and results await client handoff. Harness
  endpoint/private-token fixes landed at `758e43f`; their ordinary native acceptance remains pending.
- The release combat p99 gate remains pending. Optional JetStream ack-latency timing is unmeasured.
- Original curated Nightfall display names are mutable design metadata; retail names remain in
  `l2_ref`. All races/sexes share the existing Manny prototype body and index-zero appearance.
  Distinct racial/sex art is unavailable. Client `889af68` shows the authoritative Master name
  as a readable gold screen-space label, without an HP bar or additional actor. Only Human XP, Elf run/evasion, and Dark Elf critical
  damage modifiers are active; SP/regen/stun/weight/spoil/craft hooks remain deferred.
- Subclass runtime, playable third classes, manual skill purchase/effects and equipment effects
  are outside this implementation. The [fixture guide](../../apps/client-unreal/Scripts/phase2-fixtures.md)
  describes isolated seeded accounts; fixtures never mutate an admitted character.
- After reviewed main integration, rebuild **Game and Editor in the root checkout** before
  cleanup, and preserve native artifacts and review/test evidence outside disposable worktrees.
  Main integration and this rebuild are not yet claimed.
- The unregistered Linux CI runner and existing two-week reliability programme are separate
  operational follow-ups, rather than a new 14-day wait before reviewed feature integration.
