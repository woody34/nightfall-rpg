#include "NightfallPlayerController.h"
#include "Net/NetClientSubsystem.h"
#include "EnhancedInputComponent.h"
#include "EnhancedInputSubsystems.h"
#include "InputAction.h"
#include "InputMappingContext.h"
#include "Blueprint/AIBlueprintHelperLibrary.h"
#include "Nightfall.h"
#include "Game/OwnEntityComponent.h"
#include "Containers/Ticker.h"
#include "Engine/Engine.h"
#include "NavigationSystem.h"
#include "Engine/GameInstance.h"
#include "GameFramework/Pawn.h"

ANightfallPlayerController::ANightfallPlayerController()
{
	bShowMouseCursor = true;
	DefaultMouseCursor = EMouseCursor::Default;
}

void ANightfallPlayerController::BeginPlay()
{
	Super::BeginPlay();
	// The login screen leaves a UI-only input mode behind; click-to-move needs the game to see clicks.
	FInputModeGameAndUI Mode;
	Mode.SetLockMouseToViewportBehavior(EMouseLockMode::DoNotLock);
	Mode.SetHideCursorDuringCapture(false);
	SetInputMode(Mode);
	UE_LOG(LogNightfall, Log, TEXT("click-to-move ready: mapping context %s, action %s"),
		*GetNameSafe(DefaultMappingContext), *GetNameSafe(ClickMoveAction));
	if (ULocalPlayer* LP = GetLocalPlayer())
	{
		if (UEnhancedInputLocalPlayerSubsystem* Input = LP->GetSubsystem<UEnhancedInputLocalPlayerSubsystem>())
		{
			if (DefaultMappingContext) Input->AddMappingContext(DefaultMappingContext, 0);
		}
	}
}

void ANightfallPlayerController::SetupInputComponent()
{
	Super::SetupInputComponent();
	if (UEnhancedInputComponent* EIC = Cast<UEnhancedInputComponent>(InputComponent))
	{
		if (ClickMoveAction) EIC->BindAction(ClickMoveAction, ETriggerEvent::Started, this, &ANightfallPlayerController::OnClickMove);
	}
}

void ANightfallPlayerController::OnClickMove()
{
	UE_LOG(LogNightfall, Log, TEXT("click"));
	FHitResult Hit;
	FVector Target;
	if (GetHitResultUnderCursor(ECC_Visibility, true, Hit))
	{
		Target = Hit.Location;
	}
	else
	{
		// Nothing with collision under the cursor (the engine plane has none): use the ground plane.
		FVector Origin, Direction;
		if (!DeprojectMousePositionToWorld(Origin, Direction) || FMath::IsNearlyZero(Direction.Z)) return;
		const double GroundZ = GetPawn() ? GetPawn()->GetActorLocation().Z - GroundOffset : 0.0;
		const double T = (GroundZ - Origin.Z) / Direction.Z;
		if (T <= 0.0) return;
		Target = Origin + Direction * T;
	}
	MoveToWorldLocation(Target);
}

uint32 ANightfallPlayerController::MoveToWorldLocation(const FVector& Target)
{
	// Local preview: start walking now. The server's answer is authoritative and will correct us.
	UAIBlueprintHelperLibrary::SimpleMoveToLocation(this, Target);
	UNavigationSystemV1* Nav = FNavigationSystem::GetCurrent<UNavigationSystemV1>(GetWorld());
	if (Nav == nullptr || Nav->GetDefaultNavDataInstance(FNavigationSystem::DontCreate) == nullptr)
	{
		UE_LOG(LogNightfall, Warning, TEXT("no nav mesh in this level: the local preview cannot walk; server corrections will move the pawn"));
	}

	uint32 Seq = 0;
	if (UNetClientSubsystem* Net = GetGameInstance()->GetSubsystem<UNetClientSubsystem>())
	{
		// World cm -> server tiles.
		const FNetVec2 Tile{ static_cast<float>(Target.X / UnitsPerTile), static_cast<float>(Target.Y / UnitsPerTile) };
		Seq = Net->SendMoveTo(Tile);
		UE_LOG(LogNightfall, Log, TEXT("click-to-move: (%.0f, %.0f) cm -> MoveTo seq %u tile (%.2f, %.2f)"),
			Target.X, Target.Y, Seq, Tile.X, Tile.Y);
	}
	if (const APawn* P = GetPawn())
	{
		if (UOwnEntityComponent* Own = P->FindComponentByClass<UOwnEntityComponent>())
		{
			Own->NotePreview(Seq, Target);
		}
	}
	return Seq;
}

#if !UE_BUILD_SHIPPING
namespace
{
	// nf.ClickMove <tileX> <tileY> [delaySeconds]: the click path without a mouse (headless -game
	// runs). With a delay it waits (the nav mesh builds after the map loads), clicks, then logs
	// where the pawn is 5 s later.
	FAutoConsoleCommandWithWorldAndArgs ClickMoveCommand(TEXT("nf.ClickMove"), TEXT("nf.ClickMove <tileX> <tileY> [delaySeconds]"),
		FConsoleCommandWithWorldAndArgsDelegate::CreateLambda([](const TArray<FString>& Args, UWorld* World)
		{
			if (World == nullptr || Args.Num() < 2) return;
			const FVector Target(FCString::Atof(*Args[0]) * 100.0, FCString::Atof(*Args[1]) * 100.0, 0.0);
			const float Delay = Args.Num() > 2 ? FCString::Atof(*Args[2]) : 0.f;
			FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateLambda([Target](float)
			{
				// Look the world up when the delay ends: the login flow travels maps in between.
				UWorld* Current = GEngine != nullptr && GEngine->GetWorldContexts().Num() > 0 ? GEngine->GetWorldContexts()[0].World() : nullptr;
				ANightfallPlayerController* PC = Current != nullptr ? Cast<ANightfallPlayerController>(Current->GetFirstPlayerController()) : nullptr;
				if (PC == nullptr) return false;
				PC->MoveToWorldLocation(Target);
				FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateLambda([PC = TWeakObjectPtr<ANightfallPlayerController>(PC)](float)
				{
					if (PC.IsValid() && PC->GetPawn()) UE_LOG(LogNightfall, Log, TEXT("nf.ClickMove: pawn now at %s"), *PC->GetPawn()->GetActorLocation().ToString());
					return false;
				}), 5.f);
				return false;
			}), Delay);
		}));
}
#endif
