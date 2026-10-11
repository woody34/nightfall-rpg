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
#endif
