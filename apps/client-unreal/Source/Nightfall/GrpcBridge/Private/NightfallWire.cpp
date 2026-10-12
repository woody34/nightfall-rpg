#include "NightfallWire.h"
#include "SNightfallV1/WorldMarshaling.h"
#include "SNightfallV1/GameMarshaling.h"

namespace NightfallWire
{
	void EncodeSkillCatalogResponse(const FGrpcNightfallV1GetSkillCatalogResponse& In, TArray<uint8>& Out)
	{
		::nightfall::v1::GetSkillCatalogResponse Msg;
		TURBOLINK_TO_GRPC(&In, &Msg);
		Out.SetNumUninitialized(static_cast<int32>(Msg.ByteSizeLong()));
		Msg.SerializeWithCachedSizesToArray(Out.GetData());
	}

	bool DecodeSkillCatalogResponse(const uint8* Data, int32 Size, FGrpcNightfallV1GetSkillCatalogResponse& Out)
	{
		Out = {};
		::nightfall::v1::GetSkillCatalogResponse Msg;
		if (Size < 0 || (Size > 0 && !Data) || !Msg.ParseFromArray(Data, Size)) return false;
		GRPC_TO_TURBOLINK(&Msg, &Out);
		return true;
	}

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
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::Respawn && Intent.Respawn.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::CastSkill && Intent.CastSkill.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::CancelCast && Intent.CancelCast.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::AcceptResurrect && Intent.AcceptResurrect.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::EquipItem && Intent.EquipItem.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::UnequipItem && Intent.UnequipItem.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::UseItem && Intent.UseItem.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::DestroyItem && Intent.DestroyItem.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::EnchantItem && Intent.EnchantItem.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::ToggleAutoShot && Intent.ToggleAutoShot.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::RefineItem && Intent.RefineItem.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::CancelRefine && Intent.CancelRefine.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::ExchangeSpecialAbility && Intent.ExchangeSpecialAbility.IsValid()) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::CrystallizeItem && Intent.CrystallizeItem.IsValid());

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

	bool DecodeClientMessage(const uint8* Data, int32 Size, FGrpcNightfallV1ClientMessage& Out)
	{
		Out = {};
		::nightfall::v1::ClientMessage Msg;
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
  // TurboLink materializes absent shared message fields. Preserve protobuf presence here.
  if(Msg.has_event() && Msg.event().has_spawn() && Out.Payload.Event.IsValid() && Out.Payload.Event->Event.Spawn.IsValid())
  {
   auto& Spawn=*Out.Payload.Event->Event.Spawn;
   if(!Msg.event().spawn().has_pending_swing())Spawn.PendingSwing.Reset();
   if(!Msg.event().spawn().has_pending_cast())Spawn.PendingCast.Reset();
  }
		return true;
	}
}

void NightfallWire::EncodeServerMessage(const FGrpcNightfallV1ServerMessage& In, TArray<uint8>& Out)
{
	::nightfall::v1::ServerMessage Msg;
	// The generated server marshaler dereferences nullable spawn message fields.
 FGrpcNightfallV1ServerMessage Safe=In;
 bool bSpawn=false,bSwing=false,bCast=false;
 if(In.Payload.PayloadCase==EGrpcNightfallV1ServerMessagePayload::Event && In.Payload.Event.IsValid() && In.Payload.Event->Event.EventCase==EGrpcNightfallV1WorldEventEvent::Spawn && In.Payload.Event->Event.Spawn.IsValid())
 {
  bSpawn=true;Safe.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>(*In.Payload.Event);
  Safe.Payload.Event->Event.Spawn=MakeShared<FGrpcNightfallV1EntitySpawn>(*In.Payload.Event->Event.Spawn);
  auto& Spawn=*Safe.Payload.Event->Event.Spawn;bSwing=Spawn.PendingSwing.IsValid();bCast=Spawn.PendingCast.IsValid();
  if(!bSwing)Spawn.PendingSwing=MakeShared<FGrpcNightfallV1AttackStarted>();
  if(!bCast)Spawn.PendingCast=MakeShared<FGrpcNightfallV1SkillCastStarted>();
 }
 TURBOLINK_TO_GRPC(&Safe,&Msg);
 if(bSpawn){if(!bSwing)Msg.mutable_event()->mutable_spawn()->clear_pending_swing();if(!bCast)Msg.mutable_event()->mutable_spawn()->clear_pending_cast();}
	Out.SetNumUninitialized(static_cast<int32>(Msg.ByteSizeLong()));
	Msg.SerializeWithCachedSizesToArray(Out.GetData());
}
