#!/usr/bin/env bash
# Gauntlet casting soak: [--clients 8] [--seconds 1200] [--skip-stage] [--artifacts DIR]
# --legacy-attacks retains the previous workload, without Phase 3 acceptance.
# Each run owns a fresh Compose project on dedicated ports and deletes only its own volumes.
# SOAK_PORT_OFFSET (default 10000) offsets normal stack ports; API 13000, gRPC 15051 by default.
# Set SOAK_API_BIN to an already-built API to avoid a redundant Rust build.
# SOAK_TMPFS_VOLUMES=<size> (e.g. 8g) backs the stack's Postgres/NATS volumes with tmpfs so host
# filesystem fsync stalls cannot leak into tick latency; the run records storage.txt either way.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/sim-lib.sh"
CLIENTS=8; DURATION=1200; STAGE=1; OUT=""; CASTING=1
while (($#)); do
  case "$1" in
    --clients) CLIENTS="${2:?}"; shift 2 ;;
    --seconds) DURATION="${2:?}"; shift 2 ;;
    --legacy-attacks) CASTING=0; shift ;;
    --skip-stage) STAGE=0; shift ;;
    --artifacts) OUT="${2:?}"; shift 2 ;;
    -h|--help) sed -n '2,/^set -/p' "$0" | sed '$d'; exit 0 ;;
    *) sim_die "unknown argument $1" ;;
  esac
done
[[ "$CLIENTS" =~ ^[0-9]+$ && "$DURATION" =~ ^[0-9]+$ ]] || sim_die "clients and seconds must be integers"
((CLIENTS >= 1 && CLIENTS <= 8 && DURATION >= 1)) || sim_die "clients must be 1..8; seconds positive"
OUT="${OUT:-$SIM_PROJECT_DIR/Saved/Soak/$(date +%Y%m%d-%H%M%S)-$$}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
[[ -z "$(find "$OUT" -mindepth 1 -maxdepth 1 -print -quit)" ]] || sim_die "artifacts directory must be empty; choose a fresh directory"
if ((CASTING)); then
  # Client-owned Gauntlet role wiring is an explicit integration dependency.
  grep -Eq 'SoakRoleManifest' "$SIM_PROJECT_DIR/Build/Scripts/NightfallSoak.cs" \
    || sim_die "casting soak requires client Gauntlet SoakRoleManifest support; see phase3-fixtures.md"
  grep -Eq 'SoakScenarioSeconds' "$SIM_PROJECT_DIR/Build/Scripts/NightfallSoak.cs" \
    || sim_die "casting soak requires client SoakScenarioSeconds budget support; see phase3-fixtures.md"
fi
sim_resolve_env
export UE_ROOT
if ((STAGE)); then bash "$HERE/stage-linux.sh" >"$OUT/stage.log" 2>&1; fi
[[ -d "$SIM_PROJECT_DIR/Saved/StagedBuilds/Linux" ]] || sim_die "staged Linux game missing; run stage-linux.sh"
DOTNET="$UE_ROOT/Engine/Binaries/ThirdParty/DotNet/10.0/linux-x64/dotnet"
"$DOTNET" build "$SIM_PROJECT_DIR/Build/Scripts/Nightfall.Automation.csproj" -c Development --nologo >"$OUT/gauntlet-build.log" 2>&1
CAST_ARGS=(); if ((CASTING)); then CAST_ARGS+=(--casting); fi
python3 "$HERE/soak-fixtures.py" "$SIM_REPO" "$OUT/fixtures" "$CLIENTS" "${CAST_ARGS[@]}"
OFFSET="${SOAK_PORT_OFFSET:-10000}"
[[ "$OFFSET" =~ ^[0-9]+$ ]] && ((OFFSET >= 1 && OFFSET <= 15000)) || sim_die "SOAK_PORT_OFFSET must be 1..15000"
# The project's gRPC port is high; use a separate low default instead of adding to 50051.
GRPC_PORT=$((5051 + OFFSET)); HTTP_PORT=$((3000 + OFFSET))
COMPOSE_PROJECT="nightfall-soak-$$-$(date +%s)"
if ((CASTING)); then COMPOSE_PROJECT="nightfall-phase3-harness-soak-$$-$(date +%s)"; fi
touch "$OUT/empty.env"
docker compose --env-file "$OUT/empty.env" -f "$SIM_REPO/docker-compose.yml" \
  config --format json >"$OUT/compose-source.json"
SOAK_DB=nightfall; DB_ARGS=()
if ((CASTING)); then SOAK_DB="nf_phase3_fixture_soak_$$_$(date +%s)"; DB_ARGS+=(--database "$SOAK_DB"); fi
TMPFS_ARGS=()
if [[ -n "${SOAK_TMPFS_VOLUMES:-}" ]]; then TMPFS_ARGS=(--tmpfs-volumes "$SOAK_TMPFS_VOLUMES"); fi
python3 "$HERE/soak-compose.py" "$OUT/compose-source.json" "$OUT/compose.json" "$OFFSET" "${DB_ARGS[@]}" "${TMPFS_ARGS[@]}"
printf 'volumes=%s\n' "${SOAK_TMPFS_VOLUMES:+tmpfs:$SOAK_TMPFS_VOLUMES}${SOAK_TMPFS_VOLUMES:-docker-local}" >"$OUT/storage.txt"
COMPOSE=(docker compose --env-file "$OUT/empty.env" -p "$COMPOSE_PROJECT" -f "$OUT/compose.json")
API_PID=""
stop_api() {
  [[ -n "$API_PID" ]] || return 0
  local stop_code=0 waited_code=0
  kill -TERM "$API_PID" 2>/dev/null || true
  for ((grace=0; grace<15; grace++)); do
    kill -0 "$API_PID" 2>/dev/null || break
    sleep 1
  done
  if kill -0 "$API_PID" 2>/dev/null; then
    kill -KILL "$API_PID" 2>/dev/null || true
    stop_code=137
  fi
  wait "$API_PID" 2>/dev/null || waited_code=$?
  ((stop_code != 0)) || stop_code=$waited_code
  API_PID=""
  printf '%s\n' "$stop_code" >"$OUT/api-exit-code.txt"
  return "$stop_code"
}
cleanup() {
  local original_code="$1" cleanup_code=0
  trap - EXIT
  stop_api || original_code=1
  "${COMPOSE[@]}" logs --no-color >"$OUT/compose.log" 2>&1 || true
  timeout --kill-after=10 60 "${COMPOSE[@]}" down -v >"$OUT/cleanup.log" 2>&1 || cleanup_code=$?
  python3 "$HERE/soak-report.py" --teardown "$OUT" "$cleanup_code" || original_code=1
  exit "$original_code"
}
trap 'cleanup "$?"' EXIT
trap 'exit 130' INT TERM
"${COMPOSE[@]}" up -d --wait >"$OUT/compose-start.log" 2>&1
if [[ -z "${SOAK_API_BIN:-}" ]]; then
  (cd "$SIM_REPO" && cargo build --release --quiet -p nightfall-api --bins) >"$OUT/api-build.log" 2>&1
  SOAK_API_BIN="$(cd "$SIM_REPO" && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/nightfall-api"
fi
SOAK_REPLAY_BIN="${SOAK_REPLAY_BIN:-$(dirname "$SOAK_API_BIN")/nightfall-replay}"
[[ -x "$SOAK_REPLAY_BIN" ]] || sim_die "nightfall-replay missing beside API; build --bins or set SOAK_REPLAY_BIN"
if ((CASTING)); then
  SOAK_MIGRATE_BIN="${SOAK_MIGRATE_BIN:-$(dirname "$SOAK_API_BIN")/nightfall-migrate}"
  [[ -x "$SOAK_MIGRATE_BIN" ]] || sim_die "casting soak needs candidate nightfall-migrate"
  DATABASE_URL="postgres://nightfall:nightfall@localhost:$((5432 + OFFSET))/$SOAK_DB" \
    "$SOAK_MIGRATE_BIN" up >"$OUT/migrate.log" 2>&1
  DATABASE_URL="postgres://nightfall:nightfall@localhost:$((5432 + OFFSET))/$SOAK_DB" \
    AUTH_DEV_TOKENS=1 NIGHTFALL_PHASE3_FIXTURE=1 \
    python3 "$SIM_REPO/infra/scripts/seed-phase3-soak.py" --manifest "$OUT/fixtures/roles.json" \
    --compose-project "$COMPOSE_PROJECT" --compose-file "$OUT/compose.json" \
    >"$OUT/seed.log" 2>&1
fi
DATABASE_URL="postgres://nightfall:nightfall@localhost:$((5432 + OFFSET))/$SOAK_DB" \
NATS_URL="nats://localhost:$((4222 + OFFSET))" \
HTTP_ADDR="127.0.0.1:$HTTP_PORT" GRPC_ADDR="127.0.0.1:$GRPC_PORT" \
WS_PUBLIC_URL="ws://localhost:$HTTP_PORT/ws" AUTH_DEV_TOKENS=1 WS_MAX_SESSIONS_PER_IP=32 \
OIDC_ISSUER="http://localhost:$((8080 + OFFSET))/realms/nightfall" OIDC_AUDIENCE=nightfall-api \
OTEL_EXPORTER_OTLP_ENDPOINT="http://localhost:$((4317 + OFFSET))" OTEL_SERVICE_NAME=nightfall-api \
ZONE_FILE="$OUT/fixtures/data/zones/test_zone.toml" RULES_DIR="$OUT/fixtures/data" \
PHASE3_ENABLED="$CASTING" \
"$SOAK_API_BIN" >"$OUT/api.log" 2>&1 &
API_PID=$!
for ((i=0; i<180; i++)); do
  curl -fsS -m 2 "http://localhost:$HTTP_PORT/health" >/dev/null 2>&1 && break
  kill -0 "$API_PID" 2>/dev/null || sim_die "API exited; see $OUT/api.log"
  sleep 1
done
curl -fsS -m 2 "http://localhost:$HTTP_PORT/health" >/dev/null || sim_die "API startup timed out"
ROLE_ARGS=(); if ((CASTING)); then
  ROLE_ARGS+=("-SoakRoleManifest=$OUT/fixtures/roles.json" "-SoakScenarioSeconds=360")
fi
START=$(date +%s)
CODE=0
timeout --kill-after=20 "$((DURATION + 600))" "$UE_ROOT/Engine/Build/BatchFiles/RunUAT.sh" \
  -ScriptDir="$SIM_PROJECT_DIR/Build/Scripts" RunUnreal \
  -project="$SIM_PROJECT_DIR/Nightfall.uproject" -platform=Linux -configuration=Development \
  -build="$SIM_PROJECT_DIR/Saved/StagedBuilds" -test=Nightfall.NightfallSoak \
  -MaxLocalDevices="$CLIENTS" -SoakClients="$CLIENTS" -SoakSeconds="$DURATION" -SoakGrpc="localhost:$GRPC_PORT" \
  "${ROLE_ARGS[@]}" -SoakFixtures="$OUT/fixtures" -SoakOutput="$OUT" \
  -artifacts="$OUT/Gauntlet" -tempdir="$OUT/Temp" -unattended -utf8output \
  >"$OUT/gauntlet.log" 2>&1 || CODE=$?
END=$(date +%s)
# Graceful API shutdown flushes the final cumulative histogram without adding idle ticks.
stop_api || printf '%s\n' 'API shutdown failed or exceeded its grace period; final metrics may be incomplete' >>"$OUT/runner-errors.txt"
# A fresh Postgres/NATS namespace deterministically starts zone 1 at epoch 1. Exporting
# that exact epoch both records the seed and requires the graceful-shutdown watermark.
if ! "$SOAK_REPLAY_BIN" export --zone 1 --epoch 1 --nats "nats://localhost:$((4222 + OFFSET))" \
  --out "$OUT/soak.nfr" >"$OUT/replay-export.log" 2>&1; then
  printf '%s\n' 'Could not export complete zone 1 epoch 1 recording' >>"$OUT/runner-errors.txt"
elif ! "$SOAK_REPLAY_BIN" check --file "$OUT/soak.nfr" >"$OUT/replay-check.log" 2>&1; then
  printf '%s\n' 'Soak recording did not replay byte-identically' >>"$OUT/runner-errors.txt"
fi
if ((CASTING)) && [[ -s "$OUT/soak.nfr" ]]; then
  SIM_REPLAY_BIN="$SOAK_REPLAY_BIN" python3 "$HERE/sim-gates.py" coverage \
    --file "$OUT/soak.nfr" --out "$OUT/coverage.transitions.json" \
    >"$OUT/coverage.log" 2>&1 || printf '%s\n' 'Casting soak coverage export failed' >>"$OUT/runner-errors.txt"
fi
# The API exports OTLP metrics every 10s; allow export plus the Prometheus scrape.
sleep "${SOAK_TELEMETRY_FLUSH_SECONDS:-25}"
export SOAK_GRAFANA_URL="${SOAK_GRAFANA_URL:-http://localhost:$((3300 + OFFSET))}"
REPORT_ARGS=(); if ((CASTING)); then REPORT_ARGS+=(--require-casting); fi
python3 "$HERE/soak-report.py" "$OUT" "$CLIENTS" "$DURATION" "$START" "$END" "$CODE" \
  --metrics-at "$(date +%s)" "${REPORT_ARGS[@]}"
