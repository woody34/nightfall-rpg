#pragma once

#include "CoreMinimal.h"
#include "GameFramework/Actor.h"
#include "RemoteEntityActor.generated.h"

/**
 * Visual proxy for one server entity (another player or an NPC). It has no gameplay logic:
 * every tick it asks the snapshot buffer where the server says it is and moves there.
 * Subclass in Blueprint to attach a skeletal mesh and animation blueprint.
 */
UCLASS(Blueprintable)
class NIGHTFALL_API ARemoteEntityActor : public AActor
{
	GENERATED_BODY()

public:
	ARemoteEntityActor();

	UPROPERTY(BlueprintReadOnly, Category = "Nightfall")
	FString EntityId;

	UPROPERTY(BlueprintReadOnly, Category = "Nightfall")
	FString DisplayName;

	/** World units per server tile unit (Phase 6 §3: 32 px tiles become 100 cm). */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall")
	float UnitsPerTile = 100.f;

	/** Set by the spawner so the actor can drive itself from server state. */
	void Bind(class UNetClientSubsystem* InNet);

	virtual void Tick(float DeltaSeconds) override;

private:
	TWeakObjectPtr<class UNetClientSubsystem> Net;
};
