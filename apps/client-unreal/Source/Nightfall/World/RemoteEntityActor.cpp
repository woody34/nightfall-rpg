#include "RemoteEntityActor.h"
#include "Net/NetClientSubsystem.h"
#include "Components/SceneComponent.h"
#include "Components/StaticMeshComponent.h"
#include "Components/CapsuleComponent.h"

ARemoteEntityActor::ARemoteEntityActor()
{
	PrimaryActorTick.bCanEverTick = true;
	RootComponent = CreateDefaultSubobject<USceneComponent>(TEXT("Root"));
	Body = CreateDefaultSubobject<UStaticMeshComponent>(TEXT("Body"));
	Body->SetupAttachment(RootComponent);
	Body->SetCollisionEnabled(ECollisionEnabled::NoCollision);
	Body->SetRelativeLocation(FVector(0.f, 0.f, 95.f));
	Body->SetRelativeScale3D(FVector(0.8f, 0.8f, 1.9f));   // sized for the engine's 100 cm shapes

	ClickVolume = CreateDefaultSubobject<UCapsuleComponent>(TEXT("ClickVolume"));
	ClickVolume->SetupAttachment(RootComponent);
	ClickVolume->InitCapsuleSize(60.f, 100.f);
	ClickVolume->SetRelativeLocation(FVector(0.f, 0.f, 100.f));
	ClickVolume->SetCollisionEnabled(ECollisionEnabled::QueryOnly);
	ClickVolume->SetCollisionResponseToAllChannels(ECR_Ignore);
	ClickVolume->SetCollisionResponseToChannel(ECC_Visibility, ECR_Block);
	ClickVolume->SetCanEverAffectNavigation(false);
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
