**Verdict: follow-up fixes required. Phase 1 acceptance is incomplete.**

**High — P1**

- **Recovery silently skips aged epochs** — [zone_runtime.rs:78](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/zone_runtime.rs:78). Recovery discovers epochs only through surviving JetStream snapshots. After seven days, the initial snapshot expires before later applied records; discovery returns `None`, allowing admission without recovering pending death/level checkpoints. **Fix:** consult the durable epoch index, fail closed on missing recovery history, and retain or periodically replace the recovery baseline. Test snapshot expiry with an uncommitted critical checkpoint.

**Medium — P2**

- **XP can go to someone who did not land the killing blow** — [state_combat.rs:668](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/domain/zone/state_combat.rs:668). Same-tick pending attackers compete using historical damage, even though their swings are cancelled and never validated at impact. This contradicts plan §3.3. **Fix:** credit the eligible living killing-blow player; replace the test that enshrines damage-share allocation and reconcile the architecture/sequence diagram.

- **Client discards authoritative XP** — [ProtoCodec.cpp:94](/home/matt-woodruff/repos/nightfall-rpg/apps/client-unreal/Source/Nightfall/Net/ProtoCodec.cpp:94). `StatsChanged.xp` never reaches the projection. Reconnect displays `XP --`; death leaves pre-penalty XP displayed until another gain. **Fix:** carry XP through `FStatsChanged` and update `Own.Xp`/`bXpKnown`; test admission, death/delevel and reconnect through encoded frames.

- **Respawn does not advance the client’s life fence** — [CombatStateSubsystem.cpp:247](/home/matt-woodruff/repos/nightfall-rpg/apps/client-unreal/Source/Nightfall/Combat/CombatStateSubsystem.cpp:247). The bridge drops `EntityRespawned.incarnation`; the handler checks only tick and leaves `Incarnation` unchanged. **Fix:** propagate the incarnation, reject older lives and update both combat state and network tombstones. Test stale respawn/death/spawn events after respawn.

- **Mid-fight snapshots omit checkpoint state** — [checkpoint.rs:104](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/application/checkpoint.rs:104). Checkpoint lanes initialize only from subsequent `SpawnPlayer` commands. Restoring a snapshot containing existing players leaves them absent from `players`, so their checkpoints and progression events are silently skipped. **Fix:** snapshot/restore revision, cadence, dirty checkpoint and pending events alongside simulation state; test persistence after a mid-fight restore.

- **Overflowing respawn configuration is silently accepted** — [npc_data.rs:173](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/infrastructure/npc_data.rs:173). A delay/random value of `4294967296` passes validation and becomes zero; slot overrides silently fall back to defaults. **Fix:** reject failed conversions for templates and overrides, and prove the inclusive jitter range fits its sampling type. Add boundary tests.

- **UE combat acceptance can pass without combat** — [CombatEndToEndTest.cpp:124](/home/matt-woodruff/repos/nightfall-rpg/apps/client-unreal/Source/Nightfall/Tests/CombatEndToEndTest.cpp:124). Any Attack rejection passes; the attack-result timeout is ignored; an empty results collection passes. Missing NPCs substitute an unrelated rejection test. E5.3’s Done-when is therefore unenforced. **Fix:** require the fixture NPC, successful attack and matching result; assert repeated-click timing, ground-click cancellation and damage-number deduplication.

- **The performance Done-when is unenforced** — [combat_tests.rs:397](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/infrastructure/telemetry/combat_tests.rs:397). The sole combat timing test is ignored, uses `OpenGate`, has no checkpoint lane or sockets, and prints p99 without checking the budget. E6.2 is marked complete despite the recorded 55 ms result. **Fix:** run the specified release-build 200-session workload with a defined monster count and production persistence paths; enforce `<20 ms` or reopen the story.

**Low — P3**

- **Docs describe incompatible implementation states** — [phase-1-combat.md:25](/home/matt-woodruff/repos/nightfall-rpg/docs/decisions/phase-1-combat.md:25) claims deadline-driven animation, while the client drops attack-start/cancel events pending E5.4. [api-guidelines.md:172](/home/matt-woodruff/repos/nightfall-rpg/docs/engineering/api-guidelines.md:172) still calls Respawn a stub. **Fix:** describe current behavior and explicitly mark deferred animation work.

- **Unexplained production lint suppressions** — [stat_rules.rs:212](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/domain/zone/stat_rules.rs:212), [checkpoint.rs:331](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/application/checkpoint.rs:331). `missing_docs` and narrowing-cast allowances lack the reasons required by Rust guidelines §1.1. **Fix:** document public fields and justify necessary casts locally.

Validation: `cargo fmt --all -- --check` passed. Current runtime, database and UE suites were not run in the restricted environment.