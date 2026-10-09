# Phase 1 Combat Decision Record

## Context

Phase 1 delivers the minimal playable Lineage 2 loop: targeting, auto-attacking a monster, damage, death, XP, leveling, and respawn in a flat test zone. Initial planning documents mixed Interlude, High Five (HF), and Ertheia mechanics with informal approximations. This record establishes authoritative High Five source provenance, deterministic fixed-point execution, persistence guarantees, and asset selections.

## Decisions

- **High Five source precedence:** Authoritative mechanics derive from pinned L2J High Five revisions `l2j-server-game@abfde0490ac52a2106e884b7242c0276c44cdf76` and `l2j-server-datapack@3ca488dd2bd0bfaca43e378886a3c2e37968153a`. Pinned XML/JSON tables (`statBonus.xml`, `expData.json`, `playerXpPercentLost.xml`) supersede legacy planning formulas and Ertheia-derived errata (E-1..E-11).
- **Physical damage coefficient:** Normal attack damage coefficient is **76** (`calcPhysDam`), resolving planning doc ambiguity with 77 (which applies only to critical additive damage and skill/blow calculations; E-8).
- **Attack interval:** Attack timing uses HF's **`500000 / pAtkSpd` ms** interval with impact at half-interval, superseding Interlude's 470000 ms (E-9).
- **Accuracy and evasion:** Accuracy uses **`sqrt(DEX) * 6 + L`** with HF per-level additions `+(L-69)` above 69 and `+(L-76)` above 77 (`FuncAtkAccuracy`; E-3), superseding stats §2.4's `* 5`. Evasion uses `sqrt(DEX) * 6 + L` (capped at 250).
- **Weapon stats replace bare hands:** Weapon stats are order 0 `<set>` funcs replacing bare-hand values (`P.Atk = weapon * STR * LM`, weapon speed 379, crit 8 for Squire's Sword; E-5), rather than adding to fists.
- **Spawn protection:** Town respawn grants **6000 ticks** (600 seconds) spawn protection (`PlayerSpawnProtection = 600` s; E-6), canceled early if the player initiates an attack.
- **Fixed-point arithmetic & E1.4 rounding:** Simulation uses **`Q = 1_000_000`** (`i64`) integer fixed point with checked `i128` intermediates; no runtime floating point. E1.4 rational oracle audit aligns rounding with L2J's `Math.round` sites: accuracy/evasion getters, attack speed, integer millisecond attack scheduling casts, and death loss rounding, eliminating up to 19‰ hit chance and 1 XP/delevel discrepancies.
- **NPC AI & respawn scheduler:** NPC AI runs in the deterministic actor as an L2J `L2AttackableAI` intention subset (`Idle`, `Active`, `Attack`, `ReturnHome`, `Dead`) with 10-tick think, 1-in-30 wander, social aggro clan calls, and 1200-tick leash timeout. The spawn-slot respawn scheduler is the single NPC death→corpse decay→respawn lifecycle path, ensuring stable slot identity and new life incarnations without double-spawning.
- **Checkpoints & revision fencing:** Progression checkpoints commit every 50 ticks and immediately on death, level-up, respawn, or disconnect in one atomic SeaORM transaction with the outbox. Stale repository revisions fence writes. The zone actor stalls and backpressures during save retries so database completion time never enters simulation.
- **Asset selection & licensing:** Player uses installed UE 5.8.3 Epic Manny template assets (zero download, UE EULA examples). Monster uses Animated Cockatrice (Marko J / Marko Jäntti; advertised idle, walk, attack, death clips). Single owner download session for monster FBX/textures; Fab licence labels to be confirmed by the owner at download.

## Consequences

- Full High Five combat formula fidelity with bit-exact determinism across platforms and headless replay.
- Pure integer zone simulation ensures byte-identical replay verification from snapshots and applied logs.
- Database outages stall the zone actor rather than allowing progression drift or race conditions.
- Animation playback is driven strictly by server combat cycle deadlines and life states; client never determines damage or hit success.

## Known limitations

- Debug-build tick p99 of 55 ms with 200 players and 200 keltirs needs a release-build measurement against the <20 ms budget.
- The return-home arrival-order bug found by a property test and fixed (path overshoot/arrival sequence).
- Fab licence labels unverified (public Fab pages exposed empty licence fields; owner must inspect at download).
- Scope excludes skills, items/inventory (beyond fixed starter weapon), equipment slots, and parties/shared XP.

## References

- Planning: [Phase 1 plan](../plans/phase-1-kill-a-monster.md), [Asset manifest](../plans/assets/phase-1-asset-manifest.md), [Stat formulas](../planning/01-stat-formulas.md), [Combat and skills](../planning/03-combat-and-skills.md), [World and content](../planning/06-world-and-content.md), [Planning README](../planning/README.md)
- Engineering: [Architecture](../engineering/architecture.md), [API guidelines](../engineering/api-guidelines.md)
- Data sources: [SOURCES.md](../../packages/data/SOURCES.md)
- Diagrams: [Diagrams README](../diagrams/README.md), [Combat sequence](../diagrams/combat-sequence.html), [NPC state machine](../diagrams/npc-state-machine.html), [Stat derivation data flow](../diagrams/stat-derivation-data-flow.html)
