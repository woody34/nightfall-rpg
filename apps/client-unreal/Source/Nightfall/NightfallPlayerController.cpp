#include "NightfallPlayerController.h"
#include "Net/NetClientSubsystem.h"
#include "EnhancedInputComponent.h"
#include "EnhancedInputSubsystems.h"
#include "Blueprint/AIBlueprintHelperLibrary.h"
#include "Engine/GameInstance.h"

ANightfallPlayerController::ANightfallPlayerController()
{
	bShowMouseCursor = true;
	DefaultMouseCursor = EMouseCursor::Default;
}

void ANightfallPlayerController::BeginPlay()
{
	Super::BeginPlay();
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
	FHitResult Hit;
	if (!GetHitResultUnderCursor(ECC_Visibility, true, Hit)) return;

	// Local preview: start walking now. The server's answer is authoritative and will correct us.
	UAIBlueprintHelperLibrary::SimpleMoveToLocation(this, Hit.Location);

	if (UNetClientSubsystem* Net = GetGameInstance()->GetSubsystem<UNetClientSubsystem>())
	{
		Net->SendMoveTo(FNetVec2{ static_cast<float>(Hit.Location.X / UnitsPerTile),
		                          static_cast<float>(Hit.Location.Y / UnitsPerTile) });
	}
}
