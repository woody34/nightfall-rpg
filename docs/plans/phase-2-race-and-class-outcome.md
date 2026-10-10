# Phase 2 race and class: verification outcome

Status: implementation, ordinary native acceptance, current packaged combat soak, and root main merge / post-merge Game and Editor builds verified on 2026-10-09. User accepted once-per-character transfer tokens at 20/40 including eligible existing backfill (2026-10-10): policy accepted, E4 implementation and validation pending (no production grants or backfill at baseline `88b9063`). Whole Phase 2 completion and full normal progression are not claimed.
See the [executable plan](phase-2-race-and-class.md), [reference design](../planning/02-race-and-class.md),
and [class transfer observability](../engineering/class-transfer-observability.md).

## Implemented contracts

Five races and 89 classes retain the 9/18/31/31 distribution and 170-point base-stat budget.
Authoritative level cap is **85**; playable transfers stop at tier **2** (levels **20/40**). Third classes
are metadata only, even at 85. Production creation starts at **(0,0)**. Players reach Class Master at
**(126,128)**, radius **3**, by normal movement. Testing fixtures seed at **(126,126)** before admission;
fixtures never mutate an admitted character.

Pinned L2J datapack `3ca488dd2bd0bfaca43e378886a3c2e37968153a` supplies 7,565 growth rows
(22,695 HP/MP/CP values), 39 direct learning trees with 6,927 entries, 3,521 proficiency rows, and
378 known skill references. Fifty other direct trees are deferred with ancestor metadata inherited.
Auto-get metadata persists at max learned rank without SP charges and survives deleveling.
Skill execution, manual SP learning, and equipment effects remain future work. Subclass eligibility
rules and models are defined; runtime switching, acquisition, and certification effects are deferred.
See [data provenance](../../packages/data/SOURCES.md).

Transfers consume an existing typed token, preserve resource percentages with floor rounding, and
retain at least one HP for living characters. CP starts at zero, remains owner-private, and does
not absorb damage. Durability order is **log acknowledgement → atomic class/token/receipt/outbox checkpoint →
RPC success and world output**. At most two successful receipts per character retain the original full
response, including offline retries. Under the accepted 2026-10-10 policy (E4 implementation pending),
claim bits are authoritative forever (claimed zero never refills; death/delevel never clears bits). In tier
order, an unclaimed tier with existing positive balance or completed lineage/validated receipt marks claimed
without adding a token; otherwise level >= 20/40 grants 1 and marks claimed. Every claim-bit change, including mark-only reconciliation, is critical: log acknowledgement -> atomic checkpoint -> visibility/output. First post-upgrade admission
reconciles eligible existing characters via deterministic recorded Join and a critical checkpoint before
visibility (no bulk offline SQL backfill). True live upward crossings use the same helper in tier order.
Historical snapshot 7 / record 5 / BinaryV3 epochs stay policy-disabled with no-grant replay. Recover if needed and durably close the old epoch before establishing a fresh policy-enabled baseline, then permit live admission. Architecture plans snapshot8/record6/BinaryV4; implementation and compatibility validation remain pending.

## Verified evidence

| Gate | Result and scope |
|---|---|
| Data and eligibility | 13 Rust catalogue tests and 3 Python provenance tests passed. Independent audits matched every growth, learning, and proficiency tuple; admission review passed at `4adde1e`. Subclass eligibility and reserved types passed; runtime switching and certification effects absent. |
| Server functional checks | 38 suites reported **642 passed, 0 failed, 3 ignored**, using real isolated PostgreSQL on 26432 and NATS on 25422. The external Keycloak case conditionally returned early without its service; that count did not establish Keycloak. A separate real-provider run is recorded below. |
| Creation and migration | Nine base paths, explicit class 0 versus omission, appearance/sex/name validation, 7-slot concurrency, and retries passed. Migration fixtures for nine legacy profiles preserved level/XP/vitals/position/revision and rejected invalid profiles (historical Oct 9 receipt; migration 000007 performed no token backfill, which remains historical evidence rather than new-policy verification). |
| Transfer and progression | All 18 first and 31 second branches passed; 31 third branches refused at 85. Resource/CP arithmetic, racial XP/run/evasion/crit vectors, learned metadata, immutable retries, privacy, fencing, and recovery passed. Runtime review `8edffd1` passed. |
| Replay and size | Snapshot 7 / record 5 / BinaryV3 retain snapshot 4/5/6 and record 3/4. 467-tick v4 recording retained exact replay bytes/digests. Codec review `a8ac679` and outer NFR review `5b6861b` passed. Snapshot measured **2,060,904 raw → 152,362 compressed bytes** (< 1 MiB − 1 KiB budget). Protobuf catalogue: **455,324 / 4,194,304 bytes**. |
| Real Keycloak authentication | At server `7ceb323`, device-flow test passed **1 / 0 / 0** in **4.06 s** with explicit `KEYCLOAK_URL` against pinned Keycloak **26.8.0** on **28080**. Provider and port cleaned up (`/tmp/nightfall-phase2/api-keycloak-metadata.json`). |
| Release combat performance | At server `7ceb323`, release `combat_tick_p99_simulation` passed on quiescent host (`/tmp/nightfall-phase2/api-combat-perf-metadata.json`): 200 players + 200 Keltirs over 500 ticks measured p50 **211 µs**, p99 **670 µs** (1,000 attacks); 50 pairs measured p50 **31 µs**, p99 **125 µs** (253 attacks), both below the **20 ms** budget. |
| Packaged combat soak | `/tmp/phase2-client-final-soak-01/report.json`: **PASS**, wrapper exit 0, errors `[]`, cleanup exit 0. **8 clients × 1,200 s** requested; **23 cycles each / 184 total**, actual **1,231.441–1,250.215 s** per client. Combat p99 **16.087356 ms < 20 ms**. Histogram and complete recording: **12,612 ticks** each. Replay: **96,281 outputs / 22,662,346 bytes**, byte-identical, **0 digest-only**. Game staging passed **49.25 s**, SHA-256 `3af752926c8ca3a384c736509bf18217d66fbb3cf27555b7fe002bc47b7eff99` matches built Game. Client `4f61f0a` has no native/production/API/data difference from tested `92b34d2`; frozen release API/replay. |
| Static checks | Strict all-target Rust Clippy, formatting, entity drift check, protobuf build, and breaking checks passed. Four pre-existing Buf lint findings remain; lint is not claimed clean. |
| Required-live client automation | Client `6820c62` automation passed **62/62 with zero skips** against API `7ceb323`. Rendered transfer (12.48 s) and persisted second launch (10.39 s) verified wire decoding, stale-session fences, owner privacy, and reconnect. |
| Ordinary native scenario suite | Coordinator suite (`/tmp/phase2-client-full-suite-01/summary.md`, `suite.xml`) at client `92b34d2` (with `1a9c5cb` and API `7ceb323`) measured **1,193.284 s (19m 53.284s)**, exit **0**, across **30 scenario files / 25 logical units** (19 Phase 1a + 11 Phase 2): **25 PASS, 0 FAIL, 0 quarantine**, with 35 recordings, 35 replay logs, and 35 trace HTML files retained; all owned stacks cleaned. |
| Telemetry and diagrams | Phase 2 telemetry imports (`b50dbc4` / `b8a7151`) add five Grafana transfer panels to `nightfall-api` (`docs/engineering/class-transfer-observability.md`). Prometheus exporter validation verified scrape counts (0→1=1, 1→2=1, success=2) with fixed `otel_scope_name="nightfall-api"`. Offline class tree (`docs/diagrams/phase-2-class-tree.html`, 89 nodes / 80 edges) and transfer diagram (`docs/diagrams/phase-2-class-transfer.html`) passed integrity checks, corruption probes, and 1440/390px renders. |
| Root main build receipt | Main `00b9ac168414b99513726903cef829d3bda9ea0c` merged and pushed. Root Game **PASS, 42.11 s**; Editor **PASS, 32.70 s**; focused class smoke **6/6 PASS, zero skips**, all exit 0. Public receipt: `~/.config/Claude/side-session-notes/phase2-codex-2026-10-09/client-evidence/root-build/receipt.json`. Tracked root clean, user trace preserved; generated bridge modules verified root-local. |

## Ignored tests, benchmark results, and diagnostic failures

The three ignored server tests in the 642-pass suite are distinct from Keycloak:
`full_registry_tick_and_digest_measurement` separately measured 200-player idle p99 at **2.600 ms**;
`combat_tick_p99_simulation` separately passed in the release performance gate above (p99 670 µs);
only the optional JetStream ack-latency benchmark remains unrun.

The Cargo all-target debug command **failed** when standalone debug `zone_step` reached
NPC mean **3.090733 ms** (p99 **4.030963 ms**) against the unchanged **2 ms** budget. A separate quiescent run of the documented optimized
`cargo bench -p nightfall-api --bench zone_step` **passed** at server source `7ceb323`:

| Optimized quiescent budget | Mean | p99 |
|---|---:|---:|
| 1,000 moving NPCs: mean < 2 ms | 0.143480 ms | 0.170123 ms |
| 1,000 moving players with all observers: mean < 10 ms | 5.087414 ms | 7.913913 ms |

The optimized benchmark results and 642 functional passes are separate from the debug command failure.

Native `diagnostic-01` **failed**: its test expected a nonzero second-transfer grant, and the coverage reader accepted schema 1 only rather than emitted schema 2. Replay and cleanup passed. Corrected `retry-02` **passed** (65 steps in 2.94 s, wrapper 12 s), including replay, trace, contracts, schema 2 coverage, and cleanup; the subsequent 25-unit suite passed.

Direct pinned XML oracle verification of inherited Human fighter progression at level 40 confirms identical
ranks across classes 0, 1, and 2: skills 1320 (rank 4), 1322 (rank 1), 194 (rank 1), and 239 (rank 2).
The pre-admission fixture provisions a sparse ledger containing only `racial.adaptable 1` without L2
auto-get entries. Consequently, the first transfer catches up four omitted metadata keys and the second
zero; these are catch-up reconciliations rather than class-specific new unlocks. A fully learned class 0
ledger at 40 would show zero delta on both transfers.

`2-class-transfer-reconnect.nfs` replays keyA after keyB and reconnect: the frozen RPC returns class 1 while live state remains class 2, without another event or token mutation. This extends an existing file.

Aggregate suite coverage is informational:
contract events **13/13**, payloads **3/3**, intents **5/6** (missing `stop_move`), reject reasons **3/13**
(10 unhit), close codes **1/4** (missing 4400, 4408, 4429), reachable NPC transitions **7/10**, and
player attack states **3/4**. Informational gaps are not gate failures; 100% contract coverage is
neither claimed nor required.

## Open acceptance and limits

- **Token policy accepted; E4 implementation and validation pending.** The 2026-10-10 policy above
  requires real native crossing, delevel/re-cross, and admission backfill evidence, separately from
  consumption-only seeded fixtures. Grant instrumentation and dashboard validation also remain pending.
  Migration 000007 and the Oct 9 suite/soak/root receipts are historical evidence, not verification of
  the new token implementation. Baseline `88b9063` has no production grant mechanism or grant metric.
- **External CI runner and reliability programme.** The Linux self-hosted runner remains unregistered.
  The existing two-week reliability programme is pending as a separate operational follow-up and does
  not impose a new 14-day delay on reviewed feature integration.
- **Explicit prototype limitations.** Original curated Nightfall display names are design metadata;
  retail names remain in `l2_ref`. Race, sex, and index-zero appearance metadata controls work with shared prototype/placeholder bodies. `SKM_Manny_Simple` is not imported or cooked in this checkout; remote-player fallback logs and the rendered world smoke show cylinders. Distinct racial/sex 3D art is unavailable. The Class Master has a readable gold screen-space name without an HP bar or extra actor. Only Human XP, Elf run/evasion,
  and Dark Elf critical damage traits are active. Subclass runtime, playable third classes, manual SP
  learning, and equipment effects are excluded.
