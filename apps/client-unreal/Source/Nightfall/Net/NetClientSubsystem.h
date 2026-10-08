#pragma once

#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "ProtoCodec.h"
#include "SnapshotBuffer.h"
#include "NetClientSubsystem.generated.h"

class IWebSocket;

DECLARE_DYNAMIC_MULTICAST_DELEGATE(FOnNetConnected);
DECLARE_DYNAMIC_MULTICAST_DELEGATE_OneParam(FOnNetDisconnected, const FString&, Reason);
DECLARE_MULTICAST_DELEGATE_OneParam(FOnEntitySpawn, const FEntitySpawn&);
DECLARE_MULTICAST_DELEGATE_OneParam(FOnEntityMove, const FEntityMove&);
DECLARE_MULTICAST_DELEGATE_OneParam(FOnEntityDespawn, const FEntityDespawn&);

/** What a WebSocket upgrade is made from. The ticket rides in Headers, never in Url (plan §8 #7). */
struct FWsUpgradeRequest
{
	FString Url;
	TArray<FString> Protocols;
	TMap<FString, FString> Headers;
};

/** A single-use admission to the real-time channel (SessionService.IssuePlayTicket). */
struct FPlayTicket
{
	FString WsUrl;
	FString Ticket;
};

/**
 * The client's single connection to the Rust server's real-time channel (Phase 0 §3.2):
 * one WebSocket, one protobuf ClientMessage / ServerMessage per binary frame.
 *
 * This is the ONLY place network bytes exist. Gameplay code sends typed intents and listens to
 * typed events. There is no Unreal replication anywhere in this project.
 */
UCLASS()
class NIGHTFALL_API UNetClientSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	using FTicketCallback = TFunction<void(bool bOk, const FPlayTicket& Ticket)>;
	using FTicketProvider = TFunction<void(FTicketCallback OnTicket)>;
	using FSocketFactory = TFunction<TSharedRef<IWebSocket>(const FWsUpgradeRequest& Request)>;
	using FScheduler = TFunction<void(float DelaySeconds, TFunction<void()> Fn)>;

	/**
	 * Opens WsUrl (the ws_url from IssuePlayTicket) presenting PlayTicket as
	 * `Authorization: Bearer <ticket>` on the upgrade request. The ticket is used for this one
	 * attempt only. After a failure or disconnect it reconnects with backoff (0.5 s doubling,
	 * capped at 10 s) until Disconnect(), fetching a NEW ticket from the provider set with
	 * SetTicketProvider before every attempt; without a provider it does not reconnect.
	 */
	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net")
	void Connect(const FString& WsUrl, const FString& PlayTicket);

	/**
	 * Where reconnects get a fresh play ticket (plan §8 #9: a consumed ticket is never reused).
	 * The provider must call OnTicket exactly once; bOk=false retries after the next backoff.
	 * A provider that hits a fatal error (e.g. logged out) should call Disconnect() instead.
	 */
	void SetTicketProvider(FTicketProvider Provider) { TicketProvider = MoveTemp(Provider); }

	/** The upgrade request Connect makes. Static and pure so tests can check it. */
	static FWsUpgradeRequest MakeUpgradeRequest(const FString& WsUrl, const FString& PlayTicket);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net")
	void Disconnect();

	UFUNCTION(BlueprintPure, Category = "Nightfall|Net")
	bool IsConnected() const { return bConnected; }

	/** Sends a MoveTo intent. Returns the seq the server will ack. */
	uint32 SendMoveTo(const FNetVec2& Destination);

	/** Server time estimate for interpolation. Offset is learned from EntityMove timestamps. */
	int64 EstimatedServerTimeMs() const;

	FSnapshotBuffer& Snapshots() { return SnapshotBuffer; }

	/** Entities spawned and not yet despawned. A world that loads after connecting starts from these. */
	const TMap<FString, FEntitySpawn>& GetKnownEntities() const { return KnownEntities; }

	// Test seams. Defaults: FWebSocketsModule and FTSTicker.
	void SetSocketFactoryForTesting(FSocketFactory Factory) { SocketFactory = MoveTemp(Factory); }
	void SetSchedulerForTesting(FScheduler InScheduler) { Scheduler = MoveTemp(InScheduler); }

	UPROPERTY(BlueprintAssignable, Category = "Nightfall|Net")
	FOnNetConnected OnConnected;

	UPROPERTY(BlueprintAssignable, Category = "Nightfall|Net")
	FOnNetDisconnected OnDisconnected;

	FOnEntitySpawn OnEntitySpawn;
	FOnEntityMove OnEntityMove;
	FOnEntityDespawn OnEntityDespawn;

private:
	void Open(const FString& WsUrl, const FString& PlayTicket);
	void CloseSocket();
	void HandleRawMessage(const void* Data, SIZE_T Size, SIZE_T BytesRemaining);
	void HandleClosed(int32 StatusCode, const FString& Reason, bool bWasClean);
	void ScheduleReconnect();
	void Reconnect();
	void Send(const FClientMessage& Msg);

	TSharedPtr<IWebSocket> Socket;
	FTicketProvider TicketProvider;
	FSocketFactory SocketFactory;
	FScheduler Scheduler;
	bool bConnected = false;
	bool bWantConnected = false;
	bool bReconnectPending = false;
	/** Bumped by Connect and Disconnect; pending reconnects and ticket replies compare it. */
	uint32 ConnectGeneration = 0;
	int32 ReconnectAttempt = 0;
	uint32 NextSeq = 0;
	uint32 LastAckedSeq = 0;
	int64 ServerClockOffsetMs = 0;
	TArray<uint8> Frame;              // reassembly buffer for fragmented frames
	FSnapshotBuffer SnapshotBuffer;
	TMap<FString, FEntitySpawn> KnownEntities;
};
