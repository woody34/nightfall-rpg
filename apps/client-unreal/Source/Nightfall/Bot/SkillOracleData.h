#pragma once
#include "CoreMinimal.h"
#include "Net/ProtoCodec.h"
class FJsonObject;

/** Test-only frozen E1 lookup. Never calculates a gameplay result or selects by output. */
struct NIGHTFALL_API FSkillOracleData
{
 static const FSkillOracleData& Get();
 static FSkillOracleData Load(const FString& DataDir);
 TOptional<uint32> KarmaGain(uint32 PkBefore) const;
 TOptional<uint32> KarmaLost(uint32 LevelBefore,uint64 XpCredit) const;
 TOptional<bool> Revival(const FStatsChanged& Dead,const FStatsChanged& Revived) const;
 FString Error;
private:
 TSharedPtr<FJsonObject> Formulas;
};
