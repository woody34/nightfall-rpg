#include "NightfallGameMode.h"
#include "NightfallCharacter.h"
#include "NightfallLoginPlayerController.h"
#include "NightfallPlayerController.h"
#include "World/RemoteEntityActor.h"
#include "World/WorldProxySubsystem.h"
#include "Engine/World.h"

ANightfallGameMode::ANightfallGameMode()
{
	PlayerControllerClass = ANightfallPlayerController::StaticClass();
	DefaultPawnClass = ANightfallCharacter::StaticClass();
	EntityClass = ARemoteEntityActor::StaticClass();
}

void ANightfallGameMode::InitGame(const FString& MapName, const FString& Options, FString& ErrorMessage)
{
	Super::InitGame(MapName, Options, ErrorMessage);
	if (UWorldProxySubsystem* Proxies = GetWorld()->GetSubsystem<UWorldProxySubsystem>())
	{
		Proxies->EntityClass = EntityClass;
	}
}

ANightfallLoginGameMode::ANightfallLoginGameMode()
{
	PlayerControllerClass = ANightfallLoginPlayerController::StaticClass();
	DefaultPawnClass = nullptr;
}
