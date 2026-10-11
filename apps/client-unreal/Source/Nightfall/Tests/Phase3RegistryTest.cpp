#include "Misc/AutomationTest.h"
#include "Bot/BotPredicates.h"
#include "Bot/BotSteps.h"
#include "HAL/FileManager.h"
#include "HAL/IConsoleManager.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FPhase3RegistryTest,"Nightfall.Skill.Phase3ScenarioRegistry",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FPhase3RegistryTest::RunTest(const FString& Parameters)
{
 FBotPredicateRegistry R;R.RegisterBuiltins();
 // Exactly the same gated commands and command lookup as a -BotScenario runner. No execution.
 const auto Commands=BotSteps::RegisterTestOnlyCommands();
 TArray<FString> Files;IFileManager::Get().FindFilesRecursive(Files,*FPaths::Combine(FPaths::ProjectDir(),TEXT("Scenarios")),TEXT("3-*.nfs"),true,false);
 TestTrue(TEXT("canonical and nested Phase 3 sources found"),Files.Num()>=38);
 for(const auto& Path:Files)
 {
  FString Text;TestTrue(TEXT("scenario readable"),FFileHelper::LoadFileToString(Text,*Path));FBotScenario Scenario;TArray<FString> Errors;
  const bool Parsed=BotScenario::Parse(Path,Text,R,[](const FString& Name){return IConsoleManager::Get().FindConsoleObject(*Name)!=nullptr;},Scenario,Errors);
  TestTrue(Path+TEXT(": ")+FString::Join(Errors,TEXT("; ")),Parsed);
 }
 for(auto* Command:Commands)IConsoleManager::Get().UnregisterConsoleObject(Command,false);
 for(const TCHAR* Text:{TEXT("regen_pulses >= 1"),TEXT("moving_regen_pulses == 0"),TEXT("regen_matches_oracle"),TEXT("moving_regen_matches_oracle"),TEXT("death_record_matches_fixture"),TEXT("death_record_matches_before_disconnect"),TEXT("restore_did_not_reapply_hooks"),TEXT("skill_hate_outcome_coherent"),TEXT("skill_damage_matches_oracle 1177"),TEXT("skill_effect_matches_oracle 1184"),TEXT("stat_matches_oracle"),TEXT("resurrect_refund_matches_oracle")})
 {FString Error;const auto Tokens=FBotPredicateRegistry::Tokenize(Text);TestNull(TEXT("replay-only predicate absent from client registry"),R.Find(Tokens[0]));TestFalse(TEXT("replay-only grammar rejected"),bool(R.Parse(Tokens,Error)));}
 for(const TCHAR* Text:{TEXT("regen_pulses"),TEXT("regen_pulses >= -1"),TEXT("regen_pulses >= 18446744073709551616"),TEXT("skill_damage_matches_oracle 0"),TEXT("skill_effect_matches_oracle"),TEXT("stat_matches_oracle 1"),TEXT("invented_phase3_predicate")})
 {FString Error;TestFalse(TEXT("invalid grammar fails closed"),bool(R.Parse(FBotPredicateRegistry::Tokenize(Text),Error)));}
 return true;
}
#endif
