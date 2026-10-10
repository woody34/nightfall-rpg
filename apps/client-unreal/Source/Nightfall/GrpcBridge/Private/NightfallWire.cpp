#include "NightfallWire.h"
#include "SNightfallV1/WorldMarshaling.h"
#include "SNightfallV1/GameMarshaling.h"

namespace NightfallWire
{
	void EncodeClientMessage(const FGrpcNightfallV1ClientMessage& In, TArray<uint8>& Out)
	{
		// TurboLink's oneof structs have no "not set" case (the default is the first member), and its
		// marshaler dereferences the active member unchecked. Only marshal the intent when it exists.
		const FGrpcNightfallV1ClientMessageIntent& Intent = In.Intent;
		const bool bHasIntent =
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::MoveTo && Intent.MoveTo.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::StopMove && Intent.StopMove.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::SetTarget && Intent.SetTarget.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::Attack && Intent.Attack.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::StopAttack && Intent.StopAttack.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::Respawn && Intent.Respawn.IsValid());

		::nightfall::v1::ClientMessage Msg;
		if (bHasIntent)
		{
			TURBOLINK_TO_GRPC(&In, &Msg);
		}
		else
		{
			Msg.set_seq(In.Seq);
		}

		Out.SetNumUninitialized(static_cast<int32>(Msg.ByteSizeLong()));
		Msg.SerializeWithCachedSizesToArray(Out.GetData());
	}

	void EncodeCreateCharacterRequest(const FGrpcNightfallV1CreateCharacterRequest& In, TArray<uint8>& Out)
	{
		::nightfall::v1::CreateCharacterRequest Msg;
		TURBOLINK_TO_GRPC(&In, &Msg);
		Out.SetNumUninitialized(static_cast<int32>(Msg.ByteSizeLong()));
		Msg.SerializeWithCachedSizesToArray(Out.GetData());
	}

	bool DecodeCreateCharacterRequest(const uint8* Data, int32 Size, FGrpcNightfallV1CreateCharacterRequest& Out)
	{
		::nightfall::v1::CreateCharacterRequest Msg;
		if (Size < 0 || (Size > 0 && !Data) || !Msg.ParseFromArray(Data, Size)) return false;
		GRPC_TO_TURBOLINK(&Msg, &Out);
		return true;
	}

	bool DecodeServerMessage(const uint8* Data, int32 Size, FGrpcNightfallV1ServerMessage& Out)
	{
		Out = FGrpcNightfallV1ServerMessage();
		::nightfall::v1::ServerMessage Msg;
		if (Size < 0 || (Size > 0 && Data == nullptr) || !Msg.ParseFromArray(Data, Size))
		{
			return false;
		}
		GRPC_TO_TURBOLINK(&Msg, &Out);
		return true;
	}
}
