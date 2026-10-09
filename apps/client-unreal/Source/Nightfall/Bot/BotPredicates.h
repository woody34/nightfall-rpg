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

	void Bind(UGameInstance* GameInstance);
	void Unbind();
	void Reset();
	/** Updates LastTargetId / LastTargetHp from the projection; the runner calls it every tick. */
	void Observe(const UCombatStateSubsystem* Combat);

private:
	TWeakObjectPtr<UNetClientSubsystem> BoundNet;
	TWeakObjectPtr<UCombatStateSubsystem> BoundCombat;
	FDelegateHandle SpawnHandle, AckHandle, RejectHandle, NumberHandle, TargetHandle, HitHandle, DiedHandle;
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
}
