#include "BotPredicates.h"
#include "BotFixtureData.h"
#include "BotScenarioRunner.h"
#include "Auth/AuthSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Game/NightfallGameMode.h"
#include "Net/NetClientSubsystem.h"
#include "World/WorldProxySubsystem.h"
#include "SNightfallV1/WorldMessage.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"

// --- Observations ------------------------------------------------------------------------------

void FBotObservations::Bind(UGameInstance* GameInstance)
{
	Unbind();
	UNetClientSubsystem* Net = GameInstance ? GameInstance->GetSubsystem<UNetClientSubsystem>() : nullptr;
	UCombatStateSubsystem* Combat = GameInstance ? GameInstance->GetSubsystem<UCombatStateSubsystem>() : nullptr;
	if (Net)
	{
		BoundNet = Net;
		AckHandle = Net->OnIntentAck.AddLambda([this](const FAck& Ack) { LastAckSeq = FMath::Max(LastAckSeq, Ack.Seq); });
		SpawnHandle = Net->OnEntitySpawn.AddLambda([this, Net](const FEntitySpawn& Spawn)
		{
			if (!Net->IsOwnEntity(Spawn.EntityId)) return;
			++OwnSpawns;
			if (FirstOwnEntityId.IsEmpty()) FirstOwnEntityId = Spawn.EntityId.ToLower();
		});
		RejectHandle = Net->OnIntentRejected.AddLambda([this](const FIntentRejected& Rejected)
		{
			bRejected = true;
			LastRejectReason = Rejected.Reason;
			LastRejectSeq = Rejected.Seq;
		});
		// The last target's HP is kept from the events, not only the projection: the corpse may
		// despawn (and leave the projection) before a wait on target_hp == 0 polls again.
		TargetHandle = Net->OnTargetChanged.AddLambda([this, Net](const FTargetChanged& Changed)
		{
			if (!Net->IsOwnEntity(Changed.Entity) || Changed.Target.IsEmpty() || Changed.Target.Equals(LastTargetId, ESearchCase::IgnoreCase)) return;
			LastTargetId = Changed.Target.ToLower();
			LastTargetHp.Reset();
			Observe(BoundCombat.Get());
		});
		HitHandle = Net->OnAttackResult.AddLambda([this](const FAttackResult& Result)
		{
			if (!LastTargetId.IsEmpty() && Result.Target.Equals(LastTargetId, ESearchCase::IgnoreCase) && Result.Outcome != ENetAttackOutcome::Unspecified) LastTargetHp = Result.TargetHpAfter;
		});
		DiedHandle = Net->OnEntityDied.AddLambda([this](const FEntityDied& Died)
		{
			if (!LastTargetId.IsEmpty() && Died.Entity.Equals(LastTargetId, ESearchCase::IgnoreCase)) LastTargetHp = 0u;
		});
		BindPhase1(Net);
	}
	if (Combat)
	{
		BoundCombat = Combat;
		NumberHandle = Combat->OnDamageNumber.AddLambda([this](const FDamageNumber&) { ++DamageNumbers; });
	}
}

void FBotObservations::Unbind()
{
	if (UNetClientSubsystem* Net = BoundNet.Get())
	{
		Net->OnEntitySpawn.Remove(SpawnHandle);
		Net->OnIntentAck.Remove(AckHandle);
		Net->OnIntentRejected.Remove(RejectHandle);
		Net->OnTargetChanged.Remove(TargetHandle);
		Net->OnAttackResult.Remove(HitHandle);
		Net->OnEntityDied.Remove(DiedHandle);
	}
	for (const TFunction<void()>& Unbind : Phase1Unbinders) Unbind();
	Phase1Unbinders.Reset();
	if (UCombatStateSubsystem* Combat = BoundCombat.Get())
	{
		Combat->OnDamageNumber.Remove(NumberHandle);
	}
	BoundNet.Reset();
	BoundCombat.Reset();
}

void FBotObservations::Reset()
{
	Acks = 0;
	OwnSpawns = 0;
	FirstOwnEntityId.Reset();
	LastAckSeq = 0;
	bRejected = false;
	LastRejectReason = 0;
	LastRejectSeq = 0;
	DamageNumbers = 0;
	LastTargetId.Reset();
	LastTargetHp.Reset();
	AttackResultKeys.Reset();
	NpcsAttackingOwn.Reset();
	NpcsAttackingPlayers.Reset();
	NewestTick = 0;
	bOwnRespawned = false;
	OwnRespawnTick = 0;
	bAttackedSinceRespawn = false;
	TrackedXp.Reset();
	TrackedLevel = 0;
	bOwnDeathStatsSeen = false;
	XpBeforeDeath.Reset();
	LevelBeforeDeath = 0;
	XpAfterDeath.Reset();
	Connects = 0;
	bAwaitingStats = false;
	bXpUnknownSeenAfterReconnect = false;
	XpKnownBeforeStats = 0;
	XpAtDisconnect.Reset();
	XpGainedSinceReconnect = 0;
	WoundedSpawns.Reset();
	SpawnsMidFight = 0;
	LateSpawnHpOk = 0;
	LateSpawnHpBad = 0;
	MovingEntities.Reset();
	LastHitOn.Reset();
	LateSpawnProjectionOk = 0;
	LateSpawnProjectionBad = 0;
	HitMarks.Reset();
}

void FBotObservations::Observe(const UCombatStateSubsystem* Combat)
{
	if (!Combat) return;
	ObservePhase1(Combat);
	if (!Combat->GetTargetId().IsEmpty() && Combat->GetTargetId() != LastTargetId)
	{
		LastTargetId = Combat->GetTargetId();
		LastTargetHp.Reset();
	}
	if (const FCombatEntity* E = LastTargetId.IsEmpty() ? nullptr : Combat->FindEntity(LastTargetId))
	{
		LastTargetHp = E->bDead ? 0u : E->Hp;
	}
}

void FBotObservations::BindPhase1(UNetClientSubsystem* Net)
{
	TWeakObjectPtr<UNetClientSubsystem> Weak(Net);
	auto KindOf = [Weak](const FString& Id) -> uint32
	{
		const UNetClientSubsystem* N = Weak.Get();
		const FEntitySpawn* S = N ? N->GetKnownEntities().Find(Id) : nullptr;
		if (!S) S = N ? N->GetKnownEntities().Find(Id.ToLower()) : nullptr;
		return S ? S->Kind : 0u;
	};
	const FDelegateHandle Ack = Net->OnIntentAck.AddLambda([this](const FAck& A)
	{
		++Acks;
		NewestTick = FMath::Max(NewestTick, A.Tick);
	});
	const FDelegateHandle Hit = Net->OnAttackResult.AddLambda([this, Weak, KindOf](const FAttackResult& R)
	{
		const UNetClientSubsystem* N = Weak.Get();
		if (!N || R.Outcome == ENetAttackOutcome::Unspecified) return;
		NewestTick = FMath::Max(NewestTick, R.Tick);
		AttackResultKeys.Add(FString::Printf(TEXT("%llu|%s|%s"), R.Tick, *R.Attacker.ToLower(), *R.Target.ToLower()));
		if (R.Outcome != ENetAttackOutcome::Miss)
		{
			FLastHit& Last = LastHitOn.FindOrAdd(R.Target.ToLower());
			Last.HpAfter = R.TargetHpAfter;
			Last.Damage = R.Damage;
			++Last.Count;
		}
		const bool bOwnTarget = N->IsOwnEntity(R.Target);
		if (KindOf(R.Attacker) == 2 && (bOwnTarget || KindOf(R.Target) == 1))
		{
			NpcsAttackingPlayers.Add(R.Attacker.ToLower());
			if (bOwnTarget) NpcsAttackingOwn.Add(R.Attacker.ToLower());
		}
		// A wounded NPC that entered view: its next hit must continue from the HP its spawn carried.
		if (FLateSpawn* Late = WoundedSpawns.Find(R.Target.ToLower()); Late && !Late->bChecked
			&& (R.TargetIncarnation == 0 || Late->Incarnation == 0 || R.TargetIncarnation == Late->Incarnation))
		{
			Late->bChecked = true;
			const uint32 Expected = Late->Hp > R.Damage ? Late->Hp - R.Damage : 0u;
			(R.TargetHpAfter == Expected ? LateSpawnHpOk : LateSpawnHpBad)++;
			if (R.TargetHpAfter != Expected)
			{
				UE_LOG(LogNightfallBot, Display, TEXT("bot: late spawn HP mismatch on %s: spawn hp %u, damage %u, hp after %u"), *R.Target, Late->Hp, R.Damage, R.TargetHpAfter);
			}
		}
	});
	const FDelegateHandle Spawn = Net->OnEntitySpawn.AddLambda([this, Weak](const FEntitySpawn& S)
	{
		const UNetClientSubsystem* N = Weak.Get();
		if (!N || N->IsOwnEntity(S.EntityId) || S.Kind != 2 || !S.bCombatant) return;
		const bool bWounded = !S.bDead && S.Hp < S.MaxHp;
		if (bWounded || S.bDead || S.LifeIncarnation > 1) ++SpawnsMidFight;
		if (bWounded) WoundedSpawns.Add(S.EntityId.ToLower(), FLateSpawn{ S.Hp, S.LifeIncarnation, false });
	});
	const FDelegateHandle Move = Net->OnEntityMove.AddLambda([this](const FEntityMove& M)
	{
		NewestTick = FMath::Max(NewestTick, M.Tick);
		if (M.Destination.X != 0.f || M.Destination.Y != 0.f) MovingEntities.Add(M.EntityId.ToLower());
		else MovingEntities.Remove(M.EntityId.ToLower());
	});
	const FDelegateHandle Died = Net->OnEntityDied.AddLambda([this](const FEntityDied& D) { NewestTick = FMath::Max(NewestTick, D.Tick); });
	const FDelegateHandle Respawned = Net->OnEntityRespawned.AddLambda([this, Weak](const FEntityRespawned& R)
	{
		NewestTick = FMath::Max(NewestTick, R.Tick);
		const UNetClientSubsystem* N = Weak.Get();
		if (!N || !N->IsOwnEntity(R.Entity)) return;
		bOwnRespawned = true;
		OwnRespawnTick = R.Tick;
		bAttackedSinceRespawn = false;
		NpcsAttackingOwn.Reset();
	});
	const FDelegateHandle Stats = Net->OnStatsChanged.AddLambda([this, Weak](const FStatsChanged& S)
	{
		const UNetClientSubsystem* N = Weak.Get();
		if (!N || !N->IsOwnEntity(S.Entity)) return;
		if (S.Hp == 0)
		{
			// Dead: the first HP-0 StatsChanged may still carry the old total (the loss follows in
			// a later one), so "before" is the total before the first and "after" the newest.
			if (!bOwnDeathStatsSeen)
			{
				bOwnDeathStatsSeen = true;
				XpBeforeDeath = TrackedXp;
				LevelBeforeDeath = TrackedLevel;
			}
			XpAfterDeath = S.Xp;
		}
		else
		{
			bOwnDeathStatsSeen = false;
		}
		TrackedXp = S.Xp;
		TrackedLevel = S.Level;
		bAwaitingStats = false;
	});
	const FDelegateHandle Xp = Net->OnXpGained.AddLambda([this, Weak](const FXpGained& G)
	{
		const UNetClientSubsystem* N = Weak.Get();
		if (!N || !N->IsOwnEntity(G.Entity)) return;
		TrackedXp = G.Total;
		XpGainedSinceReconnect += G.Amount;
		bAwaitingStats = false;
	});
	Phase1Unbinders.Add([Weak, Ack, Hit, Spawn, Move, Died, Respawned, Stats, Xp]
	{
		if (UNetClientSubsystem* N = Weak.Get())
		{
			N->OnIntentAck.Remove(Ack);
			N->OnAttackResult.Remove(Hit);
			N->OnEntitySpawn.Remove(Spawn);
			N->OnEntityMove.Remove(Move);
			N->OnEntityDied.Remove(Died);
			N->OnEntityRespawned.Remove(Respawned);
			N->OnStatsChanged.Remove(Stats);
			N->OnXpGained.Remove(Xp);
		}
	});
}

void FBotObservations::ObservePhase1(const UCombatStateSubsystem* Combat)
{
	// OnConnected / OnDisconnected are dynamic delegates (UFUNCTION only), so the socket is polled.
	const UNetClientSubsystem* Net = BoundNet.Get();
	const bool bConnected = Net && Net->IsConnected();
	if (bConnected && !bWasConnected) ++Connects;
	if (!bConnected && bWasConnected)
	{
		XpAtDisconnect = TrackedXp;
		TrackedXp.Reset();
		XpGainedSinceReconnect = 0;
		bAwaitingStats = true;
		bXpUnknownSeenAfterReconnect = false;
		NpcsAttackingOwn.Reset();   // the server despawns a dropped player: the reconnect is a new life on the wire
	}
	bWasConnected = bConnected;
	if (bAwaitingStats)
	{
		if (Combat->GetOwn().bXpKnown) ++XpKnownBeforeStats;
		else bXpUnknownSeenAfterReconnect = true;
	}
	if (bOwnRespawned && Combat->GetAttackState() == EAttackState::Active) bAttackedSinceRespawn = true;
	// A wounded spawn must show up in the projection as it arrived (HP and life) until it is hit.
	for (TPair<FString, FLateSpawn>& Late : WoundedSpawns)
	{
		const FCombatEntity* E = Combat->FindEntity(Late.Key);
		if (Late.Value.bProjectionChecked || Late.Value.bChecked || !E) continue;
		Late.Value.bProjectionChecked = true;
		const bool bOk = !E->bDead && E->Hp == Late.Value.Hp && (Late.Value.Incarnation == 0 || E->Incarnation == Late.Value.Incarnation);
		(bOk ? LateSpawnProjectionOk : LateSpawnProjectionBad)++;
		if (!bOk)
		{
			UE_LOG(LogNightfallBot, Display, TEXT("bot: late spawn %s: spawn hp %u life %u, projection hp %u life %u"), *Late.Key, Late.Value.Hp, Late.Value.Incarnation, E->Hp, E->Incarnation);
		}
	}
}

// --- Context -----------------------------------------------------------------------------------

UNetClientSubsystem* FBotContext::Net() const { return GameInstance ? GameInstance->GetSubsystem<UNetClientSubsystem>() : nullptr; }
UCombatStateSubsystem* FBotContext::Combat() const { return GameInstance ? GameInstance->GetSubsystem<UCombatStateSubsystem>() : nullptr; }
UAuthSubsystem* FBotContext::Auth() const { return GameInstance ? GameInstance->GetSubsystem<UAuthSubsystem>() : nullptr; }
UWorld* FBotContext::World() const { return GameInstance ? GameInstance->GetWorld() : nullptr; }

// --- Helpers -----------------------------------------------------------------------------------

namespace BotPredicates
{
	int32 ParseRejectReason(const FString& Text)
	{
		if (Text.IsNumeric()) return FCString::Atoi(*Text);
		const UEnum* Enum = StaticEnum<EGrpcNightfallV1RejectReason>();
		const FString Upper = Text.ToUpper();
		const FString Full = Upper.StartsWith(TEXT("REJECT_REASON_")) ? Upper : TEXT("REJECT_REASON_") + Upper;
		const int64 Value = Enum->GetValueByNameString(Full);
		return Value == INDEX_NONE ? -1 : static_cast<int32>(Value);
	}

	FString RejectReasonName(uint32 Reason)
	{
		FString Name = StaticEnum<EGrpcNightfallV1RejectReason>()->GetNameStringByValue(Reason);
		if (Name.IsEmpty()) return FString::Printf(TEXT("%u"), Reason);
		Name.RemoveFromStart(TEXT("REJECT_REASON_"));
		return Name;
	}

	bool OwnPosition(const FBotContext& Context, FVector2D& OutTiles)
	{
		UNetClientSubsystem* Net = Context.Net();
		FNetVec2 Pos;
		// Sampling far in the future holds the newest server sample (the buffer never extrapolates).
		if (!Net || Net->GetOwnEntityId().IsEmpty() || !Net->Snapshots().Sample(Net->GetOwnEntityId(), TNumericLimits<int64>::Max() / 2, Pos)) return false;
		OutTiles = FVector2D(Pos.X, Pos.Y);
		return true;
	}

	FString NearestAttackable(const FBotContext& Context)
	{
		UNetClientSubsystem* Net = Context.Net();
		UCombatStateSubsystem* Combat = Context.Combat();
		if (!Net || !Combat) return FString();
		FVector2D Own(0.0, 0.0);
		OwnPosition(Context, Own);
		FString Best;
		double BestDistance = TNumericLimits<double>::Max();
		for (const TPair<FString, FEntitySpawn>& Known : Net->GetKnownEntities())
		{
			if (Net->IsOwnEntity(Known.Key) || !Combat->IsAttackable(Known.Key)) continue;
			FNetVec2 Pos = Known.Value.Position;
			Net->Snapshots().Sample(Known.Key, TNumericLimits<int64>::Max() / 2, Pos);
			const double Distance = FVector2D::Distance(Own, FVector2D(Pos.X, Pos.Y));
			if (Distance < BestDistance) { BestDistance = Distance; Best = Known.Key; }
		}
		return Best;
	}

	bool EntityPosition(const FBotContext& Context, const FString& EntityId, FVector2D& OutTiles)
	{
		UNetClientSubsystem* Net = Context.Net();
		if (!Net || EntityId.IsEmpty()) return false;
		const FEntitySpawn* Known = Net->GetKnownEntities().Find(EntityId);
		if (!Known) Known = Net->GetKnownEntities().Find(EntityId.ToLower());
		FNetVec2 Pos;
		if (Net->Snapshots().Sample(Known ? Known->EntityId : EntityId, TNumericLimits<int64>::Max() / 2, Pos))
		{
			OutTiles = FVector2D(Pos.X, Pos.Y);
			return true;
		}
		if (!Known) return false;
		OutTiles = FVector2D(Known->Position.X, Known->Position.Y);
		return true;
	}

	FString NearestAttacker(const FBotContext& Context)
	{
		const UCombatStateSubsystem* Combat = Context.Combat();
		FVector2D Own(0.0, 0.0), Pos;
		if (!Combat || !Context.Observations || !OwnPosition(Context, Own)) return FString();
		FString Best;
		double BestDistance = TNumericLimits<double>::Max();
		for (const FString& Id : Context.Observations->NpcsAttackingOwn)
		{
			const FCombatEntity* E = Combat->FindEntity(Id);
			if (!E || E->bDead || !EntityPosition(Context, E->Spawn.EntityId, Pos)) continue;
			const double Distance = FVector2D::Distance(Own, Pos);
			if (Distance < BestDistance) { BestDistance = Distance; Best = Id; }
		}
		return Best;
	}

	FString CurrentOrLastTarget(const FBotContext& Context)
	{
		const UCombatStateSubsystem* Combat = Context.Combat();
		if (Combat && !Combat->GetTargetId().IsEmpty()) return Combat->GetTargetId();
		return Context.Observations ? Context.Observations->LastTargetId : FString();
	}

	bool ParseNumber(const FString& Text, double& Out)
	{
		if (!Text.IsNumeric()) return false;   // optional sign, digits, one decimal point
		Out = FCString::Atod(*Text);
		return true;
	}

	FString Describe(double Value)
	{
		return FMath::IsNearlyEqual(Value, FMath::RoundToDouble(Value)) ? FString::Printf(TEXT("%.0f"), Value) : FString::Printf(TEXT("%.2f"), Value);
	}

	FString AttackStateName(EAttackState State)
	{
		switch (State)
		{
		case EAttackState::Pending: return TEXT("pending");
		case EAttackState::Active: return TEXT("active");
		default: return TEXT("idle");
		}
	}
}

// --- Registry ----------------------------------------------------------------------------------

FBotPredicateRegistry& FBotPredicateRegistry::Get()
{
	static FBotPredicateRegistry Registry = []
	{
		FBotPredicateRegistry R;
		R.RegisterBuiltins();
		return R;
	}();
	return Registry;
}

TArray<FString> FBotPredicateRegistry::Tokenize(const FString& Text)
{
	TArray<FString> Tokens;
	Text.ParseIntoArrayWS(Tokens);
	return Tokens;
}

void FBotPredicateRegistry::Register(FBotPredicateDef Def)
{
	Defs.RemoveAll([&](const FBotPredicateDef& D) { return D.Name == Def.Name; });
	Defs.Add(MoveTemp(Def));
}

const FBotPredicateDef* FBotPredicateRegistry::Find(const FString& Name) const
{
	return Defs.FindByPredicate([&](const FBotPredicateDef& D) { return D.Name.Equals(Name, ESearchCase::IgnoreCase); });
}

FBotPredicateFn FBotPredicateRegistry::Parse(const TArray<FString>& Tokens, FString& OutError) const
{
	if (Tokens.IsEmpty())
	{
		OutError = TEXT("missing predicate");
		return nullptr;
	}
	const FBotPredicateDef* Def = Find(Tokens[0]);
	if (!Def)
	{
		OutError = FString::Printf(TEXT("unknown predicate '%s'"), *Tokens[0]);
		return nullptr;
	}
	TArray<FString> Args(Tokens);
	Args.RemoveAt(0);
	FBotPredicateFn Fn = Def->Bind(Args, OutError);
	if (!Fn && !OutError.IsEmpty()) OutError = FString::Printf(TEXT("%s (usage: %s)"), *OutError, *Def->Usage);
	return Fn;
}

void FBotPredicateRegistry::RegisterFlag(const FString& Name, const FString& Description, TFunction<bool(const FBotContext&)> Holds)
{
	Register({ Name, Name, Description, [Holds](const TArray<FString>& Args, FString& OutError) -> FBotPredicateFn
	{
		if (!Args.IsEmpty()) { OutError = TEXT("takes no arguments"); return nullptr; }
		return [Holds](const FBotContext& C) { const bool b = Holds(C); return FBotPredicateValue{ b, b ? TEXT("true") : TEXT("false") }; };
	} });
}

void FBotPredicateRegistry::RegisterNumber(const FString& Name, const FString& Description, TFunction<TOptional<double>(const FBotContext&)> Value)
{
	Register({ Name, Name + TEXT(" <op> <n>  (op: < <= == != >= >)"), Description, [Value](const TArray<FString>& Args, FString& OutError) -> FBotPredicateFn
	{
		static const TCHAR* const Ops[] = { TEXT("<"), TEXT("<="), TEXT("=="), TEXT("!="), TEXT(">="), TEXT(">") };
		double Rhs = 0.0;
		if (Args.Num() != 2) { OutError = TEXT("expects an operator and a number"); return nullptr; }
		int32 Op = INDEX_NONE;
		for (int32 I = 0; I < UE_ARRAY_COUNT(Ops); ++I) if (Args[0] == Ops[I]) Op = I;
		if (Op == INDEX_NONE) { OutError = FString::Printf(TEXT("bad operator '%s'"), *Args[0]); return nullptr; }
		if (!BotPredicates::ParseNumber(Args[1], Rhs)) { OutError = FString::Printf(TEXT("'%s' is not a number"), *Args[1]); return nullptr; }
		return [Value, Op, Rhs](const FBotContext& C) -> FBotPredicateValue
		{
			const TOptional<double> Lhs = Value(C);
			if (!Lhs.IsSet()) return { false, TEXT("unknown") };
			const double L = Lhs.GetValue();
			bool b = false;
			switch (Op)
			{
			case 0: b = L < Rhs; break;
			case 1: b = L <= Rhs; break;
			case 2: b = FMath::IsNearlyEqual(L, Rhs); break;
			case 3: b = !FMath::IsNearlyEqual(L, Rhs); break;
			case 4: b = L >= Rhs; break;
			default: b = L > Rhs; break;
			}
			return { b, BotPredicates::Describe(L) };
		};
	} });
}

void FBotPredicateRegistry::RegisterEquality(const FString& Name, const FString& Usage, const FString& Description,
	TFunction<bool(const FString& Value, FString& OutError)> Validate,
	TFunction<bool(const FBotContext&, const FString& Value, FString& OutObserved)> Matches)
{
	Register({ Name, Usage, Description, [Validate, Matches](const TArray<FString>& Args, FString& OutError) -> FBotPredicateFn
	{
		if (Args.Num() != 2 || (Args[0] != TEXT("==") && Args[0] != TEXT("!="))) { OutError = TEXT("expects == or != and a value"); return nullptr; }
		if (Validate && !Validate(Args[1], OutError)) return nullptr;
		const bool bWantEqual = Args[0] == TEXT("==");
		const FString Rhs = Args[1];
		return [Matches, bWantEqual, Rhs](const FBotContext& C) -> FBotPredicateValue
		{
			FString Observed;
			const bool bEqual = Matches(C, Rhs, Observed);
			return { bEqual == bWantEqual, Observed };
		};
	} });
}

void FBotPredicateRegistry::RegisterBuiltins()
{
	using namespace BotPredicates;

	// --- Phase 0b: login, world, movement -------------------------------------------------------
	RegisterFlag(TEXT("connected"), TEXT("Logged in: the API's access token is held, so gRPC calls are authenticated (nf.Login done)"),
		[](const FBotContext& C) { const UAuthSubsystem* A = C.Auth(); return A && A->IsLoggedIn(); });
	RegisterFlag(TEXT("ws_connected"), TEXT("The real-time WebSocket is open"),
		[](const FBotContext& C) { const UNetClientSubsystem* N = C.Net(); return N && N->IsConnected(); });
	RegisterFlag(TEXT("in_world"), TEXT("WebSocket open, own EntitySpawn received and the world map (ANightfallGameMode) loaded"),
		[](const FBotContext& C)
		{
			const UNetClientSubsystem* N = C.Net();
			const UWorld* W = C.World();
			return N && N->IsConnected() && !N->GetOwnEntityId().IsEmpty() && N->GetKnownEntities().Contains(N->GetOwnEntityId())
				&& W && Cast<ANightfallGameMode>(W->GetAuthGameMode()) != nullptr;
		});
	Register({ TEXT("own_at"), TEXT("own_at <x> <y> <tol>"), TEXT("The newest server position of the own entity is within <tol> tiles of (x, y)"),
		[](const TArray<FString>& Args, FString& OutError) -> FBotPredicateFn
		{
			double X = 0, Y = 0, Tol = 0;
			if (Args.Num() != 3 || !ParseNumber(Args[0], X) || !ParseNumber(Args[1], Y) || !ParseNumber(Args[2], Tol)) { OutError = TEXT("expects three numbers"); return nullptr; }
			return [X, Y, Tol](const FBotContext& C) -> FBotPredicateValue
			{
				FVector2D Pos;
				if (!OwnPosition(C, Pos)) return { false, TEXT("unknown") };
				return { FVector2D::Distance(Pos, FVector2D(X, Y)) <= Tol, FString::Printf(TEXT("(%.2f, %.2f)"), Pos.X, Pos.Y) };
			};
		} });
	RegisterNumber(TEXT("proxies"), TEXT("Entities in view other than the player's own (net cache; what UWorldProxySubsystem spawns proxies for)"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UNetClientSubsystem* N = C.Net();
			if (!N) return {};
			int32 Count = 0;
			for (const TPair<FString, FEntitySpawn>& Known : N->GetKnownEntities()) Count += !N->IsOwnEntity(Known.Key);
			return static_cast<double>(Count);
		});
	RegisterNumber(TEXT("last_ack_seq"), TEXT("The highest intent seq the server Acked"),
		[](const FBotContext& C) -> TOptional<double> { return C.Observations ? TOptional<double>(C.Observations->LastAckSeq) : TOptional<double>(); });
	RegisterEquality(TEXT("rejected"), TEXT("rejected == <reason|none>"), TEXT("The newest IntentRejected reason (TOO_FAR, UNKNOWN_ENTITY, ... or the number); none = no rejection seen"),
		[](const FString& Value, FString& OutError)
		{
			if (Value.Equals(TEXT("none"), ESearchCase::IgnoreCase) || ParseRejectReason(Value) >= 0) return true;
			OutError = FString::Printf(TEXT("unknown reject reason '%s'"), *Value);
			return false;
		},
		[](const FBotContext& C, const FString& Value, FString& OutObserved)
		{
			const FBotObservations* O = C.Observations;
			const bool bAny = O && O->bRejected;
			OutObserved = bAny ? RejectReasonName(O->LastRejectReason) : TEXT("none");
			if (Value.Equals(TEXT("none"), ESearchCase::IgnoreCase)) return !bAny;
			return bAny && static_cast<int32>(O->LastRejectReason) == ParseRejectReason(Value);
		});

	// --- Phase 1a E2: reconnect, two clients, replacement ---------------------------------------
	RegisterNumber(TEXT("own_spawns"), TEXT("EntitySpawns of the own entity received (one per admission; a reconnect adds one)"),
		[](const FBotContext& C) -> TOptional<double> { return C.Observations ? TOptional<double>(C.Observations->OwnSpawns) : TOptional<double>(); });
	RegisterFlag(TEXT("own_entity_unchanged"), TEXT("The own entity id equals the one of the first own spawn"),
		[](const FBotContext& C)
		{
			const UNetClientSubsystem* N = C.Net();
			return N && C.Observations && !C.Observations->FirstOwnEntityId.IsEmpty() && N->IsOwnEntity(C.Observations->FirstOwnEntityId);
		});
	RegisterFlag(TEXT("ticket_refreshed"), TEXT("At least two play tickets were presented and the newest differs from the one before (a reconnect fetched a fresh ticket)"),
		[](const FBotContext& C) { const UNetClientSubsystem* N = C.Net(); return N && N->IsNewestTicketFresh(); });
	RegisterFlag(TEXT("ws_closed"), TEXT("The WebSocket is closed and the client is not connected (after a 4409 it stays closed: no reconnect)"),
		[](const FBotContext& C) { const UNetClientSubsystem* N = C.Net(); return N && !N->IsConnected(); });
	RegisterNumber(TEXT("proxy_actors"), TEXT("Live proxy actors in the world (UWorldProxySubsystem): a despawn that leaks an actor leaves this above proxies"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UWorld* W = C.World();
			const UWorldProxySubsystem* P = W ? W->GetSubsystem<UWorldProxySubsystem>() : nullptr;
			return P ? TOptional<double>(P->GetProxies().Num()) : TOptional<double>();
		});
	RegisterNumber(TEXT("other_players"), TEXT("Other players in view (net cache; NPCs excluded, unlike proxies)"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UNetClientSubsystem* N = C.Net();
			if (!N) return {};
			int32 Count = 0;
			for (const TPair<FString, FEntitySpawn>& Known : N->GetKnownEntities()) Count += !N->IsOwnEntity(Known.Key) && Known.Value.Kind == 1;
			return static_cast<double>(Count);
		});
	RegisterFlag(TEXT("proxy_actors_in_sync"), TEXT("The world's proxy actors are exactly the entities in view other than the player (none leaked, none missing)"),
		[](const FBotContext& C)
		{
			const UNetClientSubsystem* N = C.Net();
			const UWorld* W = C.World();
			const UWorldProxySubsystem* P = W ? W->GetSubsystem<UWorldProxySubsystem>() : nullptr;
			if (!N || !P) return false;
			int32 Expected = 0;
			for (const TPair<FString, FEntitySpawn>& Known : N->GetKnownEntities())
			{
				if (N->IsOwnEntity(Known.Key)) continue;
				++Expected;
				if (!P->GetProxies().Contains(Known.Key)) return false;
			}
			return P->GetProxies().Num() == Expected;
		});
	RegisterNumber(TEXT("close_code"), TEXT("The WebSocket close code of the newest close (4409 = replaced by a newer session); unknown before the first close"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UNetClientSubsystem* N = C.Net();
			return N && N->GetLastCloseCode() != 0 ? TOptional<double>(N->GetLastCloseCode()) : TOptional<double>();
		});
	Register({ TEXT("proxy_at"), TEXT("proxy_at <x> <y> <tol>"), TEXT("Some entity other than the player is within <tol> tiles of (x, y) (newest server sample)"),
		[](const TArray<FString>& Args, FString& OutError) -> FBotPredicateFn
		{
			double X = 0, Y = 0, Tol = 0;
			if (Args.Num() != 3 || !ParseNumber(Args[0], X) || !ParseNumber(Args[1], Y) || !ParseNumber(Args[2], Tol)) { OutError = TEXT("expects three numbers"); return nullptr; }
			return [X, Y, Tol](const FBotContext& C) -> FBotPredicateValue
			{
				UNetClientSubsystem* N = C.Net();
				if (!N) return { false, TEXT("no net") };
				double Best = TNumericLimits<double>::Max();
				FString Seen = TEXT("no proxies");
				for (const TPair<FString, FEntitySpawn>& Known : N->GetKnownEntities())
				{
					if (N->IsOwnEntity(Known.Key)) continue;
					FNetVec2 Pos = Known.Value.Position;
					N->Snapshots().Sample(Known.Key, TNumericLimits<int64>::Max() / 2, Pos);
					const double D = FVector2D::Distance(FVector2D(Pos.X, Pos.Y), FVector2D(X, Y));
					if (D < Best) { Best = D; Seen = FString::Printf(TEXT("%s (%.2f, %.2f)"), *Known.Key, Pos.X, Pos.Y); }
				}
				return { Best <= Tol, Seen };
			};
		} });

	// --- Phase 1: combat projection (UCombatStateSubsystem) -------------------------------------
	RegisterEquality(TEXT("target"), TEXT("target == <name|id|none>"), TEXT("The confirmed selection (TargetChanged), by entity name or id; none = no selection"),
		nullptr,
		[](const FBotContext& C, const FString& Value, FString& OutObserved)
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			const FString Id = Combat ? Combat->GetTargetId() : FString();
			const FCombatEntity* E = Combat && !Id.IsEmpty() ? Combat->FindEntity(Id) : nullptr;
			OutObserved = Id.IsEmpty() ? FString(TEXT("none")) : E ? FString::Printf(TEXT("%s (%s)"), *E->Spawn.Name, *Id) : Id;
			if (Value.Equals(TEXT("none"), ESearchCase::IgnoreCase)) return Id.IsEmpty();
			return !Id.IsEmpty() && (Id.Equals(Value, ESearchCase::IgnoreCase) || (E && E->Spawn.Name.Equals(Value, ESearchCase::IgnoreCase)));
		});
	RegisterNumber(TEXT("target_hp"), TEXT("HP of the selection; after it clears, of the last selection (0 once it died, also after its corpse despawned)"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			if (!Combat) return {};
			if (const FCombatEntity* E = Combat->GetTargetId().IsEmpty() ? nullptr : Combat->FindEntity(Combat->GetTargetId()))
			{
				return static_cast<double>(E->bDead ? 0u : E->Hp);
			}
			return C.Observations && C.Observations->LastTargetHp.IsSet() ? TOptional<double>(C.Observations->LastTargetHp.GetValue()) : TOptional<double>();
		});
	RegisterNumber(TEXT("own_hp"), TEXT("The player's HP"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			const FCombatEntity* O = Combat ? Combat->FindOwnEntity() : nullptr;
			return O && O->bCombatant ? TOptional<double>(O->Hp) : TOptional<double>();
		});
	RegisterNumber(TEXT("own_level"), TEXT("The player's level"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			const FCombatEntity* O = Combat ? Combat->FindOwnEntity() : nullptr;
			return O && O->bCombatant ? TOptional<double>(O->Level) : TOptional<double>();
		});
	RegisterNumber(TEXT("own_xp"), TEXT("The player's XP total (unknown until StatsChanged / XpGained)"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			return Combat && Combat->GetOwn().bXpKnown ? TOptional<double>(static_cast<double>(Combat->GetOwn().Xp)) : TOptional<double>();
		});
	RegisterFlag(TEXT("own_dead"), TEXT("The player is dead (the HUD shows the dead overlay)"),
		[](const FBotContext& C) { const UCombatStateSubsystem* Combat = C.Combat(); return Combat && Combat->BuildHudModel().bDeadOverlay; });
	RegisterFlag(TEXT("xp_known"), TEXT("The XP total is known (the HUD shows a number, not XP --)"),
		[](const FBotContext& C) { const UCombatStateSubsystem* Combat = C.Combat(); return Combat && Combat->GetOwn().bXpKnown; });
	RegisterEquality(TEXT("attack_state"), TEXT("attack_state == <idle|pending|active>"), TEXT("The HUD attack state: pending = Attack sent and not yet Acked"),
		[](const FString& Value, FString& OutError)
		{
			if (Value == TEXT("idle") || Value == TEXT("pending") || Value == TEXT("active")) return true;
			OutError = FString::Printf(TEXT("'%s' is not idle, pending or active"), *Value);
			return false;
		},
		[](const FBotContext& C, const FString& Value, FString& OutObserved)
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			OutObserved = AttackStateName(Combat ? Combat->BuildHudModel().AttackState : EAttackState::Idle);
			return OutObserved == Value;
		});
	RegisterNumber(TEXT("damage_numbers"), TEXT("Floating damage numbers shown so far (one per distinct AttackResult)"),
		[](const FBotContext& C) -> TOptional<double> { return C.Observations ? TOptional<double>(C.Observations->DamageNumbers) : TOptional<double>(); });

	// --- Phase 1a E3: combat scenarios (1-target-attack ... 1-social-aggro) ---------------------
	// Values that come from packages/data (XP table, death penalty, town respawn) are read through
	// FBotFixtureData, so a changed table row changes what these predicates expect.
	auto Own = [](const FBotContext& C) -> const FCombatEntity*
	{
		const UCombatStateSubsystem* Combat = C.Combat();
		const FCombatEntity* O = Combat ? Combat->FindOwnEntity() : nullptr;
		return O && O->bCombatant ? O : nullptr;
	};
	auto Fixture = [](FString& OutObserved) -> const FBotFixtureData*
	{
		const FBotFixtureData& D = FBotFixtureData::Get();
		if (D.IsValid()) return &D;
		OutObserved = D.Error;
		return nullptr;
	};
	auto Count = [](TFunction<int32(const FBotObservations&)> Of)
	{
		return [Of](const FBotContext& C) -> TOptional<double> { return C.Observations ? TOptional<double>(Of(*C.Observations)) : TOptional<double>(); };
	};
	auto Custom = [this](const FString& Name, const FString& Description, TFunction<FBotPredicateValue(const FBotContext&)> Eval)
	{
		Register({ Name, Name, Description, [Eval](const TArray<FString>& Args, FString& OutError) -> FBotPredicateFn
		{
			if (!Args.IsEmpty()) { OutError = TEXT("takes no arguments"); return nullptr; }
			return Eval;
		} });
	};

	RegisterFlag(TEXT("hud_target_visible"), TEXT("The HUD target frame shows the selection"),
		[](const FBotContext& C) { const UCombatStateSubsystem* Combat = C.Combat(); return Combat && Combat->BuildHudModel().bTargetVisible; });
	RegisterNumber(TEXT("acks"), TEXT("Acks received so far (one per accepted intent; keep-alives are refused, never Acked)"),
		Count([](const FBotObservations& O) { return O.Acks; }));
	RegisterNumber(TEXT("own_mp"), TEXT("The player's MP (owner-only StatsChanged)"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			return Combat && Combat->GetOwn().bMpKnown ? TOptional<double>(Combat->GetOwn().Mp) : TOptional<double>();
		});
	Custom(TEXT("own_hp_is_respawn_hp"), TEXT("Own HP == max(1, floor(maxHP * restore_hp_q / 1e6)) from tables/formulas.toml [formulas.town_respawn] (65 %)"),
		[Own, Fixture](const FBotContext& C) -> FBotPredicateValue
		{
			FString Observed;
			const FBotFixtureData* D = Fixture(Observed);
			const FCombatEntity* O = Own(C);
			if (!D || !O) return { false, Observed.IsEmpty() ? FString(TEXT("unknown")) : Observed };
			const uint32 Expected = D->RespawnHp(O->MaxHp);
			return { O->Hp == Expected, FString::Printf(TEXT("%u / %u (respawn HP %u)"), O->Hp, O->MaxHp, Expected) };
		});
	Custom(TEXT("own_mp_is_respawn_mp"), TEXT("Own MP == floor(maxMP * restore_mp_q / 1e6) from tables/formulas.toml (0)"),
		[Fixture](const FBotContext& C) -> FBotPredicateValue
		{
			FString Observed;
			const FBotFixtureData* D = Fixture(Observed);
			const UCombatStateSubsystem* Combat = C.Combat();
			if (!D || !Combat || !Combat->GetOwn().bMpKnown) return { false, Observed.IsEmpty() ? FString(TEXT("unknown")) : Observed };
			const FOwnCombatState& O = Combat->GetOwn();
			const uint32 Expected = D->RespawnMp(O.MaxMp);
			return { O.Mp == Expected, FString::Printf(TEXT("%u / %u (respawn MP %u)"), O.Mp, O.MaxMp, Expected) };
		});
	Custom(TEXT("protection_active"), TEXT("Respawn protection holds by the server's rule (world.proto RespawnRequest): an EntityRespawned for the player, no Attack accepted since, and fewer than spawn_protection_seconds * 10 ticks elapsed (newest tick seen). Inferred: the wire carries no protection flag"),
		[Fixture](const FBotContext& C) -> FBotPredicateValue
		{
			FString Observed;
			const FBotFixtureData* D = Fixture(Observed);
			const FBotObservations* O = C.Observations;
			const UCombatStateSubsystem* Combat = C.Combat();
			if (!D || !O || !Combat) return { false, Observed.IsEmpty() ? FString(TEXT("unknown")) : Observed };
			if (!O->bOwnRespawned) return { false, TEXT("not respawned") };
			if (O->bAttackedSinceRespawn) return { false, TEXT("ended by an accepted Attack") };
			const uint64 Until = O->OwnRespawnTick + static_cast<uint64>(D->SpawnProtectionSeconds) * 10;
			const bool b = O->NewestTick < Until && !Combat->IsOwnDead();
			return { b, FString::Printf(TEXT("respawned tick %llu, until %llu, newest %llu"), O->OwnRespawnTick, Until, O->NewestTick) };
		});
	Custom(TEXT("own_level_matches_xp"), TEXT("Own level == the highest level whose tables/experience.toml threshold the XP total reaches"),
		[Own, Fixture](const FBotContext& C) -> FBotPredicateValue
		{
			FString Observed;
			const FBotFixtureData* D = Fixture(Observed);
			const UCombatStateSubsystem* Combat = C.Combat();
			const FCombatEntity* O = Own(C);
			if (!D || !O || !Combat->GetOwn().bXpKnown) return { false, Observed.IsEmpty() ? FString(TEXT("unknown")) : Observed };
			const uint32 Expected = D->LevelForXp(Combat->GetOwn().Xp);
			return { O->Level == Expected, FString::Printf(TEXT("level %u at XP %llu (table: level %u)"), O->Level, Combat->GetOwn().Xp, Expected) };
		});
	RegisterNumber(TEXT("death_xp_loss"), TEXT("XP the newest death took: total before it minus the total its StatsChanged carried"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const FBotObservations* O = C.Observations;
			if (!O || !O->XpBeforeDeath.IsSet() || !O->XpAfterDeath.IsSet()) return {};
			return static_cast<double>(O->XpBeforeDeath.GetValue()) - static_cast<double>(O->XpAfterDeath.GetValue());
		});
	Custom(TEXT("death_xp_loss_matches_table"), TEXT("The newest death took min(XP, round((X[L+1] - X[L]) * fraction_q[L] / 1e6)) at the level L it died on (tables/penalties.toml, experience.toml)"),
		[Fixture](const FBotContext& C) -> FBotPredicateValue
		{
			FString Observed;
			const FBotFixtureData* D = Fixture(Observed);
			const FBotObservations* O = C.Observations;
			if (!D || !O || !O->XpBeforeDeath.IsSet() || !O->XpAfterDeath.IsSet()) return { false, Observed.IsEmpty() ? FString(TEXT("no death seen")) : Observed };
			const uint64 Before = O->XpBeforeDeath.GetValue(), After = O->XpAfterDeath.GetValue();
			const TOptional<uint64> Row = D->DeathXpLoss(O->LevelBeforeDeath);
			if (!Row.IsSet()) return { false, FString::Printf(TEXT("no table row for level %u"), O->LevelBeforeDeath) };
			const uint64 Expected = FMath::Min(Row.GetValue(), Before);
			return { After <= Before && Before - After == Expected,
				FString::Printf(TEXT("XP %llu -> %llu at level %u (table loss %llu)"), Before, After, O->LevelBeforeDeath, Expected) };
		});
	RegisterNumber(TEXT("attack_results"), TEXT("Distinct AttackResults received (tick + attacker + target), whoever was involved"),
		Count([](const FBotObservations& O) { return O.AttackResultKeys.Num(); }));
	Custom(TEXT("damage_numbers_match_results"), TEXT("Exactly one floating damage number was shown per distinct AttackResult received"),
		[](const FBotContext& C) -> FBotPredicateValue
		{
			const FBotObservations* O = C.Observations;
			if (!O) return { false, TEXT("unknown") };
			return { O->DamageNumbers == O->AttackResultKeys.Num(), FString::Printf(TEXT("%d numbers, %d results"), O->DamageNumbers, O->AttackResultKeys.Num()) };
		});
	RegisterNumber(TEXT("target_distance"), TEXT("Tiles from the player to the selection (after it clears, the last selection), newest server positions"),
		[](const FBotContext& C) -> TOptional<double>
		{
			FVector2D OwnPos, TargetPos;
			if (!OwnPosition(C, OwnPos) || !EntityPosition(C, CurrentOrLastTarget(C), TargetPos)) return {};
			return FVector2D::Distance(OwnPos, TargetPos);
		});
	RegisterNumber(TEXT("npc_home_distance"), TEXT("Tiles from where the selection (or last selection) stands to the nearest spawn-slot home of its template (zones/test_zone.toml); unknown while it walks (newest EntityMove has a destination)"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UNetClientSubsystem* Net = C.Net();
			const FString Id = CurrentOrLastTarget(C);
			const FEntitySpawn* Known = Net && !Id.IsEmpty() ? Net->GetKnownEntities().Find(Id) : nullptr;
			if (Net && !Known)
			{
				for (const TPair<FString, FEntitySpawn>& E : Net->GetKnownEntities()) if (E.Key.Equals(Id, ESearchCase::IgnoreCase)) Known = &E.Value;
			}
			FVector2D Pos;
			const FBotFixtureData& D = FBotFixtureData::Get();
			if (!Known || !D.IsValid() || !C.Observations || C.Observations->MovingEntities.Contains(Id.ToLower()) || !EntityPosition(C, Known->EntityId, Pos)) return {};
			return D.NearestHomeDistance(Pos, Known->TemplateId);
		});
	RegisterFlag(TEXT("target_wounded"), TEXT("The selection (or last selection) is alive and below its max HP in the projection"),
		[](const FBotContext& C)
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			const FCombatEntity* E = Combat ? Combat->FindEntity(CurrentOrLastTarget(C)) : nullptr;
			return E && !E->bDead && E->MaxHp > 0 && E->Hp < E->MaxHp;
		});
	RegisterFlag(TEXT("target_hp_full"), TEXT("The selection (or last selection) is alive at its max HP in the projection"),
		[](const FBotContext& C)
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			const FCombatEntity* E = Combat ? Combat->FindEntity(CurrentOrLastTarget(C)) : nullptr;
			return E && !E->bDead && E->MaxHp > 0 && E->Hp == E->MaxHp;
		});
	Custom(TEXT("target_hit_from_full"), TEXT("The newest landed hit on the selection (or last selection), after the newest nf.Mark when there is one, started from its max HP: hp_after + damage == max HP. Shows a heal the wire does not report (an NPC that walked home)"),
		[](const FBotContext& C) -> FBotPredicateValue
		{
			const UCombatStateSubsystem* Combat = C.Combat();
			const FString Id = CurrentOrLastTarget(C).ToLower();
			const FCombatEntity* E = Combat ? Combat->FindEntity(Id) : nullptr;
			const FBotObservations::FLastHit* Hit = C.Observations ? C.Observations->LastHitOn.Find(Id) : nullptr;
			if (!E || !Hit) return { false, TEXT("no hit seen") };
			if (const int32* Mark = C.Observations->HitMarks.Find(Id); Mark && Hit->Count <= *Mark) return { false, TEXT("no hit since nf.Mark") };
			return { Hit->HpAfter + Hit->Damage == E->MaxHp, FString::Printf(TEXT("hp after %u + damage %u, max %u"), Hit->HpAfter, Hit->Damage, E->MaxHp) };
		});
	RegisterNumber(TEXT("target_hits"), TEXT("Landed hits (HIT/CRIT AttackResults) on the selection (or last selection), by anyone"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const FBotObservations::FLastHit* Hit = C.Observations ? C.Observations->LastHitOn.Find(CurrentOrLastTarget(C).ToLower()) : nullptr;
			return C.Observations ? TOptional<double>(Hit ? Hit->Count : 0) : TOptional<double>();
		});
	RegisterNumber(TEXT("target_hits_since_mark"), TEXT("Landed hits on the selection (or last selection) since the newest nf.Mark"),
		[](const FBotContext& C) -> TOptional<double>
		{
			if (!C.Observations) return {};
			const FString Id = CurrentOrLastTarget(C).ToLower();
			const FBotObservations::FLastHit* Hit = C.Observations->LastHitOn.Find(Id);
			return static_cast<double>((Hit ? Hit->Count : 0) - C.Observations->HitMarks.FindRef(Id));
		});
	RegisterFlag(TEXT("target_engaged_me"), TEXT("The selection (or last selection) swung at the player (an AttackResult with it as attacker) in the player's current life"),
		[](const FBotContext& C) { return C.Observations && C.Observations->NpcsAttackingOwn.Contains(CurrentOrLastTarget(C).ToLower()); });
	RegisterNumber(TEXT("attacked_by"), TEXT("Distinct NPCs that swung at the player (AttackResult, hit or miss) since its newest respawn or reconnect"),
		Count([](const FBotObservations& O) { return O.NpcsAttackingOwn.Num(); }));
	RegisterNumber(TEXT("npcs_fighting"), TEXT("Distinct NPCs seen swinging at any player, this one or another"),
		Count([](const FBotObservations& O) { return O.NpcsAttackingPlayers.Num(); }));
	RegisterNumber(TEXT("players_in_view"), TEXT("Other players in view (net cache, kind PLAYER)"),
		[](const FBotContext& C) -> TOptional<double>
		{
			const UNetClientSubsystem* N = C.Net();
			if (!N) return {};
			int32 Players = 0;
			for (const TPair<FString, FEntitySpawn>& Known : N->GetKnownEntities()) Players += Known.Value.Kind == 1 && !N->IsOwnEntity(Known.Key);
			return static_cast<double>(Players);
		});
	RegisterNumber(TEXT("reconnects"), TEXT("WebSocket connections opened after the first"),
		Count([](const FBotObservations& O) { return FMath::Max(0, O.Connects - 1); }));
	RegisterFlag(TEXT("xp_unknown_after_reconnect"), TEXT("After the newest disconnect the HUD showed XP -- until a StatsChanged / XpGained arrived"),
		[](const FBotContext& C) { return C.Observations && C.Observations->bXpUnknownSeenAfterReconnect; });
	RegisterNumber(TEXT("xp_known_before_stats"), TEXT("Ticks the projection showed an XP total after a disconnect before any StatsChanged / XpGained (must stay 0)"),
		Count([](const FBotObservations& O) { return O.XpKnownBeforeStats; }));
	Custom(TEXT("xp_restored"), TEXT("The XP total is known again and equals the total before the newest disconnect plus XpGained since"),
		[](const FBotContext& C) -> FBotPredicateValue
		{
			const FBotObservations* O = C.Observations;
			const UCombatStateSubsystem* Combat = C.Combat();
			if (!O || !Combat || !O->XpAtDisconnect.IsSet()) return { false, TEXT("no disconnect seen") };
			if (!Combat->GetOwn().bXpKnown) return { false, TEXT("XP --") };
			const uint64 Expected = O->XpAtDisconnect.GetValue() + O->XpGainedSinceReconnect;
			return { Combat->GetOwn().Xp == Expected, FString::Printf(TEXT("XP %llu (before the drop %llu, gained since %llu)"), Combat->GetOwn().Xp, O->XpAtDisconnect.GetValue(), O->XpGainedSinceReconnect) };
		});
	RegisterNumber(TEXT("spawns_mid_fight"), TEXT("NPC spawns received wounded, dead or in a life after the first (late AOI entry onto a fight)"),
		Count([](const FBotObservations& O) { return O.SpawnsMidFight; }));
	RegisterNumber(TEXT("late_spawn_hp_ok"), TEXT("Wounded NPC spawns whose next AttackResult continued from the spawn's HP (hp_after = spawn HP - damage)"),
		Count([](const FBotObservations& O) { return O.LateSpawnHpOk; }));
	RegisterNumber(TEXT("late_spawn_projection_ok"), TEXT("Wounded NPC spawns the combat projection showed with the spawn's HP and life before any hit on them"),
		Count([](const FBotObservations& O) { return O.LateSpawnProjectionOk; }));
	RegisterNumber(TEXT("late_spawn_projection_bad"), TEXT("Wounded NPC spawns the projection showed with another HP or life (must stay 0)"),
		Count([](const FBotObservations& O) { return O.LateSpawnProjectionBad; }));
	RegisterNumber(TEXT("late_spawn_hp_bad"), TEXT("Wounded NPC spawns whose next AttackResult did not continue from the spawn's HP (must stay 0)"),
		Count([](const FBotObservations& O) { return O.LateSpawnHpBad; }));
}
