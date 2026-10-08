#include "NetClientSubsystem.h"
#include "Nightfall.h"
#include "IWebSocket.h"
#include "WebSocketsModule.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "TimerManager.h"

void UNetClientSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	FModuleManager::LoadModuleChecked<FWebSocketsModule>("WebSockets");
}

void UNetClientSubsystem::Deinitialize()
{
	Disconnect();
	Super::Deinitialize();
}

void UNetClientSubsystem::Connect(const FString& WsBaseUrl, const FString& PlayTicket)
{
	BaseUrl = WsBaseUrl;
	Ticket = PlayTicket;
	bWantConnected = true;
	ReconnectAttempt = 0;
	Open();
}

void UNetClientSubsystem::Disconnect()
{
	bWantConnected = false;
	if (UWorld* World = GetWorld())
	{
		World->GetTimerManager().ClearTimer(ReconnectTimer);
	}
	if (Socket.IsValid())
	{
		Socket->OnRawMessage().Clear();
		Socket->OnClosed().Clear();
		Socket->OnConnected().Clear();
		Socket->OnConnectionError().Clear();
		if (Socket->IsConnected()) Socket->Close();
		Socket.Reset();
	}
	bConnected = false;
}

void UNetClientSubsystem::Open()
{
	const FString Url = FString::Printf(TEXT("%s/ws?ticket=%s"), *BaseUrl, *FGenericPlatformHttp::UrlEncode(Ticket));
	Socket = FWebSocketsModule::Get().CreateWebSocket(Url, TEXT(""));

	Socket->OnConnected().AddLambda([this]()
	{
		bConnected = true;
		ReconnectAttempt = 0;
		Frame.Reset();
		UE_LOG(LogNightfall, Log, TEXT("ws connected"));
		OnConnected.Broadcast();
	});
	Socket->OnConnectionError().AddLambda([this](const FString& Error)
	{
		UE_LOG(LogNightfall, Warning, TEXT("ws connection error: %s"), *Error);
		bConnected = false;
		ScheduleReconnect();
	});
	Socket->OnClosed().AddUObject(this, &UNetClientSubsystem::HandleClosed);
	Socket->OnRawMessage().AddUObject(this, &UNetClientSubsystem::HandleRawMessage);
	Socket->Connect();
}

void UNetClientSubsystem::HandleClosed(int32 StatusCode, const FString& Reason, bool bWasClean)
{
	UE_LOG(LogNightfall, Log, TEXT("ws closed (%d, clean=%d): %s"), StatusCode, bWasClean, *Reason);
	bConnected = false;
	OnDisconnected.Broadcast(Reason);
	ScheduleReconnect();
}

void UNetClientSubsystem::ScheduleReconnect()
{
	if (!bWantConnected) return;
	UWorld* World = GetWorld();
	if (!World) return;
	// 0.5, 1, 2, 4 ... capped at 10 s (Phase 8 §5.1). The play ticket is single-use; a real
	// reconnect must first fetch a fresh one over gRPC. That hook lands with the auth slice.
	const float Delay = FMath::Min(10.f, 0.5f * FMath::Pow(2.f, static_cast<float>(ReconnectAttempt++)));
	World->GetTimerManager().SetTimer(ReconnectTimer, [this]() { Open(); }, Delay, false);
}

void UNetClientSubsystem::HandleRawMessage(const void* Data, SIZE_T Size, SIZE_T BytesRemaining)
{
	Frame.Append(static_cast<const uint8*>(Data), static_cast<int32>(Size));
	if (BytesRemaining > 0) return; // fragmented frame; wait for the rest

	FServerMessage Msg;
	const bool bOk = NightfallProto::Decode(Frame.GetData(), Frame.Num(), Msg);
	Frame.Reset();
	if (!bOk)
	{
		UE_LOG(LogNightfall, Warning, TEXT("ws: malformed ServerMessage dropped"));
		return;
	}

	if (Msg.AckSeq.IsSet())
	{
		LastAckedSeq = FMath::Max(LastAckedSeq, *Msg.AckSeq);
	}
	if (Msg.Event.IsSet())
	{
		const FWorldEvent& E = *Msg.Event;
		if (E.Spawn.IsSet())
		{
			SnapshotBuffer.Push(E.Spawn->EntityId, E.Spawn->Position, EstimatedServerTimeMs());
			OnEntitySpawn.Broadcast(*E.Spawn);
		}
		if (E.Move.IsSet())
		{
			// Learn the server clock from authoritative timestamps (simple EMA).
			const int64 LocalNow = FDateTime::UtcNow().ToUnixTimestamp() * 1000 + FDateTime::UtcNow().GetMillisecond();
			const int64 Observed = E.Move->ServerTimeMs - LocalNow;
			ServerClockOffsetMs = (ServerClockOffsetMs == 0) ? Observed : (ServerClockOffsetMs * 7 + Observed) / 8;

			SnapshotBuffer.Push(E.Move->EntityId, E.Move->Position, E.Move->ServerTimeMs);
			OnEntityMove.Broadcast(*E.Move);
		}
		if (E.Despawn.IsSet())
		{
			SnapshotBuffer.Remove(E.Despawn->EntityId);
			OnEntityDespawn.Broadcast(*E.Despawn);
		}
	}
}

uint32 UNetClientSubsystem::SendMoveTo(const FNetVec2& Destination)
{
	FClientMessage Msg;
	Msg.Seq = ++NextSeq;
	Msg.MoveTo = FMoveToIntent{ Destination };
	Send(Msg);
	return Msg.Seq;
}

void UNetClientSubsystem::Send(const FClientMessage& Msg)
{
	if (!Socket.IsValid() || !bConnected) return;
	TArray<uint8> Bytes;
	NightfallProto::Encode(Msg, Bytes);
	Socket->Send(Bytes.GetData(), Bytes.Num(), /*bIsBinary=*/true);
}

int64 UNetClientSubsystem::EstimatedServerTimeMs() const
{
	const FDateTime Now = FDateTime::UtcNow();
	return Now.ToUnixTimestamp() * 1000 + Now.GetMillisecond() + ServerClockOffsetMs;
}
