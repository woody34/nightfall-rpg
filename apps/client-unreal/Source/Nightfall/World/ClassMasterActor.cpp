#include "ClassMasterActor.h"
#include "Components/StaticMeshComponent.h"
#include "UObject/ConstructorHelpers.h"

AClassMasterActor::AClassMasterActor()
{
	Body->SetRelativeLocation(FVector(0.f, 0.f, 85.f));
	static ConstructorHelpers::FObjectFinder<UStaticMesh> Cylinder(TEXT("/Engine/BasicShapes/Cylinder.Cylinder"));
	if (Cylinder.Succeeded()) Body->SetStaticMesh(Cylinder.Object);
	Body->SetRelativeScale3D(FVector(0.6f, 0.6f, 1.7f));
	Body->SetCollisionEnabled(ECollisionEnabled::QueryOnly);
	Body->SetCollisionResponseToAllChannels(ECR_Ignore);
	Body->SetCollisionResponseToChannel(ECC_Visibility, ECR_Block);
	Body->SetCanEverAffectNavigation(false);
}

void AClassMasterActor::Configure(const FString& Name, const FVector2D& Tile)
{
	SetActorLocation(FVector(Tile.X * 100.f, Tile.Y * 100.f, 0.f));
	DisplayName = Name; // The existing HUD projects a readable screen-space nameplate.
}
