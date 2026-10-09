#!/usr/bin/env bash
# Non-mutating prerequisite check; --video checks optional nightly diagnostics dependencies.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
. "$repo/apps/client-unreal/Scripts/sim-lib.sh"
sim_resolve_env
if [[ -n "${GITHUB_ENV:-}" ]]; then
  printf 'UE_ROOT=%s\nLINUX_MULTIARCH_ROOT=%s\n' "$UE_ROOT" "$LINUX_MULTIARCH_ROOT" >> "$GITHUB_ENV"
fi
failed=0
require() { if ! "$@"; then echo "runner: missing prerequisite: $*" >&2; failed=1; fi; }
for command in docker curl python3 timeout git cargo moon; do require command -v "$command"; done
require docker compose version
require docker info --format '{{.ServerVersion}}'
require test -x "$UE_ROOT/Engine/Build/BatchFiles/Linux/Build.sh"
require test -x "$UE_ROOT/Engine/Binaries/Linux/UnrealEditor"
require test -d "$LINUX_MULTIARCH_ROOT"
python3 - "$UE_ROOT/Engine/Build/Build.version" <<'PY' || failed=1
import json, sys
version = json.load(open(sys.argv[1]))
actual = tuple(version[key] for key in ('MajorVersion', 'MinorVersion', 'PatchVersion'))
if actual != (5, 8, 3):
    raise SystemExit(f'runner: UE 5.8.3 required, found {actual}')
PY
if [[ "${1:-}" == --video ]]; then
  for command in xvfb-run ffmpeg xauth; do require command -v "$command"; done
  require test -f "${SIM_VIDEO_ICD:-/usr/share/vulkan/icd.d/lvp_icd.json}"
fi
((failed == 0)) && echo 'runner: prerequisites present; acceptance builds/tests still required'
exit "$failed"
