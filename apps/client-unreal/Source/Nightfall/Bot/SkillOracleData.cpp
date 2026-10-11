#include "SkillOracleData.h"
#include "Auth/AuthCrypto.h"
#include "Misc/CommandLine.h"
#include "Misc/FileHelper.h"
#include "Misc/Parse.h"
#include "Misc/Paths.h"
#include "Serialization/JsonSerializer.h"

namespace SkillOracleDetail
{
 // E1 codex/phase3-data533608c. Deliberately pin the entire oracle, including expected outputs.
 const TCHAR* OracleSha=TEXT("0a7380d0b5086c52c92bb075df475b6e3a793dbedc3b2ef3bd8fe84b6830e3c9");
 FString Hash(const TArray<uint8>& Bytes){const auto Digest=NightfallAuthCrypto::Sha256(Bytes.GetData(),Bytes.Num());return BytesToHex(Digest.GetData(),Digest.Num()).ToLower();}
 bool Integer(const TSharedPtr<FJsonObject>& O,const TCHAR* Key,uint64& Out)
 {
  double N=0;if(!O || !O->TryGetNumberField(Key,N) || !FMath::IsFinite(N) || N<0 || N>9007199254740991.0 || FMath::FloorToDouble(N)!=N)return false;Out=uint64(N);return true;
 }
 bool Matches(const TSharedPtr<FJsonObject>& O,const TCHAR* Key,uint64 Input){uint64 V=0;return Integer(O,Key,V) && V==Input;}
 TOptional<uint32> Result(const TSharedPtr<FJsonObject>& O){uint64 V=0;return Integer(O,TEXT("rounded_result"),V) && V<=MAX_uint32?TOptional<uint32>(uint32(V)):TOptional<uint32>();}
}
FSkillOracleData FSkillOracleData::Load(const FString& Dir)
{
 using namespace SkillOracleDetail;FSkillOracleData D;TArray<uint8> Bytes;
 if(!FFileHelper::LoadFileToArray(Bytes,*FPaths::Combine(Dir,TEXT("fixtures/skill-oracle.json"))) || Hash(Bytes)!=OracleSha){D.Error=TEXT("missing or changed pinned E1 skill-oracle.json");return D;}
 FString Text;if(!FFileHelper::LoadFileToString(Text,*FPaths::Combine(Dir,TEXT("fixtures/skill-oracle.json")))){D.Error=TEXT("unreadable E1 oracle");return D;}
 TSharedPtr<FJsonObject> Root;auto Reader=TJsonReaderFactory<>::Create(Text);
 if(!FJsonSerializer::Deserialize(Reader,Root) || !Matches(Root,TEXT("schema"),1)){D.Error=TEXT("unsupported E1 oracle schema");return D;}
 const TSharedPtr<FJsonObject>* Revisions=nullptr;const TSharedPtr<FJsonObject>* Hashes=nullptr;
 if(!Root->TryGetObjectField(TEXT("source_revisions"),Revisions) || !Root->TryGetObjectField(TEXT("inputs_sha256"),Hashes) || (*Revisions)->GetStringField(TEXT("l2j_game"))!=TEXT("abfde0490ac52a2106e884b7242c0276c44cdf76") || (*Revisions)->GetStringField(TEXT("l2j_datapack"))!=TEXT("3ca488dd2bd0bfaca43e378886a3c2e37968153a")){D.Error=TEXT("E1 source revision mismatch");return D;}
 for(const TCHAR* Relative:{TEXT("npcs/keltir.toml"),TEXT("scripts/skill_oracle.py"),TEXT("tables/formulas.toml"),TEXT("tables/pvp.toml"),TEXT("tables/regen.toml"),TEXT("tables/stat_bonus.toml")})
 {
  FString Expected;TArray<uint8> Input;if(!(*Hashes)->TryGetStringField(Relative,Expected) || !FFileHelper::LoadFileToArray(Input,*FPaths::Combine(Dir,Relative)) || Hash(Input)!=Expected){D.Error=FString(TEXT("E1 input hash mismatch: "))+Relative;return D;}
 }
 const TSharedPtr<FJsonObject>* F=nullptr;if(!Root->TryGetObjectField(TEXT("formulas"),F)){D.Error=TEXT("missing E1 formulas");return D;}D.Formulas=*F;return D;
}
const FSkillOracleData& FSkillOracleData::Get()
{
 static const FSkillOracleData D=[](){FString Dir;if(!FParse::Value(FCommandLine::Get(),TEXT("-BotDataDir="),Dir))Dir=FPaths::Combine(FPaths::ProjectDir(),TEXT("../../packages/data"));FPaths::CollapseRelativeDirectories(Dir);return Load(FPaths::ConvertRelativePathToFull(Dir));}();return D;
}
TOptional<uint32> FSkillOracleData::KarmaGain(uint32 PkBefore) const
{
 using namespace SkillOracleDetail;if(!Formulas)return {};const TArray<TSharedPtr<FJsonValue>>* Rows=nullptr;if(!Formulas->TryGetArrayField(TEXT("calculateKarmaGain"),Rows))return {};
 TOptional<uint32> Found;
 for(const auto& V:*Rows){const auto R=V->AsObject();const auto I=R->GetObjectField(TEXT("inputs"));bool Summon=true;if(Matches(I,TEXT("pk_count"),PkBefore) && I->TryGetBoolField(TEXT("is_summon"),Summon) && !Summon){if(Found.IsSet())return {};Found=Result(R);}}
 return Found;
}
TOptional<uint32> FSkillOracleData::KarmaLost(uint32 LevelBefore,uint64 XpCredit) const
{
 using namespace SkillOracleDetail;if(!Formulas)return {};const TArray<TSharedPtr<FJsonValue>>* Rows=nullptr;if(!Formulas->TryGetArrayField(TEXT("calculateKarmaLost"),Rows))return {};
 TOptional<uint32> Found;
 // The hash-pinned generator uses the pinned level's multiplier. No division or interpolation here.
 for(const auto& V:*Rows){const auto R=V->AsObject();const auto I=R->GetObjectField(TEXT("inputs"));if(Matches(I,TEXT("player_level"),LevelBefore) && Matches(I,TEXT("exp"),XpCredit)){if(Found.IsSet())return {};Found=Result(R);}}
 return Found;
}
TOptional<bool> FSkillOracleData::Revival(const FStatsChanged& Dead,const FStatsChanged& Revived) const
{
 using namespace SkillOracleDetail;if(!Formulas)return {};const TArray<TSharedPtr<FJsonValue>>* Rows=nullptr;if(!Formulas->TryGetArrayField(TEXT("doRevive"),Rows))return {};
 TOptional<bool> Found;
 for(const auto& V:*Rows)
 {
  const auto R=V->AsObject();const auto I=R->GetObjectField(TEXT("inputs"));
  if(!Matches(I,TEXT("max_hp"),Dead.MaxHp) || !Matches(I,TEXT("current_hp"),Dead.Hp) || !Matches(I,TEXT("max_mp"),Dead.MaxMp) || !Matches(I,TEXT("current_mp"),Dead.Mp) || !Matches(I,TEXT("max_cp"),Dead.MaxCp) || !Matches(I,TEXT("current_cp"),Dead.Cp))continue;
  if(Found.IsSet())return {};const auto Expected=R->GetObjectField(TEXT("rounded_result"));Found=Matches(Expected,TEXT("hp"),Revived.Hp) && Matches(Expected,TEXT("mp"),Revived.Mp) && Matches(Expected,TEXT("cp"),Revived.Cp);
 }
 return Found;
}
