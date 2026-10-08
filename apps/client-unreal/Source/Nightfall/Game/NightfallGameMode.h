#pragma once

#include "CoreMinimal.h"
#include "GameFramework/GameModeBase.h"
#include "NightfallGameMode.generated.h"

class ARemoteEntityActor;

/**
 * World-map game mode (L_TestZone via BP_NightfallGameMode). Purely local: there is no UE server.
 * Hands EntityClass to UWorldProxySubsystem before any actor begins play.
 */
UCLASS()
class NIGHTFALL_API ANightfallGameMode : public AGameModeBase
{
	GENERATED_BODY()

public:
	ANightfallGameMode();

	virtual void InitGame(const FString& MapName, const FString& Options, FString& ErrorMessage) override;

	/** Spawns the pawn on the player's own EntitySpawn position when known (PlayerStart may be anywhere). */
	virtual APawn* SpawnDefaultPawnAtTransform_Implementation(AController* NewPlayer, const FTransform& SpawnTransform) override;

	/** Proxy class for server entities (BP_RemoteEntity). */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall")
	TSubclassOf<ARemoteEntityActor> EntityClass;
};

/** Login-map game mode (L_Login): no pawn, ANightfallLoginPlayerController shows the login screen. */
UCLASS()
class NIGHTFALL_API ANightfallLoginGameMode : public AGameModeBase
{
	GENERATED_BODY()

public:
	ANightfallLoginGameMode();
};
