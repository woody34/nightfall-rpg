# Architecture diagrams

Self-contained HTML with inline SVG, in Diagram Design's **minimal-light, default style**.
Open locally in a browser; repository viewers may show HTML source instead of rendering it.

| Diagram | Type | Contents |
|---|---|---|
| [System architecture](system-architecture.html) | Architecture | Unreal client, four Rust layers, Postgres, NATS, Keycloak and LGTM with development ports. |
| [Login to zone admission](login-zone-sequence.html) | Sequence | Device flow, `IssuePlayTicket`, ticket-in-header WebSocket upgrade and acknowledged zone admission. |
| [Click to move](click-to-move-sequence.html) | Sequence | Click, local preview, `MoveTo` in tiles, `Ack` / `IntentRejected`, own-entity `EntityMove` and pawn reconciliation. |
| [Deterministic tick and replay](deterministic-tick-replay.html) | Process | Draft → run → durable ack → sockets; unsampled records, shutdown watermark, incomplete-epoch refusal and output comparison. |
| [Replay tool](replay-tool.html) | Flowchart | JetStream or `.nfr` source, watermark check, rebuild, re-run, digest/byte compare, exit codes. |
| [Session lifecycle](session-lifecycle.html) | State machine | HTTP admission, active session, recoverable rate limiting, replacement, idle timeout and close codes. |
| [Persistence model](data-model.html) | ER / data model | All columns of the seven current tables, keys and declared versus logical relationships. |
| [Module dependencies](module-dependencies.html) | Dependency graph | Composition, shared inner layers and the interface-to-infrastructure telemetry exception. |
| [Stat derivation](stat-derivation-data-flow.html) | Data flow | Pinned L2J HF source → decimal generator → TOML → `rules_data` → `StatRules` → `StatSheet` → combat and progression. |
| [Combat sequence](combat-sequence.html) | Sequence | Proxy raycast → SetTarget / Attack → chase → swing start → durable ack → impact draws → dispatch to combat state & HUD dedupe/numbers, with the per-impact loop; then death consequences, kill XP or death XP loss, `DEAD_ACTOR` and `Respawn`. |
| [NPC state machine](npc-state-machine.html) | State machine | Idle / Active / Attack / ReturnHome / Dead with think, aggro, clan call, leash and timeout guards; corpse → hidden → respawn of a spawn-slot member. |
| [Proxy animation states](proxy-animation-states.html) | State machine | Idle / Walk / Run / Attack / Flinch / Dying / Corpse with speed guards, swing impact lead, hit reaction, death priority, late AOI entry and respawn. |

## Regenerate and validate

1. Read the [diagram-design skill](/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design/skills/diagram-design/SKILL.md).
   The owner has chosen **keep default**; do not customize or repeat the style-guide question.
2. Re-read the code linked beneath each diagram. Use `apps/api/src`, the final Postgres
   migrations and SeaORM entities, `docker-compose.yml`, and the Unreal auth/net sources as
   authority. Existing prose can be stale. Verify behavior as well as identifiers.
3. Start from the skill's `assets/template.html`; read the matching `references/type-*.md`,
   `semantic-patterns.md`, style guide and output spec. Keep static inline SVG and embedded CSS;
   only the template's Google Fonts stylesheet is external. Fonts fall back to local sans,
   monospace and serif when offline.
4. Keep the types above. No catalog semantic pattern exactly matches these views: session
   handling uses ordinary state-machine transitions; tick/replay uses actor lanes and ordered
   process steps; the replay tool is a top-down flowchart with a merge dot. The system uses
   `doc-wide` (1280×720), dependencies use `doc-inline`
   (960×600), and sequence/state/ER use fitted canvases. The process uses the type reference's
   parametric five-step, four-lane canvas (728×436); the replay tool a fitted 720×700; stat
   derivation the data-flow type's parametric six-step, four-lane canvas (840×436); combat a fitted
   1280×820 five-lifeline sequence with one `LOOP` fragment.
5. Apply the skill's taste gate: complexity limits, ≤2 general focal elements (the process
   uses its type-specific focal step/node/handoff set), orthogonal connectors, masked labels,
   distinct ports, accessibility, readable type and local horizontal scrolling. Keep each
   diagram's documented grouping/omissions explicit; do not invent missing relationships.
6. Run from the repository root:

   ```bash
   diagram_skill=/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design/skills/diagram-design
   for diagram in docs/diagrams/*.html; do
     python3 "$diagram_skill/scripts/self_check.py" "$diagram" || exit 1
   done
   ```

   When the skill's repository checkout is available, also run its geometry checker:

   ```bash
   diagram_repo=/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design
   for diagram in docs/diagrams/*.html; do
     python3 "$diagram_repo/scripts/verify-geometry.py" "$diagram" || exit 1
   done
   ```

7. Inspect every rendered diagram, check narrow-screen scrolling and print, then update the
   links and overlapping prose in [architecture](../engineering/architecture.md),
   [API guidelines](../engineering/api-guidelines.md) and the
   [Unreal README](../../apps/client-unreal/README.md).

## Scope and verification notes

- The tick runs **before** `DurableTickGate::admit`; the ack gates visibility and the next
  draft. Replay is the `nightfall-replay` tool over library APIs, not automatic restart
  recovery.
- Generations are per account; the live registry replaces sockets by player `EntityId`.
  Rate limiting returns `RATE_LIMITED`; `4429` is a slow-consumer close.
- The ER view retains every current column, grouping related columns on some rows. SQL
  types, checks and indexes remain in migrations. Only two foreign keys are declared.
- Component/module families are grouped; login retries and character-selection RPCs are
  omitted from the happy-path sequence. The dependency graph counts visible edges and
  omits direct composition imports of inner layers, tests and third-party crates.
- Stat derivation groups the five tables and nine class files into one node and omits NPC
  final values (`StatSheet::from_final`) and the replay/snapshot use of `config_hash` (E2.2).
- Names and values shown were verified in code/configuration. The older docs' NATS command
  bus is a future design, not the current runtime path.
