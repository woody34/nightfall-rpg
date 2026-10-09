#pragma once

#include "CoreMinimal.h"

class UGameInstance;
class UNetClientSubsystem;
class UCombatStateSubsystem;
class UAuthSubsystem;
class UWorld;

/**
 * What the bot has seen on the wire that the projections do not keep: the newest acked seq, the
 * last rejection, how many damage numbers were shown, the last selection. Fed by the typed
 * delegates of UNetClientSubsystem / UCombatStateSubsystem (Bind), or directly by tests.
 */
struct NIGHTFALL_API FBotObservations
{
	uint32 LastAckSeq = 0;       // highest seq the server Acked (rejections excluded)
	bool bRejected = false;      // any IntentRejected seen
	uint32 LastRejectReason = 0; // nightfall.v1.RejectReason of the newest rejection
	uint32 LastRejectSeq = 0;
	int32 DamageNumbers = 0;     // UCombatStateSubsystem::OnDamageNumber count (deduped AttackResults)
	int32 Acks = 0;              // Acks received (every Ack, keep-alives excluded by the server: they are rejected)
	int32 OwnSpawns = 0;         // EntitySpawns of the own entity (one per session admission)
	FString FirstOwnEntityId;    // the own entity id at the first own spawn; own_entity_unchanged compares it
	FString LastTargetId;        // the newest non-empty selection, kept after it clears (target_hp reads it)
	TOptional<uint32> LastTargetHp; // its newest HP from AttackResult / EntityDied (0) / the projection; survives its despawn

	// --- Phase 1a E3 (combat scenarios). Own XP/level are tracked from the events here, not read
	// from the projection, so a death or reconnect can be compared against what came before. ---
	TSet<FString> AttackResultKeys;         // distinct valid AttackResults seen: tick|attacker|target
	TSet<FString> NpcsAttackingOwn;         // NPCs whose AttackResult targeted the player since its newest respawn / reconnect
	TSet<FString> NpcsAttackingPlayers;     // NPCs seen swinging at any player (this one or another)
	uint64 NewestTick = 0;                  // newest server tick seen in an Ack or a combat fact
	bool bOwnRespawned = false;             // an EntityRespawned for the player arrived
	uint64 OwnRespawnTick = 0;
	bool bAttackedSinceRespawn = false;     // an Attack was accepted (attack state active) after it
	TOptional<uint64> TrackedXp;            // own XP from StatsChanged / XpGained; reset on disconnect
	uint32 TrackedLevel = 0;
	bool bOwnDeathStatsSeen = false;        // a StatsChanged with HP 0 arrived for the current death
	TOptional<uint64> XpBeforeDeath;        // own XP / level just before the newest death's StatsChanged
	uint32 LevelBeforeDeath = 0;
	TOptional<uint64> XpAfterDeath;
	int32 Connects = 0;                     // WebSocket connections opened (1 + reconnects)
	bool bAwaitingStats = false;            // disconnected or reconnected; no StatsChanged / XpGained since
	bool bXpUnknownSeenAfterReconnect = false;
	int32 XpKnownBeforeStats = 0;           // ticks the projection showed XP while bAwaitingStats (must stay 0)
	TOptional<uint64> XpAtDisconnect;
	uint64 XpGainedSinceReconnect = 0;
	struct FLateSpawn { uint32 Hp = 0; uint32 Incarnation = 0; bool bChecked = false; bool bProjectionChecked = false; };
	TMap<FString, FLateSpawn> WoundedSpawns; // NPC spawns that arrived wounded, until the next AttackResult on them
	int32 SpawnsMidFight = 0;               // NPC spawns that arrived wounded, dead or in a life after the first
	int32 LateSpawnHpOk = 0;                // the next AttackResult on a wounded spawn continued from its HP
	int32 LateSpawnHpBad = 0;
	int32 LateSpawnProjectionOk = 0;        // the projection showed a wounded spawn's HP and life before any hit on it
	int32 LateSpawnProjectionBad = 0;
	TMap<FString, int32> HitMarks;          // nf.Mark: landed hits per target at the mark
	TSet<FString> MovingEntities;           // entities whose newest EntityMove has a destination
	struct FLastHit { uint32 HpAfter = 0; uint32 Damage = 0; int32 Count = 0; };
	TMap<FString, FLastHit> LastHitOn;      // newest landed hit (HIT/CRIT) per target

	/** nf.Mark: the current landed-hit count per target becomes the baseline. */
	void MarkHits()
	{
		HitMarks.Reset();
		for (const TPair<FString, FLastHit>& Hit : LastHitOn) HitMarks.Add(Hit.Key, Hit.Value.Count);
	}

	void Bind(UGameInstance* GameInstance);
	void Unbind();
	void Reset();
	/** Updates LastTargetId / LastTargetHp from the projection; the runner calls it every tick. */
	void Observe(const UCombatStateSubsystem* Combat);

private:
	void BindPhase1(UNetClientSubsystem* Net);
	void ObservePhase1(const UCombatStateSubsystem* Combat);

	TWeakObjectPtr<UNetClientSubsystem> BoundNet;
	TWeakObjectPtr<UCombatStateSubsystem> BoundCombat;
	FDelegateHandle SpawnHandle, AckHandle, RejectHandle, NumberHandle, TargetHandle, HitHandle, DiedHandle;
	TArray<TFunction<void()>> Phase1Unbinders;
	bool bWasConnected = false;
};

/** Everything a predicate may read. Accessors return nullptr when that part is absent. */
struct NIGHTFALL_API FBotContext
{
	UGameInstance* GameInstance = nullptr;
	const FBotObservations* Observations = nullptr;

	UNetClientSubsystem* Net() const;
	UCombatStateSubsystem* Combat() const;
	UAuthSubsystem* Auth() const;
	UWorld* World() const;
};

/** One evaluation: whether the predicate holds and what it saw (for the run log and JUnit). */
struct FBotPredicateValue
{
	bool bTrue = false;
	FString Observed;
};

using FBotPredicateFn = TFunction<FBotPredicateValue(const FBotContext&)>;

/**
 * A named predicate. Bind turns the argument tokens after the name into an evaluator, or fails
 * with OutError: a scenario with a bad predicate is rejected when it is parsed, not when the step
 * runs (E1.2).
 */
struct FBotPredicateDef
{
	FString Name;
	FString Usage;        // e.g. "proxies >= <n>"; listed in the README
	FString Description;
	TFunction<FBotPredicateFn(const TArray<FString>& Args, FString& OutError)> Bind;
};

/**
 * The predicate registry. Built-ins (RegisterBuiltins) cover Phase 0b and Phase 1; later phases
 * add theirs with Register, the helpers below, or by appending to BotPredicates.cpp.
 */
class NIGHTFALL_API FBotPredicateRegistry
{
public:
	/** The process-wide registry with the built-ins. */
	static FBotPredicateRegistry& Get();

	void Register(FBotPredicateDef Def);

	/** A predicate without arguments, e.g. `own_dead`. */
	void RegisterFlag(const FString& Name, const FString& Description, TFunction<bool(const FBotContext&)> Holds);

	/**
	 * `<name> <op> <n>` with op in < <= == != >= >. Value returns the number, or unset when it is
	 * not known (then the predicate is false).
	 */
	void RegisterNumber(const FString& Name, const FString& Description, TFunction<TOptional<double>(const FBotContext&)> Value);

	/**
	 * `<name> == <value>` / `<name> != <value>`. Matches(Context, Value, OutObserved) decides
	 * equality and reports what it saw; Validate rejects bad values at parse time (may be null).
	 */
	void RegisterEquality(const FString& Name, const FString& Usage, const FString& Description,
		TFunction<bool(const FString& Value, FString& OutError)> Validate,
		TFunction<bool(const FBotContext&, const FString& Value, FString& OutObserved)> Matches);

	/** Tokens = name and arguments. Returns null and sets OutError for an unknown name or bad arguments. */
	FBotPredicateFn Parse(const TArray<FString>& Tokens, FString& OutError) const;

	const FBotPredicateDef* Find(const FString& Name) const;
	const TArray<FBotPredicateDef>& All() const { return Defs; }

	void RegisterBuiltins();

	/** Splits on whitespace. */
	static TArray<FString> Tokenize(const FString& Text);

private:
	TArray<FBotPredicateDef> Defs;
};

namespace BotPredicates
{
	/** RejectReason by name: TOO_FAR, REJECT_REASON_TOO_FAR or the number. -1 when unknown. */
	NIGHTFALL_API int32 ParseRejectReason(const FString& Text);
	NIGHTFALL_API FString RejectReasonName(uint32 Reason);

	/** The player's newest server position in tiles (net cache), if known. */
	NIGHTFALL_API bool OwnPosition(const FBotContext& Context, FVector2D& OutTiles);

	/** The attackable entity closest to the player (net cache + projection); empty when none is in view. */
	NIGHTFALL_API FString NearestAttackable(const FBotContext& Context);

	/** An entity's newest server position in tiles (net cache), if it is in view. */
	NIGHTFALL_API bool EntityPosition(const FBotContext& Context, const FString& EntityId, FVector2D& OutTiles);

	/** The nearest living NPC in view that swung at the player in its current life; empty when none. */
	NIGHTFALL_API FString NearestAttacker(const FBotContext& Context);

	/** The selection, or after it cleared the last one (the id target_hp reads); empty when none yet. */
	NIGHTFALL_API FString CurrentOrLastTarget(const FBotContext& Context);
}
