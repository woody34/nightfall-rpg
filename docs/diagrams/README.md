# Architecture diagrams

Each diagram is a self-contained HTML file with inline SVG. The diagrams use the **minimal-light, default style** of Diagram Design.
Open the files locally in a browser. Repository viewers can show the HTML source and not the rendered diagram.

| Diagram | Type | Contents |
|---|---|---|
| [System architecture](system-architecture.html) | Architecture | The Unreal client, the four Rust layers, Postgres, NATS, Keycloak and LGTM, with the development ports. |
| [Login to zone admission](login-zone-sequence.html) | Sequence | The device flow, `IssuePlayTicket`, the WebSocket upgrade with the ticket in the header, and the acknowledged zone admission. |
| [Click to move](click-to-move-sequence.html) | Sequence | The click, the local preview, `MoveTo` in tiles, `Ack` / `IntentRejected`, the `EntityMove` of the own entity, and the pawn reconciliation. |
| [Deterministic tick and replay](deterministic-tick-replay.html) | Process | Draft → run → durable ack → sockets. Also the unsampled records, the shutdown watermark, the refusal of an incomplete epoch, and the output comparison. |
| [Replay tool](replay-tool.html) | Flowchart | The JetStream or `.nfr` source, the watermark check, the rebuild, the second run, the digest/byte comparison, and the exit codes. |
| [Session lifecycle](session-lifecycle.html) | State machine | The HTTP admission, the active session, the recoverable rate limit, the replacement, the idle timeout and the close codes. |
| [Persistence model](data-model.html) | ER / data model | All the columns of the seven tables that exist now, the keys, and the declared and the logical relationships. |
| [Module dependencies](module-dependencies.html) | Dependency graph | The composition, the shared inner layers and the telemetry exception from the interface layer to the infrastructure layer. |
| [Stat derivation](stat-derivation-data-flow.html) | Data flow | Pinned L2J HF source → decimal generator → TOML → `rules_data` → `StatRules` → `StatSheet` → combat and progression. |
| [Combat sequence](combat-sequence.html) | Sequence | Proxy raycast → SetTarget / Attack → chase → swing start → durable ack → impact draws → dispatch to the combat state and to HUD dedupe/numbers, with the loop for each impact. Then the death consequences, the kill XP or the death XP loss, `DEAD_ACTOR` and `Respawn`. |
| [Bot runner lifecycle](bot-runner-lifecycle.html) | State machine | `-BotScenario` → armed (run log, sentinel, allow-list, parse) → wait for the world → one step for each tick → JUnit report → exit 0 only when all steps passed and the log/ensure sentinel is clean. Without the flag, the runner does nothing. |
| [NPC state machine](npc-state-machine.html) | State machine | Idle / Active / Attack / ReturnHome / Dead, with the guards for think, aggro, clan call, leash and timeout. Corpse → hidden → respawn of a member of a spawn slot. |
| [Proxy animation states](proxy-animation-states.html) | State machine | Idle / Walk / Run / Attack / Flinch / Dying / Corpse, with the speed guards, the swing impact lead, the hit reaction, the death priority, the late AOI entry and the respawn. |
| [Simulation pipeline](simulation-pipeline.html) | Swimlane | The push-to-main/nightly lane and the PR-nonblocking lane, the self-hosted runner, the Compose stack, the dev-token API, N bot processes, the artifacts, the server replay-check, the coverage summary and the quarantine list. |
| [Class progression tree](phase-2-class-tree.html) | Tree / Hierarchy | All 89 Nightfall professions in five races, nine root branches and four tiers (the playable cap is 85 and the maximum tier is 2; tier 3 is deferred). |
| [Class transfer lifecycle](phase-2-class-transfer.html) | Process | The frozen idempotency check for each account, the live actor guards (Class Master at 126, 128), the durable replay log ack, the revision-fenced atomic Postgres checkpoint, and the public release. |

## Regenerate and validate

1. Read the [diagram-design skill](/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design/skills/diagram-design/SKILL.md).
   The owner selected **keep default**. Do not customize the style. Do not ask the style-guide question again.
2. Read again the code that each diagram links to below it. Use these as the authority: `apps/api/src`, the final Postgres
   migrations and SeaORM entities, `docker-compose.yml`, and the Unreal auth/net sources.
   The prose that exists now can be out of date. Make sure that the behavior is correct, and also the identifiers.
3. Start from the `assets/template.html` of the skill. Read the applicable `references/type-*.md`,
   `semantic-patterns.md`, the style guide and the output spec. Keep the static inline SVG and the embedded CSS.
   Only the Google Fonts stylesheet of the template is external. If the computer is offline, the fonts change to the local sans,
   monospace and serif fonts.
4. Keep the types above. No semantic pattern in the catalog agrees fully with these views:
   - The session diagram uses usual state-machine transitions.
   - The tick/replay diagram uses actor lanes and process steps in sequence.
   - The replay tool is a top-down flowchart with a merge dot.
   - The system diagram uses `doc-wide` (1280×720), and the dependencies diagram uses `doc-inline`
     (960×600). The sequence, state and ER diagrams use fitted canvases.
   - The process diagram uses the parametric canvas of the type reference, with five steps and four lanes (728×436).
   - The replay tool uses a fitted canvas of 720×700.
   - The stat derivation diagram uses the parametric canvas of the data-flow type, with six steps and four lanes (840×436).
   - The combat diagram is a fitted
     1280×820 sequence with five lifelines and one `LOOP` fragment.
   - The bot runner lifecycle is a fitted 1080×700
     state machine with ringed exit dots. The sentinel and the verdict are its two focal states.
5. Apply the taste gate of the skill. It includes complexity limits, ≤2 general focal elements, orthogonal connectors, masked labels,
   distinct ports, accessibility, readable type and local horizontal scrolling. (The process diagram
   uses its focal step/node/handoff set, which is specific to its type.) Keep the documented groups and omissions of each
   diagram explicit. Do not invent relationships that are not there.
6. Run these commands from the repository root:

   ```bash
   diagram_skill=/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design/skills/diagram-design
   for diagram in docs/diagrams/*.html; do
     python3 "$diagram_skill/scripts/self_check.py" "$diagram" || exit 1
   done
   ```

   If the repository checkout of the skill is available, also run its geometry checker:

   ```bash
   diagram_repo=/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design
   for diagram in docs/diagrams/*.html; do
     python3 "$diagram_repo/scripts/verify-geometry.py" "$diagram" || exit 1
   done
   ```

7. Examine each rendered diagram. Examine the scroll behavior on a narrow screen, and examine the printed output. Then update the
   links and the prose that has the same content in [architecture](../engineering/architecture.md),
   [API guidelines](../engineering/api-guidelines.md) and the
   [Unreal README](../../apps/client-unreal/README.md).

## Scope and verification notes

- The tick runs **before** `DurableTickGate::admit`. The ack controls the visibility and the next
  draft. Replay is the `nightfall-replay` tool, which uses library APIs. Replay is not an automatic recovery at restart.
- Each account has its own generations. The live registry replaces sockets by the player `EntityId`.
  The rate limit returns `RATE_LIMITED`. `4429` is a close code for a slow consumer.
- The ER view keeps all the columns that exist now. On some rows, it puts related columns together. The SQL
  types, checks and indexes stay in the migrations. Only two foreign keys are declared.
- The diagrams put the component families and the module families together. The happy-path sequence does not show the login retries and the character-selection RPCs.
  The dependency graph counts the visible edges. It does not show the direct composition imports of inner layers, the tests and the third-party crates.
- Stat derivation puts the five tables and the nine class files into one node. It does not show the NPC
  final values (`StatSheet::from_final`) and the replay/snapshot use of `config_hash` (E2.2).
- We examined the names and the values that the diagrams show in the code/configuration. The NATS command
  bus of the older docs is a future design. It is not the runtime path that the system uses now.
