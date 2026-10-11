#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Net/NetClientSubsystem.h"
#include "Skills/SkillStateSubsystem.h"
#include "Bot/SkillAuditSubsystem.h"

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSwingHydrationTest,"Nightfall.Skill.PendingSwingBaseline",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSwingHydrationTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 FEntitySpawn Identity;Identity.EntityId=TEXT("owner");Identity.Kind=1;Identity.LifeIncarnation=2;Identity.SessionGeneration=1;Identity.StateTick=50;FWorldEvent E;E.Spawn=Identity;Send(E);Identity.EntityId=TEXT("peer");E.Spawn=Identity;Send(E);
 FAttackStarted Swing;Swing.Attacker=TEXT("peer");Swing.AttackerIncarnation=2;Swing.Target=TEXT("owner");Swing.TargetIncarnation=2;Swing.Tick=44;Swing.ImpactTick=60;Swing.ReadyTick=70;Swing.bBaseline=true;Swing.BaselineTick=50;E={};E.AttackStarted=Swing;Send(E);
 const auto* View=S->FindSwing(TEXT("peer"));
 if(!TestTrue(TEXT("swing that began before admission is hydrated"),View && View->Swing.IsSet()))return false;
 TestEqual(TEXT("ordering fence uses admission tick"),View->Tick,uint64(50));TestEqual(TEXT("animation start remains original"),View->Swing->Tick,uint64(44));TestEqual(TEXT("impact remains original"),View->Swing->ImpactTick,uint64(60));TestEqual(TEXT("ready remains original"),View->Swing->ReadyTick,uint64(70));TestEqual(TEXT("audit hydrates same pending swing"),Audit->Number(TEXT("players_attacking_me")).Get(0),uint64(1));
 Swing.BaselineTick=49;Swing.ImpactTick=999;E.AttackStarted=Swing;Send(E);TestEqual(TEXT("wrong baseline cannot overwrite"),S->FindSwing(TEXT("peer"))->Swing->ImpactTick,uint64(60));
 Swing.BaselineTick=50;Swing.AttackerIncarnation=1;E.AttackStarted=Swing;Send(E);TestEqual(TEXT("old life cannot overwrite"),S->FindSwing(TEXT("peer"))->Swing->ImpactTick,uint64(60));
 Swing.AttackerIncarnation=2;Swing.bBaseline=false;Swing.Tick=50;E.AttackStarted=Swing;Send(E);TestEqual(TEXT("ordinary frame at identity tick suppressed"),S->FindSwing(TEXT("peer"))->Swing->ImpactTick,uint64(60));
 FAttackCancelled Stop;Stop.Attacker=TEXT("peer");Stop.Tick=51;E={};E.AttackCancelled=Stop;Send(E);E={};Swing.bBaseline=true;Swing.Tick=44;E.AttackStarted=Swing;Send(E);TestFalse(TEXT("delayed hydration cannot revive cancelled swing"),S->FindSwing(TEXT("peer"))->Swing.IsSet());TestEqual(TEXT("audit also retains cancellation fence"),Audit->Number(TEXT("players_attacking_me")).Get(1),uint64(0));
 Swing.bBaseline=false;Swing.Tick=52;Swing.ImpactTick=62;E.AttackStarted=Swing;Send(E);TestEqual(TEXT("later ordinary start accepted"),S->FindSwing(TEXT("peer"))->Swing->Tick,uint64(52));
 return true;
}
// Captured r10 PvP: Started115, impact122 (CP5), ready129; StopAttack123 is
// after impact, so the server has no pending swing to cancel (state_combat.rs::disengage).
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSwingRecoveryAuditTest,"Nightfall.Skill.SwingRecoveryReceivedDeadline",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSwingRecoveryAuditTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* A=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 auto Tick=[&](uint64 T){FServerMessage M;M.Ack=FAck{1,T};N->DispatchServerMessage(M);};
 auto Count=[&](){return A->Number(TEXT("players_attacking_me"));};
 TestFalse(TEXT("missing owner is unknown"),Count().IsSet());
 FEntitySpawn I;I.EntityId=TEXT("owner");I.Kind=1;I.LifeIncarnation=1;I.SessionGeneration=1;I.StateTick=112;I.Hp=I.MaxHp=100;I.bCombatant=true;FWorldEvent E;E.Spawn=I;Send(E);I.EntityId=TEXT("peer");E.Spawn=I;Send(E);
 FAttackStarted S;S.Attacker=TEXT("peer");S.AttackerIncarnation=1;S.Target=TEXT("owner");S.TargetIncarnation=1;S.Tick=115;S.ImpactTick=122;S.ReadyTick=129;E={};E.AttackStarted=S;Send(E);
 TestEqual(TEXT("received in-flight swing counts"),Count().Get(0),uint64(1));
 FAttackResult R;R.Attacker=S.Attacker;R.Target=S.Target;R.TargetIncarnation=1;R.Tick=122;R.Outcome=ENetAttackOutcome::Hit;R.TargetHpAfter=100;R.CpDamage=5;E={};E.AttackResult=R;Send(E);
 TestEqual(TEXT("result retains recovery and impact evidence"),Count().Get(0),uint64(1));
 TestEqual(TEXT("CP evidence retained"),A->Number(TEXT("own_cp_damage_total")).Get(0),uint64(5));
 Tick(128);TestEqual(TEXT("before received recovery deadline"),Count().Get(0),uint64(1));
 Tick(129);TestEqual(TEXT("recovery ends without cancellation frame"),Count().Get(1),uint64(0));
 S.Tick=129;S.ImpactTick=136;S.ReadyTick=143;E={};E.AttackStarted=S;Send(E);
 TestEqual(TEXT("next start at boundary counts again"),Count().Get(0),uint64(1));
 E={};E.AttackResult=R;Send(E);TestEqual(TEXT("duplicate old result cannot end new swing"),Count().Get(0),uint64(1));
 FAttackCancelled C;C.Attacker=S.Attacker;C.Target=S.Target;C.Tick=130;E={};E.AttackCancelled=C;Send(E);
 TestEqual(TEXT("explicit cancellation ends pending swing immediately"),Count().Get(1),uint64(0));
 S.Tick=131;S.ImpactTick=138;S.ReadyTick=145;E={};E.AttackStarted=S;Send(E);
 FEntityRespawned Life;Life.Entity=TEXT("owner");Life.Incarnation=2;Life.Tick=132;Life.Hp=65;E={};E.EntityRespawned=Life;Send(E);
 TestEqual(TEXT("old target life cannot count as attacking new life"),Count().Get(1),uint64(0));
 N->OnDisconnected.Broadcast(TEXT("reset"));TestFalse(TEXT("disconnect clears evidence"),Count().IsSet());
 return true;
}
#endif
