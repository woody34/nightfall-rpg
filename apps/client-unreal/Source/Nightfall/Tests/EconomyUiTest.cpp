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
 TestTrue(TEXT("multisell body"), First.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::MultisellChoose && First.Intent.MultisellChoose.Npc == FString(Merchant).ToLower() && First.Intent.MultisellChoose.ListId.Value == 9001 && First.Intent.MultisellChoose.EntryId.Value == 200000 && First.Intent.MultisellChoose.Amount.Value == 2);
 TestEqual(TEXT("dialog key"), First.Intent.MultisellChoose.IdempotencyKey, D->RetryKey());
 S.Reject(First.Seq.Value); D->RetryControl()->OnClicked.Broadcast(); const FGrpcNightfallV1ClientMessage Second = S.Last();
 TestEqual(TEXT("retry resends the frozen body"), Second.Intent.MultisellChoose.ToJsonString(false), First.Intent.MultisellChoose.ToJsonString(false));
 S.Reject(Second.Seq.Value);
 // A maintained list takes the kept enchant: the prepared id is entry x 100000 + enchant (Z lane 3.7).
 Type(D->EnchantControl(), TEXT("3")); D->ConfirmControl()->OnClicked.Broadcast();
 TestEqual(TEXT("prepared entry id with the enchant"), S.Last().Intent.MultisellChoose.EntryId.Value, 200003u); S.Reject(S.Last().Seq.Value);
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
// Story 5.4: the trade window and the private-store windows, over the received state only.
namespace EconomyUiTest
{
 void Trade(FScope& S, EGrpcNightfallV1TradePhase Phase, uint64 Tick, bool bOwn, bool bPartner, TArray<uint64> OwnIds = {}, const TCHAR* Reason = TEXT(""))
 {
  FGrpcNightfallV1TradeSessionChanged T; T.Partner = Seller; T.Phase = Phase; T.Tick = Tick; T.OwnConfirmed = bOwn; T.PartnerConfirmed = bPartner; T.CancelReason = Reason;
  for (uint64 Id : OwnIds) { FGrpcNightfallV1TradeEntry E; E.InstanceId = Id; E.TemplateId = 1864; E.Count = 4; T.Own.Add(E); }
  FGrpcNightfallV1TradeEntry P; P.InstanceId = 70; P.TemplateId = 1666; P.Count = 1; P.Enchant = 2; T.PartnerItems.Add(P);
  FWorldEvent E; E.TradeSessionChanged = T; S.Dispatch(E);
 }
 void Store(FScope& S, const TCHAR* Who, EGrpcNightfallV1StoreKind Kind, uint64 Tick, TFunction<void(FGrpcNightfallV1PrivateStoreChanged&)> Fill = nullptr)
 {
  FGrpcNightfallV1PrivateStoreChanged C; C.Owner = Who; C.Kind = Kind; C.Title = TEXT("Shop"); C.Tick = Tick; if (Fill) Fill(C);
  FWorldEvent E; E.PrivateStoreChanged = C; S.Dispatch(E);
 }
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyTradeUiTest, "Nightfall.Economy.UI.TradeRequestAnswerAddConfirmCancel", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyTradeUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; using P = EGrpcNightfallV1TradePhase; using I = EGrpcNightfallV1ClientMessageIntent; FScope S;
 auto* D = NewObject<UNightfallTradeDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items, Seller);
 TestEqual(TEXT("no session: only a request to the target"), D->ActionControl()->GetOptionCount(), 1);
 D->ConfirmControl()->OnClicked.Broadcast(); if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage Request = S.Last();
 TestTrue(TEXT("trade request body"), Request.Intent.IntentCase == I::TradeRequest && Request.Intent.TradeRequest.Target == FString(Seller).ToLower() && Request.Intent.TradeRequest.IdempotencyKey == D->RetryKey());
 S.Reject(Request.Seq.Value); D->RetryControl()->OnClicked.Broadcast();
 TestEqual(TEXT("retry keeps the frozen request"), S.Last().Intent.TradeRequest.ToJsonString(false), Request.Intent.TradeRequest.ToJsonString(false)); S.Reject(S.Last().Seq.Value);
 // A received request: accept or decline only.
 { FGrpcNightfallV1TradeRequestReceived R; R.Requester = Seller; R.ExpiresTick = 140; FWorldEvent E; E.TradeRequestReceived = R; S.Dispatch(E); }
 TestTrue(TEXT("received request shown"), D->SessionText().Contains(TEXT("Trade request from")) && D->SessionText().Contains(TEXT("140")));
 TestEqual(TEXT("accept and decline"), D->ActionControl()->GetOptionCount(), 2);
 D->ActionControl()->SetSelectedOption(TEXT("Accept")); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("answer accept body"), S.Last().Intent.IntentCase == I::TradeAnswer && S.Last().Intent.TradeAnswer.Accept); TestNotEqual(TEXT("a new request has a new key"), S.Last().Intent.TradeAnswer.IdempotencyKey, Request.Intent.TradeRequest.IdempotencyKey); S.Reject(S.Last().Seq.Value);
 // The received session: both lists, the partner's enchant, and the add/confirm/cancel actions.
 Trade(S, P::TRADE_PHASE_STARTED, 20, false, false);
 TestTrue(TEXT("partner and phase"), D->SessionText().Contains(TEXT("started")) && D->SessionText().Contains(TEXT("+2 x1")));
 TestEqual(TEXT("open session actions"), D->ActionControl()->GetOptionCount(), 3);
 D->ActionControl()->SetSelectedOption(TEXT("Add item")); TestEqual(TEXT("owned items offered"), D->ItemControl()->GetOptionCount(), 3);
 D->ItemControl()->SetSelectedIndex(0); Type(D->CountControl(), TEXT("4")); D->ConfirmControl()->OnClicked.Broadcast();
 const FGrpcNightfallV1ClientMessage Add = S.Last();
 TestTrue(TEXT("add item body"), Add.Intent.IntentCase == I::TradeAddItem && Add.Intent.TradeAddItem.InstanceId.Value == 1 && Add.Intent.TradeAddItem.Count.Value == 4); S.Reject(Add.Seq.Value);
 Trade(S, P::TRADE_PHASE_ITEM_ADDED, 21, false, true, {1});
 TestEqual(TEXT("an offered item is not offered again"), D->ItemControl()->GetOptionCount(), 2);
 TestTrue(TEXT("own list and partner confirmation"), D->SessionText().Contains(TEXT("x4")) && D->SessionText().Contains(TEXT("Partner offers (confirmed)")));
 Trade(S, P::TRADE_PHASE_STARTED, 19, false, false); TestTrue(TEXT("an older session tick is ignored"), D->SessionText().Contains(TEXT("item added")));
 D->ActionControl()->SetSelectedOption(TEXT("Confirm")); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("confirm body"), S.Last().Intent.IntentCase == I::TradeConfirm); S.Reject(S.Last().Seq.Value);
 Trade(S, P::TRADE_PHASE_PARTNER_CONFIRMED, 22, true, true, {1}); D->ActionControl()->SetSelectedOption(TEXT("Confirm"));
 TestFalse(TEXT("confirmed once: no second confirm"), D->ConfirmControl()->GetIsEnabled());
 D->ActionControl()->SetSelectedOption(TEXT("Cancel")); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("cancel body"), S.Last().Intent.IntentCase == I::TradeCancel); S.Reject(S.Last().Seq.Value);
 Trade(S, P::TRADE_PHASE_CANCELLED, 23, false, false, {}, TEXT("too_far"));
 TestTrue(TEXT("cancel reason shown"), D->SessionText().Contains(TEXT("cancelled")) && D->SessionText().Contains(TEXT("too_far")));
 TestEqual(TEXT("a closed session allows a new request"), D->ActionControl()->GetSelectedOption(), FString(TEXT("Request")));
 S.Economy->Reset(); TestTrue(TEXT("reset clears the session"), D->SessionText().StartsWith(TEXT("No trade")));
 S.Net->Disconnect(); return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyStoreSetupUiTest, "Nightfall.Economy.UI.PrivateStoreSetupSellBuyManufactureClose", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyStoreSetupUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; using K = EGrpcNightfallV1StoreKind; using I = EGrpcNightfallV1ClientMessageIntent; FScope S;
 auto* D = NewObject<UNightfallStoreSetupDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items);
 TestTrue(TEXT("closed"), D->OpenStoreText().Contains(TEXT("closed"))); TestFalse(TEXT("no entries: nothing to open"), D->ConfirmControl()->GetIsEnabled());
 // Sell: two owned items, collected locally, sent in one request.
 Type(D->TitleControl(), TEXT("Cheap")); D->EntryControl()->SetSelectedIndex(0); Type(D->CountControl(), TEXT("5")); Type(D->PriceControl(), TEXT("30"));
 D->AddControl()->OnClicked.Broadcast(); TestEqual(TEXT("one entry"), D->PendingCount(), 1);
 TestEqual(TEXT("a listed item is not offered again"), D->EntryControl()->GetOptionCount(), 2);
 D->EntryControl()->SetSelectedIndex(1); Type(D->CountControl(), TEXT("1")); Type(D->PriceControl(), TEXT("900")); D->AddControl()->OnClicked.Broadcast();
 const FString KeyBefore = D->RetryKey(); D->ConfirmControl()->OnClicked.Broadcast();
 if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage Sell = S.Last();
 TestTrue(TEXT("sell store body"), Sell.Intent.IntentCase == I::SetPrivateStoreSell && !Sell.Intent.SetPrivateStoreSell.PackageSale && Sell.Intent.SetPrivateStoreSell.Title == TEXT("Cheap") && Sell.Intent.SetPrivateStoreSell.Entries.Num() == 2
  && Sell.Intent.SetPrivateStoreSell.Entries[0].InstanceId.Value == 1 && Sell.Intent.SetPrivateStoreSell.Entries[0].TemplateId.Value == 1864 && Sell.Intent.SetPrivateStoreSell.Entries[0].Count.Value == 5 && Sell.Intent.SetPrivateStoreSell.Entries[0].Price.Value == 30);
 TestEqual(TEXT("the shown key is sent"), Sell.Intent.SetPrivateStoreSell.IdempotencyKey, KeyBefore);
 S.Reject(Sell.Seq.Value); D->RetryControl()->OnClicked.Broadcast(); TestEqual(TEXT("retry resends the frozen store"), S.Last().Intent.SetPrivateStoreSell.ToJsonString(false), Sell.Intent.SetPrivateStoreSell.ToJsonString(false)); S.Reject(S.Last().Seq.Value);
 // Package sell keeps the flag; another kind drops the collected entries.
 D->KindControl()->SetSelectedOption(TEXT("Package sell")); TestEqual(TEXT("a kind change drops the entries"), D->PendingCount(), 0);
 D->EntryControl()->SetSelectedIndex(0); Type(D->CountControl(), TEXT("10")); Type(D->PriceControl(), TEXT("25")); D->AddControl()->OnClicked.Broadcast(); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("package sell body"), S.Last().Intent.IntentCase == I::SetPrivateStoreSell && S.Last().Intent.SetPrivateStoreSell.PackageSale && S.Last().Intent.SetPrivateStoreSell.Entries.Num() == 1); S.Reject(S.Last().Seq.Value);
 // Buy: a template, a count and a unit price.
 D->KindControl()->SetSelectedOption(TEXT("Buy")); Type(D->TemplateControl(), TEXT("1870")); Type(D->CountControl(), TEXT("5")); Type(D->PriceControl(), TEXT("300"));
 D->AddControl()->OnClicked.Broadcast(); D->ConfirmControl()->OnClicked.Broadcast();
 const FGrpcNightfallV1ClientMessage Buy = S.Last();
 TestTrue(TEXT("buy store body"), Buy.Intent.IntentCase == I::SetPrivateStoreBuy && Buy.Intent.SetPrivateStoreBuy.Entries.Num() == 1 && Buy.Intent.SetPrivateStoreBuy.Entries[0].TemplateId.Value == 1870 && Buy.Intent.SetPrivateStoreBuy.Entries[0].Count.Value == 5 && Buy.Intent.SetPrivateStoreBuy.Entries[0].Price.Value == 300); S.Reject(Buy.Seq.Value);
 // Manufacture: the received recipe book, a fee per recipe.
 { FWorldEvent E; FGrpcNightfallV1RecipeBookSync B; B.Baseline = true; B.Incarnation = 1; B.Tick = 11; B.Dwarven.Add(FUInt32(322)); B.DwarvenLimit = 50; B.CommonLimit = 50; E.RecipeBookSync = B; S.Dispatch(E); }
 D->KindControl()->SetSelectedOption(TEXT("Manufacture")); TestEqual(TEXT("registered recipes offered"), D->EntryControl()->GetOptionCount(), 1);
 Type(D->PriceControl(), TEXT("1500")); D->AddControl()->OnClicked.Broadcast(); TestFalse(TEXT("one entry per recipe"), D->AddControl()->GetIsEnabled());
 D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("manufacture store body"), S.Last().Intent.IntentCase == I::SetPrivateManufacture && S.Last().Intent.SetPrivateManufacture.Entries.Num() == 1 && S.Last().Intent.SetPrivateManufacture.Entries[0].RecipeId.Value == 322 && S.Last().Intent.SetPrivateManufacture.Entries[0].Cost.Value == 1500); S.Reject(S.Last().Seq.Value);
 // The open store is the received one; close sends ClosePrivateStore.
 D->KindControl()->SetSelectedOption(TEXT("Close store")); TestFalse(TEXT("nothing open: nothing to close"), D->ConfirmControl()->GetIsEnabled());
 Store(S, Owner, K::STORE_KIND_MANUFACTURE, 30, [](FGrpcNightfallV1PrivateStoreChanged& C) { FGrpcNightfallV1ManufactureEntry M; M.RecipeId = 322; M.Cost = 1500; C.Manufacture.Add(M); });
 TestTrue(TEXT("received open store"), D->OpenStoreText().Contains(TEXT("Manufacture store")) && D->OpenStoreText().Contains(TEXT("1 entries")));
 D->ConfirmControl()->OnClicked.Broadcast(); TestTrue(TEXT("close body"), S.Last().Intent.IntentCase == I::ClosePrivateStore); S.Reject(S.Last().Seq.Value);
 Store(S, Owner, K::STORE_KIND_NONE, 31); TestTrue(TEXT("received close"), D->OpenStoreText().Contains(TEXT("closed")));
 S.Net->Disconnect(); return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyStoreVisitUiTest, "Nightfall.Economy.UI.PrivateStoreVisitBuySellPackageOrder", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyStoreVisitUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; using K = EGrpcNightfallV1StoreKind; using I = EGrpcNightfallV1ClientMessageIntent; FScope S;
 auto* D = NewObject<UNightfallStoreVisitDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items, Seller);
 TestTrue(TEXT("no store yet"), D->StoreText().Contains(TEXT("Waiting"))); TestFalse(TEXT("nothing to confirm"), D->ConfirmControl()->GetIsEnabled());
 // A sell store: the received entry and price; the buy names the received instance.
 Store(S, Seller, K::STORE_KIND_SELL, 40, [](FGrpcNightfallV1PrivateStoreChanged& C) { FGrpcNightfallV1StoreSellEntry E; E.InstanceId = 501; E.TemplateId = 1864; E.Count = 10; E.Price = 25; C.Sell.Add(E); });
 TestTrue(TEXT("received sell entry"), D->EntryControl()->GetSelectedOption().Contains(TEXT("x10 for 25 adena each")));
 Type(D->CountControl(), TEXT("3")); D->ConfirmControl()->OnClicked.Broadcast(); if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage Buy = S.Last();
 TestTrue(TEXT("store buy body"), Buy.Intent.IntentCase == I::PrivateStoreBuy && Buy.Intent.PrivateStoreBuy.StoreOwner == FString(Seller).ToLower() && Buy.Intent.PrivateStoreBuy.Entries.Num() == 1 && Buy.Intent.PrivateStoreBuy.Entries[0].InstanceId.Value == 501 && Buy.Intent.PrivateStoreBuy.Entries[0].Count.Value == 3 && Buy.Intent.PrivateStoreBuy.Entries[0].Price.Value == 25);
 S.Reject(Buy.Seq.Value); D->RetryControl()->OnClicked.Broadcast(); TestEqual(TEXT("retry keeps the key and body"), S.Last().Intent.PrivateStoreBuy.ToJsonString(false), Buy.Intent.PrivateStoreBuy.ToJsonString(false)); S.Reject(S.Last().Seq.Value);
 // A package store: one choice, the whole package at the received counts.
 Store(S, Seller, K::STORE_KIND_PACKAGE_SELL, 41, [](FGrpcNightfallV1PrivateStoreChanged& C) { for (uint64 Id : {601ull, 602ull}) { FGrpcNightfallV1StoreSellEntry E; E.InstanceId = Id; E.TemplateId = 1864; E.Count = Id - 600; E.Price = 7; C.Sell.Add(E); } });
 TestEqual(TEXT("one package choice"), D->EntryControl()->GetOptionCount(), 1); TestFalse(TEXT("no count for a package"), D->CountControl()->GetIsEnabled());
 D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("whole package"), S.Last().Intent.IntentCase == I::PrivateStoreBuy && S.Last().Intent.PrivateStoreBuy.Entries.Num() == 2 && S.Last().Intent.PrivateStoreBuy.Entries[1].Count.Value == 2); S.Reject(S.Last().Seq.Value);
 // A buy store: only owned instances of the entry's template; the received price.
 Store(S, Seller, K::STORE_KIND_BUY, 42, [](FGrpcNightfallV1PrivateStoreChanged& C) { FGrpcNightfallV1StoreBuyEntry E; E.TemplateId = 1864; E.Count = 5; E.Price = 40; C.Buy.Add(E); });
 TestEqual(TEXT("one matching owned item"), D->ItemControl()->GetOptionCount(), 1);
 Type(D->CountControl(), TEXT("2")); D->ConfirmControl()->OnClicked.Broadcast();
 const FGrpcNightfallV1ClientMessage Sell = S.Last();
 TestTrue(TEXT("store sell body"), Sell.Intent.IntentCase == I::PrivateStoreSell && Sell.Intent.PrivateStoreSell.Entries.Num() == 1 && Sell.Intent.PrivateStoreSell.Entries[0].InstanceId.Value == 1 && Sell.Intent.PrivateStoreSell.Entries[0].TemplateId.Value == 1864 && Sell.Intent.PrivateStoreSell.Entries[0].Count.Value == 2 && Sell.Intent.PrivateStoreSell.Entries[0].Price.Value == 40); S.Reject(Sell.Seq.Value);
 // A manufacture store: the order names the owner and the recipe.
 Store(S, Seller, K::STORE_KIND_MANUFACTURE, 43, [](FGrpcNightfallV1PrivateStoreChanged& C) { FGrpcNightfallV1ManufactureEntry M; M.RecipeId = 322; M.Cost = 1500; C.Manufacture.Add(M); });
 D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("manufacture order body"), S.Last().Intent.IntentCase == I::PrivateManufactureOrder && S.Last().Intent.PrivateManufactureOrder.StoreOwner == FString(Seller).ToLower() && S.Last().Intent.PrivateManufactureOrder.RecipeId.Value == 322); S.Reject(S.Last().Seq.Value);
 // A closed store: nothing to confirm.
 Store(S, Seller, K::STORE_KIND_NONE, 44); TestFalse(TEXT("closed store"), D->ConfirmControl()->GetIsEnabled()); TestTrue(TEXT("waiting again"), D->StoreText().Contains(TEXT("Waiting")));
 S.Net->Disconnect(); return true;
}
// Story 5.5: the warehouse keeper, mail, auction and manor dialogs over the read views.
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyKeeperUiTest, "Nightfall.Economy.UI.KeeperDepositWithdrawFreightBusyRetry", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyKeeperUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; using I = EGrpcNightfallV1ClientMessageIntent; FScope S; int32 Warehouse = 0, Freight = 0;
 auto Row = [](uint64 Id, uint32 Template, uint64 Count, EGrpcNightfallV1ItemLocation L) { FGrpcNightfallV1OwnedItem R; R.InstanceId = Id; R.TemplateId = Template; R.Count = Count; R.Location = L; return R; };
 S.Economy->SetWarehouseProviderForTesting([&](const FGrpcNightfallV1GetWarehouseRequest&, USessionClient::FGetWarehouseCallback Cb) { ++Warehouse; FGrpcNightfallV1GetWarehouseResponse R; R.Capacity = 100; R.Items.Add(Row(80, 1864, 5, EGrpcNightfallV1ItemLocation::ITEM_LOCATION_WAREHOUSE)); Cb({}, R); });
 S.Economy->SetFreightProviderForTesting([&](const FGrpcNightfallV1GetFreightRequest&, USessionClient::FGetFreightCallback Cb) { ++Freight; FGrpcNightfallV1GetFreightResponse R; R.Capacity = 200; R.Items.Add(Row(81, 20678, 1, EGrpcNightfallV1ItemLocation::ITEM_LOCATION_FREIGHT)); Cb({}, R); });
 auto* D = NewObject<UNightfallKeeperDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items, Merchant);
 TestTrue(TEXT("both views asked"), Warehouse == 1 && Freight == 1);
 TestTrue(TEXT("received capacities"), D->StatusText().Contains(TEXT("capacity 100")) && D->StatusText().Contains(TEXT("capacity 200")));
 D->EntryControl()->SetSelectedIndex(0); Type(D->CountControl(), TEXT("4")); D->ConfirmControl()->OnClicked.Broadcast();
 if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage Deposit = S.Last();
 TestTrue(TEXT("deposit body"), Deposit.Intent.IntentCase == I::WarehouseDeposit && Deposit.Intent.WarehouseDeposit.Keeper == FString(Merchant).ToLower() && Deposit.Intent.WarehouseDeposit.Entries.Num() == 1 && Deposit.Intent.WarehouseDeposit.Entries[0].InstanceId.Value == 1 && Deposit.Intent.WarehouseDeposit.Entries[0].Count.Value == 4);
 // CONTAINER_BUSY: the notice names the retry; Retry sends the same key and body.
 S.Reject(Deposit.Seq.Value);
 TestTrue(TEXT("busy notice"), D->StatusText().Contains(TEXT("CONTAINER_BUSY")) && D->StatusText().Contains(TEXT("same key")));
 D->RetryControl()->OnClicked.Broadcast(); const FGrpcNightfallV1ClientMessage Again = S.Last();
 TestEqual(TEXT("busy retry keeps the frozen body"), Again.Intent.WarehouseDeposit.ToJsonString(false), Deposit.Intent.WarehouseDeposit.ToJsonString(false));
 TestFalse(TEXT("no notice while the retry is outstanding"), D->StatusText().Contains(TEXT("CONTAINER_BUSY")));
 { FServerMessage M; M.Rejected = FIntentRejected{ Again.Seq.Value, uint32(EGrpcNightfallV1RejectReason::REJECT_REASON_INVALID), TEXT("invalid") }; S.Net->DispatchServerMessage(M); }
 TestFalse(TEXT("another rejection clears the notice"), D->StatusText().Contains(TEXT("CONTAINER_BUSY")));
 // Withdraw names a received warehouse row; freight send names the target character.
 D->TabControl()->SetSelectedOption(TEXT("Withdraw")); TestEqual(TEXT("warehouse rows offered"), D->EntryControl()->GetOptionCount(), 1);
 Type(D->CountControl(), TEXT("5")); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("withdraw body"), S.Last().Intent.IntentCase == I::WarehouseWithdraw && S.Last().Intent.WarehouseWithdraw.Entries[0].InstanceId.Value == 80 && S.Last().Intent.WarehouseWithdraw.Entries[0].Count.Value == 5); S.Reject(S.Last().Seq.Value);
 D->TabControl()->SetSelectedOption(TEXT("Freight send")); Type(D->CountControl(), TEXT("1")); TestFalse(TEXT("freight send needs a target"), D->ConfirmControl()->GetIsEnabled());
 Type(D->TargetControl(), Seller); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("freight send body"), S.Last().Intent.IntentCase == I::FreightSend && S.Last().Intent.FreightSend.TargetCharacter == FString(Seller).ToLower()); S.Reject(S.Last().Seq.Value);
 D->TabControl()->SetSelectedOption(TEXT("Freight withdraw")); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("freight withdraw body"), S.Last().Intent.IntentCase == I::FreightWithdraw && S.Last().Intent.FreightWithdraw.Entries[0].InstanceId.Value == 81);
 S.Net->Disconnect(); return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyMailUiTest, "Nightfall.Economy.UI.MailReadTakeRejectCancelDeleteSend", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyMailUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; using I = EGrpcNightfallV1ClientMessageIntent; FScope S; TArray<FString> Read;
 const FString In = TEXT("aaaaaaaa-0000-4000-8000-000000000001"), Out = TEXT("aaaaaaaa-0000-4000-8000-000000000002");
 S.Economy->SetMailProviderForTesting([&](const FGrpcNightfallV1ListMailRequest& Q, USessionClient::FListMailCallback Cb) {
  FGrpcNightfallV1ListMailResponse R; R.Count = 1; R.Limit = 240; FGrpcNightfallV1MailHeader H; H.Subject = Q.MailBox == EGrpcNightfallV1MailBox::MAIL_BOX_INBOX ? TEXT("Pay me") : TEXT("Gift");
  H.MessageId = Q.MailBox == EGrpcNightfallV1MailBox::MAIL_BOX_INBOX ? In : Out; H.SenderName = TEXT("Bob"); H.ReceiverName = TEXT("Ann"); H.CodAdena = Q.MailBox == EGrpcNightfallV1MailBox::MAIL_BOX_INBOX ? 100 : 0; H.HasAttachments = true; R.Messages.Add(H); Cb({}, R); });
 S.Economy->SetReadMailProviderForTesting([&](const FGrpcNightfallV1ReadMailRequest& Q, USessionClient::FReadMailCallback Cb) {
  Read.Add(Q.MessageId); FGrpcNightfallV1ReadMailResponse R; R.HasHeader = true; R.Header.Subject = TEXT("Pay me"); R.Body = TEXT("hello there"); FGrpcNightfallV1OwnedItem A; A.InstanceId = 90; A.TemplateId = 1666; A.Count = 1; R.Attachments.Add(A); Cb({}, R); });
 auto* D = NewObject<UNightfallMailDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items);
 TestTrue(TEXT("received inbox header with COD"), D->MessageControl()->GetSelectedOption().Contains(TEXT("[COD 100 adena]")) && D->MessageControl()->GetSelectedOption().Contains(TEXT("[attachments]")));
 // Read is the ReadMail RPC: no intent.
 D->ConfirmControl()->OnClicked.Broadcast(); TestEqual(TEXT("read sends no intent"), S.Socket->Sent.Num(), 0);
 TestTrue(TEXT("ReadMail for the message"), Read.Num() == 1 && Read[0] == In); TestTrue(TEXT("received body and attachment"), D->ReadText().Contains(TEXT("hello there")) && D->ReadText().Contains(TEXT("x1")));
 D->ActionControl()->SetSelectedOption(TEXT("Take attachments")); D->ConfirmControl()->OnClicked.Broadcast();
 if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage Take = S.Last();
 TestTrue(TEXT("take body"), Take.Intent.IntentCase == I::MailTakeAttachments && Take.Intent.MailTakeAttachments.MessageId == In);
 S.Reject(Take.Seq.Value); D->RetryControl()->OnClicked.Broadcast(); TestEqual(TEXT("take retry keeps the key"), S.Last().Intent.MailTakeAttachments.IdempotencyKey, Take.Intent.MailTakeAttachments.IdempotencyKey); S.Reject(S.Last().Seq.Value);
 for (const TCHAR* A : {TEXT("Reject"), TEXT("Mark read"), TEXT("Delete")}) { D->ActionControl()->SetSelectedOption(A); D->ConfirmControl()->OnClicked.Broadcast(); S.Reject(S.Last().Seq.Value); }
 TestTrue(TEXT("delete from the inbox"), S.Last().Intent.IntentCase == I::MailDelete && S.Last().Intent.MailDelete.MessageIds.Num() == 1 && !S.Last().Intent.MailDelete.SentBox);
 D->BoxControl()->SetSelectedOption(TEXT("Sent")); D->ActionControl()->SetSelectedOption(TEXT("Cancel")); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("cancel a sent mail"), S.Last().Intent.IntentCase == I::MailCancel && S.Last().Intent.MailCancel.MessageId == Out); S.Reject(S.Last().Seq.Value);
 // Write: receiver, subject, text, COD and one attachment.
 D->BoxControl()->SetSelectedOption(TEXT("Write")); TestFalse(TEXT("no receiver yet"), D->ConfirmControl()->GetIsEnabled());
 Type(D->ReceiverControl(), TEXT("Bob")); Type(D->SubjectControl(), TEXT("Cod")); Type(D->BodyControl(), TEXT("pay")); Type(D->CodControl(), TEXT("500"));
 D->AttachmentControl()->SetSelectedIndex(0); Type(D->CountControl(), TEXT("2")); D->AttachControl()->OnClicked.Broadcast();
 D->ConfirmControl()->OnClicked.Broadcast(); const FGrpcNightfallV1ClientMessage Send = S.Last();
 TestTrue(TEXT("mail send body"), Send.Intent.IntentCase == I::MailSend && Send.Intent.MailSend.ReceiverName == TEXT("Bob") && Send.Intent.MailSend.CodAdena.Value == 500 && Send.Intent.MailSend.Attachments.Num() == 1 && Send.Intent.MailSend.Attachments[0].Count.Value == 2);
 S.Net->Disconnect(); return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyAuctionUiTest, "Nightfall.Economy.UI.AuctionBidAndCancel", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyAuctionUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; using I = EGrpcNightfallV1ClientMessageIntent; FScope S; TArray<uint32> Asked;
 S.Economy->SetAuctionsProviderForTesting([&](const FGrpcNightfallV1ListItemAuctionsRequest& Q, USessionClient::FListItemAuctionsCallback Cb) {
  Asked.Add(Q.InstanceNpcId.Value); FGrpcNightfallV1ListItemAuctionsResponse R; R.HasCurrent = true; R.Current.AuctionId = 1; R.Current.State = EGrpcNightfallV1AuctionState::AUCTION_STATE_STARTED; R.Current.TemplateId = 1864; R.Current.Count = 1; R.Current.InitBid = 100000; R.Current.HighestBid = 150000; R.Current.OwnBid = 120000;
  FGrpcNightfallV1AuctionView Own = R.Current; R.OwnBids.Add(Own); Cb({}, R); });
 auto* D = NewObject<UNightfallAuctionDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items, Merchant, 32320);
 TestTrue(TEXT("ListItemAuctions for the instance"), Asked.Num() == 1 && Asked[0] == 32320u);
 TestTrue(TEXT("received bids"), D->AuctionText().Contains(TEXT("highest 150000")) && D->AuctionText().Contains(TEXT("your bid 120000")) && D->AuctionText().Contains(TEXT("started")));
 Type(D->BidControl(), TEXT("160000")); D->ConfirmControl()->OnClicked.Broadcast();
 if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage Bid = S.Last();
 TestTrue(TEXT("bid body"), Bid.Intent.IntentCase == I::ItemAuctionBid && Bid.Intent.ItemAuctionBid.InstanceNpcId.Value == 32320 && Bid.Intent.ItemAuctionBid.Bid.Value == 160000);
 S.Reject(Bid.Seq.Value); D->RetryControl()->OnClicked.Broadcast(); TestEqual(TEXT("bid retry keeps the frozen body"), S.Last().Intent.ItemAuctionBid.ToJsonString(false), Bid.Intent.ItemAuctionBid.ToJsonString(false)); S.Reject(S.Last().Seq.Value);
 D->ActionControl()->SetSelectedOption(TEXT("Cancel bid")); D->ConfirmControl()->OnClicked.Broadcast();
 TestTrue(TEXT("cancel bid body"), S.Last().Intent.IntentCase == I::ItemAuctionCancelBid && S.Last().Intent.ItemAuctionCancelBid.Auctioneer == FString(Merchant).ToLower() && S.Last().Intent.ItemAuctionCancelBid.InstanceNpcId.Value == 32320);
 S.Net->Disconnect(); return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyManorUiTest, "Nightfall.Economy.UI.ManorBuySeedSellCrop", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyManorUiTest::RunTest(const FString&)
{
 using namespace EconomyUiTest; using I = EGrpcNightfallV1ClientMessageIntent; FScope S; TArray<uint32> Asked;
 S.Economy->SetManorProviderForTesting([&](const FGrpcNightfallV1GetManorDataRequest& Q, USessionClient::FGetManorDataCallback Cb) {
  Asked.Add(Q.CastleId.Value); FGrpcNightfallV1GetManorDataResponse R; R.CastleId = Q.CastleId;
  FGrpcNightfallV1SeedProductionView Seed; Seed.SeedId = 5016; Seed.Price = 100; Seed.StartAmount = 100; Seed.Amount = 50; R.Seeds.Add(Seed);
  FGrpcNightfallV1CropProcureView Crop; Crop.CropId = 1864; Crop.Price = 300; Crop.StartAmount = 20; Crop.Amount = 20; Crop.RewardType = 1; R.Crops.Add(Crop); Cb({}, R); });
 auto* D = NewObject<UNightfallManorDialog>(S.Scope.GameInstance); D->BindState(S.Economy, S.Items, Merchant, 1);
 TestTrue(TEXT("GetManorData for castle 1"), Asked.Num() == 1 && Asked[0] == 1u);
 TestTrue(TEXT("received seed price and amount"), D->EntryControl()->GetSelectedOption().Contains(TEXT("100 adena each, 50/100 left")));
 Type(D->CountControl(), TEXT("3")); D->ConfirmControl()->OnClicked.Broadcast();
 if (!TestEqual(TEXT("one intent"), S.Socket->Sent.Num(), 1)) return false;
 const FGrpcNightfallV1ClientMessage Buy = S.Last();
 TestTrue(TEXT("buy seed body"), Buy.Intent.IntentCase == I::ManorBuySeed && Buy.Intent.ManorBuySeed.Manager == FString(Merchant).ToLower() && Buy.Intent.ManorBuySeed.ManorId.Value == 1 && Buy.Intent.ManorBuySeed.Seeds.Num() == 1 && Buy.Intent.ManorBuySeed.Seeds[0].TemplateId.Value == 5016 && Buy.Intent.ManorBuySeed.Seeds[0].Count.Value == 3);
 S.Reject(Buy.Seq.Value);
 D->ActionControl()->SetSelectedOption(TEXT("Sell crop")); TestEqual(TEXT("only owned instances of the crop"), D->ItemControl()->GetOptionCount(), 1);
 Type(D->CountControl(), TEXT("2")); D->ConfirmControl()->OnClicked.Broadcast();
 const FGrpcNightfallV1ClientMessage Sell = S.Last();
 TestTrue(TEXT("sell crop body"), Sell.Intent.IntentCase == I::ManorSellCrop && Sell.Intent.ManorSellCrop.Entries.Num() == 1 && Sell.Intent.ManorSellCrop.Entries[0].InstanceId.Value == 1 && Sell.Intent.ManorSellCrop.Entries[0].CropId.Value == 1864 && Sell.Intent.ManorSellCrop.Entries[0].ManorId.Value == 1 && Sell.Intent.ManorSellCrop.Entries[0].Count.Value == 2);
 S.Net->Disconnect(); return true;
}
#endif
