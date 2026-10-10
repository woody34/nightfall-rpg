#include "Misc/AutomationTest.h"
#include "Net/ProtoCodec.h"
#include "NightfallWire.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillNestedCodecTest,"Nightfall.Skill.CodecNestedPresence",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillNestedCodecTest::RunTest(const FString& Parameters)
{
 FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
 auto& E=M.Payload.Event->Event;E.EventCase=EGrpcNightfallV1WorldEventEvent::Spawn;E.Spawn=MakeShared<FGrpcNightfallV1EntitySpawn>();
 auto& S=*E.Spawn;S.EntityId=TEXT("own");
 TArray<uint8> Bytes;FServerMessage Out;NightfallWire::EncodeServerMessage(M,Bytes);
 if(!TestTrue(TEXT("absent nested message decode"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),Out)))return false;
 TestFalse(TEXT("absent swing preserved"),Out.Event->Spawn->PendingSwing.IsSet());TestFalse(TEXT("absent cast preserved"),Out.Event->Spawn->PendingCast.IsSet());
 S.PendingSwing=MakeShared<FGrpcNightfallV1AttackStarted>();S.PendingSwing->Attacker=TEXT("own");S.PendingSwing->ImpactTick=60;
 S.PendingCast=MakeShared<FGrpcNightfallV1SkillCastStarted>();S.PendingCast->Caster=TEXT("own");S.PendingCast->CastId=MAX_uint64;S.PendingCast->StartTick=41;S.PendingCast->CompleteTick=63;
 auto A=MakeShared<FGrpcNightfallV1ActiveEffect>();A->EffectId=99;A->SkillId=3;A->RemainingTicks=8;S.ActiveEffects.Add(A);
 NightfallWire::EncodeServerMessage(M,Bytes);if(!TestTrue(TEXT("nested spawn decode"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),Out)))return false;
 TestEqual(TEXT("pending original deadline"),Out.Event->Spawn->PendingCast->CompleteTick,uint64(63));TestEqual(TEXT("cast id exact64"),Out.Event->Spawn->PendingCast->CastId,MAX_uint64);TestEqual(TEXT("pending swing deadline"),Out.Event->Spawn->PendingSwing->ImpactTick,uint64(60));TestEqual(TEXT("stable effect id"),Out.Event->Spawn->ActiveEffects[0].EffectId,uint64(99));
 E.EventCase=EGrpcNightfallV1WorldEventEvent::SkillStateSync;E.SkillStateSync=MakeShared<FGrpcNightfallV1SkillStateSync>();
 auto& Sync=*E.SkillStateSync;Sync.Sp=MAX_uint64;auto& L=Sync.Learned.AddDefaulted_GetRef();L.SkillId=3;L.SkillLevel=2;auto& R=Sync.Reuse.AddDefaulted_GetRef();R.SkillId=3;R.ReadyTick=100;
 NightfallWire::EncodeServerMessage(M,Bytes);if(!TestTrue(TEXT("sync repeated rows decode"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),Out)))return false;
 TestEqual(TEXT("learned rank"),Out.Event->SkillStateSync->Learned[0].SkillLevel,2u);TestEqual(TEXT("reuse deadline"),Out.Event->SkillStateSync->Reuse[0].ReadyTick,uint64(100));TestEqual(TEXT("SP exact64"),Out.Event->SkillStateSync->Sp,MAX_uint64);
 E.EventCase=EGrpcNightfallV1WorldEventEvent::SkillCastResult;E.SkillCastResult=MakeShared<FGrpcNightfallV1SkillCastResult>();
 auto& Hit=E.SkillCastResult->Targets.AddDefaulted_GetRef();Hit.Target=TEXT("own");Hit.CpAbsorbed=7;Hit.HpRemoved=8;Hit.TargetHpAfter=92;Hit.AppliedEffectIds.Add(MAX_uint64);
 NightfallWire::EncodeServerMessage(M,Bytes);if(!TestTrue(TEXT("result nested rows decode"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),Out)))return false;
 TestEqual(TEXT("CP absorption remains distinct"),Out.Event->SkillCastResult->Targets[0].CpAbsorbed,7u);TestEqual(TEXT("HP removal remains distinct"),Out.Event->SkillCastResult->Targets[0].HpRemoved,8u);TestEqual(TEXT("applied IDs exact64"),Out.Event->SkillCastResult->Targets[0].AppliedEffectIds[0],MAX_uint64);
 E.EventCase=EGrpcNightfallV1WorldEventEvent::StatsChanged;E.StatsChanged=MakeShared<FGrpcNightfallV1StatsChanged>();E.StatsChanged->HasEffectiveStats=true;E.StatsChanged->EffectiveStats.MAtkQ=-123;E.StatsChanged->EffectiveStats.CastSpeed=1999;
 NightfallWire::EncodeServerMessage(M,Bytes);if(!TestTrue(TEXT("effective stats nested decode"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),Out)))return false;
 TestEqual(TEXT("signed effective field"),Out.Event->StatsChanged->EffectiveStats->MAtkQ,int64(-123));TestEqual(TEXT("received cast speed"),Out.Event->StatsChanged->EffectiveStats->CastSpeed,1999u);
 return true;
}
#endif
