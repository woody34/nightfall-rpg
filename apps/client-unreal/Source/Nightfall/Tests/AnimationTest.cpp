#include "Misc/AutomationTest.h"
#include "TestGameInstance.h"
#include "Anim/EntityAnimationComponent.h"
#include "Anim/EntityAnimInstance.h"
#include "Anim/ProxyAnimState.h"
#include "Combat/CombatStateSubsystem.h"
#include "Game/NightfallGameMode.h"
#include "Net/NetClientSubsystem.h"
#include "World/RemoteEntityActor.h"
#include "World/VendorPropScatter.h"
#include "World/WorldProxySubsystem.h"
#include "Animation/AnimSequenceBase.h"
#include "Components/SkeletalMeshComponent.h"
#include "Engine/Level.h"
#include "Engine/World.h"
#include "Misc/PackageName.h"

#if WITH_DEV_AUTOMATION_TESTS

// E5.4: proxy animation states from synthetic server events. The state machine is pure, so most
// of this needs neither a world nor the (git-ignored) vendor art; Nightfall.Content.VendorArt is
// the one test that loads the imported assets, and it reports a skip when they are absent.

namespace
{
	constexpr EAutomationTestFlags AnimTestFlags = EAutomationTestFlags::EditorContext | EAutomationTestFlags::EngineFilter;

	const TCHAR* const Keltir = TEXT("0b6e2f6e-0000-4000-8000-0000000000c1");
	const TCHAR* const Player = TEXT("0b6e2f6e-0000-4000-8000-0000000000c2");

	/** The cockatrice's measured timing: 4.167 s attack, impact 1.58 s at rate 1.6, 3 s death. */
	FProxyClipTiming CockatriceTiming()
	{
		FProxyClipTiming T;
		T.AttackLengthSeconds = 4.167;
		T.AttackImpactSeconds = 1.58;
		T.AttackPlayRate = 1.6;
		T.FlinchSeconds = 0.35;
		T.DeathLengthSeconds = 3.0;
		return T;
	}

	FServerMessage Msg(const FWorldEvent& E)
	{
		FServerMessage M;
		M.Event = E;
		return M;
	}

	FEntitySpawn SpawnOf(const TCHAR* Id, uint32 Kind, const TCHAR* Template, bool bDead)
	{
		FEntitySpawn S;
		S.EntityId = Id;
		S.Name = TEXT("Keltir");
		S.Kind = Kind;
		S.TemplateId = Template;
		S.bCombatant = true;
		S.bAttackable = Kind == 2;
		S.bDead = bDead;
		S.Hp = bDead ? 0 : 44;
		S.MaxHp = 44;
		S.Level = 1;
		S.LifeIncarnation = 1;
		return S;
	}
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAnimStateLocomotionTest, "Nightfall.Anim.State.Locomotion", AnimTestFlags)

bool FAnimStateLocomotionTest::RunTest(const FString& Parameters)
{
	FProxyAnimStateMachine M;
	FProxyClipTiming T = CockatriceTiming();
	T.RunSpeedTilesPerSecond = 4.5f;
	M.SetTiming(T);

	TestEqual(TEXT("no facts: idle"), M.Evaluate(1000).State, EProxyAnimState::Idle);
	M.NoteMove(2000, 4.f);
	M.NoteMove(3000, 5.f);
	M.NoteMove(4000, 0.f);
	TestEqual(TEXT("before the move is rendered: still idle"), M.Evaluate(1999).State, EProxyAnimState::Idle);
	TestEqual(TEXT("speed 4 < run threshold: walk"), M.Evaluate(2000).State, EProxyAnimState::Walk);
	TestEqual(TEXT("speed 5: run"), M.Evaluate(3500).State, EProxyAnimState::Run);
	TestEqual(TEXT("speed 0: idle"), M.Evaluate(4000).State, EProxyAnimState::Idle);
	TestTrue(TEXT("loops loop"), M.Evaluate(2500).bLoop);

	// Out-of-order arrival is sorted by server time.
	FProxyAnimStateMachine Late;
	Late.NoteMove(3000, 0.f);
	Late.NoteMove(2000, 4.f);
	TestEqual(TEXT("late older sample: walk in between"), Late.Evaluate(2500).State, EProxyAnimState::Walk);
	TestEqual(TEXT("late older sample: idle after"), Late.Evaluate(3100).State, EProxyAnimState::Idle);

	// Pruning keeps the current speed.
	M.Prune(3500);
	TestEqual(TEXT("prune keeps the current sample"), M.Evaluate(3600).State, EProxyAnimState::Run);

	// The own pawn's override wins over samples.
	M.SetSpeedOverride(1.f);
	TestEqual(TEXT("override: walk"), M.Evaluate(4500).State, EProxyAnimState::Walk);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAnimStateAttackTimingTest, "Nightfall.Anim.State.AttackTiming", AnimTestFlags)

bool FAnimStateAttackTimingTest::RunTest(const FString& Parameters)
{
	FProxyAnimStateMachine M;
	M.SetTiming(CockatriceTiming());
	const int64 Impact = 10000;
	const int64 Lead = FMath::RoundToInt64(1.58 / 1.6 * 1000.0);   // 988 ms of wind-up
	M.NoteAttack(Impact);
	M.NoteAttack(Impact);   // a replayed AttackResult changes nothing

	TestEqual(TEXT("before the wind-up: idle"), M.Evaluate(Impact - Lead - 1).State, EProxyAnimState::Idle);
	const FProxyAnimPose Start = M.Evaluate(Impact - Lead);
	TestEqual(TEXT("wind-up starts lead before the impact"), Start.State, EProxyAnimState::Attack);
	TestEqual(TEXT("clip from its start"), Start.ClipTime, 0.0, 0.001);
	const FProxyAnimPose Hit = M.Evaluate(Impact);
	TestEqual(TEXT("impact frame on the impact tick"), Hit.ClipTime, 1.58, 0.002);
	TestFalse(TEXT("one-shot"), Hit.bLoop);
	TestEqual(TEXT("rate carried"), Hit.PlayRate, 1.6, 0.001);
	const int64 End = Impact - Lead + FMath::RoundToInt64(4.167 / 1.6 * 1000.0);
	TestEqual(TEXT("still attacking before the clip ends"), M.Evaluate(End - 5).State, EProxyAnimState::Attack);
	TestEqual(TEXT("back to idle after"), M.Evaluate(End + 5).State, EProxyAnimState::Idle);

	// A proxy renders 150 ms behind the server: an AttackResult received at its impact time (plus
	// latency L < 150 ms) still has its impact ahead of the proxy, so the clip is joined at most
	// (lead - (150 - L)) in and the impact frame still lands on the impact tick.
	const int64 ReceivedAt = Impact + 40;
	const FProxyAnimPose Joined = M.Evaluate(ReceivedAt - UEntityAnimationComponent::InterpolationDelayMs);
	TestEqual(TEXT("proxy joins the swing mid wind-up"), Joined.State, EProxyAnimState::Attack);
	TestTrue(TEXT("before the impact frame"), Joined.ClipTime < 1.58);

	// The next swing overrides the tail of the previous one.
	M.NoteAttack(Impact + 2000);
	const FProxyAnimPose Second = M.Evaluate(Impact + 2000 - Lead + 100);
	TestEqual(TEXT("second swing restarts the clip"), Second.ClipTime, 0.16, 0.002);

	// Hits on this body flinch it, after its own swing ends and only for FlinchSeconds.
	FProxyAnimStateMachine Target;
	Target.SetTiming(CockatriceTiming());
	Target.NoteHit(5000);
	TestEqual(TEXT("not before the impact"), Target.Evaluate(4999).State, EProxyAnimState::Idle);
	const FProxyAnimPose Flinch = Target.Evaluate(5100);
	TestEqual(TEXT("flinch from the impact"), Flinch.State, EProxyAnimState::Flinch);
	TestEqual(TEXT("flinch clip time"), Flinch.ClipTime, 0.1, 0.001);
	TestEqual(TEXT("flinch ends"), Target.Evaluate(5400).State, EProxyAnimState::Idle);
	Target.NoteAttack(5300);
	TestEqual(TEXT("own swing beats a flinch"), Target.Evaluate(5100).State, EProxyAnimState::Attack);

	FProxyClipTiming NoFlinch = CockatriceTiming();
	NoFlinch.FlinchSeconds = 0.0;
	FProxyAnimStateMachine Stoic;
	Stoic.SetTiming(NoFlinch);
	Stoic.NoteHit(5000);
	TestEqual(TEXT("no flinch clip: keeps locomotion"), Stoic.Evaluate(5100).State, EProxyAnimState::Idle);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAnimStateDeathTest, "Nightfall.Anim.State.DeathAndRespawn", AnimTestFlags)

bool FAnimStateDeathTest::RunTest(const FString& Parameters)
{
	FProxyAnimStateMachine M;
	M.SetTiming(CockatriceTiming());
	M.NoteMove(0, 4.f);
	M.NoteAttack(20000);   // a swing that was in flight when it died
	M.NoteDied(10000);

	TestEqual(TEXT("alive before the death tick"), M.Evaluate(9999).State, EProxyAnimState::Walk);
	const FProxyAnimPose Dying = M.Evaluate(11000);
	TestEqual(TEXT("dying"), Dying.State, EProxyAnimState::Dying);
	TestEqual(TEXT("death clip"), Dying.Clip, EProxyClip::Death);
	TestEqual(TEXT("death clip time"), Dying.ClipTime, 1.0, 0.001);
	const FProxyAnimPose Corpse = M.Evaluate(19500);
	TestEqual(TEXT("corpse after the clip"), Corpse.State, EProxyAnimState::Corpse);
	TestEqual(TEXT("frozen on the last frame"), Corpse.ClipTime, 3.0, 0.001);
	TestEqual(TEXT("death beats a later swing"), M.Evaluate(20000).State, EProxyAnimState::Corpse);
	M.NoteDied(15000);
	TestEqual(TEXT("a repeated EntityDied does not restart the clip"), M.Evaluate(14000).State, EProxyAnimState::Corpse);

	M.NoteMove(30000, 0.f);
	M.NoteAlive(30000);
	// Received early (a proxy renders 150 ms behind): still a corpse until the respawn tick.
	TestEqual(TEXT("corpse until the respawn time"), M.Evaluate(29850).State, EProxyAnimState::Corpse);
	TestTrue(TEXT("dead before the respawn"), M.IsDeadAt(29999));
	TestFalse(TEXT("alive at the respawn"), M.IsDeadAt(30000));
	TestEqual(TEXT("respawned: idle"), M.Evaluate(30100).State, EProxyAnimState::Idle);
	M.NoteAlive(5000);   // an older life's alive fact changes nothing
	M.Prune(30100);
	TestEqual(TEXT("still alive after pruning"), M.Evaluate(30200).State, EProxyAnimState::Idle);
	M.NoteDied(40000);
	TestEqual(TEXT("dies again in the new life"), M.Evaluate(40100).State, EProxyAnimState::Dying);

	FProxyAnimStateMachine Late;
	Late.SetTiming(CockatriceTiming());
	Late.NoteSpawnedDead();
	const FProxyAnimPose LatePose = Late.Evaluate(5);
	TestEqual(TEXT("late AOI entry: already a corpse"), LatePose.State, EProxyAnimState::Corpse);
	TestEqual(TEXT("late AOI entry: last frame"), LatePose.ClipTime, 3.0, 0.001);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAnimComponentEventsTest, "Nightfall.Anim.Component.ServerEvents", AnimTestFlags)

bool FAnimComponentEventsTest::RunTest(const FString& Parameters)
{
	FScopedTestGameInstance Instance;
	UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
	UEntityAnimationComponent* Anim = NewObject<UEntityAnimationComponent>();
	Anim->AnimSet = UEntityAnimationComponent::PresetAnimSet(EEntityAnimPreset::Cockatrice);
	Anim->SetTimingForTesting(CockatriceTiming());

	// The keltir was already in view before its proxy existed, alive.
	FWorldEvent SpawnEvent;
	SpawnEvent.Spawn = SpawnOf(Keltir, 2, TEXT("keltir"), false);
	Net->DispatchServerMessage(Msg(SpawnEvent));
	Anim->Bind(Net, FString(Keltir).ToUpper());   // ids compare case-insensitively
	TestEqual(TEXT("spawned alive: idle"), Anim->EvaluateAt(Anim->RenderTimeMs()).State, EProxyAnimState::Idle);

	// An EntityMove teaches tick -> server time: origin = 1'000'000 ms.
	const int64 Origin = 1000000;
	FWorldEvent Move;
	Move.Move = FEntityMove{ Keltir, {}, { 10.f, 10.f }, 4.f, Origin + 100 * 100, 100 };
	Net->DispatchServerMessage(Msg(Move));
	TestEqual(TEXT("tick time from the move"), Net->TickToServerTimeMs(150), Origin + 15000);
	TestEqual(TEXT("moving: walk"), Anim->EvaluateAt(Origin + 10000).State, EProxyAnimState::Walk);

	// Someone else's events change nothing.
	FWorldEvent OtherDied;
	OtherDied.EntityDied = FEntityDied{ Player, 120, Keltir, 1 };
	Net->DispatchServerMessage(Msg(OtherDied));
	TestEqual(TEXT("another entity's death ignored"), Anim->EvaluateAt(Origin + 12500).State, EProxyAnimState::Walk);

	// It swings at the player (impact tick 130) and the player hits back at 140 (a miss at 145).
	FWorldEvent Swing;
	Swing.AttackResult = FAttackResult{ Keltir, Player, 130, ENetAttackOutcome::Hit, 5, 95, 1 };
	Net->DispatchServerMessage(Msg(Swing));
	const FProxyAnimPose AtImpact = Anim->EvaluateAt(Origin + 13000);
	TestEqual(TEXT("attacker swings"), AtImpact.State, EProxyAnimState::Attack);
	TestEqual(TEXT("impact frame on tick 130"), AtImpact.ClipTime, 1.58, 0.002);

	FWorldEvent Hit;
	Hit.AttackResult = FAttackResult{ Player, Keltir, 160, ENetAttackOutcome::Crit, 30, 14, 1 };
	Net->DispatchServerMessage(Msg(Hit));
	FWorldEvent Miss;
	Miss.AttackResult = FAttackResult{ Player, Keltir, 200, ENetAttackOutcome::Miss, 0, 14, 1 };
	Net->DispatchServerMessage(Msg(Miss));
	TestEqual(TEXT("hit target flinches"), Anim->EvaluateAt(Origin + 16100).State, EProxyAnimState::Flinch);
	TestEqual(TEXT("a miss does not flinch (moving again: walk)"), Anim->EvaluateAt(Origin + 20100).State, EProxyAnimState::Walk);

	// Killed at tick 250: dying from 25000, corpse after the 3 s clip, until the despawn.
	FWorldEvent Died;
	Died.EntityDied = FEntityDied{ Keltir, 250, Player, 1 };
	Net->DispatchServerMessage(Msg(Died));
	TestEqual(TEXT("dying"), Anim->EvaluateAt(Origin + 25500).State, EProxyAnimState::Dying);
	TestEqual(TEXT("corpse"), Anim->EvaluateAt(Origin + 29000).State, EProxyAnimState::Corpse);

	// Render time: proxies 150 ms behind the server, the own pawn at server time.
	TestTrue(TEXT("proxy render delay"), FMath::Abs(Net->EstimatedServerTimeMs() - Anim->RenderTimeMs() - UEntityAnimationComponent::InterpolationDelayMs) <= 2);
	Anim->bRenderAtServerTime = true;
	TestTrue(TEXT("own pawn renders at server time"), FMath::Abs(Net->EstimatedServerTimeMs() - Anim->RenderTimeMs()) <= 2);
	Anim->bRenderAtServerTime = false;

	// Respawned at tick 400: idle again.
	FWorldEvent Respawned;
	Respawned.EntityRespawned = FEntityRespawned{ Keltir, 400, { 100.f, 100.f }, 44 };
	Net->DispatchServerMessage(Msg(Respawned));
	FWorldEvent Stop;
	Stop.Move = FEntityMove{ Keltir, { 100.f, 100.f }, {}, 0.f, Origin + 400 * 100, 400 };
	Net->DispatchServerMessage(Msg(Stop));
	TestEqual(TEXT("respawned: idle"), Anim->EvaluateAt(Origin + 40100).State, EProxyAnimState::Idle);

	// Facts of an earlier life arriving late change nothing: a new life (incarnation 2) spawns...
	FWorldEvent NewLife;
	NewLife.Spawn = SpawnOf(Keltir, 2, TEXT("keltir"), false);
	NewLife.Spawn->LifeIncarnation = 2;
	Net->DispatchServerMessage(Msg(NewLife));
	FWorldEvent OldDeath;
	OldDeath.EntityDied = FEntityDied{ Keltir, 450, Player, 1 };
	Net->DispatchServerMessage(Msg(OldDeath));
	TestEqual(TEXT("an earlier life's death is ignored"), Anim->EvaluateAt(Origin + 46000).State, EProxyAnimState::Idle);
	FWorldEvent OldHit;
	OldHit.AttackResult = FAttackResult{ Player, Keltir, 470, ENetAttackOutcome::Hit, 3, 41, 1 };
	Net->DispatchServerMessage(Msg(OldHit));
	TestEqual(TEXT("a hit on an earlier life does not flinch"), Anim->EvaluateAt(Origin + 47100).State, EProxyAnimState::Idle);
	// ...then dies at 500; a replayed respawn from tick 400 does not revive it.
	FWorldEvent NewDeath;
	NewDeath.EntityDied = FEntityDied{ Keltir, 500, Player, 2 };
	Net->DispatchServerMessage(Msg(NewDeath));
	Net->DispatchServerMessage(Msg(Respawned));
	TestEqual(TEXT("a replayed older respawn is ignored"), Anim->EvaluateAt(Origin + 60000).State, EProxyAnimState::Corpse);

	// A second proxy entering the AOI of a dead keltir starts as a corpse.
	UEntityAnimationComponent* LateAnim = NewObject<UEntityAnimationComponent>();
	FWorldEvent DeadSpawn;
	DeadSpawn.Spawn = SpawnOf(TEXT("0b6e2f6e-0000-4000-8000-0000000000c3"), 2, TEXT("keltir"), true);
	DeadSpawn.Spawn->LifeIncarnation = 2;
	Net->DispatchServerMessage(Msg(DeadSpawn));
	LateAnim->Bind(Net, DeadSpawn.Spawn->EntityId);
	TestEqual(TEXT("late AOI entry corpse"), LateAnim->EvaluateAt(LateAnim->RenderTimeMs()).State, EProxyAnimState::Corpse);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAnimOwnDeathHudTest, "Nightfall.Anim.Component.OwnDeathRespawnHud", AnimTestFlags)

bool FAnimOwnDeathHudTest::RunTest(const FString& Parameters)
{
	// The own pawn and the HUD agree through death and respawn: Dying/Corpse with the dead overlay,
	// then idle, overlay gone, HP from the respawn and no target.
	FScopedTestGameInstance Instance;
	UNetClientSubsystem* Net = Instance.Get<UNetClientSubsystem>();
	UCombatStateSubsystem* Combat = Instance.Get<UCombatStateSubsystem>();
	Net->SetOwnEntityId(Player);
	Net->SetTickTimeOriginForTesting(0);

	FWorldEvent Own;
	Own.Spawn = SpawnOf(Player, 1, TEXT(""), false);
	Own.Spawn->Hp = 120;
	Own.Spawn->MaxHp = 120;
	Net->DispatchServerMessage(Msg(Own));
	FWorldEvent Npc;
	Npc.Spawn = SpawnOf(Keltir, 2, TEXT("keltir"), false);
	Net->DispatchServerMessage(Msg(Npc));
	FWorldEvent Selected;
	Selected.TargetChanged = FTargetChanged{ Player, Keltir };
	Net->DispatchServerMessage(Msg(Selected));
	TestTrue(TEXT("target frame"), Combat->BuildHudModel().bTargetVisible);

	UEntityAnimationComponent* Anim = NewObject<UEntityAnimationComponent>();
	Anim->AnimSet = UEntityAnimationComponent::PresetAnimSet(EEntityAnimPreset::Manny);
	Anim->SetTimingForTesting(CockatriceTiming());
	Anim->bRenderAtServerTime = true;
	Anim->Bind(Net, Net->GetOwnEntityId());

	FWorldEvent Died;
	Died.EntityDied = FEntityDied{ Player, 100, Keltir, 0 };
	Net->DispatchServerMessage(Msg(Died));
	const FCombatHudModel Dead = Combat->BuildHudModel();
	TestTrue(TEXT("dead overlay"), Dead.bDeadOverlay);
	TestFalse(TEXT("own death clears the target"), Dead.bTargetVisible);
	TestEqual(TEXT("own pawn dying"), Anim->EvaluateAt(10100).State, EProxyAnimState::Dying);
	TestEqual(TEXT("own pawn corpse"), Anim->EvaluateAt(20000).State, EProxyAnimState::Corpse);

	FWorldEvent Respawned;
	Respawned.EntityRespawned = FEntityRespawned{ Player, 300, { 126.f, 126.f }, 78 };
	Net->DispatchServerMessage(Msg(Respawned));
	const FCombatHudModel Alive = Combat->BuildHudModel();
	TestFalse(TEXT("overlay cleared"), Alive.bDeadOverlay);
	TestFalse(TEXT("no target after respawn"), Alive.bTargetVisible);
	TestEqual(TEXT("HP from the respawn"), Alive.OwnHpText, FString(TEXT("78 / 120")));
	TestEqual(TEXT("own pawn idle"), Anim->EvaluateAt(30100).State, EProxyAnimState::Idle);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FAnimProxyClassTest, "Nightfall.Anim.ProxyClassByTemplate", AnimTestFlags)

bool FAnimProxyClassTest::RunTest(const FString& Parameters)
{
	UClass* KeltirClass = LoadClass<ARemoteEntityActor>(nullptr, TEXT("/Game/Blueprints/BP_Keltir.BP_Keltir_C"));
	UClass* PlayerClass = LoadClass<ARemoteEntityActor>(nullptr, TEXT("/Game/Blueprints/BP_RemotePlayer.BP_RemotePlayer_C"));
	UClass* RemoteClass = LoadClass<ARemoteEntityActor>(nullptr, TEXT("/Game/Blueprints/BP_RemoteEntity.BP_RemoteEntity_C"));
	UClass* GmClass = LoadClass<ANightfallGameMode>(nullptr, TEXT("/Game/Blueprints/BP_NightfallGameMode.BP_NightfallGameMode_C"));
	if (!TestNotNull(TEXT("BP_Keltir"), KeltirClass) || !TestNotNull(TEXT("BP_RemotePlayer"), PlayerClass)
		|| !TestNotNull(TEXT("BP_RemoteEntity"), RemoteClass) || !TestNotNull(TEXT("BP_NightfallGameMode"), GmClass))
	{
		return false;
	}
	const ANightfallGameMode* Gm = GetDefault<ANightfallGameMode>(GmClass);
	const TSubclassOf<ARemoteEntityActor>* Mapped = Gm->TemplateClasses.Find(TEXT("keltir"));
	TestTrue(TEXT("game mode maps keltir to BP_Keltir"), Mapped != nullptr && Mapped->Get() == KeltirClass);
	TestEqual(TEXT("game mode player proxy"), Gm->PlayerClass.Get(), PlayerClass);
	TestEqual(TEXT("BP_Keltir uses the cockatrice"), GetDefault<ARemoteEntityActor>(KeltirClass)->Animation->AnimSet.Mesh,
		UEntityAnimationComponent::PresetAnimSet(EEntityAnimPreset::Cockatrice).Mesh);
	TestEqual(TEXT("BP_RemotePlayer uses Manny"), GetDefault<ARemoteEntityActor>(PlayerClass)->Animation->AnimSet.Mesh,
		UEntityAnimationComponent::PresetAnimSet(EEntityAnimPreset::Manny).Mesh);
	TestTrue(TEXT("placeholder kept for clones without the art"), GetDefault<ARemoteEntityActor>(KeltirClass)->Body->GetStaticMesh() != nullptr);

	UWorldProxySubsystem* Proxies = NewObject<UWorldProxySubsystem>();
	Proxies->EntityClass = RemoteClass;
	Proxies->PlayerClass = PlayerClass;
	Proxies->TemplateClasses = Gm->TemplateClasses;
	TestEqual(TEXT("keltir npc"), Proxies->ClassFor(SpawnOf(Keltir, 2, TEXT("keltir"), false)).Get(), KeltirClass);
	TestEqual(TEXT("template id case-insensitive"), Proxies->ClassFor(SpawnOf(Keltir, 2, TEXT("Keltir"), false)).Get(), KeltirClass);
	TestEqual(TEXT("unknown template"), Proxies->ClassFor(SpawnOf(Keltir, 2, TEXT("wolf"), false)).Get(), RemoteClass);
	TestEqual(TEXT("fixture npc without template"), Proxies->ClassFor(SpawnOf(Keltir, 2, TEXT(""), false)).Get(), RemoteClass);
	TestEqual(TEXT("player"), Proxies->ClassFor(SpawnOf(Player, 1, TEXT(""), false)).Get(), PlayerClass);
	Proxies->PlayerClass = nullptr;
	TestEqual(TEXT("player without a player class"), Proxies->ClassFor(SpawnOf(Player, 1, TEXT(""), false)).Get(), RemoteClass);
	return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FVendorArtTest, "Nightfall.Content.VendorArt", AnimTestFlags)

bool FVendorArtTest::RunTest(const FString& Parameters)
{
	// Scripts/import-vendor.sh output: mesh + the four required clips per body, on a real mesh
	// component with the native anim instance. Lists every clip with its length.
	if (!FPackageName::DoesPackageExist(TEXT("/Game/Vendor/Cockatrice/SK_Cockatrice")))
	{
		AddInfo(TEXT("SKIPPED: Content/Vendor not imported (run moon run client-unreal:import-vendor)"));
		return true;
	}
	FScopedTestGameInstance Instance;
	UWorld* World = Instance.GameInstance->GetWorld();
	if (!TestNotNull(TEXT("test world"), World)) return false;
	AActor* Host = World->SpawnActor<AActor>();
	struct FBody { const TCHAR* Name; EEntityAnimPreset Preset; bool bRun; };
	for (const FBody& Body : { FBody{ TEXT("cockatrice"), EEntityAnimPreset::Cockatrice, false }, FBody{ TEXT("manny"), EEntityAnimPreset::Manny, true } })
	{
		const FEntityAnimSet Set = UEntityAnimationComponent::PresetAnimSet(Body.Preset);
		for (const TPair<const TCHAR*, FSoftObjectPath>& Clip : TArray<TPair<const TCHAR*, FSoftObjectPath>>{
			{ TEXT("idle"), Set.Idle }, { TEXT("walk"), Set.Walk }, { TEXT("attack"), Set.Attack }, { TEXT("death"), Set.Death }, { TEXT("run"), Set.Run } })
		{
			if (Clip.Value.IsNull())
			{
				TestFalse(FString::Printf(TEXT("%s has a run clip"), Body.Name), Body.bRun && FCString::Strcmp(Clip.Key, TEXT("run")) == 0);
				continue;
			}
			const UAnimSequenceBase* Sequence = Cast<UAnimSequenceBase>(Clip.Value.TryLoad());
			if (TestNotNull(FString::Printf(TEXT("%s %s %s"), Body.Name, Clip.Key, *Clip.Value.ToString()), Sequence))
			{
				AddInfo(FString::Printf(TEXT("%s %-6s %s %.3f s"), Body.Name, Clip.Key, *Clip.Value.ToString(), Sequence->GetPlayLength()));
				TestTrue(FString::Printf(TEXT("%s %s has length"), Body.Name, Clip.Key), Sequence->GetPlayLength() > 0.1f);
			}
		}
		const UAnimSequenceBase* Attack = Cast<UAnimSequenceBase>(Set.Attack.TryLoad());
		TestTrue(FString::Printf(TEXT("%s impact inside the attack clip"), Body.Name), Attack && Set.AttackImpactSeconds < Attack->GetPlayLength());

		UEntityAnimationComponent* Anim = NewObject<UEntityAnimationComponent>();
		Anim->AnimSet = Set;
		USkeletalMeshComponent* Mesh = NewObject<USkeletalMeshComponent>(Host);
		Mesh->RegisterComponent();   // an anim instance exists only on a registered component
		if (TestTrue(FString::Printf(TEXT("%s applies to a mesh"), Body.Name), Anim->ApplyToMesh(Mesh)))
		{
			UEntityAnimInstance* AnimInstance = Anim->GetAnimInstance();
			if (TestNotNull(TEXT("native anim instance"), AnimInstance))
			{
				TestNotNull(TEXT("idle clip set"), AnimInstance->GetClip(EProxyClip::Idle));
				TestNotNull(TEXT("death clip set"), AnimInstance->GetClip(EProxyClip::Death));
			}
		}
	}

	// NatureLite (optional): the L_TestZone props resolve.
	UWorld* TestZone = LoadObject<UWorld>(nullptr, TEXT("/Game/Maps/L_TestZone.L_TestZone"));
	if (TestNotNull(TEXT("L_TestZone"), TestZone))
	{
		int32 Props = 0, Loadable = 0;
		for (const AActor* Actor : TestZone->PersistentLevel->Actors)
		{
			if (const AVendorPropScatter* Scatter = Cast<AVendorPropScatter>(Actor))
			{
				for (const FVendorProp& Prop : Scatter->Props)
				{
					++Props;
					Loadable += Prop.Mesh.TryLoad() != nullptr;
				}
			}
		}
		TestTrue(TEXT("L_TestZone has props"), Props > 0);
		if (FPackageName::DoesPackageExist(TEXT("/Game/Vendor/NatureLite/SM_Tree_01")))
		{
			TestEqual(TEXT("every prop mesh loads"), Loadable, Props);
		}
	}
	return true;
}

#endif
