#include "ProtoCodec.h"
#include "NightfallWire.h"

// Maps the client-facing FNet* structs to TurboLink's generated nightfall.v1 USTRUCTs. The bytes
// themselves are produced and parsed by protoc-generated code behind NightfallWire.

namespace
{
	FNetVec2 FromGrpc(const FGrpcNightfallV1Position& P)
	{
		return FNetVec2{ P.X, P.Y };
	}

	FEntitySpawn FromGrpc(const FGrpcNightfallV1EntitySpawn& S)
	{
		FEntitySpawn Out;
		Out.EntityId = S.EntityId;
		Out.Name = S.Name;
		Out.Position = FromGrpc(S.Position);
		Out.Kind = static_cast<uint32>(S.Kind);
		Out.SessionGeneration = S.SessionGeneration;
		return Out;
	}

	FEntityMove FromGrpc(const FGrpcNightfallV1EntityMove& M)
	{
		FEntityMove Out;
		Out.EntityId = M.EntityId;
		Out.Position = FromGrpc(M.Position);
		Out.Destination = FromGrpc(M.Destination);
		Out.Speed = M.Speed;
		Out.ServerTimeMs = M.ServerTimeMs;
		Out.Tick = M.Tick;
		return Out;
	}

	FWorldEvent FromGrpc(const FGrpcNightfallV1WorldEvent& E)
	{
		// The oneof's case enum defaults to its first member, so the payload pointer is what says
		// whether a member is actually present.
		FWorldEvent Out;
		const FGrpcNightfallV1WorldEventEvent& Ev = E.Event;
		switch (Ev.EventCase)
		{
		case EGrpcNightfallV1WorldEventEvent::Spawn:
			if (Ev.Spawn.IsValid()) { Out.Spawn = FromGrpc(*Ev.Spawn); }
			break;
		case EGrpcNightfallV1WorldEventEvent::Move:
			if (Ev.Move.IsValid()) { Out.Move = FromGrpc(*Ev.Move); }
			break;
		case EGrpcNightfallV1WorldEventEvent::Despawn:
			if (Ev.Despawn.IsValid()) { Out.Despawn = FEntityDespawn{ Ev.Despawn->EntityId }; }
			break;
		}
		return Out;
	}
}

namespace NightfallProto
{
	void Encode(const FClientMessage& In, TArray<uint8>& Out)
	{
		FGrpcNightfallV1ClientMessage Msg;
		Msg.Seq = In.Seq;
		if (In.MoveTo.IsSet())
		{
			TSharedPtr<FGrpcNightfallV1MoveToRequest> MoveTo = MakeShared<FGrpcNightfallV1MoveToRequest>();
			MoveTo->Destination.X = In.MoveTo->Destination.X;
			MoveTo->Destination.Y = In.MoveTo->Destination.Y;
			Msg.Intent.IntentCase = EGrpcNightfallV1ClientMessageIntent::MoveTo;
			Msg.Intent.MoveTo = MoveTo;
		}
		NightfallWire::EncodeClientMessage(Msg, Out);
	}

	bool Decode(const uint8* Data, int32 Size, FServerMessage& Out)
	{
		Out = FServerMessage();
		FGrpcNightfallV1ServerMessage Msg;
		if (!NightfallWire::DecodeServerMessage(Data, Size, Msg))
		{
			return false;
		}

		const FGrpcNightfallV1ServerMessagePayload& Payload = Msg.Payload;
		switch (Payload.PayloadCase)
		{
		case EGrpcNightfallV1ServerMessagePayload::Ack:
			if (Payload.Ack.IsValid()) { Out.Ack = FAck{ Payload.Ack->Seq, Payload.Ack->Tick }; }
			break;
		case EGrpcNightfallV1ServerMessagePayload::Event:
			if (Payload.Event.IsValid()) { Out.Event = FromGrpc(*Payload.Event); }
			break;
		case EGrpcNightfallV1ServerMessagePayload::Rejected:
			if (Payload.Rejected.IsValid())
			{
				Out.Rejected = FIntentRejected{ Payload.Rejected->Seq, static_cast<uint32>(Payload.Rejected->Reason), Payload.Rejected->Detail };
			}
			break;
		}
		return true;
	}
}
