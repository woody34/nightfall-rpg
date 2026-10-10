#include "ClassStateSubsystem.h"
#include "Game/LoginFlowSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "Engine/GameInstance.h"

void UClassStateSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	Flow = Collection.InitializeDependency<ULoginFlowSubsystem>();
	Net = Collection.InitializeDependency<UNetClientSubsystem>();
	if (Net)
	{
		SpawnHandle = Net->OnEntitySpawn.AddUObject(this, &UClassStateSubsystem::ApplySpawn);
		ClassHandle = Net->OnClassChanged.AddUObject(this, &UClassStateSubsystem::ApplyClassChanged);
		StatsHandle = Net->OnStatsChanged.AddLambda([this](const FStatsChanged& Stats) { if (!Net->IsOwnEntity(Stats.Entity)) ++PrivateStatsLeakCount; });
		MoveHandle = Net->OnEntityMove.AddLambda([this](const FEntityMove& Move) { if (Net->IsOwnEntity(Move.EntityId)) OwnMoveSpeed = Move.Speed; });
		Net->OnConnected.AddDynamic(this, &UClassStateSubsystem::ResetAdmission);
		Net->OnDisconnected.AddDynamic(this, &UClassStateSubsystem::ResetWorld);
	}
}

void UClassStateSubsystem::Deinitialize()
{
	++OperationSerial;
	if (Net)
	{
		Net->OnEntitySpawn.Remove(SpawnHandle);
		Net->OnClassChanged.Remove(ClassHandle);
		Net->OnStatsChanged.Remove(StatsHandle);
		Net->OnEntityMove.Remove(MoveHandle);
		Net->OnDisconnected.RemoveAll(this);
		Net->OnConnected.RemoveAll(this);
	}
	Super::Deinitialize();
}

uint64 UClassStateSubsystem::Begin(const FString& Operation)
{
	LastOperation = Operation;
	LastResult = FNetResult();
	bBusy = true;
	return ++OperationSerial;
}

void UClassStateSubsystem::Complete(uint64 Serial, const FNetResult& Result, FDone Done)
{
	if (Serial != OperationSerial) return;
	bBusy = false;
	LastResult = Result;
	OnChanged.Broadcast();
	if (Done) Done(Result);
}

void UClassStateSubsystem::LoadCatalogue(FDone Done)
{
	if (!Flow || bBusy) return;
	const uint64 Serial = Begin(TEXT("catalogue"));
	Flow->ListClasses([Weak = TWeakObjectPtr<UClassStateSubsystem>(this), Serial, Done](const FNetResult& R, const FGrpcNightfallV1ListClassesResponse& Response)
	{
		if (!Weak.IsValid() || Serial != Weak->OperationSerial) return;
		if (R.IsOk()) Weak->ApplyCatalogue(Response);
		Weak->Complete(Serial, R, Done);
	});
}

void UClassStateSubsystem::ApplyCatalogue(const FGrpcNightfallV1ListClassesResponse& Response)
{
	// A version describes the entire catalogue; replace it atomically rather than mixing rows.
	Catalogue = Response;
	OnChanged.Broadcast();
}

void UClassStateSubsystem::Create(const FGrpcNightfallV1CreateCharacterRequest& Request, FDone Done)
{
	if (!Flow || bBusy) return;
	const uint64 Serial = Begin(TEXT("create"));
	Flow->CreateCharacter(Request, [Weak = TWeakObjectPtr<UClassStateSubsystem>(this), Serial, Done](const FNetResult& R, const FGrpcNightfallV1Character& Created)
	{
		if (!Weak.IsValid() || Serial != Weak->OperationSerial) return;
		if (R.IsOk()) { Weak->LastCreated = Created; ++Weak->CreationCount; }
		Weak->Complete(Serial, R, Done);
	});
}

FString UClassStateSubsystem::SelectedId() const { return Flow ? Flow->GetSelectedCharacterId() : FString(); }

void UClassStateSubsystem::LoadOptions(FDone Done)
{
	if (!Flow || bBusy) return;
	const uint64 Serial = Begin(TEXT("options"));
	Options = FGrpcNightfallV1TransferOptionsResponse();
	bHasOptions = false;
	Flow->TransferOptions(SelectedId(), [Weak = TWeakObjectPtr<UClassStateSubsystem>(this), Serial, Done](const FNetResult& R, const FGrpcNightfallV1TransferOptionsResponse& Response)
	{
		if (Weak.IsValid()) Weak->CompleteOptions(Serial, R, Response, Done);
	});
}

void UClassStateSubsystem::Transfer(uint32 TargetClassId, FDone Done, const FString& IdempotencyKey)
{
	if (!Flow || bBusy) return;
	const uint64 Serial = Begin(TEXT("transfer"));
	Flow->ChangeClass(SelectedId(), TargetClassId, [Weak = TWeakObjectPtr<UClassStateSubsystem>(this), Serial, Done](const FNetResult& R, const FGrpcNightfallV1ChangeClassResponse& Response)
	{
		if (!Weak.IsValid() || Serial != Weak->OperationSerial) return;
		Weak->CompleteTransfer(Serial, R, Response, Done);
	}, IdempotencyKey);
}

void UClassStateSubsystem::CompleteTransfer(uint64 Serial, const FNetResult& Result, const FGrpcNightfallV1ChangeClassResponse& Response, FDone Done)
{
	if (Serial != OperationSerial) return;
	if (Result.IsOk())
	{
		// An idempotent receipt is immutable: a replay may describe an earlier class and token
		// balance. Keep it separate from GetCharacter, options, and live world/resource facts.
		LastTransferResponse = Response;
		LastGrantedSkillKeys = Response.GrantedSkillKeys;
		++TransferCount;
	}
	Complete(Serial, Result, Done);
}

void UClassStateSubsystem::RefreshCharacter(FDone Done)
{
	if (!Flow || bBusy) return;
	const uint64 Serial = Begin(TEXT("character"));
	Flow->GetCharacter(SelectedId(), [Weak = TWeakObjectPtr<UClassStateSubsystem>(this), Serial, Done](const FNetResult& R, const FGrpcNightfallV1Character& Response)
	{
		if (!Weak.IsValid() || Serial != Weak->OperationSerial) return;
		if (R.IsOk()) Weak->Character = Response;
		Weak->Complete(Serial, R, Done);
	});
}

void UClassStateSubsystem::EnterCreated()
{
	if (Flow && !LastCreated.Id.IsEmpty()) Flow->EnterWorld(LastCreated.Id, [](const FNetResult&) {});
}

const FGrpcNightfallV1ClassInfo* UClassStateSubsystem::FindClass(uint32 ClassId) const
{
	for (const FGrpcNightfallV1ClassInfo& C : Catalogue.Classes) if (C.ClassId.Value == ClassId) return &C;
	return nullptr;
}

const FGrpcNightfallV1RaceInfo* UClassStateSubsystem::FindRace(uint32 RaceId) const
{
	for (const FGrpcNightfallV1RaceInfo& R : Catalogue.Races) if (static_cast<uint32>(R.Race) == RaceId) return &R;
	return nullptr;
}

TArray<uint32> UClassStateSubsystem::PathTo(uint32 ClassId) const
{
	TArray<uint32> Path;
	const FGrpcNightfallV1ClassInfo* Current = FindClass(ClassId);
	while (Current && !Path.Contains(Current->ClassId.Value))
	{
		Path.Insert(Current->ClassId.Value, 0);
		Current = FindClass(Current->ParentClassId.Value);
	}
	return Path;
}

FString UClassStateSubsystem::OwnClassLabel() const
{
	const FGrpcNightfallV1ClassInfo* C = OwnClass.IsSet() ? FindClass(OwnClass.GetValue()) : nullptr;
	return C ? C->DisplayName : FString(TEXT("Class --"));
}

bool UClassStateSubsystem::IsAtMaster() const
{
	if (!Net || !HasCatalogue()) return false;
	const FEntitySpawn* Own = Net->GetKnownEntities().Find(Net->GetOwnEntityId());
	// Movement samples are authoritative and supersede the admission position.
	FNetVec2 Position;
	if (!Own || !Net->Snapshots().Sample(Net->GetOwnEntityId(), Net->EstimatedServerTimeMs(), Position)) return false;
	const FGrpcNightfallV1ClassMasterInfo& M = Catalogue.ClassMaster;
	return FVector2D::Distance(FVector2D(Position.X, Position.Y), FVector2D(M.Position.X, M.Position.Y)) <= M.InteractionRadius;
}

bool UClassStateSubsystem::CreatedMatchesCatalogue() const
{
	const FGrpcNightfallV1ClassInfo* C = LastCreated.Id.IsEmpty() ? nullptr : FindClass(LastCreated.ClassId.Value);
	if (!C || C->Race != LastCreated.Race || C->Tier.Value != 0) return false;
	const auto& A = C->BaseStats;
	const auto& B = LastCreated.Stats;
	return A.Str.Value == B.Str.Value && A.Dex.Value == B.Dex.Value && A.Con.Value == B.Con.Value
		&& A.Int.Value == B.Int.Value && A.Wit.Value == B.Wit.Value && A.Men.Value == B.Men.Value;
}

void UClassStateSubsystem::ApplySpawn(const FEntitySpawn& Spawn)
{
	if (!Net || !Net->IsOwnEntity(Spawn.EntityId)) return;
	if (OwnClass.IsSet() && (Spawn.SessionGeneration < Generation || (Spawn.SessionGeneration == Generation && Spawn.StateTick < ClassTick))) return;
	OwnClass = Spawn.ClassId;
	ClassTick = Spawn.StateTick;
	Generation = Spawn.SessionGeneration;
	OnChanged.Broadcast();
}

void UClassStateSubsystem::ApplyClassChanged(const FClassChanged& Changed)
{
	if (Net && !Net->IsOwnEntity(Changed.Entity)) ++ObservedTransferCount;
	if (!Net || !Net->IsOwnEntity(Changed.Entity) || !OwnClass.IsSet() || Changed.SessionGeneration != Generation || Changed.Tick < ClassTick) return;
	const bool bAdvanced = OwnClass.GetValue() != Changed.ClassId;
	if (bAdvanced) ++OwnClassEventCount;
	OwnClass = Changed.ClassId;
	if (bAdvanced && Flow) Flow->SetStatus(TEXT("Class advanced: ") + OwnClassLabel());
	ClassTick = Changed.Tick;
	OnChanged.Broadcast();
}

void UClassStateSubsystem::ResetAdmission() { ResetWorld(TEXT("New admission")); }

void UClassStateSubsystem::ResetWorld(const FString& Reason)
{
	++OperationSerial;
	bBusy = false;
	OwnClass.Reset();
	ClassTick = 0;
	Generation = 0;
	Options = FGrpcNightfallV1TransferOptionsResponse();
	bHasOptions = false;
	ObservedTransferCount = 0;
	OwnClassEventCount = 0;
	PrivateStatsLeakCount = 0;
	OwnMoveSpeed = 0.f;
	OnChanged.Broadcast();
}

void UClassStateSubsystem::CompleteOptions(uint64 Serial, const FNetResult& Result, const FGrpcNightfallV1TransferOptionsResponse& Response, FDone Done)
{
	if (Serial != OperationSerial) return;
	if (Result.IsOk()) ApplyOptions(Response);
	Complete(Serial, Result, Done);
}

void UClassStateSubsystem::ResetAccount()
{
	ResetWorld(TEXT("Left account"));
	LastCreated = FGrpcNightfallV1Character();
	Character = FGrpcNightfallV1Character();
	LastTransferResponse = FGrpcNightfallV1ChangeClassResponse();
	CreationCount = 0;
	TransferCount = 0;
	LastGrantedSkillKeys.Reset();
	LastOperation.Empty();
	OnChanged.Broadcast();
}

void UClassStateSubsystem::ApplyOptions(const FGrpcNightfallV1TransferOptionsResponse& Response)
{
	Options = Response;
	bHasOptions = true;
	OnChanged.Broadcast();
}
