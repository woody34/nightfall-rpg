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

	// The socket connects before this map loads (login -> connect -> travel), so the spawns for
	// everything already around us arrived while no world was listening.
	for (const TPair<FString, FEntitySpawn>& Known : Net->GetKnownEntities())
	{
		HandleSpawn(Known.Value);
	}
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
	// The player's own entity is the controlled pawn (UOwnEntityComponent), not a proxy.
	const UGameInstance* GI = World ? World->GetGameInstance() : nullptr;
	const UNetClientSubsystem* Net = GI ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr;
	if (Net && Net->IsOwnEntity(Spawn.EntityId)) return;
	const TSubclassOf<ARemoteEntityActor> Class = ClassFor(Spawn);
	if (!World || !Class) 
	{
		UE_LOG(LogNightfall, Warning, TEXT("EntityClass not set; cannot spawn %s"), *Spawn.EntityId);
		return;
	}
	ARemoteEntityActor* Actor = World->SpawnActor<ARemoteEntityActor>(Class);
	if (!Actor) return;
	Actor->EntityId = Spawn.EntityId;
	Actor->DisplayName = Spawn.Name;
	Actor->Bind(World->GetGameInstance()->GetSubsystem<UNetClientSubsystem>());
	Entities.Add(Spawn.EntityId, Actor);
}

TSubclassOf<ARemoteEntityActor> UWorldProxySubsystem::ClassFor(const FEntitySpawn& Spawn) const
{
	constexpr uint32 KindPlayer = 1, KindNpc = 2;
	if (Spawn.Kind == KindNpc)
	{
		if (const TSubclassOf<ARemoteEntityActor>* Found = TemplateClasses.Find(Spawn.TemplateId.ToLower()))
		{
			if (*Found) return *Found;
		}
	}
	if (Spawn.Kind == KindPlayer && PlayerClass) return PlayerClass;
	return EntityClass;
}

void UWorldProxySubsystem::HandleDespawn(const FEntityDespawn& Despawn)
{
	if (TObjectPtr<ARemoteEntityActor>* Found = Entities.Find(Despawn.EntityId))
	{
		if (*Found) (*Found)->Destroy();
		Entities.Remove(Despawn.EntityId);
	}
}
