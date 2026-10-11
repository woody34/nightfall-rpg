#pragma once
#include "CoreMinimal.h"
#include "CommonActivatableWidget.h"
#include "NightfallSkillDialog.generated.h"
class USkillStateSubsystem;
class UVerticalBox;
class UTextBlock;
class UButton;
class UComboBoxString;
class UPanelWidget;
class UNightfallSkillDialog;
UCLASS()
class NIGHTFALL_API UNightfallSkillChoice : public UObject
{
 GENERATED_BODY()
public:
 uint32 Id = 0, Rank = 0;
 TWeakObjectPtr<UNightfallSkillDialog> Dialog;
 UFUNCTION() void Click();
};
/** Trainer choices and costs are received RPC metadata; no locally inferred eligibility. */
UCLASS()
class NIGHTFALL_API UNightfallSkillDialog : public UCommonActivatableWidget
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(USkillStateSubsystem* State, bool bRefresh = true);
 void Render();
 void Select(uint32 Id, uint32 Rank);
 bool CanConfirm() const;
 UComboBoxString* HotbarSkillControl() const { return HotbarSkillPicker; }
 UComboBoxString* HotbarSlotControl() const { return HotbarSlotPicker; }
 UButton* AssignControl() const { return AssignButton; }
 const FString& GetRetryKey() const { return RetryKey; }
protected:
 virtual void NativeOnInitialized() override;
 virtual void NativeDestruct() override;
 virtual TOptional<FUIInputConfig> GetDesiredInputConfig() const override;
private:
 UTextBlock* Text(UPanelWidget* Parent, const FString& Label);
 UButton* Button(UPanelWidget* Parent, const FString& Label);
 UFUNCTION() void Refresh();
 UFUNCTION() void AssignHotbar();
 UFUNCTION() void ClearHotbar();
 UFUNCTION() void Confirm();
 UFUNCTION() void Close();
 UPROPERTY() TObjectPtr<USkillStateSubsystem> State;
 UPROPERTY() TObjectPtr<UVerticalBox> Rows;
 UPROPERTY() TObjectPtr<UComboBoxString> HotbarSkillPicker;
 UPROPERTY() TObjectPtr<UComboBoxString> HotbarSlotPicker;
 UPROPERTY() TObjectPtr<UButton> AssignButton;
 TArray<FString> HotbarOptions;
 UPROPERTY() TObjectPtr<UTextBlock> Status;
 UPROPERTY() TObjectPtr<UTextBlock> Selection;
 UPROPERTY() TObjectPtr<UButton> ConfirmButton;
 UPROPERTY() TArray<TObjectPtr<UNightfallSkillChoice>> Choices;
 uint32 SelectedId = 0, SelectedRank = 0;
 FString RetryKey;
 FDelegateHandle Changed;
};
