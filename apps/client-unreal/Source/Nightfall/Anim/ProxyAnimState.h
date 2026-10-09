#pragma once

#include "CoreMinimal.h"

/** What an entity's body is doing, as derived from server facts. Diagram: docs/diagrams/proxy-animation-states.html. */
enum class EProxyAnimState : uint8
{
	Idle,
	Walk,
	Run,
	Attack,   // a swing whose impact the server reported; the clip is placed so the hit lands on the impact
	Flinch,   // hit (not missed) by someone else's swing, from its impact
	Dying,    // the death clip, from the death tick
	Corpse,   // the death clip's last frame, held until despawn or respawn
};

enum class EProxyClip : uint8 { None, Idle, Walk, Run, Attack, Flinch, Death };

const TCHAR* LexToString(EProxyAnimState State);

/** Per-body clip timing. Lengths come from the loaded sequences; the rest from the art (THIRD_PARTY_ASSETS.md). */
struct FProxyClipTiming
{
	double AttackLengthSeconds = 1.0;
	double AttackImpactSeconds = 0.5;  // where in the clip (at rate 1) the blow lands
	double AttackPlayRate = 1.0;
	double FlinchSeconds = 0.0;        // 0: no flinch clip, the body keeps its locomotion
	double DeathLengthSeconds = 1.0;
	float RunSpeedTilesPerSecond = 0.f; // at or above this a moving body runs; 0: never runs
};

/** One evaluated frame: which clip, where in it, and whether it loops. */
struct FProxyAnimPose
{
	EProxyAnimState State = EProxyAnimState::Idle;
	EProxyClip Clip = EProxyClip::Idle;
	double ClipTime = 0.0;   // seconds into the clip; a looping clip wraps it by its own length
	bool bLoop = true;
	double PlayRate = 1.0;    // how fast ClipTime advances per second (the attack's rate; 1 otherwise)
};

/**
 * The animation state of one body, computed purely from timestamped server facts and the time
 * the body is rendered at. No engine objects, so it is tested without a world.
 *
 * Times are server milliseconds (time_origin + tick * 100). Remote proxies render 150 ms behind
 * the server (FSnapshotBuffer::INTERP_DELAY_MS), so an AttackResult that arrives about when its
 * impact happened still has its impact in the proxy's future: the clip starts
 * AttackImpactSeconds / AttackPlayRate before the impact and the hit lands on the impact tick.
 * When the impact is already in the past (the own pawn renders at server time), the clip starts
 * part-way through so the impact frame still coincides with the impact.
 *
 * Priority: dead > attack > flinch > locomotion. A respawn (or a live spawn) clears death.
 */
class NIGHTFALL_API FProxyAnimStateMachine
{
public:
	void SetTiming(const FProxyClipTiming& InTiming) { Timing = InTiming; }
	const FProxyClipTiming& GetTiming() const { return Timing; }

	/** EntityMove: speed (tiles/s) valid from ServerTimeMs. */
	void NoteMove(int64 ServerTimeMs, float SpeedTilesPerSecond);
	/** Overrides the move samples (the own pawn walks its local preview). Negative: use the samples. */
	void SetSpeedOverride(float SpeedTilesPerSecond) { SpeedOverride = SpeedTilesPerSecond; }
	/** AttackResult with this body as the attacker. */
	void NoteAttack(int64 ImpactServerTimeMs);
	/** AttackResult with this body as the target (hit or crit). */
	void NoteHit(int64 ImpactServerTimeMs);
	/** EntityDied. */
	void NoteDied(int64 DeathServerTimeMs);
	/** Spawned dead (late AOI entry): a corpse from the start, no death clip. */
	void NoteSpawnedDead();
	/** EntityRespawned or a live spawn: alive again from ServerTimeMs (a delayed body stays dead until then). */
	void NoteAlive(int64 ServerTimeMs);

	FProxyAnimPose Evaluate(int64 RenderTimeMs) const;

	/** Forgets facts older than this; called once per frame with the render time. */
	void Prune(int64 RenderTimeMs);

	/** Dead at this render time (dying or corpse). */
	bool IsDeadAt(int64 RenderTimeMs) const;

private:
	struct FMoveSample { int64 TimeMs; float Speed; };

	float SpeedAt(int64 RenderTimeMs) const;
	int64 AttackStartMs(int64 ImpactMs) const;

	FProxyClipTiming Timing;
	TArray<FMoveSample> Moves;        // ascending time
	TArray<int64> AttackImpacts;      // ascending
	TArray<int64> HitImpacts;         // ascending
	TOptional<int64> DeathMs;         // INT64_MIN: spawned dead
	TOptional<int64> AliveMs;         // a respawn after DeathMs: dead until then, alive from then
	float SpeedOverride = -1.f;
};
