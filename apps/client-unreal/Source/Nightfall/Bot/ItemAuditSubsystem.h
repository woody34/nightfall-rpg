#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "Net/ProtoCodec.h"
#include "ItemAuditSubsystem.generated.h"
class UNetClientSubsystem;
class UInventoryStateSubsystem;
/** Sticky audit of accepted owner receipts and their received inventory consequences. */
UCLASS()
class NIGHTFALL_API UItemAuditSubsystem : public UGameInstanceSubsystem
{
 GENERATED_BODY()
public:
 virtual void Initialize(FSubsystemCollectionBase& C) override;
 virtual void Deinitialize() override;
 UFUNCTION() void Reset();
 UFUNCTION() void Disconnected(const FString& Reason) {Reset();}
 void Observe(const FWorldEvent& E);
 TOptional<bool> Check(FString& Detail) const;
private:
 UPROPERTY() TObjectPtr<UNetClientSubsystem> Net;
 UPROPERTY() TObjectPtr<UInventoryStateSubsystem> State;
 TArray<FString> Violations;
 bool bObserved=false;
};
