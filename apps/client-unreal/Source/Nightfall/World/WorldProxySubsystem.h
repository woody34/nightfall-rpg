#pragma once

#include "CoreMinimal.h"
#include "Subsystems/WorldSubsystem.h"
#include "WorldProxySubsystem.generated.h"

struct FClassChanged;
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

	/** Blueprint class to spawn for entities; ANightfallGameMode sets it from its EntityClass in InitGame. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	TSubclassOf<class ARemoteEntityActor> EntityClass;

	/** NPC template id (EntitySpawn.template_id, e.g. "keltir") -> proxy class. Others use EntityClass. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	TMap<FString, TSubclassOf<class ARemoteEntityActor>> TemplateClasses;

	/** Proxy class for other players; EntityClass when unset. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	TSubclassOf<class ARemoteEntityActor> PlayerClass;

	/** The class a spawn gets: template class for NPCs, PlayerClass for players, else EntityClass. */
	TSubclassOf<class ARemoteEntityActor> ClassFor(const FEntitySpawn& Spawn) const;

	/** The live proxies by entity id (as the server spelled it). */
	const TMap<FString, TObjectPtr<class ARemoteEntityActor>>& GetProxies() const { return Entities; }

private:
	void HandleClassChanged(const FClassChanged& Changed);
	void UpdateClassPresentation(ARemoteEntityActor* Actor, uint32 ClassId, bool bTransferred);
	void HandleSpawn(const FEntitySpawn& Spawn);
	void HandleDespawn(const FEntityDespawn& Despawn);

	UPROPERTY()
	TMap<FString, TObjectPtr<class ARemoteEntityActor>> Entities;

	FDelegateHandle SpawnHandle;
	FDelegateHandle DespawnHandle;
	FDelegateHandle ClassHandle;
	FDelegateHandle CatalogueHandle;
	UFUNCTION()
	void ResetProxies();
	UFUNCTION()
	void HandleDisconnected(const FString& Reason);
	void RefreshClassTitles();
};
