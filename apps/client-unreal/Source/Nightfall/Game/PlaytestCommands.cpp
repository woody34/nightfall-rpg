// Developer console commands for the scripted playtest (README "Art and animation"). Non-shipping.
//
//   nf.Status                  logs the HUD model, the own pawn's and every proxy's animation state
//   nf.Respawn                 the dead overlay's Respawn button
//   nf.Playtest hunter [kills] attacks the nearest live keltir (the click path) until [kills]
//                              EntityDied facts it caused, then quits
//   nf.Playtest victim         walks into the keltir meadow, never fights back, respawns when
//                              killed, then quits once alive again
// Both first walk to the meadow in hops (a fresh character starts at tile (0, 0); one MoveTo may
// cover at most 64 tiles, and the keltirs at (100, 100) are outside its area of interest). While
// not attacking they re-send a MoveTo to their own tile every 20 s: the server closes a socket
// that sends nothing for 60 s (4408 idle timeout) and the client has no keep-alive of its own.
// Each step and every animation state change of proxies and the pawn is logged with "playtest:"
// and a screenshot is requested at the interesting moments (Saved/Screenshots/Linux/playtest-*.png).

#include "CoreMinimal.h"

#if !UE_BUILD_SHIPPING

#include "Nightfall.h"
#include "Anim/EntityAnimationComponent.h"
#include "Combat/CombatStateSubsystem.h"
#include "Game/NightfallCharacter.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallPlayerController.h"
#include "World/RemoteEntityActor.h"
#include "World/WorldProxySubsystem.h"
#include "Containers/Ticker.h"
#include "Engine/Engine.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "HAL/IConsoleManager.h"
#include "Kismet/GameplayStatics.h"
#include "Misc/DateTime.h"
#include "UnrealClient.h"

namespace
{
	UWorld* GameWorld(const TWeakObjectPtr<UGameInstance>& GI)
	{
		return GI.IsValid() ? GI->GetWorld() : nullptr;
	}

	ANightfallCharacter* OwnPawn(UWorld* World)
	{
		APlayerController* PC = World ? World->GetFirstPlayerController() : nullptr;
		return PC ? Cast<ANightfallCharacter>(PC->GetPawn()) : nullptr;
	}

	void Shot(const FString& Label)
	{
		const FString Name = FString::Printf(TEXT("playtest-%s-%s"), *Label, *FDateTime::Now().ToString(TEXT("%H%M%S")));
		FScreenshotRequest::RequestScreenshot(Name, true, false);   // with the HUD (dead overlay, target frame)
		UE_LOG(LogNightfall, Display, TEXT("playtest: screenshot %s"), *Name);
	}

	FString StatusLine(UGameInstance* GI)
	{
		UWorld* World = GI->GetWorld();
		const UCombatStateSubsystem* Combat = GI->GetSubsystem<UCombatStateSubsystem>();
		const FCombatHudModel M = Combat->BuildHudModel();
		FString Line = FString::Printf(TEXT("own %s %s HP %s %s target=%s%s attack=%s"),
			*M.OwnName, *M.LevelText, *M.OwnHpText, *M.XpText,
			M.bTargetVisible ? *M.TargetName : TEXT("-"), M.bTargetVisible ? *(TEXT(" ") + M.TargetHpText) : TEXT(""),
			M.AttackText.IsEmpty() ? TEXT("idle") : *M.AttackText);
		if (M.bDeadOverlay) Line += TEXT(" DEAD-OVERLAY");
		if (const ANightfallCharacter* Pawn = OwnPawn(World))
		{
			Line += FString::Printf(TEXT(" pawn=%s%s"), LexToString(Pawn->Animation->GetCurrentState()),
				Pawn->Animation->GetAnimInstance() ? TEXT("(manny)") : TEXT("(placeholder)"));
		}
		if (const UWorldProxySubsystem* Proxies = World ? World->GetSubsystem<UWorldProxySubsystem>() : nullptr)
		{
			TMap<FString, int32> Counts;
			for (const TPair<FString, TObjectPtr<ARemoteEntityActor>>& P : Proxies->GetProxies())
			{
				if (!P.Value) continue;
				const FString Kind = P.Value->GetClass()->GetName().Replace(TEXT("_C"), TEXT(""));
				Counts.FindOrAdd(FString::Printf(TEXT("%s%s:%s"), *Kind, P.Value->HasAnimatedBody() ? TEXT("") : TEXT("(placeholder)"),
					LexToString(P.Value->Animation->GetCurrentState())))++;
			}
			Counts.KeySort(TLess<FString>());
			for (const TPair<FString, int32>& C : Counts) Line += FString::Printf(TEXT(" %s=%d"), *C.Key, C.Value);
		}
		return Line;
	}

	/** One running playtest per process; ticks every half second on the core ticker. */
	struct FPlaytest
	{
		enum class ERole : uint8 { Hunter, Victim };
		ERole Role = ERole::Hunter;
		int32 KillsWanted = 3;
		int32 Kills = 0;
		int32 Deaths = 0;
		bool bRespawned = false;
		int32 Hop = 0;
		double HopStarted = 0.0;
		double LastKeepAlive = 0.0;
		double Started = 0.0;
		double DeadSince = 0.0;
		double DoneAt = 0.0;
		uint64 LastXp = 0;
		FString LastLine;
		TMap<FString, EProxyAnimState> LastStates;
		TWeakObjectPtr<UGameInstance> GI;
		FTSTicker::FDelegateHandle Ticker;
		FDelegateHandle DiedHandle, XpHandle, LevelHandle, RespawnedHandle;

		void Start()
		{
			Started = FPlatformTime::Seconds();
			UNetClientSubsystem* Net = GI->GetSubsystem<UNetClientSubsystem>();
			DiedHandle = Net->OnEntityDied.AddLambda([this](const FEntityDied& D)
			{
				UNetClientSubsystem* N = GI.IsValid() ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr;
				if (!N) return;
				if (N->IsOwnEntity(D.Killer))
				{
					++Kills;
					UE_LOG(LogNightfall, Display, TEXT("playtest: KILL %d %s at tick %llu"), Kills, *D.Entity, D.Tick);
					Shot(FString::Printf(TEXT("kill%d"), Kills));
				}
				if (N->IsOwnEntity(D.Entity))
				{
					++Deaths;
					DeadSince = FPlatformTime::Seconds();
					UE_LOG(LogNightfall, Display, TEXT("playtest: OWN DEATH %d at tick %llu (killer %s)"), Deaths, D.Tick, *D.Killer);
				}
			});
			XpHandle = Net->OnXpGained.AddLambda([](const FXpGained& X)
			{
				UE_LOG(LogNightfall, Display, TEXT("playtest: XP +%llu total %llu"), X.Amount, X.Total);
			});
			LevelHandle = Net->OnLevelUp.AddLambda([](const FLevelUp& L)
			{
				UE_LOG(LogNightfall, Display, TEXT("playtest: LEVEL UP %s -> %u"), *L.Entity, L.Level);
			});
			RespawnedHandle = Net->OnEntityRespawned.AddLambda([this](const FEntityRespawned& R)
			{
				UNetClientSubsystem* N = GI.IsValid() ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr;
				if (N && N->IsOwnEntity(R.Entity))
				{
					bRespawned = true;
					UE_LOG(LogNightfall, Display, TEXT("playtest: OWN RESPAWN hp %u at (%.0f, %.0f)"), R.Hp, R.Position.X, R.Position.Y);
				}
			});
			Ticker = FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateLambda([this](float) { return Tick(); }), 0.5f);
		}

		void LogAnimChanges(UWorld* World)
		{
			auto Note = [this](const FString& Who, EProxyAnimState State)
			{
				EProxyAnimState* Last = LastStates.Find(Who);
				if (Last && *Last == State) return;
				UE_LOG(LogNightfall, Display, TEXT("playtest: anim %s %s -> %s"), *Who, Last ? LexToString(*Last) : TEXT("(new)"), LexToString(State));
				LastStates.Add(Who, State);
			};
			if (const ANightfallCharacter* Pawn = OwnPawn(World)) Note(TEXT("own-pawn"), Pawn->Animation->GetCurrentState());
			if (const UWorldProxySubsystem* Proxies = World->GetSubsystem<UWorldProxySubsystem>())
			{
				for (const TPair<FString, TObjectPtr<ARemoteEntityActor>>& P : Proxies->GetProxies())
				{
					if (P.Value) Note(P.Value->GetClass()->GetName().Replace(TEXT("_C"), TEXT("")) + TEXT(" ") + P.Key.Left(8), P.Value->Animation->GetCurrentState());
				}
			}
		}

		bool Tick()
		{
			UWorld* World = GameWorld(GI);
			if (!World) return Finish(TEXT("game instance gone"));
			if (FPlatformTime::Seconds() - Started > 600.0) return Finish(TEXT("TIMEOUT after 600 s"));
			UCombatStateSubsystem* Combat = GI->GetSubsystem<UCombatStateSubsystem>();
			ANightfallCharacter* Pawn = OwnPawn(World);
			if (!Pawn || !World->GetSubsystem<UWorldProxySubsystem>()) return true;   // still logging in / travelling

			LogAnimChanges(World);
			const FString Line = StatusLine(GI.Get());
			if (Line != LastLine)
			{
				UE_LOG(LogNightfall, Display, TEXT("playtest: %s"), *Line);
				LastLine = Line;
			}

			if (DoneAt > 0.0)
			{
				return FPlatformTime::Seconds() - DoneAt < 4.0 ? true : Finish(TEXT("PASS"));
			}

			if (Combat->IsOwnDead())
			{
				// Let the death clip play and the overlay show before pressing Respawn.
				if (DeadSince == 0.0) DeadSince = FPlatformTime::Seconds();
				if (FPlatformTime::Seconds() - DeadSince > 4.0 && !Combat->IsRespawnPending())
				{
					Shot(TEXT("dead"));
					UE_LOG(LogNightfall, Display, TEXT("playtest: pressing Respawn (seq %u)"), Combat->RequestRespawn());
				}
				return true;
			}
			if (Role == ERole::Victim)
			{
				if (bRespawned && Deaths > 0)
				{
					const FCombatHudModel M = Combat->BuildHudModel();
					UE_LOG(LogNightfall, Display, TEXT("playtest: after respawn overlay=%d target=%d hp=%s"), M.bDeadOverlay, M.bTargetVisible, *M.OwnHpText);
					Shot(TEXT("respawned"));
					DoneAt = FPlatformTime::Seconds();
					return true;
				}
				if (Walk(World, Pawn)) KeepAlive(World, Pawn);
				return true;
			}

			// Hunter.
			if (!Walk(World, Pawn)) return true;
			if (Kills >= KillsWanted)
			{
				Shot(TEXT("done"));
				DoneAt = FPlatformTime::Seconds();
				return true;
			}
			if (Combat->GetAttackState() == EAttackState::Idle)
			{
				const UWorldProxySubsystem* Proxies = World->GetSubsystem<UWorldProxySubsystem>();
				const FVector Me = Pawn->GetActorLocation();
				FString Best;
				double BestDist = TNumericLimits<double>::Max();
				for (const TPair<FString, TObjectPtr<ARemoteEntityActor>>& P : Proxies->GetProxies())
				{
					const FCombatEntity* E = Combat->FindEntity(P.Key);
					if (!P.Value || !E || E->bDead || !Combat->IsAttackable(P.Key)) continue;
					const double D = FVector::Dist2D(Me, P.Value->GetActorLocation());
					if (D < BestDist) { BestDist = D; Best = P.Key; }
				}
				if (!Best.IsEmpty())
				{
					UE_LOG(LogNightfall, Display, TEXT("playtest: attacking %s at %.0f cm"), *Best, BestDist);
					Combat->ClickEntity(Best);
					LastKeepAlive = FPlatformTime::Seconds();
				}
				else
				{
					KeepAlive(World, Pawn);
				}
			}
			return true;
		}

		/** Walks the hops to the meadow; true once there. */
		bool Walk(UWorld* World, const APawn* Pawn)
		{
			static const FVector2D Hops[] = { { 45.0, 45.0 }, { 90.0, 90.0 }, { 98.0, 98.0 } };
			if (Hop >= UE_ARRAY_COUNT(Hops)) return true;
			const FVector2D Goal = Hops[Hop];
			const FVector2D At(Pawn->GetActorLocation().X / 100.0, Pawn->GetActorLocation().Y / 100.0);
			const double Now = FPlatformTime::Seconds();
			if (HopStarted > 0.0 && (FVector2D::Distance(At, Goal) < 2.0 || Now - HopStarted > 25.0))
			{
				++Hop;
				HopStarted = 0.0;
				return Hop >= UE_ARRAY_COUNT(Hops);
			}
			if (HopStarted == 0.0)
			{
				HopStarted = Now;
				LastKeepAlive = Now;
				UE_LOG(LogNightfall, Display, TEXT("playtest: walking to (%.0f, %.0f)"), Goal.X, Goal.Y);
				IConsoleManager::Get().ProcessUserConsoleInput(*FString::Printf(TEXT("nf.ClickMove %.0f %.0f"), Goal.X, Goal.Y), *GLog, World);
			}
			return false;
		}

		void KeepAlive(UWorld* World, const APawn* Pawn)
		{
			const double Now = FPlatformTime::Seconds();
			if (Now - LastKeepAlive < 20.0) return;
			LastKeepAlive = Now;
			const FVector At = Pawn->GetActorLocation();
			IConsoleManager::Get().ProcessUserConsoleInput(*FString::Printf(TEXT("nf.ClickMove %.1f %.1f"), At.X / 100.0, At.Y / 100.0), *GLog, World);
		}

		~FPlaytest() { Teardown(); }

		/** Removes the ticker and every delegate (they capture this). Idempotent. */
		void Teardown()
		{
			if (Ticker.IsValid())
			{
				FTSTicker::GetCoreTicker().RemoveTicker(Ticker);
				Ticker.Reset();
			}
			if (UNetClientSubsystem* Net = GI.IsValid() ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr)
			{
				Net->OnEntityDied.Remove(DiedHandle);
				Net->OnXpGained.Remove(XpHandle);
				Net->OnLevelUp.Remove(LevelHandle);
				Net->OnEntityRespawned.Remove(RespawnedHandle);
			}
			DiedHandle.Reset();
			XpHandle.Reset();
			LevelHandle.Reset();
			RespawnedHandle.Reset();
		}

		bool Finish(const TCHAR* Outcome)
		{
			UE_LOG(LogNightfall, Display, TEXT("playtest: RESULT %s role=%s kills=%d deaths=%d respawned=%d"), Outcome,
				Role == ERole::Hunter ? TEXT("hunter") : TEXT("victim"), Kills, Deaths, bRespawned);
			// Returning false removes the ticker; drop the handle so Teardown does not remove it twice.
			Ticker.Reset();
			Teardown();
			// The console manager only knows cvars and console objects; "quit" is an engine exec.
			FPlatformMisc::RequestExit(false, TEXT("nf.Playtest finished"));
			return false;
		}
	};

	TUniquePtr<FPlaytest> Running;

	FAutoConsoleCommandWithWorldAndArgs PlaytestCommand(TEXT("nf.Playtest"), TEXT("nf.Playtest hunter [kills=3] | victim"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& Args, UWorld* World)
		{
			if (!World || !World->GetGameInstance()) return;
			Running.Reset();   // a previous run's ticker and delegates go first
			Running = MakeUnique<FPlaytest>();
			Running->GI = World->GetGameInstance();
			Running->Role = Args.Num() > 0 && Args[0] == TEXT("victim") ? FPlaytest::ERole::Victim : FPlaytest::ERole::Hunter;
			if (Args.Num() > 1) Running->KillsWanted = FMath::Max(1, FCString::Atoi(*Args[1]));
			UE_LOG(LogNightfall, Display, TEXT("playtest: start %s"), Args.Num() > 0 ? *Args[0] : TEXT("hunter"));
			Running->Start();
		}));

	FAutoConsoleCommandWithWorld StatusCommand(TEXT("nf.Status"), TEXT("Log the HUD model and animation states"),
		FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
		{
			if (World && World->GetGameInstance()) UE_LOG(LogNightfall, Display, TEXT("nf.Status: %s"), *StatusLine(World->GetGameInstance()));
		}));

	FAutoConsoleCommandWithWorld RespawnCommand(TEXT("nf.Respawn"), TEXT("The dead overlay's Respawn button"),
		FConsoleCommandWithWorldDelegate::CreateLambda([](UWorld* World)
		{
			UGameInstance* GI = World ? World->GetGameInstance() : nullptr;
			if (UCombatStateSubsystem* Combat = GI ? GI->GetSubsystem<UCombatStateSubsystem>() : nullptr)
			{
				UE_LOG(LogNightfall, Display, TEXT("nf.Respawn: seq %u"), Combat->RequestRespawn());
			}
		}));
}

#endif
