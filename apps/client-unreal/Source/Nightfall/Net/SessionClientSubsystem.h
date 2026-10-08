#pragma once

#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "TurboLinkGrpcClient.h"
#include "SNightfallV1/GameMessage.h"
#include "SessionClientSubsystem.generated.h"

class UGameService;
class UGameServiceClient;
class UTurboLinkGrpcManager;

/** Canonical gRPC status codes as the client sees them. Values match grpc::StatusCode. */
UENUM(BlueprintType)
enum class ENetError : uint8
{
	None = 0,
	Cancelled = 1,
	Unknown = 2,
	InvalidArgument = 3,
	DeadlineExceeded = 4,
	NotFound = 5,
	AlreadyExists = 6,
	PermissionDenied = 7,
	ResourceExhausted = 8,
	FailedPrecondition = 9,
	Aborted = 10,
	OutOfRange = 11,
	Unimplemented = 12,
	Internal = 13,
	Unavailable = 14,
	DataLoss = 15,
	Unauthenticated = 16,
};

/** Outcome of one call. Message is the server's status message (never infrastructure detail). */
USTRUCT(BlueprintType)
struct NIGHTFALL_API FNetResult
{
	GENERATED_BODY()

	UPROPERTY(BlueprintReadOnly, Category = "Nightfall|Net")
	ENetError Error = ENetError::None;

	UPROPERTY(BlueprintReadOnly, Category = "Nightfall|Net")
	FString Message;

	bool IsOk() const { return Error == ENetError::None; }

	static FNetResult FromGrpc(const FGrpcResult& Result);
};

DECLARE_DYNAMIC_DELEGATE_TwoParams(FOnNetPingDone, const FNetResult&, Result, const FGrpcNightfallV1PingResponse&, Response);
DECLARE_DYNAMIC_DELEGATE_TwoParams(FOnNetCharacterDone, const FNetResult&, Result, const FGrpcNightfallV1Character&, Character);

/**
 * Request/response channel to the API's GameService over gRPC (TurboLink). Holds one channel to
 * UNetSettings::GrpcEndpoint for the lifetime of the game instance. The real-time world stream
 * stays on UNetClientSubsystem's WebSocket.
 *
 * Callbacks run on the game thread from TurboLink's tick. Every call carries the configured
 * deadline and, once SetBearerToken has been called, an `authorization: Bearer ...` header.
 */
UCLASS()
class NIGHTFALL_API USessionClient : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	using FPingCallback = TFunction<void(const FNetResult&, const FGrpcNightfallV1PingResponse&)>;
	using FCharacterCallback = TFunction<void(const FNetResult&, const FGrpcNightfallV1Character&)>;

	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	void Ping(FPingCallback Callback);
	void GetCharacter(const FString& CharacterId, FCharacterCallback Callback);
	void CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& Request, FCharacterCallback Callback);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net", meta = (DisplayName = "Ping"))
	void K2_Ping(FOnNetPingDone OnDone);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net", meta = (DisplayName = "Get Character"))
	void K2_GetCharacter(const FString& CharacterId, FOnNetCharacterDone OnDone);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net", meta = (DisplayName = "Create Character"))
	void K2_CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& Request, FOnNetCharacterDone OnDone);

	/** Sets the access token sent on every later call. An empty string stops sending the header. */
	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net")
	void SetBearerToken(const FString& Token) { BearerToken = Token; }

	/** host:port this client is connected to. */
	UFUNCTION(BlueprintPure, Category = "Nightfall|Net")
	FString GetEndpoint() const { return Endpoint; }

	/** The TurboLink manager that pumps this client's completion queue. Exposed for tests. */
	UTurboLinkGrpcManager* GetGrpcManager() const { return Manager; }

private:
	FGrpcMetaData MakeMetaData() const;

	/** Fails Callback immediately when the service could not be created. */
	bool EnsureClient(const TFunctionRef<void(const FNetResult&)>& Fail) const;

	UFUNCTION()
	void HandlePing(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1PingResponse& Response);

	UFUNCTION()
	void HandleGetCharacter(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1Character& Response);

	UFUNCTION()
	void HandleCreateCharacter(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1Character& Response);

	UPROPERTY()
	TObjectPtr<UTurboLinkGrpcManager> Manager;

	UPROPERTY()
	TObjectPtr<UGameService> Service;

	UPROPERTY()
	TObjectPtr<UGameServiceClient> Client;

	FString Endpoint;
	float CallTimeoutSeconds = 5.f;
	FString BearerToken;

	TMap<uint32, FPingCallback> PendingPing;
	TMap<uint32, FCharacterCallback> PendingGetCharacter;
	TMap<uint32, FCharacterCallback> PendingCreateCharacter;
};
