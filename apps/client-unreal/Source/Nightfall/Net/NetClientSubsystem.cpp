#include "NetClientSubsystem.h"
#include "Nightfall.h"
#include "IWebSocket.h"
#include "WebSocketsModule.h"
#include "Containers/Ticker.h"
#include "Misc/SecureHash.h"

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
	CancelKeepAlive();
}

void UNetClientSubsystem::Open(const FString& WsUrl, const FString& PlayTicket)
{
	CloseSocket();
	PreviousTicketDigest = NewestTicketDigest;
	NewestTicketDigest = FMD5::HashAnsiString(*PlayTicket);
	++TicketsPresented;
	Socket = SocketFactory(MakeUpgradeRequest(WsUrl, PlayTicket));
	UE_LOG(LogNightfall, Log, TEXT("ws connecting to %s"), *WsUrl);

	Socket->OnConnected().AddLambda([this]()
	{
		bConnected = true;
		ReconnectAttempt = 0;
		Frame.Reset();
		KnownEntities.Reset();
		Tombstones.Reset();   // the server re-sends every spawn in our area of interest
		TickTimeOriginMs.Reset();   // a new session may be a new zone epoch; the next EntityMove re-learns it
		KeepAliveSeqs.Reset();
		ArmKeepAlive();
		UE_LOG(LogNightfall, Log, TEXT("ws connected"));
		OnConnected.Broadcast();
	});
	Socket->OnConnectionError().AddLambda([this](const FString& Error)
	{
		// A rejected ticket (HTTP 401/409 at the upgrade) also lands here.
		UE_LOG(LogNightfall, Warning, TEXT("ws connection error: %s"), *Error);
		bConnected = false;
		CancelKeepAlive();
		ScheduleReconnect();
	});
	Socket->OnClosed().AddUObject(this, &UNetClientSubsystem::HandleClosed);
	Socket->OnRawMessage().AddUObject(this, &UNetClientSubsystem::HandleRawMessage);
	Socket->Connect();
}

void UNetClientSubsystem::HandleClosed(int32 StatusCode, const FString& Reason, bool bWasClean)
{
	OnWireClosed.Broadcast(StatusCode);
	ApplyClosedState(StatusCode, Reason, bWasClean);
}

void UNetClientSubsystem::ApplyClosedState(int32 StatusCode, const FString& Reason, bool bWasClean)
{
	UE_LOG(LogNightfall, Log, TEXT("ws closed (%d, clean=%d): %s"), StatusCode, bWasClean, *Reason);
	bConnected = false;
	LastCloseCode = StatusCode;
	LastCloseReason = Reason;
	CancelKeepAlive();
	if (StatusCode == CloseCodeReplaced)
	{
		// Replaced by a newer session of this entity: reconnecting would replace that one in turn.
		UE_LOG(LogNightfall, Log, TEXT("ws: replaced by a newer session; not reconnecting"));
		bWantConnected = false;
	}
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
	OnWireReceived.Broadcast(Frame); // diagnostics validates raw envelopes before projection conversion
	const bool bOk = NightfallProto::Decode(Frame.GetData(), Frame.Num(), Msg);
	Frame.Reset();
	if (!bOk)
	{
		UE_LOG(LogNightfall, Warning, TEXT("ws: malformed ServerMessage dropped"));
		return;
	}

	DispatchServerMessage(Msg);
}

void UNetClientSubsystem::DispatchServerMessage(const FServerMessage& Msg)
{
	if (Msg.Ack.IsSet())
	{
		LastAckedSeq = FMath::Max(LastAckedSeq, Msg.Ack->Seq);
		UE_LOG(LogNightfall, Verbose, TEXT("ws: ack seq %u (applies on tick %llu)"), Msg.Ack->Seq, Msg.Ack->Tick);
		OnIntentAck.Broadcast(*Msg.Ack);
	}
	if (Msg.Rejected.IsSet() && KeepAliveSeqs.Remove(Msg.Rejected->Seq) > 0)
	{
		// The expected answer to a keep-alive: not an intent anybody is waiting on.
		LastAckedSeq = FMath::Max(LastAckedSeq, Msg.Rejected->Seq);
		UE_LOG(LogNightfall, VeryVerbose, TEXT("ws: keep-alive seq %u answered"), Msg.Rejected->Seq);
	}
	else if (Msg.Rejected.IsSet())
	{
		LastAckedSeq = FMath::Max(LastAckedSeq, Msg.Rejected->Seq);
		UE_LOG(LogNightfall, Warning, TEXT("ws: intent %u rejected (reason %u): %s"),
			Msg.Rejected->Seq, Msg.Rejected->Reason, *Msg.Rejected->Detail);
		OnIntentRejected.Broadcast(*Msg.Rejected);
	}
	if (Msg.Event.IsSet())
	{
		const FWorldEvent& E = *Msg.Event;
		if (E.Spawn.IsSet())
		{
			const FEntitySpawn* Known = KnownEntities.Find(E.Spawn->EntityId);
			if (!Known) Known = Tombstones.Find(E.Spawn->EntityId);
			if (Known && NightfallProto::IsStaleSpawn(*Known, *E.Spawn))
			{
				// A replaced session's or an earlier life's spawn arriving late: the newer one wins.
				UE_LOG(LogNightfall, Verbose, TEXT("ws: stale spawn for %s dropped"), *E.Spawn->EntityId);
			}
			else
			{
				SnapshotBuffer.Push(E.Spawn->EntityId, E.Spawn->Position, EstimatedServerTimeMs());
				KnownEntities.Add(E.Spawn->EntityId, *E.Spawn);
				Tombstones.Remove(E.Spawn->EntityId);
				OnEntitySpawn.Broadcast(*E.Spawn);
				OnEntitySpawnProjected.Broadcast(*E.Spawn);
			}
		}
		if (E.Move.IsSet())
		{
			// Learn the server clock from authoritative timestamps (simple EMA).
			const int64 LocalNow = FDateTime::UtcNow().ToUnixTimestamp() * 1000 + FDateTime::UtcNow().GetMillisecond();
			const int64 Observed = E.Move->ServerTimeMs - LocalNow;
			ServerClockOffsetMs = (ServerClockOffsetMs == 0) ? Observed : (ServerClockOffsetMs * 7 + Observed) / 8;

			TickTimeOriginMs = E.Move->ServerTimeMs - static_cast<int64>(E.Move->Tick) * TICK_MS;
			SnapshotBuffer.Push(E.Move->EntityId, E.Move->Position, E.Move->ServerTimeMs);
			OnEntityMove.Broadcast(*E.Move);
		}
		if (E.Despawn.IsSet())
		{
			UE_LOG(LogNightfall, Log, TEXT("ws: despawn %s"), *E.Despawn->EntityId);
			SnapshotBuffer.Remove(E.Despawn->EntityId);
			if (const FEntitySpawn* Gone = KnownEntities.Find(E.Despawn->EntityId)) Tombstones.Add(E.Despawn->EntityId, *Gone);
			KnownEntities.Remove(E.Despawn->EntityId);
			OnEntityDespawn.Broadcast(*E.Despawn);
		}
		if (E.AttackResult.IsSet()) OnAttackResult.Broadcast(*E.AttackResult);
		if (E.EntityDied.IsSet()) OnEntityDied.Broadcast(*E.EntityDied);
		if (E.EntityRespawned.IsSet())
		{
			// The respawn point is authoritative: remote proxies jump there; the own pawn is snapped by UOwnEntityComponent.
			// The new life advances the fence on the cached spawn (live or tombstoned), so a spawn,
			// death or swing of an earlier life arriving afterwards is stale. An older life's
			// respawn is itself stale and dropped.
			const FEntityRespawned& Re = *E.EntityRespawned;
			FEntitySpawn* Held = KnownEntities.Find(Re.Entity);
			if (!Held) Held = Tombstones.Find(Re.Entity);
			const bool bStaleRespawn = Held && Held->LifeIncarnation != 0 && Re.Incarnation != 0 && Re.Incarnation < Held->LifeIncarnation;
			if (bStaleRespawn)
			{
				UE_LOG(LogNightfall, Verbose, TEXT("ws: stale respawn for %s dropped"), *Re.Entity);
			}
			else
			{
				if (Held && Re.Incarnation > Held->LifeIncarnation) Held->LifeIncarnation = Re.Incarnation;
				SnapshotBuffer.Push(Re.Entity, Re.Position, EstimatedServerTimeMs());
				OnEntityRespawned.Broadcast(Re);
			}
		}
		if (E.ClassChanged.IsSet())
		{
			const FClassChanged& Changed = *E.ClassChanged;
			FEntitySpawn* Held = KnownEntities.Find(Changed.Entity);
			// A class fact needs an admitted entity and must match its session incarnation.
			if (Held && Changed.SessionGeneration == Held->SessionGeneration && Changed.Tick >= Held->StateTick
				&& !(Changed.Tick == Held->StateTick && Changed.ClassId == Held->ClassId))
			{
				Held->ClassId = Changed.ClassId;
				Held->StateTick = Changed.Tick;
				OnClassChanged.Broadcast(Changed);
			}
		}
		if (E.StatsChanged.IsSet()) OnStatsChanged.Broadcast(*E.StatsChanged);
		if (E.XpGained.IsSet()) OnXpGained.Broadcast(*E.XpGained);
		if (E.LevelUp.IsSet()) OnLevelUp.Broadcast(*E.LevelUp);
		if (E.TargetChanged.IsSet()) OnTargetChanged.Broadcast(*E.TargetChanged);
	}
}

uint32 UNetClientSubsystem::SendMoveTo(const FNetVec2& Destination)
{
	FClientMessage Msg;
	Msg.Seq = ++NextSeq;
	Msg.MoveTo = FMoveToIntent{ Destination };
	UE_LOG(LogNightfall, Verbose, TEXT("ws: send MoveTo seq %u to tile (%.2f, %.2f)%s"),
		Msg.Seq, Destination.X, Destination.Y, bConnected ? TEXT("") : TEXT(" [not connected: dropped]"));
	Send(Msg);
	return Msg.Seq;
}

uint32 UNetClientSubsystem::SendSetTarget(const FString& EntityId)
{
	FClientMessage Msg;
	Msg.Seq = ++NextSeq;
	Msg.SetTarget = FSetTargetIntent{ EntityId };
	UE_LOG(LogNightfall, Verbose, TEXT("ws: send SetTarget seq %u -> '%s'"), Msg.Seq, *EntityId);
	Send(Msg);
	return bConnected ? Msg.Seq : 0;
}

uint32 UNetClientSubsystem::SendAttack()
{
	FClientMessage Msg;
	Msg.Seq = ++NextSeq;
	Msg.bAttack = true;
	UE_LOG(LogNightfall, Verbose, TEXT("ws: send Attack seq %u"), Msg.Seq);
	Send(Msg);
	return bConnected ? Msg.Seq : 0;
}

uint32 UNetClientSubsystem::SendStopAttack()
{
	FClientMessage Msg;
	Msg.Seq = ++NextSeq;
	Msg.bStopAttack = true;
	UE_LOG(LogNightfall, Verbose, TEXT("ws: send StopAttack seq %u"), Msg.Seq);
	Send(Msg);
	return bConnected ? Msg.Seq : 0;
}

uint32 UNetClientSubsystem::SendRespawn()
{
	FClientMessage Msg;
	Msg.Seq = ++NextSeq;
	Msg.bRespawn = true;
	UE_LOG(LogNightfall, Verbose, TEXT("ws: send Respawn seq %u"), Msg.Seq);
	Send(Msg);
	return bConnected ? Msg.Seq : 0;
}

void UNetClientSubsystem::Send(const FClientMessage& Msg)
{
	if (!Socket.IsValid() || !bConnected) return;
	TArray<uint8> Bytes;
	NightfallProto::Encode(Msg, Bytes);
	Socket->Send(Bytes.GetData(), Bytes.Num(), /*bIsBinary=*/true);
	OnWireSent.Broadcast(Bytes);
	bSentSinceKeepAliveArmed = true;
}

void UNetClientSubsystem::ArmKeepAlive()
{
	bSentSinceKeepAliveArmed = false;
	Scheduler(KeepAliveSeconds, [Weak = TWeakObjectPtr<UNetClientSubsystem>(this), Gen = ++KeepAliveGeneration]()
	{
		if (Weak.IsValid() && Weak->KeepAliveGeneration == Gen)
		{
			Weak->OnKeepAliveTimer();
		}
	});
}

void UNetClientSubsystem::OnKeepAliveTimer()
{
	if (!bConnected) return;
	if (!bSentSinceKeepAliveArmed)
	{
		// No intent set: the server answers IntentRejected{INVALID} and resets its idle timer.
		FClientMessage Msg;
		Msg.Seq = ++NextSeq;
		KeepAliveSeqs.Add(Msg.Seq);
		UE_LOG(LogNightfall, VeryVerbose, TEXT("ws: send keep-alive seq %u"), Msg.Seq);
		Send(Msg);
	}
	ArmKeepAlive();
}

int64 UNetClientSubsystem::TickToServerTimeMs(uint64 Tick) const
{
	return TickTimeOriginMs.IsSet() ? *TickTimeOriginMs + static_cast<int64>(Tick) * TICK_MS : EstimatedServerTimeMs();
}

int64 UNetClientSubsystem::EstimatedServerTimeMs() const
{
	const FDateTime Now = FDateTime::UtcNow();
	return Now.ToUnixTimestamp() * 1000 + Now.GetMillisecond() + ServerClockOffsetMs;
}

#if !UE_BUILD_SHIPPING
void UNetClientSubsystem::DropSocketForTesting()
{
	if (!Socket.IsValid()) return;
	CloseSocket();
	ApplyClosedState(1006, TEXT("dropped by nf.DropSocket"), false);
}
#endif
