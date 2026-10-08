#pragma once

#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "TurboLinkGrpcClient.h"
#include "SNightfallV1/GameMessage.h"
#include "SNightfallV1/SessionMessage.h"
#include "SessionClientSubsystem.generated.h"

class UGameService;
class UGameServiceClient;
class USessionService;
class USessionServiceClient;
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
DECLARE_DYNAMIC_DELEGATE_TwoParams(FOnNetCharacterListDone, const FNetResult&, Result, const TArray<FGrpcNightfallV1Character>&, Characters);
DECLARE_DYNAMIC_DELEGATE_TwoParams(FOnNetPlayTicketDone, const FNetResult&, Result, const FGrpcNightfallV1IssuePlayTicketResponse&, Ticket);

/**
 * Request/response channel to the API's GameService and SessionService over gRPC (TurboLink).
 * Holds one channel per service to UNetSettings::GrpcEndpoint for the lifetime of the game
 * instance. The real-time world stream stays on UNetClientSubsystem's WebSocket.
 *
 * Callbacks run on the game thread from TurboLink's tick. Every call carries the configured
 * deadline. Every call except Ping carries `authorization: Bearer <access token>` once
 * SetBearerToken has been called (UAuthSubsystem does that on login and on every refresh).
 */
UCLASS()
class NIGHTFALL_API USessionClient : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	using FPingCallback = TFunction<void(const FNetResult&, const FGrpcNightfallV1PingResponse&)>;
	using FCharacterCallback = TFunction<void(const FNetResult&, const FGrpcNightfallV1Character&)>;
	using FCharacterListCallback = TFunction<void(const FNetResult&, const TArray<FGrpcNightfallV1Character>&)>;
	using FPlayTicketCallback = TFunction<void(const FNetResult&, const FGrpcNightfallV1IssuePlayTicketResponse&)>;

	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	void Ping(FPingCallback Callback);
	void GetCharacter(const FString& CharacterId, FCharacterCallback Callback);
	void CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& Request, FCharacterCallback Callback);
	void ListMyCharacters(FCharacterListCallback Callback);
	/** Request.IdempotencyKey must be new for every connection attempt (plan §8 #9). */
	void IssuePlayTicket(const FGrpcNightfallV1IssuePlayTicketRequest& Request, FPlayTicketCallback Callback);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net", meta = (DisplayName = "Ping"))
	void K2_Ping(FOnNetPingDone OnDone);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net", meta = (DisplayName = "Get Character"))
	void K2_GetCharacter(const FString& CharacterId, FOnNetCharacterDone OnDone);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net", meta = (DisplayName = "Create Character"))
	void K2_CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& Request, FOnNetCharacterDone OnDone);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net", meta = (DisplayName = "List My Characters"))
	void K2_ListMyCharacters(FOnNetCharacterListDone OnDone);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net", meta = (DisplayName = "Issue Play Ticket"))
	void K2_IssuePlayTicket(const FGrpcNightfallV1IssuePlayTicketRequest& Request, FOnNetPlayTicketDone OnDone);

	/** Sets the access token sent on every later call except Ping. Empty stops sending the header. */
	void SetBearerToken(const FString& Token) { BearerToken = Token; }

	/** True once a bearer token is set. The token itself is never exposed. */
	UFUNCTION(BlueprintPure, Category = "Nightfall|Net")
	bool HasBearerToken() const { return !BearerToken.IsEmpty(); }

	/** The metadata a call would carry; bAuthenticated=false is what Ping sends. Exposed for tests. */
	FGrpcMetaData MakeMetaData(bool bAuthenticated) const;

	/** host:port this client is connected to. */
	UFUNCTION(BlueprintPure, Category = "Nightfall|Net")
	FString GetEndpoint() const { return Endpoint; }

	/** The TurboLink manager that pumps this client's completion queue. Exposed for tests. */
	UTurboLinkGrpcManager* GetGrpcManager() const { return Manager; }

private:

	/** Fails Callback immediately when the service client could not be created. */
	static bool EnsureClient(const UObject* ServiceClient, const TFunctionRef<void(const FNetResult&)>& Fail);

	UFUNCTION()
	void HandlePing(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1PingResponse& Response);

	UFUNCTION()
	void HandleGetCharacter(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1Character& Response);

	UFUNCTION()
	void HandleCreateCharacter(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1Character& Response);

	UFUNCTION()
	void HandleListMyCharacters(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1ListMyCharactersResponse& Response);

	UFUNCTION()
	void HandleIssuePlayTicket(FGrpcContextHandle Handle, const FGrpcResult& Result, const FGrpcNightfallV1IssuePlayTicketResponse& Response);

	UPROPERTY()
	TObjectPtr<UTurboLinkGrpcManager> Manager;

	UPROPERTY()
	TObjectPtr<UGameService> Service;

	UPROPERTY()
	TObjectPtr<UGameServiceClient> Client;

	UPROPERTY()
	TObjectPtr<USessionService> SessionService;

	UPROPERTY()
	TObjectPtr<USessionServiceClient> SessionServiceClient;

	FString Endpoint;
	float CallTimeoutSeconds = 5.f;
	FString BearerToken;

	TMap<uint32, FPingCallback> PendingPing;
	TMap<uint32, FCharacterCallback> PendingGetCharacter;
	TMap<uint32, FCharacterCallback> PendingCreateCharacter;
	TMap<uint32, FCharacterListCallback> PendingListMyCharacters;
	TMap<uint32, FPlayTicketCallback> PendingIssuePlayTicket;
};
