# Simulation Runner Provisioning and Operations

This document specifies the setup, provisioning, operation, and gating policies for the
dedicated self-hosted Linux simulation runner supporting Nightfall's headless client testing
suite ([.github/workflows/sim.yml](../../.github/workflows/sim.yml)).

---

## 1. Owner Provisioning Checklist

Provisioning the simulation runner is an **owner-only action** (Phase 1a plan Story 4.1).
The runner executes real headless Unreal Engine 5 client instances against containerized
backend stacks (Postgres, NATS, Keycloak, and the Rust game server API).

### Prerequisites Summary

| Item | Requirement | Purpose |
|---|---|---|
| **Host System** | Dedicated Linux x86_64 physical machine or persistent VM with space for the engine, build outputs and DDC | Engine compilation, DDC, container stacks, concurrent client processes |
| **Runner Labels** | `self-hosted`, `linux`, `unreal` | Target selector for [.github/workflows/sim.yml](../../.github/workflows/sim.yml) (`runs-on: [self-hosted, linux, unreal]`) |
| **Unreal Engine** | UE 5.8.3 legally installed under an authorized Epic Games account | Real client headless binary (`-game -nullrhi -nosound -unattended`) |
| **Clang Toolchain** | Epic native toolchain `v26_clang-20.1.8-rockylinux8` | Unreal Build Tool (UBT) C++ compilation (`LINUX_MULTIARCH_ROOT`) |
| **Container Engine** | Docker Engine + Docker Compose v2 plugin | Per-unit ephemeral service stacks (`docker compose`) |
| **Language Toolchains** | Repository-pinned Rust toolchain (`cargo`, `rustc`; see `rust-toolchain.toml`), `moon` CLI, `python3`, `curl`, `git`, `timeout` | Building API binaries, running tasks, running CI orchestration |
| **Cache Warming** | TurboLink third-party libs (~900 MB) extracted, DDC warmed once | Avoids repeated third-party downloads and shader compilation |
| **Environment Vars** | `UE_ROOT`, `LINUX_MULTIARCH_ROOT` set in runner profile and GitHub repo variables | Engine and toolchain location resolution |
| **Credentials** | Owner GitHub repository admin credentials | Generating the runner registration token (cannot be automated/fabricated) |

---

### Step-by-Step Provisioning Checklist

#### Step 1: Dedicated Host and OS Isolation
- [ ] Allocate a dedicated Linux box or VM. Do not co-locate with shared build agents:
  the simulation suite binds host network ports (`3000` HTTP API, `4222` NATS, `5432` Postgres,
  `8080` Keycloak, `50051` gRPC).
- [ ] Configure concurrency: workflow-level concurrency group `nightfall-unreal-stack` ensures
  only one simulation run executes on the runner at a time.

#### Step 2: Legally Install Unreal Engine 5.8.3
- [ ] Unreal Engine requires an Epic Games account and acceptance of the Unreal Engine EULA.
  Engine binaries cannot be redistributed through the repository.
- [ ] Download official Linux prebuilt binaries:
  1. Sign in at <https://www.unrealengine.com/linux>.
  2. Download the Linux UE 5.8.3 distribution available to your account.
  3. Extract to a stable directory, e.g. `/home/ubuntu/Linux_Unreal_Engine_5.8.3`.
- [ ] Confirm file permissions:
  ```bash
  chmod +x "$UE_ROOT/Engine/Build/BatchFiles/Linux/Build.sh"
  chmod +x "$UE_ROOT/Engine/Binaries/Linux/UnrealEditor"
  ```
- [ ] Validate engine version in `$UE_ROOT/Engine/Build/Build.version`:
  `MajorVersion=5`, `MinorVersion=8`, `PatchVersion=3`.

#### Step 3: Install Native Clang Toolchain
- [ ] Epic's prebuilt Linux engine requires Epic's bundled clang toolchain. Without it,
  Unreal Build Tool (UBT) fails SDK validation with `Platform Linux is not a valid platform to build`.
- [ ] Obtain `v26_clang-20.1.8-rockylinux8` from the engine installation instructions; unpack it into `$HOME/UnrealToolchains`.
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
- [ ] Grant non-root Docker execution to the runner user:
  ```bash
  sudo usermod -aG docker "$USER"
  # Log out and log back in to apply group changes
  ```

#### Step 5: Install Rust and moon Toolchains
- [ ] Install Rust stable via `rustup` (`cargo`, `rustc`):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  ```
- [ ] Install `moon` CLI:
  ```bash
  curl -fsSL https://moonrepo.dev/install/moon.sh | bash
  ```
- [ ] Ensure required CLI utilities are present:
  ```bash
  sudo apt-get update && sudo apt-get install -y curl python3 git coreutils
  ```

#### Step 6: TurboLink Setup and DDC Warming
- [ ] Unpack TurboLink gRPC/Protobuf prebuilt libraries (~900 MB) into `Plugins/TurboLink/Source/ThirdParty/`:
  ```bash
  moon run client-unreal:setup
  ```
- [ ] Warm the Derived Data Cache (DDC) and resolve ICU asset dependencies:
  The Linux UnrealEditor binary resolves `Engine/Content` relative to its working directory.
  Run it once from its binary directory:
  ```bash
  "$UE_ROOT/Engine/Binaries/Linux/UnrealEditor" "$PWD/apps/client-unreal/Nightfall.uproject" \
    -run=DerivedDataCache -fill -unattended -nosound -nop4
  ```

#### Step 7: Environment and GitHub Repository Variables
- [ ] Persist environment variables in the runner user profile (`~/.bashrc`):
  ```bash
  export UE_ROOT="/home/ubuntu/Linux_Unreal_Engine_5.8.3"
  export LINUX_MULTIARCH_ROOT="/home/ubuntu/UnrealToolchains/v26_clang-20.1.8-rockylinux8"
  ```
- [ ] Set GitHub repository variables under **Settings > Secrets and variables > Actions > Variables**:
  - `UE_ROOT`: `/home/ubuntu/Linux_Unreal_Engine_5.8.3`
  - `LINUX_MULTIARCH_ROOT`: `/home/ubuntu/UnrealToolchains/v26_clang-20.1.8-rockylinux8`
  - `SIM_PR_REQUIRED`: initially unset or `false` (do not set `true` until the 2-week gate passes)

#### Step 8: Register GitHub Self-Hosted Runner
> [!IMPORTANT]
> Runner registration requires **repository owner admin credentials**.
> Do not invent, mock, or assume registration success in documentation or automated scripts.
> The owner must generate a fresh registration token from GitHub:
> 1. Go to repository **Settings > Actions > Runners > New self-hosted runner**.
> 2. Select **OS: Linux**, Architecture: **x64**.
> 3. Execute the `./config.sh` command with labels: `--labels linux,unreal`.
> 4. Install and start the service: `sudo ./svc.sh install && sudo ./svc.sh start`.

#### Step 9: Warmed Acceptance Commands
Run the following verification sequence directly on the runner by hand to confirm operational readiness:
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

In CI ([.github/workflows/sim.yml](../../.github/workflows/sim.yml)), the test suite is driven by
[apps/client-unreal/Scripts/run-sim-ci.py](../../apps/client-unreal/Scripts/run-sim-ci.py).

### Ephemeral Stack Isolation

1. **Dedicated Project Namespace**:
   The workflow sets `COMPOSE_PROJECT_NAME=nightfall-sim-${{ github.run_id }}-${{ github.run_attempt }}`.
   `run-sim-ci.py` verifies this namespace format before allowing `--fresh-stack` to run, preventing
   accidental volume deletion on non-CI environments.

2. **Per-Unit Teardown and Volume Wipe**:
   Every scenario unit (single scenario or paired multi-client role) begins by wiping the stack:
   ```bash
   docker compose down --volumes --remove-orphans
   ```
   This guarantees that database rows, Keycloak realm accounts, NATS JetStream state, and zone
   instances from one scenario cannot bleed into the next.

3. **Fresh Service Startup**:
   For each unit, services are started afresh (`docker compose up -d --wait`) and the Nightfall API
   is launched with `AUTH_DEV_TOKENS=1`.

4. **Telemetry and Logs**:
   Each unit preserves:
   - `orchestration.log`: Full stdout/stderr of the test launcher and client processes.
   - `compose.log`: Service container output (`docker compose logs --no-color`).
   - JUnit XML, UE logs (`Saved/Sim/<scenario>.log`), `.nfr` replay recordings, and `.trace.html` traces.

---

## 3. The 4 Paired Multi-Client Roles

Scenarios involving cross-client interaction cannot run in isolation; they execute as coordinated
pairs via [apps/client-unreal/Scripts/run-sim-multi.sh](../../apps/client-unreal/Scripts/run-sim-multi.sh),
sharing a common `-SimGroup` token. Coordination occurs strictly through the server state (e.g.
observing entity proxies), never via local IPC.

In [run-sim-ci.py](../../apps/client-unreal/Scripts/run-sim-ci.py), paired roles (`*-a.nfs` and `*-b.nfs`)
form an atomic unit. If an `-a` role exists without a matching `-b` role (or vice versa), the runner
fails immediately with a configuration error (`orphan scenario role`).

| Paired Unit | Role Files | Systems Tested | Failure Signature |
|---|---|---|---|
| **Two Clients in View** | `0b-two-clients-a.nfs`<br>`0b-two-clients-b.nfs` | • A walks to (12, 9) and logs out.<br>• B waits for `proxies >= 1`, observes A's proxy within 2 tiles after interpolation delay.<br>• B confirms A's despawn with zero leaked proxy actors (`proxy_actors_in_sync`). | Leaked proxy actor on despawn, proxy position desync > 2 tiles. |
| **Login Replacement** | `0b-login-replaces-a.nfs`<br>`0b-login-replaces-b.nfs` | • Both processes authenticate under the same account and character via `nf.LoginGroup`.<br>• B's login triggers server disconnect of A with WebSocket close code `4409` (`SESSION_REPLACED`).<br>• A asserts clean closure (`close_code == 4409`) and halts without reconnecting; B stays `in_world`. | A attempts reconnection after 4409, or B fails to take over session. |
| **Late Entry and Stale Spawn** | `1-late-entry-a.nfs`<br>`1-late-entry-b.nfs` | • A engages and damages a keltir.<br>• B enters the world mid-fight: asserts the keltir spawn carries wounded HP and incarnation.<br>• Next hit continues from current HP.<br>• A drops socket (`nf.DropSocket`), reconnects with a fresh ticket; asserts XP unknown until the next gain and NPC HP not reset by stale spawn. | Regression in `IsStaleSpawn`, resetting wounded NPC HP to full, or stale XP display. |
| **Social Aggro** | `1-social-aggro-a.nfs`<br>`1-social-aggro-b.nfs` | • Two keltir instances.<br>• A attacks keltir 1; keltir 2 calls clan assist and engages A.<br>• B stands in aggro range of keltir 2 but does not attack.<br>• B asserts it observes the fight (`npcs_fighting >= 1`) but is never attacked (`attacked_by == 0`). | Help call incorrectly flags neutral bystander B, or social hate fails to spread. |

---

## 4. Quarantine Policy and JSON Schema

The quarantine system isolates known flaky tests or in-progress bugs from blocking CI while
ensuring failures remain visible and time-bounded.

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
   `run-sim-ci.py` parses `quarantine.json` before launching any client. It rejects:
   - Missing `schema_version: 1` or missing `scenarios` list.
   - Unknown scenario names or duplicate entries.
   - Empty or whitespace-only `owner`, `reason`, `issue`, or `expires`.
   - **Expired Quarantines**: If `date.fromisoformat(item["expires"]) < date.today()`,
     `run-sim-ci.py` raises `ValueError: expired quarantine <scenario>: <date>`.
     An expired quarantine **immediately halts CI with failure**.

2. **Execution Semantics**:
   - Quarantined scenarios **still run**. They are never skipped up-front.
   - Raw failing artifacts (JUnit XML, UE logs, recordings, traces) are generated and preserved.
   - If a quarantined scenario fails, its unit status is recorded as `QUARANTINED` in `summary.json`
     and emitted as `<skipped>` in aggregate `suite.xml`.

3. **Infrastructure Failures Cannot Be Quarantined**:
   Quarantine exempts only scenario assertion failures. A quarantined scenario **still fails CI** if:
   - Orchestration crashes (exit code not in `0` or `1`).
   - Session recording is missing (`no session recording`).
   - Replay check fails (`moon run api:replay-check` / `replay check failed` / `FAIL replay`).
   - Trace generation fails (`trace generation failed`).
   - Unexplained nonzero exit code occurs.

---

## 5. Promotion Gate: PR Gating and Opt-In Policy

Simulation testing runs real game processes on a persistent self-hosted runner. To avoid blocking
development before stability is proven, the pipeline follows a two-stage gating lifecycle.

### Stage 1: Pre-Gate Opt-In (Current State)
- **Workflow Triggers**:
  - Pushes to `main`.
  - Nightly scheduled run (`15 2 * * *`).
  - Manual `workflow_dispatch`.
- **Soft Failure**:
  Job runs with `continue-on-error: ${{ vars.SIM_PR_REQUIRED != 'true' }}`. Failures report in
  GitHub Actions summaries and artifacts but do not block branch merges.
- **PR Opt-In via `sim` Label**:
  - Pull requests do **not** run simulation CI by default.
  - Trusted developers can opt in a PR by adding the **`sim`** label to the PR.
  - **Fork PR Security**: PRs from forks **never execute** on the self-hosted runner:
    ```yaml
    if: >-
      github.event_name != 'pull_request' ||
      (github.event.pull_request.head.repo.full_name == github.repository &&
       (contains(github.event.pull_request.labels.*.name, 'sim') || vars.SIM_PR_REQUIRED == 'true'))
    ```

### Stage 2: Flake Gate and Branch Protection Enforcement
The owner enforces PR simulation gating once the runner demonstrates stability:

1. **Two-Week Stability Requirement**:
   The test suite must complete **two consecutive calendar weeks with zero unquarantined failures**
   on `main` pushes and nightly scheduled runs.
2. **Owner Actions**:
   - Set repository variable `SIM_PR_REQUIRED=true` under **Settings > Secrets and variables > Actions > Variables**.
   - In GitHub **Branch Protection Rules** for `main`:
     Add status check **`Simulation / sim`** to the required status checks list for PRs touching:
     - `apps/client-unreal/**`
     - `apps/api/**`
     - `packages/**`
     - `.github/workflows/sim.yml`

---

## 6. Optional Video Diagnostics (`--video`)

When a single-client scenario fails during a nightly run or manual dispatch, the runner can capture
an off-screen rendered replay to produce an MP4 diagnostic video.

### Runner Prerequisites

Verified via:
```bash
bash infra/scripts/check-sim-runner.sh --video
```

- `xvfb-run` (X Virtual Framebuffer).
- `ffmpeg` with `libx264` support.
- `xauth`.
- **Mesa lavapipe**: CPU-based software Vulkan ICD (`/usr/share/vulkan/icd.d/lvp_icd.json` or path in `SIM_VIDEO_ICD`).

### Execution Rules

1. **Failure-Only Trigger**:
   Video rendering runs **only when a scenario fails**. It is never executed for passing scenarios.
2. **Single-Client Scenarios Only**:
   Rendered retry is supported for single-client scenarios (`run-sim.sh`). Multi-client paired
   scenarios (`run-sim-multi.sh`) do not execute video retries.
3. **Execution Context**:
   Invoked via [apps/client-unreal/Scripts/sim-video.sh](../../apps/client-unreal/Scripts/sim-video.sh):
   - Strips `-nullrhi`.
   - Adds `-vulkan -sm5 -AllowSoftwareRendering -RenderOffscreen -DumpMovie -ForceRes -ResX=960 -ResY=540 -FPS=15 -NoVSync`.
   - Uses SM5 and permits CPU devices because the default UE profile excludes lavapipe; Lumen, Nanite and virtual shadows are disabled only in this diagnostic retry.
   - Runs under `xvfb-run -a -s '-screen 0 960x540x24'`.
   - Outputs frame dumps to `$dest/frames/`, assembles them with a concat manifest, and encodes
     to H.264 MP4 (`960x540` at 15 fps).
4. **Isolated Retry Reports**:
   The video retry writes separate outputs:
   - `retry-exit-code.txt`
   - `stdout.log`
   - `ffmpeg.log`
   - Standalone retry JUnit XML.
5. **No Verdict Overrides**:
   The video retry's exit code and report **never replace or alter the original headless test verdict**.
   The headless run remains the sole authority for test pass/fail status.
6. **No Claim Playable Until Tested**:
   Do not assume or claim diagnostic videos are playable or operational until verified on the
   specific runner host with lavapipe and ffmpeg installed.

## Scenario fixture selection

A scenario can request `# fixture: phase1a-social-aggro`. Both members of a pair must agree.
The CI orchestrator validates the supported fixture name, clears inherited `ZONE_SIM_FIXTURE`
for ordinary cases, and sets it for this fresh API process only. Fixture selection requires
`--fresh-stack`; an attached running API cannot switch fixture data.

## Headless traceability scope

Completed stories tagged `[client-visible]` must have a valid `# covers:` reference.
A reference means the scenario asserts the observable portion of the story; existing server
U/A tests, renderer automation and cooked-asset checks remain required for the rest. Tags are
chosen from story behavior before coverage exists; uncovered cases fail and remain visible.
