# Phase 4 API evidence matrix

This is the E3 implementation/evidence register for `api-guidelines` §4. A storage fixture,
transport fixture or domain replay does not establish live Phase 4 rollout. Required live
rows remain open until the canonical runtime and received-message harness are published.
The accepted E1 bundle is imported through `1ff2eb9`; no temporary runtime tables are used.

## Endpoints and transport

| Surface | Use case / no-write checks | Real transport evidence | Adapter evidence | Live limitation |
|---|---|---|---|---|
| GetItemCatalog, data_version=2 | Immutable complete catalogue, exact compiled item/skill binding; absent/deferred/corrupt ports | `grpc_item_catalog.rs`: authentication, every projected catalogue field, unsupported/corrupt sanitized statuses | Startup-resolved immutable catalogue; no database method | Catalogue availability does not enable item intents |
| GetInventory | D15: absent catalogue/zone => UNAVAILABLE; admitted => FAILED_PRECONDITION before repository read; detached owner => paused baseline | `grpc_inventory.rs`: exact baseline/header/capacities, auth/owner/UUIDs, legacy/pending/corrupt rows, absent ports; actual admitted owner refusal | `phase4_retained_inventory.rs` and PostgreSQL transport case: parent revision lock, full widths/history, scalar-row agreement | Detached tick/incarnation/baseline_tick=0 cannot hydrate a live WS owner |
| GrantItem | Exact dev gate, runtime availability before DB reads, account/operation lookup before fresh eligibility, bounded IDs/counts, owner/storable capability | `grpc_grant_item.rs`: all documented statuses, frozen retry after later inventory changes, concurrent same key, different-body conflict, absent ports, stopped/pending actor, sanitized errors | `phase4_item_settlement.rs`: immutable Grant result and full baseline, scoped lookup, atomic receipts and concurrent key settlement | Transport fixture injects an actor port; actual server-only materialization and ack-return runtime remain required |
| CreateCharacter kit marker | Canonical base-class manifest/hash and immutable request scope; no owned item allocation during creation | `grpc_pending_starter_kits.rs`: all nine base lineages including Orc/Dwarf exceptions, same-key wire retry records no second kit/event | Item admission tests and `postgres_phase4_items.rs`: kit/backfill upgrade/rerun/resume, no speculative item rows; `postgres_pending_item_scope.rs`: owner/operation constraints and concurrency | First admission must materialize before visibility through Z's final runtime |
| Character/GetCharacter/ListMyCharacters fields15/16 | Retained read header derived under the existing parent lock; legacy None omitted | `grpc_get_character.rs`, PostgreSQL `grpc_inventory.rs`: exact retained inventory revision/rules hash after write | Memory/Pg settlement and retained-inventory suites | Frozen legacy transfer reply has no invented item header; V6 transfer enrichment still requires its canonical context |
| LearnSkill with items | Existing exact-rank SP/receipt checks retained; unified item writer accepts typed learning receipt in the same transaction | Existing Phase3 learning socket/actor suites remain required regressions | Typed optional receipt is retained in V6 pending checkpoint metadata; item+learning storage cost matrix remains to complete | Book eligibility/payment and item-cast hooks require the actual Z producer; Phase3-only passes do not close E3.4 |
| WS intents19..28 | All ten normalize to canonical ItemCommand; account comes from server context; malformed slots/keys/widths/targets rejected | `interface/zone_mapping/items.rs` field/optional/width tests and `phase4_wire.rs` additive contracts | Scoped immutable item receipts are available to actor preflight | Public decoder remains fenced pending complete baseline/delta/enrichment, recorded Known/Conflict preflight and runtime admission |
| WorldEvent31..33 | Enchant/Refine/ItemOperationResult typed frozen mapping, exact original tick/receipt/refunds/recipe | Mapping tests plus owner-only domain/replay result tests | Result is frozen in the same parent item transaction | Successful receipt mapping alone is not full live mutation coverage |
| WorldEvent26..30 and enriched public/owner outputs | Z owns canonical InventorySync/Changed/EquipmentChanged/Weight/Penalty producers | Published canonical view commits queued for integration; tag41 enrichment still pending | Private checkpoint40 is excluded from observer/AOI/WS output | Do not synthesize live baselines from retained SQL rows or lift WS fence from DTO availability |

## Durability and recovery

| Required property | Concrete evidence | State |
|---|---|---|
| Parent, full paused item/skill state, rows, append-only ledger, frozen receipts, pending completion, allocator cursor and outbox share one transaction | `phase4_item_settlement.rs` memory/Pg parity and PostgreSQL injected outbox failure | Passed with owned PostgreSQL; no adapter skip |
| Exact checkpoint-key retry precedes later revision and request eligibility | Same settlement suite, concurrent same checkpoint and competing checkpoint keys | Passed; different committed request cannot acknowledge a speculative draft |
| Triple identity `(account,operation,key)` | `postgres_pending_item_scope.rs`; same-tick Grant/Use UUID reuse in settlement; SQL owner/kind-operation/uniqueness constraints | Passed; pending rows only starter/backfill, direct Grant has no pending row |
| Signed count conservation across interleaved grant/consume and transient created/destroyed IDs | Settlement rejection/parity suite | Passed; creation tick and instance ownership/identity retained |
| Pure owner sidecar change does not invent a ledger/outbox instance | Toggle-after-destruction settlement case | Passed; frozen receipt and revision still commit |
| Domain variants and stable committed parent revision/ordinal | `phase4_item_events.rs`, domain item facts, actual equipment checkpoint tests | Current ledger changes map to exact outbox variants; D38 unchanged enchant attempt audit source remains required |
| One final full transaction for an owner's entire tick | Actual Unequip+Equip+duplicate domain tick in `checkpoint_items_tests.rs` | Passed: one parent commit, two scoped receipts and two actual ledger/outbox facts |
| Log ack precedes DB dispatch; DB ack precedes acknowledgement and observable tick | Actual ZoneActor barrier with InMemoryEventLog and owned JetStream; blocked writer and lost response | Passed; no early output/ack, identical retry, one ledger/outbox batch |
| Permanent/stale/absent writer or absent private projection refuses persistence/release | Existing checkpoint stale/permanent tests, new item lane refusal tests and actor durable-gate guard | Passed: explicit actor no-durable-log negative check leaves no write, acknowledgement or output |
| Exact pending full transaction survives snapshot without changing simulation digests | `ItemCheckpointWorkSnapshot`, `CheckpointRequestSnapshot.phase4`, `CheckpointSnapshot.learning`; actual equipment pending roundtrip and unified learning-context roundtrip | Passed: exact request/events/projection/allocation/typed receipt; owner/key/source/tick/version checks; no reconstruction from a later boundary |
| Legacy bytes and raw extension-presence refusal | Four historical replay fixtures; recursive `item_work`/`learning`/`phase4` presence checks including null | Required after every canonical import; legacy absent extensions are omitted |
| Every accepted enchant attempt publishes ItemEnchanted, including unchanged target | D38/D49 bind the canonical EnchantAttempt audit captured at resolution; API will consume the published DTO | Open; do not fabricate a target ledger row or reconstruct from final inventory |
| Unified actual SP/book/rank/items/receipt atomicity and item-cast recovery | E3.4 actual Z runtime plus storage/actor matrix | Open; no gameplay eligibility claim from a transaction fixture |
| Actual Grant runtime, offline final core and first-kit visibility fence | `ItemGrantRuntime::enqueue_grant_item`, canonical MaterializePendingItems publication | Open; existing RPC actor fixture is not this proof |

## Run policy and remaining acceptance

Use only the owned dependency stack:
`DATABASE_URL=postgres://nightfall:nightfall@127.0.0.1:27432/nightfall`
and `NATS_URL=nats://127.0.0.1:27422`. Use the debug profile, `CARGO_BUILD_JOBS=2`, and
`--test-threads=2`. A conditional adapter/JetStream test with no URL is a skip, never a pass.
Strict all-target Clippy, formatting and historical byte replay accompany API milestones.
Per-run logs and commit receipts are in the worker's `api-progress.md` coordination note.

E3.5 remains a named TODO for `4-starter-kit.nfs`, `4-potion.nfs`, `4-escape-scroll.nfs`,
`4-resurrection-scroll-a.nfs`, `4-resurrection-scroll-b.nfs`, `4-item-skill-learn.nfs` and
`4-grant-item-denied.nfs` (including required `.allow` sidecars). D40 assigns received-message
grammar and fixture names to E6. The harness has now published its offline handoff at
`e4948d9`, with canonical scenarios/packs and the D46 frozen evidence contract. E3 will
consume that handoff after the low-range typed admission, actual exporter and runtime
are ready. No invented predicate, scenario receipt, skipped required-live pass or E3
Done When closure is recorded here.
