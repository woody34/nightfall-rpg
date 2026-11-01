#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Economy/EconomyStateSubsystem.h"
#include "Items/InventoryStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallWire.h"
#include "UI/NightfallEconomyDialog.h"
#include "Components/Button.h"
#include "Components/ComboBoxString.h"
#include "Components/EditableTextBox.h"
#include "IWebSocket.h"
#if WITH_DEV_AUTOMATION_TESTS
// Story 5.3: the vendor, multisell and recipe-book dialogs. Each test drives the real widgets: the
// shown values are the received ones, one key per frozen request, Retry resends the same key and
// body (D43), a changed selection is a new key, and busy/rejection gate the controls.
namespace EconomyUiTest
{
 const TCHAR* Owner = TEXT("01970000-0000-7000-8000-000000000060");
 const TCHAR* Merchant = TEXT("01970000-0000-7000-8000-00000000a061");
 const TCHAR* Seller = TEXT("01970000-0000-7000-8000-000000000062");
 class FSocket final : public IWebSocket
 {
 public:
  TArray<TArray<uint8>> Sent;
  void Connect() override {} void Close(int32, const FString&) override {} bool IsConnected() override { return true; }
  void Send(const FString&) override {} void Send(const void* Data, SIZE_T Size, bool) override { Sent.Emplace(static_cast<const uint8*>(Data), int32(Size)); }
  void SetTextMessageMemoryLimit(uint64) override {}
  FWebSocketConnectedEvent Connected; FWebSocketConnectionErrorEvent Error; FWebSocketClosedEvent Closed; FWebSocketMessageEvent Message; FWebSocketBinaryMessageEvent Binary; FWebSocketRawMessageEvent Raw; FWebSocketMessageSentEvent MessageSent;
  FWebSocketConnectedEvent& OnConnected() override { return Connected; } FWebSocketConnectionErrorEvent& OnConnectionError() override { return Error; }
  FWebSocketClosedEvent& OnClosed() override { return Closed; } FWebSocketMessageEvent& OnMessage() override { return Message; } FWebSocketBinaryMessageEvent& OnBinaryMessage() override { return Binary; }
  FWebSocketRawMessageEvent& OnRawMessage() override { return Raw; } FWebSocketMessageSentEvent& OnMessageSent() override { return MessageSent; }
 };
 struct FScope
 {
  FScopedTestGameInstance Scope; UNetClientSubsystem* Net; UEconomyStateSubsystem* Economy; UInventoryStateSubsystem* Items; TSharedPtr<FSocket> Socket;
  FScope()
  {
   Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
   Net = Scope.Get<UNetClientSubsystem>(); Economy = Scope.Get<UEconomyStateSubsystem>(); Items = Scope.Get<UInventoryStateSubsystem>();
   Net->SetSocketFactoryForTesting([this](const FWsUpgradeRequest&) -> TSharedRef<IWebSocket> { Socket = MakeShared<FSocket>(); return Socket.ToSharedRef(); });
   Net->SetSchedulerForTesting([](float, TFunction<void()>) {}); Net->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket")); Socket->Connected.Broadcast(); Net->SetOwnEntityId(Owner);
   FEntitySpawn S; S.EntityId = Owner; S.Kind = 1; S.StateTick = 10; S.LifeIncarnation = 1; S.Position = FNetVec2{ 100.f, 100.f }; FWorldEvent E; E.Spawn = S; Dispatch(E);
   FGrpcNightfallV1InventorySync Sync; Sync.EntityId = NightfallItems::UuidBytes(Owner); Sync.Incarnation = 1; Sync.Tick = Sync.BaselineTick = 10; Sync.Baseline = true; Sync.InventoryRevision = 1; Sync.HasWeight = Sync.HasPenalties = true; Sync.RulesHash.Value.Init(1, 32);
   auto Item = [&](uint64 Id, uint32 Template, uint64 Count) { FGrpcNightfallV1OwnedItem I; I.InstanceId = Id; I.TemplateId = Template; I.Count = Count; I.Location = EGrpcNightfallV1ItemLocation::ITEM_LOCATION_INVENTORY; Sync.Items.Add(I); };
   Item(1, 1864, 10); Item(2, 57, 500); Item(3, 1666, 1);
   FWorldEvent I; I.InventorySync = Sync; Dispatch(I);
  }
  void Dispatch(const FWorldEvent& E) { FServerMessage M; M.Event = E; Net->DispatchServerMessage(M); }
  FGrpcNightfallV1ClientMessage Last() { FGrpcNightfallV1ClientMessage M; NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(), Socket->Sent.Last().Num(), M); return M; }
  void Reject(uint64 Seq) { FServerMessage M; M.Rejected = FIntentRejected{ Seq, uint32(EGrpcNightfallV1RejectReason::REJECT_REASON_CONTAINER_BUSY), TEXT("busy") }; Net->DispatchServerMessage(M); }
 };
 /** A user edit: UMG's SetText does not raise OnTextChanged, so the test raises it as Slate does. */
 void Type(UEditableTextBox* Box, const TCHAR* Text) { Box->SetText(FText::FromString(Text)); Box->OnTextChanged.Broadcast(FText::FromString(Text)); }
 FGrpcNightfallV1ListBuylistResponse Buylist()
 {
  FGrpcNightfallV1ListBuylistResponse R; R.ListId = 353630; R.MerchantNpcId = 35363; R.CastleTaxPercent = 0; R.StateTick = 77;
  FGrpcNightfallV1BuylistEntry A; A.TemplateId = 8751; A.UnitPrice = 950400; A.Limited = true; A.Stock = 0; A.MaxStock = 1; R.Entries.Add(A);
  FGrpcNightfallV1BuylistEntry B; B.TemplateId = 1864; B.UnitPrice = 37; R.Entries.Add(B);
  FGrpcNightfallV1OwnedItem Refund; Refund.InstanceId = 90; Refund.TemplateId = 1666; Refund.Count = 1; Refund.Location = EGrpcNightfallV1ItemLocation::ITEM_LOCATION_REFUND; R.RefundTab.Add(Refund); return R;
 }
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyVendorUiTest, "Nightfall.Economy.UI.VendorBuySellRefundRetry", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyVendorUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; FScope S; int32 Requests = 0; FString AskedMerchant;
 S.Economy->SetBuylistProviderForTesting([&](const FGrpcNightfallV1ListBuylistRequest& Q, USessionClient::FListBuylistCallback Cb) { ++Requests; AskedMerchant = Q.Merchant; Cb({}, Buylist()); });
 auto* D = NewObject<UNightfallVendorDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items, Merchant);
 TestEqual(TEXT("one ListBuylist for the target merchant"), Requests, 1); TestEqual(TEXT("the merchant entity is asked"), AskedMerchant, FString(Merchant).ToLower());
 // The received price and stock are shown as received: no client-side price math.
 TestTrue(TEXT("received unit price"), D->BuyLabel(0).Contains(TEXT("950400 adena each")));
 TestTrue(TEXT("received stock"), D->BuyLabel(0).Contains(TEXT("stock 0/1")));
 TestTrue(TEXT("unlimited entry has no stock"), D->BuyLabel(1).Contains(TEXT("37 adena each")) && !D->BuyLabel(1).Contains(TEXT("stock")));
 D->EntryControl()->SetSelectedIndex(1); Type(D->CountControl(), TEXT("3"));
 const FString KeyBefore = D->RetryKey(); TestTrue(TEXT("buy can be confirmed"), D->ConfirmControl()->GetIsEnabled());
 D->ConfirmControl()->OnClicked.Broadcast(); if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage First = S.Last();
 TestTrue(TEXT("vendor buy body"), First.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::VendorBuy && First.Intent.VendorBuy.Merchant == FString(Merchant).ToLower() && First.Intent.VendorBuy.ListId.Value == 353630 && First.Intent.VendorBuy.Items.Num() == 1 && First.Intent.VendorBuy.Items[0].TemplateId.Value == 1864 && First.Intent.VendorBuy.Items[0].Count.Value == 3);
 TestEqual(TEXT("the dialog key is sent"), First.Intent.VendorBuy.IdempotencyKey, D->RetryKey()); TestEqual(TEXT("first send uses the shown key"), D->RetryKey(), KeyBefore);
 TestFalse(TEXT("busy disables confirm"), D->ConfirmControl()->GetIsEnabled());
 S.Reject(First.Seq.Value); TestTrue(TEXT("rejection re-enables retry"), D->RetryControl()->GetIsEnabled());
 // Retry resends the same key and the same body (D43).
 D->RetryControl()->OnClicked.Broadcast(); if (!TestEqual(TEXT("retry sends"), S.Socket->Sent.Num(), 2)) return false;
 const FGrpcNightfallV1ClientMessage Second = S.Last();
 TestEqual(TEXT("same frozen body"), Second.Intent.VendorBuy.ToJsonString(false), First.Intent.VendorBuy.ToJsonString(false)); TestTrue(TEXT("fresh sequence"), Second.Seq.Value > First.Seq.Value);
 S.Reject(Second.Seq.Value);
 // A changed count is a new request: a new key.
 Type(D->CountControl(), TEXT("4")); TestNotEqual(TEXT("a changed count makes a new key"), D->RetryKey(), KeyBefore);
 Type(D->CountControl(), TEXT("0")); TestFalse(TEXT("zero count refused locally"), D->ConfirmControl()->GetIsEnabled());
 // Sell: an owned instance; refund: the received refund-tab index.
 D->ModeControl()->SetSelectedOption(TEXT("Sell")); Type(D->CountControl(), TEXT("2")); D->EntryControl()->SetSelectedIndex(0);
 D->ConfirmControl()->OnClicked.Broadcast(); const FGrpcNightfallV1ClientMessage Sell = S.Last();
 TestTrue(TEXT("vendor sell body"), Sell.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::VendorSell && Sell.Intent.VendorSell.Items.Num() == 1 && Sell.Intent.VendorSell.Items[0].InstanceId.Value == 1 && Sell.Intent.VendorSell.Items[0].Count.Value == 2);
 TestNotEqual(TEXT("sell has its own key"), Sell.Intent.VendorSell.IdempotencyKey, First.Intent.VendorBuy.IdempotencyKey); S.Reject(Sell.Seq.Value);
 D->ModeControl()->SetSelectedOption(TEXT("Refund")); TestEqual(TEXT("refund tab entries"), D->EntryControl()->GetOptionCount(), 1);
 D->ConfirmControl()->OnClicked.Broadcast(); const FGrpcNightfallV1ClientMessage Refund = S.Last();
 TestTrue(TEXT("vendor refund body"), Refund.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::VendorRefund && Refund.Intent.VendorRefund.RefundIndices.Num() == 1 && Refund.Intent.VendorRefund.RefundIndices[0].Value == 0);
 // A transport reset clears the view: nothing can be confirmed.
 S.Economy->Reset(); TestFalse(TEXT("reset blocks confirm"), D->ConfirmControl()->GetIsEnabled());
 S.Net->Disconnect(); return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyMultisellUiTest, "Nightfall.Economy.UI.MultisellEntriesAndRetry", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyMultisellUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; FScope S; TArray<uint32> Asked;
 S.Economy->SetMultisellProviderForTesting([&](const FGrpcNightfallV1GetMultisellListRequest& Q, USessionClient::FGetMultisellListCallback Cb) {
  Asked.Add(Q.ListId.Value); FGrpcNightfallV1GetMultisellListResponse R; R.ListId = Q.ListId; R.MaintainEnchantment = true;
  FGrpcNightfallV1MultisellEntry E; E.EntryId = 2; FGrpcNightfallV1MultisellIngredient In; In.TemplateId = 1864; In.Count = 5; E.Ingredients.Add(In); FGrpcNightfallV1MultisellIngredient Out; Out.TemplateId = 1666; Out.Count = 1; E.Products.Add(Out); R.Entries.Add(E); Cb({}, R); });
 auto* D = NewObject<UNightfallMultisellDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items, Merchant, 9001);
 TestEqual(TEXT("one GetMultisellList"), Asked.Num(), 1); TestEqual(TEXT("the shown list"), Asked.IsEmpty() ? 0u : Asked[0], 9001u);
 TestTrue(TEXT("received ingredients and products"), D->EntryLabel(2).Contains(TEXT("x5")) && D->EntryLabel(2).Contains(TEXT("x1")) && D->EntryLabel(2).StartsWith(TEXT("#2")));
 D->BindState(S.Economy, S.Items, Merchant, 9001); TestEqual(TEXT("static list is cached"), Asked.Num(), 1);
 Type(D->AmountControl(), TEXT("2")); D->ConfirmControl()->OnClicked.Broadcast();
 if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage First = S.Last();
 TestTrue(TEXT("multisell body"), First.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::MultisellChoose && First.Intent.MultisellChoose.Npc == FString(Merchant).ToLower() && First.Intent.MultisellChoose.ListId.Value == 9001 && First.Intent.MultisellChoose.EntryId.Value == 2 && First.Intent.MultisellChoose.Amount.Value == 2);
 TestEqual(TEXT("dialog key"), First.Intent.MultisellChoose.IdempotencyKey, D->RetryKey());
 S.Reject(First.Seq.Value); D->RetryControl()->OnClicked.Broadcast(); const FGrpcNightfallV1ClientMessage Second = S.Last();
 TestEqual(TEXT("retry resends the frozen body"), Second.Intent.MultisellChoose.ToJsonString(false), First.Intent.MultisellChoose.ToJsonString(false));
 S.Reject(Second.Seq.Value);
 // Another typed list is requested once.
 Type(D->ListControl(), TEXT("9002")); TestEqual(TEXT("a typed list id is requested"), Asked.Last(), 9002u);
 S.Net->Disconnect(); return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyRecipeUiTest, "Nightfall.Economy.UI.RecipeBookCraftRegisterManufacture", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyRecipeUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; FScope S;
 auto* D = NewObject<UNightfallRecipeDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items);
 TestTrue(TEXT("no book yet"), D->BookText().Contains(TEXT("Waiting"))); TestFalse(TEXT("no craft without the book"), D->ConfirmControl()->GetIsEnabled());
 { FWorldEvent E; FGrpcNightfallV1RecipeBookSync B; B.Baseline = true; B.Incarnation = 1; B.Tick = 11; B.Dwarven.Add(FUInt32(322)); B.Common.Add(FUInt32(7)); B.DwarvenLimit = 50; B.CommonLimit = 50; E.RecipeBookSync = B; S.Dispatch(E); }
 TestTrue(TEXT("received book and limits"), D->BookText().Contains(TEXT("Dwarven 1/50: 322")) && D->BookText().Contains(TEXT("Common 1/50: 7")));
 TestEqual(TEXT("both recipes offered for craft"), D->ChoiceControl()->GetOptionCount(), 2);
 D->ChoiceControl()->SetSelectedOption(TEXT("Dwarven recipe 322")); D->ConfirmControl()->OnClicked.Broadcast();
 if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage Craft = S.Last();
 TestTrue(TEXT("craft body"), Craft.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::CraftItem && Craft.Intent.CraftItem.RecipeId.Value == 322 && Craft.Intent.CraftItem.IdempotencyKey == D->RetryKey());
 S.Reject(Craft.Seq.Value); D->RetryControl()->OnClicked.Broadcast(); TestEqual(TEXT("craft retry keeps the key"), S.Last().Intent.CraftItem.IdempotencyKey, Craft.Intent.CraftItem.IdempotencyKey); S.Reject(S.Last().Seq.Value);
 // Register from an owned recipe item, delete a registered id.
 D->ActionControl()->SetSelectedOption(TEXT("Register")); TestEqual(TEXT("owned items offered"), D->ChoiceControl()->GetOptionCount(), 3);
 D->ChoiceControl()->SetSelectedIndex(2); D->ConfirmControl()->OnClicked.Broadcast();
 const FGrpcNightfallV1ClientMessage Register = S.Last();
 TestTrue(TEXT("register body"), Register.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::RegisterRecipe && Register.Intent.RegisterRecipe.RecipeInstanceId.Value == 3); S.Reject(Register.Seq.Value);
 D->ActionControl()->SetSelectedOption(TEXT("Delete")); D->ChoiceControl()->SetSelectedOption(TEXT("Common recipe 7")); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("delete body"), S.Last().Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::DeleteRecipe && S.Last().Intent.DeleteRecipe.RecipeId.Value == 7); S.Reject(S.Last().Seq.Value);
 // Manufacture: a received store's recipe and cost; the order names the store owner.
 { FWorldEvent E; FGrpcNightfallV1PrivateStoreChanged Store; Store.Owner = Seller; Store.Kind = EGrpcNightfallV1StoreKind::STORE_KIND_MANUFACTURE; Store.Title = TEXT("Forge"); Store.Tick = 12; FGrpcNightfallV1ManufactureEntry M; M.RecipeId = 322; M.Cost = 1500; Store.Manufacture.Add(M); E.PrivateStoreChanged = Store; S.Dispatch(E); }
 D->ActionControl()->SetSelectedOption(TEXT("Manufacture")); TestEqual(TEXT("one manufacture entry"), D->ChoiceControl()->GetOptionCount(), 1);
 TestTrue(TEXT("received cost shown"), D->ChoiceControl()->GetSelectedOption().Contains(TEXT("1500 adena")));
 D->ConfirmControl()->OnClicked.Broadcast(); const FGrpcNightfallV1ClientMessage Order = S.Last();
 TestTrue(TEXT("manufacture order body"), Order.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::PrivateManufactureOrder && Order.Intent.PrivateManufactureOrder.StoreOwner == Seller && Order.Intent.PrivateManufactureOrder.RecipeId.Value == 322);
 S.Net->Disconnect(); return true;
}
#endif
