#!/usr/bin/env bash
# Pushes a synthetic, slow `nightfall_tick_duration` histogram to the local LGTM collector so
# the "Tick p99 above 50 ms" alert can be seen firing. Every sample lands in the >500 ms
# bucket. Reported as service "nightfall-synthetic" so it is easy to tell from real data.
#   infra/scripts/push-synthetic-tick.sh [seconds=240]
set -euo pipefail
duration="${1:-240}"
endpoint="${OTLP_HTTP:-http://localhost:4318}"
start_ns=$(( $(date +%s) * 1000000000 ))
count=0
end=$(( $(date +%s) + duration ))
while [ "$(date +%s)" -lt "$end" ]; do
  count=$(( count + 20 ))
  now_ns=$(( $(date +%s) * 1000000000 ))
  curl -fsS -o /dev/null -X POST "$endpoint/v1/metrics" -H 'content-type: application/json' -d "{
    \"resourceMetrics\":[{\"resource\":{\"attributes\":[{\"key\":\"service.name\",\"value\":{\"stringValue\":\"nightfall-synthetic\"}}]},
    \"scopeMetrics\":[{\"scope\":{\"name\":\"synthetic\"},\"metrics\":[{\"name\":\"nightfall_tick_duration\",\"unit\":\"s\",
      \"histogram\":{\"aggregationTemporality\":2,\"dataPoints\":[{
        \"startTimeUnixNano\":\"$start_ns\",\"timeUnixNano\":\"$now_ns\",\"count\":\"$count\",\"sum\":$count.0,
        \"bucketCounts\":[\"0\",\"0\",\"0\",\"0\",\"0\",\"0\",\"0\",\"0\",\"0\",\"$count\"],
        \"explicitBounds\":[0.001,0.002,0.005,0.01,0.02,0.05,0.1,0.2,0.5]}]}}]}]}]}"
  sleep 10
done
