#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Skills/SkillStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Bot/BotSteps.h"
#include "Bot/SkillBotPredicates.h"
#include "Blueprint/UserWidget.h"
#if WITH_DEV_AUTOMATION_TESTS
namespace SkillBotTests
{
 constexpr EAutomationTestFlags Flags=EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillPredicatesTest,"Nightfall.Skill.BotContract",SkillBotTests::Flags)
bool FSkillPredicatesTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;FBotPredicateRegistry R;R.RegisterBuiltins();FBotContext C;C.GameInstance=Scope.GameInstance;
 FString Error;uint64 Number=0;
 TestTrue(TEXT("uint64 max accepted exactly"),SkillBotPredicates::ParseUnsigned(TEXT("18446744073709551615"),Number));TestEqual(TEXT("exact integer"),Number,MAX_uint64);
 for(const TCHAR* Bad:{TEXT("18446744073709551616"),TEXT("-1"),TEXT("1.5"),TEXT("1e2")})TestFalse(TEXT("invalid integer rejected"),SkillBotPredicates::ParseUnsigned(Bad,Number));
 for(const TCHAR* Text:{TEXT("own_sp == 0"),TEXT("cast_state == idle"),TEXT("skill_ready 3"),TEXT("buff_inactive 3"),TEXT("learn_rpc_error == none")})
 {
  auto P=R.Parse(FBotPredicateRegistry::Tokenize(Text),Error);TestTrue(TEXT("predicate registered"),bool(P));if(P)TestFalse(TEXT("unknown scope cannot pass"),P(C).bTrue);
 }
 TestTrue(TEXT("conditional retry uses received life"),SkillBotPredicates::ValidateCommand({TEXT("nf.CastIf"),TEXT("1177"),TEXT("target_alive")},Error));
 TestTrue(TEXT("conditional retry uses received effect"),SkillBotPredicates::ValidateCommand({TEXT("nf.CastIf"),TEXT("1184"),TEXT("effect_absent")},Error));
 TestFalse(TEXT("unsupported conditional cannot become a blind cast"),SkillBotPredicates::ValidateCommand({TEXT("nf.CastIf"),TEXT("1184"),TEXT("always")},Error));
 FBotScenario Scenario;TArray<FString> Errors;
 TestFalse(TEXT("invalid rank fails scenario parse"),BotScenario::Parse(TEXT("bad"),TEXT("nf.Learn 3 1.2"),R,[](const FString&){return true;},Scenario,Errors));
 Errors.Reset();TestTrue(TEXT("retry key and explicit corpse accepted"),BotScenario::Parse(TEXT("good"),TEXT("nf.Learn 3 2 01970000-0000-7000-8000-000000000030\nnf.Cast 1016 01970000-0000-7000-8000-000000000031\nnf.AcceptResurrect latest decline"),R,[](const FString&){return true;},Scenario,Errors));
 auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();
 N->SetOwnEntityId(TEXT("own"));FEntitySpawn Spawn;Spawn.EntityId=TEXT("own");Spawn.LifeIncarnation=1;Spawn.StateTick=10;S->ApplySpawn(Spawn);
 FStatsChanged Stats;Stats.Entity=TEXT("own");Stats.Incarnation=1;Stats.Tick=Stats.BaselineTick=10;Stats.bBaseline=true;S->ApplyStats(Stats);
 auto Rank=R.Parse({TEXT("learned_rank"),TEXT("3"),TEXT("=="),TEXT("0")},Error);
 TestFalse(TEXT("stats alone cannot prove an unlearned rank"),Rank(C).bTrue);
 return true;
}
#endif
