Phase2 scenario fixtures use the ordinary simulation wrappers:

```bash
Scripts/run-sim.sh --api start --artifacts Saved/TransferAttempt Scenarios/transfer.nfs
Scripts/run-sim-multi.sh --api start --artifacts Saved/ObserverAttempt \
  Scenarios/observer-a.nfs Scenarios/observer-b.nfs
python3 Scripts/run-sim-ci.py --fresh-stack --artifacts Saved/SimCIAttempt
```

Select a new, empty artifact directory for each attempt. Headers must be exact:

| Header | Unit | Seed contract |
|---|---|---|
| `# fixture: phase2-transfer` | One scenario, including reconnect | Sole owner: female Human base class 0, level 40, tokens 1/1, milestone mask 3 |
| `# fixture: phase2-transfer-observer` | Exactly matching `-a` / `-b` roles | A is the transfer owner; B has a distinct account and sole male Human base class 0 character at level 1 |
| `# fixture: phase2-transfer-missing-token` | One scenario | Sole owner: female Human base class 0, level 20, tokens 0/0, milestone mask 1 |
| `# fixture: phase2-token-level20` | One scenario | Sole owner: female Human base class 0, level 19, tokens 0/0, milestone mask 0, XP 835861. Kill custom template `token_oracle` (raw 10235 -> Human kill 10746 XP) to cross level 20 (XP 846607) and earn tier-1 token. Retaliation by `token_sentinel` delevels to 19 (loss 14329 XP -> 832278 XP), tokens preserved. Respawn XP unchanged; second kill relevels to 20 without double grant. Transfer at Class Master consumes token (0->1). Spent-token re-crossing: second death (loss 14329 XP -> 828695 XP, level 19), respawn, and third kill (10746 XP -> 839441 XP, level 20) prove claimed zero never refills on real crossing; class 1 and options 0/0 retained. |
| `# fixture: phase2-token-level40` | One scenario | Sole owner: female Human base class 0, level 39, tokens 1/0, milestone mask 1, XP 15422928. Kill `token_oracle` (raw 62751 -> Human kill 65888 XP) to cross level 40 (XP 15488816) and earn tier-2 token (1/1). Retaliation by `token_sentinel` delevels to 39 (loss 87851 XP -> 15400965 XP), tokens preserved. Respawn XP unchanged; second kill relevels to 40 without double grant. Transfers consume 0->1->2 once. Spent-token re-crossing: second death (loss 87851 XP -> 15379002 XP, level 39), respawn, and third kill (65888 XP -> 15444890 XP, level 40) prove claimed zero never refills on real crossing; class 2 and options 0/0 retained. |
| `# fixture: phase2-token-jump19-40` | One scenario | Sole owner: female Human base class 0, level 19, tokens 0/0, milestone mask 0, XP 835861. Single massive `token_oracle` kill (raw 13892446 -> Human kill 14587068 XP) jumps to level 40 (XP 15422929), earning both tier-1 and tier-2 tokens (1/1). Transfers consume 0->1->2 once. Reconnect and historical retry prevent duplicate refills. |
| `# fixture: phase2-token-backfill` | One scenario | Sole owner: female Human base class 0, level 40, tokens 0/0, milestone mask 0, XP 15422929 before admission. Admission immediately grants tokens 1/1 without changing XP/level. Repeated reconnect never duplicates or refills. Transfers consume 0->1->2 once; first transfer catches up 4 auto-get skills (`1320`, `1322`, `194`, `239`), second catches up 0. Uses shipped data (no combat overlay). |

All fixture characters start at (126,126). Owner character ID is
`01970000-0000-7000-8000-000000000020`; observer ID is
`01970000-0000-7000-8000-000000000040`. Ordinary creation, rejected-transfer,
and racial scenarios have no fixture header. Independent creation matrix names
use `-01`, `-02`, `-03`; CI always treats `-a` / `-b` as paired roles and rejects
orphans or mismatched headers. Solo filenames for the new token profiles
(`2-token-milestone20.nfs`, `2-token-milestone40.nfs`, `2-token-jump19-40.nfs`,
`2-token-backfill.nfs`) avoid `-a`/`-b` suffixes to ensure normal solo discovery.

Phase2 requires `--api start` (CI requires `--fresh-stack`). The wrapper generates
a unique Compose project with fresh Postgres and JetStream volumes, reserves
four available loopback ports excluding defaults, runs `nightfall-migrate up`,
and calls the published seeder with the exact pack and account UUIDs before
starting the API. The seeder manifest is validated before startup. Every unit
gets a new `nf_phase2_fixture_*` database. A mixed single-wrapper sequence tears
down each Phase2 unit after recording export and gates, then provisions the
next unit afresh. Each role receives a mode-0600 token file in the private fixture directory.
The bot argument array carries `-DevTokenFile=<path>` and the supported
`-ini:Game:[/Script/Nightfall.NetSettings]:GrpcEndpoint=<owned host:port>` override.
No complete developer token appears in bot arguments or command-line logs.

### Temporary Combat Overlay and NPC Templates

For the three real-combat crossing profiles (`phase2-token-level20`, `phase2-token-level40`,
`phase2-token-jump19-40`), the harness provisions an isolated, fixture-owned temporary overlay
before starting the API:
- Shipped repository packages/data is copied to `fixture/data`, never modifying original files.
- `zones/test_zone.toml` is replaced with bounds 0..256, safe point at `(126, 126)`, noncombat
  Class Master at `(126, 128)` with speed 0, reward spawn at `(126, 125)`, and sentinel spawn
  at `(130, 126)`.
- `token_oracle`: custom NPC template with 1 HP, tiny harmless attack, positive movement speed
  (400 milli-tiles/tick), passive (`aggressive = false`), clan help disabled, 10-tick corpse decay,
  and fast deterministic respawn (delay 1s, random 0s). Raw XP rewards are profile-specific:
  10,235 for level 20, 62,751 for level 40, and 13,892,446 for jump 19->40.
- `token_sentinel`: custom NPC template with 1,000,000 HP, positive movement speed, passive,
  and audited lethal P.Atk (`10000.0`, delivering >5,000 damage on ~150 P.Def for immediate
  one-shot death). Attacking it triggers normal retaliation/death without scripted damage.
- API environment variables `RULES_DIR` and `ZONE_FILE` are pointed to the overlay directory.
- `phase2-token-backfill` uses shipped zone data with no combat overlay.
- **Unarmed Combat Cooldown and Transfer Refresh:** Architecture-audited Human unarmed combat has
  a 17-tick full cycle with impact at tick 9 at 100ms/tick (1.7s full cycle, 0.9s impact). After each
  completed oracle kill (first crossing, relevel, jump), scenarios issue `nf.StopAttack` followed by
  a bounded `nf.Sleep 2` before issuing a fresh `nf.TransferOptions` RPC and asserting candidate
  eligibility. This is isolated fixture timing rather than a universal player cooldown; cached transfer
  options cannot become eligible merely by waiting without a fresh refresh RPC.

### Pinned Oracle Constants and Manifests

Independent pinned source constants are derived from High Five tables (`experience.toml`,
`penalties.toml`, and the Human adaptable 1.05x racial trait bonus) without evaluating client
runtime gameplay formulas:
- **Level 20:** Seed level 19 (XP 835,861), tokens 0/0 mask 0. Human oracle kill award 10,746 ->
  XP 846,607 (level 20, tier-1 token granted). Retaliation death loss 14,329 -> XP 832,278
  (level 19, token preserved). Respawn XP unchanged; second kill awards 10,746 -> XP 843,024
  (level 20, no duplicate grant). Transfer consumes token once (0->1). Spent-token re-crossing:
  second retaliation death loss 14,329 -> XP 828,695 (level 19, class 1 retained, tokens 0/0);
  respawn XP unchanged; third kill awards 10,746 -> XP 839,441 (level 20), verifying claimed
  zero never refills on real crossing. Live stats and fresh options remain 0/0.
- **Level 40:** Seed level 39 (XP 15,422,928), tokens 1/0 mask 1. Human oracle kill award 65,888 ->
  XP 15,488,816 (level 40, tier-2 token granted, balance 1/1). Retaliation death loss 87,851 ->
  XP 15,400,965 (level 39, balance preserved). Respawn XP unchanged; second kill awards 65,888 ->
  XP 15,466,853 (level 40, no duplicate grant). Transfers consume 0->1->2 once. Spent-token re-crossing:
  second retaliation death loss 87,851 -> XP 15,379,002 (level 39, class 2 retained, tokens 0/0);
  respawn XP unchanged; third kill awards 65,888 -> XP 15,444,890 (level 40), verifying claimed
  zero never refills on real crossing. Live stats and fresh options remain 0/0.
- **Jump 19->40:** Seed level 19 (XP 835,861), tokens 0/0 mask 0. Human kill award 14,587,068 ->
  XP 15,422,929 (exactly level 40, both tier-1 and tier-2 tokens granted 1/1). Transfers consume
  0->1->2 once.
- **Backfill:** Legacy level 40 (XP 15,422,929), tokens 0/0 mask 0 before admission. Visible owner
  immediately displays tokens 1/1 on admission without XP/level change. Repeated reconnect never
  duplicates or refills. Transfer 0->1 catches up 4 auto-get skills (`1320`, `1322`, `194`, `239`);
  transfer 1->2 catches up 0.

A nonsecret overlay/oracle manifest is emitted at `fixture/oracle-manifest.json` and recorded in
`fixture/manifest.json`. It captures data sources (`rules_dir`, `zone_file`, `is_overlay`), SHA-256
hashes (`zone`, `token_oracle`, `token_sentinel`, and source tables), exact constants, and provenance.
Authoritative token counts are observed via `StatsChanged` predicates `own_tier1_tokens` and
`own_tier2_tokens` in addition to `TransferOptions`. First-owner admission predicates
`initial_tier1_tokens` and `initial_tier2_tokens` capture exact balances from the first
authoritative `StatsChanged` frame in the current transport admission scope. They remain `UNKNOWN`
before a valid current-connection private owner frame arrives, freeze the first observed counts in
scope, and reset on transport disconnect, entity changes, or scenario reset. Every `in_world` line
(including reconnects and negative fixture `phase2-transfer-missing-token`) asserts both initial
predicates to guarantee wire admission claimed zero cannot secretly refill or pass uninitialized
zero without an authoritative frame. Idempotent receipt retries using explicit developer UUIDs
verify that replayed requests return frozen receipts without refilling balances.

The helper uses a minimal generated Compose file and an empty explicit env file.
It never resolves development Compose configuration or sources root `.env`.
Inherited database, NATS, zone, auth and Compose settings are excluded from the
fixture API/migrator/seeder environment. Supplying a custom `SIM_API_URL` or
conflicting `DevToken`, `DevTokenFile` or `GrpcEndpoint` assignments anywhere
in extra bot arguments (including bare and mixed-case forms) is refused.
The obsolete `NfGrpc` override is also refused.
Both API listeners must be free and then owned by the launched process. A port
race fails the unit; it cannot attach to an existing API.

`SIM_API_BIN` and `SIM_MIGRATE_BIN` can select absolute executable paths from a
coordinator-approved candidate. Otherwise the helper builds the current
repository's API and migration binaries before starting infrastructure.
`SIM_PHASE2_SEEDER` can select an explicit published seeder path; its default is
`infra/scripts/seed-phase2-transfer-fixture.py`, requiring the pack interface
published in commit `1ee4338`. No external fixture environment file is sourced.
Host `psql` is unnecessary: a private shim validates the fixture database URI
and runs `psql` with stdin inside the owned Postgres service. It forwards decoded libpq components (`PGHOST`, `PGPORT`, database name in
`PGDATABASE`, `PGUSER`, `PGPASSWORD`) in environment variables, never credential
arguments. `PGDATABASE` is never forwarded as a URI to container psql.

Each unit's first scenario directory contains `fixture/manifest.json`, binary
and seeder SHA-256 hashes, `seed-manifest.json`, migration/seed/API/Compose logs,
and `cleanup-status.json`. Generated credentials/configuration are private
files inside a mode-0700 fixture directory. EXIT and signal traps stop the API,
retain logs and remove only the owned project's volumes; teardown failure is
blocking. CI retries owned cleanup if a process-group timeout interrupts the
wrapper. Phase2 CI never invokes root `docker compose down` or root log collection.
Legacy fixture routing and its existing CI project guard are unchanged.

Replay, trace, transition coverage, contract coverage and quarantine gates still
apply. Transfer RPCs are not counted as WebSocket intent coverage. Failure-only
video is diagnostic and does not alter the primary fixture evidence.

Run script regressions without Unreal, Docker or a live API:

```bash
python3 Scripts/test/phase2-fixture.test.py
bash Scripts/test/sim.test.sh
```

The Phase2 tests use command stubs, including a stub listener ownership probe.
They validate orchestration and cannot establish native gameplay acceptance.
