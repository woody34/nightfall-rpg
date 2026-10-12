#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Items/InventoryStateSubsystem.h"
#include "Bot/ItemBotPredicates.h"
#include "Bot/ItemAuditSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallWire.h"
#if WITH_DEV_AUTOMATION_TESTS
namespace ItemTest
{
 const TCHAR* Owner=TEXT("11111111-1111-4111-8111-111111111111");
 FGrpcNightfallV1OwnedItem Item(uint64 Id,uint32 Template,uint64 Count=1) { FGrpcNightfallV1OwnedItem I;I.InstanceId=Id;I.TemplateId=Template;I.Count=Count;I.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_INVENTORY;return I; }
 FEntitySpawn Spawn(const FString& Id=Owner,uint64 Tick=10,uint32 Inc=1) {FEntitySpawn S;S.EntityId=Id;S.Kind=1;S.StateTick=Tick;S.LifeIncarnation=Inc;return S;}
 FGrpcNightfallV1InventorySync Sync() {FGrpcNightfallV1InventorySync S;S.EntityId=NightfallItems::UuidBytes(Owner);S.Incarnation=1;S.Tick=S.BaselineTick=10;S.Baseline=true;S.InventoryRevision=1;S.HasWeight=S.HasPenalties=true;S.RulesHash.Value.Init(1,32);S.Items.Add(Item(1,6));return S;}
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemStateTest,"Nightfall.Item.Reducer.HydrationRevisionPrivacy",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemStateTest::RunTest(const FString&)
{
 using namespace ItemTest;FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(Owner);
 S->ApplySync(Sync());TestFalse(TEXT("identity before baseline"),S->HasBaseline());S->ApplySpawn(Spawn());S->ApplySync(Sync());TestTrue(TEXT("same-tick hydration"),S->HasBaseline());
 auto Other=Sync();Other.EntityId=NightfallItems::UuidBytes(TEXT("22222222-2222-4222-8222-222222222222"));Other.Items.Reset();S->ApplySync(Other);TestEqual(TEXT("private peer baseline ignored"),*S->Count(6),uint64(1));
 FGrpcNightfallV1WeightChanged W;W.EntityId=Sync().EntityId;W.Incarnation=1;W.Tick=11;W.InventoryRevision=2;W.HasWeight=true;W.Weight.PerMille=500;S->ApplyWeight(W);
 FGrpcNightfallV1InventoryChanged C;C.EntityId=W.EntityId;C.Incarnation=1;C.Tick=11;C.PreviousRevision=1;C.InventoryRevision=2;C.Added.Add(Item(2,2509,100));S->ApplyChanged(C);
 TestEqual(TEXT("early sibling retained"),S->GetInventory().Weight.PerMille,int64(500));
 W.Weight.PerMille=600;S->ApplyWeight(W);TestEqual(TEXT("same-revision sibling applies"),S->GetInventory().Weight.PerMille,int64(600));
 FStatsChanged Stats;Stats.Entity=Owner;Stats.Incarnation=1;Stats.Tick=11;Stats.InventoryRevision=2;FGrpcNightfallV1ItemPenaltyState P;P.WeaponLevel=1;Stats.ItemPenalties=P;S->ApplyStats(Stats);TestEqual(TEXT("stats sibling applies"),S->GetInventory().Penalties.WeaponLevel.Value,uint32(1));
 W.Tick=20;W.Weight.PerMille=700;S->ApplyWeight(W);W.Tick=12;W.Weight.PerMille=800;S->ApplyWeight(W);TestEqual(TEXT("older same-revision summary rejected"),S->GetInventory().Weight.PerMille,int64(700));
 C.Tick=10;C.InventoryRevision=3;C.PreviousRevision=2;S->ApplyChanged(C);TestEqual(TEXT("stale baseline tick rejected"),S->Revision(),uint64(2));
 C.Tick=12;C.InventoryRevision=4;C.PreviousRevision=3;S->ApplyChanged(C);TestTrue(TEXT("revision gap fails closed"),S->NeedsBaseline());TestFalse(TEXT("gap never predicts counts"),S->Count(6).IsSet());
 S->Reset();S->ApplySpawn(Spawn());auto Invalid=Sync();Invalid.Items.Add(Invalid.Items[0]);S->ApplySync(Invalid);TestFalse(TEXT("duplicate IDs atomic refusal"),S->HasBaseline());
 S->Reset();S->ApplySpawn(Spawn());S->ApplySync(Sync());auto R=FEntityRespawned{};R.Entity=Owner;R.Tick=11;R.Incarnation=2;S->ApplyRespawn(R);S->ApplySync(Sync());TestFalse(TEXT("old-life baseline rejected"),S->HasBaseline());
 FGrpcNightfallV1GetInventoryResponse Detached;Detached.HasInventory=true;Detached.Inventory=Sync();Detached.Inventory.Tick=Detached.Inventory.BaselineTick=0;Detached.Inventory.Incarnation=0;S->ApplyDetached(Detached);TestTrue(TEXT("detached sentinel accepted separately"),S->Detached().IsSet());TestFalse(TEXT("detached cannot hydrate"),S->HasBaseline());
 return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemWireTest,"Nightfall.Item.Codec.IntentsAndEnrichments",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemWireTest::RunTest(const FString&)
{
 FClientMessage M;M.Seq=1;FGrpcNightfallV1EnchantItemRequest Q;Q.IdempotencyKey=TEXT("12345678-1111-4111-8111-123456789abc");Q.ScrollInstanceId=MAX_int64;Q.TargetInstanceId=2;Q._support_instance_id._support_instance_idCase=EGrpcNightfallV1EnchantItemRequest_support_instance_id::SupportInstanceId;Q._support_instance_id.SupportInstanceId=3;M.EnchantItem=Q;
 TArray<uint8> Bytes;NightfallProto::Encode(M,Bytes);FGrpcNightfallV1ClientMessage Decoded;TestTrue(TEXT("decode intent"),NightfallWire::DecodeClientMessage(Bytes.GetData(),Bytes.Num(),Decoded));TestTrue(TEXT("intent selected"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::EnchantItem);TestEqual(TEXT("bigint-safe exact identity"),Decoded.Intent.EnchantItem->ScrollInstanceId.Value,uint64(MAX_int64));
 FGrpcNightfallV1ServerMessage Server;Server.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;Server.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();Server.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::InventorySync;Server.Payload.Event->Event.InventorySync=MakeShared<FGrpcNightfallV1InventorySync>(ItemTest::Sync());NightfallWire::EncodeServerMessage(Server,Bytes);FServerMessage Native;TestTrue(TEXT("decode baseline"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),Native));TestTrue(TEXT("baseline preserved"),Native.Event.IsSet() && Native.Event->InventorySync.IsSet());
 return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemBotTest,"Nightfall.Item.Bot.GrammarUnknownAndReset",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemBotTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(ItemTest::Owner);FBotContext C;C.GameInstance=Scope.GameInstance;FBotPredicateRegistry R;R.RegisterBuiltins();
 auto Eval=[&](const TCHAR* Text){FString E;auto P=R.Parse(R.Tokenize(Text),E);TestTrue(TEXT("bind predicate"),bool(P));return P?P(C):FBotPredicateValue{};};
 for(const TCHAR* Text:{TEXT("inventory_has 6 == 0"),TEXT("equipped rhand == 0"),TEXT("weight_pct == 0"),TEXT("item_rpc_error == none"),TEXT("item_receipt_matches_first")})TestFalse(TEXT("absence UNKNOWN"),Eval(Text).bTrue);
 S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());TestTrue(TEXT("received bag count"),Eval(TEXT("inventory_has 6 == 1")).bTrue);TestTrue(TEXT("exact received percent"),Eval(TEXT("weight_pct == 0/3")).bTrue);
 for(const TCHAR* Text:{TEXT("nf.Equip template:6 rhand"),TEXT("nf.UseItem instance:1 self"),TEXT("nf.Enchant 951 6 support:instance:2"),TEXT("nf.Refine 6 8723"),TEXT("nf.ExchangeSA 6 18446744073709551615"),TEXT("nf.AutoShot 2509 off")}){FString E;TestTrue(TEXT("valid E6 command"),ItemBotPredicates::ValidateCommand(R.Tokenize(Text),E));}
 for(const TCHAR* Text:{TEXT("nf.Equip instance:9223372036854775808"),TEXT("nf.Enchant 0 6"),TEXT("nf.AutoShot 1 maybe"),TEXT("nf.Refine 6 8723 self"),TEXT("nf.ItemCatalog extra")}){FString E;TestFalse(TEXT("malformed command"),ItemBotPredicates::ValidateCommand(R.Tokenize(Text),E));}
 S->Reset();TestFalse(TEXT("transport reset UNKNOWN"),Eval(TEXT("inventory_has 6 == 1")).bTrue);return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemCatalogTest,"Nightfall.Item.Catalog.FailureHashAndLifetime",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemCatalogTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* S=Scope.Get<UInventoryStateSubsystem>();auto* N=Scope.Get<UNetClientSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());
 USessionClient::FGetItemCatalogCallback Pending;S->SetCatalogProviderForTesting([&](auto Callback){Pending=MoveTemp(Callback);});S->LoadCatalog();FNetResult Failed;Failed.Error=ENetError::Unavailable;Failed.Message=TEXT("catalogue unavailable");Pending(Failed,{});TestFalse(TEXT("optional unavailable is fail-closed"),S->HasCatalog());TestTrue(TEXT("completed failure status visible"),S->RpcDone() && S->RpcResult().Error==ENetError::Unavailable);
 FGrpcNightfallV1GetItemCatalogResponse C;C.DataVersion=2;C.RulesHash=ItemTest::Sync().RulesHash;S->LoadCatalog();Pending({},C);TestTrue(TEXT("matching immutable catalogue"),S->HasCatalog());C.RulesHash.Value[0]=2;S->ApplyCatalog(C);TestFalse(TEXT("mismatched bundle fails closed"),S->HasCatalog());
 C.RulesHash=ItemTest::Sync().RulesHash;S->LoadCatalog();S->Reset();Pending({},C);TestFalse(TEXT("old transport callback discarded"),S->HasCatalog());
 return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemPredicateCarriersTest,"Nightfall.Item.Bot.AllReceivedCarriers",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemPredicateCarriersTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());auto Base=ItemTest::Sync();auto& I=Base.Items[0];I.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND;I.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;I.Enchant=3;FGrpcNightfallV1PaperdollEntry Doll;Doll.Slot=I.Slot;Doll.InstanceId=I.InstanceId;Base.Paperdoll.Add(Doll);Base.Weight.Weight=1;Base.Weight.MaxLoad=3;Base.Weight.PerMille=333;Base.Penalties.WeaponLevel=1;Base.Penalties.ArmorLevel=2;Base.Penalties.OverloadLevel=3;Base.AutoShotTemplates.Add(2509);S->ApplySync(Base);
 FBotPredicateRegistry R;R.RegisterBuiltins();FBotContext C;C.GameInstance=Scope.GameInstance;auto Eval=[&](const TCHAR* T){FString Error;auto P=R.Parse(R.Tokenize(T),Error);TestTrue(TEXT("predicate binds"),bool(P));return P?P(C):FBotPredicateValue{};};
 for(const TCHAR* T:{TEXT("item_baseline"),TEXT("inventory_revision == 1"),TEXT("inventory_has 6 >= 1"),TEXT("equipped rhand == 6"),TEXT("equipped lhand == 0"),TEXT("enchant_level rhand == 3"),TEXT("weight_pct == 100/3"),TEXT("weight_pct > 33.333"),TEXT("overload_level == 3"),TEXT("grade_penalty weapon == 1"),TEXT("grade_penalty armor == 2"),TEXT("autoshot_on 2509"),TEXT("arrows_remaining == 0")})TestTrue(FString(T),Eval(T).bTrue);
 TestFalse(TEXT("cleared slot enchant UNKNOWN"),Eval(TEXT("enchant_level lhand == 0")).bTrue);TestFalse(TEXT("display rounding is not equality"),Eval(TEXT("weight_pct == 33.3")).bTrue);
 FGrpcNightfallV1GetItemCatalogResponse Catalog;Catalog.DataVersion=2;Catalog.RulesHash=Base.RulesHash;S->ApplyCatalog(Catalog);TestTrue(TEXT("catalogue readiness"),Eval(TEXT("item_catalog_ready")).bTrue);
 FGrpcNightfallV1ItemOperationResult Op;Op.EntityId=Base.EntityId;Op.Incarnation=1;Op.Tick=11;Op.InventoryRevision=1;Op.ReceiptId=TEXT("12345678-1111-4111-8111-123456789abc");Op.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP;FWorldEvent E;E.ItemOperationResult=Op;FServerMessage M;M.Event=E;N->DispatchServerMessage(M);TestTrue(TEXT("only event33 establishes operation"),Eval(TEXT("item_operation == equip")).bTrue);N->DispatchServerMessage(M);TestTrue(TEXT("exact frozen bytes match"),Eval(TEXT("item_receipt_matches_first")).bTrue);
 auto Change=FGrpcNightfallV1InventoryChanged{};Change.EntityId=Base.EntityId;Change.Incarnation=1;Change.Tick=11;Change.PreviousRevision=1;Change.InventoryRevision=2;auto Updated=Base.Items[0];Updated._augment_id._augment_idCase=EGrpcNightfallV1OwnedItem_augment_id::AugmentId;Updated._augment_id.AugmentId=MAX_uint32;Change.Updated.Add(Updated);S->ApplyChanged(Change);
 FGrpcNightfallV1RefineResult Refine;Refine.EntityId=Base.EntityId;Refine.Incarnation=1;Refine.Tick=11;Refine.InventoryRevision=2;Refine.ReceiptId=Op.ReceiptId;Refine.TargetInstanceId=1;Refine.Outcome=EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED;Refine._augment_id._augment_idCase=EGrpcNightfallV1RefineResult_augment_id::AugmentId;Refine._augment_id.AugmentId=MAX_uint32;S->ApplyRefine(Refine);TestTrue(TEXT("refine carrier normalized"),Eval(TEXT("refine_outcome == applied")).bTrue);TestTrue(TEXT("augment joins result and owner item"),Eval(TEXT("refined instance:1")).bTrue);TestTrue(TEXT("refine doesn't masquerade as event33"),Eval(TEXT("item_operation == equip")).bTrue);
 Base.Weight.MaxLoad=0;S->Reset();S->ApplySpawn(ItemTest::Spawn());S->ApplySync(Base);TestFalse(TEXT("source nonpositive max load UNKNOWN"),Eval(TEXT("weight_pct == 0")).bTrue);return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemAllIntentsTest,"Nightfall.Item.Codec.AllTenIntents",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemAllIntentsTest::RunTest(const FString&)
{
 const FString Key=TEXT("12345678-1111-4111-8111-123456789abc");
 auto Verify=[&](const FClientMessage& M,EGrpcNightfallV1ClientMessageIntent Expected){TArray<uint8>B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage R;TestTrue(TEXT("wire decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),R));TestTrue(TEXT("allocated item intent retained"),R.Intent.IntentCase==Expected);};
 {FClientMessage M;FGrpcNightfallV1EquipItemRequest Q;Q.IdempotencyKey=Key;M.EquipItem=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::EquipItem);}
 {FClientMessage M;FGrpcNightfallV1UnequipItemRequest Q;Q.IdempotencyKey=Key;M.UnequipItem=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::UnequipItem);}
 {FClientMessage M;FGrpcNightfallV1UseItemRequest Q;Q.IdempotencyKey=Key;M.UseItem=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::UseItem);}
 {FClientMessage M;FGrpcNightfallV1DestroyItemRequest Q;Q.IdempotencyKey=Key;M.DestroyItem=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::DestroyItem);}
 {FClientMessage M;FGrpcNightfallV1EnchantItemRequest Q;Q.IdempotencyKey=Key;M.EnchantItem=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::EnchantItem);}
 {FClientMessage M;FGrpcNightfallV1ToggleAutoShotRequest Q;Q.IdempotencyKey=Key;M.ToggleAutoShot=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::ToggleAutoShot);}
 {FClientMessage M;FGrpcNightfallV1RefineItemRequest Q;Q.IdempotencyKey=Key;M.RefineItem=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::RefineItem);}
 {FClientMessage M;FGrpcNightfallV1CancelRefineRequest Q;Q.IdempotencyKey=Key;M.CancelRefine=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::CancelRefine);}
 {FClientMessage M;FGrpcNightfallV1ExchangeSpecialAbilityRequest Q;Q.IdempotencyKey=Key;M.ExchangeSpecialAbility=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::ExchangeSpecialAbility);}
 {FClientMessage M;FGrpcNightfallV1CrystallizeItemRequest Q;Q.IdempotencyKey=Key;M.CrystallizeItem=Q;Verify(M,EGrpcNightfallV1ClientMessageIntent::CrystallizeItem);}
 return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemEnrichedCodecTest,"Nightfall.Item.Codec.PublicSpawnAndOwnerStats",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemEnrichedCodecTest::RunTest(const FString&)
{
 // Values here are transport sentinels, not gameplay expectations.
 FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();auto& E=M.Payload.Event->Event;E.EventCase=EGrpcNightfallV1WorldEventEvent::StatsChanged;E.StatsChanged=MakeShared<FGrpcNightfallV1StatsChanged>();auto& S=*E.StatsChanged;S.Entity=ItemTest::Owner;S.HasItemWeight=S.HasItemPenalties=S.HasEffectiveStats=true;S.InventoryRevision=MAX_uint64;S.ItemWeight.Weight=123;S.ItemPenalties.WeaponLevel=2;S.EffectiveStats.ShieldDefQ=11;S.EffectiveStats.ShieldRateQ=12;S.EffectiveStats.ShieldAngleDegrees=13;S.EffectiveStats.PhysicalReachMillitiles=14;S.EffectiveStats.PhysicalAngleDegrees=15;
 TArray<uint8>B;NightfallWire::EncodeServerMessage(M,B);FServerMessage R;TestTrue(TEXT("decode enriched owner stats"),NightfallProto::Decode(B.GetData(),B.Num(),R));if(!TestTrue(TEXT("stats selected"),R.Event.IsSet() && R.Event->StatsChanged.IsSet()))return false;const auto& V=*R.Event->StatsChanged;TestEqual(TEXT("revision width"),V.InventoryRevision,uint64(MAX_uint64));TestTrue(TEXT("weight and penalty presence"),V.ItemWeight.IsSet() && V.ItemPenalties.IsSet());TestEqual(TEXT("shield defence"),V.EffectiveStats->ShieldDefQ,int64(11));TestEqual(TEXT("shield rate"),V.EffectiveStats->ShieldRateQ,int64(12));TestEqual(TEXT("shield angle"),V.EffectiveStats->ShieldAngleDegrees,uint32(13));TestEqual(TEXT("physical reach"),V.EffectiveStats->PhysicalReachMillitiles,int64(14));TestEqual(TEXT("physical angle"),V.EffectiveStats->PhysicalAngleDegrees,uint32(15));
 E.EventCase=EGrpcNightfallV1WorldEventEvent::Spawn;E.Spawn=MakeShared<FGrpcNightfallV1EntitySpawn>();E.Spawn->EntityId=ItemTest::Owner;FGrpcNightfallV1VisibleEquipment Gear;Gear.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND;Gear.TemplateId=6;Gear.GlowTier=2;E.Spawn->VisibleEquipment.Add(Gear);NightfallWire::EncodeServerMessage(M,B);TestTrue(TEXT("public equipped spawn decode"),NightfallProto::Decode(B.GetData(),B.Num(),R));TestEqual(TEXT("public gear preserved"),R.Event->Spawn->VisibleEquipment.Num(),1);E.Spawn->VisibleEquipment.Reset();NightfallWire::EncodeServerMessage(M,B);NightfallProto::Decode(B.GetData(),B.Num(),R);TestEqual(TEXT("present-empty public gear retained"),R.Event->Spawn->VisibleEquipment.Num(),0);return true;
}
#endif

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemAuditTest,"Nightfall.Item.Audit.StickyCoherenceAndReset",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemAuditTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto* A=Scope.Get<UItemAuditSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());FString Detail;TestFalse(TEXT("no outcome isn't a pass"),A->Check(Detail).IsSet());
 FGrpcNightfallV1EnchantResult R;R.EntityId=ItemTest::Sync().EntityId;R.Incarnation=1;R.Tick=11;R.InventoryRevision=1;R.TargetInstanceId=1;R.ReceiptId=TEXT("12345678-1111-4111-8111-123456789abc");R.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS;R.NewEnchant=1;FWorldEvent E;E.EnchantResult=R;FServerMessage M;M.Event=E;N->DispatchServerMessage(M);const auto Invalid=A->Check(Detail);TestTrue(TEXT("mismatch is an observed failure"),Invalid.IsSet() && !*Invalid);
 R.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SAFE_FAILED;R.NewEnchant=0;E.EnchantResult=R;M.Event=E;N->DispatchServerMessage(M);TestTrue(TEXT("later coherent fact can't erase violation"),A->Check(Detail).IsSet() && !*A->Check(Detail));A->Reset();TestFalse(TEXT("transport reset invalidates audit"),A->Check(Detail).IsSet());return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemHistoricalReceiptTest,"Nightfall.Item.Receipt.RevivalHistoryIsNotLiveState",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemHistoricalReceiptTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());
 FGrpcNightfallV1ItemOperationResult R;R.EntityId=ItemTest::Sync().EntityId;R.Incarnation=1;R.Tick=11;R.InventoryRevision=1;R.ReceiptId=TEXT("12345678-1111-4111-8111-123456789abc");R.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP;FWorldEvent E;E.ItemOperationResult=R;FServerMessage M;M.Event=E;N->DispatchServerMessage(M);
 S->ApplySpawn(ItemTest::Spawn(ItemTest::Owner,20,2));auto B=ItemTest::Sync();B.Tick=B.BaselineTick=20;B.Incarnation=2;S->ApplySync(B);N->DispatchServerMessage(M);TestTrue(TEXT("same frozen historical bytes compare across revival"),S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst());TestFalse(TEXT("historical receipt never becomes new life operation"),S->LastOperation().IsSet());TestEqual(TEXT("historical receipt cannot modify inventory"),S->Revision(),uint64(1));return true;
}
#endif
