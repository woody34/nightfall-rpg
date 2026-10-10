#include "WorldProxySubsystem.h"
#include "RemoteEntityActor.h"
#include "ClassMasterActor.h"
#include "Character/ClassStateSubsystem.h"
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
	if (UClassStateSubsystem* Classes = GI->GetSubsystem<UClassStateSubsystem>()) CatalogueHandle = Classes->OnChanged.AddUObject(this, &UWorldProxySubsystem::RefreshClassTitles);
	SpawnHandle = Net->OnEntitySpawn.AddUObject(this, &UWorldProxySubsystem::HandleSpawn);
	ClassHandle = Net->OnClassChanged.AddUObject(this, &UWorldProxySubsystem::HandleClassChanged);
	DespawnHandle = Net->OnEntityDespawn.AddUObject(this, &UWorldProxySubsystem::HandleDespawn);
	Net->OnConnected.AddDynamic(this, &UWorldProxySubsystem::ResetProxies);
	Net->OnDisconnected.AddDynamic(this, &UWorldProxySubsystem::HandleDisconnected);

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
			if (UClassStateSubsystem* Classes = GI->GetSubsystem<UClassStateSubsystem>()) Classes->OnChanged.Remove(CatalogueHandle);
			if (UNetClientSubsystem* Net = GI->GetSubsystem<UNetClientSubsystem>())
			{
				Net->OnEntitySpawn.Remove(SpawnHandle);
				Net->OnEntityDespawn.Remove(DespawnHandle);
				Net->OnClassChanged.Remove(ClassHandle);
				Net->OnConnected.RemoveDynamic(this, &UWorldProxySubsystem::ResetProxies);
				Net->OnDisconnected.RemoveDynamic(this, &UWorldProxySubsystem::HandleDisconnected);
			}
		}
	}
	Super::Deinitialize();
}

void UWorldProxySubsystem::ResetProxies()
{
	// Every new admission resends AOI state, possibly with IDs from a different zone epoch.
	for (const auto& Pair : Entities) if (IsValid(Pair.Value.Get())) Pair.Value->Destroy();
	Entities.Reset();
}

void UWorldProxySubsystem::HandleDisconnected(const FString& Reason)
{
	ResetProxies();
}

void UWorldProxySubsystem::HandleSpawn(const FEntitySpawn& Spawn)
{
	if (const auto* Existing = Entities.Find(Spawn.EntityId); Existing && *Existing)
	{
		if (ClassFor(Spawn) == AClassMasterActor::StaticClass() && !Cast<AClassMasterActor>(Existing->Get()))
		{
			// Catalogue can arrive after admission. Promote exactly this authoritative proxy.
			(*Existing)->Destroy(); Entities.Remove(Spawn.EntityId);
		}
		else
		{
			(*Existing)->DisplayName = Spawn.Name;
			(*Existing)->ClearTransferCue();
			if (auto* Master = Cast<AClassMasterActor>(Existing->Get())) Master->Configure(Spawn.Name, FVector2D(Spawn.Position.X, Spawn.Position.Y));
			if (Spawn.Kind == 1) UpdateClassPresentation(Existing->Get(), Spawn.ClassId, false);
			return;
		}
	}
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
	if (auto* Master = Cast<AClassMasterActor>(Actor)) Master->Configure(Spawn.Name, FVector2D(Spawn.Position.X, Spawn.Position.Y));
	Actor->Bind(World->GetGameInstance()->GetSubsystem<UNetClientSubsystem>());
	if (Spawn.Kind == 1) UpdateClassPresentation(Actor, Spawn.ClassId, false);
	Entities.Add(Spawn.EntityId, Actor);
}

TSubclassOf<ARemoteEntityActor> UWorldProxySubsystem::ClassFor(const FEntitySpawn& Spawn) const
{
	constexpr uint32 KindPlayer = 1, KindNpc = 2;
	const auto* GI = GetWorld() ? GetWorld()->GetGameInstance() : nullptr;
	const auto* Classes = GI ? GI->GetSubsystem<UClassStateSubsystem>() : nullptr;
	if (Spawn.Kind == KindNpc && !Spawn.bCombatant && !Spawn.bAttackable && Classes && Classes->HasCatalogue() && Spawn.Name == Classes->GetCatalogue().ClassMaster.Name) return AClassMasterActor::StaticClass();
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

void UWorldProxySubsystem::UpdateClassPresentation(ARemoteEntityActor* Actor, uint32 ClassId, bool bTransferred)
{
	const UGameInstance* GI = GetWorld() ? GetWorld()->GetGameInstance() : nullptr;
	const UClassStateSubsystem* Classes = GI ? GI->GetSubsystem<UClassStateSubsystem>() : nullptr;
	const auto* Info = Classes ? Classes->FindClass(ClassId) : nullptr;
	if (Actor) Actor->SetClassPresentation(ClassId, Info ? Info->DisplayName : FString::Printf(TEXT("Class %u"), ClassId), bTransferred);
}

void UWorldProxySubsystem::HandleClassChanged(const FClassChanged& Changed)
{
	if (const auto* Actor = Entities.Find(Changed.Entity); Actor && *Actor) UpdateClassPresentation(Actor->Get(), Changed.ClassId, true);
}

void UWorldProxySubsystem::RefreshClassTitles()
{
	const UGameInstance* GI = GetWorld() ? GetWorld()->GetGameInstance() : nullptr;
	const UNetClientSubsystem* Net = GI ? GI->GetSubsystem<UNetClientSubsystem>() : nullptr;
	TArray<FEntitySpawn> Promotions;
	for (const auto& Pair : Entities)
	{
		const FEntitySpawn* Spawn = Net ? Net->GetKnownEntities().Find(Pair.Key) : nullptr;
		if (Pair.Value && Spawn && Spawn->Kind == 1) UpdateClassPresentation(Pair.Value.Get(), Spawn->ClassId, false);
		if (Pair.Value && Spawn && ClassFor(*Spawn) == AClassMasterActor::StaticClass() && !Cast<AClassMasterActor>(Pair.Value.Get())) Promotions.Add(*Spawn);
	}
	for (const auto& Spawn : Promotions) HandleSpawn(Spawn);
}
