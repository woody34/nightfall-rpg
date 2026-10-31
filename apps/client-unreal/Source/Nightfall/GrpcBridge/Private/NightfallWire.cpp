#include "NightfallWire.h"
#include "SNightfallV1/WorldMarshaling.h"
#include "SNightfallV1/GameMarshaling.h"
#include "SNightfallV1/ItemsMarshaling.h"
#include "SNightfallV1/EconomyMarshaling.h"
#include <google/protobuf/io/coded_stream.h>
#include <google/protobuf/wire_format_lite.h>

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
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::EquipItem) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::UnequipItem) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::UseItem) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::DestroyItem) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::EnchantItem) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::ToggleAutoShot) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::RefineItem) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::CancelRefine) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::ExchangeSpecialAbility) ||
			(Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::CrystallizeItem) ||
			// Phase 5 economy intents (29..69) are value members: always present when selected.
			(uint8(Intent.IntentCase) >= uint8(EGrpcNightfallV1ClientMessageIntent::PickUpItem) && uint8(Intent.IntentCase) <= uint8(EGrpcNightfallV1ClientMessageIntent::MailMarkRead));

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

namespace
{
 // TurboLink has no NotSet case for a oneof of messages: an EconomyResult without an outcome keeps
 // OutcomeCase = Pickup with a null pointer, and the generated marshaler dereferences it. Materialize
 // the selected member for the marshal and clear the proto field afterwards (same presence as absent).
 bool MaterializeOutcome(FGrpcNightfallV1EconomyResult& R)
 {
  auto& O=R.Outcome;
  switch(O.OutcomeCase)
  {
  case EGrpcNightfallV1EconomyResultOutcome::Pickup: if(O.Pickup.IsValid())return false; O.Pickup=MakeShared<FGrpcNightfallV1PickupOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Vendor: if(O.Vendor.IsValid())return false; O.Vendor=MakeShared<FGrpcNightfallV1VendorOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Craft: if(O.Craft.IsValid())return false; O.Craft=MakeShared<FGrpcNightfallV1CraftOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Multisell: if(O.Multisell.IsValid())return false; O.Multisell=MakeShared<FGrpcNightfallV1ExchangeOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Trade: if(O.Trade.IsValid())return false; O.Trade=MakeShared<FGrpcNightfallV1TradeOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Store: if(O.Store.IsValid())return false; O.Store=MakeShared<FGrpcNightfallV1StoreOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Container: if(O.Container.IsValid())return false; O.Container=MakeShared<FGrpcNightfallV1ContainerOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Mail: if(O.Mail.IsValid())return false; O.Mail=MakeShared<FGrpcNightfallV1MailOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Manor: if(O.Manor.IsValid())return false; O.Manor=MakeShared<FGrpcNightfallV1ManorOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Auction: if(O.Auction.IsValid())return false; O.Auction=MakeShared<FGrpcNightfallV1AuctionOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Party: if(O.Party.IsValid())return false; O.Party=MakeShared<FGrpcNightfallV1PartyOutcome>(); return true;
  case EGrpcNightfallV1EconomyResultOutcome::Recipe: if(O.Recipe.IsValid())return false; O.Recipe=MakeShared<FGrpcNightfallV1RecipeOutcome>(); return true;
  default: return false;
  }
 }
}

void NightfallWire::EncodeServerMessage(const FGrpcNightfallV1ServerMessage& In, TArray<uint8>& Out)
{
	::nightfall::v1::ServerMessage Msg;
	// The generated server marshaler dereferences nullable spawn message fields.
 FGrpcNightfallV1ServerMessage Safe=In;
 bool bSpawn=false,bSwing=false,bCast=false,bEmptyOutcome=false;
 if(In.Payload.PayloadCase==EGrpcNightfallV1ServerMessagePayload::Event && In.Payload.Event.IsValid() && In.Payload.Event->Event.EventCase==EGrpcNightfallV1WorldEventEvent::Spawn && In.Payload.Event->Event.Spawn.IsValid())
 {
  bSpawn=true;Safe.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>(*In.Payload.Event);
  Safe.Payload.Event->Event.Spawn=MakeShared<FGrpcNightfallV1EntitySpawn>(*In.Payload.Event->Event.Spawn);
  auto& Spawn=*Safe.Payload.Event->Event.Spawn;bSwing=Spawn.PendingSwing.IsValid();bCast=Spawn.PendingCast.IsValid();
  if(!bSwing)Spawn.PendingSwing=MakeShared<FGrpcNightfallV1AttackStarted>();
  if(!bCast)Spawn.PendingCast=MakeShared<FGrpcNightfallV1SkillCastStarted>();
 }
 if(In.Payload.PayloadCase==EGrpcNightfallV1ServerMessagePayload::Event && In.Payload.Event.IsValid() && In.Payload.Event->Event.EventCase==EGrpcNightfallV1WorldEventEvent::EconomyResult)
 {
  Safe.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>(*In.Payload.Event);
  bEmptyOutcome=MaterializeOutcome(Safe.Payload.Event->Event.EconomyResult);
 }
 TURBOLINK_TO_GRPC(&Safe,&Msg);
 if(bSpawn){if(!bSwing)Msg.mutable_event()->mutable_spawn()->clear_pending_swing();if(!bCast)Msg.mutable_event()->mutable_spawn()->clear_pending_cast();}
 if(bEmptyOutcome)Msg.mutable_event()->mutable_economy_result()->clear_outcome();
	Out.SetNumUninitialized(static_cast<int32>(Msg.ByteSizeLong()));
	Msg.SerializeWithCachedSizesToArray(Out.GetData());
}

namespace
{
 bool ExtractPayload(const uint8* Data,int32 Size,uint32 Field,uint32 LastCase,TArray<uint8>& Out)
 {
  google::protobuf::io::CodedInputStream Stream(Data,Size);
  while(Stream.CurrentPosition()<Size){
   const uint32 Tag=Stream.ReadTag();if(!Tag)return false;
   if(Tag==(Field<<3|2)){
    uint32 Length=0;if(!Stream.ReadVarint32(&Length) || Length>uint32(Size-Stream.CurrentPosition()))return false;
    Out.Append(Data+Stream.CurrentPosition(),int32(Length));if(!Stream.Skip(int32(Length)))return false;
   }else {
    if((Tag&7)==2 && (Tag>>3)<=LastCase)Out.Reset();
    if(!google::protobuf::internal::WireFormatLite::SkipField(&Stream,Tag))return false;
   }
  }
  return true;
 }
}
void NightfallWire::ItemReceiptBytes(const uint8* Data,int32 Size,TArray<uint8>& Out)
{
 Out.Reset();::nightfall::v1::ServerMessage M;
 if(Size<0 || (Size>0 && !Data) || !M.ParseFromArray(Data,Size) || !M.has_event())return;
 const auto& E=M.event();const uint32 Field=E.has_enchant_result()?31:E.has_refine_result()?32:E.has_item_operation_result()?33:0;
 if(!Field)return;
 TArray<uint8> Envelope;
 if(!ExtractPayload(Data,Size,2,3,Envelope) || !ExtractPayload(Envelope.GetData(),Envelope.Num(),Field,33,Out))Out.Reset();
}
void NightfallWire::EncodeItemReceipt(const FGrpcNightfallV1EnchantResult& In,TArray<uint8>& Out)
{
 ::nightfall::v1::EnchantResult M;TURBOLINK_TO_GRPC(&In,&M);Out.SetNumUninitialized(static_cast<int32>(M.ByteSizeLong()));M.SerializeWithCachedSizesToArray(Out.GetData());
}
void NightfallWire::EncodeItemReceipt(const FGrpcNightfallV1RefineResult& In,TArray<uint8>& Out)
{
 ::nightfall::v1::RefineResult M;TURBOLINK_TO_GRPC(&In,&M);Out.SetNumUninitialized(static_cast<int32>(M.ByteSizeLong()));M.SerializeWithCachedSizesToArray(Out.GetData());
}
void NightfallWire::EncodeItemReceipt(const FGrpcNightfallV1ItemOperationResult& In,TArray<uint8>& Out)
{
 ::nightfall::v1::ItemOperationResult M;TURBOLINK_TO_GRPC(&In,&M);Out.SetNumUninitialized(static_cast<int32>(M.ByteSizeLong()));M.SerializeWithCachedSizesToArray(Out.GetData());
}
void NightfallWire::EconomyReceiptBytes(const uint8* Data,int32 Size,TArray<uint8>& Out)
{
 Out.Reset();::nightfall::v1::ServerMessage M;
 if(Size<0 || (Size>0 && !Data) || !M.ParseFromArray(Data,Size) || !M.has_event() || !M.event().has_economy_result())return;
 // The WorldEvent oneof ends at loot_distributed (45): a later member of the same oneof replaces an earlier one.
 TArray<uint8> Envelope;
 if(!ExtractPayload(Data,Size,2,3,Envelope) || !ExtractPayload(Envelope.GetData(),Envelope.Num(),34,45,Out))Out.Reset();
}
void NightfallWire::EncodeEconomyReceipt(const FGrpcNightfallV1EconomyResult& In,TArray<uint8>& Out)
{
 ::nightfall::v1::EconomyResult M;FGrpcNightfallV1EconomyResult Safe=In;const bool bEmptyOutcome=MaterializeOutcome(Safe);TURBOLINK_TO_GRPC(&Safe,&M);if(bEmptyOutcome)M.clear_outcome();
 Out.SetNumUninitialized(static_cast<int32>(M.ByteSizeLong()));M.SerializeWithCachedSizesToArray(Out.GetData());
}
