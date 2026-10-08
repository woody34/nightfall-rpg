#!/usr/bin/env bash
# Acceptance check for the telemetry stack, by API query only (no browser).
# Needs: `docker compose up -d lgtm` and the API running with
# OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 (HTTP on $API, default localhost:3000).
# Exits non-zero at the first missing signal. Flushing takes up to ~15 s.
set -euo pipefail
API="${API:-http://localhost:3000}"
GRAFANA="${GRAFANA:-http://localhost:3300}"
AUTH="admin:admin"
proxy() { curl -fsS -u "$AUTH" -G "$GRAFANA/api/datasources/proxy/uid/$1/$2" "${@:3}"; }

curl -fsS "$API/health" >/dev/null
sleep 15

echo "1. trace (Tempo): a /health request"
proxy tempo api/search --data-urlencode 'q={resource.service.name="nightfall-api" && name="http.request"}' \
  | grep -q '"rootTraceName":"http.request"'

echo "2. log line (Loki)"
proxy loki loki/api/v1/query_range --data-urlencode 'query={service_name="nightfall-api"}' --data-urlencode 'limit=1' \
  | grep -q '"values":\[\['

echo "3. metric sample (Prometheus, pushed over OTLP)"
proxy prometheus api/v1/query --data-urlencode 'query=nightfall_http_requests_total{route="/health"}' \
  | grep -q '"resultType":"vector","result":\[{'

echo "4. /metrics scrape on the API"
curl -fsS "$API/metrics" | grep -q '^nightfall_http_requests_total'

echo "ok. For the alert, run infra/scripts/push-synthetic-tick.sh and check:"
echo "  curl -s -u $AUTH $GRAFANA/api/prometheus/grafana/api/v1/rules | grep -o '\"state\":\"[a-z]*\"'"
