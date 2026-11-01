#pragma once
#include "CoreMinimal.h"
#include "CommonActivatableWidget.h"
#include "Net/ProtoCodec.h"
#include "NightfallEconomyDialog.generated.h"
class UEconomyStateSubsystem;
class UInventoryStateSubsystem;
class UVerticalBox;
class UComboBoxString;
class UEditableTextBox;
class UTextBlock;
class UButton;

/**
 * Story 5.3: the common form of the economy dialogs (vendor, multisell, recipe book). One request key
 * per frozen request: a changed selection or a confirmed request makes a new key, and Retry resends
 * the last frozen request with its key (D43, `UEconomyStateSubsystem::Submit`). The dialogs show the
 * received server values only: no client-side price, count or chance math.
 */
UCLASS(Abstract)
class NIGHTFALL_API UNightfallEconomyDialog : public UCommonActivatableWidget
{
 GENERATED_BODY()
public:
 const FString& RetryKey() const { return Key; }
 UButton* ConfirmControl() const { return ConfirmButton; }
 UButton* RetryControl() const { return RetryButton; }
 UTextBlock* StatusControl() const { return Status; }
 virtual void Render() PURE_VIRTUAL(UNightfallEconomyDialog::Render,);
protected:
 virtual void NativeDestruct() override;
 virtual TOptional<FUIInputConfig> GetDesiredInputConfig() const override;
 void Bind(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory);
 UVerticalBox* BeginLayout(const TCHAR* Title);
 UTextBlock* AddText(const FString& Text);
 UComboBoxString* AddChoice(const TCHAR* Label);
 UEditableTextBox* AddNumber(const TCHAR* Label, const TCHAR* Default);
 UButton* AddButton(const TCHAR* Label);
 void AddCommonButtons(const TCHAR* ConfirmLabel);
 /** The count typed in a number box; unset when it is not a positive integer. */
 static TOptional<uint64> PositiveNumber(const UEditableTextBox* Box);
 /** Freeze `Body` under a new key (or the current key when nothing was sent with it yet) and send. */
 void Send(EGrpcNightfallV1ItemOperation Op, const FString& Arguments, TFunction<void(const FString& Key, FClientMessage& Body)> Fill);
 bool Ready() const;
 FString ItemName(uint32 Template) const;
 void NewKey();
 UFUNCTION() void SelectionChanged(FString Option, ESelectInfo::Type Type);
 UFUNCTION() void TextChanged(const FText& Text);
 UFUNCTION() void Retry();
 UFUNCTION() void Close();
 UFUNCTION() virtual void Confirm() PURE_VIRTUAL(UNightfallEconomyDialog::Confirm,);
 UPROPERTY() TObjectPtr<UEconomyStateSubsystem> Economy;
 UPROPERTY() TObjectPtr<UInventoryStateSubsystem> Inventory;
 UPROPERTY() TObjectPtr<UVerticalBox> Column;
 UPROPERTY() TObjectPtr<UTextBlock> Status;
 UPROPERTY() TObjectPtr<UButton> ConfirmButton;
 UPROPERTY() TObjectPtr<UButton> RetryButton;
 FString Key;
 /** The last request sent with `Key`: its operation and arguments (the D43 frozen body is the state's). */
 TOptional<EGrpcNightfallV1ItemOperation> FrozenOp; FString FrozenArguments; FClientMessage FrozenBody;
 bool bRendering = false;
 FDelegateHandle EconomyChanged, InventoryChanged;
};

/** Story 5.3: buy, sell and refund tabs from ListBuylist (prices and stock are the server's). */
UCLASS()
class NIGHTFALL_API UNightfallVendorDialog : public UNightfallEconomyDialog
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(UEconomyStateSubsystem* Economy, UInventoryStateSubsystem* Inventory, const FString& Merchant);
 virtual void Render() override;
 bool CanConfirm() const;
 UComboBoxString* ModeControl() const { return Mode; }
 UComboBoxString* EntryControl() const { return Entry; }
 UEditableTextBox* CountControl() const { return Count; }
 /** The rendered label of buylist entry `Index` (tests read the received price from it). */
 FString BuyLabel(int32 Index) const;
protected:
 virtual void Confirm() override;
private:
 UPROPERTY() TObjectPtr<UComboBoxString> Mode;
 UPROPERTY() TObjectPtr<UComboBoxString> Entry;
 UPROPERTY() TObjectPtr<UEditableTextBox> Count;
 TMap<FString, uint64> Choices; // label -> template (buy), instance (sell) or refund index
 TArray<FString> BuyLabels;
 FString Merchant;
};

/** Story 5.3: multisell entries from GetMultisellList (static Phase5Rules data). */
UCLASS()
class NIGHTFALL_API UNightfallMultisellDialog : public UNightfallEconomyDialog
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(UEconomyStateSubsystem* Economy, UInventoryStateSubsystem* Inventory, const FString& Npc, uint32 ListId);
 virtual void Render() override;
 bool CanConfirm() const;
 UComboBoxString* EntryControl() const { return Entry; }
 UEditableTextBox* AmountControl() const { return Amount; }
 UEditableTextBox* ListControl() const { return List; }
 FString EntryLabel(uint32 EntryId) const;
protected:
 virtual void Confirm() override;
private:
 UPROPERTY() TObjectPtr<UComboBoxString> Entry;
 UPROPERTY() TObjectPtr<UEditableTextBox> Amount;
 UPROPERTY() TObjectPtr<UEditableTextBox> List;
 TMap<FString, uint32> Choices;
 TMap<uint32, FString> Labels;
 FString Npc;
 uint32 ListId = 0;
};

/** Story 5.3: the recipe book from RecipeBookSync, craft, register, delete and manufacture orders. */
UCLASS()
class NIGHTFALL_API UNightfallRecipeDialog : public UNightfallEconomyDialog
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(UEconomyStateSubsystem* Economy, UInventoryStateSubsystem* Inventory);
 virtual void Render() override;
 UComboBoxString* ActionControl() const { return Action; }
 UComboBoxString* ChoiceControl() const { return Choice; }
 bool CanConfirm() const;
 /** The rendered recipe book: limits and the received recipe ids. */
 FString BookText() const;
protected:
 virtual void Confirm() override;
private:
 UPROPERTY() TObjectPtr<UComboBoxString> Action;
 UPROPERTY() TObjectPtr<UComboBoxString> Choice;
 UPROPERTY() TObjectPtr<UTextBlock> Book;
 struct FChoice { uint64 Id = 0; FString Owner; };
 TMap<FString, FChoice> Choices;
};
