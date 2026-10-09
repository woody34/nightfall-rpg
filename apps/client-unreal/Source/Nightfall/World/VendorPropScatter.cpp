#include "VendorPropScatter.h"
#include "Nightfall.h"
#include "Components/SceneComponent.h"
#include "Components/StaticMeshComponent.h"
#include "Engine/StaticMesh.h"

AVendorPropScatter::AVendorPropScatter()
{
	RootComponent = CreateDefaultSubobject<USceneComponent>(TEXT("Root"));
}

void AVendorPropScatter::BeginPlay()
{
	Super::BeginPlay();
	SpawnProps();
}

int32 AVendorPropScatter::SpawnProps()
{
	int32 Spawned = 0;
	for (const FVendorProp& Prop : Props)
	{
		UStaticMesh* Mesh = Cast<UStaticMesh>(Prop.Mesh.TryLoad());
		if (!Mesh) continue;
		UStaticMeshComponent* Component = NewObject<UStaticMeshComponent>(this);
		Component->SetStaticMesh(Mesh);
		Component->SetCollisionEnabled(ECollisionEnabled::NoCollision);
		Component->SetCanEverAffectNavigation(false);
		Component->SetupAttachment(RootComponent);
		Component->SetWorldTransform(Prop.Transform);
		Component->RegisterComponent();
		++Spawned;
	}
	if (Spawned < Props.Num())
	{
		UE_LOG(LogNightfall, Log, TEXT("%d of %d vendor props not imported (Scripts/import-vendor.sh)"), Props.Num() - Spawned, Props.Num());
	}
	return Spawned;
}
