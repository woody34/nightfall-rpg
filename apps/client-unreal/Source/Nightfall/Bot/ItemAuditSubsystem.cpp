#include "ItemAuditSubsystem.h"
#include "Items/InventoryStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
void UItemAuditSubsystem::Initialize(FSubsystemCollectionBase& C)
{
 Super::Initialize(C);State=C.InitializeDependency<UInventoryStateSubsystem>();Net=C.InitializeDependency<UNetClientSubsystem>();
 Net->OnItemReceiptPayload.AddUObject(this,&UItemAuditSubsystem::Observe);Net->OnConnected.AddDynamic(this,&UItemAuditSubsystem::Reset);Net->OnDisconnected.AddDynamic(this,&UItemAuditSubsystem::Disconnected);
 Net->OnEntitySpawn.AddWeakLambda(this,[this](const FEntitySpawn& S){if(Net->IsOwnEntity(S.EntityId))Reset();});
}
void UItemAuditSubsystem::Deinitialize(){if(Net){Net->OnItemReceiptPayload.RemoveAll(this);Net->OnConnected.RemoveAll(this);Net->OnDisconnected.RemoveAll(this);Net->OnEntitySpawn.RemoveAll(this);}Super::Deinitialize();}
void UItemAuditSubsystem::Reset(){bObserved=false;Violations.Reset();}
void UItemAuditSubsystem::Observe(const FWorldEvent& E)
{
 if(!State || !State->HasBaseline())return;
 if(E.EnchantResult.IsSet()){
  const auto& R=*E.EnchantResult;const auto& Held=State->GetEnchant();if(!Held.IsSet() || Held->ReceiptId!=R.ReceiptId || R.InventoryRevision.Value!=State->Revision())return;
  bObserved=true;if(!State->EnchantConsistent())Violations.AddUnique(TEXT("received enchant result/inventory mismatch"));
 }
 if(E.RefineResult.IsSet()){
  const auto& R=*E.RefineResult;const auto& Held=State->GetRefine();if(!Held.IsSet() || Held->ReceiptId!=R.ReceiptId || R.InventoryRevision.Value!=State->Revision())return;
  const auto* I=State->Find(R.TargetInstanceId);bObserved=true;
  const bool Present=I && I->_augment_id._augment_idCase==EGrpcNightfallV1OwnedItem_augment_id::AugmentId;
  const bool Expected=R.Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED;
  if(!I || Present!=Expected || (Expected && (R._augment_id._augment_idCase!=EGrpcNightfallV1RefineResult_augment_id::AugmentId || I->_augment_id.AugmentId.Value!=R._augment_id.AugmentId.Value)))Violations.AddUnique(TEXT("received refine result/inventory mismatch"));
 }
}
TOptional<bool> UItemAuditSubsystem::Check(FString& Detail) const
{
 if(!State || !State->HasBaseline() || !bObserved){Detail=TEXT("unknown: accepted current-revision item outcome missing");return {};}
 Detail=Violations.IsEmpty()?TEXT("received item outcomes coherent"):FString::Join(Violations,TEXT("; "));return Violations.IsEmpty();
}
