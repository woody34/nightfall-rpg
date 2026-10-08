#include "RemoteEntityActor.h"
#include "Net/NetClientSubsystem.h"

ARemoteEntityActor::ARemoteEntityActor()
{
	PrimaryActorTick.bCanEverTick = true;
}

void ARemoteEntityActor::Bind(UNetClientSubsystem* InNet)
{
	Net = InNet;
}

void ARemoteEntityActor::Tick(float DeltaSeconds)
{
	Super::Tick(DeltaSeconds);
	if (!Net.IsValid()) return;

	FNetVec2 P;
	if (Net->Snapshots().Sample(EntityId, Net->EstimatedServerTimeMs(), P))
	{
		const FVector Target(P.X * UnitsPerTile, P.Y * UnitsPerTile, GetActorLocation().Z);
		const FVector Current = GetActorLocation();
		if (!Current.Equals(Target, 1.f))
		{
			SetActorRotation(FRotationMatrix::MakeFromX(Target - Current).Rotator());
		}
		SetActorLocation(Target);
	}
}
