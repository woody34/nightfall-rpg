#pragma once

#include "CoreMinimal.h"
#include "GameFramework/PlayerController.h"
#include "NightfallPlayerController.generated.h"

class UInputMappingContext;
class UInputAction;

/**
 * Click-to-move, Lineage 2 style: a click raycasts the ground, the destination is sent to the
 * server as a MoveTo intent, and the local pawn previews the move immediately (Phase 0 §3.2
 * optimistic movement). The server's EntityMove for our own entity corrects the preview.
 */
UCLASS()
class NIGHTFALL_API ANightfallPlayerController : public APlayerController
{
	GENERATED_BODY()

public:
	ANightfallPlayerController();

protected:
	virtual void BeginPlay() override;
	virtual void SetupInputComponent() override;

	UPROPERTY(EditDefaultsOnly, Category = "Nightfall|Input")
	TObjectPtr<UInputMappingContext> DefaultMappingContext;

	UPROPERTY(EditDefaultsOnly, Category = "Nightfall|Input")
	TObjectPtr<UInputAction> ClickMoveAction;

	/** World units per server tile unit; must match ARemoteEntityActor::UnitsPerTile. */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall")
	float UnitsPerTile = 100.f;

private:
	void OnClickMove();
};
