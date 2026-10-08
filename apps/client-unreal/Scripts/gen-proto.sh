#!/usr/bin/env bash
# Generates C++ protobuf code for the Unreal client from packages/proto.
# Requires protoc (https://github.com/protocolbuffers/protobuf/releases). Output is committed so
# the editor builds without protoc installed. Until the vendored protobuf-lite ThirdParty module
# exists, generated code is NOT compiled (see Net/ProtoCodec.h); this script prepares for it.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
PROTO_ROOT="$HERE/../../../packages/proto"
OUT="$HERE/../Source/Nightfall/Generated"
mkdir -p "$OUT"
protoc -I "$PROTO_ROOT" --cpp_out=lite:"$OUT" \
  "$PROTO_ROOT/nightfall/v1/game.proto" \
  "$PROTO_ROOT/nightfall/v1/world.proto"
echo "generated into $OUT"
