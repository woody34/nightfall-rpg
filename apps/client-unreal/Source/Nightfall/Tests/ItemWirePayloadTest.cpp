#include "Misc/AutomationTest.h"
#include "Net/ProtoCodec.h"
#include "NightfallWire.h"
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemAllEventsTest,"Nightfall.Item.Codec.AllEightEventPayloads",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemAllEventsTest::RunTest(const FString&)
{
 {FGrpcNightfallV1InventorySync Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.BaselineTick=uint64(MAX_uint64-5);
Q.Baseline=true;
{
auto& Nested=Q.Items.AddDefaulted_GetRef();
Nested.InstanceId=uint64(MAX_uint64-1);
Nested.TemplateId=uint32(MAX_uint32-2);
Nested.Count=uint64(MAX_uint64-3);
Nested.Enchant=uint32(MAX_uint32-4);
Nested.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.ChargeBits=uint32(MAX_uint32-8);
Nested.Revision=uint64(MAX_uint64-9);
}
{
auto& Nested=Q.Paperdoll.AddDefaulted_GetRef();
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.InstanceId=uint64(MAX_uint64-2);
}
Q.AutoShotTemplates.Add(uint32(MAX_uint32-9));
{
auto& Nested=Q.Reuse.AddDefaulted_GetRef();
Nested.Kind=EGrpcNightfallV1ItemReuseKind::ITEM_REUSE_KIND_SHARED_GROUP;
Nested.Key=uint32(MAX_uint32-2);
Nested.UntilTick=uint64(MAX_uint64-3);
}
Q.RulesHash.Value.Init(30,32);
Q.OrdinaryCapacity=uint32(MAX_uint32-14);
Q.QuestCapacity=uint32(MAX_uint32-15);
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::InventorySync;M.Payload.Event->Event.InventorySync=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("InventorySync decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("InventorySync dispatch"),Decoded.Event.IsSet() && Decoded.Event->InventorySync.IsSet()))return false;
TestEqual(TEXT("InventorySync all fields presence False"),Decoded.Event->InventorySync->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1InventorySync Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.BaselineTick=uint64(MAX_uint64-5);
Q.Baseline=true;
{
auto& Nested=Q.Items.AddDefaulted_GetRef();
Nested.InstanceId=uint64(MAX_uint64-1);
Nested.TemplateId=uint32(MAX_uint32-2);
Nested.Count=uint64(MAX_uint64-3);
Nested.Enchant=uint32(MAX_uint32-4);
Nested._augment_id._augment_idCase=EGrpcNightfallV1OwnedItem_augment_id::AugmentId;
Nested._augment_id.AugmentId=uint32(MAX_uint32-5);
Nested.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.ChargeBits=uint32(MAX_uint32-8);
Nested.Revision=uint64(MAX_uint64-9);
}
{
auto& Nested=Q.Paperdoll.AddDefaulted_GetRef();
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.InstanceId=uint64(MAX_uint64-2);
}
Q.AutoShotTemplates.Add(uint32(MAX_uint32-9));
Q.HasWeight=true;
Q.Weight.Weight=MIN_int64;
Q.Weight.MaxLoad=MAX_int64;
Q.Weight.PenaltyBonus=MIN_int64;
Q.Weight.PerMille=MAX_int64;
Q.HasPenalties=true;
Q.Penalties.Expertise=uint32(MAX_uint32-1);
Q.Penalties.WeaponLevel=uint32(MAX_uint32-2);
Q.Penalties.ArmorLevel=uint32(MAX_uint32-3);
Q.Penalties.OverloadLevel=uint32(MAX_uint32-4);
Q.Penalties.Overloaded=true;
{
auto& Nested=Q.Reuse.AddDefaulted_GetRef();
Nested.Kind=EGrpcNightfallV1ItemReuseKind::ITEM_REUSE_KIND_SHARED_GROUP;
Nested.Key=uint32(MAX_uint32-2);
Nested.UntilTick=uint64(MAX_uint64-3);
}
Q.RulesHash.Value.Init(30,32);
Q.OrdinaryCapacity=uint32(MAX_uint32-14);
Q.QuestCapacity=uint32(MAX_uint32-15);
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::InventorySync;M.Payload.Event->Event.InventorySync=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("InventorySync decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("InventorySync dispatch"),Decoded.Event.IsSet() && Decoded.Event->InventorySync.IsSet()))return false;
TestEqual(TEXT("InventorySync all fields presence True"),Decoded.Event->InventorySync->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1InventoryChanged Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.PreviousRevision=uint64(MAX_uint64-5);
{
auto& Nested=Q.Added.AddDefaulted_GetRef();
Nested.InstanceId=uint64(MAX_uint64-1);
Nested.TemplateId=uint32(MAX_uint32-2);
Nested.Count=uint64(MAX_uint64-3);
Nested.Enchant=uint32(MAX_uint32-4);
Nested.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.ChargeBits=uint32(MAX_uint32-8);
Nested.Revision=uint64(MAX_uint64-9);
}
Q.RemovedIds.Add(uint64(MAX_uint64-7));
{
auto& Nested=Q.Updated.AddDefaulted_GetRef();
Nested.InstanceId=uint64(MAX_uint64-1);
Nested.TemplateId=uint32(MAX_uint32-2);
Nested.Count=uint64(MAX_uint64-3);
Nested.Enchant=uint32(MAX_uint32-4);
Nested.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.ChargeBits=uint32(MAX_uint32-8);
Nested.Revision=uint64(MAX_uint64-9);
}
Q.AutoShotTemplates.Add(uint32(MAX_uint32-9));
{
auto& Nested=Q.Reuse.AddDefaulted_GetRef();
Nested.Kind=EGrpcNightfallV1ItemReuseKind::ITEM_REUSE_KIND_SHARED_GROUP;
Nested.Key=uint32(MAX_uint32-2);
Nested.UntilTick=uint64(MAX_uint64-3);
}
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::InventoryChanged;M.Payload.Event->Event.InventoryChanged=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("InventoryChanged decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("InventoryChanged dispatch"),Decoded.Event.IsSet() && Decoded.Event->InventoryChanged.IsSet()))return false;
TestEqual(TEXT("InventoryChanged all fields presence False"),Decoded.Event->InventoryChanged->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1InventoryChanged Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.PreviousRevision=uint64(MAX_uint64-5);
{
auto& Nested=Q.Added.AddDefaulted_GetRef();
Nested.InstanceId=uint64(MAX_uint64-1);
Nested.TemplateId=uint32(MAX_uint32-2);
Nested.Count=uint64(MAX_uint64-3);
Nested.Enchant=uint32(MAX_uint32-4);
Nested._augment_id._augment_idCase=EGrpcNightfallV1OwnedItem_augment_id::AugmentId;
Nested._augment_id.AugmentId=uint32(MAX_uint32-5);
Nested.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.ChargeBits=uint32(MAX_uint32-8);
Nested.Revision=uint64(MAX_uint64-9);
}
Q.RemovedIds.Add(uint64(MAX_uint64-7));
{
auto& Nested=Q.Updated.AddDefaulted_GetRef();
Nested.InstanceId=uint64(MAX_uint64-1);
Nested.TemplateId=uint32(MAX_uint32-2);
Nested.Count=uint64(MAX_uint64-3);
Nested.Enchant=uint32(MAX_uint32-4);
Nested._augment_id._augment_idCase=EGrpcNightfallV1OwnedItem_augment_id::AugmentId;
Nested._augment_id.AugmentId=uint32(MAX_uint32-5);
Nested.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.ChargeBits=uint32(MAX_uint32-8);
Nested.Revision=uint64(MAX_uint64-9);
}
Q.AutoShotTemplates.Add(uint32(MAX_uint32-9));
{
auto& Nested=Q.Reuse.AddDefaulted_GetRef();
Nested.Kind=EGrpcNightfallV1ItemReuseKind::ITEM_REUSE_KIND_SHARED_GROUP;
Nested.Key=uint32(MAX_uint32-2);
Nested.UntilTick=uint64(MAX_uint64-3);
}
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::InventoryChanged;M.Payload.Event->Event.InventoryChanged=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("InventoryChanged decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("InventoryChanged dispatch"),Decoded.Event.IsSet() && Decoded.Event->InventoryChanged.IsSet()))return false;
TestEqual(TEXT("InventoryChanged all fields presence True"),Decoded.Event->InventoryChanged->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1EquipmentChanged Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
{
auto& Nested=Q.Visible.AddDefaulted_GetRef();
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.TemplateId=uint32(MAX_uint32-2);
Nested.GlowTier=uint32(MAX_uint32-3);
}
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::EquipmentChanged;M.Payload.Event->Event.EquipmentChanged=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("EquipmentChanged decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("EquipmentChanged dispatch"),Decoded.Event.IsSet() && Decoded.Event->EquipmentChanged.IsSet()))return false;
TestEqual(TEXT("EquipmentChanged all fields presence False"),Decoded.Event->EquipmentChanged->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1EquipmentChanged Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
{
auto& Nested=Q.Visible.AddDefaulted_GetRef();
Nested.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_BELT;
Nested.TemplateId=uint32(MAX_uint32-2);
Nested.GlowTier=uint32(MAX_uint32-3);
}
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::EquipmentChanged;M.Payload.Event->Event.EquipmentChanged=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("EquipmentChanged decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("EquipmentChanged dispatch"),Decoded.Event.IsSet() && Decoded.Event->EquipmentChanged.IsSet()))return false;
TestEqual(TEXT("EquipmentChanged all fields presence True"),Decoded.Event->EquipmentChanged->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1WeightChanged Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::WeightChanged;M.Payload.Event->Event.WeightChanged=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("WeightChanged decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("WeightChanged dispatch"),Decoded.Event.IsSet() && Decoded.Event->WeightChanged.IsSet()))return false;
TestEqual(TEXT("WeightChanged all fields presence False"),Decoded.Event->WeightChanged->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1WeightChanged Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.HasWeight=true;
Q.Weight.Weight=MIN_int64;
Q.Weight.MaxLoad=MAX_int64;
Q.Weight.PenaltyBonus=MIN_int64;
Q.Weight.PerMille=MAX_int64;
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::WeightChanged;M.Payload.Event->Event.WeightChanged=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("WeightChanged decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("WeightChanged dispatch"),Decoded.Event.IsSet() && Decoded.Event->WeightChanged.IsSet()))return false;
TestEqual(TEXT("WeightChanged all fields presence True"),Decoded.Event->WeightChanged->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1PenaltyChanged Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::PenaltyChanged;M.Payload.Event->Event.PenaltyChanged=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("PenaltyChanged decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("PenaltyChanged dispatch"),Decoded.Event.IsSet() && Decoded.Event->PenaltyChanged.IsSet()))return false;
TestEqual(TEXT("PenaltyChanged all fields presence False"),Decoded.Event->PenaltyChanged->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1PenaltyChanged Q;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.HasPenalties=true;
Q.Penalties.Expertise=uint32(MAX_uint32-1);
Q.Penalties.WeaponLevel=uint32(MAX_uint32-2);
Q.Penalties.ArmorLevel=uint32(MAX_uint32-3);
Q.Penalties.OverloadLevel=uint32(MAX_uint32-4);
Q.Penalties.Overloaded=true;
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::PenaltyChanged;M.Payload.Event->Event.PenaltyChanged=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("PenaltyChanged decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("PenaltyChanged dispatch"),Decoded.Event.IsSet() && Decoded.Event->PenaltyChanged.IsSet()))return false;
TestEqual(TEXT("PenaltyChanged all fields presence True"),Decoded.Event->PenaltyChanged->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1EnchantResult Q;
Q.RequestKey.Value.Init(0xab,16);Q.RequestFingerprint=TEXT("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");Q.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_ENCHANT;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.ReceiptId=TEXT("EnchantResult-receipt_id");
Q.TargetInstanceId=uint64(MAX_uint64-6);
Q.OldEnchant=uint32(MAX_uint32-7);
Q.NewEnchant=uint32(MAX_uint32-8);
Q.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS_NO_INCREMENT;
{
auto& Nested=Q.Refunds.AddDefaulted_GetRef();
Nested.TemplateId=uint32(MAX_uint32-1);
Nested.Count=uint64(MAX_uint64-2);
Nested.InstanceId=uint64(MAX_uint64-3);
}
Q.AnnouncementTier=uint32(MAX_uint32-11);
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::EnchantResult;M.Payload.Event->Event.EnchantResult=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("EnchantResult decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("EnchantResult dispatch"),Decoded.Event.IsSet() && Decoded.Event->EnchantResult.IsSet()))return false;
TestEqual(TEXT("EnchantResult all fields presence False"),Decoded.Event->EnchantResult->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1EnchantResult Q;
Q._accepted_client_seq._accepted_client_seqCase=EGrpcNightfallV1EnchantResult_accepted_client_seq::AcceptedClientSeq;Q._accepted_client_seq.AcceptedClientSeq=MAX_uint64;
Q.RequestKey.Value.Init(0xab,16);Q.RequestFingerprint=TEXT("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");Q.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_ENCHANT;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.ReceiptId=TEXT("EnchantResult-receipt_id");
Q.TargetInstanceId=uint64(MAX_uint64-6);
Q.OldEnchant=uint32(MAX_uint32-7);
Q.NewEnchant=uint32(MAX_uint32-8);
Q.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS_NO_INCREMENT;
{
auto& Nested=Q.Refunds.AddDefaulted_GetRef();
Nested.TemplateId=uint32(MAX_uint32-1);
Nested.Count=uint64(MAX_uint64-2);
Nested.InstanceId=uint64(MAX_uint64-3);
}
Q.AnnouncementTier=uint32(MAX_uint32-11);
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::EnchantResult;M.Payload.Event->Event.EnchantResult=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("EnchantResult decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("EnchantResult dispatch"),Decoded.Event.IsSet() && Decoded.Event->EnchantResult.IsSet()))return false;
TestEqual(TEXT("EnchantResult all fields presence True"),Decoded.Event->EnchantResult->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1RefineResult Q;
Q.RequestKey.Value.Init(0xab,16);Q.RequestFingerprint=TEXT("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");Q.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_CANCEL_REFINE;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.ReceiptId=TEXT("RefineResult-receipt_id");
Q.TargetInstanceId=uint64(MAX_uint64-6);
Q.Outcome=EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_REMOVED;
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::RefineResult;M.Payload.Event->Event.RefineResult=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("RefineResult decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("RefineResult dispatch"),Decoded.Event.IsSet() && Decoded.Event->RefineResult.IsSet()))return false;
TestEqual(TEXT("RefineResult all fields presence False"),Decoded.Event->RefineResult->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1RefineResult Q;
Q._accepted_client_seq._accepted_client_seqCase=EGrpcNightfallV1RefineResult_accepted_client_seq::AcceptedClientSeq;Q._accepted_client_seq.AcceptedClientSeq=MAX_uint64;
Q.RequestKey.Value.Init(0xab,16);Q.RequestFingerprint=TEXT("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");Q.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_CANCEL_REFINE;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.ReceiptId=TEXT("RefineResult-receipt_id");
Q.TargetInstanceId=uint64(MAX_uint64-6);
Q._augment_id._augment_idCase=EGrpcNightfallV1RefineResult_augment_id::AugmentId;
Q._augment_id.AugmentId=uint32(MAX_uint32-7);
Q.Outcome=EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_REMOVED;
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::RefineResult;M.Payload.Event->Event.RefineResult=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("RefineResult decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("RefineResult dispatch"),Decoded.Event.IsSet() && Decoded.Event->RefineResult.IsSet()))return false;
TestEqual(TEXT("RefineResult all fields presence True"),Decoded.Event->RefineResult->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1ItemOperationResult Q;
Q.RequestKey.Value.Init(0xab,16);Q.RequestFingerprint=TEXT("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");Q.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_DESTROY;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.ReceiptId=TEXT("ItemOperationResult-receipt_id");
Q.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_BACKFILL;
Q.AffectedInstanceIds.Add(uint64(MAX_uint64-7));
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::ItemOperationResult;M.Payload.Event->Event.ItemOperationResult=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("ItemOperationResult decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("ItemOperationResult dispatch"),Decoded.Event.IsSet() && Decoded.Event->ItemOperationResult.IsSet()))return false;
TestEqual(TEXT("ItemOperationResult all fields presence False"),Decoded.Event->ItemOperationResult->ToJsonString(false),Q.ToJsonString(false));}
 {FGrpcNightfallV1ItemOperationResult Q;
Q._accepted_client_seq._accepted_client_seqCase=EGrpcNightfallV1ItemOperationResult_accepted_client_seq::AcceptedClientSeq;Q._accepted_client_seq.AcceptedClientSeq=MAX_uint64;
Q.RequestKey.Value.Init(0xab,16);Q.RequestFingerprint=TEXT("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");Q.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_DESTROY;
Q.EntityId.Value.Init(18,16);
Q.Incarnation=uint32(MAX_uint32-2);
Q.Tick=uint64(MAX_uint64-3);
Q.InventoryRevision=uint64(MAX_uint64-4);
Q.ReceiptId=TEXT("ItemOperationResult-receipt_id");
Q.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_BACKFILL;
Q.AffectedInstanceIds.Add(uint64(MAX_uint64-7));
Q._recipe_id._recipe_idCase=EGrpcNightfallV1ItemOperationResult_recipe_id::RecipeId;
Q._recipe_id.RecipeId=uint64(MAX_uint64-8);
FGrpcNightfallV1ServerMessage M;M.Payload.PayloadCase=EGrpcNightfallV1ServerMessagePayload::Event;M.Payload.Event=MakeShared<FGrpcNightfallV1WorldEvent>();
M.Payload.Event->Event.EventCase=EGrpcNightfallV1WorldEventEvent::ItemOperationResult;M.Payload.Event->Event.ItemOperationResult=Q;TArray<uint8> B;NightfallWire::EncodeServerMessage(M,B);FServerMessage Decoded;
TestTrue(TEXT("ItemOperationResult decode"),NightfallProto::Decode(B.GetData(),B.Num(),Decoded));
if(!TestTrue(TEXT("ItemOperationResult dispatch"),Decoded.Event.IsSet() && Decoded.Event->ItemOperationResult.IsSet()))return false;
TestEqual(TEXT("ItemOperationResult all fields presence True"),Decoded.Event->ItemOperationResult->ToJsonString(false),Q.ToJsonString(false));}
 return true;
}
#endif
