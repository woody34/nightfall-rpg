The Phase2 scenario fixtures use the ordinary simulation wrappers:

```bash
Scripts/run-sim.sh --api start --artifacts Saved/TransferAttempt Scenarios/transfer.nfs
Scripts/run-sim-multi.sh --api start --artifacts Saved/ObserverAttempt \
  Scenarios/observer-a.nfs Scenarios/observer-b.nfs
python3 Scripts/run-sim-ci.py --fresh-stack --artifacts Saved/SimCIAttempt
```

Select a new, empty artifact directory for each attempt. The headers must be exact:

| Header | Unit | Seed contract |
|---|---|---|
| `# fixture: phase2-transfer` | One scenario, including reconnect | Only owner: female Human base class 0, level 40, tokens 1/1, milestone mask 3 |
| `# fixture: phase2-transfer-observer` | Exactly matching `-a` / `-b` roles | A is the transfer owner. B has a different account and only one character: male Human base class 0 at level 1 |
| `# fixture: phase2-transfer-missing-token` | One scenario | Only owner: female Human base class 0, level 20, tokens 0/0, milestone mask 1 |
| `# fixture: phase2-token-level20` | One scenario | Only owner: female Human base class 0, level 19, tokens 0/0, milestone mask 0, XP 835861. Kill the custom template `token_oracle` (raw 10235 -> Human kill 10746 XP) to go above level 20 (XP 846607) and get a tier-1 token. Retaliation by `token_sentinel` decreases the level to 19 (loss 14329 XP -> 832278 XP). The tokens stay. Respawn does not change XP. The second kill increases the level to 20 again, with no second grant. A transfer at the Class Master consumes the token (0->1). Spent-token re-crossing: a second death (loss 14329 XP -> 828695 XP, level 19), a respawn, and a third kill (10746 XP -> 839441 XP, level 20) prove that a claimed zero never refills on a real crossing. Class 1 and options 0/0 stay. |
| `# fixture: phase2-token-level40` | One scenario | Only owner: female Human base class 0, level 39, tokens 1/0, milestone mask 1, XP 15422928. Kill `token_oracle` (raw 62751 -> Human kill 65888 XP) to go above level 40 (XP 15488816) and get a tier-2 token (1/1). Retaliation by `token_sentinel` decreases the level to 39 (loss 87851 XP -> 15400965 XP). The tokens stay. Respawn does not change XP. The second kill increases the level to 40 again, with no second grant. The transfers consume 0->1->2 one time. Spent-token re-crossing: a second death (loss 87851 XP -> 15379002 XP, level 39), a respawn, and a third kill (65888 XP -> 15444890 XP, level 40) prove that a claimed zero never refills on a real crossing. Class 2 and options 0/0 stay. |
| `# fixture: phase2-token-jump19-40` | One scenario | Only owner: female Human base class 0, level 19, tokens 0/0, milestone mask 0, XP 835861. One very large `token_oracle` kill (raw 13892446 -> Human kill 14587068 XP) moves the character to level 40 (XP 15422929). The character gets the tier-1 token and the tier-2 token (1/1). The transfers consume 0->1->2 one time. Reconnect and historical retry prevent duplicate refills. |
| `# fixture: phase2-token-backfill` | One scenario | Only owner: female Human base class 0, level 40, tokens 0/0, milestone mask 0, XP 15422929 before admission. Admission immediately gives tokens 1/1 and does not change XP/level. Repeated reconnect never makes duplicates or refills. The transfers consume 0->1->2 one time. The first transfer catches up 4 auto-get skills (`1320`, `1322`, `194`, `239`). The second transfer catches up 0. Uses the shipped data (no combat overlay). |

All fixture characters start at (126,126). The owner character ID is
`01970000-0000-7000-8000-000000000020`. The observer ID is
`01970000-0000-7000-8000-000000000040`. Ordinary creation, rejected-transfer
and racial scenarios have no fixture header. Independent creation matrix names
use `-01`, `-02`, `-03`. CI always uses `-a` / `-b` as paired roles. CI rejects
orphans and headers that do not agree. The solo file names for the new token profiles
(`2-token-milestone20.nfs`, `2-token-milestone40.nfs`, `2-token-jump19-40.nfs`,
`2-token-backfill.nfs`) do not use `-a`/`-b` suffixes. This makes sure that normal solo discovery finds them.

Phase2 must have `--api start` (CI must have `--fresh-stack`). Before the wrapper starts the API, it does these steps:

1. It makes a unique Compose project with new Postgres and JetStream volumes.
2. It reserves four available loopback ports that are not the default ports.
3. It runs `nightfall-migrate up`.
4. It calls the published seeder with the exact pack and account UUIDs.

The wrapper validates the seeder manifest before startup. Each unit
gets a new `nf_phase2_fixture_*` database. If one wrapper runs a mixed sequence, it removes
each Phase2 unit after it records the export and the gates. Then it makes the
next unit again from the start. Each role receives a mode-0600 token file in the private fixture directory.
The bot argument array has `-DevTokenFile=<path>` and the supported
`-ini:Game:[/Script/Nightfall.NetSettings]:GrpcEndpoint=<owned host:port>` override.
No complete developer token is in the bot arguments or the command-line logs.

### Temporary Combat Overlay and NPC Templates

For the three real-combat crossing profiles (`phase2-token-level20`, `phase2-token-level40`,
`phase2-token-jump19-40`), the harness makes an isolated temporary overlay that the fixture controls.
The harness does this before it starts the API:
- The harness copies the shipped repository packages/data to `fixture/data`. It never changes the original files.
- The harness replaces `zones/test_zone.toml`. The new file has bounds 0..256 and a safe point at `(126, 126)`. It has a noncombat
  Class Master at `(126, 128)` with speed 0. It has a reward spawn at `(126, 125)` and a sentinel spawn
  at `(130, 126)`.
- `token_oracle`: a custom NPC template with 1 HP and a very small harmless attack. It has a positive movement speed
  (400 milli-tiles/tick) and is passive (`aggressive = false`). Clan help is disabled. It has a 10-tick corpse decay
  and a fast deterministic respawn (delay 1s, random 0s). The raw XP rewards are different for each profile:
  10,235 for level 20, 62,751 for level 40, and 13,892,446 for jump 19->40.
- `token_sentinel`: a custom NPC template with 1,000,000 HP, a positive movement speed, passive,
  and an audited lethal P.Atk (`10000.0`). This gives >5,000 damage on approximately 150 P.Def, thus immediate
  one-shot death. An attack on it causes normal retaliation/death without scripted damage.
- The API environment variables `RULES_DIR` and `ZONE_FILE` point to the overlay directory.
- `phase2-token-backfill` uses the shipped zone data with no combat overlay.
- **Unarmed Combat Cooldown and Transfer Refresh:** The architecture audit of Human unarmed combat shows
  a 17-tick full cycle with impact at tick 9. The rate is 100ms/tick (1.7s full cycle, 0.9s impact). After each
  completed oracle kill (first crossing, relevel, jump), the scenarios send `nf.StopAttack`. Then they send
  a bounded `nf.Sleep 2`. Then they send a new `nf.TransferOptions` RPC and assert the candidate
  eligibility. This is isolated fixture timing. It is not a universal player cooldown. Cached transfer
  options cannot become eligible if the scenario only waits and does not send a new refresh RPC.

### Pinned Oracle Constants and Manifests

The independent pinned source constants come from High Five tables (`experience.toml`,
`penalties.toml`, and the Human adaptable 1.05x racial trait bonus). They do not evaluate the client
runtime gameplay formulas:
- **Level 20:** Seed level 19 (XP 835,861), tokens 0/0 mask 0. The Human oracle kill gives 10,746 ->
  XP 846,607 (level 20, tier-1 token granted). Retaliation death loss 14,329 -> XP 832,278
  (level 19, the token stays). Respawn does not change XP. The second kill gives 10,746 -> XP 843,024
  (level 20, no duplicate grant). The transfer consumes the token one time (0->1). Spent-token re-crossing:
  second retaliation death loss 14,329 -> XP 828,695 (level 19, class 1 stays, tokens 0/0).
  Respawn does not change XP. The third kill gives 10,746 -> XP 839,441 (level 20). This makes sure that a claimed
  zero never refills on a real crossing. The live stats and the new options stay 0/0.
- **Level 40:** Seed level 39 (XP 15,422,928), tokens 1/0 mask 1. The Human oracle kill gives 65,888 ->
  XP 15,488,816 (level 40, tier-2 token granted, balance 1/1). Retaliation death loss 87,851 ->
  XP 15,400,965 (level 39, the balance stays). Respawn does not change XP. The second kill gives 65,888 ->
  XP 15,466,853 (level 40, no duplicate grant). The transfers consume 0->1->2 one time. Spent-token re-crossing:
  second retaliation death loss 87,851 -> XP 15,379,002 (level 39, class 2 stays, tokens 0/0).
  Respawn does not change XP. The third kill gives 65,888 -> XP 15,444,890 (level 40). This makes sure that a claimed
  zero never refills on a real crossing. The live stats and the new options stay 0/0.
- **Jump 19->40:** Seed level 19 (XP 835,861), tokens 0/0 mask 0. The Human kill gives 14,587,068 ->
  XP 15,422,929 (exactly level 40, the tier-1 and tier-2 tokens are granted, 1/1). The transfers consume
  0->1->2 one time.
- **Backfill:** Legacy level 40 (XP 15,422,929), tokens 0/0 mask 0 before admission. The visible owner
  immediately shows tokens 1/1 on admission, without a change to XP/level. Repeated reconnect never makes
  duplicates or refills. Transfer 0->1 catches up 4 auto-get skills (`1320`, `1322`, `194`, `239`).
  Transfer 1->2 catches up 0.

The harness emits a nonsecret overlay/oracle manifest at `fixture/oracle-manifest.json` and records it in
`fixture/manifest.json`. The manifest records:

- The data sources (`rules_dir`, `zone_file`, `is_overlay`).
- The SHA-256 hashes (`zone`, `token_oracle`, `token_sentinel`, and the source tables).
- The exact constants, and the provenance.

The scenarios observe the authoritative token counts through the `StatsChanged` predicates `own_tier1_tokens` and
`own_tier2_tokens`, and also through `TransferOptions`. The first-owner admission predicates
`initial_tier1_tokens` and `initial_tier2_tokens` capture the exact balances from the first
authoritative `StatsChanged` frame in the current transport admission scope. Their behavior is:

- They stay `UNKNOWN` until a valid private owner frame of the current connection arrives.
- They freeze the first observed counts in the scope.
- They reset on transport disconnect, on entity changes, or on scenario reset.

Each `in_world` line asserts the two initial predicates. This includes reconnects and the negative fixture `phase2-transfer-missing-token`.
This makes sure that a claimed zero on wire admission cannot secretly refill. It also makes sure that an uninitialized
zero cannot pass without an authoritative frame. Idempotent receipt retries with explicit developer UUIDs
make sure that replayed requests return frozen receipts and do not refill the balances.

The helper uses a minimal generated Compose file and an empty explicit env file.
It never resolves the development Compose configuration, and it never sources the root `.env`.
The fixture API/migrator/seeder environment does not include inherited database, NATS, zone, auth and Compose settings.
The helper refuses these items:

- A custom `SIM_API_URL`.
- Conflicting `DevToken`, `DevTokenFile` or `GrpcEndpoint` assignments in the extra bot arguments, at any position (including bare and mixed-case forms).
- The obsolete `NfGrpc` override.

The two API listeners must be free, and then the launched process must own them. A port
race fails the unit. The unit cannot attach to an existing API.

`SIM_API_BIN` and `SIM_MIGRATE_BIN` can select absolute executable paths from a
candidate that the coordinator approved. If they do not, the helper builds the API and the migration binaries of the current
repository before it starts the infrastructure.
`SIM_PHASE2_SEEDER` can select an explicit published seeder path. Its default is
`infra/scripts/seed-phase2-transfer-fixture.py`. This path must have the pack interface that was
published in commit `1ee4338`. The helper does not source an external fixture environment file.
Host `psql` is not necessary. A private shim validates the fixture database URI
and runs `psql` with stdin in the owned Postgres service. It forwards the decoded libpq components (`PGHOST`, `PGPORT`, the database name in
`PGDATABASE`, `PGUSER`, `PGPASSWORD`) in environment variables, never as credential
arguments. The shim never forwards `PGDATABASE` as a URI to the container psql.

The first scenario directory of each unit has these items:

- `fixture/manifest.json`.
- The SHA-256 hashes of the binaries and the seeder.
- `seed-manifest.json`.
- The migration/seed/API/Compose logs.
- `cleanup-status.json`.

The generated credentials/configuration are private
files in a mode-0700 fixture directory. The EXIT and signal traps stop the API,
keep the logs and remove only the volumes of the owned project. A teardown failure makes
the run fail. If a process-group timeout interrupts the wrapper, CI tries the owned cleanup again.
Phase2 CI never calls the root `docker compose down` or the root log collection.
The legacy fixture routing and its existing CI project guard did not change.

The replay, trace, transition coverage, contract coverage and quarantine gates continue to
apply. Transfer RPCs are not counted as WebSocket intent coverage. The failure-only
video is for diagnosis, and it does not change the primary fixture evidence.

Run the script regressions without Unreal, Docker or a live API:

```bash
python3 Scripts/test/phase2-fixture.test.py
bash Scripts/test/sim.test.sh
```

The Phase2 tests use command stubs, including a stub probe for listener ownership.
They validate the orchestration. They cannot prove native gameplay acceptance.

### V5 sentinel retaliation budget (round 2)

The r7 recordings show late retaliation that is correct to the source. They do not show regeneration that heals more than the
NPC damage. In `2-token-milestone20`, the second player attack misses at ticks
748/760/772/784/796/808 and hits at 820. The sentinel kills at 831. The client fails
at 826. The related level40 ticks are 750/762/774/786/798/810, 822, 833 and 828.

Evidence: native `nightfall-replay export --evidence-out` on the r7 unit recordings, and
`token20.jsonl:142-180`, `token40.jsonl:144-182` in the preserved round2 evidence.
The unit `.failure.json:7,12-16` gives the client boundary and the failed step. The two
first deaths were successful at 457 (same native evidence). The API export continues to
run through the client shutdown. Thus, the recording proves the later death.

Pinned game `abfde049`: `L2Character.java:4020-4025` sends EVT_EVADED on a miss.
`L2CharacterAI.java:619-621` does nothing. Only positive landed damage gets to
EVT_ATTACKED (`L2Character.java:4056-4138`). `L2AttackableAI.java:1871-1884`
adds hate and requests an attack. The engine behavior does not change.

`Formulas.java:1054-1063` clamps the hit threshold to 200..980. It misses only when
the 1000-way roll is more than the threshold. The actual player accuracy 53/73 against the
sentinel evasion 125 hits on 201/1000 rolls. `L2Character.java:887,4336` gives
12-tick player intervals at 417 speed and 20-tick sentinel intervals at 253.
`token-v5-oracle.py` examines these actual replay inputs. It calls the committed
Fraction functions in `packages/data/scripts/skill_oracle.py` (C15).

The test policy allows 64 player rolls, 3 sentinel rolls, two times the direct Manhattan
travel allowance, and 3 seconds of delivery grace. These are explicit harness
allowances. They are not source constants, and they are not a guarantee for all RNG streams. The fixture
home separation is 6 tiles (`phase3-fixtures.py:upgrade_npc_overlays`). The drift
per axis is 300/32 tiles (`npc.properties:35`, `zone/ai.rs:24-30`). The actual
received movement speed is 3.95 tiles/s. Thus:

`2*(6+2*300/32)/3.95 + (64*12+3*20)/10 + 3 = 98.3316… seconds`.

If you round this up to a 30-second harness quantum, the result is 120 seconds. The probability of 64 misses
at the floor chance is `(799/1000)^64 < 1/1000000`. This probability is explicitly not zero.
The two death waits now use 120. The total `Within` increased by 220 because the two waits changed.
All exact death-loss, level, XP, token and transfer assertions
stay. The geometry and the NPC physical inputs did not change.

`Formulas.java:665-738` shows that the fixture P.Atk 10000 of the NPC is lethal, also at the
minimum 87% spread. This is against the recorded maximum defence 103.2 and the maximum HP 1649.
`PcStatus.java:185` limits CP absorption to playable attackers. Each recorded
sentinel hit has CP absorption 0. The native Rust regression does these steps:

1. It admits base Human fighters.
2. It does real transfers to classes 1/2.
3. It attacks the real generated sentinel.
4. It records and replays each case.
5. It uses the independent oracle to audit the actual hit RNG, the lethal HP damage and the CP bypass.

All four class states must die in the same 120-second budget. A live database or a live client is not necessary for that regression.
