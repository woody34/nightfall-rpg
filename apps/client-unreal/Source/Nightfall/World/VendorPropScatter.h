#pragma once

#include "CoreMinimal.h"
#include "GameFramework/Actor.h"
#include "VendorPropScatter.generated.h"

USTRUCT(BlueprintType)
struct NIGHTFALL_API FVendorProp
{
	GENERATED_BODY()

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall", meta = (AllowedClasses = "/Script/Engine.StaticMesh"))
	FSoftObjectPath Mesh;

	/** World transform (cm). */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	FTransform Transform;
};

/**
 * Set dressing from git-ignored vendor meshes (Content/Vendor/NatureLite). The level holds only
 * soft paths, so it loads cleanly on a clone without the imported art: missing meshes are skipped.
 * Purely visual: no collision, no navigation effect (the server has no obstacles in Phase 1).
 */
UCLASS()
class NIGHTFALL_API AVendorPropScatter : public AActor
{
	GENERATED_BODY()

public:
	AVendorPropScatter();

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	TArray<FVendorProp> Props;

	/** Creates one mesh component per loadable prop; returns how many. Runs at BeginPlay. */
	int32 SpawnProps();

	virtual void BeginPlay() override;
};
