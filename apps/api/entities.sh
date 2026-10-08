#!/usr/bin/env bash
# Regenerates apps/api/src/infrastructure/postgres/entities from the migrations.
#   entities.sh            rewrite the entities in place
#   entities.sh --check    regenerate into a temp dir and fail if it differs from the repo
# Migrations run into a throwaway schema so the result never depends on dev-database state.
set -euo pipefail
cd "$(dirname "$0")"

: "${DATABASE_URL:=postgres://nightfall:nightfall@localhost:5432/nightfall}"
export DATABASE_URL
ENTITIES=src/infrastructure/postgres/entities
SCHEMA="entities_gen_$$"

if ! command -v sea-orm-cli >/dev/null; then
  echo "sea-orm-cli not found. Install: cargo install sea-orm-cli --locked --no-default-features --features codegen,sqlx-postgres,runtime-tokio-rustls" >&2
  exit 1
fi

out="$ENTITIES"
if [[ "${1:-}" == "--check" ]]; then
  out="$(mktemp -d)"
fi

cleanup() { cargo run -q -p nightfall-api --bin nightfall-migrate -- drop "$SCHEMA" || true; }
trap cleanup EXIT

cargo run -q -p nightfall-api --bin nightfall-migrate -- up "$SCHEMA"
sea-orm-cli generate entity --database-url "$DATABASE_URL" --database-schema "$SCHEMA" \
  --output-dir "$out" --with-serde none --date-time-crate chrono

# The scratch schema name must not leak into the entities: tables resolve through search_path.
sed -i -E 's/schema_name = "[^"]*", //' "$out"/*.rs
rustfmt --edition 2021 --config-path ../../rustfmt.toml "$out"/*.rs

if [[ "${1:-}" == "--check" ]]; then
  if ! diff -ru "$ENTITIES" "$out"; then
    echo "entities are out of date: run 'moon run api:entities' and commit the result" >&2
    rm -rf "$out"
    exit 1
  fi
  rm -rf "$out"
fi
