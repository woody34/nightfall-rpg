#!/usr/bin/env bash
# Builds the Linux code generators TurboLink needs. TurboLink only ships them as Windows .exe files.
#
#   protoc 23.4 + grpc_cpp_plugin 1.57.0  built from grpc v1.57.0, which is the exact gRPC and protobuf
#                                         TurboLink v1.4.2's prebuilt libraries were compiled from
#                                         (protobuf generated code refuses to compile against a
#                                         different runtime version).
#   protoc-gen-turbolink                  built from source with the .NET SDK bundled in Unreal.
#
# Output: $NIGHTFALL_PROTO_TOOLS/bin (default ~/.cache/nightfall/proto-tools/bin). Takes ~10 minutes
# on first run, needs git, a C++ compiler, python3-venv and UE_ROOT. Re-running is a no-op.
set -euo pipefail

GRPC_TAG=v1.57.0
TURBOLINK_GEN_REPO=https://github.com/thejinchao/protoc-gen-turbolink
TURBOLINK_GEN_COMMIT=184d0f1   # v2.7.0, the generator shipped in TurboLink v1.4.2

TOOLS="${NIGHTFALL_PROTO_TOOLS:-$HOME/.cache/nightfall/proto-tools}"
BIN="$TOOLS/bin"
SRC="$TOOLS/src"
mkdir -p "$BIN" "$SRC"

: "${UE_ROOT:?UE_ROOT must point at the Unreal install (its bundled .NET SDK builds protoc-gen-turbolink)}"
DOTNET_ROOT="$(ls -d "$UE_ROOT"/Engine/Binaries/ThirdParty/DotNet/*/linux-x64 | sort -V | tail -1)"
export DOTNET_ROOT DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1
DOTNET="$DOTNET_ROOT/dotnet"

# --- cmake + ninja (from PyPI, so no system packages are needed) -----------------------------
if [ ! -x "$TOOLS/venv/bin/cmake" ] || "$TOOLS/venv/bin/cmake" --version | grep -q "version 4"; then
  python3 -m venv "$TOOLS/venv"
  "$TOOLS/venv/bin/pip" install --quiet 'cmake<4' ninja   # grpc 1.57 predates CMake 4's policy floor
fi
export PATH="$TOOLS/venv/bin:$PATH"

# --- protoc + grpc_cpp_plugin ----------------------------------------------------------------
if [ ! -x "$BIN/protoc" ] || [ ! -x "$BIN/grpc_cpp_plugin" ]; then
  if [ ! -d "$SRC/grpc" ]; then
    git clone --quiet --depth 1 --branch "$GRPC_TAG" --recurse-submodules --shallow-submodules \
      https://github.com/grpc/grpc "$SRC/grpc"
  fi
  cmake -S "$SRC/grpc" -B "$SRC/grpc/build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
    -DgRPC_BUILD_TESTS=OFF -DgRPC_BUILD_CSHARP_EXT=OFF \
    -Dprotobuf_BUILD_TESTS=OFF -DABSL_PROPAGATE_CXX_STD=ON -DgRPC_SSL_PROVIDER=module \
    > "$SRC/grpc-configure.log"
  cmake --build "$SRC/grpc/build" --target protoc grpc_cpp_plugin
  cp "$SRC/grpc/build/third_party/protobuf/protoc" "$BIN/protoc"
  cp "$SRC/grpc/build/grpc_cpp_plugin" "$BIN/grpc_cpp_plugin"
  # Well-known types (google/protobuf/*.proto) for imports.
  rm -rf "$TOOLS/include" && mkdir -p "$TOOLS/include"
  cp -r "$SRC/grpc/third_party/protobuf/src/google" "$TOOLS/include/"
  find "$TOOLS/include" -type f ! -name '*.proto' -delete
fi

# --- protoc-gen-turbolink --------------------------------------------------------------------
if [ ! -x "$BIN/protoc-gen-turbolink" ]; then
  GEN="$SRC/protoc-gen-turbolink"
  if [ ! -d "$GEN" ]; then
    git clone --quiet "$TURBOLINK_GEN_REPO" "$GEN"
  fi
  git -C "$GEN" checkout --quiet "$TURBOLINK_GEN_COMMIT"

  # The upstream project is a .NET Framework 4.6.2 csproj whose templates are T4 files preprocessed
  # by Visual Studio. Preprocess them with dotnet-t4 and build an SDK-style project instead.
  "$DOTNET" tool update --tool-path "$TOOLS/dotnet-tools" dotnet-t4 --version 3.0.0 > /dev/null
  for tt in "$GEN"/Template/*.tt; do
    name="$(basename "$tt" .tt)"
    "$TOOLS/dotnet-tools/t4" -c "protoc_gen_turbolink.Template.$name" -o "$GEN/Template/$name.cs" "$tt"
    # TemplateModel.cs declares the other half of each class as internal.
    sed -i 's/public partial class /partial class /' "$GEN/Template/$name.cs"
  done

  NET_TFM="net$(basename "$(dirname "$DOTNET_ROOT")")"   # e.g. net10.0
  cat > "$GEN/linux.csproj" <<EOF
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>$NET_TFM</TargetFramework>
    <AssemblyName>protoc-gen-turbolink</AssemblyName>
    <RootNamespace>protoc_gen_turbolink</RootNamespace>
    <Nullable>disable</Nullable>
    <GenerateAssemblyInfo>false</GenerateAssemblyInfo>
    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
    <NoWarn>\$(NoWarn);CS0618;CS8981</NoWarn>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="*.cs;Properties/*.cs;Template/*.cs" />
    <PackageReference Include="Google.Protobuf" Version="3.19.0" />
    <PackageReference Include="System.CodeDom" Version="8.0.0" />
  </ItemGroup>
</Project>
EOF
  "$DOTNET" publish "$GEN/linux.csproj" -c Release -o "$TOOLS/turbolink-gen" > "$SRC/turbolink-gen-build.log"
  cat > "$BIN/protoc-gen-turbolink" <<EOF
#!/usr/bin/env bash
DOTNET_ROOT="$DOTNET_ROOT" exec "$DOTNET" "$TOOLS/turbolink-gen/protoc-gen-turbolink.dll" "\$@"
EOF
  chmod +x "$BIN/protoc-gen-turbolink"
fi

"$BIN/protoc" --version
echo "proto tools ready in $BIN"
