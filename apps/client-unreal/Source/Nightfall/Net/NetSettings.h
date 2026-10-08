#pragma once

#include "CoreMinimal.h"
#include "Engine/DeveloperSettings.h"
#include "NetSettings.generated.h"

/** Server endpoints. Config/DefaultGame.ini, section [/Script/Nightfall.NetSettings]. */
UCLASS(Config = Game, DefaultConfig, meta = (DisplayName = "Nightfall Net"))
class NIGHTFALL_API UNetSettings : public UDeveloperSettings
{
	GENERATED_BODY()

public:
	/** host:port of the API's gRPC listener (tonic). No scheme; plaintext HTTP/2 until TLS lands. */
	UPROPERTY(Config, EditAnywhere, Category = "gRPC")
	FString GrpcEndpoint = TEXT("localhost:50051");

	/** Deadline applied to every unary call. Expired calls fail with ENetError::DeadlineExceeded. */
	UPROPERTY(Config, EditAnywhere, Category = "gRPC", meta = (ClampMin = "0.1"))
	float CallTimeoutSeconds = 5.f;
};
