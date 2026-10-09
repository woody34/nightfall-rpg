#pragma once

#include "CoreMinimal.h"
#include "Engine/Engine.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "Misc/AutomationTest.h"
#include "Misc/CommandLine.h"

#if WITH_DEV_AUTOMATION_TESTS

/**
 * A standalone game instance for the duration of a test, so game-instance subsystems (TurboLink's
 * manager, ours) get a real owner exactly as in a packaged game. Shut down on scope exit.
 */
struct FScopedTestGameInstance
{
	UGameInstance* GameInstance = nullptr;

	FScopedTestGameInstance()
	{
		GameInstance = NewObject<UGameInstance>(GEngine);
		GameInstance->AddToRoot();
		GameInstance->InitializeStandalone();
	}

	~FScopedTestGameInstance()
	{
		UWorld* World = GameInstance->GetWorld();
		GameInstance->Shutdown();
		if (World != nullptr)
		{
			GEngine->DestroyWorldContext(World);
			World->DestroyWorld(false);
		}
		GameInstance->RemoveFromRoot();
	}

	FScopedTestGameInstance(const FScopedTestGameInstance&) = delete;
	FScopedTestGameInstance& operator=(const FScopedTestGameInstance&) = delete;

	template <typename T>
	T* Get() const { return GameInstance->GetSubsystem<T>(); }
};

namespace NightfallTest
{
	/** `-RequireLiveApi` (CI): a live test that cannot reach the API fails instead of skipping. */
	inline bool RequireLiveApi() { return FParse::Param(FCommandLine::Get(), TEXT("RequireLiveApi")); }

	/**
	 * TurboLink logs an Error ("CallRpcError: Unavailable") when the API is down, and automation
	 * counts logged errors as test failures. Declare it expected (-1: any number of times, or none) so
	 * the skip/fail decision is made only by SkipLive.
	 */
	inline void AllowApiUnavailableLogs(FAutomationTestBase& Test)
	{
		Test.AddExpectedError(TEXT("CallRpcError: Unavailable"), EAutomationExpectedErrorFlags::Contains, -1);
	}

	/**
	 * Call when a live test finds the API (or its auth) unavailable. Without -RequireLiveApi it
	 * warns and returns true (skip counts as pass); with it, it reports an error and returns false.
	 * Usage: `return NightfallTest::SkipLive(*this, TEXT("..."));`
	 */
	inline bool SkipLive(FAutomationTestBase& Test, const FString& Why)
	{
		if (RequireLiveApi())
		{
			Test.AddError(FString::Printf(TEXT("%s [-RequireLiveApi: a down API is a failure]"), *Why));
			return false;
		}
		Test.AddWarning(Why);
		return true;
	}
}

#endif
