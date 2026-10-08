#pragma once

#include "CoreMinimal.h"
#include "Subsystems/WorldSubsystem.h"
#include "WorldProxySubsystem.generated.h"

struct FEntitySpawn;
struct FEntityDespawn;

/** Spawns and destroys ARemoteEntityActor proxies in response to server events. */
UCLASS()
class NIGHTFALL_API UWorldProxySubsystem : public UWorldSubsystem
{
	GENERATED_BODY()

public:
	virtual void OnWorldBeginPlay(UWorld& InWorld) override;
	virtual void Deinitialize() override;

	/** Blueprint class to spawn for entities; set from the GameMode or a config actor. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	TSubclassOf<class ARemoteEntityActor> EntityClass;

private:
	void HandleSpawn(const FEntitySpawn& Spawn);
	void HandleDespawn(const FEntityDespawn& Despawn);

	UPROPERTY()
	TMap<FString, TObjectPtr<class ARemoteEntityActor>> Entities;

	FDelegateHandle SpawnHandle;
	FDelegateHandle DespawnHandle;
};
