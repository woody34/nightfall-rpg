#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Skills/SkillStateSubsystem.h"
#include "Effects/EffectStateSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Bot/BotPredicates.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillHandoffTest,"Nightfall.Skill.HarnessHandoff",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillHandoffTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();auto* Effects=Scope.Get<UEffectStateSubsystem>();Scope.Get<UCombatStateSubsystem>();
 FBotPredicateRegistry Registry;Registry.RegisterBuiltins();FBotContext Context;Context.GameInstance=Scope.GameInstance;
 auto Holds=[&](const TCHAR* Text){FString Error;auto P=Registry.Parse(FBotPredicateRegistry::Tokenize(Text),Error);TestTrue(TEXT("handoff syntax registered"),bool(P));return P && P(Context).bTrue;};
 for(const TCHAR* Text:{TEXT("initial_mp == 0"),TEXT("entity_dead own"),TEXT("cast_cancel_reason != peace_zone"),TEXT("own_flag != pvp"),TEXT("effect_remove_reason != expired"),TEXT("buff_replacements == 0")})TestFalse(TEXT("unknown cannot pass including inequality"),Holds(Text));
 N->SetOwnEntityId(TEXT("owner"));FEntitySpawn Spawn;Spawn.EntityId=TEXT("owner");Spawn.LifeIncarnation=1;Spawn.SessionGeneration=1;Spawn.StateTick=10;Spawn.Hp=Spawn.MaxHp=100;Spawn.bCombatant=true;
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};FWorldEvent E;E.Spawn=Spawn;Send(E);
 TestTrue(TEXT("received alive entity"),Holds(TEXT("entity_alive own")));TestTrue(TEXT("received initial HP"),Holds(TEXT("initial_hp == 100")));
 FStatsChanged Stats;Stats.Entity=TEXT("owner");Stats.Incarnation=1;Stats.Tick=Stats.BaselineTick=10;Stats.bBaseline=true;Stats.Mp=40;Stats.Cp=12;Stats.Hp=100;Stats.MaxHp=100;E={};E.StatsChanged=Stats;Send(E);
 Stats.Tick=11;Stats.bBaseline=false;Stats.Mp=30;Stats.Cp=2;E.StatsChanged=Stats;Send(E);
 TestTrue(TEXT("initial MP frozen"),Holds(TEXT("initial_mp == 40")));TestTrue(TEXT("initial CP frozen"),Holds(TEXT("initial_cp == 12")));
 FSkillCastStarted Cast;Cast.Caster=TEXT("owner");Cast.CasterIncarnation=1;Cast.CastId=3;Cast.SkillId=1177;Cast.Tick=12;E={};E.SkillCastStarted=Cast;Send(E);TestTrue(TEXT("skill-specific entity cast"),Holds(TEXT("entity_casting own 1177")));
 FSkillCastCancelled Cancel;Cancel.Caster=TEXT("owner");Cancel.CasterIncarnation=1;Cancel.CastId=3;Cancel.Tick=13;Cancel.Reason=9;E={};E.SkillCastCancelled=Cancel;Send(E);Send(E);
 TestTrue(TEXT("received own cancellation reason"),Holds(TEXT("cast_cancel_reason == peace_zone")));TestTrue(TEXT("received observer cancellation reason"),Holds(TEXT("observed_cast_cancel_reason == peace_zone")));
 Cancel.CasterIncarnation=0;Cancel.Tick=14;Cancel.Reason=1;E.SkillCastCancelled=Cancel;Send(E);TestTrue(TEXT("stale cancellation cannot replace reason"),Holds(TEXT("cast_cancel_reason == peace_zone")));
 FEffectApplied A;A.Target=TEXT("owner");A.TargetIncarnation=1;A.EffectId=5;A.SkillId=1068;A.Pool=1;A.Tick=14;A.EndTick=100;E={};E.EffectApplied=A;Send(E);
 FEffectRemoved Removed;Removed.Target=TEXT("owner");Removed.TargetIncarnation=1;Removed.EffectId=5;Removed.Reason=2;Removed.Tick=15;E={};E.EffectRemoved=Removed;Send(E);Send(E);
 TestFalse(TEXT("removal alone cannot prove completed replacement"),Holds(TEXT("replacement_remove_before_apply")));TestTrue(TEXT("replacement count waits for new ID"),Holds(TEXT("buff_replacements == 0")));
 A.EffectId=6;A.Tick=15;E={};E.EffectApplied=A;Send(E);Send(E);TestTrue(TEXT("received remove-before-new-ID order"),Holds(TEXT("replacement_remove_before_apply")));TestTrue(TEXT("duplicate envelopes counted once"),Holds(TEXT("buff_replacements == 1")));
 TestTrue(TEXT("last removal reason received"),Holds(TEXT("effect_remove_reason == replaced")));
 N->OnDisconnected.Broadcast(TEXT("test"));TestFalse(TEXT("scope reset forgets history"),Holds(TEXT("buff_replacements == 0")));TestFalse(TEXT("scope reset forgets initial MP"),S->GetInitialMp().IsSet());TestFalse(TEXT("scope reset forgets removal"),Effects->GetRemoveReason().IsSet());
 return true;
}
#endif
