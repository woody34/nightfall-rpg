#include "OwnEntityComponent.h"
#include "Net/NetClientSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "Nightfall.h"
#include "Blueprint/AIBlueprintHelperLibrary.h"
#include "Engine/GameInstance.h"
#include "GameFramework/Character.h"
#include "GameFramework/CharacterMovementComponent.h"
#include "GameFramework/Controller.h"

UOwnEntityComponent::UOwnEntityComponent()
{
	PrimaryComponentTick.bCanEverTick = false;
}

UNetClientSubsystem* UOwnEntityComponent::Net() const
{
	const UGameInstance* GI = GetWorld() ? GetWorld()->GetGameInstance() : nullptr;
	return GI ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr;
}

void UOwnEntityComponent::BeginPlay()
{
	Super::BeginPlay();
	UNetClientSubsystem* N = Net();
	if (!N) return;
	SpawnHandle = N->OnEntitySpawn.AddUObject(this, &UOwnEntityComponent::ApplySpawn);
	MoveHandle = N->OnEntityMove.AddUObject(this, &UOwnEntityComponent::ApplyMove);
	RejectedHandle = N->OnIntentRejected.AddUObject(this, &UOwnEntityComponent::ApplyRejected);
	RespawnedHandle = N->OnEntityRespawned.AddUObject(this, &UOwnEntityComponent::ApplyRespawned);

	// The spawn usually arrived before this map loaded.
	if (const FEntitySpawn* Known = N->GetKnownEntities().Find(N->GetOwnEntityId()))
	{
		ApplySpawn(*Known);
	}
}

void UOwnEntityComponent::EndPlay(const EEndPlayReason::Type EndPlayReason)
{
	if (UNetClientSubsystem* N = Net())
	{
		N->OnEntitySpawn.Remove(SpawnHandle);
		N->OnEntityMove.Remove(MoveHandle);
		N->OnIntentRejected.Remove(RejectedHandle);
		N->OnEntityRespawned.Remove(RespawnedHandle);
	}
	Super::EndPlay(EndPlayReason);
}

FVector UOwnEntityComponent::TileToWorld(const FNetVec2& Tile) const
{
	const double Z = GetOwner() ? GetOwner()->GetActorLocation().Z : 0.0;
	return FVector(Tile.X * UnitsPerTile, Tile.Y * UnitsPerTile, Z);
}

void UOwnEntityComponent::NotePreview(uint32 Seq, const FVector& GoalCm)
{
	PendingSeq = Seq;
	LocalGoal = GoalCm;
}

void UOwnEntityComponent::Snap(const FVector& XYCm)
{
	AActor* Owner = GetOwner();
	if (!Owner) return;
	UE_LOG(LogNightfall, Verbose, TEXT("own entity: snap %s -> (%.0f, %.0f) cm"), *Owner->GetActorLocation().ToString(), XYCm.X, XYCm.Y);
	Owner->SetActorLocation(FVector(XYCm.X, XYCm.Y, Owner->GetActorLocation().Z), /*bSweep=*/false, nullptr, ETeleportType::TeleportPhysics);
}

void UOwnEntityComponent::MovePreviewTo(const FVector& GoalCm)
{
	const APawn* Pawn = Cast<APawn>(GetOwner());
	AController* Controller = Pawn ? Pawn->GetController() : nullptr;
	if (!Controller) return;
	LocalGoal = GoalCm;
	UAIBlueprintHelperLibrary::SimpleMoveToLocation(Controller, GoalCm);
}

void UOwnEntityComponent::StopPreview()
{
	LocalGoal.Reset();
	if (const APawn* Pawn = Cast<APawn>(GetOwner()))
	{
		if (AController* Controller = Pawn->GetController())
		{
			Controller->StopMovement();
		}
	}
}

void UOwnEntityComponent::ApplySpawn(const FEntitySpawn& Spawn)
{
	UNetClientSubsystem* N = Net();
	if (!N || !N->IsOwnEntity(Spawn.EntityId)) return;
	LastServerPos = TileToWorld(Spawn.Position);
	bHaveServerPos = true;
	UE_LOG(LogNightfall, Log, TEXT("own entity: spawn at tile (%.2f, %.2f)"), Spawn.Position.X, Spawn.Position.Y);
	Snap(LastServerPos);
}

void UOwnEntityComponent::ApplyRespawned(const FEntityRespawned& Respawned)
{
	UNetClientSubsystem* N = Net();
	if (!N || !N->IsOwnEntity(Respawned.Entity)) return;
	StopPreview();
	PendingSeq = 0;
	LastServerPos = TileToWorld(Respawned.Position);
	bHaveServerPos = true;
	Snap(LastServerPos);
}

void UOwnEntityComponent::ApplyMove(const FEntityMove& Move)
{
	UNetClientSubsystem* N = Net();
	AActor* Owner = GetOwner();
	if (!N || !Owner || !N->IsOwnEntity(Move.EntityId)) return;

	const FVector ServerPos = TileToWorld(Move.Position);
	LastServerPos = ServerPos;
	bHaveServerPos = true;

	const double Drift = FVector::Dist2D(Owner->GetActorLocation(), ServerPos);
	UE_LOG(LogNightfall, Verbose, TEXT("own entity: move tick %llu server (%.2f, %.2f) -> (%.2f, %.2f) speed %.2f, drift %.0f cm"),
		Move.Tick, Move.Position.X, Move.Position.Y, Move.Destination.X, Move.Destination.Y, Move.Speed, Drift);
	if (Drift > SnapDistance)
	{
		Snap(ServerPos);
	}

	if (Move.Speed > 0.f)
	{
		// Walk at the server's speed, nudged by how far ahead of (+) or behind (-) the server the
		// preview is along the travel direction, so the two converge instead of hopping at the snap.
		const FVector Toward = (TileToWorld(Move.Destination) - ServerPos).GetSafeNormal2D();
		const double Lead = FVector::DotProduct(Owner->GetActorLocation() - ServerPos, Toward);
		const double Scale = FMath::Clamp(1.0 - Lead / (3.0 * UnitsPerTile), 0.5, 1.5);
		if (const ACharacter* Character = Cast<ACharacter>(Owner))
		{
			Character->GetCharacterMovement()->MaxWalkSpeed = static_cast<float>(Move.Speed * UnitsPerTile * Scale);
		}
	}

	if (Move.Speed <= 0.f)
	{
		// Stopped (the final EntityMove has a zero destination and speed).
		if (Drift <= UnitsPerTile) StopPreview();
		return;
	}

	const FVector ServerDest = TileToWorld(Move.Destination);
	if (!LocalGoal.IsSet() || FVector::Dist2D(*LocalGoal, ServerDest) > UnitsPerTile || Drift > SnapDistance)
	{
		UE_LOG(LogNightfall, Verbose, TEXT("own entity: server destination (%.2f, %.2f) tiles differs from local goal; re-issuing move"),
			Move.Destination.X, Move.Destination.Y);
		MovePreviewTo(ServerDest);
	}
}

FString UOwnEntityComponent::RejectReasonText(uint32 Reason)
{
	switch (Reason)
	{
	case 1: return TEXT("destination is outside the zone");
	case 2: return TEXT("destination is too far");
	case 3: return TEXT("your character is not in the zone");
	case 4: return TEXT("server busy, try again");
	case 5: return TEXT("too many clicks, slow down");
	case 6: return TEXT("invalid request");
	case 7: return TEXT("you are dead");
	case 8: return TEXT("that can't be attacked");
	case 9: return TEXT("target is out of sight");
	case 10: return TEXT("too far away");
	case 11: return TEXT("target is protected");
	case 12: return TEXT("not available yet");
	default: return TEXT("rejected");
	}
}

void UOwnEntityComponent::SetStatus(const FString& Status) const
{
	const UGameInstance* GI = GetWorld() ? GetWorld()->GetGameInstance() : nullptr;
	if (ULoginFlowSubsystem* Flow = GI ? GI->GetSubsystem<ULoginFlowSubsystem>() : nullptr)
	{
		Flow->SetStatus(Status);
	}
	else
	{
		UE_LOG(LogNightfall, Log, TEXT("%s"), *Status);
	}
}

void UOwnEntityComponent::ApplyRejected(const FIntentRejected& Rejected)
{
	if (Rejected.Seq != PendingSeq || PendingSeq == 0) return;   // not our latest preview
	PendingSeq = 0;
	UE_LOG(LogNightfall, Verbose, TEXT("own entity: MoveTo seq %u rejected (%u): %s"), Rejected.Seq, Rejected.Reason, *Rejected.Detail);
	StopPreview();
	if (bHaveServerPos && GetOwner() && FVector::Dist2D(GetOwner()->GetActorLocation(), LastServerPos) > UnitsPerTile)
	{
		Snap(LastServerPos);
	}
	SetStatus(FString::Printf(TEXT("Can't move there: %s"), *RejectReasonText(Rejected.Reason)));
}
