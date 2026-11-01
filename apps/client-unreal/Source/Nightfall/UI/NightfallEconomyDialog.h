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

/**
 * Story 5.4: the trade window. It shows the received request, the received session (both lists, both
 * confirmations, the phase and the cancel reason) and sends TradeRequest / TradeAnswer / TradeAddItem /
 * TradeConfirm / TradeCancel. The actions follow the received state only.
 */
UCLASS()
class NIGHTFALL_API UNightfallTradeDialog : public UNightfallEconomyDialog
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(UEconomyStateSubsystem* Economy, UInventoryStateSubsystem* Inventory, const FString& Target);
 virtual void Render() override;
 bool CanConfirm() const;
 UComboBoxString* ActionControl() const { return Action; }
 UComboBoxString* ItemControl() const { return Item; }
 UEditableTextBox* CountControl() const { return Count; }
 /** The rendered session: request, partner, phase, both lists and confirmations. */
 FString SessionText() const;
protected:
 virtual void Confirm() override;
private:
 UPROPERTY() TObjectPtr<UTextBlock> Session;
 UPROPERTY() TObjectPtr<UComboBoxString> Action;
 UPROPERTY() TObjectPtr<UComboBoxString> Item;
 UPROPERTY() TObjectPtr<UEditableTextBox> Count;
 TMap<FString, uint64> Items;
 FString Target;
};

/**
 * Story 5.4: the owner's private store: sell, package sell, buy or manufacture entries are collected
 * locally and sent in one SetPrivateStore* request; Close store sends ClosePrivateStore. The open
 * store shown is the received PrivateStoreChanged of the own entity.
 */
UCLASS()
class NIGHTFALL_API UNightfallStoreSetupDialog : public UNightfallEconomyDialog
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(UEconomyStateSubsystem* Economy, UInventoryStateSubsystem* Inventory);
 virtual void Render() override;
 bool CanConfirm() const;
 bool CanAdd() const;
 UComboBoxString* KindControl() const { return Kind; }
 UEditableTextBox* TitleControl() const { return Title; }
 UComboBoxString* EntryControl() const { return Entry; }
 UEditableTextBox* TemplateControl() const { return Template; }
 UEditableTextBox* CountControl() const { return Count; }
 UEditableTextBox* PriceControl() const { return Price; }
 UButton* AddControl() const { return AddEntry; }
 UButton* ClearControl() const { return ClearEntries; }
 int32 PendingCount() const;
 /** The received open store of the own entity, as shown. */
 FString OpenStoreText() const;
protected:
 virtual void Confirm() override;
private:
 UFUNCTION() void Add();
 UFUNCTION() void Clear();
 UPROPERTY() TObjectPtr<UComboBoxString> Kind;
 UPROPERTY() TObjectPtr<UEditableTextBox> Title;
 UPROPERTY() TObjectPtr<UComboBoxString> Entry;
 UPROPERTY() TObjectPtr<UEditableTextBox> Template;
 UPROPERTY() TObjectPtr<UEditableTextBox> Count;
 UPROPERTY() TObjectPtr<UEditableTextBox> Price;
 UPROPERTY() TObjectPtr<UButton> AddEntry;
 UPROPERTY() TObjectPtr<UButton> ClearEntries;
 UPROPERTY() TObjectPtr<UTextBlock> Pending;
 UPROPERTY() TObjectPtr<UTextBlock> Open;
 TMap<FString, uint64> Choices; // label -> instance (sell) or recipe id (manufacture)
 TArray<FGrpcNightfallV1StoreSellEntry> SellEntries;
 TArray<FGrpcNightfallV1StoreBuyEntry> BuyEntries;
 TArray<FGrpcNightfallV1ManufactureEntry> MakeEntries;
 FString PendingKind;
};

/**
 * Story 5.4: another player's private store, from its received PrivateStoreChanged. Buy from a sell
 * store (a package store sells only all of its entries), sell into a buy store, or order from a
 * manufacture store. Prices are the store's received prices.
 */
UCLASS()
class NIGHTFALL_API UNightfallStoreVisitDialog : public UNightfallEconomyDialog
{
 GENERATED_BODY()
public:
 void EnsureLayout();
 void BindState(UEconomyStateSubsystem* Economy, UInventoryStateSubsystem* Inventory, const FString& StoreOwner);
 virtual void Render() override;
 bool CanConfirm() const;
 UComboBoxString* EntryControl() const { return Entry; }
 UComboBoxString* ItemControl() const { return Item; }
 UEditableTextBox* CountControl() const { return Count; }
 FString StoreText() const;
protected:
 virtual void Confirm() override;
private:
 const FGrpcNightfallV1PrivateStoreChanged* Store() const;
 UPROPERTY() TObjectPtr<UTextBlock> Shown;
 UPROPERTY() TObjectPtr<UComboBoxString> Entry;
 UPROPERTY() TObjectPtr<UComboBoxString> Item;
 UPROPERTY() TObjectPtr<UEditableTextBox> Count;
 TMap<FString, int32> Entries; // label -> index into the received sell/buy/manufacture rows
 TMap<FString, uint64> Items;  // label -> owned instance (sell into a buy store)
 FString Owner;
};
