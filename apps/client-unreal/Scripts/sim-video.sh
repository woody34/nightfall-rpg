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
[[ -z "$(find "$dest" -mindepth 1 -maxdepth 1 -print -quit)" ]] || sim_die "video destination must be empty; previous evidence retained"
for command in xvfb-run ffmpeg python3; do
  command -v "$command" >/dev/null || sim_die "video requires $command"
done
icd="${SIM_VIDEO_ICD:-/usr/share/vulkan/icd.d/lvp_icd.json}"
[[ -f "$icd" ]] || sim_die "Mesa lavapipe ICD not found: $icd"
sim_resolve_env
sim_ensure_build
sim_bot_cmd "$scenario"
# Preserve scenario/group identity for the retry; all headless-only flags are removed.
args=()
for arg in "${SIM_BOT_CMD[@]}"; do [[ "$arg" == -nullrhi ]] || args+=("$arg"); done
[[ -z "${3:-}" ]] || args+=("-SimGroup=$3")
args+=(-vulkan -sm5 -AllowSoftwareRendering -RenderOffscreen -DumpMovie -ForceRes -ResX=960 -ResY=540 -NoVSync)
# Lavapipe does not satisfy UE's SM6 profile. Force SM5 for the diagnostic retry; disable
# renderer features requiring SM6 without changing any networking/gameplay configuration.
args+=("-ini:Engine:[/Script/Engine.RendererSettings]:r.DynamicGlobalIlluminationMethod=0,r.ReflectionMethod=0,r.Shadow.Virtual.Enable=0,r.Nanite.ProjectEnabled=False")
# UE's GameScreenshotSaveDirectory drives -DumpMovie output. Config override avoids stale frames
# and keeps the frames for this retry separate even on a persistent runner.
frames="$dest/frames"
mkdir -p "$frames"
args+=("-ini:Engine:[/Script/Engine.Engine]:GameScreenshotSaveDirectory=(Path=\"$frames\")")
marker="$(sim_new_marker)"
code=0
capture_start="$(python3 -c 'import time; print(time.time_ns())')"
(cd "$(sim_bot_cwd)" && VK_ICD_FILENAMES="$icd" xvfb-run -a -s '-screen 0 960x540x24' \
  timeout --kill-after=10 "${SIM_VIDEO_TIMEOUT:-110}" "${args[@]}") >"$dest/stdout.log" 2>&1 || code=$?
capture_end="$(python3 -c 'import time; print(time.time_ns())')"
sim_collect "$marker" "$dest"
rm -f "$marker"
echo "$code" >"$dest/retry-exit-code.txt"
# DumpMovie frame names may have gaps; a concat manifest handles png/bmp/exr without assuming
# the engine's numbering. The standalone retry JUnit is deliberately kept in dest/video/.
python3 - "$frames" "$dest/frames.txt" "$capture_start" "$capture_end" <<'PY'
from pathlib import Path
import json
import sys
files = sorted((p for p in Path(sys.argv[1]).glob('MovieFrame*') if p.suffix.lower() in ('.png', '.bmp', '.exr')),
               key=lambda p: (p.stat().st_mtime_ns, p.name))
if not files:
    raise SystemExit('video retry produced no DumpMovie frames; see stdout.log')
start, end = map(int, sys.argv[3:5])
times = [file.stat().st_mtime_ns for file in files]
if end <= start or any(t < start or t > end for t in times):
    raise SystemExit('frame timestamps fall outside capture wall clock bounds')
# Hold the first frame across startup and the last through shutdown. Interior frame changes
# follow file mtimes. Equal timestamps are collapsed to the last frame written at that instant.
frames = []
for file, timestamp in zip(files, times):
    if frames and frames[-1][1] == timestamp:
        frames[-1] = (file, timestamp)
    else:
        frames.append((file, timestamp))
durations = []
for i, (file, timestamp) in enumerate(frames):
    following = frames[i + 1][1] if i + 1 < len(frames) else end
    durations.append(following - (start if i == 0 else timestamp))
Path(sys.argv[2]).with_name('frame-timing.json').write_text(json.dumps({
    'capture_start_ns': start, 'capture_end_ns': end,
    'frames': [{'file': str(f), 'mtime_ns': t, 'duration_ns': d}
               for (f, t), d in zip(frames, durations)]}, indent=2) + '\n')
with open(sys.argv[2], 'w') as output:
    for (file, _), duration in zip(frames, durations):
        output.write("file '" + str(file).replace("'", "'\\''") + "'\n")
        output.write('option framerate 1000\n')
        output.write(f'duration {duration / 1_000_000_000:.9f}\n')
    output.write("file '" + str(frames[-1][0]).replace("'", "'\\''") + "'\n")
    output.write('option framerate 1000\n')
PY
ffmpeg -hide_banner -loglevel warning -y -safe 0 -f concat -i "$dest/frames.txt" \
  -vf 'scale=960:540:force_original_aspect_ratio=decrease,pad=960:540:(ow-iw)/2:(oh-ih)/2,format=yuv420p' \
  -fps_mode vfr -c:v libx264 -movflags +faststart "$dest/$(basename "$scenario" .nfs).mp4" >"$dest/ffmpeg.log" 2>&1
sim_log "diagnostic video: $dest/$(basename "$scenario" .nfs).mp4 (retry exit $code)"
