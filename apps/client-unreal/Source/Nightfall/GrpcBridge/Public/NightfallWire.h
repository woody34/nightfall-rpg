#pragma once

// Binary protobuf encode/decode for the WebSocket envelopes in world.proto.
//
// Compiled into the TurboLinkGrpc module (Scripts/setup-turbolink.sh links it there) because the
// protobuf runtime and the protoc-generated nightfall::v1 classes are private to that module. The
// Nightfall module must not link its own copy of protobuf/gRPC, so it reaches the generated
// classes only through these two exported functions and TurboLink's generated USTRUCTs.

#include "CoreMinimal.h"
#include "SNightfallV1/WorldMessage.h"
#include "SNightfallV1/GameMessage.h"

#include "SNightfallV1/ItemsMessage.h"

namespace NightfallWire
{
 TURBOLINKGRPC_API void ItemReceiptBytes(const uint8* Data,int32 Size,TArray<uint8>& Out);
 TURBOLINKGRPC_API void EncodeItemReceipt(const FGrpcNightfallV1EnchantResult& In,TArray<uint8>& Out);
 TURBOLINKGRPC_API void EncodeItemReceipt(const FGrpcNightfallV1RefineResult& In,TArray<uint8>& Out);
 TURBOLINKGRPC_API void EncodeItemReceipt(const FGrpcNightfallV1ItemOperationResult& In,TArray<uint8>& Out);

	/** Serializes a populated server envelope for offline contract/replay tests. */
	TURBOLINKGRPC_API void EncodeServerMessage(const FGrpcNightfallV1ServerMessage& In, TArray<uint8>& Out);
	/** Serialises nightfall.v1.ClientMessage. An Intent whose active case has no payload is omitted. */
	TURBOLINKGRPC_API void EncodeClientMessage(const FGrpcNightfallV1ClientMessage& In, TArray<uint8>& Out);

	/** Encodes creation for wire-contract tests, including optional base-class presence. */
	TURBOLINKGRPC_API void EncodeCreateCharacterRequest(const FGrpcNightfallV1CreateCharacterRequest& In, TArray<uint8>& Out);

	/** Parses creation through the generated optional-presence marshaler; preserves unknown enums. */
	TURBOLINKGRPC_API bool DecodeCreateCharacterRequest(const uint8* Data, int32 Size, FGrpcNightfallV1CreateCharacterRequest& Out);

	/** Catalogue contract tests use the same generated marshaler as the production RPC. */
	TURBOLINKGRPC_API void EncodeSkillCatalogResponse(const FGrpcNightfallV1GetSkillCatalogResponse& In, TArray<uint8>& Out);
	TURBOLINKGRPC_API bool DecodeSkillCatalogResponse(const uint8* Data, int32 Size, FGrpcNightfallV1GetSkillCatalogResponse& Out);

	TURBOLINKGRPC_API bool DecodeClientMessage(const uint8* Data, int32 Size, FGrpcNightfallV1ClientMessage& Out);

	/** Parses nightfall.v1.ServerMessage. Returns false on malformed input; Out is reset first. */
	TURBOLINKGRPC_API bool DecodeServerMessage(const uint8* Data, int32 Size, FGrpcNightfallV1ServerMessage& Out);
}
