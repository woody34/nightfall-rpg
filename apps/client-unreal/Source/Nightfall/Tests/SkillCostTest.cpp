#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Bot/SkillAuditSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Skills/SkillStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillCostTest,"Nightfall.Skill.CatalogueCosts",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillCostTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Skills=Scope.Get<USkillStateSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 FBotPredicateRegistry R;R.RegisterBuiltins();FBotContext Context;Context.GameInstance=Scope.GameInstance;
 auto Holds=[&](const TCHAR* Text){FString Error;auto P=R.Parse(FBotPredicateRegistry::Tokenize(Text),Error);TestTrue(TEXT("cost predicate registered"),bool(P));return P && P(Context).bTrue;};
 TestFalse(TEXT("missing catalogue/observation is unknown"),Holds(TEXT("own_mp_cost_matches 1177")));
 FGrpcNightfallV1GetSkillCatalogResponse Catalog;FGrpcNightfallV1SkillCatalogEntry Row;Row.SkillId=1177;Row.MaxLevel=2;Row.MpConsumeStart={FUInt32(0),FUInt32(3)};Row.MpConsumeEnd={FUInt32(5),FUInt32(7)};Row.HpConsume={FUInt32(0),FUInt32(0)};Row.ReuseMs={FUInt32(1000),FUInt32(1200)};Row.HitTimeMs={FUInt32(4000),FUInt32(4000)};Catalog.Skills.Add(Row);Skills->ApplyCatalog(Catalog);
 FGrpcNightfallV1GetSkillCatalogResponse Json;TestTrue(TEXT("generated cost arrays JSON round trip"),Json.FromJsonString(Catalog.ToJsonString(false)));TestEqual(TEXT("explicit zero rank preserved"),Json.Skills[0].MpConsumeStart[0].Value,uint32(0));TestEqual(TEXT("rank order preserved"),Json.Skills[0].MpConsumeEnd[1].Value,uint32(7));
 FEntitySpawn Spawn;Spawn.EntityId=TEXT("owner");Spawn.LifeIncarnation=1;Spawn.StateTick=10;Spawn.Hp=Spawn.MaxHp=100;FWorldEvent E;E.Spawn=Spawn;Send(E);
 FStatsChanged Stats;Stats.Entity=TEXT("owner");Stats.Incarnation=1;Stats.bBaseline=true;Stats.Tick=Stats.BaselineTick=10;Stats.Mp=100;Stats.Hp=Stats.MaxHp=100;E={};E.StatsChanged=Stats;Send(E);
 FSkillCastStarted Start;Start.Caster=TEXT("owner");Start.CasterIncarnation=1;Start.SkillId=1177;Start.SkillLevel=2;Start.Tick=11;Start.CastId=1;E={};E.SkillCastStarted=Start;Send(E);
 Stats.bBaseline=false;Stats.Tick=11;Stats.Mp=97;E={};E.StatsChanged=Stats;Send(E);
 // A separately received MP gain between stages must not corrupt either consumption delta.
 Stats.Tick=12;Stats.Mp=99;E.StatsChanged=Stats;Send(E);
 FSkillCastResult Result;Result.Caster=TEXT("owner");Result.CasterIncarnation=1;Result.CastId=1;Result.SkillId=1177;Result.SkillLevel=2;Result.Tick=13;E={};E.SkillCastResult=Result;Send(E);
 TestFalse(TEXT("completion requires resource sample"),Holds(TEXT("own_mp_cost_matches 1177")));
 Stats.Tick=13;Stats.Mp=92;E={};E.StatsChanged=Stats;Send(E);TestTrue(TEXT("exact rank stage deltas with intermediate gain"),Holds(TEXT("own_mp_cost_matches 1177")));Send(E);TestTrue(TEXT("duplicate sample harmless"),Holds(TEXT("own_mp_cost_matches 1177")));
 Catalog.Skills[0].MpConsumeEnd[1]=8;Skills->ApplyCatalog(Catalog);TestFalse(TEXT("mismatched server cost fails"),Holds(TEXT("own_mp_cost_matches 1177")));Catalog.Skills[0].MpConsumeEnd[1]=7;Skills->ApplyCatalog(Catalog);
 Start.CastId=2;Start.SkillLevel=1;Start.Tick=14;E={};E.SkillCastStarted=Start;Send(E);
 Result.CastId=2;Result.SkillLevel=1;Result.Tick=15;E={};E.SkillCastResult=Result;Send(E);Stats.Tick=15;Stats.Mp=87;E={};E.StatsChanged=Stats;Send(E);TestTrue(TEXT("received zero MP1 needs no dirty start sample"),Holds(TEXT("own_mp_cost_matches 1177")));
 Catalog.Skills[0].MpConsumeStart.Reset();Skills->ApplyCatalog(Catalog);TestFalse(TEXT("legacy/malformed array is unknown not zero"),Holds(TEXT("own_mp_cost_matches 1177")));Catalog.Skills[0].MpConsumeStart=Row.MpConsumeStart;Skills->ApplyCatalog(Catalog);
 Start.CastId=3;Start.SkillLevel=2;Start.Tick=16;E={};E.SkillCastStarted=Start;Send(E);Stats.Tick=16;Stats.Mp=84;E={};E.StatsChanged=Stats;Send(E);
 FSkillReuse Reuse;Reuse.Entity=TEXT("owner");Reuse.Incarnation=1;Reuse.SkillId=1177;Reuse.Tick=16;Reuse.ReadyTick=30;E={};E.SkillReuse=Reuse;Send(E);
 FSkillCastCancelled Cancel;Cancel.Caster=TEXT("owner");Cancel.CasterIncarnation=1;Cancel.CastId=3;Cancel.Tick=17;Cancel.Reason=2;E={};E.SkillCastCancelled=Cancel;Send(E);TestTrue(TEXT("cancel retains received MP1 and reuse"),Holds(TEXT("cancelled_cast_costs_retained")));
 Stats.Tick=17;Stats.Mp=87;E={};E.StatsChanged=Stats;Send(E);TestFalse(TEXT("observed MP refund cannot pass retention"),Holds(TEXT("cancelled_cast_costs_retained")));
 N->OnDisconnected.Broadcast(TEXT("end"));TestFalse(TEXT("transport clears MP proof"),Audit->MpCostMatches(1177).IsSet());
 return true;
}
#endif
