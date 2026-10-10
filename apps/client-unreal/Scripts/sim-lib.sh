# Shared helpers for run-sim.sh and run-sim-multi.sh. Source it; it runs nothing by itself.
#
# Environment overrides (mainly for tests):
#   SIM_API_BIN       optional API executable (Phase2 defaults to a current-repo build)
#   SIM_MIGRATE_BIN   Phase2 migration executable; must match the current candidate
#   SIM_PHASE2_SEEDER explicit published pack seeder path; see phase2-fixtures.md
#   SIM_BOT_BIN       bot executable to run instead of the UnrealEditor binary (no uproject arg is passed)
#   SIM_SKIP_BUILD=1  never build
#   SIM_SAVED_DIR     where the bot writes its outputs (default <project>/Saved/Sim)
#   SIM_COVERAGE_CMD  CLI prefix accepting `coverage --file PATH --out PATH` (default Rust replay CLI)
#   SIM_PIPELINE_VERDICT completed schema v1 JSON; written only after final wrapper gates
#   SIM_REPLAY_CMD    command run as `$SIM_REPLAY_CMD <file.nfr>` (default: nightfall-replay --source file --file)
#   SIM_API_URL       API base for the health check (default http://localhost:3000)
#   SIM_REQUIRE_OWNED_API=1 reject attach/occupied endpoints; verify the launched API owns its listener
#   SIM_REQUIRE_FRESH_ARTIFACTS=1 snapshot outputs before launch; collect only changed files within wall bounds
#   SIM_TIMEOUT       seconds per scenario / per group (default 600)
#   SIM_BOT_ARGS_JSON additional launch args as a JSON array (e.g. ["-ResX=1280"])

SIM_HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SIM_PROJECT_DIR="$(cd "$SIM_HERE/.." && pwd)"
SIM_REPO="$(cd "$SIM_HERE/../../.." && pwd)"
SIM_API_URL="${SIM_API_URL:-http://localhost:3000}"
SIM_TIMEOUT="${SIM_TIMEOUT:-600}"
SIM_SAVED_DIR="${SIM_SAVED_DIR:-$SIM_PROJECT_DIR/Saved/Sim}"
SIM_STARTED_API_PID=""

sim_die() { echo "sim: $*" >&2; exit 2; }
sim_log() { echo "sim: $*" >&2; }

# UE_ROOT / LINUX_MULTIARCH_ROOT from the environment, else scraped from ~/.bashrc (a non-interactive
# shell returns early from it, so sourcing is not enough).
sim_resolve_env() {
  local var line
  for var in UE_ROOT LINUX_MULTIARCH_ROOT; do
    if [[ -z "${!var:-}" && -f "$HOME/.bashrc" ]]; then
      line="$(grep -E "^[[:space:]]*export[[:space:]]+$var=" "$HOME/.bashrc" | tail -1 || true)"
      if [[ -n "$line" ]]; then
        line="${line#*=}"; line="${line%\"}"; line="${line#\"}"; line="${line%\'}"; line="${line#\'}"
        printf -v "$var" '%s' "${line//\$HOME/$HOME}"
        export "$var"
      fi
    fi
  done
  if [[ -z "${SIM_BOT_BIN:-}" ]]; then
    [[ -n "${UE_ROOT:-}" && -d "$UE_ROOT" ]] || sim_die "UE_ROOT is not set (env or ~/.bashrc)"
    [[ -n "${LINUX_MULTIARCH_ROOT:-}" ]] || sim_die "LINUX_MULTIARCH_ROOT is not set (env or ~/.bashrc)"
  fi
}

# The bot is the editor binary run with -game (README "Headless"): the Nightfall game target needs
# cooked content and crashes loading engine packages. The staleness probe is the project module.
sim_bot_bin() { echo "${SIM_BOT_BIN:-$UE_ROOT/Engine/Binaries/Linux/UnrealEditor}"; }
sim_module_lib() { echo "$SIM_PROJECT_DIR/Binaries/Linux/libUnrealEditor-Nightfall.so"; }

# Builds the Nightfall game target when the binary is missing or older than any source/uproject file.
sim_ensure_build() {
  [[ -n "${SIM_SKIP_BUILD:-}" || -n "${SIM_BOT_BIN:-}" ]] && return 0
  local bin; bin="$(sim_module_lib)"
  if [[ -f "$bin" ]] && [[ -z "$(find "$SIM_PROJECT_DIR/Source" "$SIM_PROJECT_DIR/Nightfall.uproject" \
        -type f \( -name '*.cpp' -o -name '*.h' -o -name '*.cs' -o -name '*.uproject' \) -newer "$bin" -print -quit 2>/dev/null)" ]]; then
    sim_log "game binary is up to date"
    return 0
  fi
  sim_log "building NightfallEditor (Linux Development)"
  bash "$SIM_HERE/setup-turbolink.sh" >&2
  "$UE_ROOT/Engine/Build/BatchFiles/Linux/Build.sh" NightfallEditor Linux Development \
    -Project="$SIM_PROJECT_DIR/Nightfall.uproject" -WaitMutex -NoHotReloadFromIDE >&2 || sim_die "build failed"
}

sim_api_healthy() { curl -sf -m 2 -o /dev/null "$SIM_API_URL/health"; }

# Linux CI ownership check: health alone cannot establish which process answered. Include cargo's
# descendants, and fail closed on unreadable /proc data or a non-loopback endpoint. Default user
# startup/attach behavior does not need this probe.
sim_api_endpoint_probe() {
  python3 - "$SIM_API_URL" "$1" "${SIM_STARTED_API_PID:-}" <<'PY'
import ipaddress
from pathlib import Path
import socket
import sys
from urllib.parse import urlsplit

try:
    url = urlsplit(sys.argv[1])
    if url.scheme != 'http' or url.username or url.password or url.path not in ('', '/'):
        raise ValueError('owned API requires a local HTTP base URL')
    if url.hostname != 'localhost' and not ipaddress.ip_address(url.hostname).is_loopback:
        raise ValueError('owned API requires a loopback endpoint')
    addresses = socket.getaddrinfo(url.hostname, url.port or 80, type=socket.SOCK_STREAM)
    if not addresses or not all(ipaddress.ip_address(a[4][0]).is_loopback for a in addresses):
        raise ValueError('owned API requires a loopback endpoint')
    port = url.port or 80
    listeners = set()
    for table in ('tcp', 'tcp6'):
        for line in Path('/proc/net/' + table).read_text().splitlines()[1:]:
            fields = line.split()
            if fields[3] == '0A' and int(fields[1].split(':')[1], 16) == port:
                listeners.add(fields[9])
    if sys.argv[2] == 'free':
        if listeners:
            raise ValueError('API endpoint port is occupied; refusing to attach')
    else:
        root = int(sys.argv[3])
        parents = {}
        for process in Path('/proc').glob('[0-9]*'):
            try:
                stat = (process / 'stat').read_text().rsplit(')', 1)[1].split()
                parents[int(process.name)] = int(stat[1])
            except (FileNotFoundError, ProcessLookupError):
                continue
        owned = {root}
        while True:
            children = {pid for pid, parent in parents.items() if parent in owned}
            if children <= owned:
                break
            owned |= children
        sockets = set()
        for pid in owned:
            try:
                for fd in Path(f'/proc/{pid}/fd').iterdir():
                    try:
                        target = str(fd.readlink())
                        if target.startswith('socket:['):
                            sockets.add(target[8:-1])
                    except FileNotFoundError:
                        continue
            except FileNotFoundError:
                continue
        if root not in parents or not listeners or not listeners <= sockets:
            raise ValueError('healthy API listener is not owned by the launched process')
except (OSError, ValueError, TypeError) as error:
    print(f'sim: {error}', file=sys.stderr)
    sys.exit(1)
PY
}

# $1 = attach | start. attach requires a healthy API; start brings up compose and the API.
sim_api_prepare() {
  case "$1" in
    attach)
      [[ "${SIM_REQUIRE_OWNED_API:-0}" != 1 ]] || sim_die "owned API mode requires --api start"
      sim_api_healthy || sim_die "no API answering $SIM_API_URL/health (use --api start, or run \`moon run api:dev\`)"
      ;;
    start)
      if [[ "${SIM_REQUIRE_OWNED_API:-0}" == 1 ]]; then
        sim_api_endpoint_probe free || sim_die "cannot start an owned API at $SIM_API_URL"
      elif sim_api_healthy; then sim_log "API already healthy at $SIM_API_URL; attaching"; return 0; fi
      sim_log "starting compose stack and the API (AUTH_DEV_TOKENS=1)"
      (cd "$SIM_REPO" && docker compose up -d --wait >&2) || sim_die "docker compose up failed"
      (
        cd "$SIM_REPO"
        if [[ -f .env ]]; then set -a; . ./.env; set +a; fi
        if [[ -n "${SIM_API_BIN:-}" ]]; then
          AUTH_DEV_TOKENS=1 exec "$SIM_API_BIN"
        fi
        AUTH_DEV_TOKENS=1 exec cargo run --quiet -p nightfall-api
      ) >"${SIM_API_LOG:-/dev/null}" 2>&1 &
      SIM_STARTED_API_PID=$!
      trap sim_exit_cleanup EXIT
      local i
      for ((i = 0; i < ${SIM_API_WAIT:-180}; i++)); do
        kill -0 "$SIM_STARTED_API_PID" 2>/dev/null || sim_die "API exited during startup (see ${SIM_API_LOG:-its log})"
        if sim_api_healthy; then
          if [[ "${SIM_REQUIRE_OWNED_API:-0}" == 1 ]]; then
            sim_api_endpoint_probe owned || sim_die "API ownership verification failed"
            kill -0 "$SIM_STARTED_API_PID" 2>/dev/null || sim_die "owned API exited during startup"
          fi
          return 0
        fi
        sleep 1
      done
      sim_die "API did not become healthy within ${SIM_API_WAIT:-180}s"
      ;;
    *) sim_die "--api must be attach or start" ;;
  esac
}

sim_api_cleanup() {
  [[ -n "$SIM_STARTED_API_PID" ]] || return 0
  pkill -TERM -P "$SIM_STARTED_API_PID" 2>/dev/null || true
  kill -TERM "$SIM_STARTED_API_PID" 2>/dev/null || true
  if [[ -n "${SIM_FIXTURE_DIR:-}" ]]; then
    local grace
    for ((grace=0; grace<15; grace++)); do
      kill -0 "$SIM_STARTED_API_PID" 2>/dev/null || break
      sleep 1
    done
    kill -KILL "$SIM_STARTED_API_PID" 2>/dev/null || true
  fi
  wait "$SIM_STARTED_API_PID" 2>/dev/null || true
  SIM_STARTED_API_PID=""
}

# Expands scenario args (paths, quoted globs) to absolute .nfs paths, one per line; in order, deduplicated.
sim_expand_scenarios() {
  local arg f
  local -A seen=()
  for arg in "$@"; do
    if [[ -f "$arg" ]]; then
      set -- "$arg"
    elif [[ -f "$SIM_PROJECT_DIR/$arg" ]]; then
      set -- "$SIM_PROJECT_DIR/$arg"
    else
      # shellcheck disable=SC2086
      set -- $(compgen -G "$arg" || compgen -G "$SIM_PROJECT_DIR/$arg" || true)
    fi
    for f in "$@"; do
      [[ -f "$f" ]] || continue
      f="$(cd "$(dirname "$f")" && pwd)/$(basename "$f")"
      [[ -n "${seen[$f]:-}" ]] && continue
      seen[$f]=1
      echo "$f"
    done
    set --
  done
}

# Full command line of the bot for one scenario (extra args follow the scenario path).
# Echoes NUL-free, space-safe args via the SIM_BOT_CMD array.
sim_bot_cmd() {
  local scenario="$1"; shift
  SIM_BOT_CMD=()
  local bin; bin="$(sim_bot_bin)"
  SIM_BOT_CMD+=("$bin")
  [[ -n "${SIM_BOT_BIN:-}" ]] || SIM_BOT_CMD+=("$SIM_PROJECT_DIR/Nightfall.uproject")
  SIM_BOT_CMD+=(-game -nullrhi -nosound -unattended "-BotScenario=$scenario" "$@")
  if [[ -n "${SIM_BOT_ARGS_JSON:-}" ]]; then
    local args_file
    args_file="$(mktemp)"
    if ! python3 -c 'import json,os,sys; args=json.loads(os.environ["SIM_BOT_ARGS_JSON"]); assert isinstance(args,list) and all(isinstance(a,str) and "\0" not in a for a in args); sys.stdout.buffer.write(b"".join(a.encode()+b"\0" for a in args))' >"$args_file"; then
      rm -f "$args_file"; sim_die "SIM_BOT_ARGS_JSON must be an array of strings"
    fi
    local -a extra_args=()
    mapfile -d '' -t extra_args <"$args_file"
    rm -f "$args_file"
    SIM_BOT_CMD+=("${extra_args[@]}")
  fi
  if [[ -n "${SIM_FIXTURE_DIR:-}" ]]; then
    local fixture_args_file
    fixture_args_file="$(mktemp)"
    python3 "$SIM_HERE/phase2-fixture.py" args --folder "$SIM_FIXTURE_DIR" "$scenario" >"$fixture_args_file" \
      || { rm -f "$fixture_args_file"; sim_die "invalid fixture role arguments"; }
    local -a fixture_args=()
    mapfile -d '' -t fixture_args <"$fixture_args_file"
    rm -f "$fixture_args_file"
    SIM_BOT_CMD+=("${fixture_args[@]}")
  fi
}

# The engine resolves Engine/Content relative to the working directory (see run-tests.sh).
sim_bot_cwd() { if [[ -n "${SIM_BOT_BIN:-}" ]]; then pwd; else echo "$UE_ROOT/Engine/Binaries/Linux"; fi; }

# Copies everything the bot wrote into $SIM_SAVED_DIR after marker $1 to directory $2.
sim_collect() {
  local marker="$1" dest="$2"
  mkdir -p "$dest"
  [[ -d "$SIM_SAVED_DIR" ]] || return 0
  if [[ "${SIM_REQUIRE_FRESH_ARTIFACTS:-0}" == 1 ]]; then
    python3 - "$marker" "$SIM_SAVED_DIR" "$dest" <<'PY'
import json
from pathlib import Path
import shutil
import sys
import time

marker, saved, dest = map(Path, sys.argv[1:])
before = json.loads(marker.read_text())
start, end = marker.stat().st_mtime_ns, time.time_ns()
for file in saved.rglob('*'):
    if not file.is_file() or file.is_symlink():
        continue
    stat = file.stat()
    identity = [stat.st_mtime_ns, stat.st_ctime_ns, stat.st_size, stat.st_ino]
    if start < stat.st_mtime_ns <= end and before.get(str(file.resolve())) != identity:
        shutil.copy2(file, dest / file.name)
PY
    return $?
  fi
  find "$SIM_SAVED_DIR" -type f -newer "$marker" -exec cp -p {} "$dest"/ \;
}

sim_new_marker() {
  local m; m="$(mktemp)"
  if [[ "${SIM_REQUIRE_FRESH_ARTIFACTS:-0}" == 1 ]]; then
    python3 - "$SIM_SAVED_DIR" "$m" <<'PY' || { rm -f "$m"; return 1; }
import json
from pathlib import Path
import sys

saved, marker = map(Path, sys.argv[1:])
before = {}
for file in saved.rglob('*'):
    if file.is_file() and not file.is_symlink():
        stat = file.stat()
        before[str(file.resolve())] = [stat.st_mtime_ns, stat.st_ctime_ns, stat.st_size, stat.st_ino]
marker.write_text(json.dumps(before))
PY
  fi
  touch "$m"; sleep 0.05; echo "$m"
}

# nightfall-replay binary (built once per script run). SIM_REPLAY_BIN overrides.
sim_replay_bin() {
  if [[ -z "${SIM_REPLAY_BIN:-}" ]]; then
    (cd "$SIM_REPO" && cargo build --quiet -p nightfall-api --bin nightfall-replay >&2) || sim_die "building nightfall-replay failed"
    SIM_REPLAY_BIN="$(cd "$SIM_REPO" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/nightfall-replay"
  fi
  echo "$SIM_REPLAY_BIN"
}

# Exports the whole live zone prefix, checking the bot's session presence (not filtering):
# $1 = artifact dir, $2 = scenario name. The player entity is the JUnit
# own_entity_id. Zone SIM_ZONE_ID (default 1, test_zone) at its newest epoch; NATS from NATS_URL.
# Does nothing when a recording is already there or the report names no entity.
sim_export_recording() {
  local dest="$1" name="$2" entity bin
  [[ -f "$dest/$name.nfr" ]] && return 0
  [[ -n "${SIM_REPLAY_CMD:-}" ]] && return 0
  entity="$(sed -n 's/.*name="own_entity_id" value="\([^"]*\)".*/\1/p' "$dest/$name.xml" 2>/dev/null | head -1)"
  [[ -n "$entity" ]] || return 1
  bin="$(sim_replay_bin)"
  "$bin" export --zone "${SIM_ZONE_ID:-1}" --latest --session "$entity" --live --out "$dest/$name.nfr" >"$dest/$name.export.log" 2>&1
}

sim_capture_group() {
  local scenario bin names=()
  for scenario in "$@"; do names+=("$(basename "$scenario" .nfs)"); done
  bin="$(sim_replay_bin)" || return 1
  python3 "$SIM_HERE/sim-gates.py" capture-group --folder "$ARTIFACTS" \
    --binary "$bin" --zone "${SIM_ZONE_ID:-1}" "${names[@]}"
}

# Replays one recording; returns the tool's exit code (non-zero = divergence).
sim_replay() {
  local nfr="$1" log="$2"
  if [[ -n "${SIM_REPLAY_CMD:-}" ]]; then
    # shellcheck disable=SC2086
    $SIM_REPLAY_CMD "$nfr" >"$log" 2>&1
  else
    "$(sim_replay_bin)" check --file "$nfr" >"$log" 2>&1
  fi
}

# Runs `cmd...` with a wall-clock limit of $1 seconds; 124 on timeout.
sim_with_timeout() { local t="$1"; shift; timeout --kill-after=10 "$t" "$@"; }

# Wrappers claim an empty destination before any launch. CI pre-opens orchestration.log only.
# Evidence is retained on rejection; hidden files count as nonempty too.
sim_claim_artifacts() {
  local scenario names=()
  for scenario in "$@"; do names+=("$(basename "$scenario" .nfs)"); done
  python3 "$SIM_HERE/sim-gates.py" claim "$ARTIFACTS" "${names[@]}" || sim_die "artifact destination must be empty"
  # Snapshot even in local attach mode so future-dated unchanged Saved outputs cannot be reused.
  export SIM_REQUIRE_FRESH_ARTIFACTS=1
}

# Every current recording is counted using the Rust catalogue; never infer it from scenarios.
# --no-replay still covers any recording the bot produced, but permits no recordings locally.
sim_transition_coverage() {
  local dest="$1" required="$2" nfr failed=0 inputs=() nfrs=()
  shopt -s nullglob; nfrs=("$dest"/*.nfr); shopt -u nullglob
  if ((${#nfrs[@]} == 0)); then
    ((required == 0)) && [[ "${SIM_REQUIRE_TRANSITION_COVERAGE:-0}" != 1 ]] && return 0
    sim_log "missing transition coverage: no current recording"; return 1
  fi
  for nfr in "${nfrs[@]}"; do
    python3 "$SIM_HERE/sim-gates.py" coverage --file "$nfr" --out "${nfr%.nfr}.coverage.transitions.json" || failed=1
    inputs+=("${nfr%.nfr}.coverage.transitions.json")
  done
  python3 "$SIM_HERE/sim-gates.py" merge-transitions --unique-recordings --out "$dest/coverage.transitions.json" "${inputs[@]}" || failed=1
  ((failed == 0))
}

sim_suite_transitions() {
  local scenario name nfr failed=0 inputs=() raw=() shared=()
  for scenario in "$@"; do
    name="$(basename "$scenario" .nfs)"
    if [[ ! -f "$ARTIFACTS/$name/coverage.transitions.json" ]] && {
      ((REPLAY)) || [[ "${SIM_REQUIRE_TRANSITION_COVERAGE:-0}" == 1 ]];
    }; then
      failed=1
    fi
    shopt -s nullglob; raw=("$ARTIFACTS/$name"/*.nfr); shopt -u nullglob
    if [[ "${GROUP_CAPTURE:-0}" != 1 ]]; then
      for nfr in "${raw[@]}"; do inputs+=("${nfr%.nfr}.coverage.transitions.json"); done
    fi
  done
  # Legacy bots may write one shared group recording without a scenario prefix.
  # Canonical live groups count only group.nfr; per-role gates above remain mandatory.
  # Offline fixtures can also contain independent recordings, deduplicated within this unit.
  shopt -s nullglob; shared=("$ARTIFACTS"/*.nfr); shopt -u nullglob
  for nfr in "${shared[@]}"; do
    if [[ "${GROUP_CAPTURE:-0}" == 1 && "$nfr" != "$ARTIFACTS/group.nfr" ]]; then failed=1; continue; fi
    if ((REPLAY)); then sim_replay "$nfr" "${nfr%.nfr}.replay.log" || failed=1; fi
    # A shared recording has no unique client report/session. Generate a whole-zone trace.
    if ! python3 "$SIM_HERE/sim-trace.py" "$nfr" "$ARTIFACTS/_shared-trace-report.xml" || [[ ! -s "${nfr%.nfr}.trace.html" || ! -r "${nfr%.nfr}.trace.html" ]]; then failed=1; fi
    python3 "$SIM_HERE/sim-gates.py" coverage --file "$nfr" --out "${nfr%.nfr}.coverage.transitions.json" || failed=1
    inputs+=("${nfr%.nfr}.coverage.transitions.json")
  done
  if [[ "${GROUP_CAPTURE:-0}" == 1 ]]; then
    [[ -f "$ARTIFACTS/group.nfr" ]] || failed=1
  fi
  ((${#inputs[@]})) || { ((failed == 0)); return; }
  python3 "$SIM_HERE/sim-gates.py" merge-transitions --unique-recordings --out "$ARTIFACTS/coverage.transitions.json" "${inputs[@]}" || failed=1
  ((failed == 0))
}

# Only called after all gates and group merge. Early aborts cannot publish completed evidence.
sim_finalize_verdict() {
  local code="$1"; shift
  local scenario names=() args=()
  for scenario in "${SCENARIOS[@]}"; do names+=("$(basename "$scenario" .nfs)"); done
  if [[ -n "${GROUP:-}" ]]; then args+=(--group "$ARTIFACTS/group.xml"); fi
  for scenario in "$@"; do args+=(--infrastructure "$scenario"); done
  python3 "$SIM_HERE/sim-gates.py" finalize --folder "$ARTIFACTS" \
    --out "${SIM_PIPELINE_VERDICT:-$ARTIFACTS/pipeline-verdict.json}" --code "$code" \
    "${args[@]}" "${names[@]}"
}

# Phase2 is always freshly provisioned before admission. The helper never reads root .env.
sim_fixture_detect() { python3 "$SIM_HERE/phase2-fixture.py" detect "$@"; }
sim_fixture_validate_mode() {
  [[ "$1" == phase2-* ]] || return 0
  [[ "$2" == start ]] || sim_die "Phase2 fixture requires --api start; attaching is refused"
  [[ "$SIM_API_URL" == http://localhost:3000 || "$SIM_API_URL" == http://127.0.0.1:3000 ]] \
    || sim_die "Phase2 chooses fresh endpoints; explicit SIM_API_URL is unsupported"
}

sim_exit_cleanup() {
  local original_code=$? cleanup_code=0
  trap - EXIT
  sim_api_cleanup
  if [[ -n "${SIM_FIXTURE_DIR:-}" ]]; then
    python3 "$SIM_HERE/phase2-fixture.py" cleanup --folder "$SIM_FIXTURE_DIR" || cleanup_code=$?
  fi
  if ((cleanup_code != 0)); then original_code=2; fi
  exit "$original_code"
}

sim_fixture_finish() {
  [[ -n "${SIM_FIXTURE_DIR:-}" ]] || return 0
  local code=0
  sim_api_cleanup
  python3 "$SIM_HERE/phase2-fixture.py" cleanup --folder "$SIM_FIXTURE_DIR" || code=$?
  # On failure leave the directory active so EXIT retries the owned teardown.
  ((code == 0)) || return "$code"
  SIM_FIXTURE_DIR=""
  SIM_API_URL="$SIM_PRE_FIXTURE_API_URL"
  if [[ "$SIM_PRE_FIXTURE_NATS_SET" == x ]]; then export NATS_URL="$SIM_PRE_FIXTURE_NATS"; else unset NATS_URL; fi
}

sim_unit_prepare() {
  local selected="$1" mode="$2"; shift 2
  if [[ "$selected" != phase2-* ]]; then
    [[ -n "$SIM_STARTED_API_PID" ]] || sim_api_prepare "$mode"
    return
  fi
  sim_api_cleanup
  SIM_PRE_FIXTURE_API_URL="$SIM_API_URL"
  SIM_PRE_FIXTURE_NATS_SET="${NATS_URL+x}"
  SIM_PRE_FIXTURE_NATS="${NATS_URL:-}"
  SIM_FIXTURE_DIR="$ARTIFACTS/$(basename "$1" .nfs)/fixture"
  trap sim_exit_cleanup EXIT
  trap 'exit 130' INT TERM
  python3 "$SIM_HERE/phase2-fixture.py" provision --folder "$SIM_FIXTURE_DIR" "$@" \
    || sim_die "Phase2 fixture provisioning failed (before API startup)"
  local endpoints_file
  endpoints_file="$(mktemp)"
  python3 "$SIM_HERE/phase2-fixture.py" endpoint --folder "$SIM_FIXTURE_DIR" >"$endpoints_file" \
    || { rm -f "$endpoints_file"; sim_die "invalid fixture endpoints"; }
  local -a endpoints=()
  mapfile -t endpoints <"$endpoints_file"; rm -f "$endpoints_file"
  SIM_API_URL="${endpoints[0]}"; export NATS_URL="${endpoints[1]}"
  sim_api_endpoint_probe free || sim_die "fixture API endpoint occupied"
  SIM_API_URL="${endpoints[2]}" sim_api_endpoint_probe free || sim_die "fixture gRPC endpoint occupied"
  python3 "$SIM_HERE/phase2-fixture.py" exec-api --folder "$SIM_FIXTURE_DIR" >"$SIM_FIXTURE_DIR/api.log" 2>&1 &
  SIM_STARTED_API_PID=$!
  local i
  for ((i = 0; i < ${SIM_API_WAIT:-180}; i++)); do
    kill -0 "$SIM_STARTED_API_PID" 2>/dev/null || sim_die "fixture API exited during startup"
    if sim_api_healthy; then
      sim_api_endpoint_probe owned || sim_die "fixture API ownership verification failed"
      SIM_API_URL="${endpoints[2]}" sim_api_endpoint_probe owned || sim_die "fixture gRPC ownership verification failed"
      kill -0 "$SIM_STARTED_API_PID" 2>/dev/null || sim_die "fixture API exited during startup"
      return 0
    fi
    sleep 1
  done
  sim_die "fixture API did not become healthy"
}
