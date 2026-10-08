#pragma once

#include "CoreMinimal.h"
#include "Engine/DeveloperSettings.h"
#include "NetSettings.generated.h"

/** Server and identity endpoints. Config/DefaultGame.ini, section [/Script/Nightfall.NetSettings]. */
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

	/**
	 * OIDC issuer (Keycloak realm URL, no trailing slash). The device authorization and token
	 * endpoints are derived from it with Keycloak's paths (`/protocol/openid-connect/...`).
	 */
	UPROPERTY(Config, EditAnywhere, Category = "Identity")
	FString OidcIssuer = TEXT("http://localhost:8080/realms/nightfall");

	/** Public OIDC client the game logs in as (device authorization grant + PKCE S256). */
	UPROPERTY(Config, EditAnywhere, Category = "Identity")
	FString OidcClientId = TEXT("nightfall-client");

	/** Audience the API checks. A token without it in `aud` is refused before it is ever sent. */
	UPROPERTY(Config, EditAnywhere, Category = "Identity")
	FString OidcAudience = TEXT("nightfall-api");

	/** Scopes requested at login. `offline_access` makes the stored refresh token outlive the SSO session. */
	UPROPERTY(Config, EditAnywhere, Category = "Identity")
	FString OidcScope = TEXT("openid offline_access");

	/** Map opened once the WebSocket connects. */
	UPROPERTY(Config, EditAnywhere, Category = "Maps", meta = (AllowedClasses = "/Script/Engine.World"))
	FSoftObjectPath WorldMap = FSoftObjectPath(TEXT("/Game/Maps/L_TestZone.L_TestZone"));
};
