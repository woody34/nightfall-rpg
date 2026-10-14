# E6 handoff and live gates

The offline artifact set is on `codex/phase4-sim-harness`. The progress inbox
contains milestone SHAs, test results, source blockers and interface questions.

| Story | Implemented and tested offline | Accepted by the live gate run |
|---|---|---|
| E6.1 | Exact parser-tested grammar/carrier/reset contract; source-bound packs and variants; distinct paired roles/tokens; owned-container exec checks; dispatcher; overlay at hashed blacksmith coordinate. D112/D113 provisioning: pre-admission Phase 3 seed + 000011 pending-kit backfill (no item rows), then dev GrantItem per pack item (UUIDv5 keys, serial, UNAVAILABLE-only retry with the same key) and offline GetInventory confirmation (pack multiset incl. enchant; final revision = last receipt); all 39 packs provision; `instance:N` mapped per role | Every Phase 4 unit provisions and admits on its owned V6 fixture (the opt-in live grant test passed on API 977f87c) |
| E6.2 | Strict item export/oracle consumer (D46 sidecar); complete watermark/hash/owner/sequence/RNG/scalar/conservation checks; D117 one pre-tick baseline per owner-tick; T10 zone draws; B4 learn payments; corrupt/missing/repeated/baseline/UNKNOWN tests; additive coverage preserving old fields/reachability; all historical .nfr bytes preserved. Real producer exports validate: zone T10/B4 fixtures and the live phase4-enchant epoch at zone 1cebefa | Every unit's V6 recording replays to a match and its items evidence passes validate() and its item gates/phases |
| E6.3 | Every historical scenario/variant and assertion inventoried in phase4-regression.pending.json (inventory only, no verdict); 12 independently derived isolated real-kit stat/weight vectors; exact pending backfill kits added to fresh V6 Phase2/3 seeds; every admissible profession's alive/dead/never-admitted backfill case (D119: 58 professions, 174 cases; tier 3 out of scope; API session 17 results verify 174/174). D114: all units, required-live automation and the soak run on V6; `phase4-regression.py --suite … --backfill-results … --require-live` derives the verdict from the suite's V6 recordings (PASS, clean pipeline, replay match, V6 zone flags) and the API phase4_backfill results (every gate) | A gate-worker live suite run on V6 and the API phase4_backfill results file. A Phase 0b-3 unit that does not match on V6 is a zone-lane finding, not a relaxed expectation |
| E6.4 | Eight canonical item/casting soak roles; weapon swaps, auto-shot use, one keyed enchant per client/loop retry; private identities/tokens; unchanged 60-tile lanes/64-tile cap and peace square; per-client paid-shot/scroll/equip gate; cleanup-aware report | Candidate server/replay/cooked client, complete 20-minute packaged V6 run (seeded pending kits, overlay RULES_DIR for replay) and CPU/actor/log/DB/memory/latency/p99 budgets |
| E6.5 | All 23 exact canonical headers/bodies and declared shield/resurrection pairs; source-bound runtime variants; precise refusal sidecars; sum-of-waits+grace budgets; parser/traceability and export declarations | All actual roles/variants: nonvacuous JUnit, record/replay/export/oracle/trace/contract/coverage/verdict/cleanup; required live automation zero skips, rendering/cook proof |

Live commands require explicit candidate binaries. The actual C++ registry check
must pass; the separate offline grammar audit never substitutes for it. No baseline
relaxation or .nfr edits were made.

Scope blockers in the accepted E1 tables at branch base: player MP elixir8628 and
CP elixir8634 are deferred by skills2288:1 and2289:1 respectively
(DP@3ca488dd stats/items/08600-08699.xml:314,427; capabilities.toml).
D47 assigns these dependencies to E2 followed by E1 promotion. Book-learn skill1405
(4-item-skill-learn.nfs) is produced by zone b63f0ec (B4) and selected by its `book-learn`
phase. Mana Potion728 remains excluded by D33/default config. HP727 is the
positive potion case; the source precision setup includes the deferred cases,
without pretending their live Done When has passed.

`phase4-fixtures.md` and `phase4-evidence-contract.md` are the current E5/E2/E3
contracts. Blacksmith intents resolve the unique received noncombat entity named
`blacksmith` by default (`blacksmith_known`), because legacy nf.Target template
resolution excludes nonattackable NPCs.
