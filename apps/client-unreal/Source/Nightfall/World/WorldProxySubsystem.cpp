#include "WorldProxySubsystem.h"
#include "RemoteEntityActor.h"
#include "Net/NetClientSubsystem.h"
#include "Nightfall.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"

void UWorldProxySubsystem::OnWorldBeginPlay(UWorld& InWorld)
{
	Super::OnWorldBeginPlay(InWorld);
	UGameInstance* GI = InWorld.GetGameInstance();
	UNetClientSubsystem* Net = GI ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr;
	if (!Net) return;
	SpawnHandle = Net->OnEntitySpawn.AddUObject(this, &UWorldProxySubsystem::HandleSpawn);
	DespawnHandle = Net->OnEntityDespawn.AddUObject(this, &UWorldProxySubsystem::HandleDespawn);
}

void UWorldProxySubsystem::Deinitialize()
{
	if (UWorld* World = GetWorld())
	{
		if (UGameInstance* GI = World->GetGameInstance())
		{
			if (UNetClientSubsystem* Net = GI->GetSubsystem<UNetClientSubsystem>())
			{
				Net->OnEntitySpawn.Remove(SpawnHandle);
				Net->OnEntityDespawn.Remove(DespawnHandle);
			}
		}
	}
	Super::Deinitialize();
}

void UWorldProxySubsystem::HandleSpawn(const FEntitySpawn& Spawn)
{
	if (Entities.Contains(Spawn.EntityId)) return;
	UWorld* World = GetWorld();
	if (!World || !EntityClass) 
	{
		UE_LOG(LogNightfall, Warning, TEXT("EntityClass not set; cannot spawn %s"), *Spawn.EntityId);
		return;
	}
	ARemoteEntityActor* Actor = World->SpawnActor<ARemoteEntityActor>(EntityClass);
	if (!Actor) return;
	Actor->EntityId = Spawn.EntityId;
	Actor->DisplayName = Spawn.Name;
	Actor->Bind(World->GetGameInstance()->GetSubsystem<UNetClientSubsystem>());
	Entities.Add(Spawn.EntityId, Actor);
}

void UWorldProxySubsystem::HandleDespawn(const FEntityDespawn& Despawn)
{
	if (TObjectPtr<ARemoteEntityActor>* Found = Entities.Find(Despawn.EntityId))
	{
		if (*Found) (*Found)->Destroy();
		Entities.Remove(Despawn.EntityId);
	}
}
