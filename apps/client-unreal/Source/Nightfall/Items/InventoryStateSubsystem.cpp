#include "InventoryStateSubsystem.h"
#include "NightfallWire.h"
#include "Net/NetClientSubsystem.h"
#include "Combat/CombatStateSubsystem.h"
#include "Bot/ItemBotPredicates.h"
#include "Bot/SkillBotPredicates.h"
#include "Engine/GameInstance.h"
#include "Misc/ConfigCacheIni.h"
#include "Misc/Paths.h"
#include "HAL/FileManager.h"
namespace NightfallItems
{
 FBytes UuidBytes(const FString& Id)
 {
  FGuid G; FBytes B; if (!FGuid::ParseExact(Id, EGuidFormats::DigitsWithHyphens, G) || !G.IsValid()) return B;
  const FString Hex = G.ToString(EGuidFormats::Digits);
  for (int32 I=0; I<32; I+=2) B.Value.Add(uint8(FParse::HexDigit(Hex[I])*16+FParse::HexDigit(Hex[I+1])));
  return B;
 }
 FString UuidString(const FBytes& B)
 {
  if (B.Value.Num()!=16) return {};
  FString Hex; for (uint8 V:B.Value) Hex+=FString::Printf(TEXT("%02x"),V);
  FGuid G; return FGuid::ParseExact(Hex,EGuidFormats::Digits,G) && G.IsValid()?G.ToString(EGuidFormats::DigitsWithHyphensLower):FString();
 }
 const TCHAR* Slots[] = {TEXT("unspecified"),TEXT("under"),TEXT("head"),TEXT("hair"),TEXT("hair2"),TEXT("neck"),TEXT("rhand"),TEXT("chest"),TEXT("lhand"),TEXT("rear"),TEXT("lear"),TEXT("gloves"),TEXT("legs"),TEXT("feet"),TEXT("rfinger"),TEXT("lfinger"),TEXT("lbracelet"),TEXT("rbracelet"),TEXT("deco1"),TEXT("deco2"),TEXT("deco3"),TEXT("deco4"),TEXT("deco5"),TEXT("deco6"),TEXT("cloak"),TEXT("belt")};
 EGrpcNightfallV1EquipSlot Slot(const FString& Name) { for (uint8 I=1;I<UE_ARRAY_COUNT(Slots);++I) if (Name==Slots[I]) return EGrpcNightfallV1EquipSlot(I); return EGrpcNightfallV1EquipSlot::EQUIP_SLOT_UNSPECIFIED; }
 FString SlotName(EGrpcNightfallV1EquipSlot S) { return uint8(S)<UE_ARRAY_COUNT(Slots)?Slots[uint8(S)]:TEXT("unknown"); }
}
void UInventoryStateSubsystem::Initialize(FSubsystemCollectionBase& C)
{
 Super::Initialize(C); Net=C.InitializeDependency<UNetClientSubsystem>(); Session=C.InitializeDependency<USessionClient>();
 Session->OnAuthenticationChanged.AddUObject(this,&UInventoryStateSubsystem::AuthenticationChanged);
 Net->OnEntitySpawn.AddUObject(this,&UInventoryStateSubsystem::ApplySpawn);
 Net->OnEntityDespawn.AddUObject(this,&UInventoryStateSubsystem::ApplyDespawn);
 Net->OnEntityRespawned.AddUObject(this,&UInventoryStateSubsystem::ApplyRespawn);
 Net->OnInventorySync.AddUObject(this,&UInventoryStateSubsystem::ApplySync);
 Net->OnInventoryChanged.AddUObject(this,&UInventoryStateSubsystem::ApplyChanged);
 Net->OnEquipmentChanged.AddUObject(this,&UInventoryStateSubsystem::ApplyEquipment);
 Net->OnWeightChanged.AddUObject(this,&UInventoryStateSubsystem::ApplyWeight);
 Net->OnPenaltyChanged.AddUObject(this,&UInventoryStateSubsystem::ApplyPenalty);
 Net->OnEnchantResult.AddUObject(this,&UInventoryStateSubsystem::ApplyEnchant);
 Net->OnRefineResult.AddUObject(this,&UInventoryStateSubsystem::ApplyRefine);
 Net->OnItemOperationResult.AddUObject(this,&UInventoryStateSubsystem::ApplyOperation);
 Net->OnStatsChanged.AddUObject(this,&UInventoryStateSubsystem::ApplyStats);
 Net->OnItemReceiptPayload.AddUObject(this,&UInventoryStateSubsystem::ApplyReceiptPayload);
 Net->OnIntentRejected.AddUObject(this,&UInventoryStateSubsystem::ApplyRejected);
 Net->OnConnected.AddDynamic(this,&UInventoryStateSubsystem::Reset);
 Net->OnDisconnected.AddDynamic(this,&UInventoryStateSubsystem::Disconnected);
}
void UInventoryStateSubsystem::Deinitialize()
{
 if(Session)Session->OnAuthenticationChanged.RemoveAll(this);
 if (Net) {
  Net->OnEntitySpawn.RemoveAll(this); Net->OnEntityDespawn.RemoveAll(this); Net->OnEntityRespawned.RemoveAll(this);
  Net->OnInventorySync.RemoveAll(this); Net->OnInventoryChanged.RemoveAll(this); Net->OnEquipmentChanged.RemoveAll(this);
  Net->OnWeightChanged.RemoveAll(this); Net->OnPenaltyChanged.RemoveAll(this); Net->OnEnchantResult.RemoveAll(this);
  Net->OnRefineResult.RemoveAll(this); Net->OnItemOperationResult.RemoveAll(this); Net->OnStatsChanged.RemoveAll(this);
  Net->OnItemReceiptPayload.RemoveAll(this); Net->OnIntentRejected.RemoveAll(this); Net->OnConnected.RemoveAll(this); Net->OnDisconnected.RemoveAll(this);
 } ++RpcSerial; ++CatalogSerial; Super::Deinitialize();
}
void UInventoryStateSubsystem::ClearOwner()
{
 CurrentStats.Reset();
 // Keep comparison references across re-admission, scoped to the selected character.
 if(!Net || MarkOwner!=Net->GetOwnEntityId().ToLower()){MarkedStats.Reset();MarkRevision.Reset();MarkOwner.Reset();}
 ++RpcSerial; Inventory={}; Items.Reset(); PendingWeights.Reset(); PendingPenalties.Reset(); Enchant.Reset(); Refine.Reset(); Operation.Reset();
 ReceiptMatches.Reset(); DetachedViews.Reset(); OwnerTick=WeightTick=PenaltyTick=SyncTick=0; bBaseline=bResync=bRpcBusy=bRpcDone=false; StatusText.Reset();
}
void UInventoryStateSubsystem::AuthenticationChanged() { MarkedStats.Reset();MarkRevision.Reset();MarkOwner.Reset();Reset(); }
void UInventoryStateSubsystem::Reset() { Outstanding.Reset();SequenceRequests.Reset();ClearOwner(); ++CatalogSerial; bCatalog=false; Fence.Reset(); Equipment.Reset(); EquipmentTicks.Reset(); OnChanged.Broadcast(); }
void UInventoryStateSubsystem::ApplySpawn(const FEntitySpawn& S)
{
 if (!Fence.Spawn(S)) return;
 if (Net && Net->IsOwnEntity(S.EntityId)) ClearOwner();
 TSet<uint8> Slots;bool Valid=true;for(const auto& E:S.VisibleEquipment){if(!uint8(E.Slot) || uint8(E.Slot)>25 || !E.TemplateId.Value || E.GlowTier.Value>2 || Slots.Contains(uint8(E.Slot))){Valid=false;break;}Slots.Add(uint8(E.Slot));}
 Equipment.Add(S.EntityId.ToLower(),Valid?S.VisibleEquipment:TArray<FGrpcNightfallV1VisibleEquipment>()); EquipmentTicks.Add(S.EntityId.ToLower(),S.StateTick); OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyDespawn(const FEntityDespawn& S)
{
 Fence.Despawn(S.EntityId); Equipment.Remove(S.EntityId.ToLower()); EquipmentTicks.Remove(S.EntityId.ToLower());
 if (Net && Net->IsOwnEntity(S.EntityId)) ClearOwner(); OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyRespawn(const FEntityRespawned& S)
{
 if (!Fence.Respawn(S)) return;
 // Inventory is retained across death but requires new-life hydration before accepting deltas.
 if (Net && Net->IsOwnEntity(S.Entity)) ClearOwner(); OnChanged.Broadcast();
}
bool UInventoryStateSubsystem::Owner(const FBytes& B) const { const FString Id=NightfallItems::UuidString(B); return !Id.IsEmpty() && Net && Net->IsOwnEntity(Id); }
void UInventoryStateSubsystem::Gap()
{
 if (bResync) return; bResync=true; StatusText=TEXT("Inventory needs a fresh server baseline"); OnChanged.Broadcast();
 if (Net) Net->RequestInventoryBaseline();
}
bool UInventoryStateSubsystem::Accept(const FBytes& Id,uint32 Inc,uint64 Tick,uint64 Rev,bool bReceipt)
{
 if (!Owner(Id) || !HasBaseline() || !Fence.Visible(NightfallItems::UuidString(Id),Inc)) return false;
 // Frozen receipts can predate this baseline; they never advance world state.
 if (bReceipt) return true;
 if (!Fence.Accept(NightfallItems::UuidString(Id),Inc,Tick) || Tick<OwnerTick || Rev<Revision()) return false;
 if (Rev>Revision() && (Revision()==MAX_uint64 || Rev!=Revision()+1)) { Gap(); return false; }
 return true;
}
bool UInventoryStateSubsystem::Validate(const TMap<uint64,FGrpcNightfallV1OwnedItem>& Candidate) const
{
 TSet<uint8> Slots;
 for (const auto& P:Candidate) {
  const auto& I=P.Value;
  if (!P.Key || P.Key>MAX_int64 || I.InstanceId.Value!=P.Key || !I.TemplateId.Value || !I.Count.Value || I.Count.Value>MAX_int64) return false;
  if (I.Location==EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL) {
   if (!uint8(I.Slot) || uint8(I.Slot)>25 || Slots.Contains(uint8(I.Slot))) return false; Slots.Add(uint8(I.Slot));
  } else if (I.Location!=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_INVENTORY || uint8(I.Slot)!=0) return false;
 }
 return true;
}
void UInventoryStateSubsystem::Publish(const TMap<uint64,FGrpcNightfallV1OwnedItem>& Candidate)
{
 Items=Candidate; Inventory.Items.Reset(); Inventory.Paperdoll.Reset(); TArray<uint64> Ids; Items.GetKeys(Ids); Ids.Sort();
 for (uint64 Id:Ids) {
  const auto& I=Items[Id]; Inventory.Items.Add(I);
  if (uint8(I.Slot)) { FGrpcNightfallV1PaperdollEntry E; E.Slot=I.Slot; E.InstanceId=Id; Inventory.Paperdoll.Add(E); }
 }
}
void UInventoryStateSubsystem::ApplySync(const FGrpcNightfallV1InventorySync& S)
{
 const FString Id=NightfallItems::UuidString(S.EntityId);
 if (!Owner(S.EntityId) || !S.Baseline || !S.HasWeight || !S.HasPenalties || S.RulesHash.Value.Num()!=32 || !Fence.Accept(Id,S.Incarnation,S.Tick,true,S.BaselineTick)) return;
 if (bBaseline && (S.InventoryRevision.Value<Revision() || S.Tick.Value<OwnerTick || S.Tick.Value<=SyncTick)) return;
 TMap<uint64,FGrpcNightfallV1OwnedItem> Candidate;
 for (const auto& I:S.Items) { if (Candidate.Contains(I.InstanceId.Value)) { Gap(); return; } Candidate.Add(I.InstanceId,I); }
 if (!Validate(Candidate)) { Gap(); return; }
 TSet<uint8> Slots;
 for (const auto& P:S.Paperdoll) { const auto* I=Candidate.Find(P.InstanceId); if (!I || !uint8(P.Slot) || I->Slot!=P.Slot || Slots.Contains(uint8(P.Slot))) { Gap(); return; } Slots.Add(uint8(P.Slot)); }
 for (const auto& P:Candidate) if (uint8(P.Value.Slot) && !Slots.Contains(uint8(P.Value.Slot))) { Gap(); return; }
 if (bCatalog && Catalog.RulesHash.Value!=S.RulesHash.Value) bCatalog=false;
 Inventory=S; Publish(Candidate); OwnerTick=WeightTick=PenaltyTick=SyncTick=S.Tick; bBaseline=true; bResync=false; PendingWeights.Reset(); PendingPenalties.Reset(); OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyChanged(const FGrpcNightfallV1InventoryChanged& S)
{
 if (!Accept(S.EntityId,S.Incarnation,S.Tick,S.InventoryRevision)) return;
 if (S.InventoryRevision.Value==Revision()) return; // duplicate delta, siblings still accepted separately
 if (S.PreviousRevision.Value!=Revision()) { Gap(); return; }
 auto Candidate=Items; TSet<uint64> Touched;
 for (const auto& Id:S.RemovedIds) { if (!Candidate.Contains(Id) || Touched.Contains(Id)) { Gap(); return; } Candidate.Remove(Id); Touched.Add(Id); }
 for (const auto& I:S.Added) { if (Candidate.Contains(I.InstanceId) || Touched.Contains(I.InstanceId)) { Gap(); return; } Candidate.Add(I.InstanceId,I); Touched.Add(I.InstanceId); }
 for (const auto& I:S.Updated) { const auto* Old=Candidate.Find(I.InstanceId); if (!Old || Old->TemplateId.Value!=I.TemplateId.Value || Touched.Contains(I.InstanceId)) { Gap(); return; } Candidate.Add(I.InstanceId,I); Touched.Add(I.InstanceId); }
 if (!Validate(Candidate)) { Gap(); return; }
 Publish(Candidate); Inventory.InventoryRevision=S.InventoryRevision; Inventory.Tick=S.Tick; OwnerTick=S.Tick;
 Inventory.AutoShotTemplates=S.AutoShotTemplates; Inventory.Reuse=S.Reuse;
 if (auto* W=PendingWeights.Find(Revision())) Inventory.Weight=*W;
 if (auto* P=PendingPenalties.Find(Revision())) Inventory.Penalties=*P;
 PendingWeights.Reset(); PendingPenalties.Reset(); OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyWeight(const FGrpcNightfallV1WeightChanged& S)
{
 if (!S.HasWeight || S.Tick.Value<WeightTick || !Accept(S.EntityId,S.Incarnation,S.Tick,S.InventoryRevision)) return;
 WeightTick=S.Tick;
 if (S.InventoryRevision.Value==Revision()) Inventory.Weight=S.Weight; else PendingWeights.Add(S.InventoryRevision,S.Weight);
 OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyPenalty(const FGrpcNightfallV1PenaltyChanged& S)
{
 if (!S.HasPenalties || S.Tick.Value<PenaltyTick || !Accept(S.EntityId,S.Incarnation,S.Tick,S.InventoryRevision)) return;
 PenaltyTick=S.Tick;
 if (S.InventoryRevision.Value==Revision()) Inventory.Penalties=S.Penalties; else PendingPenalties.Add(S.InventoryRevision,S.Penalties);
 OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyStats(const FStatsChanged& S)
{
 if(!Owner(NightfallItems::UuidBytes(S.Entity)) || !Fence.Accept(S.Entity,S.Incarnation,S.Tick,S.bBaseline,S.BaselineTick))return;
 if(CurrentStats.IsSet() && (S.Tick<CurrentStats->Tick || S.InventoryRevision<CurrentStats->InventoryRevision))return;
 if(!S.bBaseline && !Accept(NightfallItems::UuidBytes(S.Entity),S.Incarnation,S.Tick,S.InventoryRevision))return;
 if(HasBaseline() && S.InventoryRevision<Revision())return;
 CurrentStats=S;
 FGrpcNightfallV1WeightChanged W; W.EntityId=NightfallItems::UuidBytes(S.Entity); W.Incarnation=S.Incarnation; W.Tick=S.Tick; W.InventoryRevision=S.InventoryRevision;
 if (S.ItemWeight.IsSet()) { W.HasWeight=true; W.Weight=*S.ItemWeight; ApplyWeight(W); }
 if (S.ItemPenalties.IsSet()) { FGrpcNightfallV1PenaltyChanged P; P.EntityId=W.EntityId; P.Incarnation=W.Incarnation; P.Tick=W.Tick; P.InventoryRevision=W.InventoryRevision; P.HasPenalties=true; P.Penalties=*S.ItemPenalties; ApplyPenalty(P); }
 OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyEquipment(const FGrpcNightfallV1EquipmentChanged& S)
{
 const FString Id=NightfallItems::UuidString(S.EntityId);
 if (!Fence.Accept(Id,S.Incarnation,S.Tick) || S.Tick.Value<EquipmentTicks.FindRef(Id)) return;
 for (const auto& E:S.Visible) if (!uint8(E.Slot) || uint8(E.Slot)>25 || E.GlowTier.Value>2) return;
 auto& Rows=Equipment.FindOrAdd(Id);
 for (const auto& E:S.Visible) { Rows.RemoveAll([&](const auto& R){return R.Slot==E.Slot;}); if (E.TemplateId.Value) Rows.Add(E); }
 EquipmentTicks.Add(Id,S.Tick); OnChanged.Broadcast();
}
const FGrpcNightfallV1OwnedItem* UInventoryStateSubsystem::Equipped(EGrpcNightfallV1EquipSlot S) const { if (HasBaseline()) for (const auto& P:Items) if (P.Value.Slot==S && uint8(S)) return &P.Value; return nullptr; }
TOptional<uint64> UInventoryStateSubsystem::Count(uint32 T) const { if (!HasBaseline()) return {}; uint64 N=0; for (const auto& P:Items) if (P.Value.TemplateId.Value==T) { if (P.Value.Count.Value>MAX_uint64-N) return {}; N+=P.Value.Count.Value; } return N; }
TOptional<uint64> UInventoryStateSubsystem::Resolve(const FString& S) const
{
 if (!HasBaseline()) return {}; uint64 N=0;
 if (S.StartsWith(TEXT("instance:"))) { if (!SkillBotPredicates::ParseUnsigned(S.Mid(9),N) || !Items.Contains(N)) return {}; return N; }
 if (!SkillBotPredicates::ParseUnsigned(S.StartsWith(TEXT("template:"))?S.Mid(9):S,N) || !N || N>MAX_uint32) return {};
 TOptional<uint64> Id; for (const auto& P:Items) if (P.Value.TemplateId.Value==N) { if (!Id.IsSet() || P.Key<*Id) Id=P.Key; } return Id;
}
bool UInventoryStateSubsystem::AutoShot(uint32 T) const { return HasBaseline() && Inventory.AutoShotTemplates.ContainsByPredicate([T](const auto& V){return V.Value==T;}); }
FString UInventoryStateSubsystem::RequestScope(EGrpcNightfallV1ItemOperation Op,const FString& Key) const
{
 const FString Normalized=NightfallItems::UuidString(NightfallItems::UuidBytes(Key));
 if(!Session || Session->AccountId().IsEmpty() || Normalized.IsEmpty() || !uint8(Op))return {};
 return Session->AccountId()+FString::Printf(TEXT(":%u:%s"),uint8(Op),*Normalized);
}
void UInventoryStateSubsystem::Remember(EGrpcNightfallV1ItemOperation Op,const FString& Key,const TArray<uint8>& Payload)
{
 const FString Scope=RequestScope(Op,Key);if(Scope.IsEmpty())return;
 if(const auto* First=Receipts.Find(Scope))ReceiptMatches=(*First==Payload);
 else {Receipts.Add(Scope,Payload);ReceiptMatches.Reset();}
 if(const auto* Sequences=Outstanding.Find(Scope))for(uint32 Seq:*Sequences)SequenceRequests.Remove(Seq);
 Outstanding.Remove(Scope);
 StatusText=IsBusy()?TEXT("Waiting for server item result"):TEXT("Server committed item operation");OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyOperation(const FGrpcNightfallV1ItemOperationResult& S) { if (uint8(S.Operation)>0 && uint8(S.Operation)<=11 && S.Operation!=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_REFINE && S.Operation!=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_CANCEL_REFINE && Accept(S.EntityId,S.Incarnation,S.Tick,S.InventoryRevision)) {Operation=S.Operation; OnChanged.Broadcast();} }
void UInventoryStateSubsystem::ApplyEnchant(const FGrpcNightfallV1EnchantResult& S)
{
 if (!Accept(S.EntityId,S.Incarnation,S.Tick,S.InventoryRevision) || !uint8(S.Outcome)) return;
 Enchant=S; OnChanged.Broadcast();
}
void UInventoryStateSubsystem::ApplyRefine(const FGrpcNightfallV1RefineResult& S) { if (Accept(S.EntityId,S.Incarnation,S.Tick,S.InventoryRevision) && (S.Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED || S.Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_REMOVED)) { Refine=S;OnChanged.Broadcast(); } }
bool UInventoryStateSubsystem::EnchantConsistent() const
{
 if (!HasBaseline() || !Enchant.IsSet() || Enchant->InventoryRevision.Value!=Revision()) return false;
 const auto& E=*Enchant; const auto* I=Find(E.TargetInstanceId);
 if (E.Outcome==EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_DESTROYED) {
  if (I) return false; for (const auto& R:E.Refunds) { const auto* Refund=Find(R.InstanceId); if (!Refund || Refund->TemplateId.Value!=R.TemplateId.Value || Refund->Count.Value<R.Count.Value) return false; } return true;
 }
 bool Transition=false;
 switch(E.Outcome){case EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS:Transition=E.OldEnchant.Value<MAX_uint32 && E.NewEnchant.Value==E.OldEnchant.Value+1;break;case EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_BLESSED_RESET:Transition=E.NewEnchant.Value==0;break;case EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SAFE_FAILED:case EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS_NO_INCREMENT:Transition=E.NewEnchant.Value==E.OldEnchant.Value;break;default:break;}
 return Transition && I && I->Enchant.Value==E.NewEnchant.Value && (E.Outcome==EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS || E.Outcome==EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS_NO_INCREMENT || E.Outcome==EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SAFE_FAILED || E.Outcome==EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_BLESSED_RESET);
}
bool UInventoryStateSubsystem::StatsSynchronized() const
{
 return HasBaseline() && CurrentStats.IsSet() && CurrentStats->EffectiveStats.IsSet()
  && Net && Net->IsOwnEntity(CurrentStats->Entity) && Fence.Visible(CurrentStats->Entity,CurrentStats->Incarnation)
  && CurrentStats->InventoryRevision==Revision();
}
bool UInventoryStateSubsystem::MarkItemState()
{
 if(!StatsSynchronized())return false;
 MarkOwner=Net->GetOwnEntityId().ToLower();MarkRevision=Revision();MarkedStats=CurrentStats;return true;
}
TOptional<uint64> UInventoryStateSubsystem::RevisionSinceMark() const
{
 if(!HasBaseline() || !MarkRevision.IsSet() || !Net || MarkOwner!=Net->GetOwnEntityId().ToLower() || Revision()<*MarkRevision)return {};
 return Revision()-*MarkRevision;
}
TOptional<FString> UInventoryStateSubsystem::OwnerStatDelta(const FString& Name) const
{
 if(!StatsSynchronized() || !RevisionSinceMark().IsSet() || !MarkedStats.IsSet())return {};
 const __int128 Current=Name==TEXT("max_hp")?CurrentStats->MaxHp:CurrentStats->EffectiveStats->PDefQ;
 const __int128 Saved=Name==TEXT("max_hp")?MarkedStats->MaxHp:MarkedStats->EffectiveStats->PDefQ;
 const __int128 Delta=Current-Saved;
 return FString::Printf(TEXT("%s%llu"),Delta<0?TEXT("-"):TEXT(""),uint64(Delta<0?-Delta:Delta));
}
TOptional<bool> UInventoryStateSubsystem::OwnerStatsMatchMark() const
{
 if(!StatsSynchronized() || !RevisionSinceMark().IsSet() || !MarkedStats.IsSet())return {};
 const auto& A=*CurrentStats->EffectiveStats;const auto& B=*MarkedStats->EffectiveStats;
 return CurrentStats->MaxHp==MarkedStats->MaxHp && CurrentStats->MaxMp==MarkedStats->MaxMp && CurrentStats->MaxCp==MarkedStats->MaxCp
  && A.PAtkQ==B.PAtkQ && A.PDefQ==B.PDefQ && A.MAtkQ==B.MAtkQ && A.MDefQ==B.MDefQ
  && A.AttackSpeed==B.AttackSpeed && A.CastSpeed==B.CastSpeed && A.SpeedQ==B.SpeedQ
  && A.AccuracyQ==B.AccuracyQ && A.EvasionQ==B.EvasionQ && A.CritPermille==B.CritPermille
  && A.ShieldDefQ==B.ShieldDefQ && A.ShieldRateQ==B.ShieldRateQ && A.ShieldAngleDegrees==B.ShieldAngleDegrees
  && A.PhysicalReachMillitiles==B.PhysicalReachMillitiles && A.PhysicalAngleDegrees==B.PhysicalAngleDegrees;
}
TOptional<bool> UInventoryStateSubsystem::RefineConsistent() const
{
 if(!HasBaseline() || !Refine.IsSet() || Refine->InventoryRevision.Value>Revision())return {};
 const auto* I=Find(Refine->TargetInstanceId);
 if(!I)return false;
 const bool Present=I->_augment_id._augment_idCase==EGrpcNightfallV1OwnedItem_augment_id::AugmentId;
 if(Refine->Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED)
  return Present && Refine->_augment_id._augment_idCase==EGrpcNightfallV1RefineResult_augment_id::AugmentId
   && I->_augment_id.AugmentId.Value==Refine->_augment_id.AugmentId.Value;
 if(Refine->Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_REMOVED)
  return !Present && Refine->_augment_id._augment_idCase==EGrpcNightfallV1RefineResult_augment_id::NotSet;
 return {};
}

void UInventoryStateSubsystem::ApplyRejected(const FIntentRejected& R)
{
 const auto* Found=SequenceRequests.Find(R.Seq);if(!Found)return;const FString Scope=*Found;SequenceRequests.Remove(R.Seq);
 if(auto* Sequences=Outstanding.Find(Scope)){Sequences->Remove(R.Seq);if(Sequences->IsEmpty())Outstanding.Remove(Scope);}
 StatusText=R.Detail;OnChanged.Broadcast();
}
void UInventoryStateSubsystem::FinishSend(uint32 Seq,const FString& Key,EGrpcNightfallV1ItemOperation Op)
{
 LastSendSeq=Seq;const FString Scope=RequestScope(Op,Key);
 if(Seq && !Scope.IsEmpty()){Outstanding.FindOrAdd(Scope).Add(Seq);SequenceRequests.Add(Seq,Scope);}
 StatusText=Seq?TEXT("Waiting for server item result"):TEXT("World channel unavailable");OnChanged.Broadcast();
}
FString UInventoryStateSubsystem::Blacksmith() const
{
 FString Id; for (const auto& P:Fence.Identities) if (P.Value.Kind==2 && !P.Value.bCombatant && P.Value.Name==TEXT("Blacksmith")) { if (!Id.IsEmpty()) return {}; Id=P.Key; } return Id;
}
TOptional<FGrpcNightfallV1ItemTarget> UInventoryStateSubsystem::Target(const FString& Selector,bool bNpc) const
{
 if (!Net) return {}; FString Id=Selector;
 if (Selector==TEXT("self")) Id=Net->GetOwnEntityId();
 else if (Selector==TEXT("target")) { auto* C=GetGameInstance()->GetSubsystem<UCombatStateSubsystem>(); Id=C?C->GetOwn().TargetId:FString(); }
 else if (Selector.IsEmpty()) { if (!bNpc) return {}; Id=Blacksmith(); }
 else if (Selector.StartsWith(TEXT("entity:"))) Id=Selector.Mid(7);
 const auto* I=Fence.Identities.Find(Id.ToLower()); if (!I || !I->LifeIncarnation || (bNpc && (I->Kind!=2 || I->bCombatant))) return {};
 FGrpcNightfallV1ItemTarget T; T.EntityId=NightfallItems::UuidBytes(Id); T.Incarnation=I->LifeIncarnation; if (T.EntityId.Value.Num()!=16) return {}; return T;
}
void UInventoryStateSubsystem::ApplyCatalog(const FGrpcNightfallV1GetItemCatalogResponse& R)
{
 bCatalog=false;
 if (R.DataVersion.Value!=2 || R.RulesHash.Value.Num()!=32 || (HasBaseline() && R.RulesHash.Value!=Inventory.RulesHash.Value)) { StatusText=TEXT("Item catalogue version/hash mismatch"); OnChanged.Broadcast(); return; }
 TSet<uint32> Ids; for (const auto& I:R.Items) { if (!I.TemplateId.Value || Ids.Contains(I.TemplateId) || !uint8(I.Kind) || !uint8(I.Grade)) return; Ids.Add(I.TemplateId); }
 Catalog=R; bCatalog=true; OnChanged.Broadcast();
}
const FGrpcNightfallV1ItemDefinition* UInventoryStateSubsystem::Definition(uint32 Id) const { if (bCatalog) for (const auto& I:Catalog.Items) if (I.TemplateId.Value==Id) return &I; return nullptr; }
void UInventoryStateSubsystem::LoadCatalog()
{
 if (!Session && !CatalogProvider) return; bCatalog=false; bRpcDone=false; const uint64 Serial=++CatalogSerial;
 auto Callback=[Weak=TWeakObjectPtr<UInventoryStateSubsystem>(this),Serial](const FNetResult& R,const FGrpcNightfallV1GetItemCatalogResponse& C){if (!Weak.IsValid() || Serial!=Weak->CatalogSerial) return; Weak->LastRpc=R; Weak->bRpcDone=true; if (R.IsOk()) Weak->ApplyCatalog(C); else Weak->StatusText=R.Message; Weak->OnChanged.Broadcast();};
 if (CatalogProvider) CatalogProvider(Callback); else Session->GetItemCatalog({},Callback);
}
const FGrpcNightfallV1InventorySync* UInventoryStateSubsystem::Detached(const FString& Character) const
{
 const auto* View=DetachedViews.Find(Character.ToLower());
 return View && Session && View->Generation==Session->AuthenticationGeneration() && View->Inventory.IsSet()?&*View->Inventory:nullptr;
}
void UInventoryStateSubsystem::ApplyDetached(const FString& Character,const FGrpcNightfallV1GetInventoryResponse& R)
{
 auto* View=DetachedViews.Find(Character.ToLower());if(!View || !Session || View->Generation!=Session->AuthenticationGeneration())return;
 View->Inventory.Reset();
 if(R.HasInventory && NightfallItems::UuidString(R.Inventory.EntityId)==Character.ToLower() && !R.Inventory.Tick.Value && !R.Inventory.BaselineTick.Value && !R.Inventory.Incarnation.Value)View->Inventory=R.Inventory;
 OnChanged.Broadcast();
}
void UInventoryStateSubsystem::LoadDetached(const FString& Character)
{
 if(!Session)return;const FString Id=NightfallItems::UuidString(NightfallItems::UuidBytes(Character));if(Id.IsEmpty())return;
 FGrpcNightfallV1GetInventoryRequest Q;Q.CharacterId=NightfallItems::UuidBytes(Id);
 auto& View=DetachedViews.FindOrAdd(Id);View.Inventory.Reset();View.Generation=Session->AuthenticationGeneration();View.Serial=++RpcSerial;
 const uint64 Serial=View.Serial,Generation=View.Generation;bRpcDone=false;StatusText=TEXT("Loading inventory");OnChanged.Broadcast();
 auto Callback=[Weak=TWeakObjectPtr<UInventoryStateSubsystem>(this),Id,Serial,Generation](const FNetResult& R,const FGrpcNightfallV1GetInventoryResponse& V){
  if(!Weak.IsValid() || !Weak->Session || Generation!=Weak->Session->AuthenticationGeneration())return;
  auto* Held=Weak->DetachedViews.Find(Id);if(!Held || Held->Serial!=Serial)return;
  Held->Inventory.Reset();Weak->bRpcDone=true;Weak->LastRpc=R;Weak->StatusText=R.Message;
  if(R.IsOk())Weak->ApplyDetached(Id,V);Weak->OnChanged.Broadcast();};
 if(InventoryProvider)InventoryProvider(Q,MoveTemp(Callback));else Session->GetInventory(Q,MoveTemp(Callback));
}
void UInventoryStateSubsystem::Grant(uint32 Template,uint64 Count,const FString& Key)
{
 // AUTH_DEV_TOKENS is enforced by the server; production/real-token refusals remain observable.
 if (!Session || !Net || bRpcBusy) return;
 FGrpcNightfallV1GrantItemRequest Q; Q.CharacterId=NightfallItems::UuidBytes(Net->GetOwnEntityId()); Q.TemplateId=Template; Q.Count=Count; Q.IdempotencyKey=Key;
 const uint64 Serial=++RpcSerial; bRpcBusy=true; bRpcDone=false;
 Session->GrantItem(Q,[Weak=TWeakObjectPtr<UInventoryStateSubsystem>(this),Serial](const FNetResult& R,const FGrpcNightfallV1GrantItemResponse&){if (!Weak.IsValid() || Serial!=Weak->RpcSerial) return; Weak->bRpcBusy=false; Weak->bRpcDone=true; Weak->LastRpc=R; Weak->StatusText=R.Message; Weak->OnChanged.Broadcast();});
}
bool UInventoryStateSubsystem::Execute(const TArray<FString>& Tokens)
{
 FString Error; if(Tokens.IsEmpty() || !Net || !ItemBotPredicates::ValidateCommand(Tokens,Error))return false;
 const FString Name=Tokens[0].ToLower();TArray<FString>A=Tokens;A.RemoveAt(0);
 if(Name==TEXT("nf.itemcatalog")){LoadCatalog();return true;}
 if(Name==TEXT("nf.markitemstate"))return MarkItemState();
 FGuid G; FString Key=FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);
 if(!A.IsEmpty() && FGuid::ParseExact(A.Last(),EGuidFormats::DigitsWithHyphens,G)){Key=G.ToString(EGuidFormats::DigitsWithHyphensLower);A.Pop();}
 auto Number=[](const FString& Text){uint64 V=0;SkillBotPredicates::ParseUnsigned(Text,V);return V;};
 if(Name==TEXT("nf.grantitem")){Grant(uint32(Number(A[0])),Number(A[1]),Key);return true;}
 if(!HasBaseline())return false;
 static const TCHAR* CommandNames[]={TEXT(""),TEXT("nf.equip"),TEXT("nf.unequip"),TEXT("nf.useitem"),TEXT("nf.destroy"),TEXT("nf.enchant"),TEXT("nf.autoshot"),TEXT("nf.refine"),TEXT("nf.cancelrefine"),TEXT("nf.exchangesa"),TEXT("nf.crystallize")};
 EGrpcNightfallV1ItemOperation Op=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_UNSPECIFIED;for(uint8 I=1;I<UE_ARRAY_COUNT(CommandNames);++I)if(Name==CommandNames[I])Op=EGrpcNightfallV1ItemOperation(I);
 const FString Scope=RequestScope(Op,Key), Arguments=FString::Join(A,TEXT(" "));
 if(Scope.IsEmpty())return false;
 const auto* Previous=FrozenRequests.Find(Scope);
 TArray<FString> Original;FrozenArguments.FindRef(Scope).ParseIntoArrayWS(Original);
 if (const auto* Frozen=Previous) {
  if (FrozenArguments.FindRef(Scope)==Arguments) {const uint32 Retry=Net->SendItemMessage(*Frozen); FinishSend(Retry,Key,Op); return Retry!=0;}
 }
 auto ResolveAt=[&](int32 Index,const FString& Selector)->TOptional<uint64>{
  // Unchanged selectors retain the originally received binding even after consumption.
  if(Previous && Original.IsValidIndex(Index) && Original[Index]==A[Index]){
   if(Index==0){
    if(Previous->EquipItem.IsSet())return Previous->EquipItem->InstanceId.Value;
    if(Previous->UseItem.IsSet())return Previous->UseItem->InstanceId.Value;
    if(Previous->DestroyItem.IsSet())return Previous->DestroyItem->InstanceId.Value;
    if(Previous->CrystallizeItem.IsSet())return Previous->CrystallizeItem->InstanceId.Value;
    if(Previous->EnchantItem.IsSet())return Previous->EnchantItem->ScrollInstanceId.Value;
    if(Previous->RefineItem.IsSet())return Previous->RefineItem->TargetInstanceId.Value;
    if(Previous->CancelRefine.IsSet())return Previous->CancelRefine->TargetInstanceId.Value;
    if(Previous->ExchangeSpecialAbility.IsSet())return Previous->ExchangeSpecialAbility->TargetInstanceId.Value;
   }
   if(Index==1 && Previous->EnchantItem.IsSet())return Previous->EnchantItem->TargetInstanceId.Value;
   if(Index==1 && Previous->RefineItem.IsSet())return Previous->RefineItem->StoneInstanceId.Value;
   if(Index==2 && Previous->EnchantItem.IsSet() && Previous->EnchantItem->_support_instance_id._support_instance_idCase==EGrpcNightfallV1EnchantItemRequest_support_instance_id::SupportInstanceId)return Previous->EnchantItem->_support_instance_id.SupportInstanceId.Value;
  }
  return Resolve(Selector);
 };
 auto ResolveTarget=[&](int32 Index,bool Npc)->TOptional<FGrpcNightfallV1ItemTarget>{
  const FString Selector=A.IsValidIndex(Index)?A[Index]:FString();
  if(Previous && Selector==(Original.IsValidIndex(Index)?Original[Index]:FString())){
   if(Previous->UseItem.IsSet() && Previous->UseItem->HasTarget)return Previous->UseItem->Target;
   if(Previous->RefineItem.IsSet())return Previous->RefineItem->Npc;
   if(Previous->CancelRefine.IsSet())return Previous->CancelRefine->Npc;
   if(Previous->ExchangeSpecialAbility.IsSet())return Previous->ExchangeSpecialAbility->Npc;
  }
  return Target(Selector,Npc);
 };
 FClientMessage FrozenMessage;
 uint32 Seq=0;
 if(Name==TEXT("nf.unequip")){FGrpcNightfallV1UnequipItemRequest Q;Q.IdempotencyKey=Key;Q.Slot=NightfallItems::Slot(A[0]);FrozenMessage.UnequipItem=Q;Seq=Net->SendUnequipItem(Q);}
 else if(Name==TEXT("nf.autoshot")){FGrpcNightfallV1ToggleAutoShotRequest Q;Q.IdempotencyKey=Key;Q.TemplateId=uint32(Number(A[0]));Q.Enabled=A[1]==TEXT("on");FrozenMessage.ToggleAutoShot=Q;Seq=Net->SendToggleAutoShot(Q);}
 else {
  const auto Id=ResolveAt(0,A[0]);if(!Id.IsSet())return false;
  if(Name==TEXT("nf.equip")){FGrpcNightfallV1EquipItemRequest Q;Q.IdempotencyKey=Key;Q.InstanceId=*Id;if(A.Num()>1)Q.PreferredSlot=NightfallItems::Slot(A[1]);FrozenMessage.EquipItem=Q;Seq=Net->SendEquipItem(Q);}
  else if(Name==TEXT("nf.useitem")){FGrpcNightfallV1UseItemRequest Q;Q.IdempotencyKey=Key;Q.InstanceId=*Id;if(A.Num()>1){const auto T=ResolveTarget(1,false);if(!T.IsSet())return false;Q.HasTarget=true;Q.Target=*T;}FrozenMessage.UseItem=Q;Seq=Net->SendUseItem(Q);}
  else if(Name==TEXT("nf.destroy")){FGrpcNightfallV1DestroyItemRequest Q;Q.IdempotencyKey=Key;Q.InstanceId=*Id;Q.Count=Number(A[1]);FrozenMessage.DestroyItem=Q;Seq=Net->SendDestroyItem(Q);}
  else if(Name==TEXT("nf.crystallize")){FGrpcNightfallV1CrystallizeItemRequest Q;Q.IdempotencyKey=Key;Q.InstanceId=*Id;FrozenMessage.CrystallizeItem=Q;Seq=Net->SendCrystallizeItem(Q);}
  else if(Name==TEXT("nf.enchant")){const auto TargetId=ResolveAt(1,A[1]);if(!TargetId.IsSet())return false;FGrpcNightfallV1EnchantItemRequest Q;Q.IdempotencyKey=Key;Q.ScrollInstanceId=*Id;Q.TargetInstanceId=*TargetId;if(A.Num()>2){const auto Support=ResolveAt(2,A[2].Mid(8));if(!Support.IsSet())return false;Q._support_instance_id._support_instance_idCase=EGrpcNightfallV1EnchantItemRequest_support_instance_id::SupportInstanceId;Q._support_instance_id.SupportInstanceId=*Support;}FrozenMessage.EnchantItem=Q;Seq=Net->SendEnchantItem(Q);}
  else if(Name==TEXT("nf.refine") || Name==TEXT("nf.cancelrefine") || Name==TEXT("nf.exchangesa")) {
   const int32 Index=Name==TEXT("nf.cancelrefine")?1:2;const auto Npc=ResolveTarget(Index,true);if(!Npc.IsSet())return false;
   if(Name==TEXT("nf.refine")){const auto Stone=ResolveAt(1,A[1]);if(!Stone.IsSet())return false;FGrpcNightfallV1RefineItemRequest Q;Q.IdempotencyKey=Key;Q.HasNpc=true;Q.Npc=*Npc;Q.TargetInstanceId=*Id;Q.StoneInstanceId=*Stone;FrozenMessage.RefineItem=Q;Seq=Net->SendRefineItem(Q);}
   else if(Name==TEXT("nf.cancelrefine")){FGrpcNightfallV1CancelRefineRequest Q;Q.IdempotencyKey=Key;Q.HasNpc=true;Q.Npc=*Npc;Q.TargetInstanceId=*Id;FrozenMessage.CancelRefine=Q;Seq=Net->SendCancelRefine(Q);}
   else {FGrpcNightfallV1ExchangeSpecialAbilityRequest Q;Q.IdempotencyKey=Key;Q.HasNpc=true;Q.Npc=*Npc;Q.TargetInstanceId=*Id;Q.RecipeId=Number(A[1]);FrozenMessage.ExchangeSpecialAbility=Q;Seq=Net->SendExchangeSpecialAbility(Q);}
  } else return false;
 }
 if(Seq && !FrozenRequests.Contains(Scope)){FrozenRequests.Add(Scope,FrozenMessage);FrozenArguments.Add(Scope,Arguments);}
 FinishSend(Seq,Key,Op);return Seq!=0;
}

TArray<TPair<uint32,bool>> UInventoryStateSubsystem::Hotbar() const
{
 TArray<TPair<uint32,bool>> Slots; Slots.Init(TPair<uint32,bool>(0,false),9); if (!Net) return Slots;
 const FString Id=Net->GetOwnEntityId().ToLower(); if (const auto* Held=Hotbars.Find(Id)) return *Held;
 FConfigFile C; C.Read(HotbarConfigFile()); TArray<FString> Saved; C.GetArray(TEXT("Items"),*Id,Saved);
 if (Saved.Num()==9) for(int32 I=0;I<9;++I){FString T,A;uint64 N=0;if(Saved[I].Split(TEXT(":"),&T,&A) && SkillBotPredicates::ParseUnsigned(T,N) && N<=MAX_uint32 && (A==TEXT("0") || A==TEXT("1")))Slots[I]={uint32(N),A==TEXT("1")};}
 return Slots;
}
bool UInventoryStateSubsystem::AssignHotbar(int32 Index,uint32 Template,bool Auto)
{
 if (!Net || Index<0 || Index>=9 || !HasBaseline() || (Template && !Definition(Template))) return false;
 auto Slots=Hotbar(); Slots[Index]={Template,Auto}; Hotbars.Add(Net->GetOwnEntityId().ToLower(),Slots);
 FConfigFile C;const FString File=HotbarConfigFile(); C.Read(File); C.bCanSaveAllSections=true;
 TArray<FString> Saved; for (const auto& S:Slots) Saved.Add(FString::Printf(TEXT("%u:%d"),S.Key,S.Value)); C.SetArray(TEXT("Items"),*Net->GetOwnEntityId().ToLower(),Saved); IFileManager::Get().MakeDirectory(*FPaths::GetPath(File),true); C.Write(File); OnChanged.Broadcast(); return true;
}
uint32 UInventoryStateSubsystem::ActivateHotbar(int32 Index)
{
 const auto Slots=Hotbar(); if (!Slots.IsValidIndex(Index) || !Slots[Index].Key || !HasBaseline()) return 0;
 const auto& Slot=Slots[Index];TArray<FString> T;
 if (Slot.Value) T={TEXT("nf.AutoShot"),FString::Printf(TEXT("%u"),Slot.Key),AutoShot(Slot.Key)?TEXT("off"):TEXT("on")};
 else {
  T={TEXT("nf.UseItem"),FString::Printf(TEXT("template:%u"),Slot.Key)};
  auto* C=GetGameInstance()->GetSubsystem<UCombatStateSubsystem>(); if(C && !C->GetOwn().TargetId.IsEmpty()) T.Add(TEXT("target"));
 }
 return Execute(T)?LastSendSeq:0;
}

FString UInventoryStateSubsystem::HotbarConfigFile() const {return HotbarConfigOverride.IsEmpty()?FPaths::Combine(FPaths::GeneratedConfigDir(),TEXT("NightfallItems.ini")):HotbarConfigOverride;}

void UInventoryStateSubsystem::ApplyReceiptPayload(const FWorldEvent& E)
{
 auto Receive=[&](const auto& R,EGrpcNightfallV1ItemOperation Op){
  const FString Scope=RequestScope(Op,R.ReceiptId);if(Scope.IsEmpty())return;
  const bool Requested=Outstanding.Contains(Scope);
  if(!Accept(R.EntityId,R.Incarnation,R.Tick,R.InventoryRevision,true) && !Receipts.Contains(Scope) && !Requested)return;
  TArray<uint8> Bytes=E.ItemReceiptPayload;if(Bytes.IsEmpty() && !E.bReceivedFromWire)NightfallWire::EncodeItemReceipt(R,Bytes);Remember(Op,R.ReceiptId,Bytes);};
 if(E.ItemOperationResult.IsSet())Receive(*E.ItemOperationResult,E.ItemOperationResult->Operation);
 if(E.EnchantResult.IsSet())Receive(*E.EnchantResult,EGrpcNightfallV1ItemOperation::ITEM_OPERATION_ENCHANT);
 if(E.RefineResult.IsSet())Receive(*E.RefineResult,E.RefineResult->Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED?EGrpcNightfallV1ItemOperation::ITEM_OPERATION_REFINE:EGrpcNightfallV1ItemOperation::ITEM_OPERATION_CANCEL_REFINE);
}
