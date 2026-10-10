#pragma once

#include "CoreMinimal.h"
#include "GameFramework/PlayerController.h"
#include "NightfallPlayerController.generated.h"

class UInputMappingContext;
class UInputAction;
class UNightfallHud;
class UNightfallClassDialog;
class AClassMasterActor;

/**
 * Click-to-move, Lineage 2 style: a click raycasts the ground, the destination is sent to the
 * server as a MoveTo intent, and the local pawn previews the move immediately (Phase 0 §3.2
 * optimistic movement). The server's EntityMove for our own entity corrects the preview.
 *
 * A click is raycast against the NPC proxies first and the ground second: an attackable NPC gets
 * SetTarget + Attack (UCombatStateSubsystem::ClickEntity), anything else is a ground click.
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

	/** Distance from the pawn's origin down to the ground (capsule half height), for the plane fallback. */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall")
	float GroundOffset = 96.f;

	/** Combat HUD widget created in BeginPlay. */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall|HUD")
	TSubclassOf<UNightfallHud> HudClass;

	UPROPERTY(Transient)
	TObjectPtr<UNightfallHud> Hud;

public:
	/**
	 * Ground click: StopAttack if an attack is outstanding, then MoveToWorldLocation.
	 * Returns the MoveTo seq.
	 */
	void OpenClassDialog() { ShowClassMaster(); }
	UNightfallClassDialog* GetClassDialog() const { return ClassDialog; }

	uint32 ClickGroundLocation(const FVector& Target);

	/**
	 * The click path after the raycast: previews the move locally and sends MoveTo with the
	 * destination converted from world cm to tiles. Returns the intent's seq (0 if not sent).
	 */
	uint32 MoveToWorldLocation(const FVector& Target);

private:
	void OnClickMove();
	void ShowClassMaster();
	void LoadClassCatalogue();
	UPROPERTY(Transient) TObjectPtr<UNightfallClassDialog> ClassDialog;
};
