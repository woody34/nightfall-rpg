#pragma once

#include "CoreMinimal.h"
#include "GameFramework/PlayerController.h"
#include "NightfallLoginPlayerController.generated.h"

class UNightfallLoginScreen;

/** Shows the login screen on L_Login. The class comes from config so no Blueprint is needed here. */
UCLASS(Config = Game)
class NIGHTFALL_API ANightfallLoginPlayerController : public APlayerController
{
	GENERATED_BODY()

public:
	ANightfallLoginPlayerController();

protected:
	virtual void BeginPlay() override;

	/** [/Script/Nightfall.NightfallLoginPlayerController] LoginScreenClass; falls back to the C++ screen. */
	UPROPERTY(Config, EditDefaultsOnly, Category = "Nightfall|UI")
	TSoftClassPtr<UNightfallLoginScreen> LoginScreenClass;

private:
	UPROPERTY()
	TObjectPtr<UNightfallLoginScreen> Screen;
};
