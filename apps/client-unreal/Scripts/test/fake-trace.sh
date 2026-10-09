#!/usr/bin/env bash
# Stand-in for the offline trace subcommand.
set -euo pipefail
while (($#)); do
  if [[ "$1" == --out ]]; then printf '<!doctype html><title>Trace</title>' > "$2"; exit 0; fi
  shift
done
exit 2
