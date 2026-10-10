#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Net/NetClientSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Bot/SkillAuditSubsystem.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillHydrationTest,"Nightfall.Skill.HydrationAbsenceAndOrder",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillHydrationTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* Audit=Scope.Get<USkillAuditSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 FBotPredicateRegistry Registry;Registry.RegisterBuiltins();FBotContext C;C.GameInstance=Scope.GameInstance;
 auto Value=[&](const TCHAR* Text){FString Error;auto P=Registry.Parse(FBotPredicateRegistry::Tokenize(Text),Error);TestTrue(TEXT("predicate binds"),bool(P));return P?P(C):FBotPredicateValue{false,TEXT("unknown")};};
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};
 auto Spawn=[&](const TCHAR* Id,uint64 Tick,uint32 Inc=1){FEntitySpawn S;S.EntityId=Id;S.LifeIncarnation=Inc;S.StateTick=Tick;S.Kind=1;FWorldEvent E;E.Spawn=S;Send(E);};
 Spawn(TEXT("owner"),10);
 for(const TCHAR* Text:{TEXT("buff_inactive 3"),TEXT("entity_effect_inactive own 3"),TEXT("effect_count == 0"),TEXT("cast_state == idle")})
 {const auto V=Value(Text);TestFalse(TEXT("identity cannot prove empty state"),V.bTrue);TestEqual(TEXT("absence remains unknown between baseline frames"),V.Observed,FString(TEXT("unknown")));}
 FSkillStateSync Sync;Sync.Entity=TEXT("owner");Sync.Incarnation=1;Sync.Tick=Sync.BaselineTick=10;Sync.bBaseline=true;FWorldEvent E;E.SkillStateSync=Sync;Send(E);
 TestFalse(TEXT("owner sync does not close remaining effect/cast hydration"),Value(TEXT("cast_state == idle")).bTrue);
 FEffectApplied Effect;Effect.Target=TEXT("owner");Effect.TargetIncarnation=1;Effect.SkillId=3;Effect.EffectId=1;Effect.Pool=1;Effect.Tick=Effect.BaselineTick=10;Effect.bBaseline=true;Effect.EndTick=30;E={};E.EffectApplied=Effect;Send(E);
 TestFalse(TEXT("hydrated effect is not absent"),Value(TEXT("buff_inactive 3")).bTrue);
 FSkillCastStarted Cast;Cast.Caster=TEXT("owner");Cast.CasterIncarnation=1;Cast.SkillId=3;Cast.CastId=1;Cast.Tick=Cast.BaselineTick=10;Cast.StartTick=8;Cast.CompleteTick=20;Cast.bBaseline=true;E={};E.SkillCastStarted=Cast;Send(E);
 FServerMessage Close;Close.Ack=FAck{1,11};N->DispatchServerMessage(Close);
 TestTrue(TEXT("later stream tick establishes hydrated cast"),Value(TEXT("cast_state == casting")).bTrue);TestTrue(TEXT("hydrated effect count"),Value(TEXT("effect_count == 1")).bTrue);
 TestTrue(TEXT("later tick proves absent other skill"),Value(TEXT("buff_inactive 4")).bTrue);
 // Audit subjects as well as optional endpoints, before projection filtering.
 auto ValidReference=[&]{Cast.Caster=TEXT("owner");Cast.CasterIncarnation=1;Cast.Target=TEXT("owner");Cast.TargetIncarnation=1;Cast.Tick=Cast.BaselineTick=20;Cast.StartTick=18;Cast.CompleteTick=30;E={};E.SkillCastStarted=Cast;Send(E);};
 for(int32 Missing=0;Missing<2;++Missing)
 {
  Audit->Reset();Spawn(TEXT("owner"),20);
  if(Missing==0){Cast.Caster=TEXT("missing");Cast.CasterIncarnation=1;Cast.Target=TEXT("owner");Cast.TargetIncarnation=1;Cast.Tick=Cast.BaselineTick=20;E={};E.SkillCastStarted=Cast;Send(E);}
  else {Effect.Target=TEXT("missing");Effect.Caster=TEXT("owner");Effect.CasterIncarnation=1;Effect.Tick=Effect.BaselineTick=20;E={};E.EffectApplied=Effect;Send(E);}
  ValidReference();TestFalse(TEXT("later valid reference cannot erase missing subject violation"),Value(TEXT("spawn_before_skill_references")).bTrue);
 }
 Audit->Reset();Spawn(TEXT("owner"),20,2);Cast.Caster=TEXT("owner");Cast.CasterIncarnation=1;Cast.Target=TEXT("missing");Cast.Tick=Cast.BaselineTick=10;E={};E.SkillCastStarted=Cast;Send(E);
 Cast.CasterIncarnation=2;Cast.TargetIncarnation=2;Cast.Tick=Cast.BaselineTick=20;Cast.Target=TEXT("owner");Cast.StartTick=18;Cast.CompleteTick=30;E.SkillCastStarted=Cast;Send(E);
 TestTrue(TEXT("old-life stale references do not poison valid ordering"),Value(TEXT("spawn_before_skill_references")).bTrue);
 return true;
}
#endif
