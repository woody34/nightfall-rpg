#include "NightfallCharacter.h"
#include "OwnEntityComponent.h"
#include "Anim/EntityAnimationComponent.h"
#include "Net/NetClientSubsystem.h"
#include "Components/SkeletalMeshComponent.h"
#include "Engine/GameInstance.h"
#include "Camera/CameraComponent.h"
#include "Components/CapsuleComponent.h"
#include "Components/StaticMeshComponent.h"
#include "Engine/StaticMesh.h"
#include "GameFramework/CharacterMovementComponent.h"
#include "GameFramework/SpringArmComponent.h"
#include "UObject/ConstructorHelpers.h"

ANightfallCharacter::ANightfallCharacter()
{
	GetCapsuleComponent()->InitCapsuleSize(42.f, 96.f);
	bUseControllerRotationPitch = false;
	bUseControllerRotationYaw = false;
	bUseControllerRotationRoll = false;
	GetCharacterMovement()->bOrientRotationToMovement = true;
	GetCharacterMovement()->RotationRate = FRotator(0.f, 640.f, 0.f);
	GetCharacterMovement()->bConstrainToPlane = true;
	GetCharacterMovement()->bSnapToPlaneAtStart = true;

	Body = CreateDefaultSubobject<UStaticMeshComponent>(TEXT("Body"));
	Body->SetupAttachment(GetCapsuleComponent());
	Body->SetCollisionEnabled(ECollisionEnabled::NoCollision);
	Body->SetRelativeScale3D(FVector(0.8f, 0.8f, 1.9f));   // the engine cylinder is 100 cm
	static ConstructorHelpers::FObjectFinder<UStaticMesh> Cylinder(TEXT("/Engine/BasicShapes/Cylinder.Cylinder"));
	if (Cylinder.Succeeded())
	{
		Body->SetStaticMesh(Cylinder.Object);
	}

	OwnEntity = CreateDefaultSubobject<UOwnEntityComponent>(TEXT("OwnEntity"));

	Animation = CreateDefaultSubobject<UEntityAnimationComponent>(TEXT("Animation"));
	Animation->AnimSet = UEntityAnimationComponent::PresetAnimSet(EEntityAnimPreset::Manny);
	Animation->bRenderAtServerTime = true;   // the own pawn is not interpolated

	CameraBoom = CreateDefaultSubobject<USpringArmComponent>(TEXT("CameraBoom"));
	CameraBoom->SetupAttachment(RootComponent);
	CameraBoom->SetUsingAbsoluteRotation(true);   // stays put while the character turns
	CameraBoom->TargetArmLength = 1600.f;
	CameraBoom->SetRelativeRotation(FRotator(-55.f, 0.f, 0.f));
	CameraBoom->bDoCollisionTest = false;

	TopDownCamera = CreateDefaultSubobject<UCameraComponent>(TEXT("TopDownCamera"));
	TopDownCamera->SetupAttachment(CameraBoom, USpringArmComponent::SocketName);
	TopDownCamera->bUsePawnControlRotation = false;
}

void ANightfallCharacter::BeginPlay()
{
	Super::BeginPlay();
	// The capsule's origin is at its centre; the mesh's is at its feet.
	const FVector Feet(0.f, 0.f, -GetCapsuleComponent()->GetScaledCapsuleHalfHeight());
	if (Animation->ApplyToMesh(GetMesh(), Feet))
	{
		Body->SetVisibility(false);
	}
	UGameInstance* GI = GetGameInstance();
	if (UNetClientSubsystem* Net = GI ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr)
	{
		Animation->Bind(Net, Net->GetOwnEntityId());
	}
}
