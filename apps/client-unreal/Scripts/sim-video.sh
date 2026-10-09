#!/usr/bin/env bash
# Render one diagnostic retry. Exit/report never replaces the original headless verdict.
# Usage: sim-video.sh SCENARIO OUTPUT_DIRECTORY [GROUP_ID]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/sim-lib.sh"
scenario="${1:?scenario required}"
dest="${2:?output directory required}"
mkdir -p "$dest"
dest="$(cd "$dest" && pwd)"
for command in xvfb-run ffmpeg python3; do
  command -v "$command" >/dev/null || sim_die "video requires $command"
done
icd="${SIM_VIDEO_ICD:-/usr/share/vulkan/icd.d/lvp_icd.x86_64.json}"
[[ -f "$icd" ]] || sim_die "Mesa lavapipe ICD not found: $icd"
sim_resolve_env
sim_ensure_build
sim_bot_cmd "$scenario"
# Preserve scenario/group identity for the retry; all headless-only flags are removed.
args=()
for arg in "${SIM_BOT_CMD[@]}"; do [[ "$arg" == -nullrhi ]] || args+=("$arg"); done
[[ -z "${3:-}" ]] || args+=("-SimGroup=$3")
args+=(-vulkan -RenderOffscreen -DumpMovie -ForceRes -ResX=960 -ResY=540 -FPS=15 -NoVSync)
# UE's GameScreenshotSaveDirectory drives -DumpMovie output. Config override avoids stale frames
# and keeps the frames for this retry separate even on a persistent runner.
frames="$dest/frames"
mkdir -p "$frames"
args+=("-ini:Engine:[/Script/Engine.Engine]:GameScreenshotSaveDirectory=(Path=\"$frames\")")
marker="$(sim_new_marker)"
code=0
(cd "$(sim_bot_cwd)" && VK_ICD_FILENAMES="$icd" xvfb-run -a -s '-screen 0 960x540x24' \
  timeout --kill-after=10 "${SIM_VIDEO_TIMEOUT:-110}" "${args[@]}") >"$dest/stdout.log" 2>&1 || code=$?
sim_collect "$marker" "$dest"
rm -f "$marker"
echo "$code" >"$dest/retry-exit-code.txt"
# DumpMovie frame names may have gaps; a concat manifest handles png/bmp/exr without assuming
# the engine's numbering. The standalone retry JUnit is deliberately kept in dest/video/.
python3 - "$frames" "$dest/frames.txt" <<'PY'
from pathlib import Path
import sys
files = sorted(p for p in Path(sys.argv[1]).glob('MovieFrame*') if p.suffix.lower() in ('.png', '.bmp', '.exr'))
if not files:
    raise SystemExit('video retry produced no DumpMovie frames; see stdout.log')
with open(sys.argv[2], 'w') as output:
    for file in files:
        output.write("file '" + str(file).replace("'", "'\\''") + "'\n")
        output.write('duration 0.066666667\n')
    output.write("file '" + str(files[-1]).replace("'", "'\\''") + "'\n")
PY
ffmpeg -hide_banner -loglevel warning -y -safe 0 -f concat -i "$dest/frames.txt" \
  -vf 'scale=960:540:force_original_aspect_ratio=decrease,pad=960:540:(ow-iw)/2:(oh-ih)/2,format=yuv420p' \
  -r 15 -c:v libx264 -movflags +faststart "$dest/$(basename "$scenario" .nfs).mp4" >"$dest/ffmpeg.log" 2>&1
sim_log "diagnostic video: $dest/$(basename "$scenario" .nfs).mp4 (retry exit $code)"
