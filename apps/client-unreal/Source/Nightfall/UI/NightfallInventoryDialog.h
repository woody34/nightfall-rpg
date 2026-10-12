#pragma once
#include "CoreMinimal.h"
#include "CommonActivatableWidget.h"
#include "NightfallInventoryDialog.generated.h"
class UInventoryStateSubsystem;
class UVerticalBox;
class UUniformGridPanel;
class UTextBlock;
class UButton;
class UProgressBar;
class UComboBoxString;
class UNightfallInventoryDialog;
UCLASS()
class NIGHTFALL_API UNightfallItemChoice : public UObject
{
 GENERATED_BODY()
public:
 uint64 Id=0;
 FString Slot;
 TWeakObjectPtr<UNightfallInventoryDialog> Dialog;
 UFUNCTION() void Click();
};
UCLASS()
class NIGHTFALL_API UNightfallInventoryDialog : public UCommonActivatableWidget
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(UInventoryStateSubsystem* State,bool Refresh=true);
 void ShowDetached(const FString& Character);
 void Render();
 void Select(uint64 Instance);
 void SelectSlot(const FString& Slot);
 FString ItemLabel(uint64 Instance) const;
 FString Tooltip(uint32 Template) const;
 UButton* EquipControl() const { return EquipButton; }
 UButton* UseControl() const { return UseButton; }
 UButton* UnequipControl() const { return UnequipButton; }
 UButton* EnchantControl() const { return EnchantButton; }
 UProgressBar* WeightControl() const { return Weight; }
 UUniformGridPanel* GridControl() const { return Grid; }
 const FString& RetryKey() const { return Key; }
protected:
 virtual void NativeOnInitialized() override;
 virtual void NativeDestruct() override;
 virtual TOptional<FUIInputConfig> GetDesiredInputConfig() const override;
private:
 UFUNCTION() void Equip();
 UFUNCTION() void Unequip();
 UFUNCTION() void Use();
 UFUNCTION() void Enchant();
 UFUNCTION() void Blacksmith();
 UFUNCTION() void Assign();
 UFUNCTION() void ClearSlot();
 UFUNCTION() void Refresh();
 UFUNCTION() void Close();
 UPROPERTY() TObjectPtr<UInventoryStateSubsystem> State;
 UPROPERTY() TObjectPtr<UUniformGridPanel> Grid;
 UPROPERTY() TObjectPtr<UVerticalBox> Paperdoll;
 UPROPERTY() TObjectPtr<UTextBlock> Status;
 UPROPERTY() TObjectPtr<UTextBlock> Summary;
 UPROPERTY() TObjectPtr<UTextBlock> Selection;
 UPROPERTY() TObjectPtr<UProgressBar> Weight;
 UPROPERTY() TObjectPtr<UButton> EquipButton;
 UPROPERTY() TObjectPtr<UButton> UnequipButton;
 UPROPERTY() TObjectPtr<UButton> UseButton;
 UPROPERTY() TObjectPtr<UButton> EnchantButton;
 UPROPERTY() TObjectPtr<UButton> BlacksmithButton;
 UPROPERTY() TObjectPtr<UComboBoxString> SlotPicker;
 UPROPERTY() TObjectPtr<UComboBoxString> ModePicker;
 UPROPERTY() TArray<TObjectPtr<UNightfallItemChoice>> Choices;
 uint64 Selected=0;
 FString SelectedSlot, Key, DetachedCharacter;
 FDelegateHandle Changed;
};
