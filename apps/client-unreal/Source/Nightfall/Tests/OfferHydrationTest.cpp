#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Net/NetClientSubsystem.h"
#include "Skills/SkillStateSubsystem.h"
#include "Bot/SkillAuditSubsystem.h"
#include "NightfallWire.h"

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FOfferHydrationTest,"Nightfall.Skill.ReplacementOfferBaseline",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FOfferHydrationTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 FEntitySpawn Identity;Identity.EntityId=TEXT("owner");Identity.Kind=1;Identity.bDead=true;Identity.LifeIncarnation=1;Identity.SessionGeneration=2;Identity.StateTick=40;FServerMessage Spawn;Spawn.Event=FWorldEvent();Spawn.Event->Spawn=Identity;N->DispatchServerMessage(Spawn);
 auto Row=MakeShared<FGrpcNightfallV1ResurrectOffer>();Row->Target=TEXT("owner");Row->TargetIncarnation=1;Row->OfferId=7;Row->ExpiryTick=100;Row->State=EGrpcNightfallV1ResurrectOfferState::RESURRECT_OFFER_STATE_OFFERED;Row->Tick=40;
 FGrpcNightfallV1ServerMessage Envelope;Envelope.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;Envelope.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();Envelope.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::ResurrectOffer;Envelope.Payload.Event->Event.ResurrectOffer=Row;
 auto Deliver=[&]{TArray<uint8> Bytes;NightfallWire::EncodeServerMessage(Envelope,Bytes);FServerMessage M;if(!TestTrue(TEXT("offer wire decodes"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),M)))return;N->DispatchServerMessage(M);};
 // Independent pre-C12 bytes: additive defaults remain absent.
 const TArray<uint8> Legacy={0x12,0x14,0xb2,0x01,0x11,0x0a,0x05,'o','w','n','e','r',0x10,0x01,0x18,0x07,0x40,0x64,0x48,0x01,0x50,0x28};
 TArray<uint8> Bytes;NightfallWire::EncodeServerMessage(Envelope,Bytes);TestTrue(TEXT("old offer bytes unchanged"),Bytes==Legacy);FServerMessage Old;TestTrue(TEXT("legacy offer decode"),NightfallProto::Decode(Legacy.GetData(),Legacy.Num(),Old));if(!Old.Event.IsSet() || !Old.Event->ResurrectOffer.IsSet())return false;TestFalse(TEXT("legacy baseline absent"),Old.Event->ResurrectOffer->bBaseline);TestEqual(TEXT("legacy baseline tick defaults zero"),Old.Event->ResurrectOffer->BaselineTick,uint64(0));
 Deliver();TestFalse(TEXT("ordinary event at replacement identity tick stays stale"),S->OfferPending());TestFalse(TEXT("audit also rejects ordinary offer at identity"),Audit->OfferState().IsSet());
 Row->Baseline=true;Row->BaselineTick=40;Deliver();TestTrue(TEXT("replacement presents retained offer immediately"),S->OfferPending());TestEqual(TEXT("retained offer id"),S->GetLastOfferId(),uint64(7));TestEqual(TEXT("audit consumes marked offer baseline"),Audit->OfferState().Get(0),uint32(1));
 Row->OfferId=8;Row->BaselineTick=39;Deliver();TestEqual(TEXT("different identity baseline rejected"),S->GetLastOfferId(),uint64(7));Row->BaselineTick=40;Row->Tick=41;Deliver();TestEqual(TEXT("baseline timestamp mismatch rejected"),S->GetLastOfferId(),uint64(7));Row->Tick=40;Row->TargetIncarnation=2;Deliver();TestEqual(TEXT("wrong corpse incarnation rejected"),S->GetLastOfferId(),uint64(7));
 Row->TargetIncarnation=1;Row->OfferId=7;Row->Baseline=false;Row->BaselineTick=0;Row->Tick=41;Row->State=EGrpcNightfallV1ResurrectOfferState::RESURRECT_OFFER_STATE_DECLINED;Deliver();TestFalse(TEXT("ordinary later decline closes hydrated offer"),S->OfferPending());TestEqual(TEXT("audit terminal follows hydrated identity"),Audit->OfferState().Get(0),uint32(2));
 return true;
}
#endif
