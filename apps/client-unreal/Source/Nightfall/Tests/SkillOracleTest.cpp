#include "Misc/AutomationTest.h"
#include "Bot/SkillOracleData.h"
#include "Auth/AuthCrypto.h"
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
 // A generator-only change refreshes its manifest hash without changing any vectors.
 const FString OraclePath=FPaths::Combine(Dir,TEXT("fixtures/skill-oracle.json"));
 const FString ScriptPath=FPaths::Combine(Dir,TEXT("scripts/skill_oracle.py"));
 FString Oracle,Script;TestTrue(TEXT("read copied oracle"),FFileHelper::LoadFileToString(Oracle,*OraclePath));TestTrue(TEXT("read copied generator"),FFileHelper::LoadFileToString(Script,*ScriptPath));
 TArray<uint8> ScriptBytes;TestTrue(TEXT("read generator bytes"),FFileHelper::LoadFileToArray(ScriptBytes,*ScriptPath));
 auto Sha=[](const TArray<uint8>& Bytes){const auto Digest=NightfallAuthCrypto::Sha256(Bytes.GetData(),Bytes.Num());return BytesToHex(Digest.GetData(),Digest.Num()).ToLower();};
 const FString OldScriptSha=Sha(ScriptBytes);Script+=TEXT("\n# Manifest-only regression input.\n");TestTrue(TEXT("update copied generator"),FFileHelper::SaveStringToFile(Script,*ScriptPath,FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM));
 TestFalse(TEXT("unrefreshed generator hash fails closed"),FSkillOracleData::Load(Dir).KarmaGain(0).IsSet());
 TestTrue(TEXT("read changed generator bytes"),FFileHelper::LoadFileToArray(ScriptBytes,*ScriptPath));FString Refreshed=Oracle;
 TestEqual(TEXT("refresh exactly the generator manifest hash"),Refreshed.ReplaceInline(*OldScriptSha,*Sha(ScriptBytes)),1);TestTrue(TEXT("write refreshed manifest"),FFileHelper::SaveStringToFile(Refreshed,*OraclePath,FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM));
 const auto RefreshedData=FSkillOracleData::Load(Dir);TestTrue(TEXT("unchanged vectors accept refreshed input manifest: ")+RefreshedData.Error,RefreshedData.Error.IsEmpty());TestEqual(TEXT("refreshed metadata keeps exact expected result"),RefreshedData.KarmaGain(0).Get(0),720u);
 FString ChangedVectors=Refreshed;TestTrue(TEXT("change pinned vector output"),ChangedVectors.ReplaceInline(TEXT("\"rounded_result\": 720"),TEXT("\"rounded_result\": 721"))>0);TestTrue(TEXT("write changed vector"),FFileHelper::SaveStringToFile(ChangedVectors,*OraclePath,FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM));
 const auto BadVectors=FSkillOracleData::Load(Dir);TestFalse(TEXT("changed vector blocks lookup"),BadVectors.KarmaGain(0).IsSet());TestTrue(TEXT("vector pin reason"),BadVectors.Error.Contains(TEXT("pinned E1 formula vectors")));
 TestTrue(TEXT("restore pinned vectors"),FFileHelper::SaveStringToFile(Refreshed,*OraclePath,FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM));
 FFileHelper::SaveStringToFile(TEXT("changed input"),*FPaths::Combine(Dir,TEXT("tables/regen.toml")));const auto BadInput=FSkillOracleData::Load(Dir);TestFalse(TEXT("changed input blocks lookup"),BadInput.KarmaGain(0).IsSet());TestTrue(TEXT("hash reason"),BadInput.Error.Contains(TEXT("input hash")));
 FFileHelper::SaveStringToFile(TEXT("{}"),*FPaths::Combine(Dir,TEXT("fixtures/skill-oracle.json")));TestFalse(TEXT("changed expected outputs cannot pass"),FSkillOracleData::Load(Dir).KarmaGain(0).IsSet());IFileManager::Get().DeleteDirectory(*Dir,false,true);
 return true;
}
#endif
