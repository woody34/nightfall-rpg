#pragma once

#include "CoreMinimal.h"
#include "RemoteEntityActor.h"
#include "ClassMasterActor.generated.h"

/** Existing-shape prototype marker. Gameplay range/eligibility is enforced by the server. */
UCLASS()
class NIGHTFALL_API AClassMasterActor : public ARemoteEntityActor
{
	GENERATED_BODY()
public:
	AClassMasterActor();
	void Configure(const FString& Name, const FVector2D& Tile);
private:
	UPROPERTY() TObjectPtr<class UTextRenderComponent> Label;
};
