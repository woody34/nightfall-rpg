#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Net/NetClientSubsystem.h"
#include "Bot/SkillAuditSubsystem.h"

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillReferenceOrderTest,"Nightfall.Skill.ReferenceOrderBeforeFiltering",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillReferenceOrderTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 auto Spawn=[&](const TCHAR* Id,bool Dead=false){FEntitySpawn S;S.EntityId=Id;S.LifeIncarnation=2;S.StateTick=20;S.Kind=1;S.bDead=Dead;FWorldEvent E;E.Spawn=S;Send(E);};
 auto Reset=[&]{Audit->Reset();Spawn(TEXT("owner"),true);Spawn(TEXT("caster"));Spawn(TEXT("target"));};
 auto Witness=[&]{FSkillCastStarted S;S.Caster=TEXT("caster");S.CasterIncarnation=2;S.Target=TEXT("target");S.TargetIncarnation=2;S.CastId=1;S.SkillId=3;S.SkillLevel=1;S.Tick=S.BaselineTick=20;S.StartTick=18;S.CompleteTick=30;S.bBaseline=true;FWorldEvent E;E.SkillCastStarted=S;Send(E);};
 auto Check=[&](const FString& Label,const FWorldEvent& E,bool Expected){Reset();Send(E);Witness();const auto Result=Audit->Check(TEXT("spawn_before_skill_references"));TestTrue(Label+TEXT(" has evidence"),Result.IsSet());if(Result.IsSet())TestEqual(Label+TEXT(" survives later valid references"),*Result,Expected);};

 FAttackStarted Swing;Swing.Attacker=TEXT("missing");Swing.AttackerIncarnation=2;Swing.Target=TEXT("owner");Swing.TargetIncarnation=2;Swing.Tick=Swing.BaselineTick=20;Swing.ImpactTick=25;Swing.bBaseline=true;
 FWorldEvent E;E.AttackStarted=Swing;Check(TEXT("missing attacker baseline"),E,false);
 Swing.AttackerIncarnation.Reset();E.AttackStarted=Swing;Check(TEXT("missing historical attacker baseline"),E,false);
 Swing.Attacker=TEXT("caster");Swing.AttackerIncarnation=2;Swing.Target=TEXT("missing");E.AttackStarted=Swing;Check(TEXT("missing swing target"),E,false);
 Swing.Target=TEXT("");Swing.TargetIncarnation=2;E.AttackStarted=Swing;Check(TEXT("hidden swing target with leaked incarnation"),E,false);
 Swing.TargetIncarnation=0;E.AttackStarted=Swing;Check(TEXT("hidden swing endpoint"),E,true);

 FSkillCastResult Result;Result.Caster=TEXT("missing");Result.CasterIncarnation=2;Result.CastId=9;Result.Tick=21;Result.SkillId=3;Result.SkillLevel=1;
 FSkillTargetResult Row;Row.Target=TEXT("target");Row.TargetIncarnation=2;Row.Tick=21;Result.Targets.Add(Row);
 E={};E.SkillCastResult=Result;Check(TEXT("missing result caster without cast history"),E,false);
 Result.Caster=TEXT("caster");Result.Targets[0].Target=TEXT("missing");E.SkillCastResult=Result;Check(TEXT("missing result row without cast history"),E,false);
 // A matching cast or duplicate terminal must not hide a delivered invalid row either.
 Reset();Witness();Result.CastId=1;E.SkillCastResult=Result;Send(E);Witness();TestFalse(TEXT("result row with matching cast fails ordering"),Audit->Check(TEXT("spawn_before_skill_references")).Get(true));
 Reset();Witness();Result.Targets[0].Target=TEXT("target");E.SkillCastResult=Result;Send(E);Result.Targets[0].Target=TEXT("missing");E.SkillCastResult=Result;Send(E);Witness();TestFalse(TEXT("duplicate terminal still audits references"),Audit->Check(TEXT("spawn_before_skill_references")).Get(true));

 FAttackResult Hit;Hit.Attacker=TEXT("missing");Hit.Target=TEXT("target");Hit.TargetIncarnation=2;Hit.Tick=21;E={};E.AttackResult=Hit;Check(TEXT("missing attack result attacker"),E,false);
 Hit.Attacker=TEXT("caster");Hit.Target=TEXT("missing");E.AttackResult=Hit;Check(TEXT("missing attack result subject"),E,false);
 FResurrectOffer Offer;Offer.Target=TEXT("missing");Offer.TargetIncarnation=2;Offer.Caster=TEXT("caster");Offer.CasterIncarnation=2;Offer.OfferId=1;Offer.Tick=21;Offer.ExpiryTick=40;Offer.State=1;
 E={};E.ResurrectOffer=Offer;Check(TEXT("missing offer owner before owner filter"),E,false);
 Offer.Target=TEXT("owner");Offer.Caster=TEXT("missing");E.ResurrectOffer=Offer;Check(TEXT("missing offered source"),E,false);
 Offer.Target=TEXT("target");E.ResurrectOffer=Offer;Check(TEXT("offer source before nonowner/alive filters"),E,false);
 Offer.Target=TEXT("owner");Offer.Caster.Reset();Offer.CasterIncarnation.Reset();E.ResurrectOffer=Offer;Check(TEXT("hidden offer source"),E,true);
 Offer.CasterIncarnation=2;E.ResurrectOffer=Offer;Check(TEXT("unpaired offer source"),E,false);
 Reset();Offer.Caster=TEXT("caster");E.ResurrectOffer=Offer;Send(E);Offer.State=2;E.ResurrectOffer=Offer;Send(E);Offer.Caster=TEXT("missing");E.ResurrectOffer=Offer;Send(E);Witness();TestFalse(TEXT("terminal offer filter cannot hide source violation"),Audit->Check(TEXT("spawn_before_skill_references")).Get(true));

 FSkillCastCancelled Cancel;Cancel.Caster=TEXT("missing");Cancel.CasterIncarnation=2;Cancel.Tick=21;E={};E.SkillCastCancelled=Cancel;Check(TEXT("missing cancellation subject"),E,false);
 FEffectRemoved Remove;Remove.Target=TEXT("missing");Remove.TargetIncarnation=2;Remove.Tick=21;Remove.EffectId=1;E={};E.EffectRemoved=Remove;Check(TEXT("missing effect removal subject"),E,false);
 FAttackCancelled Stop;Stop.Attacker=TEXT("missing");Stop.Target=TEXT("target");Stop.Tick=21;E={};E.AttackCancelled=Stop;Check(TEXT("missing swing cancellation subject"),E,false);

 // Stale known subjects retain the exception, including despawn tombstones.
 Swing.Attacker=TEXT("caster");Swing.AttackerIncarnation=1;Swing.Target=TEXT("missing");Swing.TargetIncarnation=1;E={};E.AttackStarted=Swing;Check(TEXT("old-life swing"),E,true);
 Swing.AttackerIncarnation=2;Swing.Tick=Swing.BaselineTick=19;E.AttackStarted=Swing;Check(TEXT("old-baseline swing"),E,true);
 Result.Caster=TEXT("caster");Result.CasterIncarnation=1;Result.Targets[0].Target=TEXT("missing");E={};E.SkillCastResult=Result;Check(TEXT("old-life result"),E,true);
 Result.CasterIncarnation=2;Result.Tick=20;E.SkillCastResult=Result;Check(TEXT("prebaseline result"),E,true);
 Result.Tick=21;Result.Targets[0].Target=TEXT("target");Result.Targets[0].TargetIncarnation=1;E.SkillCastResult=Result;Check(TEXT("old-life result row"),E,true);
 Offer.Target=TEXT("owner");Offer.TargetIncarnation=1;Offer.Caster=TEXT("missing");E={};E.ResurrectOffer=Offer;Check(TEXT("old-life offer"),E,true);
 Offer.TargetIncarnation=2;Offer.Tick=20;E.ResurrectOffer=Offer;Check(TEXT("prebaseline offer"),E,true);
 Reset();FEntityDespawn Despawn;Despawn.EntityId=TEXT("target");E={};E.Despawn=Despawn;Send(E);Result.Targets[0].TargetIncarnation=1;E={};E.SkillCastResult=Result;Send(E);Spawn(TEXT("target"));Witness();TestTrue(TEXT("stale despawned row is ignored"),Audit->Check(TEXT("spawn_before_skill_references")).Get(false));
 Reset();E={};E.Despawn=Despawn;Send(E);Result.Targets[0].TargetIncarnation=2;E={};E.SkillCastResult=Result;Send(E);Spawn(TEXT("target"));Witness();TestFalse(TEXT("current despawned row violates ordering"),Audit->Check(TEXT("spawn_before_skill_references")).Get(true));
 return true;
}
#endif
