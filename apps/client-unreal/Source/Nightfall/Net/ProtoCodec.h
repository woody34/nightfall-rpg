#pragma once

#include "CoreMinimal.h"

// Wire types the client sends and receives. Mirrors packages/proto/nightfall/v1/world.proto.
// The structs here are the client's view; the codec maps them to protobuf bytes.
//
// ProtoCodec.cpp converts these to TurboLink's generated FGrpcNightfallV1* structs and serialises
// them with protoc-generated code (Scripts/gen-proto.sh) through NightfallWire, which lives in the
// TurboLinkGrpc module next to the protobuf runtime. Gameplay code keeps using these structs.

struct FNetVec2
{
	float X = 0.f;
	float Y = 0.f;
};

struct FMoveToIntent
{
	FNetVec2 Destination;
};

struct FSetTargetIntent
{
	FString EntityId;      // empty clears the selection
};

struct FCastSkillIntent { uint32 SkillId = 0; TOptional<FString> Target; };
struct FCancelCastIntent { TOptional<uint64> CastId; };
struct FAcceptResurrectIntent { uint64 OfferId = 0; bool bAccept = true; };

struct FClientMessage
{
	uint32 Seq = 0;
	TOptional<FCastSkillIntent> CastSkill;
	TOptional<FCancelCastIntent> CancelCast;
	TOptional<FAcceptResurrectIntent> AcceptResurrect;
	bool bStopMove = false;
	TOptional<FMoveToIntent> MoveTo;   // ClientMessage.intent.move_to = 10
	TOptional<FSetTargetIntent> SetTarget; // set_target = 12
	bool bAttack = false;              // attack = 13 (empty message)
	bool bStopAttack = false;          // stop_attack = 14 (empty message)
	bool bRespawn = false;             // respawn = 15 (empty message)
};

struct FEffectiveStats
{
	int64 PAtkQ = 0; // 1
	int64 PDefQ = 0; // 2
	int64 MAtkQ = 0; // 3
	int64 MDefQ = 0; // 4
	uint32 AttackSpeed = 0; // 5
	uint32 CastSpeed = 0; // 6
	int64 SpeedQ = 0; // 7
	int64 AccuracyQ = 0; // 8
	int64 EvasionQ = 0; // 9
	uint32 CritPermille = 0; // 10
};

struct FAttackStarted
{
	FString Attacker; // 1
	FString Target; // 2
	uint64 Tick = 0; // 3
	uint64 ImpactTick = 0; // 4
	uint64 ReadyTick = 0; // 5
	uint32 TargetIncarnation = 0; // 6
	bool bBaseline = false; // 7
	uint64 BaselineTick = 0; // 8
	TOptional<uint32> AttackerIncarnation; // 9
};

struct FAttackCancelled
{
	FString Attacker; // 1
	FString Target; // 2
	uint64 Tick = 0; // 3
	uint32 Reason = 0; // 4
};

struct FActiveEffect
{
	uint32 SkillId = 0; // 1
	uint32 SkillLevel = 0; // 2
	uint64 RemainingTicks = 0; // 3
	bool bIsDebuff = false; // 4
	TOptional<FString> Caster; // 5
	uint64 EffectId = 0; // 6
	TOptional<uint32> CasterIncarnation; // 7
	uint32 TargetIncarnation = 0; // 8
};

struct FSkillCastStarted
{
	FString Caster; // 1
	TOptional<FString> Target; // 2
	uint32 SkillId = 0; // 3
	uint32 SkillLevel = 0; // 4
	uint64 CastId = 0; // 5
	uint64 Tick = 0; // 6
	uint64 CompleteTick = 0; // 7
	uint32 CasterIncarnation = 0; // 8
	TOptional<uint32> TargetIncarnation; // 9
	bool bBaseline = false; // 10
	uint64 BaselineTick = 0; // 11
	uint64 LaunchTick = 0; // 12
	uint64 StartTick = 0; // 13
};

struct FSkillTargetResult
{
	FString Target; // 1
	uint32 TargetIncarnation = 0; // 2
	uint64 Tick = 0; // 3
	uint32 ResolutionKind = 0; // 4
	uint32 Rolled = 0; // 5
	uint32 CpAbsorbed = 0; // 6
	uint32 HpRemoved = 0; // 7
	uint32 Heal = 0; // 8
	uint32 Outcome = 0; // 9
	uint32 TargetHpAfter = 0; // 10
	TArray<uint64> AppliedEffectIds; // 11
};

struct FSkillCastResult
{
	FString Caster; // 1
	uint32 CasterIncarnation = 0; // 2
	uint64 CastId = 0; // 3
	uint64 Tick = 0; // 4
	uint32 SkillId = 0; // 5
	uint32 SkillLevel = 0; // 6
	TArray<FSkillTargetResult> Targets; // 7
	TOptional<uint64> EffectId; // 8
};

struct FSkillCastCancelled
{
	FString Caster; // 1
	uint64 CastId = 0; // 2
	uint32 Reason = 0; // 3
	uint64 Tick = 0; // 4
	uint32 CasterIncarnation = 0; // 5
};

struct FEffectApplied
{
	FString Target; // 1
	uint32 TargetIncarnation = 0; // 2
	TOptional<FString> Caster; // 3
	TOptional<uint32> CasterIncarnation; // 4
	uint64 EffectId = 0; // 5
	uint32 SkillId = 0; // 6
	uint32 SkillLevel = 0; // 7
	uint32 Pool = 0; // 8
	uint64 Tick = 0; // 9
	uint64 EndTick = 0; // 10
	FString AbnormalType; // 11
	uint32 AbnormalLevel = 0; // 12
	uint64 StatusFlags = 0; // 13
	bool bBaseline = false; // 14
	uint64 BaselineTick = 0; // 15
	uint64 AppliedTick = 0; // 16
};

struct FEffectRemoved
{
	FString Target; // 1
	uint32 TargetIncarnation = 0; // 2
	uint64 EffectId = 0; // 3
	uint32 Reason = 0; // 4
	uint64 Tick = 0; // 5
};

struct FSkillReuse
{
	FString Entity; // 1
	uint32 SkillId = 0; // 2
	uint64 ReadyTick = 0; // 3
	uint64 Tick = 0; // 4
	uint32 Incarnation = 0; // 5
};

struct FSkillLearned
{
	FString Entity; // 1
	uint32 SkillId = 0; // 2
	uint32 SkillLevel = 0; // 3
	uint64 SpCost = 0; // 4
	uint64 SpRemaining = 0; // 5
	FString IdempotencyKey; // 6
	uint64 Tick = 0; // 7
	uint32 Incarnation = 0; // 8
};

struct FPvpStateChanged
{
	FString Entity; // 1
	uint64 FlagUntilTick = 0; // 2
	uint32 Karma = 0; // 3
	uint32 PkCount = 0; // 4
	uint32 PvpCount = 0; // 5
	uint64 Tick = 0; // 6
	uint32 Incarnation = 0; // 7
	bool bBaseline = false; // 8
	uint64 BaselineTick = 0; // 9
};

struct FResurrectOffer
{
	FString Target; // 1
	uint32 TargetIncarnation = 0; // 2
	uint64 OfferId = 0; // 3
	TOptional<FString> Caster; // 4
	TOptional<uint32> CasterIncarnation; // 5
	int64 RestorePercentQ = 0; // 6
	uint64 RecoverableXp = 0; // 7
	uint64 ExpiryTick = 0; // 8
	uint32 State = 0; // 9
	uint64 Tick = 0; // 10
	bool bBaseline = false; // 11
	uint64 BaselineTick = 0; // 12
};

struct FSpGained
{
	FString Entity; // 1
	uint64 Amount = 0; // 2
	uint64 Total = 0; // 3
	uint64 Tick = 0; // 4
	uint32 Incarnation = 0; // 5
};

struct FLearnedSkillRank
{
	uint32 SkillId = 0; // 1
	uint32 SkillLevel = 0; // 2
};

struct FSkillReuseState
{
	uint32 SkillId = 0; // 1
	uint64 ReadyTick = 0; // 2
};

struct FSkillStateSync
{
	FString Entity; // 1
	TArray<FLearnedSkillRank> Learned; // 2
	TArray<FSkillReuseState> Reuse; // 3
	uint64 Sp = 0; // 4
	uint32 Karma = 0; // 5
	uint32 PkCount = 0; // 6
	uint32 PvpCount = 0; // 7
	uint64 FlagUntilTick = 0; // 8
	uint32 DeathPenaltyLevel = 0; // 9
	uint64 Tick = 0; // 10
	uint32 Incarnation = 0; // 11
	bool bBaseline = false; // 12
	uint64 BaselineTick = 0; // 13
};

struct FVitalsChanged
{
	FString Entity; // 1
	uint32 Hp = 0; // 2
	uint32 MaxHp = 0; // 3
	uint64 Tick = 0; // 4
	uint32 Incarnation = 0; // 5
	bool bBaseline = false; // 6
	uint64 BaselineTick = 0; // 7
};

struct FEntitySpawn
{
	FString EntityId;      // 1
	FString Name;          // 2
	FNetVec2 Position;     // 3
	uint32 Kind = 0;       // 4: 1 player, 2 npc
	uint32 SessionGeneration = 0; // 5: higher replaces an earlier entity of the same account
	// Combat state at AOI entry (6-13). Meaningful only when bCombatant. MP and XP are owner-only
	// and never arrive here.
	bool bCombatant = false;       // 6
	FString TemplateId;            // 7: NPC template; empty for players
	uint32 LifeIncarnation = 0;    // 8: NPC life counter; a respawn increments it
	bool bDead = false;            // 9
	bool bAttackable = false;      // 10: the receiver may SetTarget it
	uint32 Hp = 0;                 // 11
	uint32 MaxHp = 0;              // 12
	uint32 Level = 0;              // 13
	uint32 Race = 0;
	uint32 ClassId = 0;
	uint32 Sex = 0;
	uint32 HairStyle = 0;
	uint32 HairColor = 0;
	uint32 Face = 0;
	uint64 StateTick = 0;
	uint64 FlagUntilTick = 0; // 22
	uint32 Karma = 0; // 23
	TArray<FActiveEffect> ActiveEffects; // 24
	TOptional<FSkillCastStarted> PendingCast; // 25
	uint32 PkCount = 0; // 26
	uint32 PvpCount = 0; // 27
	uint64 StatusFlags = 0; // 28
	TOptional<FAttackStarted> PendingSwing;
};

struct FEntityMove
{
	FString EntityId;      // 1
	FNetVec2 Position;     // 2
	FNetVec2 Destination;  // 3
	float Speed = 0.f;     // 4
	int64 ServerTimeMs = 0;// 5
	uint64 Tick = 0;       // 6
};

struct FEntityDespawn
{
	FString EntityId;      // 1
};

enum class ENetAttackOutcome : uint8 { Unspecified = 0, Miss = 1, Hit = 2, Crit = 3 };

struct FAttackResult
{
	FString Attacker;      // 1
	FString Target;        // 2
	uint64 Tick = 0;       // 3: impact tick
	ENetAttackOutcome Outcome = ENetAttackOutcome::Unspecified; // 4
	uint32 Damage = 0;     // 5: zero on a miss
	uint32 TargetHpAfter = 0; // 6
	uint32 TargetIncarnation = 0; // 7: the target life the swing landed on
	uint32 CpDamage = 0; // 8
};

struct FEntityDied
{
	FString Entity;        // 1
	uint64 Tick = 0;       // 2
	FString Killer;        // 3
	uint32 Incarnation = 0;// 4: the life that ended
};

struct FEntityRespawned
{
	FString Entity;        // 1
	uint64 Tick = 0;       // 2
	FNetVec2 Position;     // 3
	uint32 Hp = 0;         // 4
	uint32 Incarnation = 0;// 5: the new life; facts about older lives are stale
};

/** Owner-only. */
struct FStatsChanged
{
	FString Entity;        // 1
	uint32 Hp = 0;         // 2
	uint32 MaxHp = 0;      // 3
	uint32 Mp = 0;         // 4
	uint32 MaxMp = 0;      // 5
	uint32 Level = 0;      // 6
	uint64 Xp = 0;         // 7: cumulative XP, including death loss
	uint32 Cp = 0;
	uint32 MaxCp = 0;
	uint32 ClassId = 0;
	uint64 Sp = 0;
	uint32 TokenTier1Count = 0;
	uint32 TokenTier2Count = 0;
	uint64 Tick = 0;	uint32 Karma = 0; // 15
	uint32 PkCount = 0; // 16
	uint32 PvpCount = 0; // 17
	uint64 FlagUntilTick = 0; // 18
	uint32 DeathPenaltyLevel = 0; // 19
	TOptional<FEffectiveStats> EffectiveStats; // 20: absent remains UNKNOWN
	bool bBaseline = false; // 21
	uint64 BaselineTick = 0; // 22
	uint32 Incarnation = 0; // 23
};

/** Owner-only. */
struct FXpGained
{
	FString Entity;        // 1
	uint64 Amount = 0;     // 2
	uint64 Total = 0;      // 3
};

struct FLevelUp
{
	FString Entity;        // 1
	uint32 Level = 0;      // 2
};

/** Owner-only. */
struct FTargetChanged
{
	FString Entity;        // 1: the selecting actor
	FString Target;        // 2: empty means cleared
	bool bBaseline = false; // 3
	uint64 BaselineTick = 0; // 4
	TOptional<uint32> TargetIncarnation; // 5
	uint64 Tick = 0; // 6
};

struct FClassChanged
{
	FString Entity;
	uint32 ClassId = 0;
	uint64 Tick = 0;
	uint32 SessionGeneration = 0;
};

struct FWorldEvent
{
	TOptional<FEntitySpawn> Spawn;     // WorldEvent.spawn = 1
	TOptional<FEntityMove> Move;       // WorldEvent.move = 2
	TOptional<FEntityDespawn> Despawn; // WorldEvent.despawn = 3
	TOptional<FAttackResult> AttackResult;       // 4
	TOptional<FEntityDied> EntityDied;           // 5
	TOptional<FEntityRespawned> EntityRespawned; // 6
	TOptional<FStatsChanged> StatsChanged;       // 7
	TOptional<FXpGained> XpGained;               // 8
	TOptional<FLevelUp> LevelUp;                 // 9
	TOptional<FClassChanged> ClassChanged;       // 13
	TOptional<FTargetChanged> TargetChanged;     // 10
	TOptional<FAttackStarted> AttackStarted;
	TOptional<FAttackCancelled> AttackCancelled;
	TOptional<FSkillCastStarted> SkillCastStarted;
	TOptional<FSkillCastResult> SkillCastResult;
	TOptional<FSkillCastCancelled> SkillCastCancelled;
	TOptional<FEffectApplied> EffectApplied;
	TOptional<FEffectRemoved> EffectRemoved;
	TOptional<FSkillReuse> SkillReuse;
	TOptional<FSkillLearned> SkillLearned;
	TOptional<FPvpStateChanged> PvpStateChanged;
	TOptional<FResurrectOffer> ResurrectOffer;
	TOptional<FSpGained> SpGained;
	TOptional<FSkillStateSync> SkillStateSync;
	TOptional<FVitalsChanged> VitalsChanged;
	// attack_started (11) and attack_cancelled (12) are decoded by the wire layer but the client
	// has no consumer until the animation story (E5.4).
};

struct FAck
{
	uint32 Seq = 0;        // 1
	uint64 Tick = 0;       // 2: tick on which the intent is applied
};

struct FIntentRejected
{
	uint32 Seq = 0;        // 1
	uint32 Reason = 0;     // 2: nightfall.v1.RejectReason
	FString Detail;        // 3: for logs only
};

struct FServerMessage
{
	TOptional<FAck> Ack;                  // ServerMessage.ack = 1
	TOptional<FWorldEvent> Event;         // ServerMessage.event = 2
	TOptional<FIntentRejected> Rejected;  // ServerMessage.rejected = 3
};

namespace NightfallProto
{
	/**
	 * True when New must be discarded because Known describes a newer entity: a lower account
	 * session generation (a replaced session), or, with both known, a lower NPC life incarnation.
	 */
	bool IsStaleSpawn(const FEntitySpawn& Known, const FEntitySpawn& New);

	// Encodes a ClientMessage to protobuf bytes. Never fails for well-formed input.
	void Encode(const FClientMessage& In, TArray<uint8>& Out);

	// Decodes a ServerMessage. Returns false on malformed input; Out is unspecified then.
	bool Decode(const uint8* Data, int32 Size, FServerMessage& Out);
}
