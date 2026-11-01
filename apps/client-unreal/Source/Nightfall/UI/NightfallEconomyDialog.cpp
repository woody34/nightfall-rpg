#include "NightfallEconomyDialog.h"
#include "Economy/EconomyStateSubsystem.h"
#include "Items/InventoryStateSubsystem.h"
#include "Blueprint/WidgetTree.h"
#include "Components/VerticalBox.h"
#include "Components/ComboBoxString.h"
#include "Components/EditableTextBox.h"
#include "Components/TextBlock.h"
#include "Components/Button.h"

using EOp = EGrpcNightfallV1ItemOperation;

// --- the common form ---------------------------------------------------------------------------

void UNightfallEconomyDialog::NativeDestruct()
{
 if (Economy) Economy->OnChanged.Remove(EconomyChanged);
 if (Inventory) Inventory->OnChanged.Remove(InventoryChanged);
 Super::NativeDestruct();
}
TOptional<FUIInputConfig> UNightfallEconomyDialog::GetDesiredInputConfig() const { return FUIInputConfig(ECommonInputMode::Menu, EMouseCaptureMode::NoCapture); }
void UNightfallEconomyDialog::Bind(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory)
{
 if (Economy) Economy->OnChanged.Remove(EconomyChanged);
 if (Inventory) Inventory->OnChanged.Remove(InventoryChanged);
 Economy = InEconomy; Inventory = InInventory;
 if (Economy) EconomyChanged = Economy->OnChanged.AddUObject(this, &UNightfallEconomyDialog::Render);
 if (Inventory) InventoryChanged = Inventory->OnChanged.AddUObject(this, &UNightfallEconomyDialog::Render);
 NewKey(); FrozenOp.Reset();
}
UVerticalBox* UNightfallEconomyDialog::BeginLayout(const TCHAR* Title)
{
 if (!WidgetTree) WidgetTree = NewObject<UWidgetTree>(this);
 if (WidgetTree->RootWidget) return nullptr;
 Column = WidgetTree->ConstructWidget<UVerticalBox>(); WidgetTree->RootWidget = Column; AddText(Title); return Column;
}
UTextBlock* UNightfallEconomyDialog::AddText(const FString& Text)
{
 auto* T = WidgetTree->ConstructWidget<UTextBlock>(); T->SetText(FText::FromString(Text)); T->SetAutoWrapText(true); Column->AddChild(T); return T;
}
UComboBoxString* UNightfallEconomyDialog::AddChoice(const TCHAR* Label)
{
 AddText(Label); auto* C = WidgetTree->ConstructWidget<UComboBoxString>(); Column->AddChild(C);
 C->OnSelectionChanged.AddDynamic(this, &UNightfallEconomyDialog::SelectionChanged); return C;
}
UEditableTextBox* UNightfallEconomyDialog::AddNumber(const TCHAR* Label, const TCHAR* Default)
{
 AddText(Label); auto* B = WidgetTree->ConstructWidget<UEditableTextBox>(); B->SetText(FText::FromString(Default)); Column->AddChild(B);
 B->OnTextChanged.AddDynamic(this, &UNightfallEconomyDialog::TextChanged); return B;
}
UButton* UNightfallEconomyDialog::AddButton(const TCHAR* Label)
{
 auto* B = WidgetTree->ConstructWidget<UButton>(); auto* T = WidgetTree->ConstructWidget<UTextBlock>(); T->SetText(FText::FromString(Label)); B->AddChild(T); Column->AddChild(B); return B;
}
void UNightfallEconomyDialog::AddCommonButtons(const TCHAR* ConfirmLabel)
{
 Status = AddText(TEXT("The server decides every price, count and result."));
 ConfirmButton = AddButton(ConfirmLabel); ConfirmButton->OnClicked.AddDynamic(this, &UNightfallEconomyDialog::Confirm);
 RetryButton = AddButton(TEXT("Retry previous request")); RetryButton->OnClicked.AddDynamic(this, &UNightfallEconomyDialog::Retry);
 AddButton(TEXT("Close"))->OnClicked.AddDynamic(this, &UNightfallEconomyDialog::Close);
}
TOptional<uint64> UNightfallEconomyDialog::PositiveNumber(const UEditableTextBox* Box)
{
 if (!Box) return {}; const FString Text = Box->GetText().ToString().TrimStartAndEnd();
 if (Text.IsEmpty() || !Text.IsNumeric() || Text.Contains(TEXT(".")) || Text.Contains(TEXT("-"))) return {};
 const uint64 N = FCString::Strtoui64(*Text, nullptr, 10); return N > 0 ? TOptional<uint64>(N) : TOptional<uint64>();
}
bool UNightfallEconomyDialog::Ready() const { return Economy && Inventory && Inventory->HasBaseline() && !Economy->IsBusy(); }
FString UNightfallEconomyDialog::ItemName(uint32 Template) const
{
 const auto* D = Inventory ? Inventory->Definition(Template) : nullptr; return D ? D->Name : FString::Printf(TEXT("Item %u"), Template);
}
void UNightfallEconomyDialog::NewKey() { Key = FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower); }
void UNightfallEconomyDialog::SelectionChanged(FString, ESelectInfo::Type) { if (bRendering) return; NewKey(); FrozenOp.Reset(); Render(); }
void UNightfallEconomyDialog::TextChanged(const FText&) { if (bRendering) return; NewKey(); FrozenOp.Reset(); Render(); }
void UNightfallEconomyDialog::Send(EGrpcNightfallV1ItemOperation Op, const FString& Arguments, TFunction<void(const FString&, FClientMessage&)> Fill)
{
 // A confirmed request is a new operation: a new key. Retry (below) is the only resend of a key.
 if (FrozenOp.IsSet()) NewKey();
 FClientMessage Body; Fill(Key, Body);
 FrozenOp = Op; FrozenArguments = Arguments; FrozenBody = Body;
 Economy->Submit(Op, Key, Arguments, Body); Render();
}
void UNightfallEconomyDialog::Retry()
{
 if (!Economy || !FrozenOp.IsSet() || !RetryButton->GetIsEnabled()) return;
 Economy->Submit(*FrozenOp, Key, FrozenArguments, FrozenBody); Render();
}
void UNightfallEconomyDialog::Close() { DeactivateWidget(); RemoveFromParent(); }

// --- vendor ------------------------------------------------------------------------------------

void UNightfallVendorDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Merchant — buy, sell, refund"))) return;
 Mode = AddChoice(TEXT("Tab")); for (const TCHAR* M : {TEXT("Buy"), TEXT("Sell"), TEXT("Refund")}) Mode->AddOption(M); Mode->SetSelectedIndex(0);
 Entry = AddChoice(TEXT("Entry")); Count = AddNumber(TEXT("Count"), TEXT("1"));
 AddCommonButtons(TEXT("Confirm"));
}
void UNightfallVendorDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory, const FString& InMerchant)
{
 EnsureLayout(); Merchant = InMerchant.ToLower(); Bind(InEconomy, InInventory);
 if (Economy) Economy->OpenBuylist(Merchant);
 Render();
}
FString UNightfallVendorDialog::BuyLabel(int32 Index) const { return BuyLabels.IsValidIndex(Index) ? BuyLabels[Index] : FString(); }
void UNightfallVendorDialog::Render()
{
 if (!Entry || !Economy || !Inventory) return;
 bRendering = true; const FString Was = Entry->GetSelectedOption(), Tab = Mode->GetSelectedOption();
 Entry->ClearOptions(); Choices.Reset(); BuyLabels.Reset();
 const auto& View = Economy->Buylist();
 const bool bView = View.IsSet() && Economy->BuylistMerchant() == Merchant;
 if (bView)
  for (const auto& E : View->Entries)
  {
   // The received unit price and stock, as the server sent them.
   FString Label = FString::Printf(TEXT("%s — %llu adena each"), *ItemName(E.TemplateId.Value), E.UnitPrice.Value);
   if (E.Limited) Label += FString::Printf(TEXT(" (stock %llu/%llu)"), E.Stock.Value, E.MaxStock.Value);
   BuyLabels.Add(Label); if (Tab == TEXT("Buy")) { Choices.Add(Label, E.TemplateId.Value); Entry->AddOption(Label); }
  }
 if (Tab == TEXT("Sell"))
 {
  TArray<uint64> Ids; Inventory->GetItems().GetKeys(Ids); Ids.Sort();
  for (uint64 Id : Ids) { const auto* I = Inventory->Find(Id); const FString Label = FString::Printf(TEXT("%s #%llu x%llu"), *ItemName(I->TemplateId.Value), Id, I->Count.Value); Choices.Add(Label, Id); Entry->AddOption(Label); }
 }
 if (Tab == TEXT("Refund") && bView)
  for (int32 I = 0; I < View->RefundTab.Num(); ++I) { const auto& R = View->RefundTab[I]; const FString Label = FString::Printf(TEXT("%d: %s x%llu"), I, *ItemName(R.TemplateId.Value), R.Count.Value); Choices.Add(Label, uint64(I)); Entry->AddOption(Label); }
 if (Choices.Contains(Was)) Entry->SetSelectedOption(Was); else if (Entry->GetOptionCount() > 0) Entry->SetSelectedIndex(0);
 Count->SetIsEnabled(Tab != TEXT("Refund"));
 bRendering = false;
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet());
 FString Text = Economy->Status();
 if (!bView) Text += TEXT("\nWaiting for the merchant's buylist");
 else Text += FString::Printf(TEXT("\nList %llu, castle tax %u%%"), View->ListId.Value, View->CastleTaxPercent.Value);
 Status->SetText(FText::FromString(Text));
}
bool UNightfallVendorDialog::CanConfirm() const
{
 if (!Ready() || !Choices.Contains(Entry->GetSelectedOption())) return false;
 const auto& View = Economy->Buylist(); if (!View.IsSet() || Economy->BuylistMerchant() != Merchant) return false;
 return Mode->GetSelectedOption() == TEXT("Refund") || PositiveNumber(Count).IsSet();
}
void UNightfallVendorDialog::Confirm()
{
 if (!CanConfirm()) return;
 const FString Tab = Mode->GetSelectedOption(); const uint64 Choice = Choices.FindRef(Entry->GetSelectedOption()); const uint64 ListId = Economy->Buylist()->ListId.Value;
 const uint64 N = Tab == TEXT("Refund") ? 0 : *PositiveNumber(Count); const FString Args = FString::Printf(TEXT("%s %llu %llu %llu"), *Tab, ListId, Choice, N);
 if (Tab == TEXT("Buy"))
  Send(EOp::ITEM_OPERATION_VENDOR_BUY, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1VendorBuyRequest Q; Q.IdempotencyKey = K; Q.Merchant = Merchant; Q.ListId = ListId; FGrpcNightfallV1TemplateCount T; T.TemplateId = uint32(Choice); T.Count = N; Q.Items.Add(T); M.VendorBuy = Q; });
 else if (Tab == TEXT("Sell"))
  Send(EOp::ITEM_OPERATION_VENDOR_SELL, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1VendorSellRequest Q; Q.IdempotencyKey = K; Q.Merchant = Merchant; Q.ListId = ListId; FGrpcNightfallV1ItemCount C; C.InstanceId = Choice; C.Count = N; Q.Items.Add(C); M.VendorSell = Q; });
 else
  Send(EOp::ITEM_OPERATION_VENDOR_REFUND, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1VendorRefundRequest Q; Q.IdempotencyKey = K; Q.Merchant = Merchant; Q.ListId = ListId; Q.RefundIndices.Add(FUInt32(uint32(Choice))); M.VendorRefund = Q; });
}

// --- multisell ---------------------------------------------------------------------------------

void UNightfallMultisellDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Exchange — multisell"))) return;
 List = AddNumber(TEXT("List"), TEXT(""));
 Entry = AddChoice(TEXT("Entry")); Amount = AddNumber(TEXT("Amount"), TEXT("1"));
 AddCommonButtons(TEXT("Exchange"));
}
void UNightfallMultisellDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory, const FString& InNpc, uint32 InListId)
{
 EnsureLayout(); Npc = InNpc.ToLower(); ListId = InListId; Bind(InEconomy, InInventory);
 bRendering = true; List->SetText(FText::FromString(ListId ? FString::Printf(TEXT("%u"), ListId) : FString())); bRendering = false;
 if (Economy && ListId) Economy->OpenMultisell(ListId);
 Render();
}
FString UNightfallMultisellDialog::EntryLabel(uint32 EntryId) const { return Labels.FindRef(EntryId); }
void UNightfallMultisellDialog::Render()
{
 if (!Entry || !Economy || !Inventory) return;
 // A typed list id selects the list (one request per list; the data is static).
 if (const TOptional<uint64> Typed = PositiveNumber(List); Typed.IsSet() && *Typed <= MAX_uint32 && uint32(*Typed) != ListId) { ListId = uint32(*Typed); Economy->OpenMultisell(ListId); }
 bRendering = true; const FString Was = Entry->GetSelectedOption(); Entry->ClearOptions(); Choices.Reset(); Labels.Reset();
 const auto* View = Economy->Multisell(ListId);
 if (View)
  for (const auto& E : View->Entries)
  {
   auto Side = [&](const TArray<FGrpcNightfallV1MultisellIngredient>& Rows) { TArray<FString> Parts; for (const auto& R : Rows) Parts.Add(FString::Printf(TEXT("%s x%llu"), *ItemName(R.TemplateId.Value), R.Count.Value)); return FString::Join(Parts, TEXT(", ")); };
   const FString Label = FString::Printf(TEXT("#%u: %s → %s"), E.EntryId.Value, *Side(E.Ingredients), *Side(E.Products));
   Labels.Add(E.EntryId.Value, Label); Choices.Add(Label, E.EntryId.Value); Entry->AddOption(Label);
  }
 if (Choices.Contains(Was)) Entry->SetSelectedOption(Was); else if (Entry->GetOptionCount() > 0) Entry->SetSelectedIndex(0);
 bRendering = false;
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet());
 FString Text = Economy->Status();
 if (!View) Text += TEXT("\nWaiting for the multisell list");
 else Text += View->MaintainEnchantment ? TEXT("\nThe product keeps the ingredient's enchantment") : TEXT("");
 Status->SetText(FText::FromString(Text));
}
bool UNightfallMultisellDialog::CanConfirm() const
{
 return Ready() && Economy->Multisell(ListId) && Choices.Contains(Entry->GetSelectedOption()) && PositiveNumber(Amount).IsSet();
}
void UNightfallMultisellDialog::Confirm()
{
 if (!CanConfirm()) return;
 const uint32 EntryId = Choices.FindRef(Entry->GetSelectedOption()); const uint64 N = *PositiveNumber(Amount);
 Send(EOp::ITEM_OPERATION_MULTISELL_CHOOSE, FString::Printf(TEXT("%u %u %llu"), ListId, EntryId, N), [&](const FString& K, FClientMessage& M) {
  FGrpcNightfallV1MultisellChooseRequest Q; Q.IdempotencyKey = K; Q.Npc = Npc; Q.ListId = ListId; Q.EntryId = EntryId; Q.Amount = N; M.MultisellChoose = Q; });
}

// --- recipe book -------------------------------------------------------------------------------

void UNightfallRecipeDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Recipe book — craft and manufacture"))) return;
 Book = AddText(TEXT("Waiting for the recipe book"));
 Action = AddChoice(TEXT("Action")); for (const TCHAR* A : {TEXT("Craft"), TEXT("Register"), TEXT("Delete"), TEXT("Manufacture")}) Action->AddOption(A); Action->SetSelectedIndex(0);
 Choice = AddChoice(TEXT("Recipe, recipe item or manufacture entry"));
 AddCommonButtons(TEXT("Confirm"));
}
void UNightfallRecipeDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory)
{
 EnsureLayout(); Bind(InEconomy, InInventory); Render();
}
FString UNightfallRecipeDialog::BookText() const { return Book ? Book->GetText().ToString() : FString(); }
void UNightfallRecipeDialog::Render()
{
 if (!Choice || !Economy || !Inventory) return;
 bRendering = true; const FString Was = Choice->GetSelectedOption(), A = Action->GetSelectedOption(); Choice->ClearOptions(); Choices.Reset();
 const auto& RB = Economy->RecipeBook();
 auto Ids = [](const TArray<FUInt32>& L) { TArray<FString> S; for (const auto& R : L) S.Add(FString::Printf(TEXT("%u"), R.Value)); return FString::Join(S, TEXT(", ")); };
 if (RB.IsSet())
 {
  Book->SetText(FText::FromString(FString::Printf(TEXT("Dwarven %d/%u: %s\nCommon %d/%u: %s"), RB->Dwarven.Num(), RB->DwarvenLimit.Value, *Ids(RB->Dwarven), RB->Common.Num(), RB->CommonLimit.Value, *Ids(RB->Common))));
  if (A == TEXT("Craft") || A == TEXT("Delete"))
  {
   for (const auto& R : RB->Dwarven) { const FString L = FString::Printf(TEXT("Dwarven recipe %u"), R.Value); Choices.Add(L, {R.Value, {}}); Choice->AddOption(L); }
   for (const auto& R : RB->Common) { const FString L = FString::Printf(TEXT("Common recipe %u"), R.Value); Choices.Add(L, {R.Value, {}}); Choice->AddOption(L); }
  }
 }
 else Book->SetText(FText::FromString(TEXT("Waiting for the recipe book")));
 if (A == TEXT("Register"))
 {
  TArray<uint64> Items; Inventory->GetItems().GetKeys(Items); Items.Sort();
  for (uint64 Id : Items) { const auto* I = Inventory->Find(Id); const FString L = FString::Printf(TEXT("%s #%llu"), *ItemName(I->TemplateId.Value), Id); Choices.Add(L, {Id, {}}); Choice->AddOption(L); }
 }
 if (A == TEXT("Manufacture"))
  for (const auto& S : Economy->AllStores())
   for (const auto& M : S.Value.Manufacture)
   {
    // The received manufacture cost of the store's owner.
    const FString L = FString::Printf(TEXT("%s: recipe %u for %llu adena"), *S.Value.Title, M.RecipeId.Value, M.Cost.Value);
    Choices.Add(L, {M.RecipeId.Value, S.Value.Owner}); Choice->AddOption(L);
   }
 if (Choices.Contains(Was)) Choice->SetSelectedOption(Was); else if (Choice->GetOptionCount() > 0) Choice->SetSelectedIndex(0);
 bRendering = false;
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet()); Status->SetText(FText::FromString(Economy->Status()));
}
bool UNightfallRecipeDialog::CanConfirm() const
{
 const FString A = Action->GetSelectedOption();
 return Ready() && Choices.Contains(Choice->GetSelectedOption()) && (A == TEXT("Register") || A == TEXT("Manufacture") || Economy->RecipeBook().IsSet());
}
void UNightfallRecipeDialog::Confirm()
{
 if (!CanConfirm()) return;
 const FString A = Action->GetSelectedOption(); const FChoice C = Choices.FindRef(Choice->GetSelectedOption());
 const FString Args = FString::Printf(TEXT("%s %llu %s"), *A, C.Id, *C.Owner);
 if (A == TEXT("Craft")) Send(EOp::ITEM_OPERATION_CRAFT_ITEM, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1CraftItemRequest Q; Q.IdempotencyKey = K; Q.RecipeId = uint32(C.Id); M.CraftItem = Q; });
 else if (A == TEXT("Register")) Send(EOp::ITEM_OPERATION_REGISTER_RECIPE, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1RegisterRecipeRequest Q; Q.IdempotencyKey = K; Q.RecipeInstanceId = C.Id; M.RegisterRecipe = Q; });
 else if (A == TEXT("Delete")) Send(EOp::ITEM_OPERATION_DELETE_RECIPE, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1DeleteRecipeRequest Q; Q.IdempotencyKey = K; Q.RecipeId = uint32(C.Id); M.DeleteRecipe = Q; });
 else Send(EOp::ITEM_OPERATION_PRIVATE_MANUFACTURE, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1PrivateManufactureOrderRequest Q; Q.IdempotencyKey = K; Q.StoreOwner = C.Owner; Q.RecipeId = uint32(C.Id); M.PrivateManufactureOrder = Q; });
}
