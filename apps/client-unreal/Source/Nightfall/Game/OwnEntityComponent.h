#pragma once

#include "CoreMinimal.h"
#include "Components/ActorComponent.h"
#include "Net/ProtoCodec.h"
#include "OwnEntityComponent.generated.h"

class UNetClientSubsystem;

/**
 * Drives the controlled pawn from the server. The pawn previews a click immediately
 * (SimpleMoveToLocation, started by the player controller); this component reconciles it with
 * the server's EntityMove stream for the player's own entity:
 *  - the server position is the truth: if the pawn drifts more than SnapDistance (cm) away it is
 *    snapped back (this is also how the pawn first lands on its EntitySpawn position);
 *  - if the server's destination differs from the local goal by more than one tile, the local
 *    move is re-issued toward the server's destination;
 *  - IntentRejected for the pending MoveTo cancels the preview and shows the reason.
 * The server's speed (tiles/s) becomes the pawn's walk speed.
 */
UCLASS(ClassGroup = (Nightfall), meta = (BlueprintSpawnableComponent))
class NIGHTFALL_API UOwnEntityComponent : public UActorComponent
{
	GENERATED_BODY()

public:
	UOwnEntityComponent();

	virtual void BeginPlay() override;
	virtual void EndPlay(const EEndPlayReason::Type EndPlayReason) override;

	/** The controller tells us what it previewed: the MoveTo seq and the goal in world cm. */
	void NotePreview(uint32 Seq, const FVector& GoalCm);

	/** Test seam: feed server events without a socket. */
	void ApplySpawn(const FEntitySpawn& Spawn);
	void ApplyMove(const FEntityMove& Move);
	void ApplyRejected(const FIntentRejected& Rejected);
	/** The respawn point is authoritative: cancel the preview and place the pawn there. */
	void ApplyRespawned(const FEntityRespawned& Respawned);

	/** Pawn farther than this from the server position (cm) is snapped to it. */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall")
	float SnapDistance = 300.f;

	/** World units per server tile unit. */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall")
	float UnitsPerTile = 100.f;

	static FString RejectReasonText(uint32 Reason);

private:
	void MovePreviewTo(const FVector& GoalCm);
	void StopPreview();
	void Snap(const FVector& XYCm);
	FVector TileToWorld(const FNetVec2& Tile) const;
	void SetStatus(const FString& Status) const;
	UNetClientSubsystem* Net() const;

	FDelegateHandle SpawnHandle, MoveHandle, RejectedHandle, RespawnedHandle;
	TOptional<FVector> LocalGoal;     // cm; unset when the preview is idle
	uint32 PendingSeq = 0;
	FVector LastServerPos = FVector::ZeroVector;
	bool bHaveServerPos = false;
};
