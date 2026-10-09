# Shared helpers for run-sim.sh and run-sim-multi.sh. Source it; it runs nothing by itself.
#
# Environment overrides (mainly for tests):
#   SIM_BOT_BIN       bot executable to run instead of the UnrealEditor binary (no uproject arg is passed)
#   SIM_SKIP_BUILD=1  never build
#   SIM_SAVED_DIR     where the bot writes its outputs (default <project>/Saved/Sim)
#   SIM_REPLAY_CMD    command run as `$SIM_REPLAY_CMD <file.nfr>` (default: nightfall-replay --source file --file)
#   SIM_API_URL       API base for the health check (default http://localhost:3000)
#   SIM_TIMEOUT       seconds per scenario / per group (default 600)

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

# $1 = attach | start. attach requires a healthy API; start brings up compose and the API.
sim_api_prepare() {
  case "$1" in
    attach)
      sim_api_healthy || sim_die "no API answering $SIM_API_URL/health (use --api start, or run \`moon run api:dev\`)"
      ;;
    start)
      if sim_api_healthy; then sim_log "API already healthy at $SIM_API_URL; attaching"; return 0; fi
      sim_log "starting compose stack and the API (AUTH_DEV_TOKENS=1)"
      (cd "$SIM_REPO" && docker compose up -d --wait >&2) || sim_die "docker compose up failed"
      (
        cd "$SIM_REPO"
        if [[ -f .env ]]; then set -a; . ./.env; set +a; fi
        AUTH_DEV_TOKENS=1 exec cargo run --quiet -p nightfall-api
      ) >"${SIM_API_LOG:-/dev/null}" 2>&1 &
      SIM_STARTED_API_PID=$!
      trap sim_api_cleanup EXIT
      local i
      for ((i = 0; i < ${SIM_API_WAIT:-180}; i++)); do
        sim_api_healthy && return 0
        kill -0 "$SIM_STARTED_API_PID" 2>/dev/null || sim_die "API exited during startup (see ${SIM_API_LOG:-its log})"
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
}

# The engine resolves Engine/Content relative to the working directory (see run-tests.sh).
sim_bot_cwd() { if [[ -n "${SIM_BOT_BIN:-}" ]]; then pwd; else echo "$UE_ROOT/Engine/Binaries/Linux"; fi; }

# Copies everything the bot wrote into $SIM_SAVED_DIR after marker $1 to directory $2.
sim_collect() {
  local marker="$1" dest="$2"
  mkdir -p "$dest"
  [[ -d "$SIM_SAVED_DIR" ]] || return 0
  find "$SIM_SAVED_DIR" -type f -newer "$marker" -exec cp -p {} "$dest"/ \;
}

sim_new_marker() { local m; m="$(mktemp)"; touch "$m"; sleep 0.05; echo "$m"; }

# nightfall-replay binary (built once per script run). SIM_REPLAY_BIN overrides.
sim_replay_bin() {
  if [[ -z "${SIM_REPLAY_BIN:-}" ]]; then
    (cd "$SIM_REPO" && cargo build --quiet -p nightfall-api --bin nightfall-replay >&2) || sim_die "building nightfall-replay failed"
    SIM_REPLAY_BIN="$(cd "$SIM_REPO" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/nightfall-replay"
  fi
  echo "$SIM_REPLAY_BIN"
}

# Exports the bot's session from the live event log (the server records every session; the bot
# writes no file): $1 = artifact dir, $2 = scenario name. The player entity is the JUnit
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
