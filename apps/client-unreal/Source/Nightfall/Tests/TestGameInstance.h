#pragma once

#include "CoreMinimal.h"
#include "Engine/Engine.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"

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

#endif
