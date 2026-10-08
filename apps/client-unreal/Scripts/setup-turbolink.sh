#!/usr/bin/env bash
# Makes Plugins/TurboLink buildable after a fresh clone. Idempotent and fast on re-runs.
#
#   1. Checks out the TurboLink submodule (pinned to v1.4.2).
#   2. Installs TurboLink's prebuilt gRPC 1.57 / protobuf 23.4 / abseil / re2 static libraries into
#      Plugins/TurboLink/Source/ThirdParty. They are not in the TurboLink git repo, only in its
#      release zip (~900 MB download, cached in ~/.cache/nightfall, pinned by sha256).
#   3. Symlinks our generated code (Source/Nightfall/Generated) and the hand-written wire bridge
#      (Source/Nightfall/GrpcBridge) into the TurboLinkGrpc module. TurboLink's generated classes
#      must compile inside that module: they use its private headers, its private protobuf/gRPC
#      include paths, and reflection paths hard-coded to /Script/TurboLinkGrpc. Both folders carry
#      a .ubtignore so the Nightfall module does not compile them a second time.
#
# None of this touches tracked files in the submodule; .gitmodules sets ignore = untracked.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CLIENT="$(cd "$HERE/.." && pwd)"
PLUGIN="$CLIENT/Plugins/TurboLink"
MODULE="$PLUGIN/Source/TurboLinkGrpc"

LIBS_URL=https://github.com/thejinchao/turbolink/releases/download/v1.4.2/TurboLink.UE56.zip
LIBS_SHA256=9ad1ecd9e64d4166fff973bcb5b3f493756987a99f2d3c2e9933b9c2908eaf39
CACHE="${NIGHTFALL_CACHE:-$HOME/.cache/nightfall}"

# --- 1. submodule ---------------------------------------------------------------------------
if [ ! -f "$PLUGIN/TurboLink.uplugin" ]; then
  git -C "$CLIENT" submodule update --init -- Plugins/TurboLink
fi

# --- 2. third-party libraries ---------------------------------------------------------------
case "$(uname -s)" in
  Linux) LIB_PLATFORM=linux ;;
  Darwin) LIB_PLATFORM=mac ;;
  *) LIB_PLATFORM=win64 ;;
esac
MARKER="$PLUGIN/Source/ThirdParty/.installed-$LIBS_SHA256-$LIB_PLATFORM"
if [ ! -f "$MARKER" ]; then
  mkdir -p "$CACHE"
  ZIP="$CACHE/TurboLink.UE56.zip"
  if [ ! -f "$ZIP" ] || ! echo "$LIBS_SHA256  $ZIP" | sha256sum -c --status; then
    echo "downloading TurboLink third-party libraries (~900 MB) ..."
    curl -fL --progress-bar -o "$ZIP.part" "$LIBS_URL"
    echo "$LIBS_SHA256  $ZIP.part" | sha256sum -c --status || { echo "sha256 mismatch for $LIBS_URL" >&2; exit 1; }
    mv "$ZIP.part" "$ZIP"
  fi
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT
  unzip -q "$ZIP" 'Source/ThirdParty/*/include/*' "Source/ThirdParty/*/lib/$LIB_PLATFORM/*" -d "$TMP"
  for lib in "$TMP"/Source/ThirdParty/*; do
    rm -rf "$PLUGIN/Source/ThirdParty/$(basename "$lib")"
    mv "$lib" "$PLUGIN/Source/ThirdParty/"
  done
  rm -f "$PLUGIN"/Source/ThirdParty/.installed-*
  touch "$MARKER"
fi

# --- 3. link generated code into the TurboLinkGrpc module -----------------------------------
# Drop links from a previous run whose targets are gone (e.g. a removed .proto).
find "$MODULE/Public" "$MODULE/Private" -maxdepth 1 -type l ! -exec test -e {} \; -delete
for src in "$CLIENT/Source/Nightfall/Generated" "$CLIENT/Source/Nightfall/GrpcBridge"; do
  for side in Public Private; do
    [ -d "$src/$side" ] || continue
    for entry in "$src/$side"/*; do
      [ -e "$entry" ] || continue
      ln -sfnr "$entry" "$MODULE/$side/$(basename "$entry")"
    done
  done
done
