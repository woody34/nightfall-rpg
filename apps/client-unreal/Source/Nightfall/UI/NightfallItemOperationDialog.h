#pragma once
#include "CoreMinimal.h"
#include "CommonActivatableWidget.h"
#include "NightfallItemOperationDialog.generated.h"
class UInventoryStateSubsystem;
class UComboBoxString;
class UTextBlock;
class UButton;
/** Enchant, crystallize and blacksmith forms select received instances/recipes. */
UCLASS()
class NIGHTFALL_API UNightfallItemOperationDialog : public UCommonActivatableWidget
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(UInventoryStateSubsystem* State,bool Blacksmith=false,uint64 Selected=0);
 void Render();
 bool CanConfirm() const;
 const FString& RetryKey() const { return Key; }
 UButton* RetryControl() const { return RetryButton; }
 UButton* ConfirmControl() const { return ConfirmButton; }
 UComboBoxString* ActionControl() const { return Action; }
 UComboBoxString* TargetControl() const { return Target; }
 UComboBoxString* IngredientControl() const { return Ingredient; }
protected:
 virtual void NativeDestruct() override;
 virtual TOptional<FUIInputConfig> GetDesiredInputConfig() const override;
private:
 UFUNCTION() void SelectionChanged(FString Option,ESelectInfo::Type Type);
 UFUNCTION() void Confirm();
 UFUNCTION() void Retry();
 UFUNCTION() void Close();
 UPROPERTY() TObjectPtr<UInventoryStateSubsystem> State;
 UPROPERTY() TObjectPtr<UComboBoxString> Action;
 UPROPERTY() TObjectPtr<UComboBoxString> Target;
 UPROPERTY() TObjectPtr<UComboBoxString> Ingredient;
 UPROPERTY() TObjectPtr<UComboBoxString> Support;
 UPROPERTY() TObjectPtr<UTextBlock> Status;
 UPROPERTY() TObjectPtr<UButton> ConfirmButton;
 UPROPERTY() TObjectPtr<UButton> RetryButton;
 TMap<FString,uint64> Instances, Recipes;
 TArray<FString> FrozenTokens;
 FString Key;
 bool bBlacksmith=false, bRendering=false;
 FDelegateHandle Changed;
};
