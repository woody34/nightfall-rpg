#!/usr/bin/env bash
# Fake both replay check (legacy single path) and `coverage --file PATH --out PATH`.
set -euo pipefail
export PYTHONDONTWRITEBYTECODE=1
if [[ "${1:-}" == coverage ]]; then
  shift
  file=""; out=""
  while (($#)); do
    case "$1" in --file) file="$2"; shift 2 ;; --out) out="$2"; shift 2 ;; *) exit 2 ;; esac
  done
  python3 - "$file" "$out" "$(dirname "$0")" <<'PYCODE'
import json, pathlib, sys
sys.path.insert(0, sys.argv[3])
import importlib.util
spec = importlib.util.spec_from_file_location('fake_artifacts', sys.argv[3] + '/fake-artifacts.py')
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
result, amount = pathlib.Path(sys.argv[1]).read_text().split()
if result == 'nocoverage':
    sys.exit(0)  # A lying/silent tool is still a gate failure.
if result == 'coveragefail':
    sys.exit(2)
data = fixture.transition_fixture(int(amount))
if result == 'badcoverage':
    next(row for row in data['npc_intentions'] if not row['reachable'])['count'] = 1
pathlib.Path(sys.argv[2]).write_text(json.dumps(data))
PYCODE
else
  [[ "${1:-}" != check ]] || { shift; [[ "$1" == --file ]]; shift; }
  ! grep -Eqi 'DIVERGE' "$1"
fi
