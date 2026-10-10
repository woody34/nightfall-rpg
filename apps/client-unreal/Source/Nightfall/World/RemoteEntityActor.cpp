#include "RemoteEntityActor.h"
#include "Net/NetClientSubsystem.h"
#include "Components/SceneComponent.h"
#include "Components/StaticMeshComponent.h"
#include "Components/CapsuleComponent.h"
#include "Components/SkeletalMeshComponent.h"
#include "Anim/EntityAnimationComponent.h"

ARemoteEntityActor::ARemoteEntityActor()
{
	PrimaryActorTick.bCanEverTick = true;
	RootComponent = CreateDefaultSubobject<USceneComponent>(TEXT("Root"));
	Body = CreateDefaultSubobject<UStaticMeshComponent>(TEXT("Body"));
	Body->SetupAttachment(RootComponent);
	Body->SetCollisionEnabled(ECollisionEnabled::NoCollision);
	Body->SetRelativeLocation(FVector(0.f, 0.f, 95.f));
	Body->SetRelativeScale3D(FVector(0.8f, 0.8f, 1.9f));   // sized for the engine's 100 cm shapes

	SkeletalBody = CreateDefaultSubobject<USkeletalMeshComponent>(TEXT("SkeletalBody"));
	SkeletalBody->SetupAttachment(RootComponent);
	SkeletalBody->SetCollisionEnabled(ECollisionEnabled::NoCollision);
	SkeletalBody->SetVisibility(false);
	// Proxies off screen still need their death/corpse pose when they come into view.
	SkeletalBody->VisibilityBasedAnimTickOption = EVisibilityBasedAnimTickOption::AlwaysTickPoseAndRefreshBones;

	Animation = CreateDefaultSubobject<UEntityAnimationComponent>(TEXT("Animation"));

	ClickVolume = CreateDefaultSubobject<UCapsuleComponent>(TEXT("ClickVolume"));
	ClickVolume->SetupAttachment(RootComponent);
	ClickVolume->InitCapsuleSize(60.f, 100.f);
	ClickVolume->SetRelativeLocation(FVector(0.f, 0.f, 100.f));
	ClickVolume->SetCollisionEnabled(ECollisionEnabled::QueryOnly);
	ClickVolume->SetCollisionResponseToAllChannels(ECR_Ignore);
	ClickVolume->SetCollisionResponseToChannel(ECC_Visibility, ECR_Block);
	ClickVolume->SetCanEverAffectNavigation(false);
}

void ARemoteEntityActor::BeginPlay()
{
	Super::BeginPlay();
	bAnimatedBody = Animation->ApplyToMesh(SkeletalBody);
	Body->SetVisibility(!bAnimatedBody);
}

void ARemoteEntityActor::Bind(UNetClientSubsystem* InNet)
{
	Net = InNet;
	Animation->Bind(InNet, EntityId);
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

void ARemoteEntityActor::SetClassPresentation(uint32 InClassId, const FString& Title, bool bTransferred)
{
	ClassId = InClassId;
	ClassTitle = Title;
	if (bTransferred) TransferCueUntil = FPlatformTime::Seconds() + 2.0;
}

FString ARemoteEntityActor::GetNameplate() const
{
	return DisplayName + (ClassTitle.IsEmpty() ? FString() : TEXT(" — ") + ClassTitle) + (HasTransferCue() ? TEXT(" [Class advanced]") : TEXT(""));
}

bool ARemoteEntityActor::HasTransferCue() const { return FPlatformTime::Seconds() < TransferCueUntil; }
