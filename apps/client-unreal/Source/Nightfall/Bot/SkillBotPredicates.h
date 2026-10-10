#pragma once
#include "CoreMinimal.h"
class FBotPredicateRegistry;
namespace SkillBotPredicates
{
	void Register(FBotPredicateRegistry& Registry);
	bool ParseUnsigned(const FString& Text, uint64& Out);
	bool ValidateCommand(const TArray<FString>& Tokens, FString& Error);
}
