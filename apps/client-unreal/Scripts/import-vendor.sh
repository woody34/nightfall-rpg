#!/usr/bin/env bash
# Imports the Phase 1 vendor art headless with Scripts/import_vendor.py into Content/Vendor
# (git-ignored, like Vendor/). Needs the editor target built and the owner's downloads in
#   ${NIGHTFALL_VENDOR_DIR:-Vendor/Downloads}/{Cockatrice,NatureLite}
# In a git worktree without its own Vendor/, the main checkout's Vendor/Downloads is used.
# Overwrites Content/Vendor/{Cockatrice,NatureLite,Mannequins}. See THIRD_PARTY_ASSETS.md.
set -euo pipefail
: "${UE_ROOT:?UE_ROOT must point at the Unreal install}"
HERE="$(cd "$(dirname "$0")" && pwd)"
CLIENT="$(cd "$HERE/.." && pwd)"
PROJECT="$CLIENT/Nightfall.uproject"
CONTENT="$CLIENT/Content"

VENDOR="${NIGHTFALL_VENDOR_DIR:-$CLIENT/Vendor/Downloads}"
if [ ! -d "$VENDOR" ]; then
  MAIN="$(cd "$(git -C "$CLIENT" rev-parse --git-common-dir)/.." && pwd)/apps/client-unreal/Vendor/Downloads"
  [ -d "$MAIN" ] && VENDOR="$MAIN"
fi
for f in Cockatrice/cockatrice.fbx Cockatrice/coctrice.textures.zip; do
  [ -f "$VENDOR/$f" ] || { echo "no $VENDOR/$f (see THIRD_PARTY_ASSETS.md); nothing deleted" >&2; exit 1; }
done
VENDOR="$(cd "$VENDOR" && pwd)"   # the editor runs from the engine directory
echo "vendor downloads: $VENDOR"

# The script owns these folders. Delete them before the editor starts so its asset registry never
# sees the old assets; a rerun therefore reproduces the same packages.
rm -rf "$CONTENT/Vendor/Cockatrice" "$CONTENT/Vendor/NatureLite" "$CONTENT/Vendor/Mannequins" "$CONTENT/Characters/Mannequins"
rmdir "$CONTENT/Characters" 2>/dev/null || true

# Epic's template Manny (Unreal Engine EULA "Examples"): copy the packages the player needs at
# their template path, /Game/Characters/Mannequins, so their internal references resolve; the
# script then renames the folder to /Game/Vendor/Mannequins, which rewrites those references.
TEMPLATE="$UE_ROOT/Templates/TemplateResources/High/Characters/Content/Mannequins"
MANNY_STAGED=0
if [ -d "$TEMPLATE" ]; then
  DEST="$CONTENT/Characters/Mannequins"
  for rel in Meshes/SKM_Manny_Simple Meshes/SK_Mannequin Rigs/PA_Mannequin \
             Materials/M_Mannequin Materials/Manny/MI_Manny_01_New Materials/Manny/MI_Manny_02_New \
             Textures/Manny/T_Manny_01_BN Textures/Manny/T_Manny_01_D Textures/Manny/T_Manny_01_MRA \
             Textures/Manny/T_Manny_02_BN Textures/Manny/T_Manny_02_D Textures/Manny/T_Manny_02_MRA \
             Textures/Manny/T_Manny_02_N Textures/Shared/T_UE_Logo_M \
             Anims/Unarmed/MM_Idle Anims/Unarmed/Walk/MF_Unarmed_Walk_Fwd Anims/Unarmed/Jog/MF_Unarmed_Jog_Fwd \
             Anims/Unarmed/Attack/MM_Attack_01 Anims/Death/MM_Death_Front_01; do
    mkdir -p "$DEST/$(dirname "$rel")"
    cp "$TEMPLATE/$rel.uasset" "$DEST/$rel.uasset"
  done
  chmod -R u+w "$DEST"   # the engine install is read-only; the rename rewrites these packages
  MANNY_STAGED=1
else
  echo "no template Manny at $TEMPLATE; the player keeps its placeholder body" >&2
fi

LOG="$(realpath -m "${IMPORT_VENDOR_LOG:-$(mktemp --suffix=.log)}")"
[ -n "${IMPORT_VENDOR_LOG:-}" ] || trap 'rm -f "$LOG"' EXIT
cd "$UE_ROOT/Engine/Binaries/Linux"
# Full editor (-nullrhi) like create-content.sh; success is the script's marker, not the exit code.
NIGHTFALL_VENDOR_DIR="$VENDOR" NIGHTFALL_MANNY_STAGED="$MANNY_STAGED" \
./UnrealEditor "$PROJECT" /Engine/Maps/Entry -ExecutePythonScript="$HERE/import_vendor.py" \
  -EnablePlugins=PythonScriptPlugin,EditorScriptingUtilities \
  -unattended -nullrhi -nosplash -nosound -abslog="$LOG" >/dev/null 2>&1 || true
grep -E "import_vendor|LogPython: Error" "$LOG" || true
rm -rf "$CONTENT/Characters/Mannequins"
rmdir "$CONTENT/Characters" 2>/dev/null || true
grep -q "import_vendor: done" "$LOG"
