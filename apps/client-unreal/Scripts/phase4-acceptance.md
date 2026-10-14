# E6 handoff and pending gates

The offline artifact set is on `codex/phase4-sim-harness`. The progress inbox
contains milestone SHAs, test results, source blockers and interface questions.

| Story | Implemented without a live server | Pending acceptance |
|---|---|---|
| E6.1 | Exact parser-tested grammar/carrier/reset contract; source-bound packs and variants; distinct paired roles/tokens; guarded item/ledger SQL; owned-container exec checks; dispatcher; overlay at hashed blacksmith coordinate | E2/E3 low-range typed admission/checkpoint transport. Seeder refuses before writes while admission_ready=false; no fake checkpoint or production range row |
| E6.2 | Strict item export/oracle consumer; complete watermark/hash/owner/sequence/RNG/scalar/conservation checks; corrupt/missing/repeated/baseline/UNKNOWN tests; additive coverage preserving old fields/reachability; all historical .nfr bytes preserved | D46 freezes the sidecar and harness-owned schema4; E2 producer implementation and live verification remain pending. Actual V6 byte replay, D26 wire parity, canonical round trips and producer cause/draw binding; independent review |
| E6.3 | Every historical scenario/variant and assertion inventoried in phase4-regression.pending.json (inventory only, no verdict); 12 independently derived isolated real-kit stat/weight vectors; exact pending backfill kits added to fresh V6 Phase2/3 seeds; every profession's alive/dead/never-admitted backfill case. D114: all units, required-live automation and the soak run on V6; `phase4-regression.py --suite … --backfill-results … --require-live` derives the verdict from the suite's V6 recordings (PASS, clean pipeline, replay match, V6 zone flags) and the API phase4_backfill results (every gate) | A gate-worker live suite run on V6 and the API phase4_backfill results file. A Phase 0b-3 unit that does not match on V6 is a zone-lane finding, not a relaxed expectation |
| E6.4 | Eight canonical item/casting soak roles; weapon swaps, auto-shot use, one keyed enchant per client/loop retry; private identities/tokens; unchanged 60-tile lanes/64-tile cap and peace square; per-client paid-shot/scroll/equip gate; cleanup-aware report | Root Unreal lock, candidate server/replay/cooked client, typed seed transport, complete 20-minute packaged run and CPU/actor/log/DB/memory/latency/p99 budgets |
| E6.5 | All 23 exact canonical headers/bodies and declared shield/resurrection pairs; source-bound runtime variants; precise refusal sidecars; sum-of-waits+grace budgets; parser/traceability and export declarations | All actual roles/variants: nonvacuous JUnit, record/replay/export/oracle/trace/contract/coverage/verdict/cleanup; required live automation zero skips, rendering/cook proof |

Live commands require explicit candidate binaries. The actual C++ registry check
must pass; the separate offline grammar audit never substitutes for it. No Source/
changes, live server/container/Unreal runs, baseline relaxation or .nfr edits were
made. A pre-existing Python native coverage test started Cargo internally and was
interrupted; no native result is claimed. Subsequent validation is Python/shell.

Scope blockers in the accepted E1 tables at branch base: player MP elixir8628 and
CP elixir8634 are deferred by skills2288:1 and2289:1 respectively
(DP@3ca488dd stats/items/08600-08699.xml:314,427; capabilities.toml).
Book-learn skill1405 is also deferred by `stat:enlargeAbnormalSlot`
(capabilities.toml). D47 explicitly assigns these dependencies to E2 followed by E1 promotion. These positive units require E1/E2 to promote the complete
source capability. Mana Potion728 remains excluded by D33/default config. HP727 is the
positive potion case; the source precision setup includes the deferred cases,
without pretending their live Done When has passed.

`phase4-fixtures.md` and `phase4-evidence-contract.md` are the current E5/E2/E3
contracts. Blacksmith intents resolve the unique received noncombat entity named
`blacksmith` by default (`blacksmith_known`), because legacy nf.Target template
resolution excludes nonattackable NPCs.
