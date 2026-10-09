#pragma once

#include "CoreMinimal.h"

/** Descriptor-backed metadata, exported from the sole protobuf runtime in TurboLinkGrpc. */
struct FNightfallContractObservation
{
	TArray<TPair<FString, FString>> Cases;
	FString MessageType;
	FString DecodedFields;
	TOptional<uint64> Tick;
};

namespace NightfallContractWire
{
	/** One entry for every current proto oneof case and rejection enum value, including unseen cases. */
	TURBOLINKGRPC_API TArray<TPair<FString, FString>> Catalogue();
	/** Field/enum numbers for interpreting the server audit with the same compiled schema. */
	TURBOLINKGRPC_API TMap<FString, TMap<FString, int32>> Numbers();
	/** Inspect a single envelope without lossy client projection conversion. No auth tokens exist in these messages. */
	TURBOLINKGRPC_API bool Inspect(bool bClient, const TArray<uint8>& Bytes, FNightfallContractObservation& Out);
}
