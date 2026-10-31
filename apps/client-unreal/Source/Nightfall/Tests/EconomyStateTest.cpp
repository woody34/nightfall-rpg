#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Economy/EconomyStateSubsystem.h"
#include "Items/InventoryStateSubsystem.h"
#include "Bot/EconomyBotPredicates.h"
#include "Bot/BotPredicates.h"
#include "Bot/BotSteps.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallWire.h"
#include "IWebSocket.h"
#include "HAL/FileManager.h"
#include "HAL/IConsoleManager.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#if WITH_DEV_AUTOMATION_TESTS
namespace EconomyTest
{
 const TCHAR* Owner = TEXT("01970000-0000-7000-8000-000000000050");
 const TCHAR* Partner = TEXT("01970000-0000-7000-8000-000000000051");
 const TCHAR* Merchant = TEXT("01970000-0000-7000-8000-00000000a001");
 const TCHAR* Monster = TEXT("01970000-0000-7000-8000-00000000b001");
 const TCHAR* GroundId = TEXT("01970000-0000-7000-8000-00000000c001");
 const TCHAR* Key = TEXT("12345678-1111-4111-8111-123456789abc");
 class FRecordingSocket final : public IWebSocket
 {
 public:
  TArray<TArray<uint8>> Sent;
  virtual void Connect() override {}
  virtual void Close(int32, const FString&) override {}
  virtual bool IsConnected() override { return true; }
  virtual void Send(const FString&) override {}
  virtual void Send(const void* Data, SIZE_T Size, bool) override { Sent.Emplace_GetRef().Append(static_cast<const uint8*>(Data), static_cast<int32>(Size)); }
  virtual void SetTextMessageMemoryLimit(uint64) override {}
  virtual FWebSocketConnectedEvent& OnConnected() override { return Connected; }
  virtual FWebSocketConnectionErrorEvent& OnConnectionError() override { return ConnectionError; }
  virtual FWebSocketClosedEvent& OnClosed() override { return Closed; }
  virtual FWebSocketMessageEvent& OnMessage() override { return Message; }
  virtual FWebSocketBinaryMessageEvent& OnBinaryMessage() override { return BinaryMessage; }
  virtual FWebSocketRawMessageEvent& OnRawMessage() override { return RawMessage; }
  virtual FWebSocketMessageSentEvent& OnMessageSent() override { return MessageSent; }
  FWebSocketConnectedEvent Connected; FWebSocketConnectionErrorEvent ConnectionError; FWebSocketClosedEvent Closed; FWebSocketMessageEvent Message; FWebSocketBinaryMessageEvent BinaryMessage; FWebSocketRawMessageEvent RawMessage; FWebSocketMessageSentEvent MessageSent;
 };
 FEntitySpawn Spawn(const FString& Id, uint32 Kind, uint64 Tick, uint32 Inc = 1, const FString& Name = TEXT(""), const FString& Template = TEXT(""))
 {
  FEntitySpawn S; S.EntityId = Id; S.Kind = Kind; S.StateTick = Tick; S.LifeIncarnation = Inc; S.Name = Name; S.TemplateId = Template; S.bCombatant = Kind == 1 || !Template.IsEmpty(); S.bAttackable = !Template.IsEmpty(); S.Position = FNetVec2{ 100.f, 100.f }; return S;
 }
 FEntitySpawn GroundSpawn(uint32 Template, uint64 Count, uint64 Tick, const FString& LootOwner = TEXT(""))
 {
  FEntitySpawn S = Spawn(GroundId, 3, Tick, 0); S.bCombatant = false; FGrpcNightfallV1GroundItemDetail D; D.ItemInstanceId = 4000000001ull; D.TemplateId = Template; D.Count = Count; D.LootOwner = LootOwner; D.LootUntilTick = Tick + 150; D.DropProtectUntilTick = Tick + 150; D.DroppedTick = Tick; S.GroundItem = D; return S;
 }
 FGrpcNightfallV1InventorySync Sync(uint64 Tick = 10, uint32 Inc = 1)
 {
  FGrpcNightfallV1InventorySync S; S.EntityId = NightfallItems::UuidBytes(Owner); S.Incarnation = Inc; S.Tick = S.BaselineTick = Tick; S.Baseline = true; S.InventoryRevision = 1; S.HasWeight = S.HasPenalties = true; S.RulesHash.Value.Init(1, 32);
  FGrpcNightfallV1OwnedItem Adena; Adena.InstanceId = 2; Adena.TemplateId = 57; Adena.Count = 500; Adena.Location = EGrpcNightfallV1ItemLocation::ITEM_LOCATION_INVENTORY; S.Items.Add(Adena);
  FGrpcNightfallV1OwnedItem Mats; Mats.InstanceId = 1; Mats.TemplateId = 1864; Mats.Count = 10; Mats.Location = EGrpcNightfallV1ItemLocation::ITEM_LOCATION_INVENTORY; S.Items.Add(Mats); return S;
 }
 void Dispatch(UNetClientSubsystem* N, const FWorldEvent& E) { FServerMessage M; M.Event = E; N->DispatchServerMessage(M); }
 void DispatchSpawn(UNetClientSubsystem* N, const FEntitySpawn& S) { FWorldEvent E; E.Spawn = S; Dispatch(N, E); }
 FGrpcNightfallV1EconomyResult Receipt(EGrpcNightfallV1ItemOperation Op, uint64 Tick, const TCHAR* ReceiptId, const TCHAR* RequestKey = Key)
 {
  FGrpcNightfallV1EconomyResult R; R.EntityId = NightfallItems::UuidBytes(Owner); R.Incarnation = 1; R.Tick = Tick; R.InventoryRevision = 1; R.ReceiptId = ReceiptId; R.Operation = Op; R.RequestKey = NightfallItems::UuidBytes(RequestKey); R.RequestFingerprint = TEXT("fp"); return R;
 }
 /** Encodes a populated server event through the real marshaler and decodes it the way the socket path does. */
 FServerMessage Wire(const FGrpcNightfallV1WorldEvent& Event, FAutomationTestBase& T)
 {
  FGrpcNightfallV1ServerMessage M; M.Payload.PayloadCase = EGrpcNightfallV1ServerMessagePayload::Event; M.Payload.Event = MakeShared<FGrpcNightfallV1WorldEvent>(Event);
  TArray<uint8> B; NightfallWire::EncodeServerMessage(M, B); FServerMessage Out; T.TestTrue(TEXT("wire event decodes"), NightfallProto::Decode(B.GetData(), B.Num(), Out)); return Out;
 }
 struct FScope
 {
  FScopedTestGameInstance Scope; UNetClientSubsystem* Net; UEconomyStateSubsystem* Economy; UInventoryStateSubsystem* Items; FBotPredicateRegistry Registry; FBotContext Context; FAutomationTestBase& T;
  explicit FScope(FAutomationTestBase& InT) : T(InT)
  {
   Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
   Net = Scope.Get<UNetClientSubsystem>(); Economy = Scope.Get<UEconomyStateSubsystem>(); Items = Scope.Get<UInventoryStateSubsystem>();
   Net->SetOwnEntityId(Owner); Registry.RegisterBuiltins(); Context.GameInstance = Scope.GameInstance;
   TMap<FString, FEconomyRoleIdentity> Roles; Roles.Add(TEXT("a"), { Owner, TEXT("Skilla") }); Roles.Add(TEXT("b"), { Partner, TEXT("Skillb") }); Economy->SetRolesForTesting(Roles);
  }
  FBotPredicateValue Eval(const TCHAR* Text) { FString Error; auto P = Registry.Parse(Registry.Tokenize(Text), Error); T.TestTrue(FString::Printf(TEXT("binds: %s (%s)"), Text, *Error), bool(P)); return P ? P(Context) : FBotPredicateValue{}; }
  void Admit(uint64 Tick = 10, uint32 Inc = 1) { DispatchSpawn(Net, Spawn(Owner, 1, Tick, Inc)); FWorldEvent E; E.InventorySync = Sync(Tick, Inc); Dispatch(Net, E); }
 };
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyCodecTest, "Nightfall.Economy.Codec.EventsReceiptAndIntents", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyCodecTest::RunTest(const FString&)
{
 using namespace EconomyTest;
 // Every Phase 5 WorldEvent member (34..45) decodes into exactly its own optional.
 int32 Decoded = 0;
#define NIGHTFALL_EVENT_CASE(Member, Setup) { FGrpcNightfallV1WorldEvent Ev; Ev.Event.EventCase = EGrpcNightfallV1WorldEventEvent::Member; Setup; const FServerMessage M = Wire(Ev, *this); if (TestTrue(TEXT(#Member " decoded"), M.Event.IsSet() && M.Event->Member.IsSet())) ++Decoded; TestFalse(TEXT(#Member " alone"), M.Event.IsSet() && (M.Event->Spawn.IsSet() || M.Event->InventorySync.IsSet())); }
 NIGHTFALL_EVENT_CASE(EconomyResult, Ev.Event.EconomyResult = Receipt(EGrpcNightfallV1ItemOperation::ITEM_OPERATION_VENDOR_BUY, 11, TEXT("r1")))
 NIGHTFALL_EVENT_CASE(CorpseStateChanged, Ev.Event.CorpseStateChanged.Npc = Monster; Ev.Event.CorpseStateChanged.Spoiled = true)
 NIGHTFALL_EVENT_CASE(PartyStateChanged, Ev.Event.PartyStateChanged.PartyId = 7; Ev.Event.PartyStateChanged.Baseline = true)
 NIGHTFALL_EVENT_CASE(PartyLootVote, Ev.Event.PartyLootVote.PartyId = 7)
 NIGHTFALL_EVENT_CASE(PartyInviteReceived, Ev.Event.PartyInviteReceived.Requester = Partner)
 NIGHTFALL_EVENT_CASE(TradeRequestReceived, Ev.Event.TradeRequestReceived.Requester = Partner)
 NIGHTFALL_EVENT_CASE(TradeSessionChanged, Ev.Event.TradeSessionChanged.Partner = Partner)
 NIGHTFALL_EVENT_CASE(PrivateStoreChanged, Ev.Event.PrivateStoreChanged.Owner = Partner)
 NIGHTFALL_EVENT_CASE(MailNotification, Ev.Event.MailNotification.Subject = TEXT("paycheck"))
 NIGHTFALL_EVENT_CASE(RecipeBookSync, Ev.Event.RecipeBookSync.Baseline = true)
 NIGHTFALL_EVENT_CASE(ItemAuctionStatusChanged, Ev.Event.ItemAuctionStatusChanged.InstanceNpcId = 32320)
 NIGHTFALL_EVENT_CASE(LootDistributed, Ev.Event.LootDistributed.Recipient = Partner)
#undef NIGHTFALL_EVENT_CASE
 TestEqual(TEXT("twelve Phase 5 events decode"), Decoded, 12);
 // The receipt bytes are the exact received field 34 payload (D43), also under a packed-field re-encoding.
 { FGrpcNightfallV1WorldEvent Ev; Ev.Event.EventCase = EGrpcNightfallV1WorldEventEvent::EconomyResult; Ev.Event.EconomyResult = Receipt(EGrpcNightfallV1ItemOperation::ITEM_OPERATION_VENDOR_BUY, 11, TEXT("r1")); Ev.Event.EconomyResult.AffectedInstanceIds.Add(FUInt64(5));
  const FServerMessage M = Wire(Ev, *this); TArray<uint8> Expected; NightfallWire::EncodeEconomyReceipt(Ev.Event.EconomyResult, Expected);
  TestTrue(TEXT("receipt payload equals the marshaled EconomyResult"), M.Event.IsSet() && M.Event->EconomyReceiptPayload == Expected && !Expected.IsEmpty());
  TestTrue(TEXT("item receipt payload stays empty for an economy receipt"), M.Event.IsSet() && M.Event->ItemReceiptPayload.IsEmpty());
  auto Varint = [](TArray<uint8>& B, uint64 V) { while (V >= 128) { B.Add(uint8(V) | 128); V >>= 7; } B.Add(uint8(V)); };
  TArray<uint8> Event; Varint(Event, (34 << 3) | 2); Varint(Event, Expected.Num()); Event.Append(Expected); TArray<uint8> Frame; Frame.Add(0x12); Varint(Frame, Event.Num()); Frame.Append(Event);
  TArray<uint8> Extracted; NightfallWire::EconomyReceiptBytes(Frame.GetData(), Frame.Num(), Extracted); TestTrue(TEXT("hand-framed receipt extracted byte-exact"), Extracted == Expected);
  for (int32 Size = 0; Size < Frame.Num(); ++Size) { NightfallWire::EconomyReceiptBytes(Frame.GetData(), Size, Extracted); TestTrue(TEXT("truncated receipt frame rejected"), Extracted.IsEmpty()); }
  // An unknown oneof member (tag 99) decodes to an event with nothing set and no receipt bytes: fail-closed.
  TArray<uint8> Unknown; Varint(Unknown, (99 << 3) | 2); Varint(Unknown, 2); Unknown.Add(0x08); Unknown.Add(0x01); TArray<uint8> F2; F2.Add(0x12); Varint(F2, Unknown.Num()); F2.Append(Unknown);
  FServerMessage U; TestTrue(TEXT("unknown member still decodes the envelope"), NightfallProto::Decode(F2.GetData(), F2.Num(), U));
  TestTrue(TEXT("unknown member sets nothing"), U.Event.IsSet() && !U.Event->EconomyResult.IsSet() && !U.Event->LootDistributed.IsSet() && !U.Event->Spawn.IsSet() && U.Event->EconomyReceiptPayload.IsEmpty()); }
 // Ground item detail rides EntitySpawn (kind 3, field 30); a spawn without it decodes with the optional unset.
 { FGrpcNightfallV1WorldEvent Ev; Ev.Event.EventCase = EGrpcNightfallV1WorldEventEvent::Spawn; Ev.Event.Spawn = MakeShared<FGrpcNightfallV1EntitySpawn>(); Ev.Event.Spawn->EntityId = GroundId; Ev.Event.Spawn->Kind = EGrpcNightfallV1EntityKind::ENTITY_KIND_GROUND_ITEM; Ev.Event.Spawn->HasGroundItem = true; Ev.Event.Spawn->GroundItem.TemplateId = 57; Ev.Event.Spawn->GroundItem.Count = uint64(MAX_int64); Ev.Event.Spawn->GroundItem.LootOwner = Owner; Ev.Event.Spawn->GroundItem.Herb = true;
  const FServerMessage M = Wire(Ev, *this); TestTrue(TEXT("ground item decoded"), M.Event.IsSet() && M.Event->Spawn.IsSet() && M.Event->Spawn->Kind == 3 && M.Event->Spawn->GroundItem.IsSet() && M.Event->Spawn->GroundItem->TemplateId.Value == 57 && M.Event->Spawn->GroundItem->Count.Value == uint64(MAX_int64) && M.Event->Spawn->GroundItem->Herb);
  Ev.Event.Spawn->HasGroundItem = false; Ev.Event.Spawn->Kind = EGrpcNightfallV1EntityKind::ENTITY_KIND_NPC; const FServerMessage N = Wire(Ev, *this); TestTrue(TEXT("npc spawn has no ground item"), N.Event.IsSet() && N.Event->Spawn.IsSet() && !N.Event->Spawn->GroundItem.IsSet()); }
 // All 41 Phase 5 intents encode into their own oneof member with the key preserved.
 int32 Intents = 0;
#define NIGHTFALL_INTENT_CASE(Member, Type) { FClientMessage M; M.Seq = 7; Type Q; Q.IdempotencyKey = Key; M.Member = Q; TArray<uint8> B; NightfallProto::Encode(M, B); FGrpcNightfallV1ClientMessage D; if (TestTrue(TEXT(#Member " decodes"), NightfallWire::DecodeClientMessage(B.GetData(), B.Num(), D) && D.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::Member && D.Intent.Member.IdempotencyKey == Key)) ++Intents; }
 NIGHTFALL_INTENT_CASE(PickUpItem, FGrpcNightfallV1PickUpItemRequest) NIGHTFALL_INTENT_CASE(VendorBuy, FGrpcNightfallV1VendorBuyRequest) NIGHTFALL_INTENT_CASE(VendorSell, FGrpcNightfallV1VendorSellRequest) NIGHTFALL_INTENT_CASE(VendorRefund, FGrpcNightfallV1VendorRefundRequest)
 NIGHTFALL_INTENT_CASE(MultisellChoose, FGrpcNightfallV1MultisellChooseRequest) NIGHTFALL_INTENT_CASE(RegisterRecipe, FGrpcNightfallV1RegisterRecipeRequest) NIGHTFALL_INTENT_CASE(DeleteRecipe, FGrpcNightfallV1DeleteRecipeRequest) NIGHTFALL_INTENT_CASE(CraftItem, FGrpcNightfallV1CraftItemRequest)
 NIGHTFALL_INTENT_CASE(TradeRequest, FGrpcNightfallV1TradeRequestRequest) NIGHTFALL_INTENT_CASE(TradeAnswer, FGrpcNightfallV1TradeAnswerRequest) NIGHTFALL_INTENT_CASE(TradeAddItem, FGrpcNightfallV1TradeAddItemRequest) NIGHTFALL_INTENT_CASE(TradeConfirm, FGrpcNightfallV1TradeConfirmRequest) NIGHTFALL_INTENT_CASE(TradeCancel, FGrpcNightfallV1TradeCancelRequest)
 NIGHTFALL_INTENT_CASE(SetPrivateStoreSell, FGrpcNightfallV1SetPrivateStoreSellRequest) NIGHTFALL_INTENT_CASE(SetPrivateStoreBuy, FGrpcNightfallV1SetPrivateStoreBuyRequest) NIGHTFALL_INTENT_CASE(SetPrivateManufacture, FGrpcNightfallV1SetPrivateManufactureRequest) NIGHTFALL_INTENT_CASE(ClosePrivateStore, FGrpcNightfallV1ClosePrivateStoreRequest)
 NIGHTFALL_INTENT_CASE(PrivateStoreBuy, FGrpcNightfallV1PrivateStoreBuyRequest) NIGHTFALL_INTENT_CASE(PrivateStoreSell, FGrpcNightfallV1PrivateStoreSellRequest) NIGHTFALL_INTENT_CASE(PrivateManufactureOrder, FGrpcNightfallV1PrivateManufactureOrderRequest)
 NIGHTFALL_INTENT_CASE(PartyInvite, FGrpcNightfallV1PartyInviteRequest) NIGHTFALL_INTENT_CASE(PartyAnswer, FGrpcNightfallV1PartyAnswerRequest) NIGHTFALL_INTENT_CASE(PartyLeave, FGrpcNightfallV1PartyLeaveRequest) NIGHTFALL_INTENT_CASE(PartyKick, FGrpcNightfallV1PartyKickRequest) NIGHTFALL_INTENT_CASE(PartyChangeLeader, FGrpcNightfallV1PartyChangeLeaderRequest)
 NIGHTFALL_INTENT_CASE(PartyLootChangeRequest, FGrpcNightfallV1PartyLootChangeRequest) NIGHTFALL_INTENT_CASE(PartyLootChangeAnswer, FGrpcNightfallV1PartyLootChangeAnswer) NIGHTFALL_INTENT_CASE(WarehouseDeposit, FGrpcNightfallV1WarehouseDepositRequest) NIGHTFALL_INTENT_CASE(WarehouseWithdraw, FGrpcNightfallV1WarehouseWithdrawRequest)
 NIGHTFALL_INTENT_CASE(FreightSend, FGrpcNightfallV1FreightSendRequest) NIGHTFALL_INTENT_CASE(FreightWithdraw, FGrpcNightfallV1FreightWithdrawRequest) NIGHTFALL_INTENT_CASE(MailSend, FGrpcNightfallV1MailSendRequest) NIGHTFALL_INTENT_CASE(MailTakeAttachments, FGrpcNightfallV1MailTakeAttachmentsRequest)
 NIGHTFALL_INTENT_CASE(MailReject, FGrpcNightfallV1MailRejectRequest) NIGHTFALL_INTENT_CASE(MailCancel, FGrpcNightfallV1MailCancelRequest) NIGHTFALL_INTENT_CASE(MailDelete, FGrpcNightfallV1MailDeleteRequest) NIGHTFALL_INTENT_CASE(ManorBuySeed, FGrpcNightfallV1ManorBuySeedRequest)
 NIGHTFALL_INTENT_CASE(ManorSellCrop, FGrpcNightfallV1ManorSellCropRequest) NIGHTFALL_INTENT_CASE(ItemAuctionBid, FGrpcNightfallV1ItemAuctionBidRequest) NIGHTFALL_INTENT_CASE(ItemAuctionCancelBid, FGrpcNightfallV1ItemAuctionCancelBidRequest) NIGHTFALL_INTENT_CASE(MailMarkRead, FGrpcNightfallV1MailMarkReadRequest)
#undef NIGHTFALL_INTENT_CASE
 TestEqual(TEXT("forty-one Phase 5 intents encode"), Intents, 41);
 return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyReducerTest, "Nightfall.Economy.Reducer.GroundCorpsePartyAndResets", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyReducerTest::RunTest(const FString&)
{
 using namespace EconomyTest; FScope S(*this); auto* N = S.Net;
 for (const TCHAR* Text : { TEXT("ground_item_exists 57"), TEXT("ground_item_any"), TEXT("corpse_spoiled template:npc_20003"), TEXT("party_member_count == 2"), TEXT("loot_mode == BY_TURN"), TEXT("loot_distributed"), TEXT("economy_result == vendor_buy"), TEXT("xp_gained_events >= 1"), TEXT("trade_state == STARTED"), TEXT("store_state_of b == SELL"), TEXT("auction_state == STARTED"), TEXT("recipe_registered 322"), TEXT("own_out_of_party_range") }) TestFalse(FString(TEXT("absence is UNKNOWN: ")) + Text, S.Eval(Text).bTrue);
 S.Admit();
 // Ground items (5.1): kind 3 spawns are a received fact set; the despawn moves them to the gone set.
 DispatchSpawn(N, GroundSpawn(57, 120, 11, Owner));
 TestTrue(TEXT("ground item received"), S.Eval(TEXT("ground_item_exists 57")).bTrue); TestTrue(TEXT("kind 3"), S.Eval(TEXT("ground_item_kind 57 == 3")).bTrue); TestTrue(TEXT("loot owner self"), S.Eval(TEXT("ground_loot_owner 57 == self")).bTrue);
 TestTrue(TEXT("no drop protection owner"), S.Eval(TEXT("ground_drop_protection 57 != self")).bTrue); TestTrue(TEXT("stack count"), S.Eval(TEXT("ground_item_count_between 57 100 200")).bTrue); TestTrue(TEXT("one ground entity"), S.Eval(TEXT("ground_item_count == 1")).bTrue);
 TestFalse(TEXT("protection not expired at tick 11"), S.Eval(TEXT("ground_protection_expired 57")).bTrue); TestFalse(TEXT("no PK drop yet"), S.Eval(TEXT("ground_drop_protection_none")).bTrue);
 { FWorldEvent E; E.Despawn = FEntityDespawn{ GroundId }; Dispatch(N, E); }
 TestTrue(TEXT("despawn received"), S.Eval(TEXT("ground_item_gone 57")).bTrue); TestFalse(TEXT("gone entity no longer exists"), S.Eval(TEXT("ground_item_exists 57")).bTrue); TestTrue(TEXT("count back to zero"), S.Eval(TEXT("ground_item_count == 0")).bTrue);
 // Corpses are addressed by the overlay template of the NPC spawn, also after its despawn.
 DispatchSpawn(N, Spawn(Monster, 2, 11, 1, TEXT("Npc_20003"), TEXT("npc_20003")));
 { FWorldEvent E; FGrpcNightfallV1CorpseStateChanged C; C.Npc = Monster; C.Tick = 12; C.Spoiled = true; E.CorpseStateChanged = C; Dispatch(N, E); }
 TestTrue(TEXT("spoiled corpse by template"), S.Eval(TEXT("corpse_spoiled template:npc_20003")).bTrue); TestFalse(TEXT("not yet swept"), S.Eval(TEXT("corpse_swept template:npc_20003")).bTrue);
 { FWorldEvent E; FGrpcNightfallV1CorpseStateChanged C; C.Npc = Monster; C.Tick = 11; C.Swept = true; E.CorpseStateChanged = C; Dispatch(N, E); }
 TestFalse(TEXT("stale corpse tick rejected"), S.Eval(TEXT("corpse_swept template:npc_20003")).bTrue);
 { FWorldEvent E; FGrpcNightfallV1CorpseStateChanged C; C.Npc = Monster; C.Tick = 13; C.Spoiled = C.Sweepable = C.Swept = true; E.CorpseStateChanged = C; Dispatch(N, E); }
 TestTrue(TEXT("swept"), S.Eval(TEXT("corpse_swept template:npc_20003")).bTrue); TestTrue(TEXT("party sweep done reads the same corpse"), S.Eval(TEXT("party_sweep_done template:npc_20003")).bTrue);
 { FWorldEvent E; E.Despawn = FEntityDespawn{ Monster }; Dispatch(N, E); }
 TestTrue(TEXT("entity gone by template"), S.Eval(TEXT("entity_gone template:npc_20003")).bTrue); TestTrue(TEXT("corpse state survives the despawn"), S.Eval(TEXT("corpse_swept template:npc_20003")).bTrue);
 // Party (5.2): a delta before the admission baseline is refused; the baseline admits; a stale tick is refused; disband clears.
 auto Party = [&](uint64 Tick, bool bBaseline, int32 Members, uint64 Id = 7, bool bDisbanded = false) { FGrpcNightfallV1PartyStateChanged P; P.PartyId = Id; P.Tick = Tick; P.Baseline = bBaseline; P.Disbanded = bDisbanded; P.Mode = EGrpcNightfallV1LootMode::LOOT_MODE_BY_TURN; for (int32 I = 0; I < Members; ++I) { FGrpcNightfallV1PartyMember M; M.Entity = I == 0 ? Owner : Partner; M.Level = 20; P.Members.Add(M); } FWorldEvent E; E.PartyStateChanged = P; Dispatch(N, E); };
 Party(12, false, 2); TestFalse(TEXT("delta without baseline refused"), S.Eval(TEXT("party_member_count == 2")).bTrue);
 Party(12, true, 2); TestTrue(TEXT("baseline admitted"), S.Eval(TEXT("party_member_count == 2")).bTrue); TestTrue(TEXT("loot mode"), S.Eval(TEXT("loot_mode == BY_TURN")).bTrue);
 Party(11, false, 1); TestTrue(TEXT("stale delta refused"), S.Eval(TEXT("party_member_count == 2")).bTrue);
 Party(13, false, 1, 8); TestTrue(TEXT("delta of another party refused"), S.Eval(TEXT("party_member_count == 2")).bTrue);
 { FWorldEvent E; FGrpcNightfallV1PartyLootVote V; V.PartyId = 8; V.Phase = EGrpcNightfallV1LootVotePhase::LOOT_VOTE_PHASE_REQUESTED; E.PartyLootVote = V; Dispatch(N, E); }
 TestFalse(TEXT("vote of another party refused"), S.Eval(TEXT("loot_vote_phase == REQUESTED")).bTrue);
 { FWorldEvent E; FGrpcNightfallV1PartyLootVote V; V.PartyId = 7; V.Phase = EGrpcNightfallV1LootVotePhase::LOOT_VOTE_PHASE_REQUESTED; E.PartyLootVote = V; Dispatch(N, E); }
 TestTrue(TEXT("vote of the own party"), S.Eval(TEXT("loot_vote_phase == REQUESTED")).bTrue);
 { FWorldEvent E; FGrpcNightfallV1LootDistributed L; L.Picker = Owner; L.Recipient = Partner; L.TemplateId = 57; L.Count = 3; L.Mode = EGrpcNightfallV1LootMode::LOOT_MODE_BY_TURN; E.LootDistributed = L; Dispatch(N, E); }
 TestTrue(TEXT("loot log"), S.Eval(TEXT("loot_distributed")).bTrue); TestTrue(TEXT("loot recipient is the partner"), S.Eval(TEXT("loot_recipient != self")).bTrue); TestTrue(TEXT("loot mode of the log"), S.Eval(TEXT("loot_distributed_mode == BY_TURN")).bTrue); TestTrue(TEXT("spoil false"), S.Eval(TEXT("loot_distributed_spoil == false")).bTrue); TestFalse(TEXT("one recipient is no cursor advance"), S.Eval(TEXT("by_turn_cursor_advanced")).bTrue);
 Party(14, false, 2, 7, true); TestFalse(TEXT("disband clears the party"), S.Eval(TEXT("party_member_count == 2")).bTrue); TestFalse(TEXT("disband drops the vote"), S.Eval(TEXT("loot_vote_phase == REQUESTED")).bTrue);
 { FWorldEvent E; FGrpcNightfallV1PartyInviteReceived I; I.Requester = Partner; I.ExpiresTick = 300; E.PartyInviteReceived = I; Dispatch(N, E); }
 TestTrue(TEXT("invite received"), S.Eval(TEXT("party_invite_received")).bTrue);
 // Owner-incarnation facts reset on the own respawn; identity facts reset on the next own spawn.
 { FWorldEvent E; E.XpGained = FXpGained{ Owner, 120, 1000 }; Dispatch(N, E); }
 TestTrue(TEXT("xp events counted"), S.Eval(TEXT("xp_gained_events == 1")).bTrue); TestTrue(TEXT("last xp"), S.Eval(TEXT("xp_gained_last_between 100 200")).bTrue);
 { FWorldEvent E; E.EntityRespawned = FEntityRespawned{ Owner, 30, FNetVec2{}, 100, 2 }; Dispatch(N, E); }
 TestTrue(TEXT("respawn resets owner-incarnation facts"), S.Eval(TEXT("xp_gained_events == 0")).bTrue); TestTrue(TEXT("respawn keeps identity facts"), S.Eval(TEXT("loot_distributed")).bTrue);
 S.Admit(40, 2); TestFalse(TEXT("new identity baseline clears the loot log"), S.Eval(TEXT("loot_distributed")).bTrue); TestFalse(TEXT("new identity baseline clears the invite"), S.Eval(TEXT("party_invite_received")).bTrue);
 // Owner-only carriers of another entity never reach the owner projection.
 { FWorldEvent E; FGrpcNightfallV1RecipeBookSync B; B.Baseline = true; B.Incarnation = 2; B.Tick = 41; B.Dwarven.Add(FUInt32(322)); E.RecipeBookSync = B; Dispatch(N, E); }
 TestTrue(TEXT("recipe baseline"), S.Eval(TEXT("recipe_registered 322")).bTrue);
 { FWorldEvent E; FGrpcNightfallV1RecipeBookSync B; B.Baseline = false; B.Incarnation = 1; B.Tick = 42; E.RecipeBookSync = B; Dispatch(N, E); }
 TestTrue(TEXT("old-incarnation recipe delta refused"), S.Eval(TEXT("recipe_registered 322")).bTrue);
 S.Economy->Reset(); TestFalse(TEXT("transport reset is UNKNOWN"), S.Eval(TEXT("recipe_registered 322")).bTrue);
 return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FEconomyReceiptTest, "Nightfall.Economy.Receipt.KeyedReplayAndDetachedViews", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FEconomyReceiptTest::RunTest(const FString&)
{
 using namespace EconomyTest; FScope S(*this); auto* N = S.Net; auto* E = S.Economy;
 TSharedPtr<FRecordingSocket> Socket; N->SetSocketFactoryForTesting([&](const FWsUpgradeRequest&) -> TSharedRef<IWebSocket> { Socket = MakeShared<FRecordingSocket>(); return Socket.ToSharedRef(); });
 N->SetSchedulerForTesting([](float, TFunction<void()>) {}); N->Connect(TEXT("ws://localhost:3000/ws"), TEXT("ticket")); Socket->Connected.Broadcast(); N->SetOwnEntityId(Owner);
 double Clock = 1000.0; E->SetClockForTesting([&] { return Clock; });
 S.Admit(); DispatchSpawn(N, Spawn(Merchant, 2, 10, 0, TEXT("merchant"))); DispatchSpawn(N, Spawn(Partner, 1, 10, 1, TEXT("Skillb")));
 TestTrue(TEXT("merchant known by name"), S.Eval(TEXT("merchant_known")).bTrue); TestFalse(TEXT("auctioneer absent"), S.Eval(TEXT("auctioneer_known")).bTrue);
 // Commands resolve received bindings and send one intent with the given key.
 TestFalse(TEXT("unknown role refused"), E->Execute(S.Registry.Tokenize(TEXT("nf.TradeRequest c"))));
 TestFalse(TEXT("malformed command refused"), E->Execute(S.Registry.Tokenize(TEXT("nf.Buy merchant 1 57"))));
 TestTrue(TEXT("buy sent"), E->Execute(S.Registry.Tokenize(FString::Printf(TEXT("nf.Buy merchant 314130 1294 2 %s"), Key))));
 if (!TestEqual(TEXT("one frame"), Socket->Sent.Num(), 1)) return false;
 FGrpcNightfallV1ClientMessage First; TestTrue(TEXT("sent frame decodes"), NightfallWire::DecodeClientMessage(Socket->Sent[0].GetData(), Socket->Sent[0].Num(), First));
 TestTrue(TEXT("vendor buy intent"), First.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::VendorBuy && First.Intent.VendorBuy.Merchant == Merchant && First.Intent.VendorBuy.ListId.Value == 314130 && First.Intent.VendorBuy.Items.Num() == 1 && First.Intent.VendorBuy.Items[0].TemplateId.Value == 1294 && First.Intent.VendorBuy.IdempotencyKey == Key);
 TestTrue(TEXT("pending request"), E->IsBusy());
 // D43: the same key with the same arguments resends the frozen body; other arguments with that key conflict.
 TestTrue(TEXT("retry sent"), E->Execute(S.Registry.Tokenize(FString::Printf(TEXT("nf.Buy merchant 314130 1294 2 %s"), Key))));
 FGrpcNightfallV1ClientMessage Second; TestTrue(TEXT("retry decodes"), Socket->Sent.Num() == 2 && NightfallWire::DecodeClientMessage(Socket->Sent[1].GetData(), Socket->Sent[1].Num(), Second));
 TestEqual(TEXT("frozen body resent"), Second.Intent.VendorBuy.ToJsonString(false), First.Intent.VendorBuy.ToJsonString(false)); TestTrue(TEXT("fresh seq"), Second.Seq.Value > First.Seq.Value);
 TestFalse(TEXT("same key other arguments conflict"), E->Execute(S.Registry.Tokenize(FString::Printf(TEXT("nf.Buy merchant 314130 1294 3 %s"), Key)))); TestEqual(TEXT("conflict sends nothing"), Socket->Sent.Num(), 2);
 // The receipt commits the key; its exact bytes are compared on replay; replays never count twice.
 FGrpcNightfallV1WorldEvent Ev; Ev.Event.EventCase = EGrpcNightfallV1WorldEventEvent::EconomyResult; Ev.Event.EconomyResult = Receipt(EGrpcNightfallV1ItemOperation::ITEM_OPERATION_VENDOR_BUY, 11, TEXT("r1")); Ev.Event.EconomyResult.Outcome.OutcomeCase = EGrpcNightfallV1EconomyResultOutcome::Vendor; Ev.Event.EconomyResult.Outcome.Vendor = MakeShared<FGrpcNightfallV1VendorOutcome>(); Ev.Event.EconomyResult.Outcome.Vendor->AdenaDeltaAbs = 172;
 FServerMessage Frame = Wire(Ev, *this); N->DispatchServerMessage(Frame);
 TestTrue(TEXT("receipt operation"), S.Eval(TEXT("economy_result == vendor_buy")).bTrue); TestTrue(TEXT("receipt count"), S.Eval(TEXT("economy_result_count vendor_buy == 1")).bTrue); TestFalse(TEXT("outstanding cleared"), E->IsBusy()); TestTrue(TEXT("export marker"), S.Eval(TEXT("economy_export_marker")).bTrue);
 N->DispatchServerMessage(Frame); TestTrue(TEXT("identical replay bytes match"), E->ReceiptMatchesFirst().IsSet() && *E->ReceiptMatchesFirst()); TestTrue(TEXT("replay counted once"), S.Eval(TEXT("economy_result_count vendor_buy == 1")).bTrue);
 TestFalse(TEXT("committed key with other arguments conflicts"), E->Execute(S.Registry.Tokenize(FString::Printf(TEXT("nf.Buy merchant 314130 1294 9 %s"), Key))));
 { auto Peer = Ev; Peer.Event.EconomyResult.EntityId = NightfallItems::UuidBytes(Partner); Peer.Event.EconomyResult.ReceiptId = TEXT("r2"); Peer.Event.EconomyResult.RequestKey = NightfallItems::UuidBytes(TEXT("22222222-1111-4111-8111-123456789abc")); N->DispatchServerMessage(Wire(Peer, *this)); }
 TestTrue(TEXT("a peer receipt never counts"), S.Eval(TEXT("economy_result_count vendor_buy == 1")).bTrue);
 { auto Old = Ev; Old.Event.EconomyResult.Incarnation = 9; Old.Event.EconomyResult.ReceiptId = TEXT("r3"); Old.Event.EconomyResult.RequestKey = NightfallItems::UuidBytes(TEXT("33333333-1111-4111-8111-123456789abc")); N->DispatchServerMessage(Wire(Old, *this)); }
 TestTrue(TEXT("an unknown incarnation never counts"), S.Eval(TEXT("economy_result_count vendor_buy == 1")).bTrue);
 // A rejection frees the key for a retry; CONTAINER_BUSY followed by an accepted retry is observed.
 TestTrue(TEXT("deposit needs the keeper"), !E->Execute(S.Registry.Tokenize(TEXT("nf.Deposit template:1864:5"))));
 DispatchSpawn(N, Spawn(TEXT("01970000-0000-7000-8000-00000000a002"), 2, 10, 0, TEXT("warehouse_keeper")));
 const TCHAR* DepositKey = TEXT("44444444-1111-4111-8111-123456789abc");
 TestTrue(TEXT("deposit sent"), E->Execute(S.Registry.Tokenize(FString::Printf(TEXT("nf.Deposit template:1864:5 %s"), DepositKey))));
 FGrpcNightfallV1ClientMessage Dep; TestTrue(TEXT("deposit decodes"), NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(), Socket->Sent.Last().Num(), Dep) && Dep.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::WarehouseDeposit && Dep.Intent.WarehouseDeposit.Entries.Num() == 1 && Dep.Intent.WarehouseDeposit.Entries[0].InstanceId.Value == 1);
 { FServerMessage M; M.Rejected = FIntentRejected{ Dep.Seq.Value, uint32(EGrpcNightfallV1RejectReason::REJECT_REASON_CONTAINER_BUSY), TEXT("busy") }; N->DispatchServerMessage(M); }
 TestFalse(TEXT("rejection frees the scope"), E->IsBusy()); TestFalse(TEXT("no retry yet"), S.Eval(TEXT("container_busy_retry_observed")).bTrue);
 TestTrue(TEXT("retry after busy"), E->Execute(S.Registry.Tokenize(FString::Printf(TEXT("nf.Deposit template:1864:5 %s"), DepositKey))));
 FGrpcNightfallV1ClientMessage Retry; NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(), Socket->Sent.Last().Num(), Retry); { FServerMessage M; M.Ack = FAck{ Retry.Seq.Value, 12 }; N->DispatchServerMessage(M); }
 TestTrue(TEXT("busy then accepted retry observed"), S.Eval(TEXT("container_busy_retry_observed")).bTrue);
 // Detached views: a fake provider answers; the predicate stays UNKNOWN until the view arrives; a receipt projects the received delta.
 TArray<TPair<FGrpcNightfallV1GetWarehouseRequest, USessionClient::FGetWarehouseCallback>> Pending;
 E->SetWarehouseProviderForTesting([&](const FGrpcNightfallV1GetWarehouseRequest& Q, USessionClient::FGetWarehouseCallback Cb) { Pending.Emplace(Q, MoveTemp(Cb)); });
 TestFalse(TEXT("warehouse view pending"), S.Eval(TEXT("warehouse_has 1864 == 10")).bTrue); if (!TestEqual(TEXT("one warehouse request"), Pending.Num(), 1)) return false;
 TestTrue(TEXT("request names the own character"), Pending[0].Key.CharacterId.Value == NightfallItems::UuidBytes(Owner).Value);
 FGrpcNightfallV1GetWarehouseResponse W; W.Capacity = 100; W.Revision = 3; FGrpcNightfallV1OwnedItem Row; Row.InstanceId = 900001; Row.TemplateId = 1864; Row.Count = 5; W.Items.Add(Row); Pending[0].Value({}, W);
 TestTrue(TEXT("warehouse count from the view"), S.Eval(TEXT("warehouse_has 1864 == 5")).bTrue); TestTrue(TEXT("absent template is zero"), S.Eval(TEXT("warehouse_has 119 == 0")).bTrue);
 { auto Dp = Ev; Dp.Event.EconomyResult = Receipt(EGrpcNightfallV1ItemOperation::ITEM_OPERATION_WAREHOUSE_DEPOSIT, 13, TEXT("r4"), DepositKey); Dp.Event.EconomyResult.Outcome.OutcomeCase = EGrpcNightfallV1EconomyResultOutcome::Container; Dp.Event.EconomyResult.Outcome.Container = MakeShared<FGrpcNightfallV1ContainerOutcome>(); Dp.Event.EconomyResult.Outcome.Container->Fee = 60; FGrpcNightfallV1ItemCount Mv; Mv.InstanceId = 1; Mv.Count = 5; Dp.Event.EconomyResult.Outcome.Container->Moved.Add(Mv); N->DispatchServerMessage(Wire(Dp, *this)); }
 TestTrue(TEXT("container fee"), S.Eval(TEXT("container_fee == 60")).bTrue); TestTrue(TEXT("received deposit projected onto the view"), S.Eval(TEXT("warehouse_has 1864 == 10")).bTrue); TestEqual(TEXT("refresh throttled within a second"), Pending.Num(), 1);
 Clock += 2.0; E->RefreshWarehouse(); TestEqual(TEXT("refresh after the throttle"), Pending.Num(), 2);
 TestTrue(TEXT("withdraw resolves the view instance"), E->Execute(S.Registry.Tokenize(TEXT("nf.Withdraw 1864:10"))));
 FGrpcNightfallV1ClientMessage Wd; NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(), Socket->Sent.Last().Num(), Wd); TestTrue(TEXT("withdraw names the warehouse instance"), Wd.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::WarehouseWithdraw && Wd.Intent.WarehouseWithdraw.Entries.Num() == 1 && Wd.Intent.WarehouseWithdraw.Entries[0].InstanceId.Value == 900001);
 // Mail: notification and inbox header; the fake provider is asked for both boxes.
 int32 MailRequests = 0; E->SetMailProviderForTesting([&](const FGrpcNightfallV1ListMailRequest& Q, USessionClient::FListMailCallback Cb) { ++MailRequests; FGrpcNightfallV1ListMailResponse L; if (Q.MailBox == EGrpcNightfallV1MailBox::MAIL_BOX_INBOX) { FGrpcNightfallV1MailHeader H; H.MessageId = TEXT("m1"); H.Subject = TEXT("paycheck"); H.HasAttachments = true; L.Messages.Add(H); } Cb({}, L); });
 TestFalse(TEXT("no mail yet"), S.Eval(TEXT("mail_received paycheck")).bTrue);
 Clock += 2.0; { FWorldEvent Em; FGrpcNightfallV1MailNotification Mn; Mn.MessageId = TEXT("m1"); Mn.Subject = TEXT("paycheck"); Mn.HasAttachments = true; Mn.CodAdena = 500; Em.MailNotification = Mn; Dispatch(N, Em); }
 TestTrue(TEXT("mail received"), S.Eval(TEXT("mail_received paycheck")).bTrue); TestTrue(TEXT("attachments"), S.Eval(TEXT("mail_has_attachments paycheck")).bTrue); TestTrue(TEXT("both boxes listed"), MailRequests >= 2); TestFalse(TEXT("still listed"), S.Eval(TEXT("mail_gone paycheck")).bTrue);
 TestTrue(TEXT("mail take resolves the latest notice"), E->Execute(S.Registry.Tokenize(TEXT("nf.MailTake latest"))));
 FGrpcNightfallV1ClientMessage Tk; NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(), Socket->Sent.Last().Num(), Tk); TestTrue(TEXT("take names the message"), Tk.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::MailTakeAttachments && Tk.Intent.MailTakeAttachments.MessageId == TEXT("m1"));
 // Trade, stores and roles.
 { FWorldEvent Et; FGrpcNightfallV1TradeSessionChanged Tr; Tr.Partner = Partner; Tr.Phase = EGrpcNightfallV1TradePhase::TRADE_PHASE_STARTED; Tr.Tick = 14; Et.TradeSessionChanged = Tr; Dispatch(N, Et); }
 TestTrue(TEXT("trade state"), S.Eval(TEXT("trade_state == STARTED")).bTrue); TestFalse(TEXT("no cancel reason while started"), S.Eval(TEXT("trade_cancel_reason == departure")).bTrue);
 TestTrue(TEXT("mail send uses the role name"), E->Execute(S.Registry.Tokenize(TEXT("nf.MailSend b paycheck cod 500 template:1864:3"))));
 FGrpcNightfallV1ClientMessage Ms; NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(), Socket->Sent.Last().Num(), Ms); TestTrue(TEXT("mail send body"), Ms.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::MailSend && Ms.Intent.MailSend.ReceiverName == TEXT("Skillb") && Ms.Intent.MailSend.CodAdena.Value == 500 && Ms.Intent.MailSend.Attachments.Num() == 1 && Ms.Intent.MailSend.Attachments[0].Count.Value == 3);
 { FWorldEvent Es; FGrpcNightfallV1PrivateStoreChanged St; St.Owner = Partner; St.Kind = EGrpcNightfallV1StoreKind::STORE_KIND_SELL; St.Tick = 15; FGrpcNightfallV1StoreSellEntry Se; Se.InstanceId = 77; Se.Count = 4; Se.Price = 10; St.Sell.Add(Se); Es.PrivateStoreChanged = St; Dispatch(N, Es); }
 TestTrue(TEXT("store of role b"), S.Eval(TEXT("store_state_of b == SELL")).bTrue); TestFalse(TEXT("own store unknown"), S.Eval(TEXT("store_state == SELL")).bTrue);
 TestTrue(TEXT("store buy binds the received entry by price"), E->Execute(S.Registry.Tokenize(TEXT("nf.StoreBuyFrom b template:1864:2:10"))));
 FGrpcNightfallV1ClientMessage Sb; NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(), Socket->Sent.Last().Num(), Sb); TestTrue(TEXT("store buy entry"), Sb.Intent.IntentCase == EGrpcNightfallV1ClientMessageIntent::PrivateStoreBuy && Sb.Intent.PrivateStoreBuy.StoreOwner == Partner && Sb.Intent.PrivateStoreBuy.Entries.Num() == 1 && Sb.Intent.PrivateStoreBuy.Entries[0].InstanceId.Value == 77);
 TestFalse(TEXT("store buy of an unlisted price refused"), E->Execute(S.Registry.Tokenize(TEXT("nf.StoreBuyFrom b template:1864:2:11"))));
 // Adena mark facts come from the inventory mark, never predicted.
 TestFalse(TEXT("adena delta needs a mark"), S.Eval(TEXT("adena_delta == 0")).bTrue);
 FStatsChanged Stats; Stats.Entity = Owner; Stats.Incarnation = 1; Stats.Tick = Stats.BaselineTick = 10; Stats.bBaseline = true; Stats.InventoryRevision = 1; Stats.EffectiveStats = FEffectiveStats{}; { FWorldEvent Es; Es.StatsChanged = Stats; Dispatch(N, Es); }
 TestTrue(TEXT("mark"), S.Items->Execute({ TEXT("nf.MarkItemState") })); TestTrue(TEXT("zero delta at the mark"), S.Eval(TEXT("adena_delta == 0")).bTrue);
 { FGrpcNightfallV1InventoryChanged C; C.EntityId = NightfallItems::UuidBytes(Owner); C.Incarnation = 1; C.Tick = 16; C.PreviousRevision = 1; C.InventoryRevision = 2; auto Adena = Sync().Items[0]; Adena.Count = 1000; C.Updated.Add(Adena); FWorldEvent Ec; Ec.InventoryChanged = C; Dispatch(N, Ec); }
 TestTrue(TEXT("received adena delta"), S.Eval(TEXT("adena_delta == 500")).bTrue); TestTrue(TEXT("credit increment"), S.Eval(TEXT("cod_credit_received 500")).bTrue); TestFalse(TEXT("other credit amount"), S.Eval(TEXT("cod_credit_received 499")).bTrue); TestTrue(TEXT("revision advanced counts as a sweep outcome"), S.Eval(TEXT("sweep_outcome_received")).bTrue);
 E->Reset(); TestFalse(TEXT("transport reset forgets receipts"), S.Eval(TEXT("economy_result == vendor_buy")).bTrue); TestFalse(TEXT("transport reset forgets views"), S.Eval(TEXT("trade_state == STARTED")).bTrue);
 return true;
}

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FPhase5RegistryTest, "Nightfall.Economy.Bot.Phase5ScenarioRegistry", EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
bool FPhase5RegistryTest::RunTest(const FString&)
{
 FBotPredicateRegistry Registry; Registry.RegisterBuiltins();
 TestEqual(TEXT("phase5-grammar.py predicates minus the builtin `rejected`"), EconomyBotPredicates::PredicateNames().Num(), 96); TestEqual(TEXT("phase5-grammar.py commands"), EconomyBotPredicates::CommandNames().Num(), 41);
 for (const FString& Name : EconomyBotPredicates::PredicateNames()) TestNotNull(*Name, Registry.Find(Name));
 TestNotNull(TEXT("rejected stays the Phase 1 builtin"), Registry.Find(TEXT("rejected")));
 for (const FString& Name : EconomyBotPredicates::CommandNames()) TestNotNull(*Name, IConsoleManager::Get().FindConsoleObject(*Name));
 const auto Commands = BotSteps::RegisterTestOnlyCommands();
 TArray<FString> Files; IFileManager::Get().FindFilesRecursive(Files, *FPaths::Combine(FPaths::ProjectDir(), TEXT("Scenarios")), TEXT("5-*.nfs"), true, false);
 TestEqual(TEXT("all 36 plan §2.8 sources found"), Files.Num(), 36);
 for (const auto& Path : Files)
 {
  FString Text; TestTrue(TEXT("scenario readable"), FFileHelper::LoadFileToString(Text, *Path)); FBotScenario Scenario; TArray<FString> Errors;
  const bool Parsed = BotScenario::Parse(Path, Text, Registry, [](const FString& Name) { return IConsoleManager::Get().FindConsoleObject(*Name) != nullptr; }, Scenario, Errors);
  TestTrue(Path + TEXT(": ") + FString::Join(Errors, TEXT("; ")), Parsed);
 }
 for (auto* Command : Commands) IConsoleManager::Get().UnregisterConsoleObject(Command, false);
 FString Error;
 for (const TCHAR* Text : { TEXT("rejected == CONTAINER_BUSY"), TEXT("rejected == BID_TOO_HIGH"), TEXT("economy_result == mail_mark_read"), TEXT("economy_result_count party >= 1"), TEXT("corpse_until_delta template:npc_20003 == 170"), TEXT("vendor_stock 353630 16127 == 0"), TEXT("manor_result granted 5073 >= 1"), TEXT("adena_delta == -1100"), TEXT("store_state_of c == NONE"), TEXT("trade_cancel_reason == use_item"), TEXT("rules_rate_multiplier == 1.5") })
  TestTrue(FString(TEXT("valid grammar binds: ")) + Text, bool(Registry.Parse(Registry.Tokenize(Text), Error)));
 for (const TCHAR* Text : { TEXT("economy_result == equip"), TEXT("economy_result vendor_buy"), TEXT("corpse_spoiled 20003"), TEXT("corpse_spoiled template:npc_0"), TEXT("loot_mode == FINDERS"), TEXT("party_member_count == -1"), TEXT("ground_item_kind 57 == 2"), TEXT("ground_loot_owner 57 == b"), TEXT("adena_delta == +5"), TEXT("pk_dropped_excluded == 1"), TEXT("store_state_of d == SELL"), TEXT("mail_received a@b"), TEXT("manor_result granted 5073 == 1"), TEXT("auction_leader == self"), TEXT("xp_gained_events 1"), TEXT("invented_phase5_predicate") })
  TestFalse(FString(TEXT("invalid grammar fails closed: ")) + Text, bool(Registry.Parse(Registry.Tokenize(Text), Error)));
 for (const TCHAR* Text : { TEXT("nf.PickUp nearest"), TEXT("nf.PickUp template:57 12345678-1111-4111-8111-123456789abc"), TEXT("nf.Buy merchant 314130 1294 2"), TEXT("nf.Sell merchant 314130 template:1294 3"), TEXT("nf.Refund merchant 314130 0"), TEXT("nf.Multisell exchanger 616 0 1 enchant:3"), TEXT("nf.StoreSell shop package template:1864:2:10,template:1869:2:10"), TEXT("nf.StoreBuy shop template:1870:0:5:300"), TEXT("nf.StoreManufacture shop 1:1000"), TEXT("nf.MailSend b paycheck cod 500 template:1864:3"), TEXT("nf.MailSend b cancel template:1864:2"), TEXT("nf.MailDelete latest sent"), TEXT("nf.Withdraw 1864:10"), TEXT("nf.ManorSellCrop manor_manager template:5073 5073 1 all"), TEXT("nf.PartyInvite b BY_TURN_INCLUDING_SPOIL"), TEXT("nf.PartyAnswer -1"), TEXT("nf.TradeConfirm") })
  TestTrue(FString(TEXT("valid command: ")) + Text, EconomyBotPredicates::ValidateCommand(Registry.Tokenize(Text), Error));
 for (const TCHAR* Text : { TEXT("nf.PickUp"), TEXT("nf.PickUp 57"), TEXT("nf.Buy merchant 314130 1294 0"), TEXT("nf.Refund merchant 314130 12"), TEXT("nf.Multisell exchanger 616 0 5001"), TEXT("nf.StoreSell shop bundle template:1864:2:10"), TEXT("nf.StoreBuy shop template:1870:5:300"), TEXT("nf.MailSend d paycheck"), TEXT("nf.MailSend b pay check"), TEXT("nf.MailDelete latest inbox"), TEXT("nf.PartyAnswer 2"), TEXT("nf.PartyInvite b FINDERS"), TEXT("nf.AuctionBid 32320 100000000001"), TEXT("nf.ManorBuySeed manor_manager 10 5016 3"), TEXT("nf.TradeConfirm now") })
  TestFalse(FString(TEXT("malformed command: ")) + Text, EconomyBotPredicates::ValidateCommand(Registry.Tokenize(Text), Error));
 return true;
}
#endif
