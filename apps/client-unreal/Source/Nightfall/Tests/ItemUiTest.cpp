#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Items/InventoryStateSubsystem.h"
#include "Items/EquipmentPresentationComponent.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallWire.h"
#include "NightfallPlayerController.h"
#include "UI/NightfallInventoryDialog.h"
#include "UI/NightfallHud.h"
#include "UI/NightfallItemOperationDialog.h"
#include "Components/Button.h"
#include "Components/ComboBoxString.h"
#include "Components/ProgressBar.h"
#include "Components/StaticMeshComponent.h"
#include "Components/InputComponent.h"
#include "Components/UniformGridPanel.h"
#include "IWebSocket.h"
#include "Misc/Paths.h"
#include "HAL/FileManager.h"
#if WITH_DEV_AUTOMATION_TESTS
namespace ItemUiTest
{
 const TCHAR* Owner=TEXT("11111111-1111-4111-8111-111111111111");
 class FSocket final : public IWebSocket
 {
 public:
  TArray<TArray<uint8>> Sent;
  void Connect() override {} void Close(int32,const FString&) override {} bool IsConnected() override {return true;}
  void Send(const FString&) override {} void Send(const void* Data,SIZE_T Size,bool) override {Sent.Emplace(static_cast<const uint8*>(Data),int32(Size));}
  void SetTextMessageMemoryLimit(uint64) override {}
  FWebSocketConnectedEvent Connected;FWebSocketConnectionErrorEvent Error;FWebSocketClosedEvent Closed;FWebSocketMessageEvent Message;FWebSocketBinaryMessageEvent Binary;FWebSocketRawMessageEvent Raw;FWebSocketMessageSentEvent MessageSent;
  FWebSocketConnectedEvent& OnConnected() override {return Connected;} FWebSocketConnectionErrorEvent& OnConnectionError() override {return Error;}
  FWebSocketClosedEvent& OnClosed() override {return Closed;} FWebSocketMessageEvent& OnMessage() override {return Message;} FWebSocketBinaryMessageEvent& OnBinaryMessage() override {return Binary;}
  FWebSocketRawMessageEvent& OnRawMessage() override {return Raw;} FWebSocketMessageSentEvent& OnMessageSent() override {return MessageSent;}
 };
 FGrpcNightfallV1OwnedItem Item(uint64 Id,uint32 Template) {FGrpcNightfallV1OwnedItem I;I.InstanceId=Id;I.TemplateId=Template;I.Count=1;I.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_INVENTORY;return I;}
 void Seed(UNetClientSubsystem* N,UInventoryStateSubsystem* S)
 {
  S->GetGameInstance()->GetSubsystem<USessionClient>()->SetBearerToken(TEXT("test:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));N->SetOwnEntityId(Owner);FEntitySpawn Spawn;Spawn.EntityId=Owner;Spawn.Kind=1;Spawn.StateTick=10;Spawn.LifeIncarnation=1;S->ApplySpawn(Spawn);
  FGrpcNightfallV1InventorySync Sync;Sync.EntityId=NightfallItems::UuidBytes(Owner);Sync.Tick=Sync.BaselineTick=10;Sync.Incarnation=1;Sync.InventoryRevision=1;Sync.Baseline=true;Sync.HasWeight=Sync.HasPenalties=true;Sync.RulesHash.Value.Init(1,32);Sync.Items={Item(1,6),Item(2,951)};Sync.Weight.Weight=50;Sync.Weight.MaxLoad=100;Sync.Weight.PerMille=500;Sync.Penalties.Overloaded=true;S->ApplySync(Sync);
  FGrpcNightfallV1GetItemCatalogResponse Catalog;Catalog.DataVersion=2;Catalog.RulesHash=Sync.RulesHash;
  FGrpcNightfallV1ItemDefinition D;D.TemplateId=6;D.Name=TEXT("Wand");D.Kind=EGrpcNightfallV1ItemKind::ITEM_KIND_WEAPON;D.Grade=EGrpcNightfallV1ItemGrade::ITEM_GRADE_NG;D.Usable=true;Catalog.Items.Add(D);D.TemplateId=951;D.Name=TEXT("Enchant scroll");D.Kind=EGrpcNightfallV1ItemKind::ITEM_KIND_ETC;Catalog.Items.Add(D);S->ApplyCatalog(Catalog);
 }
 TSharedRef<FSocket> Connect(UNetClientSubsystem* N)
 {
  auto Socket=MakeShared<FSocket>();N->SetSocketFactoryForTesting([Socket](const auto&)->TSharedRef<IWebSocket>{return Socket;});N->SetSchedulerForTesting([](float,TFunction<void()>){ });N->Connect(TEXT("ws://test"),TEXT("test"));Socket->Connected.Broadcast();return Socket;
 }
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemInventoryUiTest,"Nightfall.Item.UI.InventoryCountsWeightInput",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemInventoryUiTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 auto* D=NewObject<UNightfallInventoryDialog>(Scope.GameInstance);D->EnsureLayout();D->BindState(S,false);D->Select(1);
 TestEqual(TEXT("grid receives both owner items"),D->GridControl()->GetChildrenCount(),2);TestTrue(TEXT("count/enchant badge"),D->ItemLabel(1).Contains(TEXT("x1")) && D->ItemLabel(1).Contains(TEXT("+0")));TestTrue(TEXT("tooltip from catalogue"),D->Tooltip(6).Contains(TEXT("Wand")));TestEqual(TEXT("received weight fraction"),D->WeightControl()->GetPercent(),0.5f);TestEqual(TEXT("received overload tint"),D->WeightControl()->GetFillColorAndOpacity(),FLinearColor::Red);
 D->EquipControl()->OnClicked.Broadcast();TestEqual(TEXT("actual equip button sends one intent"),Socket->Sent.Num(),1);FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestEqual(TEXT("selected instance sent"),M.Intent.EquipItem.InstanceId.Value,uint64(1));TestEqual(TEXT("selection key retained"),M.Intent.EquipItem.IdempotencyKey,D->RetryKey());
 TestFalse(TEXT("pending disables action"),D->EquipControl()->GetIsEnabled());FIntentRejected R;R.Seq=N->GetLastSentSeq();R.Detail=TEXT("item refused");S->ApplyRejected(R);TestTrue(TEXT("rejection restores action"),D->EquipControl()->GetIsEnabled());
 S->Reset();TestEqual(TEXT("disconnect clears owner grid"),D->GridControl()->GetChildrenCount(),0);TestFalse(TEXT("unknown blocks equip"),D->EquipControl()->GetIsEnabled());N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemOperationUiTest,"Nightfall.Item.UI.EnchantCrystallizeBlacksmithRetry",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemOperationUiTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 auto* D=NewObject<UNightfallItemOperationDialog>(Scope.GameInstance);D->EnsureLayout();D->BindState(S,false,1);D->IngredientControl()->SetSelectedIndex(1);TestTrue(TEXT("received selections permit enchant"),D->CanConfirm());const FString Key=D->RetryKey();D->ConfirmControl()->OnClicked.Broadcast();TestEqual(TEXT("actual enchant button sends"),Socket->Sent.Num(),1);
 FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestEqual(TEXT("scroll instance"),M.Intent.EnchantItem.ScrollInstanceId.Value,uint64(2));TestEqual(TEXT("target instance"),M.Intent.EnchantItem.TargetInstanceId.Value,uint64(1));TestEqual(TEXT("same retry key"),M.Intent.EnchantItem.IdempotencyKey,Key);
 FGrpcNightfallV1InventoryChanged C;C.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);C.Incarnation=1;C.Tick=11;C.InventoryRevision=2;C.PreviousRevision=1;C.RemovedIds.Add(2);auto I=ItemUiTest::Item(1,6);I.Enchant=1;C.Updated.Add(I);S->ApplyChanged(C);
 FGrpcNightfallV1EnchantResult E;E.EntityId=C.EntityId;E.Incarnation=1;E.Tick=11;E.InventoryRevision=2;E.ReceiptId=Key;E.TargetInstanceId=1;E.NewEnchant=1;E.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS;FWorldEvent Event;Event.EnchantResult=E;FServerMessage Server;Server.Event=Event;N->DispatchServerMessage(Server);
 TestTrue(TEXT("received outcome coherent"),S->EnchantConsistent());D->RetryControl()->OnClicked.Broadcast();TestEqual(TEXT("retry sends after scroll consumed"),Socket->Sent.Num(),2);FGrpcNightfallV1ClientMessage Retry;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Retry);TestEqual(TEXT("frozen exact body"),Retry.Intent.EnchantItem.ToJsonString(false),M.Intent.EnchantItem.ToJsonString(false));
 N->DispatchServerMessage(Server);TestTrue(TEXT("frozen receipt matches first"),S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst());
 D->ActionControl()->SetSelectedOption(TEXT("Crystallize"));TestTrue(TEXT("new action gets new key"),D->RetryKey()!=Key);
 auto* B=NewObject<UNightfallItemOperationDialog>(Scope.GameInstance);B->BindState(S,true,1);TestFalse(TEXT("blacksmith unknown prevents confirmation"),B->CanConfirm());
 FEntitySpawn Blacksmith;Blacksmith.EntityId=TEXT("22222222-2222-4222-8222-222222222222");Blacksmith.Kind=2;Blacksmith.Name=TEXT("Blacksmith");Blacksmith.StateTick=10;Blacksmith.LifeIncarnation=1;S->ApplySpawn(Blacksmith);B->ActionControl()->SetSelectedOption(TEXT("CancelRefine"));TestTrue(TEXT("received blacksmith authorizes request selection"),B->CanConfirm());S->ApplyDespawn({Blacksmith.EntityId});TestFalse(TEXT("despawn invalidates blacksmith"),B->CanConfirm());N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemHotbarUiTest,"Nightfall.Item.UI.HotbarKeyAndToggle",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemHotbarUiTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 const FString Config=FPaths::Combine(FPaths::ProjectSavedDir(),TEXT("Tests"),FGuid::NewGuid().ToString()+TEXT("-items.ini"));S->SetHotbarConfigForTesting(Config);TestTrue(TEXT("assign from received catalogue"),S->AssignHotbar(0,6,true));
 auto* PC=Scope.GameInstance->GetWorld()->SpawnActor<ANightfallPlayerController>();if(!TestNotNull(TEXT("controller"),PC))return false;PC->InputComponent=NewObject<UInputComponent>(PC);PC->BindSkillKeys();auto* B=PC->InputComponent->KeyBindings.FindByPredicate([](const auto& K){return K.Chord.Key==EKeys::F1 && K.KeyEvent==IE_Pressed;});if(!TestNotNull(TEXT("actual F1 key bound"),B))return false;B->KeyDelegate.Execute(EKeys::F1);FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestTrue(TEXT("toggle request on"),M.Intent.ToggleAutoShot.Enabled);TestFalse(TEXT("no optimistic toggle"),S->AutoShot(6));
 S->SetHotbarConfigForTesting(Config);TestEqual(TEXT("character hotbar persisted"),S->Hotbar()[0].Key,uint32(6));IFileManager::Get().Delete(*Config);N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemPresentationTest,"Nightfall.Item.Presentation.PublicGearEmptyAndGlow",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemPresentationTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();ItemUiTest::Seed(N,S);
 auto* P=NewObject<UEquipmentPresentationComponent>(Scope.GameInstance);P->Bind(S,ItemUiTest::Owner);TestFalse(TEXT("empty gear hides weapon"),P->WeaponControl()->IsVisible());
 FGrpcNightfallV1EquipmentChanged E;E.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);E.Incarnation=1;E.Tick=11;FGrpcNightfallV1VisibleEquipment V;V.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND;V.TemplateId=6;V.GlowTier=1;E.Visible.Add(V);auto Chest=V;Chest.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_CHEST;Chest.TemplateId=425;E.Visible.Add(Chest);S->ApplyEquipment(E);TestTrue(TEXT("public chest displays armour placeholder"),P->ArmourControl()->IsVisible());TestTrue(TEXT("public template displays mesh"),P->WeaponControl()->IsVisible());TestEqual(TEXT("received glow tier"),P->GlowTier(),uint32(1));TestEqual(TEXT("public template cannot alter private inventory"),*S->Count(6),uint64(1));
 E.Tick=12;E.Visible[0].TemplateId=1;E.Visible[0].GlowTier=2;auto Previous=P->WeaponControl()->GetStaticMesh();S->ApplyEquipment(E);TestTrue(TEXT("template swaps placeholder mesh"),Previous!=P->WeaponControl()->GetStaticMesh());
 E.Tick=13;E.Visible[0].TemplateId=0;S->ApplyEquipment(E);TestFalse(TEXT("explicit tombstone clears mesh"),P->WeaponControl()->IsVisible());S->ApplyEquipment(E);TestFalse(TEXT("repeat tombstone stable"),P->WeaponControl()->IsVisible());return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemAccountPendingTest,"Nightfall.Item.Receipt.AccountOperationUuidAndConcurrentRequests",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemAccountPendingTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto* Session=Scope.Get<USessionClient>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 const FString Key=TEXT("ABCDEFAB-1111-4111-8111-123456789ABC");
 TestTrue(TEXT("first pending request"),S->Execute({TEXT("nf.Equip"),TEXT("template:6"),Key}));const uint32 First=N->GetLastSentSeq();
 TestTrue(TEXT("same UUID different operation legal"),S->Execute({TEXT("nf.Unequip"),TEXT("rhand"),Key}));const uint32 Second=N->GetLastSentSeq();
 FIntentRejected Rejected;Rejected.Seq=First;Rejected.Detail=TEXT("first rejected");S->ApplyRejected(Rejected);TestTrue(TEXT("second still outstanding"),S->IsBusy());TestEqual(TEXT("first rejection tracked"),S->Status(),Rejected.Detail);
 Rejected.Seq=Second;S->ApplyRejected(Rejected);TestFalse(TEXT("both independent rejections finish"),S->IsBusy());
 TestTrue(TEXT("explicit equip retry"),S->Execute({TEXT("nf.Equip"),TEXT("template:6"),Key.ToLower()}));
 const uint32 RetrySeq=N->GetLastSentSeq();TestTrue(TEXT("same scope concurrent retry"),S->Execute({TEXT("nf.Equip"),TEXT("template:6"),Key}));Rejected.Seq=RetrySeq;S->ApplyRejected(Rejected);TestTrue(TEXT("rejecting one retry retains other sequence"),S->IsBusy());
 FGrpcNightfallV1ClientMessage Original;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Original);TestEqual(TEXT("UUID normalized"),Original.Intent.EquipItem.IdempotencyKey,Key.ToLower());
 FGrpcNightfallV1ItemOperationResult Receipt;Receipt.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);Receipt.Incarnation=1;Receipt.Tick=11;Receipt.InventoryRevision=1;Receipt.ReceiptId=Key;Receipt.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP;
 FWorldEvent Event;Event.ItemOperationResult=Receipt;S->ApplyReceiptPayload(Event);TestFalse(TEXT("receipt completes matching request"),S->IsBusy());
 const FString Other=TEXT("22222222-2222-4222-8222-222222222222");N->SetOwnEntityId(Other);FEntitySpawn Spawn;Spawn.EntityId=Other;Spawn.Kind=1;Spawn.StateTick=20;Spawn.LifeIncarnation=1;S->ApplySpawn(Spawn);auto Base=S->GetInventory();Base.EntityId=NightfallItems::UuidBytes(Other);Base.Tick=Base.BaselineTick=20;Base.Incarnation=1;Base.Baseline=true;Base.HasWeight=Base.HasPenalties=true;Base.InventoryRevision=1;Base.RulesHash.Value.Init(1,32);Base.Items={ItemUiTest::Item(9,6)};S->ApplySync(Base);
 TestTrue(TEXT("cross-character explicit retry"),S->Execute({TEXT("nf.Equip"),TEXT("template:6"),Key}));FGrpcNightfallV1ClientMessage Retry;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Retry);TestEqual(TEXT("account preserves frozen body across characters"),Retry.Intent.EquipItem.ToJsonString(false),Original.Intent.EquipItem.ToJsonString(false));
 S->ApplyReceiptPayload(Event);TestTrue(TEXT("old-character receipt compares in account scope"),S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst());TestFalse(TEXT("cross-character pending completed"),S->IsBusy());TestFalse(TEXT("old character result is never live"),S->LastOperation().IsSet());
 Receipt.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_UNEQUIP;Receipt.EntityId=Base.EntityId;Receipt.Tick=21;Event.ItemOperationResult=Receipt;S->ApplyReceiptPayload(Event);TestFalse(TEXT("same UUID new operation has separate history"),S->ReceiptMatchesFirst().IsSet());S->ApplyReceiptPayload(Event);TestTrue(TEXT("operation replay compares only own history"),*S->ReceiptMatchesFirst());
 Session->SetBearerToken(TEXT("test:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"));S->ApplySpawn(Spawn);S->ApplySync(Base);S->ApplyReceiptPayload(Event);TestFalse(TEXT("different account has separate receipt history"),S->ReceiptMatchesFirst().IsSet());N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemConflictingBodyTest,"Nightfall.Item.Receipt.OriginalReceiptBeforeFingerprintConflict",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemConflictingBodyTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 const FString Key=TEXT("abcdefab-1111-4111-8111-123456789abc");
 TestTrue(TEXT("original count submitted"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("1"),Key}));const uint32 Original=N->GetLastSentSeq();
 TestTrue(TEXT("same account operation UUID different count submitted"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("2"),Key}));const uint32 Conflict=N->GetLastSentSeq();
 FGrpcNightfallV1ClientMessage First,Second;NightfallWire::DecodeClientMessage(Socket->Sent[0].GetData(),Socket->Sent[0].Num(),First);NightfallWire::DecodeClientMessage(Socket->Sent[1].GetData(),Socket->Sent[1].Num(),Second);
 TestEqual(TEXT("original wire count"),First.Intent.DestroyItem.Count.Value,uint64(1));TestEqual(TEXT("conflict wire count"),Second.Intent.DestroyItem.Count.Value,uint64(2));TestEqual(TEXT("same key on wire"),First.Intent.DestroyItem.IdempotencyKey,Second.Intent.DestroyItem.IdempotencyKey);
 TestTrue(TEXT("matching body retry submitted"),S->Execute({TEXT("nf.Destroy"),TEXT("template:6"),TEXT("1"),Key}));const uint32 Retry=N->GetLastSentSeq();
 FGrpcNightfallV1ItemOperationResult Receipt;Receipt.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);Receipt.Incarnation=1;Receipt.Tick=11;Receipt.InventoryRevision=2;Receipt.ReceiptId=Key;Receipt.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_DESTROY;
 FWorldEvent Event;Event.ItemOperationResult=Receipt;S->ApplyReceiptPayload(Event);
 TestTrue(TEXT("original receipt retains conflicting sequence pending"),S->IsBusy());TestEqual(TEXT("still awaiting conflict result"),S->Status(),FString(TEXT("Waiting for server item result")));
 FIntentRejected Rejected;Rejected.Seq=Original;Rejected.Detail=TEXT("late rejection of completed original");S->ApplyRejected(Rejected);Rejected.Seq=Retry;S->ApplyRejected(Rejected);
 TestEqual(TEXT("receipt completed both matching fingerprints despite different selectors"),S->Status(),FString(TEXT("Waiting for server item result")));TestTrue(TEXT("conflict remains pending"),S->IsBusy());
 Rejected.Seq=Conflict;Rejected.Detail=TEXT("D43 fingerprint conflict: FAILED_PRECONDITION");S->ApplyRejected(Rejected);
 TestFalse(TEXT("conflict rejection clears final pending sequence"),S->IsBusy());TestEqual(TEXT("displayed failure retained"),S->Status(),Rejected.Detail);
 S->ApplyReceiptPayload(Event);TestEqual(TEXT("replayed receipt cannot hide conflict failure"),S->Status(),Rejected.Detail);
 Rejected.Detail=TEXT("duplicate rejection");S->ApplyRejected(Rejected);TestEqual(TEXT("terminal rejection is not processed twice"),S->Status(),FString(TEXT("D43 fingerprint conflict: FAILED_PRECONDITION")));
 N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemCorrectedBodyTest,"Nightfall.Item.Receipt.CorrectedBodyAfterValidationRejection",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemCorrectedBodyTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 const FString Key=TEXT("abcdefab-1111-4111-8111-123456789abc");
 TestTrue(TEXT("excessive count submitted"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("2"),Key}));const uint32 First=N->GetLastSentSeq();
 TestTrue(TEXT("excessive count retry submitted"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("2"),Key}));const uint32 Retry=N->GetLastSentSeq();
 FIntentRejected Rejected;Rejected.Seq=First;Rejected.Detail=TEXT("excessive count: FAILED_PRECONDITION");S->ApplyRejected(Rejected);TestTrue(TEXT("one rejection leaves other attempt outstanding"),S->IsBusy());
 Rejected.Seq=Retry;S->ApplyRejected(Rejected);TestFalse(TEXT("all validation rejections clear busy"),S->IsBusy());TestEqual(TEXT("validation failure displayed"),S->Status(),Rejected.Detail);
 TestTrue(TEXT("corrected same-key body submitted"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("1"),Key}));const uint32 Corrected=N->GetLastSentSeq();
 FGrpcNightfallV1ClientMessage Accepted;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Accepted);TestEqual(TEXT("corrected wire count"),Accepted.Intent.DestroyItem.Count.Value,uint64(1));TestEqual(TEXT("corrected wire key unchanged"),Accepted.Intent.DestroyItem.IdempotencyKey,Key);
 TestTrue(TEXT("corrected request awaiting receipt"),S->IsBusy());TestEqual(TEXT("pending display"),S->Status(),FString(TEXT("Waiting for server item result")));
 FGrpcNightfallV1InventoryChanged Changed;Changed.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);Changed.Incarnation=1;Changed.Tick=11;Changed.PreviousRevision=1;Changed.InventoryRevision=2;Changed.RemovedIds.Add(1);S->ApplyChanged(Changed);
 FGrpcNightfallV1ItemOperationResult Receipt;Receipt.EntityId=Changed.EntityId;Receipt.Incarnation=1;Receipt.Tick=11;Receipt.InventoryRevision=2;Receipt.ReceiptId=Key;Receipt.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_DESTROY;
 FWorldEvent Event;Event.ItemOperationResult=Receipt;S->ApplyReceiptPayload(Event);
 TestFalse(TEXT("corrected receipt clears pending and busy"),S->IsBusy());TestEqual(TEXT("successful receipt displayed"),S->Status(),FString(TEXT("Server committed item operation")));TestEqual(TEXT("committed destroy removes displayed count"),*S->Count(6),uint64(0));
 Rejected.Seq=Corrected;Rejected.Detail=TEXT("late rejection");S->ApplyRejected(Rejected);TestEqual(TEXT("corrected sequence is terminal"),S->Status(),FString(TEXT("Server committed item operation")));
 TestTrue(TEXT("identical corrected retry after consumption"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("1"),Key}));FGrpcNightfallV1ClientMessage Replay;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Replay);TestEqual(TEXT("corrected arguments retain exact committed body"),Replay.Intent.DestroyItem.ToJsonString(false),Accepted.Intent.DestroyItem.ToJsonString(false));
 S->ApplyReceiptPayload(Event);TestFalse(TEXT("corrected retry receipt clears busy"),S->IsBusy());TestTrue(TEXT("identical receipt replay matches"),S->ReceiptMatchesFirst().IsSet() && *S->ReceiptMatchesFirst());TestEqual(TEXT("retry success displayed"),S->Status(),FString(TEXT("Server committed item operation")));
 S->ApplyReceiptPayload(Event);TestFalse(TEXT("duplicate receipt stays complete"),S->IsBusy());N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemCommittedBodyTest,"Nightfall.Item.Receipt.CommittedBodySurvivesLaterConflict",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemCommittedBodyTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 const FString Key=TEXT("abcdefab-1111-4111-8111-123456789abc");
 TestTrue(TEXT("original count submitted"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("1"),Key}));
 FGrpcNightfallV1ClientMessage Original;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Original);
 FGrpcNightfallV1ItemOperationResult Receipt;Receipt.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);Receipt.Incarnation=1;Receipt.Tick=11;Receipt.InventoryRevision=2;Receipt.ReceiptId=Key;Receipt.Operation=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_DESTROY;
 FWorldEvent Event;Event.ItemOperationResult=Receipt;S->ApplyReceiptPayload(Event);TestFalse(TEXT("original receipt commits before conflict"),S->IsBusy());
 for(int32 Attempt=0;Attempt<2;++Attempt){
  TestTrue(TEXT("later different body sent for server refusal"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("2"),Key}));const uint32 Conflict=N->GetLastSentSeq();
  FGrpcNightfallV1ClientMessage Different;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Different);TestEqual(TEXT("later body differs on wire"),Different.Intent.DestroyItem.Count.Value,uint64(2));
  S->ApplyReceiptPayload(Event);TestTrue(TEXT("committed replay cannot complete conflicting body"),S->IsBusy());
  FIntentRejected Rejected;Rejected.Seq=Conflict;Rejected.Detail=TEXT("D43 fingerprint conflict: FAILED_PRECONDITION");S->ApplyRejected(Rejected);TestFalse(TEXT("later conflict rejection clears busy"),S->IsBusy());TestEqual(TEXT("later conflict refusal displayed"),S->Status(),Rejected.Detail);
  S->ApplyReceiptPayload(Event);TestEqual(TEXT("receipt replay preserves later refusal"),S->Status(),Rejected.Detail);
 }
 TestTrue(TEXT("original committed body still retryable"),S->Execute({TEXT("nf.Destroy"),TEXT("instance:1"),TEXT("1"),Key}));FGrpcNightfallV1ClientMessage Retry;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Retry);TestEqual(TEXT("committed body remains frozen"),Retry.Intent.DestroyItem.ToJsonString(false),Original.Intent.DestroyItem.ToJsonString(false));
 S->ApplyReceiptPayload(Event);TestFalse(TEXT("original retry completes"),S->IsBusy());TestEqual(TEXT("original retry success displayed"),S->Status(),FString(TEXT("Server committed item operation")));N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemGrantDetachedOverlapTest,"Nightfall.Item.Rpc.GrantDetachedOverlapAndLifecycle",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemGrantDetachedOverlapTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto* Session=Scope.Get<USessionClient>();ItemUiTest::Seed(N,S);
 USessionClient::FGrantItemCallback Grant;USessionClient::FGetInventoryCallback Detached;
 FGrpcNightfallV1GrantItemRequest SubmittedGrant;FGrpcNightfallV1GetInventoryRequest SubmittedRead;
 S->SetGrantProviderForTesting([&](const auto& Q,auto Callback){SubmittedGrant=Q;Grant=MoveTemp(Callback);});S->SetInventoryProviderForTesting([&](const auto& Q,auto Callback){SubmittedRead=Q;Detached=MoveTemp(Callback);});
 const FString Other=TEXT("22222222-2222-4222-8222-222222222222"),Key=TEXT("abcdefab-1111-4111-8111-123456789abc");
 FGrpcNightfallV1GetInventoryResponse View;View.HasInventory=true;View.Inventory.EntityId=NightfallItems::UuidBytes(Other);auto Item=ItemUiTest::Item(9,6);Item.Count=7;View.Inventory.Items.Add(Item);
 FNetResult GrantResult;GrantResult.Error=ENetError::FailedPrecondition;GrantResult.Message=TEXT("grant completion observed");
 int32 Observed=0;const auto Handle=S->OnChanged.AddLambda([&]{if(S->RpcDone() && S->RpcResult().Message==GrantResult.Message)++Observed;});
 for(bool GrantFirst:{true,false}){
  S->Grant(6,3,Key);S->LoadDetached(Other);TestTrue(TEXT("grant stays busy during detached read"),S->IsBusy());TestTrue(TEXT("detached pending starts empty"),S->Detached(Other)==nullptr);
  TestEqual(TEXT("grant bound to owner"),NightfallItems::UuidString(SubmittedGrant.CharacterId),FString(ItemUiTest::Owner));TestEqual(TEXT("grant requested count"),SubmittedGrant.Count.Value,uint64(3));TestEqual(TEXT("read bound to other character"),NightfallItems::UuidString(SubmittedRead.CharacterId),Other);
  const int32 Before=Observed;
  if(GrantFirst){Grant(GrantResult,{});TestFalse(TEXT("grant-first busy cleared"),S->IsBusy());TestTrue(TEXT("grant-first completion observable"),S->RpcDone() && S->RpcResult().Error==ENetError::FailedPrecondition);Detached({},View);}
  else {Detached({},View);TestTrue(TEXT("read-first still waits for grant"),S->IsBusy());Grant(GrantResult,{});TestTrue(TEXT("read-first completion observable"),S->RpcDone() && S->RpcResult().Error==ENetError::FailedPrecondition);}
  TestEqual(TEXT("grant completion broadcast in either order"),Observed,Before+1);TestFalse(TEXT("both complete busy cleared"),S->IsBusy());
  const auto* Held=S->Detached(Other);TestTrue(TEXT("detached response remains bound"),Held && NightfallItems::UuidString(Held->EntityId)==Other && Held->Items[0].Count.Value==7);TestTrue(TEXT("read never populates owner's detached cache"),S->Detached(ItemUiTest::Owner)==nullptr);TestEqual(TEXT("detached never changes live owner inventory"),*S->Count(6),uint64(1));
 }
 for(bool ChangeSession:{false,true}){
  S->Grant(6,3,Key);auto Old=MoveTemp(Grant);
  if(ChangeSession)Session->SetBearerToken(TEXT("test:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"));else S->Reset();
  S->Grant(6,3,Key);const int32 Before=Observed;Old(GrantResult,{});TestTrue(TEXT("stale grant cannot clear newer grant busy"),S->IsBusy());TestEqual(TEXT("stale grant is not observable"),Observed,Before);
  Grant(GrantResult,{});TestFalse(TEXT("current grant completes after lifecycle invalidation"),S->IsBusy());TestEqual(TEXT("current grant observable"),Observed,Before+1);
 }
 S->OnChanged.Remove(Handle);return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemDetachedDialogsTest,"Nightfall.Item.UI.DetachedCharacterSessionPendingFailureAndDialogs",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemDetachedDialogsTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();ItemUiTest::Seed(N,S);auto* Session=Scope.Get<USessionClient>();
 TMap<FString,USessionClient::FGetInventoryCallback> Pending;S->SetInventoryProviderForTesting([&](const auto& Q,auto Callback){Pending.Add(NightfallItems::UuidString(Q.CharacterId),MoveTemp(Callback));});
 const FString A=ItemUiTest::Owner,B=TEXT("22222222-2222-4222-8222-222222222222");
 auto Response=[&](const FString& Id,uint64 Count){FGrpcNightfallV1GetInventoryResponse R;R.HasInventory=true;R.Inventory.EntityId=NightfallItems::UuidBytes(Id);auto I=ItemUiTest::Item(1,6);I.Count=Count;R.Inventory.Items.Add(I);return R;};
 auto* DA=NewObject<UNightfallInventoryDialog>(Scope.GameInstance);DA->BindState(S,false);DA->ShowDetached(A);TestEqual(TEXT("loading starts empty"),DA->GridControl()->GetChildrenCount(),0);Pending[A]({},Response(A,7));TestEqual(TEXT("A response renders"),DA->GridControl()->GetChildrenCount(),1);TestFalse(TEXT("detached cannot send"),DA->EquipControl()->GetIsEnabled());
 DA->ShowDetached(B);TestEqual(TEXT("A to B pending clears A"),DA->GridControl()->GetChildrenCount(),0);FNetResult Failed;Failed.Error=ENetError::FailedPrecondition;Pending[B](Failed,{});TestEqual(TEXT("failed B remains empty"),DA->GridControl()->GetChildrenCount(),0);
 auto* DB=NewObject<UNightfallInventoryDialog>(Scope.GameInstance);DB->BindState(S,false);DA->ShowDetached(A);DB->ShowDetached(B);Pending[B]({},Response(B,8));TestEqual(TEXT("A pending cannot show B"),DA->GridControl()->GetChildrenCount(),0);Pending[A]({},Response(A,7));TestEqual(TEXT("simultaneous A bound count"),S->Detached(A)->Items[0].Count.Value,uint64(7));TestEqual(TEXT("simultaneous B bound count"),S->Detached(B)->Items[0].Count.Value,uint64(8));TestEqual(TEXT("both dialogs render matching response"),DB->GridControl()->GetChildrenCount(),1);
 DB->ShowDetached(B);Pending[B]({},Response(A,99));TestEqual(TEXT("wrong entity rejected"),DB->GridControl()->GetChildrenCount(),0);auto Bad=Response(B,8);Bad.Inventory.Tick=1;DB->ShowDetached(B);Pending[B]({},Bad);TestEqual(TEXT("nonzero D36 sentinel rejected"),DB->GridControl()->GetChildrenCount(),0);
 DB->ShowDetached(B);Failed.Error=ENetError::Unavailable;Pending[B](Failed,{});TestEqual(TEXT("unavailable view empty"),DB->GridControl()->GetChildrenCount(),0);
 DA->ShowDetached(A);auto Old=MoveTemp(Pending[A]);Session->SetBearerToken(TEXT("test:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"));Old({},Response(A,99));TestEqual(TEXT("old session callback ignored"),DA->GridControl()->GetChildrenCount(),0);TestTrue(TEXT("all old session views invalidated"),S->Detached(A)==nullptr && S->Detached(B)==nullptr);return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemFreshActionKeyTest,"Nightfall.Item.UI.UnequipEquipUnequipFreshKeys",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemFreshActionKeyTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 auto Change=[&](bool Equipped,uint64 Revision){FGrpcNightfallV1InventoryChanged C;C.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);C.Incarnation=1;C.Tick=10+Revision;C.PreviousRevision=Revision-1;C.InventoryRevision=Revision;auto I=ItemUiTest::Item(1,6);if(Equipped){I.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND;I.Location=EGrpcNightfallV1ItemLocation::ITEM_LOCATION_PAPERDOLL;}C.Updated.Add(I);S->ApplyChanged(C);};Change(true,2);
 auto* D=NewObject<UNightfallInventoryDialog>(Scope.GameInstance);D->BindState(S,false);D->SelectSlot(TEXT("rhand"));TArray<FString> Keys;
 for(int32 Step=0;Step<3;++Step){auto* Button=Step==1?D->EquipControl():D->UnequipControl();TestTrue(TEXT("sequence button enabled"),Button->GetIsEnabled());Button->OnClicked.Broadcast();FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);const FString Key=Step==1?M.Intent.EquipItem.IdempotencyKey:M.Intent.UnequipItem.IdempotencyKey;Keys.Add(Key);TestFalse(TEXT("waits for authoritative completion"),Button->GetIsEnabled());Change(Step==1,3+Step);
  FGrpcNightfallV1ItemOperationResult R;R.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);R.Incarnation=1;R.Tick=13+Step;R.InventoryRevision=3+Step;R.ReceiptId=Key;R.Operation=Step==1?EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EQUIP:EGrpcNightfallV1ItemOperation::ITEM_OPERATION_UNEQUIP;FWorldEvent E;E.ItemOperationResult=R;S->ApplyReceiptPayload(E);}
 TestTrue(TEXT("two successful unequips are new operations"),Keys[0]!=Keys[2]);TestEqual(TEXT("all three clicks sent"),Socket->Sent.Num(),3);N->Disconnect();return true;
}
#endif
#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemAllOperationControlsTest,"Nightfall.Item.UI.AllOperationControlsSendCompleteReject",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemAllOperationControlsTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 FEntitySpawn Npc;Npc.EntityId=TEXT("22222222-2222-4222-8222-222222222222");Npc.Kind=2;Npc.Name=TEXT("Blacksmith");Npc.LifeIncarnation=7;Npc.StateTick=10;S->ApplySpawn(Npc);
 auto Catalog=S->GetCatalog();FGrpcNightfallV1SaRecipe Recipe;Recipe.RecipeId=MAX_uint64;Recipe.BaseTemplate=6;Recipe.ProductTemplate=6;Catalog.Recipes.Add(Recipe);S->ApplyCatalog(Catalog);
 for(const FString Action:{TEXT("Enchant"),TEXT("Crystallize"),TEXT("Refine"),TEXT("CancelRefine"),TEXT("ExchangeSA")}){
  const bool Blacksmith=Action==TEXT("Refine") || Action==TEXT("CancelRefine") || Action==TEXT("ExchangeSA");auto* D=NewObject<UNightfallItemOperationDialog>(Scope.GameInstance);D->BindState(S,Blacksmith,1);D->ActionControl()->SetSelectedOption(Action);D->IngredientControl()->SetSelectedIndex(Action==TEXT("ExchangeSA")?0:1);
  TestTrue(Action+TEXT(" confirm enabled"),D->CanConfirm());const int32 Before=Socket->Sent.Num();D->ConfirmControl()->OnClicked.Broadcast();TestEqual(Action+TEXT(" button sends"),Socket->Sent.Num(),Before+1);FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);
  const FString Key=D->RetryKey();EGrpcNightfallV1ItemOperation Op=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_UNSPECIFIED;
  if(Action==TEXT("Enchant")){TestEqual(TEXT("enchant target"),M.Intent.EnchantItem.TargetInstanceId.Value,uint64(1));TestEqual(TEXT("enchant scroll"),M.Intent.EnchantItem.ScrollInstanceId.Value,uint64(2));Op=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_ENCHANT;}
  if(Action==TEXT("Crystallize")){TestTrue(TEXT("crystallize case"),M.Intent.IntentCase==EGrpcNightfallV1ClientMessageIntent::CrystallizeItem);TestEqual(TEXT("crystallize target"),M.Intent.CrystallizeItem.InstanceId.Value,uint64(1));Op=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_CRYSTALLIZE;}
  if(Action==TEXT("Refine")){TestEqual(TEXT("refine target"),M.Intent.RefineItem.TargetInstanceId.Value,uint64(1));TestEqual(TEXT("refine stone"),M.Intent.RefineItem.StoneInstanceId.Value,uint64(2));TestEqual(TEXT("refine npc incarnation"),M.Intent.RefineItem.Npc.Incarnation.Value,uint32(7));TestTrue(TEXT("refine npc identity"),M.Intent.RefineItem.Npc.EntityId.Value==NightfallItems::UuidBytes(Npc.EntityId).Value);Op=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_REFINE;}
  if(Action==TEXT("CancelRefine")){TestEqual(TEXT("cancel target"),M.Intent.CancelRefine.TargetInstanceId.Value,uint64(1));TestEqual(TEXT("cancel npc incarnation"),M.Intent.CancelRefine.Npc.Incarnation.Value,uint32(7));TestTrue(TEXT("cancel npc identity"),M.Intent.CancelRefine.Npc.EntityId.Value==NightfallItems::UuidBytes(Npc.EntityId).Value);Op=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_CANCEL_REFINE;}
  if(Action==TEXT("ExchangeSA")){TestEqual(TEXT("SA target"),M.Intent.ExchangeSpecialAbility.TargetInstanceId.Value,uint64(1));TestEqual(TEXT("SA recipe width"),M.Intent.ExchangeSpecialAbility.RecipeId.Value,uint64(MAX_uint64));TestEqual(TEXT("SA npc incarnation"),M.Intent.ExchangeSpecialAbility.Npc.Incarnation.Value,uint32(7));TestTrue(TEXT("SA npc identity"),M.Intent.ExchangeSpecialAbility.Npc.EntityId.Value==NightfallItems::UuidBytes(Npc.EntityId).Value);Op=EGrpcNightfallV1ItemOperation::ITEM_OPERATION_EXCHANGE_SA;}
  TestFalse(TEXT("pending disables confirm"),D->ConfirmControl()->GetIsEnabled());FWorldEvent E;
  if(Op==EGrpcNightfallV1ItemOperation::ITEM_OPERATION_ENCHANT){FGrpcNightfallV1EnchantResult R;R.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);R.Incarnation=1;R.Tick=11;R.InventoryRevision=1;R.ReceiptId=Key;R.TargetInstanceId=1;R.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SAFE_FAILED;E.EnchantResult=R;}
  else if(Op==EGrpcNightfallV1ItemOperation::ITEM_OPERATION_REFINE || Op==EGrpcNightfallV1ItemOperation::ITEM_OPERATION_CANCEL_REFINE){FGrpcNightfallV1RefineResult R;R.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);R.Incarnation=1;R.Tick=11;R.InventoryRevision=1;R.ReceiptId=Key;R.TargetInstanceId=1;R.Outcome=Op==EGrpcNightfallV1ItemOperation::ITEM_OPERATION_REFINE?EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED:EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_REMOVED;E.RefineResult=R;}
  else {FGrpcNightfallV1ItemOperationResult R;R.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);R.Incarnation=1;R.Tick=11;R.InventoryRevision=1;R.ReceiptId=Key;R.Operation=Op;E.ItemOperationResult=R;}
  FServerMessage Server;Server.Event=E;N->DispatchServerMessage(Server);TestFalse(TEXT("completion clears pending"),S->IsBusy());TestTrue(TEXT("completion enables new action"),D->ConfirmControl()->GetIsEnabled());D->RetryControl()->OnClicked.Broadcast();FGrpcNightfallV1ClientMessage Retry;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Retry);TestEqual(TEXT("explicit retry retains exact original body"),Retry.Intent.ToJsonString(false),M.Intent.ToJsonString(false));FIntentRejected Rejected;Rejected.Seq=N->GetLastSentSeq();Rejected.Detail=Action+TEXT(" refused");S->ApplyRejected(Rejected);TestFalse(TEXT("rejection clears matching pending"),S->IsBusy());TestEqual(TEXT("received rejection displayed"),S->Status(),Rejected.Detail);
  D->ConfirmControl()->OnClicked.Broadcast();TestTrue(TEXT("new confirm allocates fresh UUID"),D->RetryKey()!=Key);Rejected.Seq=N->GetLastSentSeq();S->ApplyRejected(Rejected);
 }
 N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemHudControlsTest,"Nightfall.Item.UI.HudManualAutoShotReceivedStateAndReset",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemHudControlsTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 const FString Config=FPaths::Combine(FPaths::ProjectSavedDir(),TEXT("Tests"),FGuid::NewGuid().ToString()+TEXT("-hud-items.ini"));S->SetHotbarConfigForTesting(Config);S->AssignHotbar(0,6,false);S->AssignHotbar(1,6,true);
 auto* Hud=NewObject<UNightfallHud>(Scope.GameInstance);Hud->EnsureLayout();Hud->RefreshPresentation();TestTrue(TEXT("HUD manual received label"),Hud->ItemLabel(0).Contains(TEXT("Wand")) && Hud->ItemLabel(0).Contains(TEXT("x1")));TestTrue(TEXT("HUD auto received off label"),Hud->ItemLabel(1).Contains(TEXT("AUTO OFF")));TestTrue(TEXT("manual button enabled"),Hud->ItemControl(0)->GetIsEnabled());Hud->ItemControl(0)->OnClicked.Broadcast();FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestEqual(TEXT("HUD manual use instance"),M.Intent.UseItem.InstanceId.Value,uint64(1));FIntentRejected R;R.Seq=N->GetLastSentSeq();S->ApplyRejected(R);
 Hud->ItemControl(1)->OnClicked.Broadcast();NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestEqual(TEXT("HUD auto template"),M.Intent.ToggleAutoShot.TemplateId.Value,uint32(6));TestTrue(TEXT("HUD sends on"),M.Intent.ToggleAutoShot.Enabled);Hud->RefreshPresentation();TestTrue(TEXT("HUD stays off until received"),Hud->ItemLabel(1).Contains(TEXT("AUTO OFF")));
 FGrpcNightfallV1InventoryChanged C;C.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);C.Incarnation=1;C.Tick=11;C.PreviousRevision=1;C.InventoryRevision=2;C.AutoShotTemplates.Add(6);S->ApplyChanged(C);R.Seq=N->GetLastSentSeq();S->ApplyRejected(R);Hud->RefreshPresentation();TestTrue(TEXT("HUD projects received auto on"),Hud->ItemLabel(1).Contains(TEXT("AUTO ON")));Hud->ItemControl(1)->OnClicked.Broadcast();NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestFalse(TEXT("HUD sends off after received on"),M.Intent.ToggleAutoShot.Enabled);
 S->Reset();Hud->RefreshPresentation();TestFalse(TEXT("HUD reset disables manual"),Hud->ItemControl(0)->GetIsEnabled());TestFalse(TEXT("HUD reset disables auto"),Hud->ItemControl(1)->GetIsEnabled());IFileManager::Get().Delete(*Config);N->Disconnect();return true;
}
#endif
