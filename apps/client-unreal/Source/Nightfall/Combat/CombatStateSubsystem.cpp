#include "CombatStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "Game/OwnEntityComponent.h"
#include "Nightfall.h"
#include "Engine/GameInstance.h"

namespace
{
	constexpr int32 MaxRememberedEvents = 512;

	float Fraction(uint32 Value, uint32 Max)
	{
		return Max == 0 ? 0.f : FMath::Clamp(static_cast<float>(Value) / static_cast<float>(Max), 0.f, 1.f);
	}
}

void UCombatStateSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Collection.InitializeDependency<UNetClientSubsystem>();
	Super::Initialize(Collection);
	UNetClientSubsystem* N = Net();
	if (!N) return;
	Handles.Add(N->OnEntitySpawn.AddUObject(this, &UCombatStateSubsystem::ApplySpawn));
	Handles.Add(N->OnEntityDespawn.AddUObject(this, &UCombatStateSubsystem::ApplyDespawn));
	Handles.Add(N->OnAttackResult.AddUObject(this, &UCombatStateSubsystem::ApplyAttackResult));
	Handles.Add(N->OnEntityDied.AddUObject(this, &UCombatStateSubsystem::ApplyDied));
	Handles.Add(N->OnEntityRespawned.AddUObject(this, &UCombatStateSubsystem::ApplyRespawned));
	Handles.Add(N->OnStatsChanged.AddUObject(this, &UCombatStateSubsystem::ApplyStats));
	Handles.Add(N->OnXpGained.AddUObject(this, &UCombatStateSubsystem::ApplyXp));
	Handles.Add(N->OnLevelUp.AddUObject(this, &UCombatStateSubsystem::ApplyLevelUp));
	Handles.Add(N->OnTargetChanged.AddUObject(this, &UCombatStateSubsystem::ApplyTargetChanged));
	Handles.Add(N->OnIntentAck.AddUObject(this, &UCombatStateSubsystem::ApplyAck));
	Handles.Add(N->OnIntentRejected.AddUObject(this, &UCombatStateSubsystem::ApplyRejected));
	// The server re-sends every spawn (and the owner's stats) on a new connection, so the old
	// projection is dropped on both edges: nothing from the previous session can leak into the next.
	N->OnConnected.AddDynamic(this, &UCombatStateSubsystem::Reset);
	N->OnDisconnected.AddDynamic(this, &UCombatStateSubsystem::HandleNetDisconnected);
}

void UCombatStateSubsystem::Deinitialize()
{
	if (UNetClientSubsystem* N = Net())
	{
		N->OnEntitySpawn.Remove(Handles[0]);
		N->OnEntityDespawn.Remove(Handles[1]);
		N->OnAttackResult.Remove(Handles[2]);
		N->OnEntityDied.Remove(Handles[3]);
		N->OnEntityRespawned.Remove(Handles[4]);
		N->OnStatsChanged.Remove(Handles[5]);
		N->OnXpGained.Remove(Handles[6]);
		N->OnLevelUp.Remove(Handles[7]);
		N->OnTargetChanged.Remove(Handles[8]);
		N->OnIntentAck.Remove(Handles[9]);
		N->OnIntentRejected.Remove(Handles[10]);
		N->OnConnected.RemoveDynamic(this, &UCombatStateSubsystem::Reset);
		N->OnDisconnected.RemoveDynamic(this, &UCombatStateSubsystem::HandleNetDisconnected);
	}
	Handles.Reset();
	Super::Deinitialize();
}

UNetClientSubsystem* UCombatStateSubsystem::Net() const
{
	const UGameInstance* GI = GetGameInstance();
	return GI ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr;
}

bool UCombatStateSubsystem::IsOwn(const FString& EntityId) const
{
	const UNetClientSubsystem* N = Net();
	return N && N->IsOwnEntity(EntityId);
}

void UCombatStateSubsystem::HandleNetDisconnected(const FString& Reason)
{
	Reset();
}

void UCombatStateSubsystem::Reset()
{
	Entities.Reset();
	Own = FOwnCombatState();
	AttackState = EAttackState::Idle;
	PendingTargetId.Reset();
	bClearPending = false;
	AttackSeq = 0;
	RespawnSeq = 0;
	InFlight.Reset();
	Tombstones.Reset();
	SeenEvents.Reset();   // a restarted zone repeats tick numbers
	SeenOrder.Reset();
	OnChanged.Broadcast();
}

const FCombatEntity* UCombatStateSubsystem::FindEntity(const FString& EntityId) const
{
	return Entities.Find(Key(EntityId));
}

const FCombatEntity* UCombatStateSubsystem::FindOwnEntity() const
{
	const UNetClientSubsystem* N = Net();
	return N && !N->GetOwnEntityId().IsEmpty() ? FindEntity(N->GetOwnEntityId()) : nullptr;
}

bool UCombatStateSubsystem::IsOwnDead() const
{
	const FCombatEntity* E = FindOwnEntity();
	return E && E->bDead;
}

bool UCombatStateSubsystem::IsAttackable(const FString& EntityId) const
{
	const FCombatEntity* E = FindEntity(EntityId);
	return E && E->bCombatant && E->bAttackable && !E->bDead;
}

FCombatEntity& UCombatStateSubsystem::FindOrAdd(const FString& EntityId)
{
	FCombatEntity& E = Entities.FindOrAdd(Key(EntityId));
	if (E.Spawn.EntityId.IsEmpty()) E.Spawn.EntityId = EntityId;
	return E;
}

void UCombatStateSubsystem::ClearTarget()
{
	Own.TargetId.Reset();
	PendingTargetId.Reset();
	bClearPending = false;
	EndAttack();
}

bool UCombatStateSubsystem::RememberEvent(const FString& EventKey)
{
	if (SeenEvents.Contains(EventKey)) return false;
	SeenEvents.Add(EventKey);
	SeenOrder.Add(EventKey);
	if (SeenOrder.Num() > MaxRememberedEvents)
	{
		SeenEvents.Remove(SeenOrder[0]);
		SeenOrder.RemoveAt(0);
	}
	return true;
}

// --- Projection ------------------------------------------------------------------------------

void UCombatStateSubsystem::ApplySpawn(const FEntitySpawn& Spawn)
{
	FEntitySpawn Held;
	bool bHeld = false;
	if (const FCombatEntity* Known = FindEntity(Spawn.EntityId))
	{
		Held = Known->Spawn;
		Held.LifeIncarnation = Known->Incarnation;
		bHeld = true;
	}
	else if (const FEntitySpawn* Gone = Tombstones.Find(Key(Spawn.EntityId)))
	{
		Held = *Gone;
		bHeld = true;
	}
	if (bHeld && NightfallProto::IsStaleSpawn(Held, Spawn))
	{
		UE_LOG(LogNightfall, Verbose, TEXT("combat: stale spawn for %s ignored"), *Spawn.EntityId);
		return;
	}
	Tombstones.Remove(Key(Spawn.EntityId));
	FCombatEntity& E = FindOrAdd(Spawn.EntityId);
	E.Spawn = Spawn;
	E.bCombatant = Spawn.bCombatant;
	E.bDead = Spawn.bDead;
	E.bAttackable = Spawn.bAttackable;
	E.Incarnation = Spawn.LifeIncarnation;
	E.Hp = Spawn.Hp;
	E.MaxHp = Spawn.MaxHp;
	E.Level = Spawn.Level;
	E.LastFactTick = 0;
	if (Key(Spawn.EntityId) == Own.TargetId && (E.bDead || !E.bAttackable)) ClearTarget();
	if (IsOwn(Spawn.EntityId) && E.bDead) ClearTarget();
	OnChanged.Broadcast();
}

void UCombatStateSubsystem::ApplyDespawn(const FEntityDespawn& Despawn)
{
	const FString K = Key(Despawn.EntityId);
	if (const FCombatEntity* Gone = Entities.Find(K))
	{
		FEntitySpawn Last = Gone->Spawn;
		Last.LifeIncarnation = Gone->Incarnation;
		Tombstones.Add(K, Last);
	}
	Entities.Remove(K);
	if (K == Own.TargetId || K == PendingTargetId) ClearTarget();   // authoritative: the target left
	OnChanged.Broadcast();
}

void UCombatStateSubsystem::ApplyAttackResult(const FAttackResult& R)
{
	if (R.Outcome == ENetAttackOutcome::Unspecified) return;   // invalid on the wire
	FCombatEntity* Target = Entities.Find(Key(R.Target));
	if (Target)
	{
		// A swing that landed on an earlier life, or older than what we already applied, is stale.
		if (Target->Incarnation != 0 && R.TargetIncarnation != 0 && R.TargetIncarnation < Target->Incarnation) return;
		if (R.Tick < Target->LastFactTick) return;
		if (Target->bDead && !(R.TargetIncarnation > Target->Incarnation))
		{
			// AOI entry carries the end-of-tick snapshot before that tick's facts. A
			// newly visible corpse can therefore precede its legitimate lethal result.
			// Show that cue once without reviving it; an established death only admits
			// terminal facts at its own tick, never a later post-death result.
			const bool bTerminalCue = R.TargetIncarnation == Target->Incarnation && R.TargetHpAfter == 0
				&& (Target->LastFactTick == 0 || R.Tick == Target->LastFactTick);
			if (!bTerminalCue) return;
		}
	}
	if (!RememberEvent(FString::Printf(TEXT("%llu|%s|%s"), R.Tick, *Key(R.Attacker), *Key(R.Target)))) return;   // replayed event

	if (Target)
	{
		if (R.TargetIncarnation > Target->Incarnation)
		{
			Target->Incarnation = R.TargetIncarnation;   // a new life: it is alive now
			Target->bDead = false;
		}
		Target->Hp = R.TargetHpAfter;
		Target->LastFactTick = R.Tick;
	}

	FDamageNumber Number;
	Number.AttackerId = R.Attacker;
	Number.TargetId = R.Target;
	Number.Tick = R.Tick;
	Number.Outcome = R.Outcome;
	Number.Damage = R.Damage;
	Number.bTargetIsOwn = IsOwn(R.Target);
	OnDamageNumber.Broadcast(Number);
	OnChanged.Broadcast();
}

void UCombatStateSubsystem::ApplyDied(const FEntityDied& Died)
{
	FCombatEntity* E = Entities.Find(Key(Died.Entity));
	if (!E) return;   // not in our area of interest
	if (E->Incarnation != 0 && Died.Incarnation != 0 && Died.Incarnation < E->Incarnation) return;
	if (Died.Tick < E->LastFactTick) return;
	if (Died.Incarnation > E->Incarnation) E->Incarnation = Died.Incarnation;
	E->bDead = true;
	E->Hp = 0;
	E->LastFactTick = Died.Tick;
	const FString K = Key(Died.Entity);
	if (K == Own.TargetId || K == PendingTargetId || IsOwn(Died.Entity)) ClearTarget();
	OnChanged.Broadcast();
}

void UCombatStateSubsystem::ApplyRespawned(const FEntityRespawned& Respawned)
{
	FCombatEntity* E = Entities.Find(Key(Respawned.Entity));
	if (!E) return;
	// A respawn of an earlier life, or older than what we already applied, is stale.
	if (E->Incarnation != 0 && Respawned.Incarnation != 0 && Respawned.Incarnation < E->Incarnation) return;
	if (Respawned.Tick < E->LastFactTick) return;
	if (Respawned.Incarnation > E->Incarnation) E->Incarnation = Respawned.Incarnation;   // the life fence advances
	E->bDead = false;
	E->Hp = Respawned.Hp;
	E->LastFactTick = Respawned.Tick;
	if (IsOwn(Respawned.Entity)) RespawnSeq = 0;
	OnChanged.Broadcast();
}

void UCombatStateSubsystem::ApplyStats(const FStatsChanged& Stats)
{
	if (!IsOwn(Stats.Entity)) return;   // owner-only: MP is private
	FCombatEntity& E = FindOrAdd(Stats.Entity);
	E.bCombatant = true;
	E.Hp = Stats.Hp;
	E.MaxHp = Stats.MaxHp;
	E.Level = Stats.Level;
	Own.bXpKnown = true;   // StatsChanged carries the authoritative total: admission, death loss, reconnect
	Own.Xp = Stats.Xp;
	Own.bMpKnown = true;
	Own.Mp = Stats.Mp;
	Own.MaxMp = Stats.MaxMp;
	OnChanged.Broadcast();
}

void UCombatStateSubsystem::ApplyXp(const FXpGained& Xp)
{
	if (!IsOwn(Xp.Entity)) return;   // owner-only
	Own.bXpKnown = true;
	Own.Xp = Xp.Total;
	Own.LastXpGain = Xp.Amount;
	OnChanged.Broadcast();
}

void UCombatStateSubsystem::ApplyLevelUp(const FLevelUp& LevelUp)
{
	if (FCombatEntity* E = Entities.Find(Key(LevelUp.Entity)))
	{
		E->Level = LevelUp.Level;
		OnChanged.Broadcast();
	}
}

void UCombatStateSubsystem::ApplyTargetChanged(const FTargetChanged& Changed)
{
	if (!IsOwn(Changed.Entity)) return;   // owner-only
	const FString NewTarget = Key(Changed.Target);
	// With a newer selection of ours in flight, this is the echo of an earlier one: it must not
	// end the attack we have queued for the newer target.
	if (PendingTargetId.IsEmpty() && NewTarget != Own.TargetId) EndAttack();
	Own.TargetId = NewTarget;
	if (PendingTargetId == NewTarget || NewTarget.IsEmpty()) PendingTargetId.Reset();
	if (NewTarget.IsEmpty()) bClearPending = false;
	OnChanged.Broadcast();
}

// --- Intents ---------------------------------------------------------------------------------

uint32 UCombatStateSubsystem::Track(uint32 Seq, EIntentKind Kind)
{
	if (Seq != 0) InFlight.Add(Seq, Kind);
	return Seq;
}

bool UCombatStateSubsystem::ClickEntity(const FString& EntityId)
{
	UNetClientSubsystem* N = Net();
	if (!N || IsOwnDead() || !IsAttackable(EntityId)) return false;
	const FString K = Key(EntityId);

	const FString Requested = RequestedTarget();   // what the server will end up with
	if (Requested != K)
	{
		const uint32 Seq = Track(N->SendSetTarget(EntityId), EIntentKind::SetTarget);
		if (Seq == 0) return false;   // not connected
		PendingTargetId = K;
		EndAttack();                  // a different selection ends the previous attack
	}
	if (AttackState == EAttackState::Idle)
	{
		AttackSeq = Track(N->SendAttack(), EIntentKind::Attack);
		if (AttackSeq != 0) AttackState = EAttackState::Pending;
	}
	OnChanged.Broadcast();
	return true;
}

FString UCombatStateSubsystem::RequestedTarget() const
{
	if (bClearPending) return FString();
	return PendingTargetId.IsEmpty() ? Own.TargetId : PendingTargetId;
}

uint32 UCombatStateSubsystem::SelectTarget(const FString& EntityId)
{
	UNetClientSubsystem* N = Net();
	if (!N) return 0;
	const FString K = Key(EntityId);
	if (RequestedTarget() == K) return 0;
	const uint32 Seq = Track(N->SendSetTarget(EntityId), EIntentKind::SetTarget);
	if (Seq == 0) return 0;   // not connected
	PendingTargetId = K;
	bClearPending = K.IsEmpty();   // until TargetChanged(none): the selection the server will end up with is none
	EndAttack();              // a different selection ends the previous attack
	OnChanged.Broadcast();
	return Seq;
}

uint32 UCombatStateSubsystem::AttackSelection()
{
	UNetClientSubsystem* N = Net();
	const FString Selected = RequestedTarget();
	if (!N || IsOwnDead() || Selected.IsEmpty() || AttackState != EAttackState::Idle) return 0;
	AttackSeq = Track(N->SendAttack(), EIntentKind::Attack);
	if (AttackSeq != 0) AttackState = EAttackState::Pending;
	OnChanged.Broadcast();
	return AttackSeq;
}

uint32 UCombatStateSubsystem::NoteGroundClick()
{
	if (AttackState == EAttackState::Idle) return 0;
	UNetClientSubsystem* N = Net();
	EndAttack();   // the MoveTo that follows ends it on the server too
	const uint32 Seq = N ? Track(N->SendStopAttack(), EIntentKind::StopAttack) : 0;
	OnChanged.Broadcast();
	return Seq;
}

uint32 UCombatStateSubsystem::RequestRespawn()
{
	UNetClientSubsystem* N = Net();
	if (!N || !IsOwnDead() || RespawnSeq != 0) return 0;
	RespawnSeq = Track(N->SendRespawn(), EIntentKind::Respawn);
	OnChanged.Broadcast();
	return RespawnSeq;
}

void UCombatStateSubsystem::ApplyAck(const FAck& Ack)
{
	const EIntentKind* Kind = InFlight.Find(Ack.Seq);
	if (!Kind) return;
	if (*Kind == EIntentKind::Attack && Ack.Seq == AttackSeq && AttackState == EAttackState::Pending)
	{
		AttackState = EAttackState::Active;
		OnChanged.Broadcast();
	}
	InFlight.Remove(Ack.Seq);
}

void UCombatStateSubsystem::ApplyRejected(const FIntentRejected& Rejected)
{
	const EIntentKind* Found = InFlight.Find(Rejected.Seq);
	if (!Found) return;   // not a combat intent (the own-entity component handles MoveTo)
	const EIntentKind Kind = *Found;
	InFlight.Remove(Rejected.Seq);
	const FString Reason = UOwnEntityComponent::RejectReasonText(Rejected.Reason);
	switch (Kind)
	{
	case EIntentKind::SetTarget:
		PendingTargetId.Reset();
		bClearPending = false;
		if (AttackState != EAttackState::Idle)
		{
			// The Attack queued behind this SetTarget would hit the previous selection: cancel it.
			EndAttack();
			if (UNetClientSubsystem* N = Net()) Track(N->SendStopAttack(), EIntentKind::StopAttack);
		}
		SetStatus(FString::Printf(TEXT("Can't target that: %s"), *Reason));
		break;
	case EIntentKind::Attack:
		if (Rejected.Seq == AttackSeq) EndAttack();
		SetStatus(FString::Printf(TEXT("Can't attack: %s"), *Reason));
		break;
	case EIntentKind::StopAttack:
		AttackState = EAttackState::Active;   // unknown whether it stopped: the next ground click retries
		SetStatus(FString::Printf(TEXT("Can't stop attacking: %s"), *Reason));
		break;
	case EIntentKind::Respawn:
		if (Rejected.Seq == RespawnSeq) RespawnSeq = 0;
		SetStatus(FString::Printf(TEXT("Can't respawn: %s"), *Reason));
		break;
	}
	OnChanged.Broadcast();
}

void UCombatStateSubsystem::SetStatus(const FString& Text)
{
	StatusLine = Text;
	const UGameInstance* GI = GetGameInstance();
	if (ULoginFlowSubsystem* Flow = GI ? GI->GetSubsystem<ULoginFlowSubsystem>() : nullptr)
	{
		Flow->SetStatus(Text);
	}
	else
	{
		UE_LOG(LogNightfall, Log, TEXT("%s"), *Text);
	}
}

// --- HUD model -------------------------------------------------------------------------------

FCombatHudModel UCombatStateSubsystem::BuildHudModel() const
{
	FCombatHudModel M;
	if (const FCombatEntity* O = FindOwnEntity())
	{
		M.bOwnKnown = O->bCombatant;
		M.OwnName = O->Spawn.Name;
		M.LevelText = FString::Printf(TEXT("Lv %u"), O->Level);
		M.OwnHpFraction = Fraction(O->Hp, O->MaxHp);
		M.OwnHpText = FString::Printf(TEXT("%u / %u"), O->Hp, O->MaxHp);
		M.bDeadOverlay = O->bDead;
	}
	if (Own.bMpKnown)
	{
		M.OwnMpFraction = Fraction(Own.Mp, Own.MaxMp);
		M.OwnMpText = FString::Printf(TEXT("MP %u / %u"), Own.Mp, Own.MaxMp);
	}
	else
	{
		M.OwnMpText = TEXT("MP --");
	}
	M.XpText = Own.bXpKnown ? FString::Printf(TEXT("XP %llu"), Own.Xp) : FString(TEXT("XP --"));
	M.bRespawnPending = RespawnSeq != 0;
	M.AttackState = AttackState;
	M.AttackText = AttackState == EAttackState::Pending ? TEXT("Attacking...") : AttackState == EAttackState::Active ? TEXT("Attacking") : TEXT("");

	if (!Own.TargetId.IsEmpty())
	{
		if (const FCombatEntity* T = FindEntity(Own.TargetId))
		{
			M.bTargetVisible = true;
			M.TargetName = T->Spawn.Name;
			M.TargetLevel = T->Level;
			M.TargetHpFraction = Fraction(T->Hp, T->MaxHp);
			M.TargetHpText = FString::Printf(TEXT("%u / %u"), T->Hp, T->MaxHp);
		}
	}
	return M;
}
