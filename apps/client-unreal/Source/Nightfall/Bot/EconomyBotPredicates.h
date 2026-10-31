#pragma once
#include "CoreMinimal.h"
class FBotPredicateRegistry;
/** Phase 5 §5.3 bot vocabulary (Scripts/phase5-grammar.py): every COMMANDS entry and every PREDICATES entry except the Phase 1 builtin `rejected`. */
namespace EconomyBotPredicates
{
 void Register(FBotPredicateRegistry& Registry);
 bool ValidateCommand(const TArray<FString>& Tokens, FString& Error);
 /** Names registered here, in phase5-grammar.py order (the harness cross-check reads this list). */
 const TArray<FString>& PredicateNames();
 const TArray<FString>& CommandNames();
}
