#include "NetClientSubsystem.h"
#include "Nightfall.h"
#include "IWebSocket.h"
#include "WebSocketsModule.h"
#include "Containers/Ticker.h"

void UNetClientSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	FModuleManager::LoadModuleChecked<FWebSocketsModule>("WebSockets");
	SocketFactory = [](const FWsUpgradeRequest& Request)
	{
		return FWebSocketsModule::Get().CreateWebSocket(Request.Url, Request.Protocols, Request.Headers);
	};
	Scheduler = [](float DelaySeconds, TFunction<void()> Fn)
	{
		FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateLambda([Fn = MoveTemp(Fn)](float)
		{
			Fn();
			return false;   // one shot
		}), DelaySeconds);
	};
}

void UNetClientSubsystem::Deinitialize()
{
	Disconnect();
	TicketProvider = nullptr;
	Super::Deinitialize();
}

FWsUpgradeRequest UNetClientSubsystem::MakeUpgradeRequest(const FString& WsUrl, const FString& PlayTicket)
{
	FWsUpgradeRequest Request;
	Request.Url = WsUrl;
	// The ticket is a bearer credential: header only, so it never reaches access logs or spans.
	Request.Headers.Add(TEXT("Authorization"), FString::Printf(TEXT("Bearer %s"), *PlayTicket));
	return Request;
}

void UNetClientSubsystem::Connect(const FString& WsUrl, const FString& PlayTicket)
{
	CloseSocket();
	++ConnectGeneration;
	bWantConnected = true;
	bReconnectPending = false;
	ReconnectAttempt = 0;
	Open(WsUrl, PlayTicket);
}

void UNetClientSubsystem::Disconnect()
{
	++ConnectGeneration;   // cancels a pending reconnect and drops late ticket replies
	bWantConnected = false;
	bReconnectPending = false;
	CloseSocket();
}

void UNetClientSubsystem::CloseSocket()
{
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

void UNetClientSubsystem::Open(const FString& WsUrl, const FString& PlayTicket)
{
	CloseSocket();
	Socket = SocketFactory(MakeUpgradeRequest(WsUrl, PlayTicket));
	UE_LOG(LogNightfall, Log, TEXT("ws connecting to %s"), *WsUrl);

	Socket->OnConnected().AddLambda([this]()
	{
		bConnected = true;
		ReconnectAttempt = 0;
		Frame.Reset();
		KnownEntities.Reset();   // the server re-sends every spawn in our area of interest
		UE_LOG(LogNightfall, Log, TEXT("ws connected"));
		OnConnected.Broadcast();
	});
	Socket->OnConnectionError().AddLambda([this](const FString& Error)
	{
		// A rejected ticket (HTTP 401/409 at the upgrade) also lands here.
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
	if (!bWantConnected || bReconnectPending) return;
	if (!TicketProvider)
	{
		// Play tickets are single-use: retrying with the one we have can never succeed.
		UE_LOG(LogNightfall, Warning, TEXT("ws: no ticket provider set; not reconnecting"));
		bWantConnected = false;
		return;
	}
	bReconnectPending = true;
	// 0.5, 1, 2, 4 ... capped at 10 s (Phase 8 §5.1).
	const float Delay = FMath::Min(10.f, 0.5f * FMath::Pow(2.f, static_cast<float>(ReconnectAttempt++)));
	UE_LOG(LogNightfall, Log, TEXT("ws: reconnecting in %.1f s"), Delay);
	Scheduler(Delay, [Weak = TWeakObjectPtr<UNetClientSubsystem>(this), Gen = ConnectGeneration]()
	{
		if (Weak.IsValid() && Weak->ConnectGeneration == Gen)
		{
			Weak->Reconnect();
		}
	});
}

void UNetClientSubsystem::Reconnect()
{
	bReconnectPending = false;
	if (!bWantConnected) return;
	TicketProvider([Weak = TWeakObjectPtr<UNetClientSubsystem>(this), Gen = ConnectGeneration](bool bOk, const FPlayTicket& Ticket)
	{
		if (!Weak.IsValid() || Weak->ConnectGeneration != Gen || !Weak->bWantConnected) return;
		if (bOk)
		{
			Weak->Open(Ticket.WsUrl, Ticket.Ticket);
		}
		else
		{
			UE_LOG(LogNightfall, Warning, TEXT("ws: could not get a play ticket; will retry"));
			Weak->ScheduleReconnect();
		}
	});
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

	if (Msg.Ack.IsSet())
	{
		LastAckedSeq = FMath::Max(LastAckedSeq, Msg.Ack->Seq);
	}
	if (Msg.Rejected.IsSet())
	{
		LastAckedSeq = FMath::Max(LastAckedSeq, Msg.Rejected->Seq);
		UE_LOG(LogNightfall, Warning, TEXT("ws: intent %u rejected (reason %u): %s"),
			Msg.Rejected->Seq, Msg.Rejected->Reason, *Msg.Rejected->Detail);
	}
	if (Msg.Event.IsSet())
	{
		const FWorldEvent& E = *Msg.Event;
		if (E.Spawn.IsSet())
		{
			SnapshotBuffer.Push(E.Spawn->EntityId, E.Spawn->Position, EstimatedServerTimeMs());
			KnownEntities.Add(E.Spawn->EntityId, *E.Spawn);
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
			KnownEntities.Remove(E.Despawn->EntityId);
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
