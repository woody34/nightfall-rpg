#pragma once

#include "CoreMinimal.h"
#include "GameFramework/Character.h"
#include "NightfallCharacter.generated.h"

class UCameraComponent;
class USpringArmComponent;
class UStaticMeshComponent;
class UOwnEntityComponent;
class UEntityAnimationComponent;

/**
 * The local player's pawn: a capsule with a placeholder body and a fixed top-down camera, walked
 * by NightfallPlayerController's click-to-move preview. The server stays authoritative.
 * With the vendor art imported, the inherited skeletal mesh shows Manny animated by Animation
 * and the placeholder Body is hidden.
 */
UCLASS()
class NIGHTFALL_API ANightfallCharacter : public ACharacter
{
	GENERATED_BODY()

public:
	ANightfallCharacter();

	UPROPERTY(VisibleAnywhere, BlueprintReadOnly, Category = "Nightfall")
	TObjectPtr<USpringArmComponent> CameraBoom;

	UPROPERTY(VisibleAnywhere, BlueprintReadOnly, Category = "Nightfall")
	TObjectPtr<UCameraComponent> TopDownCamera;

	UPROPERTY(VisibleAnywhere, BlueprintReadOnly, Category = "Nightfall")
	TObjectPtr<UStaticMeshComponent> Body;

	/** Reconciles this pawn with the server's view of the player's own entity. */
	UPROPERTY(VisibleAnywhere, BlueprintReadOnly, Category = "Nightfall")
	TObjectPtr<UOwnEntityComponent> OwnEntity;

	/** Animates GetMesh() from the own entity's server events (Manny preset). */
	UPROPERTY(VisibleAnywhere, BlueprintReadOnly, Category = "Nightfall")
	TObjectPtr<UEntityAnimationComponent> Animation;

	virtual void BeginPlay() override;
};
