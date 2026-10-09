#!/usr/bin/env bash
# Cook and stage the Development Linux game into Saved/StagedBuilds/Linux.
# Development is required for AUTH_DEV_TOKENS and BotScenario; Shipping disables both.
# -nullrhi makes the cook headless. Staged clients separately receive -nullrhi from Gauntlet.
# Loose staged content (-skipiostore, packaging UsePakFile=false) needs no pak signing keys.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/sim-lib.sh"
sim_resolve_env
bash "$HERE/setup-turbolink.sh"
"$UE_ROOT/Engine/Build/BatchFiles/RunUAT.sh" BuildCookRun \
  -project="$SIM_PROJECT_DIR/Nightfall.uproject" -noP4 -utf8output -unattended \
  -targetplatform=Linux -clientconfig=Development -build -cook -stage \
  -stagingdirectory="$SIM_PROJECT_DIR/Saved/StagedBuilds" \
  -map=/Game/Maps/L_Login+/Game/Maps/L_TestZone -nullrhi -nosound \
  -skipiostore -ini:Game:[/Script/UnrealEd.ProjectPackagingSettings]:bUsePakFile=False \
  -ini:Game:[/Script/UnrealEd.ProjectPackagingSettings]:bUseIoStore=False "$@"
