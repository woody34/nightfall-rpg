#pragma once

#include "CoreMinimal.h"

// Wire types the client sends and receives. Mirrors packages/proto/nightfall/v1/world.proto.
// The structs here are the client's view; the codec maps them to protobuf bytes.
//
// ProtoCodec.cpp converts these to TurboLink's generated FGrpcNightfallV1* structs and serialises
// them with protoc-generated code (Scripts/gen-proto.sh) through NightfallWire, which lives in the
// TurboLinkGrpc module next to the protobuf runtime. Gameplay code keeps using these structs.

struct FNetVec2
{
	float X = 0.f;
	float Y = 0.f;
};

struct FMoveToIntent
{
	FNetVec2 Destination;
};

struct FClientMessage
{
	uint32 Seq = 0;
	TOptional<FMoveToIntent> MoveTo;   // ClientMessage.intent.move_to = 10
};

struct FEntitySpawn
{
	FString EntityId;      // 1
	FString Name;          // 2
	FNetVec2 Position;     // 3
	uint32 Kind = 0;       // 4: 1 player, 2 npc
	uint32 SessionGeneration = 0; // 5: higher replaces an earlier entity of the same account
};

struct FEntityMove
{
	FString EntityId;      // 1
	FNetVec2 Position;     // 2
	FNetVec2 Destination;  // 3
	float Speed = 0.f;     // 4
	int64 ServerTimeMs = 0;// 5
	uint64 Tick = 0;       // 6
};

struct FEntityDespawn
{
	FString EntityId;      // 1
};

struct FWorldEvent
{
	TOptional<FEntitySpawn> Spawn;     // WorldEvent.spawn = 1
	TOptional<FEntityMove> Move;       // WorldEvent.move = 2
	TOptional<FEntityDespawn> Despawn; // WorldEvent.despawn = 3
};

struct FAck
{
	uint32 Seq = 0;        // 1
	uint64 Tick = 0;       // 2: tick on which the intent is applied
};

struct FIntentRejected
{
	uint32 Seq = 0;        // 1
	uint32 Reason = 0;     // 2: nightfall.v1.RejectReason
	FString Detail;        // 3: for logs only
};

struct FServerMessage
{
	TOptional<FAck> Ack;                  // ServerMessage.ack = 1
	TOptional<FWorldEvent> Event;         // ServerMessage.event = 2
	TOptional<FIntentRejected> Rejected;  // ServerMessage.rejected = 3
};

namespace NightfallProto
{
	// Encodes a ClientMessage to protobuf bytes. Never fails for well-formed input.
	void Encode(const FClientMessage& In, TArray<uint8>& Out);

	// Decodes a ServerMessage. Returns false on malformed input; Out is unspecified then.
	bool Decode(const uint8* Data, int32 Size, FServerMessage& Out);
}
