#pragma once

#include "CoreMinimal.h"
#include "GameFramework/Character.h"
#include "NightfallCharacter.generated.h"

class UCameraComponent;
class USpringArmComponent;
class UStaticMeshComponent;

/**
 * The local player's pawn: a capsule with a placeholder body and a fixed top-down camera, walked
 * by NightfallPlayerController's click-to-move preview. The server stays authoritative.
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
};
