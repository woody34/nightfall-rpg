#pragma once

#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Net/SessionClientSubsystem.h"
#include "Net/ProtoCodec.h"
#include "ClassStateSubsystem.generated.h"

class ULoginFlowSubsystem;
class UNetClientSubsystem;

DECLARE_MULTICAST_DELEGATE(FOnClassStateChanged);

/** Server catalogue and character identity projection. Contains no derived-stat formulas. */
UCLASS()
class NIGHTFALL_API UClassStateSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()
public:
	using FDone = TFunction<void(const FNetResult&)>;
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;
	virtual void Deinitialize() override;

	void LoadCatalogue(FDone Done = nullptr);
	void Create(const FGrpcNightfallV1CreateCharacterRequest& Request, FDone Done = nullptr);
	void LoadOptions(FDone Done = nullptr);
	void Transfer(uint32 TargetClassId, FDone Done = nullptr);
	void RefreshCharacter(FDone Done = nullptr);
	void EnterCreated();

	const FGrpcNightfallV1ListClassesResponse& GetCatalogue() const { return Catalogue; }
	const FGrpcNightfallV1TransferOptionsResponse& GetOptions() const { return Options; }
	const FGrpcNightfallV1Character& GetLastCreated() const { return LastCreated; }
	const FGrpcNightfallV1Character& GetCharacter() const { return Character; }
	const FNetResult& GetLastResult() const { return LastResult; }
	const FString& GetLastOperation() const { return LastOperation; }
	bool IsBusy() const { return bBusy; }
	bool HasCatalogue() const { return !Catalogue.DataVersion.IsEmpty(); }
	const TOptional<uint32>& GetOwnClassId() const { return OwnClass; }
	int32 GetCreationCount() const { return CreationCount; }
	int32 GetTransferCount() const { return TransferCount; }
	const TArray<FString>& GetLastGrantedSkillKeys() const { return LastGrantedSkillKeys; }
	int32 GetObservedTransferCount() const { return ObservedTransferCount; }
	int32 GetPrivateStatsLeakCount() const { return PrivateStatsLeakCount; }
	float GetOwnMoveSpeed() const { return OwnMoveSpeed; }
	const FGrpcNightfallV1ClassInfo* FindClass(uint32 ClassId) const;
	const FGrpcNightfallV1RaceInfo* FindRace(uint32 RaceId) const;
	TArray<uint32> PathTo(uint32 ClassId) const;
	FString OwnClassLabel() const;
	bool IsAtMaster() const;
	bool CreatedMatchesCatalogue() const;

	/** Projection inputs are public for typed-event automation tests. */
	void ApplyOptions(const FGrpcNightfallV1TransferOptionsResponse& Response);
	bool HasOptions() const { return bHasOptions; }
	void ApplyCatalogue(const FGrpcNightfallV1ListClassesResponse& Response);
	void ApplySpawn(const FEntitySpawn& Spawn);
	void ApplyClassChanged(const FClassChanged& Changed);
	UFUNCTION() void ResetAdmission();
	void ResetAccount();
	UFUNCTION() void ResetWorld(const FString& Reason);
	FOnClassStateChanged OnChanged;

private:
	friend class FClassRequestLifecycleTest;
	void CompleteOptions(uint64 Serial, const FNetResult& Result, const FGrpcNightfallV1TransferOptionsResponse& Response, FDone Done);
	uint64 Begin(const FString& Operation);
	void Complete(uint64 Serial, const FNetResult& Result, FDone Done);
	FString SelectedId() const;
	UPROPERTY() TObjectPtr<ULoginFlowSubsystem> Flow;
	UPROPERTY() TObjectPtr<UNetClientSubsystem> Net;
	FGrpcNightfallV1ListClassesResponse Catalogue;
	FGrpcNightfallV1TransferOptionsResponse Options;
	FGrpcNightfallV1Character LastCreated;
	FGrpcNightfallV1Character Character;
	TArray<FString> LastGrantedSkillKeys;
	FNetResult LastResult;
	FString LastOperation;
	bool bBusy = false;
	bool bHasOptions = false;
	uint64 OperationSerial = 0;
	int32 CreationCount = 0;
	int32 TransferCount = 0;
	TOptional<uint32> OwnClass;
	uint64 ClassTick = 0;
	uint32 Generation = 0;
	int32 ObservedTransferCount = 0;
	int32 PrivateStatsLeakCount = 0;
	float OwnMoveSpeed = 0.f;
	FDelegateHandle SpawnHandle, ClassHandle, StatsHandle, MoveHandle;
};
