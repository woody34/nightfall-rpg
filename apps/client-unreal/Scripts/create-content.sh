#!/usr/bin/env bash
# Regenerates the Story 6.2 content (Blueprints, input assets, maps) headless with
# Scripts/create_content.py. Needs the editor target built. Overwrites Content/{Input,Blueprints,UI,Maps}.
set -euo pipefail
: "${UE_ROOT:?UE_ROOT must point at the Unreal install}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PROJECT="$(cd "$HERE/.." && pwd)/Nightfall.uproject"
CONTENT="$(cd "$HERE/.." && pwd)/Content"
# These folders belong to the script. Delete them before the editor starts so its asset registry
# never sees the old assets (it refuses to create a map or asset over one it knows about).
rm -rf "$CONTENT/Input" "$CONTENT/Blueprints" "$CONTENT/UI" "$CONTENT/Maps"
LOG="${CREATE_CONTENT_LOG:-$(mktemp --suffix=.log)}"
[ -n "${CREATE_CONTENT_LOG:-}" ] || trap 'rm -f "$LOG"' EXIT
# Full editor, not the pythonscript commandlet: level actor spawning needs it. The script quits
# the editor when done. This engine build resolves Engine/Content relative to the working directory.
cd "$UE_ROOT/Engine/Binaries/Linux"
# Start on an engine map so the editor never loads the maps the script recreates. The editor
# sometimes crashes in static teardown after "LogExit: Exiting." (after everything is saved), so
# success is the script's marker in the log, not the exit code.
./UnrealEditor "$PROJECT" /Engine/Maps/Entry -ExecutePythonScript="$HERE/create_content.py" \
  -EnablePlugins=PythonScriptPlugin,EditorScriptingUtilities \
  -unattended -nullrhi -nosplash -nosound -abslog="$LOG" >/dev/null 2>&1 || true
grep -E "create_content|LogPython: Error" "$LOG" || true
grep -q "create_content: done" "$LOG"
