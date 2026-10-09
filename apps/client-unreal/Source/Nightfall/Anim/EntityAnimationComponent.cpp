#include "EntityAnimationComponent.h"
#include "EntityAnimInstance.h"
#include "Net/NetClientSubsystem.h"
#include "Nightfall.h"
#include "Animation/AnimSequenceBase.h"
#include "Components/SkeletalMeshComponent.h"
#include "Engine/SkeletalMesh.h"
#include "GameFramework/Actor.h"

namespace
{
	const TCHAR* const CockatriceAnims = TEXT("/Game/Vendor/Cockatrice/Anims/");
	const TCHAR* const MannyAnims = TEXT("/Game/Vendor/Mannequins/Anims/");

	FSoftObjectPath Asset(const FString& PackagePath)
	{
		return FSoftObjectPath(PackagePath + TEXT(".") + FPaths::GetBaseFilename(PackagePath));
	}

	UAnimSequenceBase* LoadClip(const FSoftObjectPath& Path)
	{
		return Path.IsNull() ? nullptr : Cast<UAnimSequenceBase>(Path.TryLoad());
	}
}

UEntityAnimationComponent::UEntityAnimationComponent()
{
	PrimaryComponentTick.bCanEverTick = true;
	PrimaryComponentTick.TickGroup = TG_PrePhysics;   // before the mesh's animation update
}

FEntityAnimSet UEntityAnimationComponent::PresetAnimSet(EEntityAnimPreset Preset)
{
	FEntityAnimSet Set;
	switch (Preset)
	{
	case EEntityAnimPreset::Cockatrice:
		// Take choice and timings measured from the imported takes (README "Art and animation").
		Set.Mesh = Asset(TEXT("/Game/Vendor/Cockatrice/SK_Cockatrice"));
		Set.MeshScale = 0.8f;
		Set.Idle = Asset(FString(CockatriceAnims) + TEXT("AS_Cockatrice_Idle"));
		Set.Walk = Asset(FString(CockatriceAnims) + TEXT("AS_Cockatrice_Walk"));
		Set.Attack = Asset(FString(CockatriceAnims) + TEXT("AS_Cockatrice_Attack"));
		Set.Death = Asset(FString(CockatriceAnims) + TEXT("AS_Cockatrice_Death"));
		// Both "Damage" takes in the FBX carry no bone motion; the death clip's opening recoil stands in.
		Set.Flinch = Set.Death;
		Set.FlinchSeconds = 0.35f;
		Set.AttackImpactSeconds = 1.58f;   // head lowest and furthest forward
		Set.AttackPlayRate = 1.6f;          // wind-up ~1 s, the keltir's impact half of a ~2 s swing
		break;
	case EEntityAnimPreset::Manny:
		Set.Mesh = Asset(TEXT("/Game/Vendor/Mannequins/Meshes/SKM_Manny_Simple"));
		Set.Idle = Asset(FString(MannyAnims) + TEXT("Unarmed/MM_Idle"));
		Set.Walk = Asset(FString(MannyAnims) + TEXT("Unarmed/Walk/MF_Unarmed_Walk_Fwd"));
		Set.Run = Asset(FString(MannyAnims) + TEXT("Unarmed/Jog/MF_Unarmed_Jog_Fwd"));
		Set.Attack = Asset(FString(MannyAnims) + TEXT("Unarmed/Attack/MM_Attack_01"));
		Set.Death = Asset(FString(MannyAnims) + TEXT("Death/MM_Death_Front_01"));
		Set.Flinch = Set.Death;
		Set.FlinchSeconds = 0.2f;
		Set.AttackImpactSeconds = 0.40f;   // right hand at full reach
		Set.AttackPlayRate = 1.f;
		Set.RunSpeedTilesPerSecond = 3.5f;  // players move at 5 tiles/s
		break;
	case EEntityAnimPreset::None:
		break;
	}
	return Set;
}

bool UEntityAnimationComponent::ApplyToMesh(USkeletalMeshComponent* MeshComponent, const FVector& ExtraOffset)
{
	if (!MeshComponent || AnimSet.Mesh.IsNull()) return false;
	USkeletalMesh* Mesh = Cast<USkeletalMesh>(AnimSet.Mesh.TryLoad());
	UAnimSequenceBase* Idle = LoadClip(AnimSet.Idle);
	UAnimSequenceBase* Walk = LoadClip(AnimSet.Walk);
	UAnimSequenceBase* Attack = LoadClip(AnimSet.Attack);
	UAnimSequenceBase* Death = LoadClip(AnimSet.Death);
	if (!Mesh || !Idle || !Walk || !Attack || !Death)
	{
		UE_LOG(LogNightfall, Log, TEXT("%s: art %s not imported (Scripts/import-vendor.sh); keeping the placeholder"),
			*GetNameSafe(GetOwner()), *AnimSet.Mesh.ToString());
		return false;
	}
	UAnimSequenceBase* Run = LoadClip(AnimSet.Run);
	UAnimSequenceBase* Flinch = AnimSet.FlinchSeconds > 0.f ? LoadClip(AnimSet.Flinch) : nullptr;

	MeshComponent->SetSkeletalMesh(Mesh);
	MeshComponent->SetRelativeLocationAndRotation(ExtraOffset, AnimSet.MeshRotation);
	MeshComponent->SetRelativeScale3D(FVector(AnimSet.MeshScale));
	MeshComponent->SetAnimationMode(EAnimationMode::AnimationBlueprint);
	MeshComponent->SetAnimInstanceClass(UEntityAnimInstance::StaticClass());
	MeshComponent->SetVisibility(true);
	UEntityAnimInstance* Instance = Cast<UEntityAnimInstance>(MeshComponent->GetAnimInstance());
	if (!Instance) return false;
	Instance->SetClip(EProxyClip::Idle, Idle);
	Instance->SetClip(EProxyClip::Walk, Walk);
	Instance->SetClip(EProxyClip::Run, Run);
	Instance->SetClip(EProxyClip::Attack, Attack);
	Instance->SetClip(EProxyClip::Death, Death);
	Instance->SetClip(EProxyClip::Flinch, Flinch);
	AnimInstance = Instance;
	// The mesh must sample this frame's pose, not last frame's: tick after this component.
	MeshComponent->AddTickPrerequisiteComponent(this);

	FProxyClipTiming Timing;
	Timing.AttackLengthSeconds = Attack->GetPlayLength();
	Timing.AttackImpactSeconds = AnimSet.AttackImpactSeconds;
	Timing.AttackPlayRate = AnimSet.AttackPlayRate;
	Timing.DeathLengthSeconds = Death->GetPlayLength();
	Timing.FlinchSeconds = Flinch ? FMath::Min<double>(AnimSet.FlinchSeconds, Flinch->GetPlayLength()) : 0.0;
	Timing.RunSpeedTilesPerSecond = Run ? AnimSet.RunSpeedTilesPerSecond : 0.f;
	Machine.SetTiming(Timing);
	Instance->SetPose(Machine.Evaluate(RenderTimeMs()));
	return true;
}

void UEntityAnimationComponent::Bind(UNetClientSubsystem* InNet, const FString& InEntityId)
{
	Unbind();
	Net = InNet;
	EntityId = InEntityId;
	if (!InNet) return;
	SpawnHandle = InNet->OnEntitySpawn.AddUObject(this, &UEntityAnimationComponent::ApplySpawn);
	MoveHandle = InNet->OnEntityMove.AddUObject(this, &UEntityAnimationComponent::ApplyMove);
	AttackHandle = InNet->OnAttackResult.AddUObject(this, &UEntityAnimationComponent::ApplyAttackResult);
	DiedHandle = InNet->OnEntityDied.AddUObject(this, &UEntityAnimationComponent::ApplyDied);
	RespawnedHandle = InNet->OnEntityRespawned.AddUObject(this, &UEntityAnimationComponent::ApplyRespawned);
	for (const TPair<FString, FEntitySpawn>& Known : InNet->GetKnownEntities())
	{
		if (Matches(Known.Key)) ApplySpawn(Known.Value);
	}
}

void UEntityAnimationComponent::Unbind()
{
	if (UNetClientSubsystem* N = Net.Get())
	{
		N->OnEntitySpawn.Remove(SpawnHandle);
		N->OnEntityMove.Remove(MoveHandle);
		N->OnAttackResult.Remove(AttackHandle);
		N->OnEntityDied.Remove(DiedHandle);
		N->OnEntityRespawned.Remove(RespawnedHandle);
	}
	Net.Reset();
}

void UEntityAnimationComponent::EndPlay(const EEndPlayReason::Type EndPlayReason)
{
	Unbind();
	Super::EndPlay(EndPlayReason);
}

int64 UEntityAnimationComponent::RenderTimeMs() const
{
	const UNetClientSubsystem* N = Net.Get();
	const int64 Now = N ? N->EstimatedServerTimeMs() : FDateTime::UtcNow().ToUnixTimestamp() * 1000;
	return bRenderAtServerTime ? Now : Now - InterpolationDelayMs;
}

int64 UEntityAnimationComponent::TickToMs(uint64 Tick) const
{
	const UNetClientSubsystem* N = Net.Get();
	return N ? N->TickToServerTimeMs(Tick) : static_cast<int64>(Tick) * UNetClientSubsystem::TICK_MS;
}

void UEntityAnimationComponent::ApplySpawn(const FEntitySpawn& Spawn)
{
	if (!Matches(Spawn.EntityId)) return;
	if (Spawn.LifeIncarnation != 0 && Spawn.LifeIncarnation < Incarnation) return;   // an earlier life, late
	Incarnation = FMath::Max(Incarnation, Spawn.LifeIncarnation);
	if (Spawn.bCombatant && Spawn.bDead) Machine.NoteSpawnedDead();   // late AOI entry: already a corpse
	else Machine.NoteAlive(RenderTimeMs());
}

void UEntityAnimationComponent::ApplyMove(const FEntityMove& Move)
{
	if (Matches(Move.EntityId)) Machine.NoteMove(Move.ServerTimeMs, Move.Speed);
}

void UEntityAnimationComponent::ApplyAttackResult(const FAttackResult& Result)
{
	if (Matches(Result.Attacker)) Machine.NoteAttack(TickToMs(Result.Tick));
	const bool bOlderLife = Result.TargetIncarnation != 0 && Result.TargetIncarnation < Incarnation;
	if (Matches(Result.Target) && !bOlderLife && (Result.Outcome == ENetAttackOutcome::Hit || Result.Outcome == ENetAttackOutcome::Crit))
	{
		Machine.NoteHit(TickToMs(Result.Tick));
	}
}

void UEntityAnimationComponent::ApplyDied(const FEntityDied& Died)
{
	if (!Matches(Died.Entity)) return;
	if (Died.Incarnation != 0 && Died.Incarnation < Incarnation) return;   // an earlier life's death
	if (Died.Tick < LastLifeTick) return;
	LastLifeTick = Died.Tick;
	Incarnation = FMath::Max(Incarnation, Died.Incarnation);
	Machine.NoteDied(TickToMs(Died.Tick));
}

void UEntityAnimationComponent::ApplyRespawned(const FEntityRespawned& Respawned)
{
	if (!Matches(Respawned.Entity) || Respawned.Tick < LastLifeTick) return;   // a replayed, older respawn
	LastLifeTick = Respawned.Tick;
	Machine.NoteAlive(TickToMs(Respawned.Tick));
}

void UEntityAnimationComponent::TickComponent(float DeltaTime, ELevelTick TickType, FActorComponentTickFunction* ThisTickFunction)
{
	Super::TickComponent(DeltaTime, TickType, ThisTickFunction);
	if (bRenderAtServerTime)
	{
		const AActor* Owner = GetOwner();
		const float CmPerSecond = Owner ? Owner->GetVelocity().Size2D() : 0.f;
		Machine.SetSpeedOverride(CmPerSecond / 100.f);   // 100 cm per tile
	}
	const int64 Now = RenderTimeMs();
	const FProxyAnimPose Pose = Machine.Evaluate(Now);
	if (Pose.State != LastPose.State)
	{
		UE_LOG(LogNightfall, Verbose, TEXT("anim %s: %s -> %s"), *EntityId, LexToString(LastPose.State), LexToString(Pose.State));
	}
	LastPose = Pose;
	if (UEntityAnimInstance* Instance = AnimInstance.Get()) Instance->SetPose(Pose);
	Machine.Prune(Now);
}
