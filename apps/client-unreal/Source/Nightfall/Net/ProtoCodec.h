#pragma once

#include "CoreMinimal.h"

// Wire types the client sends and receives. Mirrors packages/proto/nightfall/v1/world.proto.
// The structs here are the client's view; the codec maps them to protobuf bytes.
//
// STATUS: ProtoCodec.cpp is a hand-written minimal protobuf wire codec covering only the
// messages below, so the project builds before protoc-generated code and the protobuf-lite
// runtime are wired in (Scripts/gen-proto.sh, ThirdParty module). The field numbers are the
// contract and must match world.proto exactly. Replace the implementation, keep the interface.

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
};

struct FEntityMove
{
	FString EntityId;      // 1
	FNetVec2 Position;     // 2
	FNetVec2 Destination;  // 3
	float Speed = 0.f;     // 4
	int64 ServerTimeMs = 0;// 5
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

struct FServerMessage
{
	TOptional<uint32> AckSeq;          // ServerMessage.ack = 1 -> Ack.seq = 1
	TOptional<FWorldEvent> Event;      // ServerMessage.event = 2
};

namespace NightfallProto
{
	// Encodes a ClientMessage to protobuf bytes. Never fails for well-formed input.
	void Encode(const FClientMessage& In, TArray<uint8>& Out);

	// Decodes a ServerMessage. Returns false on malformed input; Out is unspecified then.
	bool Decode(const uint8* Data, int32 Size, FServerMessage& Out);
}
