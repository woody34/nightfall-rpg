#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Skills/SkillStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Bot/BotSteps.h"
#include "Bot/SkillBotPredicates.h"
#include "UI/NightfallSkillDialog.h"
#include "UI/NightfallHud.h"
#include "Blueprint/UserWidget.h"
#if WITH_DEV_AUTOMATION_TESTS
namespace SkillPresentationTests
{
 constexpr EAutomationTestFlags Flags=EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillTrainerTest,"Nightfall.Skill.TrainerAndFrozenReceipt",SkillPresentationTests::Flags)
bool FSkillTrainerTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();N->SetOwnEntityId(TEXT("own"));
 FEntitySpawn Spawn;Spawn.EntityId=TEXT("own");Spawn.LifeIncarnation=1;Spawn.StateTick=10;Spawn.SessionGeneration=1;S->ApplySpawn(Spawn);
 FSkillStateSync Sync;Sync.Entity=TEXT("own");Sync.Incarnation=1;Sync.Tick=Sync.BaselineTick=10;Sync.bBaseline=true;Sync.Sp=500;Sync.Learned.Add({3,1});Sync.Learned.Add({4,1});S->ApplySync(Sync);
 FGrpcNightfallV1GetSkillCatalogResponse Catalog;auto& Executable=Catalog.Skills.AddDefaulted_GetRef();Executable.SkillId=3;Executable.DisplayName=TEXT("Wind");Executable.EffectImplemented=true;
 auto& Deferred=Catalog.Skills.AddDefaulted_GetRef();Deferred.SkillId=4;Deferred.DisplayName=TEXT("Deferred");S->ApplyCatalog(Catalog, {{3,TEXT("A1")},{4,TEXT("A1")}});
 TestEqual(TEXT("hotbar includes received executable learned skills only"),S->AvailableSkills().Num(),1);TestEqual(TEXT("correct slot"),S->Hotbar()[0],3u);
 auto* Dialog=NewObject<UNightfallSkillDialog>(Scope.GameInstance);Dialog->EnsureLayout();Dialog->BindState(S,false);
 FGrpcNightfallV1ListLearnableSkillsResponse Learnable;auto& Row=Learnable.Skills.AddDefaulted_GetRef();Row.Skill.SkillId=3;Row.Skill.SkillLevel=2;Row.Skill.SpCost=100;Row.Eligible=true;S->ApplyLearnable(Learnable);
 Dialog->Select(3,2);TestTrue(TEXT("server eligibility permits learning"),Dialog->CanConfirm());FString Key=Dialog->GetRetryKey();Dialog->Select(3,2);TestEqual(TEXT("selection retains retry UUID"),Dialog->GetRetryKey(),Key);
 Row.Eligible=false;S->ApplyLearnable(Learnable);TestFalse(TEXT("server revocation disables confirm"),Dialog->CanConfirm());
 FGrpcNightfallV1LearnSkillResponse Receipt;Receipt.SkillId=3;Receipt.SkillLevel=2;Receipt.SpRemaining=400;Receipt.Tick=11;Receipt.Hp=99;Receipt.Incarnation=1;
 const uint64 Serial=S->GetOperationSerial();S->CompleteLearn(Serial,{},Receipt,nullptr);S->CompleteLearn(Serial,{},Receipt,nullptr);
 TestTrue(TEXT("identical historical receipts compare equal"),S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst());TestEqual(TEXT("RPC never overwrites live SP"),*S->GetSp(),uint64(500));TestEqual(TEXT("RPC never overwrites live rank"),S->GetLearned().FindRef(3),1u);
 S->Reset();S->CompleteLearn(Serial,{},Receipt,nullptr);TestFalse(TEXT("late callback after reset discarded"),S->HasLearnResult());
 auto* Hud=NewObject<UNightfallHud>(Scope.GameInstance);Hud->EnsureLayout();TestEqual(TEXT("native HUD creates no transient damage cues"),Hud->NumDamageNumbers(),0);
 return true;
}
#endif
