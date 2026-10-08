#include "NightfallLoginPlayerController.h"
#include "Nightfall.h"
#include "UI/NightfallLoginScreen.h"
#include "Blueprint/UserWidget.h"

ANightfallLoginPlayerController::ANightfallLoginPlayerController()
{
	bShowMouseCursor = true;
}

void ANightfallLoginPlayerController::BeginPlay()
{
	Super::BeginPlay();
	if (!IsLocalController())
	{
		return;
	}
	TSubclassOf<UNightfallLoginScreen> Class = LoginScreenClass.LoadSynchronous();
	if (Class == nullptr)
	{
		UE_LOG(LogNightfall, Warning, TEXT("LoginScreenClass not set or missing; using the default layout"));
		Class = UNightfallLoginScreen::StaticClass();
	}
	Screen = CreateWidget<UNightfallLoginScreen>(this, Class);
	if (Screen == nullptr)
	{
		return;
	}
	Screen->AddToViewport();
	Screen->ActivateWidget();

	FInputModeUIOnly InputMode;
	InputMode.SetWidgetToFocus(Screen->TakeWidget());
	InputMode.SetLockMouseToViewportBehavior(EMouseLockMode::DoNotLock);
	SetInputMode(InputMode);
}
