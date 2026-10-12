#pragma once
#include "CoreMinimal.h"
class FBotPredicateRegistry;
namespace ItemBotPredicates
{
 void Register(FBotPredicateRegistry& Registry);
 bool ValidateCommand(const TArray<FString>& Tokens, FString& Error);
}
