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

	/** Opens ws(s)://host/ws?ticket=... Reconnects with backoff until Disconnect() is called. */
	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net")
	void Connect(const FString& WsBaseUrl, const FString& PlayTicket);

	UFUNCTION(BlueprintCallable, Category = "Nightfall|Net")
	void Disconnect();

	UFUNCTION(BlueprintPure, Category = "Nightfall|Net")
	bool IsConnected() const { return bConnected; }

	/** Sends a MoveTo intent. Returns the seq the server will ack. */
	uint32 SendMoveTo(const FNetVec2& Destination);

	/** Server time estimate for interpolation. Offset is learned from EntityMove timestamps. */
	int64 EstimatedServerTimeMs() const;

	FSnapshotBuffer& Snapshots() { return SnapshotBuffer; }

	UPROPERTY(BlueprintAssignable, Category = "Nightfall|Net")
	FOnNetConnected OnConnected;

	UPROPERTY(BlueprintAssignable, Category = "Nightfall|Net")
	FOnNetDisconnected OnDisconnected;

	FOnEntitySpawn OnEntitySpawn;
	FOnEntityMove OnEntityMove;
	FOnEntityDespawn OnEntityDespawn;

private:
	void Open();
	void HandleRawMessage(const void* Data, SIZE_T Size, SIZE_T BytesRemaining);
	void HandleClosed(int32 StatusCode, const FString& Reason, bool bWasClean);
	void ScheduleReconnect();
	void Send(const FClientMessage& Msg);

	TSharedPtr<IWebSocket> Socket;
	FString BaseUrl;
	FString Ticket;
	bool bConnected = false;
	bool bWantConnected = false;
	int32 ReconnectAttempt = 0;
	uint32 NextSeq = 0;
	uint32 LastAckedSeq = 0;
	int64 ServerClockOffsetMs = 0;
	TArray<uint8> Frame;              // reassembly buffer for fragmented frames
	FTimerHandle ReconnectTimer;
	FSnapshotBuffer SnapshotBuffer;
};
