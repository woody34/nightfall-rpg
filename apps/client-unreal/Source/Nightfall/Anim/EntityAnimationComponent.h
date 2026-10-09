#pragma once

#include "CoreMinimal.h"
#include "Components/ActorComponent.h"
#include "Anim/ProxyAnimState.h"
#include "Net/ProtoCodec.h"
#include "EntityAnimationComponent.generated.h"

class UNetClientSubsystem;
class USkeletalMeshComponent;
class UEntityAnimInstance;

/**
 * Which mesh and clips a body uses. Soft paths, so the tracked Blueprints that hold them still
 * load on a clone without the git-ignored Content/Vendor (the body then stays a placeholder).
 * Paths and timings are produced by Scripts/import_vendor.py; see THIRD_PARTY_ASSETS.md.
 */
USTRUCT(BlueprintType)
struct NIGHTFALL_API FEntityAnimSet
{
	GENERATED_BODY()

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall", meta = (AllowedClasses = "/Script/Engine.SkeletalMesh"))
	FSoftObjectPath Mesh;

	/** Mesh transform relative to the entity's ground point (both meshes face +Y, hence yaw -90). */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	FRotator MeshRotation = FRotator(0.f, -90.f, 0.f);

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	float MeshScale = 1.f;

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall", meta = (AllowedClasses = "/Script/Engine.AnimSequenceBase"))
	FSoftObjectPath Idle;

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall", meta = (AllowedClasses = "/Script/Engine.AnimSequenceBase"))
	FSoftObjectPath Walk;

	/** Optional; without it a fast body walks. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall", meta = (AllowedClasses = "/Script/Engine.AnimSequenceBase"))
	FSoftObjectPath Run;

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall", meta = (AllowedClasses = "/Script/Engine.AnimSequenceBase"))
	FSoftObjectPath Attack;

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall", meta = (AllowedClasses = "/Script/Engine.AnimSequenceBase"))
	FSoftObjectPath Death;

	/** Played from its start for FlinchSeconds when hit. Optional. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall", meta = (AllowedClasses = "/Script/Engine.AnimSequenceBase"))
	FSoftObjectPath Flinch;

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	float FlinchSeconds = 0.f;

	/** Seconds into the attack clip (at rate 1) where the blow lands. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	float AttackImpactSeconds = 0.5f;

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	float AttackPlayRate = 1.f;

	/** Server speed (tiles/s) at or above which the Run clip plays; 0 = never. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	float RunSpeedTilesPerSecond = 0.f;
};

UENUM(BlueprintType)
enum class EEntityAnimPreset : uint8
{
	None,
	Manny,       // players: Epic template mannequin
	Cockatrice,  // keltir: Animated Cockatrice by Marko Jäntti
};

/**
 * Animates one body from server facts: idle / walk (/ run) from EntityMove speed, an attack clip
 * placed on each AttackResult this body dealt so the blow lands on the impact tick, a flinch on
 * each hit it took, the death clip from EntityDied and then a frozen corpse until the despawn, and
 * back to idle on EntityRespawned or a live spawn. Nothing here decides combat; it only shows it.
 *
 * Remote proxies render InterpolationDelayMs behind the server like their position
 * (FSnapshotBuffer); the own pawn (bRenderAtServerTime) renders at server time and walks its local
 * preview velocity.
 */
UCLASS(ClassGroup = (Nightfall), meta = (BlueprintSpawnableComponent))
class NIGHTFALL_API UEntityAnimationComponent : public UActorComponent
{
	GENERATED_BODY()

public:
	UEntityAnimationComponent();

	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	FEntityAnimSet AnimSet;

	/** The own pawn: no interpolation delay; speed from the pawn's velocity instead of EntityMove. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Nightfall")
	bool bRenderAtServerTime = false;

	UFUNCTION(BlueprintPure, Category = "Nightfall")
	static FEntityAnimSet PresetAnimSet(EEntityAnimPreset Preset);

	/**
	 * Loads AnimSet and puts it on Mesh with a UEntityAnimInstance. False (and Mesh untouched) when
	 * the mesh or a required clip (idle, walk, attack, death) is not available, e.g. on a clone
	 * that has not run Scripts/import-vendor.sh.
	 */
	bool ApplyToMesh(USkeletalMeshComponent* Mesh, const FVector& ExtraOffset = FVector::ZeroVector);

	/** Starts following EntityId's server events; a spawn already known to Net seeds the life state. */
	void Bind(UNetClientSubsystem* InNet, const FString& InEntityId);

	virtual void EndPlay(const EEndPlayReason::Type EndPlayReason) override;
	virtual void TickComponent(float DeltaTime, ELevelTick TickType, FActorComponentTickFunction* ThisTickFunction) override;

	// Event inputs, public so tests can replay events without a socket.
	void ApplySpawn(const FEntitySpawn& Spawn);
	void ApplyMove(const FEntityMove& Move);
	void ApplyAttackResult(const FAttackResult& Result);
	void ApplyDied(const FEntityDied& Died);
	void ApplyRespawned(const FEntityRespawned& Respawned);

	/** The pose at a render time (server ms); also what TickComponent pushes to the mesh. */
	FProxyAnimPose EvaluateAt(int64 RenderTimeMs) const { return Machine.Evaluate(RenderTimeMs); }
	/** The render time this body uses now. */
	int64 RenderTimeMs() const;

	const FString& GetEntityId() const { return EntityId; }
	UEntityAnimInstance* GetAnimInstance() const { return AnimInstance.Get(); }
	EProxyAnimState GetCurrentState() const { return LastPose.State; }
	/** ApplyToMesh sets this from the loaded clips; tests without the art set it directly. */
	void SetTimingForTesting(const FProxyClipTiming& Timing) { Machine.SetTiming(Timing); }

	static constexpr int64 InterpolationDelayMs = 150;   // = FSnapshotBuffer::INTERP_DELAY_MS

private:
	bool Matches(const FString& Id) const { return !EntityId.IsEmpty() && EntityId.Equals(Id, ESearchCase::IgnoreCase); }
	int64 TickToMs(uint64 Tick) const;
	void Unbind();

	FProxyAnimStateMachine Machine;
	uint32 Incarnation = 0;     // newest NPC life seen (0 = unknown / player); older lives' facts are dropped
	uint64 LastLifeTick = 0;    // newest EntityDied / EntityRespawned tick applied
	FProxyAnimPose LastPose;
	FString EntityId;
	TWeakObjectPtr<UNetClientSubsystem> Net;
	TWeakObjectPtr<UEntityAnimInstance> AnimInstance;
	FDelegateHandle SpawnHandle, MoveHandle, AttackHandle, DiedHandle, RespawnedHandle;
};
