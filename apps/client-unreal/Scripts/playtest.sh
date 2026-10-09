#!/usr/bin/env bash
# Scripted two-client playtest (E5.4): two game clients against a running API, each driven by the
# non-shipping `nf.Playtest` console command (Source/Nightfall/Game/PlaytestCommands.cpp).
#
#   hunter: attacks the nearest live keltir (the click path) until it has KILLS kills -> XP / level
#   victim: walks into the keltir meadow, never fights back, is killed, presses Respawn
#
# Needs the editor target built, the vendor art imported (else the bodies stay placeholders), and
# an API that accepts `test:<uuid>` dev tokens (AUTH_DEV_TOKENS=1) so the two clients are two fresh
# accounts. Each client logs in with -DevTokenFile (README "Developer bypass").
#
#   GRPC=localhost:50051 bash Scripts/playtest.sh [KILLS=3]
#
# Renders offscreen on the GPU (-RenderOffscreen) for screenshots; set NULLRHI=1 to run without a
# GPU (no screenshots). Logs, screenshots and a summary go to $OUT (default: a temp directory).
set -euo pipefail
: "${UE_ROOT:?UE_ROOT must point at the Unreal install}"
HERE="$(cd "$(dirname "$0")" && pwd)"
CLIENT="$(cd "$HERE/.." && pwd)"
PROJECT="$CLIENT/Nightfall.uproject"
GRPC="${GRPC:-localhost:50051}"
KILLS="${1:-3}"
OUT="${OUT:-$(mktemp -d --suffix=-playtest)}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"   # the clients run from the engine directory
RHI=(-RenderOffscreen -ResX=1280 -ResY=720 -windowed)
[ "${NULLRHI:-0}" = 1 ] && RHI=(-nullrhi)
COMMON=(-game -unattended -nosound -nosplash "-ini:Game:[/Script/Nightfall.NetSettings]:GrpcEndpoint=$GRPC")
echo "playtest: API $GRPC, output $OUT"

run_client() { # name token-file exec-cmds log [timeout]
  ( cd "$UE_ROOT/Engine/Binaries/Linux" && timeout "${5:-700}" ./UnrealEditor "$PROJECT" "${COMMON[@]}" "${RHI[@]}" \
      -DevTokenFile="$2" -ExecCmds="$3" -abslog="$4" >/dev/null 2>&1 ) || true
}

# One fresh account and character per role.
for ROLE in hunter victim; do
  ( umask 077 && printf 'test:%s' "$(cat /proc/sys/kernel/random/uuid)" >"$OUT/$ROLE.token" )
  NAME="$(tr -dc 'a-f' </proc/sys/kernel/random/uuid | cut -c1-6)"
  NAME="${ROLE^}${NAME}"
  NAME="${NAME:0:16}"
  (cd "$UE_ROOT/Engine/Binaries/Linux" && timeout 120 ./UnrealEditor "$PROJECT" "${COMMON[@]}" -nullrhi \
      -DevTokenFile="$OUT/$ROLE.token" -ExecCmds="nf.Login, nf.CreateCharacter $NAME 1" \
      -abslog="$OUT/$ROLE-create.log" >/dev/null 2>&1) &
  PID=$!
  for _ in $(seq 120); do
    grep -q "Display: nf.CreateCharacter:" "$OUT/$ROLE-create.log" 2>/dev/null && break
    sleep 1
  done
  kill "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
  grep "Display: nf.CreateCharacter:" "$OUT/$ROLE-create.log" | grep -q "ENetError::None" \
    || { grep "nf.CreateCharacter\|Unauthenticated" "$OUT/$ROLE-create.log" >&2; echo "playtest: could not create the $ROLE character (API needs AUTH_DEV_TOKENS=1)" >&2; exit 1; }
  echo "playtest: $ROLE character $NAME"
done

# The victim goes first so the hunter sees it arrive (late AOI entry of a player proxy).
run_client victim "$OUT/victim.token" "nf.Login, nf.EnterWorld, nf.Playtest victim" "$OUT/victim.log" &
VICTIM=$!
sleep 20
run_client hunter "$OUT/hunter.token" "nf.Login, nf.EnterWorld, nf.Playtest hunter $KILLS" "$OUT/hunter.log" &
HUNTER=$!
wait "$HUNTER" "$VICTIM"

SHOTS="$CLIENT/Saved/Screenshots/LinuxEditor"   # -game under UnrealEditor writes here
[ -d "$SHOTS" ] && find "$SHOTS" -name 'playtest-*' -newer "$OUT/hunter.token" -exec cp {} "$OUT/" \;
for ROLE in hunter victim; do
  echo "== $ROLE"
  grep -hE "playtest: (KILL|XP|LEVEL|OWN|after respawn|anim |RESULT|pressing)" "$OUT/$ROLE.log" | sed -n 's/^.*playtest: /  /; 1,120p' || true
done | tee "$OUT/summary.txt"
grep -q "playtest: RESULT PASS" "$OUT/hunter.log" && grep -q "playtest: RESULT PASS" "$OUT/victim.log"
