#pragma once

#include "CoreMinimal.h"
#include "UObject/Object.h"
#include "AuthTestListener.generated.h"

/** Records UAuthSubsystem's dynamic delegates for automation tests (they only bind UFUNCTIONs). */
UCLASS(Transient)
class UNightfallAuthTestListener : public UObject
{
	GENERATED_BODY()

public:
	int32 LoginRequiredCount = 0;
	int32 LoggedInCount = 0;
	int32 FailedCount = 0;
	FString LastCode;
	FString LastUri;
	FString LastFailure;

	UFUNCTION()
	void HandleLoginRequired(const FString& Code, const FString& Uri)
	{
		++LoginRequiredCount;
		LastCode = Code;
		LastUri = Uri;
	}

	UFUNCTION()
	void HandleLoggedIn() { ++LoggedInCount; }

	UFUNCTION()
	void HandleLoginFailed(const FString& Reason)
	{
		++FailedCount;
		LastFailure = Reason;
	}
};
