#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Net/NetClientSubsystem.h"
#include "Skills/SkillStateSubsystem.h"
#include "Effects/EffectStateSubsystem.h"
#include "Bot/SkillAuditSubsystem.h"
#include "Misc/Base64.h"
#include "Misc/CommandLine.h"
#include "Misc/Parse.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#include "Serialization/JsonSerializer.h"

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FServerHydrationFramesTest,"Nightfall.Skill.ServerProducedHydrationFrames",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FServerHydrationFramesTest::RunTest(const FString& Parameters)
{
 FString FixtureDir=FPaths::Combine(FPaths::ProjectDir(),TEXT("../api/tests/fixtures/hydration"));
 FParse::Value(FCommandLine::Get(),TEXT("HydrationFixtureDir="),FixtureDir);
 for(const TCHAR* Name:{TEXT("pending-swings"),TEXT("npc-effects"),TEXT("replacement-offer")})
 {
  // Captured by the API's real actor/socket and zone tests; no hand-authored protobuf substitutes.
  const FString Path=FPaths::Combine(FixtureDir,FString(Name)+TEXT(".json"));FString Text;
  if(!TestTrue(TEXT("server fixture readable: ")+Path,FFileHelper::LoadFileToString(Text,*Path)))return false;
  TSharedPtr<FJsonObject> Json;auto Reader=TJsonReaderFactory<>::Create(Text);if(!TestTrue(TEXT("server fixture JSON"),FJsonSerializer::Deserialize(Reader,Json)))return false;
  if(!TestEqual(TEXT("actual ServerMessage fixture format"),Json->GetStringField(TEXT("format")),FString(TEXT("nightfall.v1.ServerMessage/base64"))))return false;
  TArray<FServerMessage> Messages;FString Owner;
  for(const auto& Frame:Json->GetArrayField(TEXT("frames")))
  {
   TArray<uint8> Bytes;FServerMessage M;
   if(!TestTrue(TEXT("protobuf frame base64"),FBase64::Decode(Frame->AsString(),Bytes)) || !TestTrue(TEXT("actual server protobuf decodes"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),M)))return false;
   if(M.Event.IsSet() && M.Event->StatsChanged.IsSet())Owner=M.Event->StatsChanged->Entity;
   Messages.Add(M);
  }
  if(!TestFalse(TEXT("owner comes from actual private baseline"),Owner.IsEmpty()))return false;
  FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Skills=Scope.Get<USkillStateSubsystem>();auto* Effects=Scope.Get<UEffectStateSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(Owner);
  int32 Swings=0,Applied=0,Offers=0,Accepted=0;
  for(const auto& M:Messages)
  {
   N->DispatchServerMessage(M);if(!M.Event.IsSet())continue;const auto& E=*M.Event;
   if(E.AttackStarted.IsSet())
   {
    const auto& A=*E.AttackStarted;const auto* View=Skills->FindSwing(A.Attacker);const auto* Audited=Audit->SwingFor(A.Attacker);
    if(!TestTrue(TEXT("server pending swing projected and audited"),View && View->Swing.IsSet() && Audited))return false;
    TestTrue(TEXT("actual pending swing predates admission"),A.bBaseline && A.Tick<A.BaselineTick);TestEqual(TEXT("projection ordering uses server baseline"),View->Tick,A.BaselineTick);TestEqual(TEXT("projection original start"),View->Swing->Tick,A.Tick);TestEqual(TEXT("projection original impact"),View->Swing->ImpactTick,A.ImpactTick);TestEqual(TEXT("projection original reuse"),View->Swing->ReadyTick,A.ReadyTick);TestEqual(TEXT("audit original start"),Audited->Tick,A.Tick);++Swings;
   }
   if(E.EffectApplied.IsSet())
   {
    const auto& A=*E.EffectApplied;const auto* Identity=N->GetKnownEntities().Find(A.Target);
    TestTrue(TEXT("actual NPC identity carries hydration tick"),Identity && Identity->Kind==2 && Identity->StateTick==A.BaselineTick);TestTrue(TEXT("actual NPC effect accepted"),Effects->Has(A.Target,A.SkillId));TestTrue(TEXT("NPC effect audit coherent"),Audit->Check(TEXT("effect_identity_coherent")).Get(false));++Applied;
   }
   if(E.ResurrectOffer.IsSet())
   {
    const auto& O=*E.ResurrectOffer;
    if(O.State==1)
    {
     TestTrue(TEXT("actual replacement offer explicitly hydrated"),O.bBaseline && O.BaselineTick==O.Tick);TestTrue(TEXT("actual offer immediately presented"),Skills->OfferPending());TestEqual(TEXT("actual retained offer ID"),Skills->GetLastOfferId(),O.OfferId);++Offers;
    }
    else if(O.State==4)
    {
     TestTrue(TEXT("actual accepted terminal is ordinary"),!O.bBaseline && O.BaselineTick==0);TestFalse(TEXT("actual revival clears consent prompt"),Skills->OfferPending());TestEqual(TEXT("actual late terminal completes witnessed refund"),Audit->Number(TEXT("resurrection_refunds")).Get(0),uint64(1));N->DispatchServerMessage(M);TestEqual(TEXT("actual terminal duplicate cannot repeat refund"),Audit->Number(TEXT("resurrection_refunds")).Get(0),uint64(1));++Accepted;
    }
    TestEqual(TEXT("actual offer audited across life transition"),Audit->OfferState().Get(0),O.State);
   }
  }
  if(FString(Name)==TEXT("pending-swings"))TestTrue(TEXT("actual pending swings exercised"),Swings>0);
  if(FString(Name)==TEXT("npc-effects"))TestTrue(TEXT("actual NPC effect exercised"),Applied>0);
  if(FString(Name)==TEXT("replacement-offer")){TestTrue(TEXT("actual pending offer exercised"),Offers>0);TestTrue(TEXT("actual accepted life transition exercised"),Accepted>0);}
  TestTrue(TEXT("actual identity batch precedes references"),Audit->Check(TEXT("spawn_before_skill_references")).Get(false));
 }
 return true;
}
#endif
