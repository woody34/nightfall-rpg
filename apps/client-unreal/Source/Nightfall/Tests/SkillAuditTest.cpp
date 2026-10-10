#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Bot/SkillAuditSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Net/NetClientSubsystem.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillAuditTest,"Nightfall.Skill.ReceivedAudit",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillAuditTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 FBotPredicateRegistry R;R.RegisterBuiltins();FBotContext C;C.GameInstance=Scope.GameInstance;
 auto Holds=[&](const TCHAR* T){FString Error;auto P=R.Parse(FBotPredicateRegistry::Tokenize(T),Error);TestTrue(TEXT("registered"),bool(P));return P && P(C).bTrue;};
 for(const TCHAR* T:{TEXT("cast_dispositions_once"),TEXT("public_vitals_coherent"),TEXT("spawn_before_skill_references"),TEXT("cp_absorption_matches_received"),TEXT("resurrect_offer_state != accepted"),TEXT("resurrection_refunds == 0")})TestFalse(TEXT("no vacuous evidence"),Holds(T));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 FEntitySpawn S;S.EntityId=TEXT("owner");S.Kind=1;S.LifeIncarnation=1;S.SessionGeneration=1;S.StateTick=10;S.Hp=S.MaxHp=100;S.bCombatant=true;FWorldEvent E;E.Spawn=S;Send(E);
 S.EntityId=TEXT("peer");E.Spawn=S;Send(E);
 FStatsChanged Stats;Stats.Entity=TEXT("owner");Stats.Incarnation=1;Stats.Tick=Stats.BaselineTick=10;Stats.bBaseline=true;Stats.Hp=Stats.MaxHp=100;Stats.Cp=20;Stats.Mp=40;Stats.DeathPenaltyLevel=2;E={};E.StatsChanged=Stats;Send(E);
 FSkillStateSync Sync;Sync.Entity=TEXT("owner");Sync.Incarnation=1;Sync.Tick=Sync.BaselineTick=10;Sync.bBaseline=true;Sync.DeathPenaltyLevel=2;E={};E.SkillStateSync=Sync;Send(E);
 TestTrue(TEXT("owner hydration accepted at identity tick"),Holds(TEXT("owner_skill_baseline_coherent")));TestTrue(TEXT("private/public baseline agrees"),Holds(TEXT("pvp_private_stats_coherent")));
 FSkillCastStarted Start;Start.Caster=TEXT("peer");Start.CasterIncarnation=1;Start.Target=TEXT("owner");Start.TargetIncarnation=1;Start.Tick=11;Start.CastId=3;Start.SkillId=1177;E={};E.SkillCastStarted=Start;Send(E);
 TestTrue(TEXT("known references"),Holds(TEXT("spawn_before_skill_references")));TestFalse(TEXT("unfinished cast fails disposition"),Holds(TEXT("cast_dispositions_once")));
 FSkillCastResult Result;Result.Caster=Start.Caster;Result.CasterIncarnation=1;Result.CastId=3;Result.Tick=12;Result.SkillId=Start.SkillId;Result.SkillLevel=Start.SkillLevel;FSkillTargetResult Target;Target.Target=TEXT("owner");Target.TargetIncarnation=1;Target.Tick=12;Target.CpAbsorbed=15;Target.TargetHpAfter=100;Result.Targets.Add(Target);E={};E.SkillCastResult=Result;Send(E);Send(E);
 TestTrue(TEXT("duplicate result counted once"),Holds(TEXT("own_cp_damage_total == 15")));TestTrue(TEXT("one terminal disposition"),Holds(TEXT("cast_dispositions_once")));
 Stats.Tick=12;Stats.bBaseline=false;Stats.Cp=5;E={};E.StatsChanged=Stats;Send(E);TestTrue(TEXT("received absorption equals CP delta"),Holds(TEXT("cp_absorption_matches_received")));
 FVitalsChanged V;V.Entity=TEXT("owner");V.Incarnation=1;V.Tick=12;V.Hp=100;V.MaxHp=150;E={};E.VitalsChanged=V;Send(E);TestTrue(TEXT("absolute final max-only vitals"),Holds(TEXT("public_vitals_coherent")));
 FSkillCastCancelled Cancel;Cancel.Caster=TEXT("peer");Cancel.CasterIncarnation=1;Cancel.CastId=3;Cancel.Tick=13;Cancel.Reason=1;E={};E.SkillCastCancelled=Cancel;Send(E);TestFalse(TEXT("contradictory terminal cannot pass"),Holds(TEXT("cast_dispositions_once")));
 N->OnDisconnected.Broadcast(TEXT("reconnect"));TestFalse(TEXT("transport evidence clears"),Holds(TEXT("own_cp_damage_total == 15")));
 S.EntityId=TEXT("owner");S.SessionGeneration=2;S.StateTick=20;E={};E.Spawn=S;Send(E);Stats.Tick=Stats.BaselineTick=20;Stats.bBaseline=true;E={};E.StatsChanged=Stats;Send(E);TestTrue(TEXT("penalty snapshot survives matching owner reconnect"),Holds(TEXT("death_penalty_matches_before_disconnect")));
 Stats.Tick=21;Stats.bBaseline=false;Stats.DeathPenaltyLevel=3;E.StatsChanged=Stats;Send(E);TestFalse(TEXT("penalty mismatch detected"),Holds(TEXT("death_penalty_matches_before_disconnect")));
 FEntityDied D;D.Entity=TEXT("owner");D.Incarnation=1;D.Tick=22;E={};E.EntityDied=D;Send(E);
 FResurrectOffer O;O.Target=TEXT("owner");O.TargetIncarnation=1;O.Tick=23;O.OfferId=8;O.ExpiryTick=100;O.State=1;E={};E.ResurrectOffer=O;Send(E);TestTrue(TEXT("offer bound to received death"),Holds(TEXT("resurrect_offer_matches_death")));
 O.Tick=24;O.State=2;E.ResurrectOffer=O;Send(E);TestTrue(TEXT("terminal offer preserved"),Holds(TEXT("resurrect_offer_state == declined")));TestTrue(TEXT("decline has no received refund"),Holds(TEXT("resurrection_refunds == 0")));
 O.State=1;O.Tick=25;E.ResurrectOffer=O;Send(E);TestTrue(TEXT("offered duplicate cannot undo terminal"),Holds(TEXT("resurrect_offer_state == declined")));
 return true;
}
#endif
