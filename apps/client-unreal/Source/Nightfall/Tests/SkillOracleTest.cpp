#include "Misc/AutomationTest.h"
#include "Bot/SkillOracleData.h"
#include "HAL/FileManager.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillOracleTest,"Nightfall.Skill.FrozenOracle",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FSkillOracleTest::RunTest(const FString& Parameters)
{
 const auto& D=FSkillOracleData::Get();TestTrue(TEXT("pinned E1 and inputs load: ")+D.Error,D.Error.IsEmpty());
 TestEqual(TEXT("PK zero exact input"),D.KarmaGain(0).Get(0),720u);TestEqual(TEXT("PK one distinct input"),D.KarmaGain(1).Get(0),1080u);TestFalse(TEXT("no extrapolation"),D.KarmaGain(987654).IsSet());
 TestEqual(TEXT("exact XP credit input"),D.KarmaLost(1,100).Get(0),4u);TestFalse(TEXT("no nearest vector"),D.KarmaLost(1,101).IsSet());
 FStatsChanged Dead;Dead.MaxHp=100;Dead.Mp=25;Dead.MaxMp=50;Dead.Cp=40;Dead.MaxCp=80;FStatsChanged Revived=Dead;Revived.Hp=65;
 TestTrue(TEXT("six exact inputs select doRevive"),D.Revival(Dead,Revived).Get(false));Revived.Cp=39;TestFalse(TEXT("CP mutation rejected"),D.Revival(Dead,Revived).Get(true));Dead.MaxMp=51;TestFalse(TEXT("missing input cannot select by output"),D.Revival(Dead,Revived).IsSet());
 const FString Dir=FPaths::Combine(FPaths::ProjectSavedDir(),TEXT("Automation/SkillOracle"));IFileManager::Get().DeleteDirectory(*Dir,false,true);
 const FString Source=FPaths::Combine(FPaths::ProjectDir(),TEXT("../../packages/data"));
 for(const TCHAR* Rel:{TEXT("fixtures/skill-oracle.json"),TEXT("npcs/keltir.toml"),TEXT("scripts/skill_oracle.py"),TEXT("tables/formulas.toml"),TEXT("tables/pvp.toml"),TEXT("tables/regen.toml"),TEXT("tables/stat_bonus.toml")})
 {const FString To=FPaths::Combine(Dir,Rel);IFileManager::Get().MakeDirectory(*FPaths::GetPath(To),true);TestEqual(TEXT("copy oracle input"),IFileManager::Get().Copy(*To,*FPaths::Combine(Source,Rel)),uint32(COPY_OK));}
 TestTrue(TEXT("complete copied fixture valid"),FSkillOracleData::Load(Dir).Error.IsEmpty());
 FFileHelper::SaveStringToFile(TEXT("changed input"),*FPaths::Combine(Dir,TEXT("tables/regen.toml")));const auto BadInput=FSkillOracleData::Load(Dir);TestFalse(TEXT("changed input blocks lookup"),BadInput.KarmaGain(0).IsSet());TestTrue(TEXT("hash reason"),BadInput.Error.Contains(TEXT("input hash")));
 FFileHelper::SaveStringToFile(TEXT("{}"),*FPaths::Combine(Dir,TEXT("fixtures/skill-oracle.json")));TestFalse(TEXT("changed expected outputs cannot pass"),FSkillOracleData::Load(Dir).KarmaGain(0).IsSet());IFileManager::Get().DeleteDirectory(*Dir,false,true);
 return true;
}
#endif
