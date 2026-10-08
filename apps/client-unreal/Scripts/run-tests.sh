#!/usr/bin/env bash
# Runs the Nightfall automation tests headless in the editor. Exit code is non-zero on failure.
# Nightfall.Net.SessionClient.Ping talks to the real API: start it first (`moon run api:dev`).
# Usage: Scripts/run-tests.sh [test filter, default "Nightfall"]
set -euo pipefail
: "${UE_ROOT:?UE_ROOT must point at the Unreal install}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)/Nightfall.uproject"
FILTER="${1:-Nightfall}"
# This engine build resolves Engine/Content relative to the working directory.
cd "$UE_ROOT/Engine/Binaries/Linux"
exec ./UnrealEditor "$PROJECT" -ExecCmds="Automation RunTests $FILTER; Quit" \
  -TestExit="Automation Test Queue Empty" -unattended -nullrhi -nosplash -nosound -log -stdout
