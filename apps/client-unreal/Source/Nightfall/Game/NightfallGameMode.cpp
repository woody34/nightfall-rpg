#include "NightfallGameMode.h"
#include "NightfallCharacter.h"
#include "NightfallLoginPlayerController.h"
#include "NightfallPlayerController.h"
#include "World/RemoteEntityActor.h"
#include "World/WorldProxySubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Engine/GameInstance.h"
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
		Proxies->PlayerClass = PlayerClass;
		Proxies->TemplateClasses.Reset();
		for (const TPair<FString, TSubclassOf<ARemoteEntityActor>>& Entry : TemplateClasses)
		{
			Proxies->TemplateClasses.Add(Entry.Key.ToLower(), Entry.Value);
		}
	}
}

ANightfallLoginGameMode::ANightfallLoginGameMode()
{
	PlayerControllerClass = ANightfallLoginPlayerController::StaticClass();
	DefaultPawnClass = nullptr;
}

APawn* ANightfallGameMode::SpawnDefaultPawnAtTransform_Implementation(AController* NewPlayer, const FTransform& SpawnTransform)
{
	FTransform Transform = SpawnTransform;
	if (const UNetClientSubsystem* Net = GetGameInstance()->GetSubsystem<UNetClientSubsystem>())
	{
		if (const FEntitySpawn* Own = Net->GetKnownEntities().Find(Net->GetOwnEntityId()))
		{
			constexpr double UnitsPerTile = 100.0;
			FVector Location = Transform.GetLocation();
			Location.X = Own->Position.X * UnitsPerTile;
			Location.Y = Own->Position.Y * UnitsPerTile;
			Transform.SetLocation(Location);
		}
	}
	return Super::SpawnDefaultPawnAtTransform_Implementation(NewPlayer, Transform);
}
