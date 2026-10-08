#!/usr/bin/env bash
# Generates the Unreal client's gRPC and protobuf code from packages/proto with TurboLink's
# generator (protoc-gen-turbolink) plus protoc's C++ and gRPC C++ outputs, which TurboLink wraps.
#
# Output goes to Source/Nightfall/Generated and is committed, so the editor builds without any of
# these tools installed. Run this after changing a .proto; Scripts/install-proto-tools.sh builds
# the tools on first use (Linux; ~10 minutes once).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CLIENT="$(cd "$HERE/.." && pwd)"
PROTO_ROOT="$(cd "$CLIENT/../../packages/proto" && pwd)"
OUT="$CLIENT/Source/Nightfall/Generated"
TOOLS="${NIGHTFALL_PROTO_TOOLS:-$HOME/.cache/nightfall/proto-tools}"
FIX_DIR="$CLIENT/Plugins/TurboLink/Tools"

if [ ! -x "$TOOLS/bin/protoc" ] || [ ! -x "$TOOLS/bin/grpc_cpp_plugin" ] || [ ! -x "$TOOLS/bin/protoc-gen-turbolink" ]; then
  bash "$HERE/install-proto-tools.sh"
fi

# Every .proto under nightfall/, relative to PROTO_ROOT: TurboLink names its output after the
# import path, so protoc must run from the proto root.
mapfile -t PROTOS < <(cd "$PROTO_ROOT" && find nightfall -name '*.proto' | sort)

rm -rf "$OUT/Public" "$OUT/Private"
mkdir -p "$OUT/Private/pb"
(
  cd "$PROTO_ROOT"
  "$TOOLS/bin/protoc" -I "$TOOLS/include" -I . \
    --cpp_out="$OUT/Private/pb" \
    --plugin=protoc-gen-grpc="$TOOLS/bin/grpc_cpp_plugin" --grpc_out="$OUT/Private/pb" \
    --plugin=protoc-gen-turbolink="$TOOLS/bin/protoc-gen-turbolink" --turbolink_out="$OUT" \
    --turbolink_opt="GenerateJsonCode=true" \
    "${PROTOS[@]}"
)

# Same post-processing as TurboLink's generate_code.cmd: prepend its MSVC warning suppressions to
# protoc's output so Windows builds stay quiet. No-ops under clang.
while IFS= read -r -d '' f; do
  case "$f" in
    *.pb.h) fix="$FIX_DIR/fix_proto_h.txt" ;;
    *) fix="$FIX_DIR/fix_proto_cpp.txt" ;;
  esac
  { tr -d '\r' < "$fix"; cat "$f"; } > "$f.tmp" && mv "$f.tmp" "$f"
done < <(find "$OUT/Private/pb" \( -name '*.pb.h' -o -name '*.pb.cc' \) ! -name '*.grpc.pb.*' -print0)

bash "$HERE/setup-turbolink.sh"
echo "generated $(find "$OUT" -type f ! -name .ubtignore | wc -l) files into $OUT"
