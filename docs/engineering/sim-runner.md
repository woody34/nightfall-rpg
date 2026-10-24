# Simulation Runner Provisioning and Operations

This document gives the setup, provisioning, operation, and gating policies for the
dedicated self-hosted Linux simulation runner. This runner is for the headless client test
suite of Nightfall ([.github/workflows/sim.yml](../../.github/workflows/sim.yml)).

---

## 1. Owner Provisioning Checklist

Only the owner can provision the simulation runner. This is an **owner-only action** (Phase 1a plan Story 4.1).
The runner runs real headless Unreal Engine 5 client instances against backend stacks in containers
(Postgres, NATS, Keycloak, and the Rust game server API).

### Prerequisites Summary

| Item | Requirement | Purpose |
|---|---|---|
| **Host System** | A dedicated Linux x86_64 physical machine or a persistent VM, with space for the engine, the build outputs and the DDC | Engine compilation, DDC, container stacks, client processes that run at the same time |
| **Runner Labels** | `self-hosted`, `linux`, `unreal` | The target selector for [.github/workflows/sim.yml](../../.github/workflows/sim.yml) (`runs-on: [self-hosted, linux, unreal]`) |
| **Unreal Engine** | UE 5.8.3, installed legally under an authorized Epic Games account | The real headless client binary (`-game -nullrhi -nosound -unattended`) |
| **Clang Toolchain** | The Epic native toolchain `v26_clang-20.1.8-rockylinux8` | Unreal Build Tool (UBT) C++ compilation (`LINUX_MULTIARCH_ROOT`) |
| **Container Engine** | Docker Engine + Docker Compose v2 plugin | Ephemeral service stacks for each unit (`docker compose`) |
| **Language Toolchains** | The Rust toolchain that the repository pins (`cargo`, `rustc`; refer to `rust-toolchain.toml`), `moon` CLI, `python3`, `curl`, `git`, `timeout` | To build the API binaries, run tasks, and run the CI orchestration |
| **Cache Warming** | TurboLink third-party libs (~900 MB) extracted. DDC warmed one time. | Prevents repeated third-party downloads and shader compilation |
| **Environment Vars** | `UE_ROOT`, `LINUX_MULTIARCH_ROOT` set in the runner profile and in the GitHub repo variables | To find the location of the engine and the toolchain |
| **Credentials** | The GitHub repository admin credentials of the owner | To make the runner registration token (automation cannot make this token, and you must not make up a token) |

The first TurboLink setup also must have `unzip`. To build the cached proto tools again, you must have
the Python `venv` support and a C++ compiler. The proto installer provisions CMake
and Ninja in its own virtual environment, and uses the .NET SDK that comes with Unreal.
The optional Keycloak token path in `Scripts/run-tests.sh` makes sure that `jq` and
`openssl` are there before it uses them. The dev-token path does not use these tools.

The runner does not need the host `psql`, `rg` and `fd`. The isolated Phase 2/3
fixture packs install a controlled `psql` shim that runs in the Postgres service that they control.
The casting soak seed also uses `docker compose exec -T`
with the explicit project of the run, the resolved Compose file and split libpq variables.
Before the seed runs, the script compares the host connection port with that service.

---

### Step-by-Step Provisioning Checklist

#### Step 1: Dedicated Host and OS Isolation
- [ ] Get a dedicated Linux box or VM. Do not put the runner on the same host as shared build agents.
  The simulation suite binds host network ports (`3000` HTTP API, `4222` NATS, `5432` Postgres,
  `8080` Keycloak, `50051` gRPC).
- [ ] Configure concurrency. The workflow-level concurrency group `nightfall-unreal-stack` makes sure that
  only one simulation run operates on the runner at a time.

#### Step 2: Legally Install Unreal Engine 5.8.3
- [ ] Get an Epic Games account and accept the Unreal Engine EULA. Unreal Engine must have these two items.
  You cannot distribute the engine binaries through the repository.
- [ ] Download the official Linux prebuilt binaries:
  1. Sign in at <https://www.unrealengine.com/linux>.
  2. Download the Linux UE 5.8.3 distribution that is available to your account.
  3. Extract it to a stable directory, for example `/home/ubuntu/Linux_Unreal_Engine_5.8.3`.
- [ ] Make sure that the file permissions are correct:
  ```bash
  chmod +x "$UE_ROOT/Engine/Build/BatchFiles/Linux/Build.sh"
  chmod +x "$UE_ROOT/Engine/Binaries/Linux/UnrealEditor"
  ```
- [ ] Validate the engine version in `$UE_ROOT/Engine/Build/Build.version`:
  `MajorVersion=5`, `MinorVersion=8`, `PatchVersion=3`.

#### Step 3: Install Native Clang Toolchain
- [ ] Install the clang toolchain from Epic. The prebuilt Linux engine from Epic must have it.
  Without it, Unreal Build Tool (UBT) fails SDK validation with `Platform Linux is not a valid platform to build`.
- [ ] Get `v26_clang-20.1.8-rockylinux8` from the engine installation instructions.
- [ ] Unpack the toolchain into `$HOME/UnrealToolchains`.
- [ ] Set `LINUX_MULTIARCH_ROOT`:
  ```bash
  export LINUX_MULTIARCH_ROOT="$HOME/UnrealToolchains/v26_clang-20.1.8-rockylinux8"
  ```

#### Step 4: Install Docker and Docker Compose
- [ ] Install Docker Engine and the `docker compose` plugin (v2):
  ```bash
  docker compose version
  docker info --format '{{.ServerVersion}}'
  ```
- [ ] Let the runner user run Docker without root:
  ```bash
  sudo usermod -aG docker "$USER"
  # Log out and log back in to apply group changes
  ```

#### Step 5: Install Rust and moon Toolchains
- [ ] Install Rust stable through `rustup` (`cargo`, `rustc`):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  ```
- [ ] Install the `moon` CLI:
  ```bash
  curl -fsSL https://moonrepo.dev/install/moon.sh | bash
  ```
- [ ] Make sure that the necessary CLI utilities are there:
  ```bash
  sudo apt-get update && sudo apt-get install -y curl python3 git coreutils
  ```

#### Step 6: TurboLink Setup and DDC Warming
- [ ] Unpack the TurboLink gRPC/Protobuf prebuilt libraries (~900 MB) into `Plugins/TurboLink/Source/ThirdParty/`:
  ```bash
  moon run client-unreal:setup
  ```
- [ ] Warm the Derived Data Cache (DDC) and resolve the ICU asset dependencies.
  The Linux UnrealEditor binary finds `Engine/Content` relative to its working directory.
  Run it one time from its binary directory:
  ```bash
  "$UE_ROOT/Engine/Binaries/Linux/UnrealEditor" "$PWD/apps/client-unreal/Nightfall.uproject" \
    -run=DerivedDataCache -fill -unattended -nosound -nop4
  ```

#### Step 7: Environment and GitHub Repository Variables
- [ ] Keep the environment variables in the runner user profile (`~/.bashrc`):
  ```bash
  export UE_ROOT="/home/ubuntu/Linux_Unreal_Engine_5.8.3"
  export LINUX_MULTIARCH_ROOT="/home/ubuntu/UnrealToolchains/v26_clang-20.1.8-rockylinux8"
  ```
- [ ] Set the GitHub repository variables under **Settings > Secrets and variables > Actions > Variables**:
  - `UE_ROOT`: `/home/ubuntu/Linux_Unreal_Engine_5.8.3`
  - `LINUX_MULTIARCH_ROOT`: `/home/ubuntu/UnrealToolchains/v26_clang-20.1.8-rockylinux8`
  - `SIM_PR_REQUIRED`: at first, not set or `false` (do not set `true` until the 2-week gate passes)

#### Step 8: Register GitHub Self-Hosted Runner
> [!IMPORTANT]
> Runner registration must have the **repository owner admin credentials**.
> Do not make up, mock, or assume a registration success in documentation or in automated scripts.
> The owner must make a new registration token from GitHub:
> 1. Go to the repository **Settings > Actions > Runners > New self-hosted runner**.
> 2. Select **OS: Linux**, Architecture: **x64**.
> 3. Run the `./config.sh` command with these labels: `--labels linux,unreal`.
> 4. Install and start the service: `sudo ./svc.sh install && sudo ./svc.sh start`.

#### Step 9: Warmed Acceptance Commands
To make sure that the runner is ready for operation, run this verification sequence manually on the runner:
```bash
# 1. Non-mutating preflight check
bash infra/scripts/check-sim-runner.sh

# 2. Build game and editor targets
moon run client-unreal:build
moon run client-unreal:build-editor

# 3. Build API tools and replay CLI
cargo build -p nightfall-api --bins

# 4. Run headless Unreal automation tests
moon run client-unreal:test-editor -- --require-live-api

# 5. Execute single scenario acceptance test against fresh Compose stack
moon run client-unreal:sim -- Scenarios/0b-login-enter-world.nfs --api start
```

---

## 2. CI Architecture: Fresh Per-Unit Compose Stacks

In CI ([.github/workflows/sim.yml](../../.github/workflows/sim.yml)),
[apps/client-unreal/Scripts/run-sim-ci.py](../../apps/client-unreal/Scripts/run-sim-ci.py) controls the test suite.

Recursive discovery finds all 121 scenario files / 106 single/group units.
This includes all 49 Phase4 files / 47 units.
The limit of each client is the largest rounded-up `nf.Within` of its roles, plus 30 seconds of launch/exit grace.
There is no 600-second minimum. An explicit `--timeout` can make the limit larger.

`run-sim-ci.py::unit_allowances` records the components that follow in the summary of each unit.
These values are enforced failure ceilings. They are not measured execution times.

| Component | Derivation |
|---|---|
| Fixture | Isolated: seeder help 10s + three logged provision commands (up, migration, seed). The limit of each command is 180s. Non-isolated fresh stack: Compose up, with a limit of 180s. The two types also add a wall-clock API startup limit of 180s, plus the last curl/sleep (2s + 1s). |
| Replay and evidence | Policy: one client window for each role, for all the export/check/trace/coverage/formula work. This includes the shared group work. The process-group watchdog of the wrapper enforces the total window. Phase3 has no replay timing measurements. |
| Cleanup | API stop grace 15s. Isolated fixtures add two commands (logs/down), with a limit of 60s each, plus one EXIT retry of these commands. |
| Fallback cleanup | Isolated fixtures only: one more logs/down attempt (2 × 60s), when a forced wrapper termination interrupts its EXIT trap. |
| Diagnostic retry | Solo units only, with `--video`: capture 110s + kill grace 10s + encode 110s + kill grace 10s. Multi-role units never retry. |
| Local work and termination | An explicit 30s policy grace for parser/artifact/JUnit/scheduling work. A 10s process-group termination grace after the watchdog deadline. |
| Outer Compose | Down 120s + log collection 60s, only for the 21 non-isolated fresh-stack units. All 85 isolated units (this includes each Phase4 unit) skip these dispatcher operations. |

The workflow builds one time. It gives the API, migration and replay binaries and the
trace/coverage commands to each wrapper. Thus, these allowances do not include
compilation for each unit. The sums of the components, with each solo video retry, are:

- clients 24,773s; fixtures 69,928s; replay/evidence 30,449s; cleanup 21,990s;
- retry 21,840s; local grace 3,180s; outer Compose 3,780s; fallback cleanup 10,200s;
- termination 1,060s.

Total: **187,200s (52 hours)**. The total for headless runs only is 165,360s.
The deadline of the self-hosted job is 3,125 minutes (187,500s). This is 5m00s more than the
failure ceilings. This margin does not guarantee a measured build/upload time.
A run that gets to the job deadline fails.
The larger failure ceiling follows all applicable provision/cleanup/retry limits. It does not use an assumed flat margin for orchestration.
The regression checks the derivations and the complete discovery. No scenarios are excluded.
Before you make the infrastructure limits smaller, get actual timing evidence.

### Ephemeral Stack Isolation

1. **Dedicated Project Namespace**:
   The workflow sets `COMPOSE_PROJECT_NAME=nightfall-sim-${{ github.run_id }}-${{ github.run_attempt }}`.
   `run-sim-ci.py` verifies this namespace format before it lets `--fresh-stack` run. This prevents
   an accidental removal of volumes in environments that are not CI.

2. **Per-Unit Teardown and Volume Wipe**:
   Each scenario unit (one scenario or a paired multi-client role) starts with the removal of the stack:
   ```bash
   docker compose down --volumes --remove-orphans
   ```
   This makes sure that database rows, Keycloak realm accounts, NATS JetStream state, and zone
   instances from one scenario cannot go into the next scenario.

3. **Fresh Service Startup**:
   For each unit, the services start again from the beginning (`docker compose up -d --wait`). The Nightfall API
   starts with `AUTH_DEV_TOKENS=1`.

4. **Telemetry and Logs**:
   Each unit keeps:
   - `orchestration.log`: the full stdout/stderr of the test launcher and the client processes.
   - `compose.log`: the output of the service containers (`docker compose logs --no-color`).
   - JUnit XML, UE logs (`Saved/Sim/<scenario>.log`), `.nfr` replay recordings, and `.trace.html` traces.

---

## 3. The 4 Paired Multi-Client Roles

Scenarios with interaction between clients cannot run in isolation. They run as coordinated
pairs through [apps/client-unreal/Scripts/run-sim-multi.sh](../../apps/client-unreal/Scripts/run-sim-multi.sh),
and they share a common `-SimGroup` token. Coordination occurs only through the server state (for example,
observation of entity proxies), never through local IPC.

In [run-sim-ci.py](../../apps/client-unreal/Scripts/run-sim-ci.py), paired roles (`*-a.nfs` and `*-b.nfs`)
make an atomic unit. If an `-a` role exists without a matching `-b` role (or the opposite), the runner
fails immediately with a configuration error (`orphan scenario role`).

| Paired Unit | Role Files | Systems Tested | Failure Signature |
|---|---|---|---|
| **Two Clients in View** | `0b-two-clients-a.nfs`<br>`0b-two-clients-b.nfs` | • A walks to (12, 9) and logs out.<br>• B waits for `proxies >= 1`, and observes the proxy of A within 2 tiles after the interpolation delay.<br>• B makes sure that A despawns with zero leaked proxy actors (`proxy_actors_in_sync`). | A leaked proxy actor on despawn, or a proxy position desync > 2 tiles. |
| **Login Replacement** | `0b-login-replaces-a.nfs`<br>`0b-login-replaces-b.nfs` | • The two processes authenticate under the same account and character through `nf.LoginGroup`.<br>• The login of B causes the server to disconnect A with the WebSocket close code `4409` (`SESSION_REPLACED`).<br>• A asserts a clean closure (`close_code == 4409`) and stops without a reconnection. B stays `in_world`. | A tries to reconnect after 4409, or B does not take control of the session. |
| **Late Entry and Stale Spawn** | `1-late-entry-a.nfs`<br>`1-late-entry-b.nfs` | • A attacks and damages a keltir.<br>• B enters the world during the fight, and asserts that the keltir spawn has the wounded HP and incarnation.<br>• The next hit continues from the current HP.<br>• A drops the socket (`nf.DropSocket`) and reconnects with a new ticket. A asserts that XP is unknown until the next gain, and that an old spawn does not reset the NPC HP. | A regression in `IsStaleSpawn`, which resets the wounded NPC HP to full, or an old XP value on the screen. |
| **Social Aggro** | `1-social-aggro-a.nfs`<br>`1-social-aggro-b.nfs` | • Two keltir instances.<br>• A attacks keltir 1. Keltir 2 calls clan assist and attacks A.<br>• B stands in the aggro range of keltir 2, but does not attack.<br>• B asserts that it observes the fight (`npcs_fighting >= 1`), but that it is never attacked (`attacked_by == 0`). | The help call incorrectly marks the neutral bystander B, or the social hate does not spread. |

---

## 4. Quarantine Policy and JSON Schema

The quarantine system isolates known flaky tests or bugs that are not repaired, so that they do not block CI.
It also makes sure that the failures stay visible and have a time limit.

### Quarantine File Location
[apps/client-unreal/Scenarios/quarantine.json](../../apps/client-unreal/Scenarios/quarantine.json)

### JSON Schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "required": ["schema_version", "scenarios"],
  "properties": {
    "schema_version": {
      "type": "integer",
      "const": 1
    },
    "scenarios": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["scenario", "owner", "reason", "issue", "expires"],
        "properties": {
          "scenario": {
            "type": "string",
            "description": "Base filename of the scenario without .nfs extension (e.g. 1-chase-leash)"
          },
          "owner": {
            "type": "string",
            "description": "GitHub username or accountable owner responsible for the fix"
          },
          "reason": {
            "type": "string",
            "description": "Concrete technical explanation of why the test flakes or fails"
          },
          "issue": {
            "type": "string",
            "description": "URL or issue number tracking the resolution"
          },
          "expires": {
            "type": "string",
            "format": "date",
            "description": "Expiry date in ISO 8601 format (YYYY-MM-DD). Past dates cause CI build failure."
          }
        }
      }
    }
  }
}
```

### Enforcement Rules

1. **Pre-Run Validation**:
   `run-sim-ci.py` parses `quarantine.json` before it starts a client. It refuses:
   - A missing `schema_version: 1` or a missing `scenarios` list.
   - Unknown scenario names or duplicate entries.
   - An empty or whitespace-only `owner`, `reason`, `issue`, or `expires`.
   - **Expired Quarantines**: If `date.fromisoformat(item["expires"]) < date.today()`,
     `run-sim-ci.py` raises `ValueError: expired quarantine <scenario>: <date>`.
     An expired quarantine **immediately stops CI with a failure**.

2. **Execution Semantics**:
   - Quarantined scenarios **still run**. CI never skips them at the start.
   - CI makes and keeps the raw artifacts of the failure (JUnit XML, UE logs, recordings, traces).
   - If a quarantined scenario fails, CI records its unit status as `QUARANTINED` in `summary.json`.
     CI emits it as `<skipped>` in the aggregate `suite.xml`.

3. **Infrastructure Failures Cannot Be Quarantined**:
   Quarantine applies only to scenario assertion failures. A quarantined scenario **still fails CI** if:
   - The orchestration crashes (exit code not `0` or `1`).
   - The session recording is missing (`no session recording`).
   - The replay check fails (`moon run api:replay-check` / `replay check failed` / `FAIL replay`).
   - The trace generation fails (`trace generation failed`).
   - A nonzero exit code occurs, and there is no explanation for it.

---

## 5. Promotion Gate: PR Gating and Opt-In Policy

Simulation tests run real game processes on a persistent self-hosted runner. The pipeline must not block
development before the runner is proved stable. Thus the pipeline follows a gating lifecycle with two stages.

### Stage 1: Pre-Gate Opt-In (Current State)
- **Workflow Triggers**:
  - Pushes to `main`.
  - The scheduled run each night (`15 2 * * *`).
  - Manual `workflow_dispatch`.
- **Soft Failure**:
  The job runs with `continue-on-error: ${{ vars.SIM_PR_REQUIRED != 'true' }}`. Failures show in the
  GitHub Actions summaries and artifacts, but they do not block branch merges.
- **PR Opt-In through the `sim` Label**:
  - By default, pull requests do **not** run simulation CI.
  - Trusted developers can opt in a PR. To do this, they add the **`sim`** label to the PR.
  - **Fork PR Security**: PRs from forks **never run** on the self-hosted runner:
    ```yaml
    if: >-
      github.event_name != 'pull_request' ||
      (github.event.pull_request.head.repo.full_name == github.repository &&
       (contains(github.event.pull_request.labels.*.name, 'sim') || vars.SIM_PR_REQUIRED == 'true'))
    ```

### Stage 2: Flake Gate and Branch Protection Enforcement
When the runner shows that it is stable, the owner enforces the PR simulation gate:

1. **Two-Week Stability Requirement**:
   The test suite must complete **two consecutive calendar weeks with zero unquarantined failures**
   on `main` pushes and on the scheduled runs each night.
2. **Owner Actions**:
   - Set the repository variable `SIM_PR_REQUIRED=true` under **Settings > Secrets and variables > Actions > Variables**.
   - In the GitHub **Branch Protection Rules** for `main`:
     Add the status check **`Simulation / sim`** to the list of required status checks for PRs that change:
     - `apps/client-unreal/**`
     - `apps/api/**`
     - `packages/**`
     - `.github/workflows/sim.yml`

---

## 6. Optional Video Diagnostics (`--video`)

When a single-client scenario fails during a run at night or a manual dispatch, the runner can capture
a replay that it renders off-screen. The runner makes an MP4 diagnostic video from this replay.

### Runner Prerequisites

This command verifies these prerequisites:
```bash
bash infra/scripts/check-sim-runner.sh --video
```

- `xvfb-run` (X Virtual Framebuffer).
- `ffmpeg` with `libx264` support.
- `xauth`.
- **Mesa lavapipe**: a software Vulkan ICD that uses the CPU (`/usr/share/vulkan/icd.d/lvp_icd.json` or the path in `SIM_VIDEO_ICD`).

### Execution Rules

1. **Failure-Only Trigger**:
   The video render runs **only when a scenario fails**. It never runs for scenarios that pass.
2. **Single-Client Scenarios Only**:
   The rendered retry operates only for single-client scenarios (`run-sim.sh`). Multi-client paired
   scenarios (`run-sim-multi.sh`) do not run video retries.
3. **Execution Context**:
   [apps/client-unreal/Scripts/sim-video.sh](../../apps/client-unreal/Scripts/sim-video.sh) starts the retry. This script:
   - Removes `-nullrhi`.
   - Adds `-vulkan -sm5 -AllowSoftwareRendering -RenderOffscreen -DumpMovie -ForceRes -ResX=960 -ResY=540 -FPS=15 -NoVSync`.
   - Uses SM5 and lets CPU devices operate, because the default UE profile does not include lavapipe. Lumen, Nanite and virtual shadows are off only in this diagnostic retry.
   - Runs under `xvfb-run -a -s '-screen 0 960x540x24'`.
   - Writes frame dumps to `$dest/frames/`, puts them together with a concat manifest, and encodes
     them to H.264 MP4 (`960x540` at 15 fps).
4. **Isolated Retry Reports**:
   The video retry writes separate outputs:
   - `retry-exit-code.txt`
   - `stdout.log`
   - `ffmpeg.log`
   - A standalone retry JUnit XML.
5. **No Verdict Overrides**:
   The exit code and the report of the video retry **never replace or change the original headless test verdict**.
   Only the headless run sets the pass/fail status of the test.
6. **No Claim Playable Until Tested**:
   Verify the diagnostic videos on the specific runner host with lavapipe and ffmpeg installed.
   Until then, do not assume or claim that the videos play or operate.

## Scenario fixture selection

A scenario can request `# fixture: phase1a-social-aggro`. The two members of a pair must agree.
The CI orchestrator validates the supported fixture name. For usual cases, it clears the inherited `ZONE_SIM_FIXTURE`.
It sets the fixture only for this new API process. The fixture selection must have
`--fresh-stack`. An attached API that runs cannot change the fixture data.

## Headless traceability scope

Each completed story that has the tag `[client-visible]` must have a valid `# covers:` reference.
A reference means that the scenario asserts the observable part of the story. For the remaining part, the server
U/A tests, the renderer automation and the cooked-asset checks are still necessary. We select the tags from the
story behavior before the coverage exists. Cases without coverage fail and stay visible.
