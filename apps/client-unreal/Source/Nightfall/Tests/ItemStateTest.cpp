#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Items/InventoryStateSubsystem.h"
#include "Bot/ItemBotPredicates.h"
#include "Bot/ItemAuditSubsystem.h"
#include "Bot/BotPredicates.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallWire.h"
#include "Misc/Base64.h"
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
 using namespace ItemTest;FScopedTestGameInstance Scope;Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(Owner);
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
 S->Reset();S->ApplySpawn(Spawn());auto Invalid=Sync();const auto Duplicate=Invalid.Items[0];Invalid.Items.Add(Duplicate);S->ApplySync(Invalid);TestFalse(TEXT("duplicate IDs atomic refusal"),S->HasBaseline());
 S->Reset();S->ApplySpawn(Spawn());S->ApplySync(Sync());auto R=FEntityRespawned{};R.Entity=Owner;R.Tick=11;R.Incarnation=2;S->ApplyRespawn(R);S->ApplySync(Sync());TestFalse(TEXT("old-life baseline rejected"),S->HasBaseline());
 FGrpcNightfallV1GetInventoryResponse Detached;Detached.HasInventory=true;Detached.Inventory=Sync();Detached.Inventory.Tick=Detached.Inventory.BaselineTick=0;Detached.Inventory.Incarnation=0;S->SetInventoryProviderForTesting([](const auto&,auto){});S->LoadDetached(Owner);S->ApplyDetached(Owner,Detached);TestTrue(TEXT("detached sentinel accepted separately"),S->Detached(Owner)!=nullptr);TestFalse(TEXT("detached cannot hydrate"),S->HasBaseline());
 return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemWireTest,"Nightfall.Item.Codec.IntentsAndEnrichments",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemWireTest::RunTest(const FString&)
{
 FClientMessage M;M.Seq=1;FGrpcNightfallV1EnchantItemRequest Q;Q.IdempotencyKey=TEXT("12345678-1111-4111-8111-123456789abc");Q.ScrollInstanceId=MAX_int64;Q.TargetInstanceId=2;Q._support_instance_id._support_instance_idCase=EGrpcNightfallV1EnchantItemRequest_support_instance_id::SupportInstanceId;Q._support_instance_id.SupportInstanceId=3;M.EnchantItem=Q;
 TArray<uint8> Bytes;NightfallProto::Encode(M,Bytes);FGrpcNightfallV1ClientMessage Decoded;TestTrue(TEXT("decode intent"),NightfallWire::DecodeClientMessage(Bytes.GetData(),Bytes.Num(),Decoded));TestTrue(TEXT("intent selected"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::EnchantItem);TestEqual(TEXT("bigint-safe exact identity"),Decoded.Intent.EnchantItem.ScrollInstanceId.Value,uint64(MAX_int64));
 FGrpcNightfallV1ServerMessage Server;Server.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;Server.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();Server.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::InventorySync;Server.Payload.Event->Event.InventorySync=ItemTest::Sync();NightfallWire::EncodeServerMessage(Server,Bytes);FServerMessage Native;TestTrue(TEXT("decode baseline"),NightfallProto::Decode(Bytes.GetData(),Bytes.Num(),Native));TestTrue(TEXT("baseline preserved"),Native.Event.IsSet() && Native.Event->InventorySync.IsSet());
 return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemBotTest,"Nightfall.Item.Bot.GrammarUnknownAndReset",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemBotTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(ItemTest::Owner);FBotContext C;C.GameInstance=Scope.GameInstance;FBotPredicateRegistry R;R.RegisterBuiltins();
 auto Eval=[&](const TCHAR* Text){FString E;auto P=R.Parse(R.Tokenize(Text),E);TestTrue(TEXT("bind predicate"),bool(P));return P?P(C):FBotPredicateValue{};};
 for(const TCHAR* Text:{TEXT("inventory_has 6 == 0"),TEXT("equipped rhand == 0"),TEXT("weight_pct == 0"),TEXT("item_rpc_error == none"),TEXT("item_receipt_matches_first")})TestFalse(TEXT("absence UNKNOWN"),Eval(Text).bTrue);
 S->ApplySpawn(ItemTest::Spawn());auto ZeroWeight=ItemTest::Sync();ZeroWeight.Weight.Weight=0;ZeroWeight.Weight.MaxLoad=1;S->ApplySync(ZeroWeight);TestTrue(TEXT("received bag count"),Eval(TEXT("inventory_has 6 == 1")).bTrue);TestTrue(TEXT("exact received percent"),Eval(TEXT("weight_pct == 0/3")).bTrue);
 for(const TCHAR* Text:{TEXT("nf.Equip template:6 rhand"),TEXT("nf.UseItem instance:1 self"),TEXT("nf.Enchant 951 6 support:instance:2"),TEXT("nf.Refine 6 8723"),TEXT("nf.ExchangeSA 6 18446744073709551615"),TEXT("nf.AutoShot 2509 off")}){FString E;TestTrue(TEXT("valid E6 command"),ItemBotPredicates::ValidateCommand(R.Tokenize(Text),E));}
 for(const TCHAR* Text:{TEXT("nf.Equip instance:9223372036854775808"),TEXT("nf.Enchant 0 6"),TEXT("nf.AutoShot 1 maybe"),TEXT("nf.Refine 6 8723 self"),TEXT("nf.ItemCatalog extra")}){FString E;TestFalse(TEXT("malformed command"),ItemBotPredicates::ValidateCommand(R.Tokenize(Text),E));}
 S->Reset();TestFalse(TEXT("transport reset UNKNOWN"),Eval(TEXT("inventory_has 6 == 1")).bTrue);return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemCatalogTest,"Nightfall.Item.Catalog.FailureHashAndLifetime",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemCatalogTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* S=Scope.Get<UInventoryStateSubsystem>();Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());
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
 FScopedTestGameInstance Scope;Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());auto Base=ItemTest::Sync();auto& I=Base.Items[0];I.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND;I.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;I.Enchant=3;FGrpcNightfallV1PaperdollEntry Doll;Doll.Slot=I.Slot;Doll.InstanceId=I.InstanceId;Base.Paperdoll.Add(Doll);Base.Weight.Weight=1;Base.Weight.MaxLoad=3;Base.Weight.PerMille=333;Base.Penalties.WeaponLevel=1;Base.Penalties.ArmorLevel=2;Base.Penalties.OverloadLevel=3;Base.AutoShotTemplates.Add(2509);S->ApplySync(Base);
 FBotPredicateRegistry R;R.RegisterBuiltins();FBotContext C;C.GameInstance=Scope.GameInstance;auto Eval=[&](const TCHAR* T){FString Error;auto P=R.Parse(R.Tokenize(T),Error);TestTrue(TEXT("predicate binds"),bool(P));return P?P(C):FBotPredicateValue{};};
 for(const TCHAR* T:{TEXT("item_baseline"),TEXT("inventory_revision == 1"),TEXT("inventory_has 6 >= 1"),TEXT("equipped rhand == 6"),TEXT("equipped lhand == 0"),TEXT("enchant_level rhand == 3"),TEXT("weight_pct == 100/3"),TEXT("weight_pct > 33.333"),TEXT("overload_level == 3"),TEXT("grade_penalty weapon == 1"),TEXT("grade_penalty armor == 2"),TEXT("autoshot_on 2509"),TEXT("arrows_remaining == 0")})TestTrue(FString(T),Eval(T).bTrue);
 TestFalse(TEXT("cleared slot enchant UNKNOWN"),Eval(TEXT("enchant_level lhand == 0")).bTrue);TestFalse(TEXT("display rounding is not equality"),Eval(TEXT("weight_pct == 33.3")).bTrue);
 FGrpcNightfallV1GetItemCatalogResponse Catalog;Catalog.DataVersion=2;Catalog.RulesHash=Base.RulesHash;S->ApplyCatalog(Catalog);TestTrue(TEXT("catalogue readiness"),Eval(TEXT("item_catalog_ready")).bTrue);
 FGrpcNightfallV1ItemOperationResult Op;Op.EntityId=Base.EntityId;Op.Incarnation=1;Op.Tick=11;Op.InventoryRevision=1;Op.ReceiptId=TEXT("12345678-1111-4111-8111-123456789abc");Op.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP;FWorldEvent E;E.ItemOperationResult=Op;FServerMessage M;M.Event=E;N->DispatchServerMessage(M);TestTrue(TEXT("only event33 establishes operation"),Eval(TEXT("item_operation == equip")).bTrue);N->DispatchServerMessage(M);TestTrue(TEXT("exact frozen bytes match"),Eval(TEXT("item_receipt_matches_first")).bTrue);
 auto Change=FGrpcNightfallV1InventoryChanged{};Change.EntityId=Base.EntityId;Change.Incarnation=1;Change.Tick=11;Change.PreviousRevision=1;Change.InventoryRevision=2;auto Updated=Base.Items[0];Updated._augment_id._augment_idCase=EGrpcNightfallV1OwnedItem_augment_id::AugmentId;Updated._augment_id.AugmentId=MAX_uint32;Change.Updated.Add(Updated);S->ApplyChanged(Change);
 FGrpcNightfallV1RefineResult Refine;Refine.EntityId=Base.EntityId;Refine.Incarnation=1;Refine.Tick=11;Refine.InventoryRevision=2;Refine.ReceiptId=Op.ReceiptId;Refine.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_REFINE;Refine.TargetInstanceId=1;Refine.Outcome=EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED;Refine._augment_id._augment_idCase=EGrpcNightfallV1RefineResult_augment_id::AugmentId;Refine._augment_id.AugmentId=MAX_uint32;S->ApplyRefine(Refine);TestTrue(TEXT("refine carrier normalized"),Eval(TEXT("refine_outcome == applied")).bTrue);TestTrue(TEXT("augment joins result and owner item"),Eval(TEXT("refined instance:1")).bTrue);TestTrue(TEXT("refine doesn't masquerade as event33"),Eval(TEXT("item_operation == equip")).bTrue);
 FGrpcNightfallV1ItemDefinition Arrow;Arrow.TemplateId=17;Arrow.Name=TEXT("Wooden Arrow");Arrow.Kind=EGrpcNightfallV1ItemKind::ITEM_KIND_ETC;Arrow.Grade=EGrpcNightfallV1ItemGrade::ITEM_GRADE_NG;Arrow.Stackable=true;Catalog.Items.Add(Arrow);S->ApplyCatalog(Catalog);
 Change.PreviousRevision=2;Change.InventoryRevision=3;Change.Tick=12;Change.Updated.Reset();auto Arrows=ItemTest::Item(2,17,5);Arrows.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_LHAND;Arrows.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;Change.Added.Add(Arrows);S->ApplyChanged(Change);TestTrue(TEXT("received ARROW stack count"),Eval(TEXT("arrows_remaining == 5")).bTrue);
 Base.Weight.MaxLoad=0;S->Reset();S->ApplySpawn(ItemTest::Spawn());S->ApplySync(Base);TestFalse(TEXT("source nonpositive max load UNKNOWN"),Eval(TEXT("weight_pct == 0")).bTrue);return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemAllIntentsTest,"Nightfall.Item.Codec.AllTenIntents",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemAllIntentsTest::RunTest(const FString&)
{
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1EquipItemRequest Q;
Q.IdempotencyKey=TEXT("EquipItemRequest-idempotency_key");
Q.InstanceId=uint64(MAX_uint64-2);
Q.PreferredSlot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
M.EquipItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("EquipItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("EquipItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::EquipItem);
TestEqual(TEXT("EquipItem all payload fields optional absent"),Decoded.Intent.EquipItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1EquipItemRequest Q;
Q.IdempotencyKey=TEXT("EquipItemRequest-idempotency_key");
Q.InstanceId=uint64(MAX_uint64-2);
Q.PreferredSlot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
M.EquipItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("EquipItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("EquipItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::EquipItem);
TestEqual(TEXT("EquipItem all payload fields optional present"),Decoded.Intent.EquipItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1UnequipItemRequest Q;
Q.IdempotencyKey=TEXT("UnequipItemRequest-idempotency_key");
Q.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
M.UnequipItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("UnequipItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("UnequipItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::UnequipItem);
TestEqual(TEXT("UnequipItem all payload fields optional absent"),Decoded.Intent.UnequipItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1UnequipItemRequest Q;
Q.IdempotencyKey=TEXT("UnequipItemRequest-idempotency_key");
Q.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
M.UnequipItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("UnequipItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("UnequipItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::UnequipItem);
TestEqual(TEXT("UnequipItem all payload fields optional present"),Decoded.Intent.UnequipItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1UseItemRequest Q;
Q.IdempotencyKey=TEXT("UseItemRequest-idempotency_key");
Q.InstanceId=uint64(MAX_uint64-2);
M.UseItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("UseItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("UseItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::UseItem);
TestEqual(TEXT("UseItem all payload fields optional absent"),Decoded.Intent.UseItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1UseItemRequest Q;
Q.IdempotencyKey=TEXT("UseItemRequest-idempotency_key");
Q.InstanceId=uint64(MAX_uint64-2);
Q.HasTarget=true;
Q.Target.EntityId.Value.Init(18,16);
Q.Target.Incarnation=uint32(MAX_uint32-2);
M.UseItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("UseItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("UseItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::UseItem);
TestEqual(TEXT("UseItem all payload fields optional present"),Decoded.Intent.UseItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1DestroyItemRequest Q;
Q.IdempotencyKey=TEXT("DestroyItemRequest-idempotency_key");
Q.InstanceId=uint64(MAX_uint64-2);
Q.Count=uint64(MAX_uint64-3);
M.DestroyItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("DestroyItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("DestroyItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::DestroyItem);
TestEqual(TEXT("DestroyItem all payload fields optional absent"),Decoded.Intent.DestroyItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1DestroyItemRequest Q;
Q.IdempotencyKey=TEXT("DestroyItemRequest-idempotency_key");
Q.InstanceId=uint64(MAX_uint64-2);
Q.Count=uint64(MAX_uint64-3);
M.DestroyItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("DestroyItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("DestroyItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::DestroyItem);
TestEqual(TEXT("DestroyItem all payload fields optional present"),Decoded.Intent.DestroyItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1EnchantItemRequest Q;
Q.IdempotencyKey=TEXT("EnchantItemRequest-idempotency_key");
Q.ScrollInstanceId=uint64(MAX_uint64-2);
Q.TargetInstanceId=uint64(MAX_uint64-3);
M.EnchantItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("EnchantItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("EnchantItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::EnchantItem);
TestEqual(TEXT("EnchantItem all payload fields optional absent"),Decoded.Intent.EnchantItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1EnchantItemRequest Q;
Q.IdempotencyKey=TEXT("EnchantItemRequest-idempotency_key");
Q.ScrollInstanceId=uint64(MAX_uint64-2);
Q.TargetInstanceId=uint64(MAX_uint64-3);
Q._support_instance_id._support_instance_idCase=EGrpcNightfallV1EnchantItemRequest_support_instance_id::SupportInstanceId;
Q._support_instance_id.SupportInstanceId=uint64(MAX_uint64-4);
M.EnchantItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("EnchantItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("EnchantItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::EnchantItem);
TestEqual(TEXT("EnchantItem all payload fields optional present"),Decoded.Intent.EnchantItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1ToggleAutoShotRequest Q;
Q.IdempotencyKey=TEXT("ToggleAutoShotRequest-idempotency_key");
Q.TemplateId=uint32(MAX_uint32-2);
Q.Enabled=false;
M.ToggleAutoShot=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("ToggleAutoShot decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("ToggleAutoShot case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::ToggleAutoShot);
TestEqual(TEXT("ToggleAutoShot all payload fields optional absent"),Decoded.Intent.ToggleAutoShot.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1ToggleAutoShotRequest Q;
Q.IdempotencyKey=TEXT("ToggleAutoShotRequest-idempotency_key");
Q.TemplateId=uint32(MAX_uint32-2);
Q.Enabled=true;
M.ToggleAutoShot=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("ToggleAutoShot decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("ToggleAutoShot case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::ToggleAutoShot);
TestEqual(TEXT("ToggleAutoShot all payload fields optional present"),Decoded.Intent.ToggleAutoShot.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1RefineItemRequest Q;
Q.IdempotencyKey=TEXT("RefineItemRequest-idempotency_key");
Q.TargetInstanceId=uint64(MAX_uint64-3);
Q.StoneInstanceId=uint64(MAX_uint64-4);
M.RefineItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("RefineItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("RefineItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::RefineItem);
TestEqual(TEXT("RefineItem all payload fields optional absent"),Decoded.Intent.RefineItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1RefineItemRequest Q;
Q.IdempotencyKey=TEXT("RefineItemRequest-idempotency_key");
Q.HasNpc=true;
Q.Npc.EntityId.Value.Init(18,16);
Q.Npc.Incarnation=uint32(MAX_uint32-2);
Q.TargetInstanceId=uint64(MAX_uint64-3);
Q.StoneInstanceId=uint64(MAX_uint64-4);
M.RefineItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("RefineItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("RefineItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::RefineItem);
TestEqual(TEXT("RefineItem all payload fields optional present"),Decoded.Intent.RefineItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1CancelRefineRequest Q;
Q.IdempotencyKey=TEXT("CancelRefineRequest-idempotency_key");
Q.TargetInstanceId=uint64(MAX_uint64-3);
M.CancelRefine=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("CancelRefine decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("CancelRefine case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::CancelRefine);
TestEqual(TEXT("CancelRefine all payload fields optional absent"),Decoded.Intent.CancelRefine.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1CancelRefineRequest Q;
Q.IdempotencyKey=TEXT("CancelRefineRequest-idempotency_key");
Q.HasNpc=true;
Q.Npc.EntityId.Value.Init(18,16);
Q.Npc.Incarnation=uint32(MAX_uint32-2);
Q.TargetInstanceId=uint64(MAX_uint64-3);
M.CancelRefine=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("CancelRefine decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("CancelRefine case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::CancelRefine);
TestEqual(TEXT("CancelRefine all payload fields optional present"),Decoded.Intent.CancelRefine.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1ExchangeSpecialAbilityRequest Q;
Q.IdempotencyKey=TEXT("ExchangeSpecialAbilityRequest-idempotency_key");
Q.TargetInstanceId=uint64(MAX_uint64-3);
Q.RecipeId=uint64(MAX_uint64-4);
M.ExchangeSpecialAbility=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("ExchangeSpecialAbility decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("ExchangeSpecialAbility case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::ExchangeSpecialAbility);
TestEqual(TEXT("ExchangeSpecialAbility all payload fields optional absent"),Decoded.Intent.ExchangeSpecialAbility.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1ExchangeSpecialAbilityRequest Q;
Q.IdempotencyKey=TEXT("ExchangeSpecialAbilityRequest-idempotency_key");
Q.HasNpc=true;
Q.Npc.EntityId.Value.Init(18,16);
Q.Npc.Incarnation=uint32(MAX_uint32-2);
Q.TargetInstanceId=uint64(MAX_uint64-3);
Q.RecipeId=uint64(MAX_uint64-4);
M.ExchangeSpecialAbility=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("ExchangeSpecialAbility decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("ExchangeSpecialAbility case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::ExchangeSpecialAbility);
TestEqual(TEXT("ExchangeSpecialAbility all payload fields optional present"),Decoded.Intent.ExchangeSpecialAbility.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1CrystallizeItemRequest Q;
Q.IdempotencyKey=TEXT("CrystallizeItemRequest-idempotency_key");
Q.InstanceId=uint64(MAX_uint64-2);
M.CrystallizeItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("CrystallizeItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("CrystallizeItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::CrystallizeItem);
TestEqual(TEXT("CrystallizeItem all payload fields optional absent"),Decoded.Intent.CrystallizeItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
 {FClientMessage M;M.Seq=MAX_uint32;FGrpcNightfallV1CrystallizeItemRequest Q;
Q.IdempotencyKey=TEXT("CrystallizeItemRequest-idempotency_key");
Q.InstanceId=uint64(MAX_uint64-2);
M.CrystallizeItem=Q;TArray<uint8> B;NightfallProto::Encode(M,B);FGrpcNightfallV1ClientMessage Decoded;
TestTrue(TEXT("CrystallizeItem decode"),NightfallWire::DecodeClientMessage(B.GetData(),B.Num(),Decoded));TestTrue(TEXT("CrystallizeItem case"),Decoded.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::CrystallizeItem);
TestEqual(TEXT("CrystallizeItem all payload fields optional present"),Decoded.Intent.CrystallizeItem.ToJsonString(false),Q.ToJsonString(false));TestEqual(TEXT("sequence boundary"),Decoded.Seq.Value,uint32(MAX_uint32));}
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
 FScopedTestGameInstance Scope;Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto* A=Scope.Get<UItemAuditSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());FString Detail;TestFalse(TEXT("no outcome isn't a pass"),A->Check(Detail).IsSet());
 FGrpcNightfallV1EnchantResult R;R.EntityId=ItemTest::Sync().EntityId;R.Incarnation=1;R.Tick=11;R.InventoryRevision=1;R.TargetInstanceId=1;R.ReceiptId=TEXT("12345678-1111-4111-8111-123456789abc");R.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_ENCHANT;R.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS;R.NewEnchant=1;FWorldEvent E;E.EnchantResult=R;FServerMessage M;M.Event=E;N->DispatchServerMessage(M);const auto Invalid=A->Check(Detail);TestTrue(TEXT("mismatch is an observed failure"),Invalid.IsSet() && !*Invalid);
 R.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_ENCHANT;R.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SAFE_FAILED;R.NewEnchant=0;E.EnchantResult=R;M.Event=E;N->DispatchServerMessage(M);TestTrue(TEXT("later coherent fact can't erase violation"),A->Check(Detail).IsSet() && !*A->Check(Detail));A->Reset();TestFalse(TEXT("transport reset invalidates audit"),A->Check(Detail).IsSet());return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemHistoricalReceiptTest,"Nightfall.Item.Receipt.RevivalHistoryIsNotLiveState",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemHistoricalReceiptTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());
 FGrpcNightfallV1ItemOperationResult R;R.EntityId=ItemTest::Sync().EntityId;R.Incarnation=1;R.Tick=11;R.InventoryRevision=1;R.ReceiptId=TEXT("12345678-1111-4111-8111-123456789abc");R.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP;FWorldEvent E;E.ItemOperationResult=R;FServerMessage M;M.Event=E;N->DispatchServerMessage(M);
 S->ApplySpawn(ItemTest::Spawn(ItemTest::Owner,20,2));auto B=ItemTest::Sync();B.Tick=B.BaselineTick=20;B.Incarnation=2;S->ApplySync(B);N->DispatchServerMessage(M);TestTrue(TEXT("same frozen historical bytes compare across revival"),S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst());TestFalse(TEXT("historical receipt never becomes new life operation"),S->LastOperation().IsSet());TestEqual(TEXT("historical receipt cannot modify inventory"),S->Revision(),uint64(1));return true;
}
#endif

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemMarkTest,"Nightfall.Item.Bot.ReceivedMarksAndStatsFences",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemMarkTest::RunTest(const FString&)
{
 using namespace ItemTest;
 FScopedTestGameInstance Scope;Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(Owner);
 FBotPredicateRegistry Registry;Registry.RegisterBuiltins();FBotContext Context;Context.GameInstance=Scope.GameInstance;
 auto Eval=[&](const TCHAR* Text){FString Error;auto Predicate=Registry.Parse(Registry.Tokenize(Text),Error);TestTrue(TEXT("revised predicate binds"),bool(Predicate));return Predicate?Predicate(Context):FBotPredicateValue{};};
 TestFalse(TEXT("missing carriers cannot mark"),S->Execute({TEXT("nf.MarkItemState")}));
 S->ApplySpawn(Spawn());
 FStatsChanged Stats;Stats.Entity=Owner;Stats.Incarnation=1;Stats.Tick=Stats.BaselineTick=10;Stats.bBaseline=true;Stats.InventoryRevision=1;Stats.MaxHp=100;Stats.MaxMp=20;Stats.MaxCp=30;Stats.EffectiveStats=FEffectiveStats{};Stats.EffectiveStats->PDefQ=MIN_int64;
 S->ApplyStats(Stats);TestFalse(TEXT("stats alone cannot mark"),S->MarkItemState());S->ApplySync(Sync());
 TestTrue(TEXT("baseline stats preceding inventory retained"),S->Execute({TEXT("nf.MarkItemState")}));
 TestTrue(TEXT("exact initial revision mark"),Eval(TEXT("inventory_revision_since_mark == 0")).bTrue);TestTrue(TEXT("complete received sheet equals mark"),Eval(TEXT("owner_stats_match_mark")).bTrue);
 FGrpcNightfallV1InventoryChanged Change;Change.EntityId=Sync().EntityId;Change.Incarnation=1;Change.Tick=11;Change.PreviousRevision=1;Change.InventoryRevision=2;Change.Added.Add(Item(2,17));S->ApplyChanged(Change);
 TestTrue(TEXT("revision delta is received"),Eval(TEXT("inventory_revision_since_mark == 1")).bTrue);TestFalse(TEXT("old revision stats cannot establish delta"),Eval(TEXT("owner_stat_delta max_hp == 0")).bTrue);
 Stats.bBaseline=false;Stats.Tick=11;Stats.InventoryRevision=2;Stats.MaxHp=110;Stats.EffectiveStats->PDefQ=MAX_int64;S->ApplyStats(Stats);
 TestTrue(TEXT("received maxima increase"),Eval(TEXT("owner_stat_delta max_hp == 10")).bTrue);TestTrue(TEXT("signed sheet delta keeps full width"),Eval(TEXT("owner_stat_delta p_def == 18446744073709551615")).bTrue);TestFalse(TEXT("different sheet does not match"),Eval(TEXT("owner_stats_match_mark")).bTrue);
 Stats.Tick=12;Stats.MaxHp=90;Stats.EffectiveStats->PDefQ=MIN_int64;S->ApplyStats(Stats);TestTrue(TEXT("negative received delta"),Eval(TEXT("owner_stat_delta max_hp == -10")).bTrue);
 Stats.Tick=11;Stats.MaxHp=999;S->ApplyStats(Stats);TestTrue(TEXT("stale same-revision stats discarded"),Eval(TEXT("owner_stat_delta max_hp == -10")).bTrue);
 Stats.Entity=TEXT("22222222-2222-4222-8222-222222222222");Stats.Tick=13;S->ApplyStats(Stats);TestTrue(TEXT("peer cannot replace owner stats"),Eval(TEXT("owner_stat_delta max_hp == -10")).bTrue);
 S->Reset();TestFalse(TEXT("reconnect projections UNKNOWN"),Eval(TEXT("owner_stats_match_mark")).bTrue);
 S->ApplySpawn(Spawn(Owner,20,2));auto Base=Sync();Base.Tick=Base.BaselineTick=20;Base.Incarnation=2;Base.InventoryRevision=2;S->ApplySync(Base);
 Stats.Entity=Owner;Stats.Incarnation=2;Stats.Tick=Stats.BaselineTick=20;Stats.bBaseline=true;Stats.MaxHp=100;Stats.InventoryRevision=2;S->ApplyStats(Stats);
 TestTrue(TEXT("same character mark retained across incarnation"),Eval(TEXT("owner_stats_match_mark")).bTrue);TestTrue(TEXT("retained comparison revision"),Eval(TEXT("inventory_revision_since_mark == 1")).bTrue);
 Stats.Hp=1;Stats.Mp=2;Stats.Cp=3;S->ApplyStats(Stats);TestTrue(TEXT("current vitals excluded from sheet equality"),Eval(TEXT("owner_stats_match_mark")).bTrue);
 N->SetOwnEntityId(TEXT("22222222-2222-4222-8222-222222222222"));S->Reset();N->SetOwnEntityId(Owner);S->ApplySpawn(Spawn());S->ApplySync(Sync());TestFalse(TEXT("character switch invalidates comparison mark"),S->RevisionSinceMark().IsSet());
 FString Error;for(const TCHAR* Text:{TEXT("nf.MarkItemState extra"),TEXT("nf.MarkItemState 12345678-1111-4111-8111-123456789abc")})TestFalse(TEXT("mark takes no arguments or key"),ItemBotPredicates::ValidateCommand(Registry.Tokenize(Text),Error));
 for(const TCHAR* Text:{TEXT("owner_stat_delta hp == 0"),TEXT("owner_stat_delta max_hp == +1"),TEXT("item_operation == refine"),TEXT("item_operation == cancel_refine")})TestFalse(TEXT("revised malformed grammar rejected"),bool(Registry.Parse(Registry.Tokenize(Text),Error)));
 return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemRefineJoinTest,"Nightfall.Item.Bot.RefineAppliedRemovedRevisionJoin",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemRefineJoinTest::RunTest(const FString&)
{
 using namespace ItemTest;FScopedTestGameInstance Scope;Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(Owner);S->ApplySpawn(Spawn());S->ApplySync(Sync());
 FBotPredicateRegistry Registry;Registry.RegisterBuiltins();FBotContext Context;Context.GameInstance=Scope.GameInstance;auto Eval=[&](const TCHAR* Text){FString Error;auto Predicate=Registry.Parse(Registry.Tokenize(Text),Error);TestTrue(TEXT("refine predicate binds"),bool(Predicate));return Predicate?Predicate(Context):FBotPredicateValue{};};
 TestFalse(TEXT("no result UNKNOWN"),Eval(TEXT("refine_result_consistent")).bTrue);
 FGrpcNightfallV1RefineResult Result;Result.EntityId=Sync().EntityId;Result.Incarnation=1;Result.Tick=11;Result.InventoryRevision=2;Result.TargetInstanceId=1;Result.Outcome=EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED;Result._augment_id._augment_idCase=EGrpcNightfallV1RefineResult_augment_id::AugmentId;Result._augment_id.AugmentId=MAX_uint32;S->ApplyRefine(Result);
 TestTrue(TEXT("received applied outcome"),Eval(TEXT("refine_outcome == applied")).bTrue);TestFalse(TEXT("future result requires owner delta"),Eval(TEXT("refine_result_consistent")).bTrue);
 FGrpcNightfallV1InventoryChanged Change;Change.EntityId=Result.EntityId;Change.Incarnation=1;Change.Tick=11;Change.PreviousRevision=1;Change.InventoryRevision=2;auto Updated=Item(1,6);Updated._augment_id._augment_idCase=EGrpcNightfallV1OwnedItem_augment_id::AugmentId;Updated._augment_id.AugmentId=MAX_uint32;Change.Updated.Add(Updated);S->ApplyChanged(Change);TestTrue(TEXT("applied result joins exact unsigned augment"),Eval(TEXT("refine_result_consistent")).bTrue);
 Result.Tick=12;Result.InventoryRevision=3;Result.Outcome=EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_REMOVED;Result._augment_id._augment_idCase=EGrpcNightfallV1RefineResult_augment_id::NotSet;S->ApplyRefine(Result);
 TestTrue(TEXT("cancellation uses received event32 outcome"),Eval(TEXT("refine_outcome == removed")).bTrue);TestFalse(TEXT("removed waits for inventory revision"),Eval(TEXT("refine_result_consistent")).bTrue);
 Change.Tick=12;Change.PreviousRevision=2;Change.InventoryRevision=3;Change.Updated[0]._augment_id._augment_idCase=EGrpcNightfallV1OwnedItem_augment_id::NotSet;S->ApplyChanged(Change);TestTrue(TEXT("removed requires augment absence"),Eval(TEXT("refine_result_consistent")).bTrue);TestFalse(TEXT("cancellation never becomes operation33"),S->LastOperation().IsSet());
 Result._augment_id._augment_idCase=EGrpcNightfallV1RefineResult_augment_id::AugmentId;S->ApplyRefine(Result);TestFalse(TEXT("malformed removed payload is inconsistent"),Eval(TEXT("refine_result_consistent")).bTrue);
 S->Reset();TestFalse(TEXT("reset outcome UNKNOWN"),Eval(TEXT("refine_outcome == removed")).bTrue);TestFalse(TEXT("reset join UNKNOWN"),Eval(TEXT("refine_result_consistent")).bTrue);return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemBaselineReplayTest,"Nightfall.Item.Reducer.DuplicateBaselineKeepsNewerSiblings",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemBaselineReplayTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());auto Base=ItemTest::Sync();S->ApplySync(Base);
 FStatsChanged Stats;Stats.Entity=ItemTest::Owner;Stats.Incarnation=1;Stats.Tick=11;Stats.InventoryRevision=1;Stats.MaxHp=123;Stats.EffectiveStats=FEffectiveStats{};S->ApplyStats(Stats);TestTrue(TEXT("new stats mark"),S->MarkItemState());
 FGrpcNightfallV1WeightChanged W;W.EntityId=Base.EntityId;W.Incarnation=1;W.Tick=11;W.InventoryRevision=1;W.HasWeight=true;W.Weight.Weight=77;S->ApplyWeight(W);
 FGrpcNightfallV1PenaltyChanged P;P.EntityId=Base.EntityId;P.Incarnation=1;P.Tick=11;P.InventoryRevision=1;P.HasPenalties=true;P.Penalties.OverloadLevel=4;S->ApplyPenalty(P);S->ApplySync(Base);
 TestEqual(TEXT("duplicate baseline keeps newer weight"),S->GetInventory().Weight.Weight,int64(77));TestEqual(TEXT("duplicate baseline keeps newer penalty"),S->GetInventory().Penalties.OverloadLevel.Value,uint32(4));TestTrue(TEXT("duplicate baseline keeps newer stats"),S->OwnerStatsMatchMark().IsSet() && *S->OwnerStatsMatchMark());
 Base.Tick=Base.BaselineTick=9;S->ApplySync(Base);TestEqual(TEXT("older baseline keeps weight"),S->GetInventory().Weight.Weight,int64(77));
 S->ApplySpawn(ItemTest::Spawn(ItemTest::Owner,20,2));Base.Tick=Base.BaselineTick=20;Base.Incarnation=2;Base.Weight.Weight=88;S->ApplySync(Base);TestTrue(TEXT("fresh identity hydrates"),S->HasBaseline());TestEqual(TEXT("fresh identity weight"),S->GetInventory().Weight.Weight,int64(88));
 S->Reset();S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());TestTrue(TEXT("reconnect permits original baseline"),S->HasBaseline());return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemRawReceiptTest,"Nightfall.Item.Receipt.ExactReceivedPayloadAndBounds",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemRawReceiptTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;Scope.Get<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();N->SetOwnEntityId(ItemTest::Owner);S->ApplySpawn(ItemTest::Spawn());S->ApplySync(ItemTest::Sync());
 FGrpcNightfallV1ItemOperationResult Result;Result.EntityId=ItemTest::Sync().EntityId;Result.Incarnation=1;Result.Tick=11;Result.InventoryRevision=1;Result.ReceiptId=TEXT("12345678-1111-4111-8111-123456789abc");Result.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP;
 TArray<uint8> Payload;NightfallWire::EncodeItemReceipt(Result,Payload);
 auto Varint=[](TArray<uint8>& B,uint64 V){while(V>=128){B.Add(uint8(V)|128);V>>=7;}B.Add(uint8(V));};
 auto Frame=[&](const TArray<uint8>& P){TArray<uint8> Event;Varint(Event,(33<<3)|2);Varint(Event,P.Num());Event.Append(P);TArray<uint8> B;B.Add(0x12);Varint(B,Event.Num());B.Append(Event);return B;};
 auto Receive=[&](const TArray<uint8>& B){FServerMessage M;TestTrue(TEXT("received frame decodes"),NightfallProto::Decode(B.GetData(),B.Num(),M));N->DispatchServerMessage(M);return M;};
 auto Original=Frame(Payload);auto A=Receive(Original);Receive(Original);TestTrue(TEXT("identical replay bytes pass"),S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst());
 // An explicitly encoded empty packed field is equivalent to absent affected IDs.
 Payload.Add(0x3a);Payload.Add(0);
 auto Distinct=Frame(Payload);auto B=Receive(Distinct);TestEqual(TEXT("same decoded result"),B.Event->ItemOperationResult->ToJsonString(false),A.Event->ItemOperationResult->ToJsonString(false));TestTrue(TEXT("byte-distinct receipt fails equality"),S->ReceiptMatchesFirst().IsSet() && !*S->ReceiptMatchesFirst());
 TArray<uint8> Extracted;NightfallWire::ItemReceiptBytes(Distinct.GetData(),Distinct.Num(),Extracted);TestTrue(TEXT("original payload preserved"),Extracted==Payload);
 for(int32 Size=0;Size<Distinct.Num();++Size){NightfallWire::ItemReceiptBytes(Distinct.GetData(),Size,Extracted);TestTrue(TEXT("truncated envelope rejected"),Extracted.IsEmpty());}NightfallWire::ItemReceiptBytes(nullptr,1,Extracted);TestTrue(TEXT("null bounds rejected"),Extracted.IsEmpty());NightfallWire::ItemReceiptBytes(nullptr,-1,Extracted);TestTrue(TEXT("negative bounds rejected"),Extracted.IsEmpty());return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemAuthenticationScopeTest,"Nightfall.Item.Receipt.StableAccountAcrossRefreshAndSessionRenewal",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemAuthenticationScopeTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* Session=Scope.Get<USessionClient>();
 const FString Account=TEXT("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
 auto Token=[&](int32 Exp){FString Payload=FBase64::Encode(FString::Printf(TEXT("{\"sub\":\"%s\",\"exp\":%d}"),*Account,Exp)).Replace(TEXT("+"),TEXT("-")).Replace(TEXT("/"),TEXT("_"));Payload.RemoveFromEnd(TEXT("="));Payload.RemoveFromEnd(TEXT("="));return TEXT("e30.")+Payload+TEXT(".signature");};
 Session->SetBearerToken(Token(1));TestEqual(TEXT("JWT subject is stable account UUID"),Session->AccountId(),Account);const uint64 First=Session->AuthenticationGeneration();
 Session->SetBearerToken(Token(2));TestEqual(TEXT("refresh preserves account"),Session->AccountId(),Account);TestEqual(TEXT("refresh preserves authenticated session"),Session->AuthenticationGeneration(),First);
 Session->SetBearerToken(TEXT("test:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"));TestEqual(TEXT("dev token maps same normalized account"),Session->AccountId(),Account);TestEqual(TEXT("same account token format doesn't change session"),Session->AuthenticationGeneration(),First);
 Session->SetBearerToken(TEXT(""));TestTrue(TEXT("logout removes account"),Session->AccountId().IsEmpty());Session->SetBearerToken(Token(3));TestTrue(TEXT("relogin creates a fresh session"),Session->AuthenticationGeneration()>First);
 Session->SetBearerToken(TEXT("opaque-token"));TestTrue(TEXT("unknown subject fails closed"),Session->AccountId().IsEmpty());return true;
}
#endif
