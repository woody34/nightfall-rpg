#include "BotPredicates.h"
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
		AckHandle = Net->OnIntentAck.AddLambda([this](const FAck& Ack) { ++Acks; LastAckSeq = FMath::Max(LastAckSeq, Ack.Seq); });
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
}

void FBotObservations::Observe(const UCombatStateSubsystem* Combat)
{
	if (!Combat) return;
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
	RegisterNumber(TEXT("acks"), TEXT("Intent Acks received so far (rejections excluded)"),
		[](const FBotContext& C) -> TOptional<double> { return C.Observations ? TOptional<double>(C.Observations->Acks) : TOptional<double>(); });
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
}
