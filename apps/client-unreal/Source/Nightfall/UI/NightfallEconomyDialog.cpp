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
FString UNightfallEconomyDialog::BusyNotice() const
{
 return Economy && FrozenOp.IsSet() && Economy->ContainerBusy(*FrozenOp, Key)
  ? FString(TEXT("\nThe container is busy (CONTAINER_BUSY). Retry sends the same request with the same key.")) : FString();
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
 Enchant = AddNumber(TEXT("Enchant of the kept ingredient (maintained lists)"), TEXT("0"));
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
 Enchant->SetIsEnabled(View && View->MaintainEnchantment);
 bRendering = false;
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet());
 FString Text = Economy->Status();
 if (!View) Text += TEXT("\nWaiting for the multisell list");
 else Text += View->MaintainEnchantment ? TEXT("\nThe product keeps the ingredient's enchantment") : TEXT("");
 Status->SetText(FText::FromString(Text));
}
bool UNightfallMultisellDialog::CanConfirm() const
{
 if (!Ready() || !Economy->Multisell(ListId) || !Choices.Contains(Entry->GetSelectedOption()) || !PositiveNumber(Amount).IsSet()) return false;
 const FString E = Enchant->GetText().ToString().TrimStartAndEnd(); const auto N = PositiveNumber(Enchant);
 return E == TEXT("0") || (Economy->Multisell(ListId)->MaintainEnchantment && N.IsSet() && *N < UEconomyStateSubsystem::MultisellEntryStride);
}
void UNightfallMultisellDialog::Confirm()
{
 if (!CanConfirm()) return;
 // The pinned PreparedEntry id (Z lane 3.7): the 1-based entry x 100000 + the kept enchant.
 const uint64 Prepared = uint64(Choices.FindRef(Entry->GetSelectedOption())) * UEconomyStateSubsystem::MultisellEntryStride + PositiveNumber(Enchant).Get(0);
 if (Prepared > MAX_uint32) return;
 const uint32 EntryId = uint32(Prepared); const uint64 N = *PositiveNumber(Amount);
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

// --- trade (story 5.4) -------------------------------------------------------------------------

namespace NightfallStoreUi
{
 using P = EGrpcNightfallV1TradePhase; using K = EGrpcNightfallV1StoreKind;
 const TCHAR* PhaseName(P Phase)
 {
  switch (Phase)
  {
  case P::TRADE_PHASE_STARTED: return TEXT("started"); case P::TRADE_PHASE_ITEM_ADDED: return TEXT("item added");
  case P::TRADE_PHASE_PARTNER_CONFIRMED: return TEXT("partner confirmed"); case P::TRADE_PHASE_COMPLETED: return TEXT("completed");
  case P::TRADE_PHASE_CANCELLED: return TEXT("cancelled"); default: return TEXT("unknown");
  }
 }
 const TCHAR* KindName(K Kind)
 {
  switch (Kind)
  {
  case K::STORE_KIND_SELL: return TEXT("Sell"); case K::STORE_KIND_PACKAGE_SELL: return TEXT("Package sell"); case K::STORE_KIND_BUY: return TEXT("Buy");
  case K::STORE_KIND_MANUFACTURE: return TEXT("Manufacture"); default: return TEXT("Closed");
  }
 }
 bool Open(const TOptional<FGrpcNightfallV1TradeSessionChanged>& T) { return T.IsSet() && T->Phase != P::TRADE_PHASE_COMPLETED && T->Phase != P::TRADE_PHASE_CANCELLED && T->Phase != P::TRADE_PHASE_UNSPECIFIED; }
}

void UNightfallTradeDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Trade"))) return;
 Session = AddText(TEXT("No trade"));
 Action = AddChoice(TEXT("Action")); Item = AddChoice(TEXT("Item to add")); Count = AddNumber(TEXT("Count"), TEXT("1"));
 AddCommonButtons(TEXT("Confirm"));
}
void UNightfallTradeDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory, const FString& InTarget)
{
 EnsureLayout(); Target = InTarget.ToLower(); Bind(InEconomy, InInventory); Render();
}
FString UNightfallTradeDialog::SessionText() const { return Session ? Session->GetText().ToString() : FString(); }
void UNightfallTradeDialog::Render()
{
 using namespace NightfallStoreUi;
 if (!Action || !Economy || !Inventory) return;
 bRendering = true; const FString WasA = Action->GetSelectedOption(), WasI = Item->GetSelectedOption();
 Action->ClearOptions(); Item->ClearOptions(); Items.Reset();
 const auto& T = Economy->Trade(); const auto& R = Economy->TradeRequest(); const bool bOpen = Open(T);
 TArray<FString> Actions;
 if (bOpen) Actions = { TEXT("Add item"), TEXT("Confirm"), TEXT("Cancel") };
 else if (R.IsSet()) Actions = { TEXT("Accept"), TEXT("Decline") };
 else if (!Target.IsEmpty()) Actions = { TEXT("Request") };
 for (const FString& A : Actions) Action->AddOption(A);
 if (Actions.Contains(WasA)) Action->SetSelectedOption(WasA); else if (!Actions.IsEmpty()) Action->SetSelectedIndex(0);
 auto Line = [&](const FGrpcNightfallV1TradeEntry& E) { return E.Enchant.Value ? FString::Printf(TEXT("  %s +%u x%llu"), *ItemName(E.TemplateId.Value), E.Enchant.Value, E.Count.Value) : FString::Printf(TEXT("  %s x%llu"), *ItemName(E.TemplateId.Value), E.Count.Value); };
 TSet<uint64> Offered;
 FString Text;
 if (R.IsSet() && !bOpen) Text = FString::Printf(TEXT("Trade request from %s (until tick %llu)"), *R->Requester, R->ExpiresTick.Value);
 else if (T.IsSet())
 {
  Text = FString::Printf(TEXT("Partner %s — %s"), *T->Partner, PhaseName(T->Phase));
  if (!T->CancelReason.IsEmpty()) Text += FString::Printf(TEXT(" (%s)"), *T->CancelReason);
  Text += FString::Printf(TEXT("\nYou offer%s:"), T->OwnConfirmed ? TEXT(" (confirmed)") : TEXT(""));
  for (const auto& E : T->Own) { Text += TEXT("\n") + Line(E); Offered.Add(E.InstanceId.Value); }
  Text += FString::Printf(TEXT("\nPartner offers%s:"), T->PartnerConfirmed ? TEXT(" (confirmed)") : TEXT(""));
  for (const auto& E : T->PartnerItems) Text += TEXT("\n") + Line(E);
 }
 else Text = Target.IsEmpty() ? TEXT("No trade. Target a player to request one.") : FString::Printf(TEXT("No trade. Request one with %s."), *Target);
 Session->SetText(FText::FromString(Text));
 if (Action->GetSelectedOption() == TEXT("Add item"))
 {
  TArray<uint64> Ids; Inventory->GetItems().GetKeys(Ids); Ids.Sort();
  for (uint64 Id : Ids) { if (Offered.Contains(Id)) continue; const auto* I = Inventory->Find(Id); const FString L = FString::Printf(TEXT("%s #%llu x%llu"), *ItemName(I->TemplateId.Value), Id, I->Count.Value); Items.Add(L, Id); Item->AddOption(L); }
 }
 if (Items.Contains(WasI)) Item->SetSelectedOption(WasI); else if (Item->GetOptionCount() > 0) Item->SetSelectedIndex(0);
 Item->SetIsEnabled(!Items.IsEmpty()); Count->SetIsEnabled(!Items.IsEmpty());
 bRendering = false;
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet()); Status->SetText(FText::FromString(Economy->Status()));
}
bool UNightfallTradeDialog::CanConfirm() const
{
 using namespace NightfallStoreUi;
 if (!Ready()) return false;
 const FString A = Action->GetSelectedOption(); const auto& T = Economy->Trade();
 if (A == TEXT("Request")) return !Target.IsEmpty() && !Open(T);
 if (A == TEXT("Accept") || A == TEXT("Decline")) return Economy->TradeRequest().IsSet();
 if (A == TEXT("Add item")) return Open(T) && Items.Contains(Item->GetSelectedOption()) && PositiveNumber(Count).IsSet();
 if (A == TEXT("Confirm")) return Open(T) && !T->OwnConfirmed;
 return A == TEXT("Cancel") && Open(T);
}
void UNightfallTradeDialog::Confirm()
{
 if (!CanConfirm()) return;
 const FString A = Action->GetSelectedOption();
 if (A == TEXT("Request"))
  Send(EOp::ITEM_OPERATION_TRADE_SESSION, TEXT("request ") + Target, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1TradeRequestRequest Q; Q.IdempotencyKey = K; Q.Target = Target; M.TradeRequest = Q; });
 else if (A == TEXT("Accept") || A == TEXT("Decline"))
 {
  const bool bAccept = A == TEXT("Accept");
  Send(EOp::ITEM_OPERATION_TRADE_SESSION, bAccept ? TEXT("accept") : TEXT("decline"), [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1TradeAnswerRequest Q; Q.IdempotencyKey = K; Q.Accept = bAccept; M.TradeAnswer = Q; });
 }
 else if (A == TEXT("Add item"))
 {
  const uint64 Id = Items.FindRef(Item->GetSelectedOption()); const uint64 N = *PositiveNumber(Count);
  Send(EOp::ITEM_OPERATION_TRADE_SESSION, FString::Printf(TEXT("add %llu %llu"), Id, N), [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1TradeAddItemRequest Q; Q.IdempotencyKey = K; Q.InstanceId = Id; Q.Count = N; M.TradeAddItem = Q; });
 }
 else if (A == TEXT("Confirm"))
  Send(EOp::ITEM_OPERATION_TRADE_CONFIRM, TEXT("confirm"), [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1TradeConfirmRequest Q; Q.IdempotencyKey = K; M.TradeConfirm = Q; });
 else
  Send(EOp::ITEM_OPERATION_TRADE_SESSION, TEXT("cancel"), [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1TradeCancelRequest Q; Q.IdempotencyKey = K; M.TradeCancel = Q; });
}

// --- private store, owner (story 5.4) ----------------------------------------------------------

void UNightfallStoreSetupDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Private store"))) return;
 Open = AddText(TEXT("Your store is closed."));
 Kind = AddChoice(TEXT("Store")); for (const TCHAR* S : {TEXT("Sell"), TEXT("Package sell"), TEXT("Buy"), TEXT("Manufacture"), TEXT("Close store")}) Kind->AddOption(S); Kind->SetSelectedIndex(0);
 Title = AddNumber(TEXT("Title"), TEXT(""));
 Entry = AddChoice(TEXT("Item or recipe")); Template = AddNumber(TEXT("Template to buy"), TEXT(""));
 Count = AddNumber(TEXT("Count"), TEXT("1")); Price = AddNumber(TEXT("Unit price or fee (adena)"), TEXT(""));
 AddEntry = AddButton(TEXT("Add entry")); AddEntry->OnClicked.AddDynamic(this, &UNightfallStoreSetupDialog::Add);
 ClearEntries = AddButton(TEXT("Clear entries")); ClearEntries->OnClicked.AddDynamic(this, &UNightfallStoreSetupDialog::Clear);
 Pending = AddText(TEXT("No entries"));
 AddCommonButtons(TEXT("Open store"));
}
void UNightfallStoreSetupDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory)
{
 EnsureLayout(); Bind(InEconomy, InInventory); Render();
}
int32 UNightfallStoreSetupDialog::PendingCount() const { return SellEntries.Num() + BuyEntries.Num() + MakeEntries.Num(); }
FString UNightfallStoreSetupDialog::OpenStoreText() const { return Open ? Open->GetText().ToString() : FString(); }
void UNightfallStoreSetupDialog::Render()
{
 using namespace NightfallStoreUi;
 if (!Entry || !Economy || !Inventory) return;
 bRendering = true; const FString Mode = Kind->GetSelectedOption(), Was = Entry->GetSelectedOption();
 // Another store kind takes other entries: the collected ones are dropped.
 if (Mode != PendingKind) { SellEntries.Reset(); BuyEntries.Reset(); MakeEntries.Reset(); PendingKind = Mode; }
 Entry->ClearOptions(); Choices.Reset();
 if (Mode == TEXT("Sell") || Mode == TEXT("Package sell"))
 {
  TSet<uint64> Listed; for (const auto& E : SellEntries) Listed.Add(E.InstanceId.Value);
  TArray<uint64> Ids; Inventory->GetItems().GetKeys(Ids); Ids.Sort();
  for (uint64 Id : Ids) { if (Listed.Contains(Id)) continue; const auto* I = Inventory->Find(Id); const FString L = FString::Printf(TEXT("%s #%llu x%llu"), *ItemName(I->TemplateId.Value), Id, I->Count.Value); Choices.Add(L, Id); Entry->AddOption(L); }
 }
 else if (Mode == TEXT("Manufacture") && Economy->RecipeBook().IsSet())
 {
  for (const auto& R : Economy->RecipeBook()->Dwarven) { const FString L = FString::Printf(TEXT("Dwarven recipe %u"), R.Value); Choices.Add(L, R.Value); Entry->AddOption(L); }
  for (const auto& R : Economy->RecipeBook()->Common) { const FString L = FString::Printf(TEXT("Common recipe %u"), R.Value); Choices.Add(L, R.Value); Entry->AddOption(L); }
 }
 if (Choices.Contains(Was)) Entry->SetSelectedOption(Was); else if (Entry->GetOptionCount() > 0) Entry->SetSelectedIndex(0);
 const bool bClose = Mode == TEXT("Close store");
 Entry->SetIsEnabled(!bClose && Mode != TEXT("Buy")); Template->SetIsEnabled(Mode == TEXT("Buy")); Count->SetIsEnabled(!bClose && Mode != TEXT("Manufacture"));
 Price->SetIsEnabled(!bClose); Title->SetIsEnabled(!bClose);
 TArray<FString> Lines;
 for (const auto& E : SellEntries) Lines.Add(FString::Printf(TEXT("%s #%llu x%llu for %llu adena each"), *ItemName(E.TemplateId.Value), E.InstanceId.Value, E.Count.Value, E.Price.Value));
 for (const auto& E : BuyEntries) Lines.Add(FString::Printf(TEXT("buy %s x%llu at %llu adena each"), *ItemName(E.TemplateId.Value), E.Count.Value, E.Price.Value));
 for (const auto& E : MakeEntries) Lines.Add(FString::Printf(TEXT("recipe %u for %llu adena"), E.RecipeId.Value, E.Cost.Value));
 Pending->SetText(FText::FromString(Lines.IsEmpty() ? FString(TEXT("No entries")) : FString::Join(Lines, TEXT("\n"))));
 // The open store is the received one, never the local entries.
 const auto* Mine = Economy->Store(Economy->OwnEntityId());
 if (Mine && Mine->Kind != K::STORE_KIND_NONE && Mine->Kind != K::STORE_KIND_UNSPECIFIED)
  Open->SetText(FText::FromString(FString::Printf(TEXT("Open: %s store \"%s\", %d entries"), KindName(Mine->Kind), *Mine->Title, Mine->Sell.Num() + Mine->Buy.Num() + Mine->Manufacture.Num())));
 else Open->SetText(FText::FromString(TEXT("Your store is closed.")));
 bRendering = false;
 AddEntry->SetIsEnabled(CanAdd()); ClearEntries->SetIsEnabled(PendingCount() > 0);
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet()); Status->SetText(FText::FromString(Economy->Status()));
}
bool UNightfallStoreSetupDialog::CanAdd() const
{
 const FString Mode = Kind->GetSelectedOption();
 if (!PositiveNumber(Price).IsSet()) return false;
 if (Mode == TEXT("Sell") || Mode == TEXT("Package sell")) return Choices.Contains(Entry->GetSelectedOption()) && PositiveNumber(Count).IsSet();
 if (Mode == TEXT("Buy")) { const auto T = PositiveNumber(Template); return T.IsSet() && *T <= MAX_uint32 && PositiveNumber(Count).IsSet(); }
 if (Mode == TEXT("Manufacture")) return Choices.Contains(Entry->GetSelectedOption()) && !MakeEntries.ContainsByPredicate([&](const auto& E) { return E.RecipeId.Value == uint32(Choices.FindRef(Entry->GetSelectedOption())); });
 return false;
}
void UNightfallStoreSetupDialog::Add()
{
 if (!CanAdd()) return;
 const FString Mode = Kind->GetSelectedOption(); const uint64 P = *PositiveNumber(Price);
 if (Mode == TEXT("Sell") || Mode == TEXT("Package sell"))
 {
  const uint64 Id = Choices.FindRef(Entry->GetSelectedOption()); const auto* I = Inventory->Find(Id); if (!I) return;
  FGrpcNightfallV1StoreSellEntry E; E.InstanceId = Id; E.TemplateId = I->TemplateId.Value; E.Count = *PositiveNumber(Count); E.Price = P; SellEntries.Add(E);
 }
 else if (Mode == TEXT("Buy")) { FGrpcNightfallV1StoreBuyEntry E; E.TemplateId = uint32(*PositiveNumber(Template)); E.Enchant = 0; E.Count = *PositiveNumber(Count); E.Price = P; BuyEntries.Add(E); }
 else { FGrpcNightfallV1ManufactureEntry E; E.RecipeId = uint32(Choices.FindRef(Entry->GetSelectedOption())); E.Cost = P; MakeEntries.Add(E); }
 // Other entries make another request.
 NewKey(); FrozenOp.Reset(); Render();
}
void UNightfallStoreSetupDialog::Clear() { SellEntries.Reset(); BuyEntries.Reset(); MakeEntries.Reset(); NewKey(); FrozenOp.Reset(); Render(); }
bool UNightfallStoreSetupDialog::CanConfirm() const
{
 using namespace NightfallStoreUi;
 if (!Ready()) return false;
 if (Kind->GetSelectedOption() == TEXT("Close store")) { const auto* Mine = Economy->Store(Economy->OwnEntityId()); return Mine && Mine->Kind != K::STORE_KIND_NONE && Mine->Kind != K::STORE_KIND_UNSPECIFIED; }
 return PendingCount() > 0 && !Title->GetText().ToString().TrimStartAndEnd().IsEmpty();
}
void UNightfallStoreSetupDialog::Confirm()
{
 if (!CanConfirm()) return;
 const FString Mode = Kind->GetSelectedOption(); const FString T = Title->GetText().ToString().TrimStartAndEnd();
 const FString Args = FString::Printf(TEXT("%s \"%s\" %d"), *Mode, *T, PendingCount());
 if (Mode == TEXT("Close store")) Send(EOp::ITEM_OPERATION_STORE_SETUP, TEXT("close"), [&](const FString& Key0, FClientMessage& M) { FGrpcNightfallV1ClosePrivateStoreRequest Q; Q.IdempotencyKey = Key0; M.ClosePrivateStore = Q; });
 else if (Mode == TEXT("Buy")) Send(EOp::ITEM_OPERATION_STORE_SETUP, Args, [&](const FString& Key0, FClientMessage& M) { FGrpcNightfallV1SetPrivateStoreBuyRequest Q; Q.IdempotencyKey = Key0; Q.Title = T; Q.Entries = BuyEntries; M.SetPrivateStoreBuy = Q; });
 else if (Mode == TEXT("Manufacture")) Send(EOp::ITEM_OPERATION_STORE_SETUP, Args, [&](const FString& Key0, FClientMessage& M) { FGrpcNightfallV1SetPrivateManufactureRequest Q; Q.IdempotencyKey = Key0; Q.Title = T; Q.Entries = MakeEntries; M.SetPrivateManufacture = Q; });
 else
 {
  const bool bPackage = Mode == TEXT("Package sell");
  Send(EOp::ITEM_OPERATION_STORE_SETUP, Args, [&](const FString& Key0, FClientMessage& M) { FGrpcNightfallV1SetPrivateStoreSellRequest Q; Q.IdempotencyKey = Key0; Q.Title = T; Q.PackageSale = bPackage; Q.Entries = SellEntries; M.SetPrivateStoreSell = Q; });
 }
}

// --- private store, customer (story 5.4) -------------------------------------------------------

void UNightfallStoreVisitDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Private store — buy, sell, order"))) return;
 Shown = AddText(TEXT("Waiting for the store"));
 Entry = AddChoice(TEXT("Entry")); Item = AddChoice(TEXT("Your item to sell")); Count = AddNumber(TEXT("Count"), TEXT("1"));
 AddCommonButtons(TEXT("Confirm"));
}
void UNightfallStoreVisitDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory, const FString& InOwner)
{
 EnsureLayout(); Owner = InOwner.ToLower(); Bind(InEconomy, InInventory); Render();
}
const FGrpcNightfallV1PrivateStoreChanged* UNightfallStoreVisitDialog::Store() const
{
 using namespace NightfallStoreUi;
 const auto* S = Economy ? Economy->Store(Owner) : nullptr;
 return S && S->Kind != K::STORE_KIND_NONE && S->Kind != K::STORE_KIND_UNSPECIFIED ? S : nullptr;
}
FString UNightfallStoreVisitDialog::StoreText() const { return Shown ? Shown->GetText().ToString() : FString(); }
void UNightfallStoreVisitDialog::Render()
{
 using namespace NightfallStoreUi;
 if (!Entry || !Economy || !Inventory) return;
 bRendering = true; const FString Was = Entry->GetSelectedOption(), WasI = Item->GetSelectedOption();
 Entry->ClearOptions(); Item->ClearOptions(); Entries.Reset(); Items.Reset();
 const auto* S = Store();
 if (S)
 {
  Shown->SetText(FText::FromString(FString::Printf(TEXT("%s store \"%s\""), KindName(S->Kind), *S->Title)));
  auto Option = [&](const FString& L, int32 I) { Entries.Add(L, I); Entry->AddOption(L); };
  if (S->Kind == K::STORE_KIND_SELL)
   for (int32 I = 0; I < S->Sell.Num(); ++I) Option(FString::Printf(TEXT("%s x%llu for %llu adena each"), *ItemName(S->Sell[I].TemplateId.Value), S->Sell[I].Count.Value, S->Sell[I].Price.Value), I);
  else if (S->Kind == K::STORE_KIND_PACKAGE_SELL)
  {
   TArray<FString> Parts; for (const auto& R : S->Sell) Parts.Add(FString::Printf(TEXT("%s x%llu for %llu each"), *ItemName(R.TemplateId.Value), R.Count.Value, R.Price.Value));
   Option(TEXT("The whole package: ") + FString::Join(Parts, TEXT(", ")), -1);
  }
  else if (S->Kind == K::STORE_KIND_BUY)
   for (int32 I = 0; I < S->Buy.Num(); ++I) Option(FString::Printf(TEXT("buys %s x%llu at %llu adena each"), *ItemName(S->Buy[I].TemplateId.Value), S->Buy[I].Count.Value, S->Buy[I].Price.Value), I);
  else
   for (int32 I = 0; I < S->Manufacture.Num(); ++I) Option(FString::Printf(TEXT("recipe %u for %llu adena"), S->Manufacture[I].RecipeId.Value, S->Manufacture[I].Cost.Value), I);
 }
 else Shown->SetText(FText::FromString(TEXT("Waiting for the store")));
 if (Entries.Contains(Was)) Entry->SetSelectedOption(Was); else if (Entry->GetOptionCount() > 0) Entry->SetSelectedIndex(0);
 // A buy store takes owned instances of the selected entry's template.
 if (S && S->Kind == K::STORE_KIND_BUY && Entries.Contains(Entry->GetSelectedOption()))
 {
  const uint32 Want = S->Buy[Entries.FindRef(Entry->GetSelectedOption())].TemplateId.Value;
  TArray<uint64> Ids; Inventory->GetItems().GetKeys(Ids); Ids.Sort();
  for (uint64 Id : Ids) { const auto* I = Inventory->Find(Id); if (I->TemplateId.Value != Want) continue; const FString L = FString::Printf(TEXT("%s #%llu x%llu"), *ItemName(I->TemplateId.Value), Id, I->Count.Value); Items.Add(L, Id); Item->AddOption(L); }
 }
 if (Items.Contains(WasI)) Item->SetSelectedOption(WasI); else if (Item->GetOptionCount() > 0) Item->SetSelectedIndex(0);
 Item->SetIsEnabled(!Items.IsEmpty()); Count->SetIsEnabled(S && (S->Kind == K::STORE_KIND_SELL || S->Kind == K::STORE_KIND_BUY));
 bRendering = false;
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet()); Status->SetText(FText::FromString(Economy->Status()));
}
bool UNightfallStoreVisitDialog::CanConfirm() const
{
 using namespace NightfallStoreUi;
 const auto* S = Store(); if (!Ready() || !S || !Entries.Contains(Entry->GetSelectedOption())) return false;
 if (S->Kind == K::STORE_KIND_SELL) return PositiveNumber(Count).IsSet();
 if (S->Kind == K::STORE_KIND_BUY) return PositiveNumber(Count).IsSet() && Items.Contains(Item->GetSelectedOption());
 return true;
}
void UNightfallStoreVisitDialog::Confirm()
{
 using namespace NightfallStoreUi;
 if (!CanConfirm()) return;
 const auto* S = Store(); const int32 I = Entries.FindRef(Entry->GetSelectedOption()); const FString Who = Owner;
 if (S->Kind == K::STORE_KIND_SELL || S->Kind == K::STORE_KIND_PACKAGE_SELL)
 {
  // The received entries, at the received prices; a package is bought whole.
  TArray<FGrpcNightfallV1StoreSellEntry> Rows;
  if (S->Kind == K::STORE_KIND_PACKAGE_SELL) Rows = S->Sell;
  else { FGrpcNightfallV1StoreSellEntry R = S->Sell[I]; R.Count = *PositiveNumber(Count); Rows.Add(R); }
  Send(EOp::ITEM_OPERATION_STORE_BUY, FString::Printf(TEXT("buy %s %d"), *Who, I), [&](const FString& Key0, FClientMessage& M) { FGrpcNightfallV1PrivateStoreBuyRequest Q; Q.IdempotencyKey = Key0; Q.StoreOwner = Who; Q.Entries = Rows; M.PrivateStoreBuy = Q; });
 }
 else if (S->Kind == K::STORE_KIND_BUY)
 {
  const auto& Row = S->Buy[I]; const uint64 Id = Items.FindRef(Item->GetSelectedOption()); const uint64 N = *PositiveNumber(Count);
  Send(EOp::ITEM_OPERATION_STORE_SELL, FString::Printf(TEXT("sell %s %d %llu"), *Who, I, Id), [&](const FString& Key0, FClientMessage& M) {
   FGrpcNightfallV1PrivateStoreSellRequest Q; Q.IdempotencyKey = Key0; Q.StoreOwner = Who; FGrpcNightfallV1PrivateStoreSellEntry E; E.InstanceId = Id; E.TemplateId = Row.TemplateId; E.Count = N; E.Price = Row.Price; Q.Entries.Add(E); M.PrivateStoreSell = Q; });
 }
 else
 {
  const uint32 Recipe = S->Manufacture[I].RecipeId.Value;
  Send(EOp::ITEM_OPERATION_PRIVATE_MANUFACTURE, FString::Printf(TEXT("order %s %u"), *Who, Recipe), [&](const FString& Key0, FClientMessage& M) { FGrpcNightfallV1PrivateManufactureOrderRequest Q; Q.IdempotencyKey = Key0; Q.StoreOwner = Who; Q.RecipeId = Recipe; M.PrivateManufactureOrder = Q; });
 }
}

// --- warehouse keeper (story 5.5) --------------------------------------------------------------

void UNightfallKeeperDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Warehouse keeper — warehouse and freight"))) return;
 Tab = AddChoice(TEXT("Action")); for (const TCHAR* T : {TEXT("Deposit"), TEXT("Withdraw"), TEXT("Freight send"), TEXT("Freight withdraw")}) Tab->AddOption(T); Tab->SetSelectedIndex(0);
 Entry = AddChoice(TEXT("Item")); Count = AddNumber(TEXT("Count"), TEXT("1"));
 Target = AddNumber(TEXT("Freight to character (id)"), TEXT(""));
 AddCommonButtons(TEXT("Confirm"));
}
void UNightfallKeeperDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory, const FString& InKeeper)
{
 EnsureLayout(); Keeper = InKeeper.ToLower(); Bind(InEconomy, InInventory);
 if (Economy) Economy->OpenContainers();
 Render();
}
FString UNightfallKeeperDialog::StatusText() const { return Status ? Status->GetText().ToString() : FString(); }
void UNightfallKeeperDialog::Render()
{
 if (!Entry || !Economy || !Inventory) return;
 bRendering = true; const FString T = Tab->GetSelectedOption(), Was = Entry->GetSelectedOption();
 Entry->ClearOptions(); Choices.Reset();
 auto Row = [&](uint64 Id, uint32 Template, uint64 N) { const FString L = FString::Printf(TEXT("%s #%llu x%llu"), *ItemName(Template), Id, N); Choices.Add(L, Id); Entry->AddOption(L); };
 if (T == TEXT("Deposit") || T == TEXT("Freight send"))
 {
  TArray<uint64> Ids; Inventory->GetItems().GetKeys(Ids); Ids.Sort();
  for (uint64 Id : Ids) { const auto* I = Inventory->Find(Id); Row(Id, I->TemplateId.Value, I->Count.Value); }
 }
 else if (T == TEXT("Withdraw") && Economy->Warehouse().IsSet()) for (const auto& I : Economy->Warehouse()->Items) Row(I.InstanceId.Value, I.TemplateId.Value, I.Count.Value);
 else if (T == TEXT("Freight withdraw") && Economy->Freight().IsSet()) for (const auto& I : Economy->Freight()->Items) Row(I.InstanceId.Value, I.TemplateId.Value, I.Count.Value);
 if (Choices.Contains(Was)) Entry->SetSelectedOption(Was); else if (Entry->GetOptionCount() > 0) Entry->SetSelectedIndex(0);
 Target->SetIsEnabled(T == TEXT("Freight send"));
 bRendering = false;
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet());
 FString Text = Economy->Status();
 const auto& W = Economy->Warehouse(); const auto& F = Economy->Freight();
 Text += W.IsSet() ? FString::Printf(TEXT("\nWarehouse: %d rows, capacity %u"), W->Items.Num(), W->Capacity.Value) : FString(TEXT("\nWaiting for the warehouse"));
 Text += F.IsSet() ? FString::Printf(TEXT("\nFreight: %d rows, capacity %u"), F->Items.Num(), F->Capacity.Value) : FString(TEXT("\nWaiting for the freight"));
 Status->SetText(FText::FromString(Text + BusyNotice()));
}
bool UNightfallKeeperDialog::CanConfirm() const
{
 if (!Ready() || Keeper.IsEmpty() || !Choices.Contains(Entry->GetSelectedOption()) || !PositiveNumber(Count).IsSet()) return false;
 return Tab->GetSelectedOption() != TEXT("Freight send") || !Target->GetText().ToString().TrimStartAndEnd().IsEmpty();
}
void UNightfallKeeperDialog::Confirm()
{
 if (!CanConfirm()) return;
 const FString T = Tab->GetSelectedOption(); const uint64 Id = Choices.FindRef(Entry->GetSelectedOption()); const uint64 N = *PositiveNumber(Count);
 const FString To = Target->GetText().ToString().TrimStartAndEnd().ToLower(); const FString K0 = Keeper;
 FGrpcNightfallV1ItemCount C; C.InstanceId = Id; C.Count = N;
 const FString Args = FString::Printf(TEXT("%s %llu %llu %s"), *T, Id, N, *To);
 if (T == TEXT("Deposit")) Send(EOp::ITEM_OPERATION_WAREHOUSE_DEPOSIT, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1WarehouseDepositRequest Q; Q.IdempotencyKey = K; Q.Keeper = K0; Q.Entries.Add(C); M.WarehouseDeposit = Q; });
 else if (T == TEXT("Withdraw")) Send(EOp::ITEM_OPERATION_WAREHOUSE_WITHDRAW, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1WarehouseWithdrawRequest Q; Q.IdempotencyKey = K; Q.Keeper = K0; Q.Entries.Add(C); M.WarehouseWithdraw = Q; });
 else if (T == TEXT("Freight send")) Send(EOp::ITEM_OPERATION_FREIGHT_SEND, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1FreightSendRequest Q; Q.IdempotencyKey = K; Q.Keeper = K0; Q.TargetCharacter = To; Q.Entries.Add(C); M.FreightSend = Q; });
 else Send(EOp::ITEM_OPERATION_FREIGHT_WITHDRAW, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1FreightWithdrawRequest Q; Q.IdempotencyKey = K; Q.Keeper = K0; Q.Entries.Add(C); M.FreightWithdraw = Q; });
}

// --- mail (story 5.5) --------------------------------------------------------------------------

void UNightfallMailDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Mail"))) return;
 Box = AddChoice(TEXT("Box")); for (const TCHAR* B : {TEXT("Inbox"), TEXT("Sent"), TEXT("Write")}) Box->AddOption(B); Box->SetSelectedIndex(0);
 Message = AddChoice(TEXT("Message")); Action = AddChoice(TEXT("Action"));
 Read = AddText(TEXT(""));
 Receiver = AddNumber(TEXT("To (character name)"), TEXT("")); Subject = AddNumber(TEXT("Subject"), TEXT("")); Body = AddNumber(TEXT("Text"), TEXT(""));
 Cod = AddNumber(TEXT("Cash on delivery (adena, 0 = none)"), TEXT("0"));
 Attachment = AddChoice(TEXT("Attachment")); Count = AddNumber(TEXT("Count"), TEXT("1"));
 Attach = AddButton(TEXT("Attach")); Attach->OnClicked.AddDynamic(this, &UNightfallMailDialog::AddAttachment);
 Attached = AddText(TEXT("No attachments"));
 AddCommonButtons(TEXT("Confirm"));
}
void UNightfallMailDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory)
{
 EnsureLayout(); Bind(InEconomy, InInventory); if (Economy) Economy->OpenMailboxes(); Render();
}
FString UNightfallMailDialog::ReadText() const { return Read ? Read->GetText().ToString() : FString(); }
FString UNightfallMailDialog::SelectedMessage() const { return Messages.FindRef(Message->GetSelectedOption()); }
void UNightfallMailDialog::Render()
{
 if (!Message || !Economy || !Inventory) return;
 bRendering = true; const FString B = Box->GetSelectedOption(), WasM = Message->GetSelectedOption(), WasA = Action->GetSelectedOption(), WasI = Attachment->GetSelectedOption();
 Message->ClearOptions(); Action->ClearOptions(); Attachment->ClearOptions(); Messages.Reset(); Items.Reset();
 const bool bWrite = B == TEXT("Write"); const auto& View = B == TEXT("Sent") ? Economy->Outbox() : Economy->Inbox();
 if (!bWrite && View.IsSet())
  for (const auto& H : View->Messages)
  {
   // The received header flags: COD amount, attachments, read, returned.
   FString L = FString::Printf(TEXT("%s — %s"), *H.Subject, B == TEXT("Sent") ? *H.ReceiverName : *H.SenderName);
   if (H.CodAdena.Value) L += FString::Printf(TEXT(" [COD %llu adena]"), H.CodAdena.Value);
   if (H.HasAttachments) L += TEXT(" [attachments]"); if (H.Read) L += TEXT(" [read]"); if (H.Returned) L += TEXT(" [returned]");
   Messages.Add(L, H.MessageId.ToLower()); Message->AddOption(L);
  }
 if (Messages.Contains(WasM)) Message->SetSelectedOption(WasM); else if (Message->GetOptionCount() > 0) Message->SetSelectedIndex(0);
 TArray<FString> Actions;
 if (bWrite) Actions = { TEXT("Send") };
 else if (B == TEXT("Sent")) Actions = { TEXT("Read"), TEXT("Cancel"), TEXT("Delete") };
 else Actions = { TEXT("Read"), TEXT("Take attachments"), TEXT("Reject"), TEXT("Mark read"), TEXT("Delete") };
 for (const FString& A : Actions) Action->AddOption(A);
 if (Actions.Contains(WasA)) Action->SetSelectedOption(WasA); else Action->SetSelectedIndex(0);
 if (bWrite)
 {
  TSet<uint64> Used; for (const auto& A : Attachments) Used.Add(A.InstanceId.Value);
  TArray<uint64> Ids; Inventory->GetItems().GetKeys(Ids); Ids.Sort();
  for (uint64 Id : Ids) { if (Used.Contains(Id)) continue; const auto* I = Inventory->Find(Id); const FString L = FString::Printf(TEXT("%s #%llu x%llu"), *ItemName(I->TemplateId.Value), Id, I->Count.Value); Items.Add(L, Id); Attachment->AddOption(L); }
 }
 if (Items.Contains(WasI)) Attachment->SetSelectedOption(WasI); else if (Attachment->GetOptionCount() > 0) Attachment->SetSelectedIndex(0);
 for (UWidget* W : TArray<UWidget*>{ Receiver, Subject, Body, Cod, Attachment, Count }) W->SetIsEnabled(bWrite);
 // The read body is the ReadMail response; attachments appear only when the server sends them.
 FString Shown;
 if (const auto* R = bWrite ? nullptr : Economy->MailBody(SelectedMessage()))
 {
  Shown = FString::Printf(TEXT("%s\n%s"), *R->Header.Subject, *R->Body);
  for (const auto& I : R->Attachments) Shown += FString::Printf(TEXT("\n  %s x%llu"), *ItemName(I.TemplateId.Value), I.Count.Value);
 }
 Read->SetText(FText::FromString(Shown));
 TArray<FString> Lines; for (const auto& A : Attachments) { const auto* I = Inventory->Find(A.InstanceId.Value); Lines.Add(FString::Printf(TEXT("%s x%llu"), *ItemName(I ? I->TemplateId.Value : 0), A.Count.Value)); }
 Attached->SetText(FText::FromString(Lines.IsEmpty() ? FString(TEXT("No attachments")) : FString::Join(Lines, TEXT("\n"))));
 bRendering = false;
 Attach->SetIsEnabled(bWrite && Items.Contains(Attachment->GetSelectedOption()) && PositiveNumber(Count).IsSet());
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet());
 FString Text = Economy->Status(); if (!bWrite && View.IsSet()) Text += FString::Printf(TEXT("\n%u of %u messages"), View->Count.Value, View->Limit.Value);
 Status->SetText(FText::FromString(Text + BusyNotice()));
}
void UNightfallMailDialog::AddAttachment()
{
 const auto N = PositiveNumber(Count); if (!Items.Contains(Attachment->GetSelectedOption()) || !N.IsSet()) return;
 FGrpcNightfallV1ItemCount C; C.InstanceId = Items.FindRef(Attachment->GetSelectedOption()); C.Count = *N; Attachments.Add(C);
 NewKey(); FrozenOp.Reset(); Render();
}
bool UNightfallMailDialog::CanConfirm() const
{
 if (!Ready()) return false;
 const FString A = Action->GetSelectedOption();
 if (A == TEXT("Send"))
 {
  const FString Cash = Cod->GetText().ToString().TrimStartAndEnd();
  return !Receiver->GetText().ToString().TrimStartAndEnd().IsEmpty() && !Subject->GetText().ToString().TrimStartAndEnd().IsEmpty() && (Cash == TEXT("0") || PositiveNumber(Cod).IsSet());
 }
 return !SelectedMessage().IsEmpty();
}
void UNightfallMailDialog::Confirm()
{
 if (!CanConfirm()) return;
 const FString A = Action->GetSelectedOption(); const FString Id = SelectedMessage();
 if (A == TEXT("Read")) { Economy->OpenMail(Id); return; } // a read RPC: no intent, no key
 if (A == TEXT("Send"))
 {
  const FString To = Receiver->GetText().ToString().TrimStartAndEnd(), Sub = Subject->GetText().ToString().TrimStartAndEnd(), Text = Body->GetText().ToString();
  const uint64 Cash = PositiveNumber(Cod).Get(0);
  Send(EOp::ITEM_OPERATION_MAIL_SEND, FString::Printf(TEXT("send %s %llu %d"), *To, Cash, Attachments.Num()), [&](const FString& K, FClientMessage& M) {
   FGrpcNightfallV1MailSendRequest Q; Q.IdempotencyKey = K; Q.ReceiverName = To; Q.Subject = Sub; Q.Body = Text; Q.CodAdena = Cash; Q.Attachments = Attachments; M.MailSend = Q; });
  return;
 }
 const FString Args = A + TEXT(" ") + Id;
 if (A == TEXT("Take attachments")) Send(EOp::ITEM_OPERATION_MAIL_TAKE, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1MailTakeAttachmentsRequest Q; Q.IdempotencyKey = K; Q.MessageId = Id; M.MailTakeAttachments = Q; });
 else if (A == TEXT("Reject")) Send(EOp::ITEM_OPERATION_MAIL_REJECT, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1MailRejectRequest Q; Q.IdempotencyKey = K; Q.MessageId = Id; M.MailReject = Q; });
 else if (A == TEXT("Mark read")) Send(EOp::ITEM_OPERATION_MAIL_MARK_READ, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1MailMarkReadRequest Q; Q.IdempotencyKey = K; Q.MessageId = Id; M.MailMarkRead = Q; });
 else if (A == TEXT("Cancel")) Send(EOp::ITEM_OPERATION_MAIL_CANCEL, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1MailCancelRequest Q; Q.IdempotencyKey = K; Q.MessageId = Id; M.MailCancel = Q; });
 else
 {
  const bool bSent = Box->GetSelectedOption() == TEXT("Sent");
  Send(EOp::ITEM_OPERATION_MAIL_DELETE, Args, [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1MailDeleteRequest Q; Q.IdempotencyKey = K; Q.MessageIds.Add(Id); Q.SentBox = bSent; M.MailDelete = Q; });
 }
}

// --- item auction (story 5.5) ------------------------------------------------------------------

void UNightfallAuctionDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Item auction"))) return;
 Instance = AddNumber(TEXT("Auctioneer instance"), TEXT(""));
 Shown = AddText(TEXT("Waiting for the auction"));
 Action = AddChoice(TEXT("Action")); for (const TCHAR* A : {TEXT("Bid"), TEXT("Cancel bid")}) Action->AddOption(A); Action->SetSelectedIndex(0);
 Bid = AddNumber(TEXT("Bid (adena)"), TEXT(""));
 AddCommonButtons(TEXT("Confirm"));
}
void UNightfallAuctionDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory, const FString& InAuctioneer, uint32 InInstance)
{
 EnsureLayout(); Auctioneer = InAuctioneer.ToLower(); Bind(InEconomy, InInventory);
 InstanceId = InInstance ? InInstance : (Economy ? Economy->AuctionInstance() : 0);
 bRendering = true; Instance->SetText(FText::FromString(InstanceId ? FString::Printf(TEXT("%u"), InstanceId) : FString())); bRendering = false;
 if (Economy && InstanceId) Economy->OpenAuctions(InstanceId);
 Render();
}
FString UNightfallAuctionDialog::AuctionText() const { return Shown ? Shown->GetText().ToString() : FString(); }
void UNightfallAuctionDialog::Render()
{
 if (!Action || !Economy || !Inventory) return;
 if (const TOptional<uint64> Typed = PositiveNumber(Instance); Typed.IsSet() && *Typed <= MAX_uint32 && uint32(*Typed) != InstanceId) { InstanceId = uint32(*Typed); Economy->OpenAuctions(InstanceId); }
 const auto& V = Economy->AuctionView();
 auto StateName = [](EGrpcNightfallV1AuctionState S) { return S == EGrpcNightfallV1AuctionState::AUCTION_STATE_STARTED ? TEXT("started") : S == EGrpcNightfallV1AuctionState::AUCTION_STATE_FINISHED ? TEXT("finished") : S == EGrpcNightfallV1AuctionState::AUCTION_STATE_CREATED ? TEXT("created") : TEXT("unknown"); };
 auto Line = [&](const TCHAR* Label, const FGrpcNightfallV1AuctionView& A) {
  // The received bids: no client-side bid arithmetic.
  return FString::Printf(TEXT("%s: %s x%llu, %s, start %llu, highest %llu, your bid %llu%s"), Label, *ItemName(A.TemplateId.Value), A.Count.Value, StateName(A.State), A.InitBid.Value, A.HighestBid.Value, A.OwnBid.Value, A.OwnCancelled ? TEXT(" (cancelled)") : TEXT("")); };
 FString Text;
 if (V.IsSet() && Economy->AuctionInstance() == InstanceId)
 {
  if (V->HasCurrent) Text += Line(TEXT("Current"), V->Current); else Text += TEXT("No current auction");
  if (V->HasNext) Text += TEXT("\n") + Line(TEXT("Next"), V->Next);
  Text += FString::Printf(TEXT("\nBids you can cancel: %d"), V->OwnBids.Num());
 }
 else Text = TEXT("Waiting for the auction");
 Shown->SetText(FText::FromString(Text));
 Bid->SetIsEnabled(Action->GetSelectedOption() == TEXT("Bid"));
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet());
 Status->SetText(FText::FromString(Economy->Status() + BusyNotice()));
}
bool UNightfallAuctionDialog::CanConfirm() const
{
 if (!Ready() || !InstanceId) return false;
 if (Action->GetSelectedOption() == TEXT("Bid")) return PositiveNumber(Bid).IsSet();
 const auto& V = Economy->AuctionView(); return !Auctioneer.IsEmpty() && V.IsSet() && Economy->AuctionInstance() == InstanceId && !V->OwnBids.IsEmpty();
}
void UNightfallAuctionDialog::Confirm()
{
 if (!CanConfirm()) return;
 const uint32 Npc = InstanceId; const FString Who = Auctioneer;
 if (Action->GetSelectedOption() == TEXT("Bid"))
 {
  const uint64 Amount = *PositiveNumber(Bid);
  Send(EOp::ITEM_OPERATION_AUCTION_BID, FString::Printf(TEXT("%u %llu"), Npc, Amount), [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1ItemAuctionBidRequest Q; Q.IdempotencyKey = K; Q.InstanceNpcId = Npc; Q.Bid = Amount; M.ItemAuctionBid = Q; });
 }
 else Send(EOp::ITEM_OPERATION_AUCTION_CANCEL_BID, FString::Printf(TEXT("%u"), Npc), [&](const FString& K, FClientMessage& M) { FGrpcNightfallV1ItemAuctionCancelBidRequest Q; Q.IdempotencyKey = K; Q.Auctioneer = Who; Q.InstanceNpcId = Npc; M.ItemAuctionCancelBid = Q; });
}

// --- manor (story 5.5) -------------------------------------------------------------------------

void UNightfallManorDialog::EnsureLayout()
{
 if (!BeginLayout(TEXT("Manor manager — seeds and crops"))) return;
 Castle = AddNumber(TEXT("Manor (castle)"), TEXT("1"));
 Action = AddChoice(TEXT("Action")); for (const TCHAR* A : {TEXT("Buy seed"), TEXT("Sell crop")}) Action->AddOption(A); Action->SetSelectedIndex(0);
 Entry = AddChoice(TEXT("Seed or crop")); Item = AddChoice(TEXT("Your crop")); Count = AddNumber(TEXT("Count"), TEXT("1"));
 AddCommonButtons(TEXT("Confirm"));
}
void UNightfallManorDialog::BindState(UEconomyStateSubsystem* InEconomy, UInventoryStateSubsystem* InInventory, const FString& InManager, uint32 InCastle)
{
 EnsureLayout(); Manager = InManager.ToLower(); CastleId = InCastle ? InCastle : 1; Bind(InEconomy, InInventory);
 bRendering = true; Castle->SetText(FText::FromString(FString::Printf(TEXT("%u"), CastleId))); bRendering = false;
 if (Economy) Economy->OpenManor(CastleId);
 Render();
}
void UNightfallManorDialog::Render()
{
 if (!Entry || !Economy || !Inventory) return;
 if (const TOptional<uint64> Typed = PositiveNumber(Castle); Typed.IsSet() && *Typed <= MAX_uint32 && uint32(*Typed) != CastleId) { CastleId = uint32(*Typed); Economy->OpenManor(CastleId); }
 bRendering = true; const FString A = Action->GetSelectedOption(), Was = Entry->GetSelectedOption(), WasI = Item->GetSelectedOption();
 Entry->ClearOptions(); Item->ClearOptions(); Entries.Reset(); Items.Reset();
 const auto& V = Economy->Manor(); const bool bView = V.IsSet() && V->CastleId.Value == CastleId;
 if (bView && A == TEXT("Buy seed"))
  for (int32 I = 0; I < V->Seeds.Num(); ++I) { const auto& S = V->Seeds[I]; const FString L = FString::Printf(TEXT("%s — %llu adena each, %llu/%llu left"), *ItemName(S.SeedId.Value), S.Price.Value, S.Amount.Value, S.StartAmount.Value); Entries.Add(L, I); Entry->AddOption(L); }
 if (bView && A == TEXT("Sell crop"))
  for (int32 I = 0; I < V->Crops.Num(); ++I) { const auto& C = V->Crops[I]; const FString L = FString::Printf(TEXT("%s — %llu adena each, %llu/%llu wanted, reward %u"), *ItemName(C.CropId.Value), C.Price.Value, C.Amount.Value, C.StartAmount.Value, C.RewardType.Value); Entries.Add(L, I); Entry->AddOption(L); }
 if (Entries.Contains(Was)) Entry->SetSelectedOption(Was); else if (Entry->GetOptionCount() > 0) Entry->SetSelectedIndex(0);
 if (bView && A == TEXT("Sell crop") && Entries.Contains(Entry->GetSelectedOption()))
 {
  const uint32 Crop = V->Crops[Entries.FindRef(Entry->GetSelectedOption())].CropId.Value;
  TArray<uint64> Ids; Inventory->GetItems().GetKeys(Ids); Ids.Sort();
  for (uint64 Id : Ids) { const auto* I = Inventory->Find(Id); if (I->TemplateId.Value != Crop) continue; const FString L = FString::Printf(TEXT("%s #%llu x%llu"), *ItemName(Crop), Id, I->Count.Value); Items.Add(L, Id); Item->AddOption(L); }
 }
 if (Items.Contains(WasI)) Item->SetSelectedOption(WasI); else if (Item->GetOptionCount() > 0) Item->SetSelectedIndex(0);
 Item->SetIsEnabled(A == TEXT("Sell crop"));
 bRendering = false;
 ConfirmButton->SetIsEnabled(CanConfirm()); RetryButton->SetIsEnabled(Ready() && FrozenOp.IsSet());
 FString Text = Economy->Status(); if (!bView) Text += TEXT("\nWaiting for the manor data");
 Status->SetText(FText::FromString(Text + BusyNotice()));
}
bool UNightfallManorDialog::CanConfirm() const
{
 if (!Ready() || Manager.IsEmpty() || !Entries.Contains(Entry->GetSelectedOption()) || !PositiveNumber(Count).IsSet()) return false;
 return Action->GetSelectedOption() == TEXT("Buy seed") || Items.Contains(Item->GetSelectedOption());
}
void UNightfallManorDialog::Confirm()
{
 if (!CanConfirm()) return;
 const auto& V = *Economy->Manor(); const int32 I = Entries.FindRef(Entry->GetSelectedOption()); const uint64 N = *PositiveNumber(Count); const uint32 Manor = CastleId; const FString Who = Manager;
 if (Action->GetSelectedOption() == TEXT("Buy seed"))
 {
  const uint32 Seed = V.Seeds[I].SeedId.Value;
  Send(EOp::ITEM_OPERATION_MANOR_BUY_SEED, FString::Printf(TEXT("seed %u %u %llu"), Manor, Seed, N), [&](const FString& K, FClientMessage& M) {
   FGrpcNightfallV1ManorBuySeedRequest Q; Q.IdempotencyKey = K; Q.Manager = Who; Q.ManorId = Manor; FGrpcNightfallV1TemplateCount T; T.TemplateId = Seed; T.Count = N; Q.Seeds.Add(T); M.ManorBuySeed = Q; });
 }
 else
 {
  const uint32 Crop = V.Crops[I].CropId.Value; const uint64 Id = Items.FindRef(Item->GetSelectedOption());
  Send(EOp::ITEM_OPERATION_MANOR_SELL_CROP, FString::Printf(TEXT("crop %u %u %llu %llu"), Manor, Crop, Id, N), [&](const FString& K, FClientMessage& M) {
   FGrpcNightfallV1ManorSellCropRequest Q; Q.IdempotencyKey = K; Q.Manager = Who; FGrpcNightfallV1CropSale C; C.InstanceId = Id; C.CropId = Crop; C.ManorId = Manor; C.Count = N; Q.Entries.Add(C); M.ManorSellCrop = Q; });
 }
}
