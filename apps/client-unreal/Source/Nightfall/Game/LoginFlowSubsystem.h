#pragma once

#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Net/SessionClientSubsystem.h"
#include "LoginFlowSubsystem.generated.h"

class UAuthSubsystem;

DECLARE_DYNAMIC_MULTICAST_DELEGATE_OneParam(FOnLoginFlowStatus, const FString&, Status);

/**
 * Login -> character -> world (Story 6.2). Lives on the game instance so it outlives the login
 * screen and the map change: it owns the ticket provider UNetClientSubsystem calls on every
 * reconnect, and travels to UNetSettings::WorldMap once the socket connects.
 *
 * Every RPC first makes sure the access token has a minute left (UAuthSubsystem::RefreshIfNeeded).
 * Every mutating RPC gets a fresh idempotency key per attempt.
 */
UCLASS()
class NIGHTFALL_API ULoginFlowSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	using FResultCallback = TFunction<void(const FNetResult&)>;
	using FTicketResultCallback = TFunction<void(const FNetResult&, const FPlayTicket&)>;

	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	void ListCharacters(USessionClient::FCharacterListCallback Callback);
	void CreateCharacter(const FGrpcNightfallV1CreateCharacterRequest& Request, USessionClient::FCharacterCallback Callback);
	void ListClasses(USessionClient::FCatalogueCallback Callback);
	void TransferOptions(const FString& CharacterId, USessionClient::FTransferOptionsCallback Callback);
	void ChangeClass(const FString& CharacterId, uint32 TargetClassId, USessionClient::FChangeClassCallback Callback);
	void GetCharacter(const FString& CharacterId, USessionClient::FCharacterCallback Callback);
	void CreateCharacter(const FString& Name, EGrpcNightfallV1Race Race, USessionClient::FCharacterCallback Callback);

	/** Gets a play ticket for CharacterId (new idempotency key) and connects with it. */
	void EnterWorld(const FString& CharacterId, FResultCallback OnTicket);

	/** One IssuePlayTicket for the selected character with a new idempotency key. */
	void RequestTicket(const FString& CharacterId, FTicketResultCallback Callback);

	/** Stops the connection and forgets the character. */
	UFUNCTION(BlueprintCallable, Category = "Nightfall|Login")
	void LeaveWorld();

	UFUNCTION(BlueprintPure, Category = "Nightfall|Login")
	FString GetSelectedCharacterId() const { return SelectedCharacterId; }

	/** Human-readable progress ("Requesting play ticket...", "Connected"). */
	UPROPERTY(BlueprintAssignable, Category = "Nightfall|Login")
	FOnLoginFlowStatus OnStatus;

	/** When false, a successful connect does not open WorldMap. Tests turn it off. */
	bool bTravelOnConnect = true;

	static FString NewIdempotencyKey();

	void SetStatus(const FString& Status);

private:
	/** Runs Call with a valid access token, or fails Callback with Unauthenticated. */
	void WithFreshToken(TFunction<void()> Call, FResultCallback Fail);

	UFUNCTION()
	void HandleConnected();

	UFUNCTION()
	void HandleDisconnected(const FString& Reason);

	UPROPERTY()
	TObjectPtr<UAuthSubsystem> Auth;

	UPROPERTY()
	TObjectPtr<USessionClient> Session;

	UPROPERTY()
	TObjectPtr<UNetClientSubsystem> Net;

	FString SelectedCharacterId;
};
