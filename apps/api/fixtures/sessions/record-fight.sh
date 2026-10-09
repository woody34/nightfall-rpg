#!/usr/bin/env bash
# Run from any directory, with the compose Postgres and JetStream services healthy.
# Changes only a temporary data copy; uses spare ports and a dedicated zone epoch.
set -euo pipefail
cd "$(dirname "$0")/../../../.."
export DATABASE_URL="${DATABASE_URL:-postgres://nightfall:nightfall@localhost:5432/nightfall}"
export NATS_URL="${NATS_URL:-nats://localhost:4222}"
export HTTP_ADDR=127.0.0.1:3107 GRPC_ADDR=127.0.0.1:50107 AUTH_DEV_TOKENS=1
export WS_PUBLIC_URL=ws://127.0.0.1:3107/ws
work=$(mktemp -d)
api_pid=''
if curl -fsS http://127.0.0.1:3107/health >/dev/null 2>&1; then
  echo 'Port 3107 already serves an API; refusing to record another process.' >&2
  rm -rf "$work"
  exit 1
fi
cleanup() {
  if [[ -n "$api_pid" ]]; then kill -INT "$api_pid" 2>/dev/null || true; wait "$api_pid" || true; fi
  if [[ -f "$work/api.log" && ! -f "$work/success" ]]; then cat "$work/api.log" >&2; fi
  rm -rf "$work"
}
trap cleanup EXIT
mkdir -p "$work/zones" "$work/npcs"
sed 's/zone_id = 1/zone_id = 6102/' packages/data/zones/test_zone.toml > "$work/zones/fight.toml"
# X[2]=68: a single kill levels A; the subsequent L2 death loses 29 XP and delevels.
sed 's/xp_reward = 28/xp_reward = 68/' packages/data/npcs/keltir.toml > "$work/npcs/keltir.toml"
export ZONE_FILE="$work/zones/fight.toml"
cargo build -p nightfall-api --bin nightfall-api --bin nightfall-replay --example record_session
# Respect an explicitly supplied Cargo target directory.
artifacts="${CARGO_TARGET_DIR:-target}/debug"
"$artifacts/nightfall-api" > "$work/api.log" 2>&1 &
api_pid=$!
for _ in {1..60}; do
  if curl -fsS http://127.0.0.1:3107/health >/dev/null 2>&1; then break; fi
  if ! kill -0 "$api_pid" 2>/dev/null; then cat "$work/api.log"; exit 1; fi
  sleep 1
done
"$artifacts/examples/record_session" --fight --http http://127.0.0.1:3107 --grpc http://127.0.0.1:50107
kill -INT "$api_pid"
wait "$api_pid"
api_pid=''
"$artifacts/nightfall-replay" export --zone 6102 --latest --out apps/api/fixtures/sessions/two-players-fight-v2.nfr
"$artifacts/nightfall-replay" --source file --file apps/api/fixtures/sessions/two-players-fight-v2.nfr

cargo test -p nightfall-api --test replay_fixture fight_
touch "$work/success"
