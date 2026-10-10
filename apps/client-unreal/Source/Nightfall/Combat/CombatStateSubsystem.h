#pragma once

#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Net/ProtoCodec.h"
#include "CombatStateSubsystem.generated.h"

class UNetClientSubsystem;

/** One server entity's public combat state, exactly as the server last said it. */
struct FCombatEntity
{
	FEntitySpawn Spawn;          // identity: id, name, kind, session generation, template
	bool bCombatant = false;
	bool bDead = false;
	bool bAttackable = false;
	uint32 Incarnation = 0;      // NPC life counter; 0 = unknown
	uint32 Hp = 0;
	uint32 MaxHp = 0;
	uint32 Level = 0;
	uint64 LastFactTick = 0;     // newest tick that touched Hp / life; older facts are stale
};

/** Owner-only state: MP, XP and the selection arrive only for the player's own entity. */
struct FOwnCombatState
{
	bool bMpKnown = false;
	uint32 Mp = 0;
	uint32 MaxMp = 0;
	bool bCpKnown = false;
	uint32 Cp = 0;
	uint32 MaxCp = 0;
	uint32 ClassId = 0;
	uint64 Sp = 0;
	uint32 TokenTier1Count = 0;
	uint32 TokenTier2Count = 0;
	uint64 LastStatsTick = 0;
	bool bXpKnown = false;       // XP total arrives with StatsChanged and XpGained; unknown until the first of them after a reconnect
	uint64 Xp = 0;
	uint64 LastXpGain = 0;
	FString TargetId;            // lower-case entity id; empty = none
};

enum class EAttackState : uint8
{
	Idle,      // no Attack outstanding
	Pending,   // Attack sent, not yet Acked
	Active,    // Attack Acked: the server is auto-attacking until something ends it
};

/** One floating damage number: produced once per AttackResult (deduped on tick+attacker+target). */
struct FDamageNumber
{
	FString AttackerId;
	FString TargetId;
	uint64 Tick = 0;
	ENetAttackOutcome Outcome = ENetAttackOutcome::Unspecified;
	uint32 Damage = 0;
	bool bTargetIsOwn = false;
};

/** Everything the HUD shows, pre-formatted. The widgets bind to this and compute nothing. */
struct FCombatHudModel
{
	bool bTargetVisible = false;
	FString TargetName;
	uint32 TargetLevel = 0;
	float TargetHpFraction = 0.f;
	FString TargetHpText;

	bool bOwnKnown = false;
	FString OwnName;
	FString LevelText;           // "Lv 3"
	float OwnHpFraction = 0.f;
	FString OwnHpText;           // "120 / 300"
	float OwnMpFraction = 0.f;
	FString OwnMpText;           // "MP —" until the owner stats arrive
	float OwnCpFraction = 0.f;
	FString OwnCpText;
	FString XpText;              // "XP --" until the owner stats arrive

	bool bDeadOverlay = false;
	bool bRespawnPending = false;
	EAttackState AttackState = EAttackState::Idle;
	FString AttackText;          // "" | "Attacking..." (pending) | "Attacking"
};

DECLARE_MULTICAST_DELEGATE_OneParam(FOnDamageNumber, const FDamageNumber&);
DECLARE_MULTICAST_DELEGATE(FOnCombatStateChanged);

/**
 * The client's projection of server combat facts (E5.2) and the combat intent logic (E5.3).
 *
 * It holds exactly what the server has said: per-entity HP / max HP / level / life, and the
 * player's own MP / XP / target. There is no damage, hit chance or cooldown arithmetic here;
 * HP only ever changes because an event carried a new number.
 *
 * Ordering and staleness (the stream is ordered, but the world is not):
 *  - a spawn with a lower session generation, or a lower life incarnation, than what is held is
 *    stale and dropped (NightfallProto::IsStaleSpawn);
 *  - AttackResult / EntityDied / EntityRespawned for an earlier incarnation, or older than the newest fact applied
 *    to that entity, are dropped, and a repeated AttackResult (same tick+attacker+target)
 *    changes nothing and shows no second number;
 *  - StatsChanged, XpGained and TargetChanged are owner-only: for anyone else they are ignored;
 *  - the whole projection is rebuilt from EntitySpawn / StatsChanged after a reconnect.
 *
 * A game-instance subsystem, so it already listens when the socket connects before the world map
 * loads, and survives map travel.
 */
UCLASS()
class NIGHTFALL_API UCombatStateSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()

public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	// --- Projection inputs: bound to UNetClientSubsystem, public so tests can replay events. ---
	void ApplyClassChanged(const FClassChanged& Changed);
	void ApplySpawn(const FEntitySpawn& Spawn);
	void ApplyDespawn(const FEntityDespawn& Despawn);
	void ApplyAttackResult(const FAttackResult& Result);
	void ApplyDied(const FEntityDied& Died);
	void ApplyRespawned(const FEntityRespawned& Respawned);
	void ApplyStats(const FStatsChanged& Stats);
	void ApplyXp(const FXpGained& Xp);
	void ApplyLevelUp(const FLevelUp& LevelUp);
	void ApplyTargetChanged(const FTargetChanged& Changed);
	void ApplyAck(const FAck& Ack);
	void ApplyRejected(const FIntentRejected& Rejected);

	/** Forgets everything the server said; the next EntitySpawn / StatsChanged stream rebuilds it. */
	UFUNCTION()
	void Reset();

	// --- Queries. Entity ids are case-insensitive. ---
	const FCombatEntity* FindEntity(const FString& EntityId) const;
	const FOwnCombatState& GetOwn() const { return Own; }
	const FCombatEntity* FindOwnEntity() const;
	const FString& GetTargetId() const { return Own.TargetId; }
	bool IsOwnDead() const;
	bool IsAttackable(const FString& EntityId) const;
	EAttackState GetAttackState() const { return AttackState; }
	bool IsRespawnPending() const { return RespawnSeq != 0; }
	const FString& GetStatusLine() const { return StatusLine; }
	int32 NumEntities() const { return Entities.Num(); }
	FCombatHudModel BuildHudModel() const;

	// --- Intent logic (E5.3). ---

	/**
	 * Left-click on an attackable NPC proxy: SetTarget (if it is not already the selection or the
	 * pending selection) then Attack (if no Attack is pending or acked for it). A repeated click
	 * sends nothing. Returns true when the click was a valid attack click.
	 */
	bool ClickEntity(const FString& EntityId);

	/**
	 * Selection without an attack (nf.Target): SetTarget unless EntityId already is the selection
	 * or the pending one; empty clears it. Any id is sent as given, so the server's answer to an
	 * unknown one can be observed. Returns the seq (0 = none sent).
	 */
	uint32 SelectTarget(const FString& EntityId);

	/** Attack the selection (nf.Attack): one Attack while idle, as a click would. Returns the seq (0 = none sent). */
	uint32 AttackSelection();

	/** Ground click: sends StopAttack when an attack is outstanding. Returns the seq (0 = none sent). */
	uint32 NoteGroundClick();

	/** The dead overlay's button. Sends one Respawn until it is answered. Returns the seq (0 = none sent). */
	uint32 RequestRespawn();

	/** Shows a line in the status line (and the login-flow status the HUD displays). */
	void SetStatus(const FString& Text);

	FOnDamageNumber OnDamageNumber;
	FOnCombatStateChanged OnChanged;

private:
	enum class EIntentKind : uint8 { SetTarget, Attack, StopAttack, Respawn };

	UNetClientSubsystem* Net() const;
	static FString Key(const FString& EntityId) { return EntityId.ToLower(); }
	bool IsOwn(const FString& EntityId) const;
	FCombatEntity& FindOrAdd(const FString& EntityId);
	void ClearTarget();
	/** The selection the server will end up with once the SetTargets in flight land. */
	FString RequestedTarget() const;
	void EndAttack() { AttackState = EAttackState::Idle; AttackSeq = 0; }
	bool RememberEvent(const FString& EventKey);
	uint32 Track(uint32 Seq, EIntentKind Kind);
	UFUNCTION() void HandleNetDisconnected(const FString& Reason);

	TMap<FString, FCombatEntity> Entities;   // keyed by lower-case id
	FOwnCombatState Own;

	EAttackState AttackState = EAttackState::Idle;
	FString PendingTargetId;                 // SetTarget sent, not yet confirmed by TargetChanged
	bool bClearPending = false;              // SetTarget(none) sent, not yet confirmed by TargetChanged(none)
	uint32 AttackSeq = 0;
	uint32 RespawnSeq = 0;
	TMap<uint32, EIntentKind> InFlight;      // seq -> what was sent, until Ack / Rejected
	FString StatusLine;

	TMap<FString, FEntitySpawn> Tombstones;  // last spawn of despawned entities: a stale spawn after an AOI exit is still stale

	TSet<FString> SeenEvents;                // dedupe of AttackResult: tick|attacker|target
	TArray<FString> SeenOrder;

	TArray<FDelegateHandle> Handles;
};
