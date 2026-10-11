#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Net/ProtoCodec.h"
#include "Net/NetClientSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "NightfallWire.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FStatsPresenceTest,"Nightfall.Skill.StatsMessagePresence",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FStatsPresenceTest::RunTest(const FString& Parameters)
{
 // Historical ServerMessage{event{stats_changed{entity:"x",hp:7,tick:9}}}; no tag20.
 const TArray<uint8> Legacy={0x12,0x09,0x3a,0x07,0x0a,0x01,0x78,0x10,0x07,0x70,0x09};
 FGrpcNightfallV1ServerMessage Wire;if(!TestTrue(TEXT("legacy bridge decode"),NightfallWire::DecodeServerMessage(Legacy.GetData(),Legacy.Num(),Wire)) || !TestTrue(TEXT("legacy stats payload present"),Wire.Payload.Event.IsValid() && Wire.Payload.Event->Event.StatsChanged.IsValid()))return false;
 TestFalse(TEXT("legacy nested sheet absent"),Wire.Payload.Event->Event.StatsChanged->HasEffectiveStats);
 TArray<uint8> RoundTrip;NightfallWire::EncodeServerMessage(Wire,RoundTrip);TestTrue(TEXT("legacy bytes exactly unchanged"),RoundTrip==Legacy);
 FServerMessage Projection;TestTrue(TEXT("legacy projection decode"),NightfallProto::Decode(Legacy.GetData(),Legacy.Num(),Projection));TestFalse(TEXT("absence is unknown"),Projection.Event->StatsChanged->EffectiveStats.IsSet());
 auto& Stats=*Wire.Payload.Event->Event.StatsChanged;Stats.HasEffectiveStats=true;
 NightfallWire::EncodeServerMessage(Wire,RoundTrip);TestTrue(TEXT("present empty has distinct encoding"),RoundTrip!=Legacy);
 FGrpcNightfallV1ServerMessage Empty;TestTrue(TEXT("present empty bridge decode"),NightfallWire::DecodeServerMessage(RoundTrip.GetData(),RoundTrip.Num(),Empty));TestTrue(TEXT("present zero sheet retained"),Empty.Payload.Event->Event.StatsChanged->HasEffectiveStats);
 TArray<uint8> EmptyAgain;NightfallWire::EncodeServerMessage(Empty,EmptyAgain);TestTrue(TEXT("present empty bytes preserved"),EmptyAgain==RoundTrip);
 TestTrue(TEXT("present empty projection decode"),NightfallProto::Decode(RoundTrip.GetData(),RoundTrip.Num(),Projection));TestTrue(TEXT("empty is a known zero-valued sheet"),Projection.Event->StatsChanged->EffectiveStats.IsSet());
 TestEqual(TEXT("known zero field"),Projection.Event->StatsChanged->EffectiveStats->CastSpeed,0u);
 Stats.EffectiveStats.MAtkQ=-123;NightfallWire::EncodeServerMessage(Wire,RoundTrip);TestTrue(TEXT("populated projection decode"),NightfallProto::Decode(RoundTrip.GetData(),RoundTrip.Num(),Projection));TestEqual(TEXT("signed stat survives"),Projection.Event->StatsChanged->EffectiveStats->MAtkQ,int64(-123));
 // Reusing a generated envelope must reset previous presence on an absent message.
 TestTrue(TEXT("decode absent over populated bridge"),NightfallWire::DecodeServerMessage(Legacy.GetData(),Legacy.Num(),Empty));TestFalse(TEXT("previous sheet cannot leak presence"),Empty.Payload.Event->Event.StatsChanged->HasEffectiveStats);
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* C=Scope.Get<UCombatStateSubsystem>();N->SetOwnEntityId(TEXT("x"));
 FEntitySpawn Spawn;Spawn.EntityId=TEXT("x");Spawn.LifeIncarnation=1;Spawn.StateTick=10;FServerMessage M;M.Event=FWorldEvent{};M.Event->Spawn=Spawn;N->DispatchServerMessage(M);
 auto S=*Projection.Event->StatsChanged;S.Entity=TEXT("x");S.Incarnation=1;S.bBaseline=true;S.Tick=S.BaselineTick=10;M.Event=FWorldEvent{};M.Event->StatsChanged=S;N->DispatchServerMessage(M);TestTrue(TEXT("owner projection holds received sheet"),C->GetOwn().EffectiveStats.IsSet());
 S.Tick=11;S.bBaseline=false;S.EffectiveStats.Reset();M.Event->StatsChanged=S;N->DispatchServerMessage(M);TestFalse(TEXT("later absent sheet remains unknown through projection"),C->GetOwn().EffectiveStats.IsSet());
 S.Tick=12;S.EffectiveStats=FEffectiveStats{};M.Event->StatsChanged=S;N->DispatchServerMessage(M);TestTrue(TEXT("present empty owner sheet stays known"),C->GetOwn().EffectiveStats.IsSet());
 return true;
}
#endif
