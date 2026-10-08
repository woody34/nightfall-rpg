#pragma once

// Binary protobuf encode/decode for the WebSocket envelopes in world.proto.
//
// Compiled into the TurboLinkGrpc module (Scripts/setup-turbolink.sh links it there) because the
// protobuf runtime and the protoc-generated nightfall::v1 classes are private to that module. The
// Nightfall module must not link its own copy of protobuf/gRPC, so it reaches the generated
// classes only through these two exported functions and TurboLink's generated USTRUCTs.

#include "CoreMinimal.h"
#include "SNightfallV1/WorldMessage.h"

namespace NightfallWire
{
	/** Serialises nightfall.v1.ClientMessage. An Intent whose active case has no payload is omitted. */
	TURBOLINKGRPC_API void EncodeClientMessage(const FGrpcNightfallV1ClientMessage& In, TArray<uint8>& Out);

	/** Parses nightfall.v1.ServerMessage. Returns false on malformed input; Out is reset first. */
	TURBOLINKGRPC_API bool DecodeServerMessage(const uint8* Data, int32 Size, FGrpcNightfallV1ServerMessage& Out);
}
