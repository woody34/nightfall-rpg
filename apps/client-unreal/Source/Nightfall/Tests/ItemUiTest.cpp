#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Items/InventoryStateSubsystem.h"
#include "Items/EquipmentPresentationComponent.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallWire.h"
#include "NightfallPlayerController.h"
#include "UI/NightfallInventoryDialog.h"
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
  N->SetOwnEntityId(Owner);FEntitySpawn Spawn;Spawn.EntityId=Owner;Spawn.Kind=1;Spawn.StateTick=10;Spawn.LifeIncarnation=1;S->ApplySpawn(Spawn);
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
 D->EquipControl()->OnClicked.Broadcast();TestEqual(TEXT("actual equip button sends one intent"),Socket->Sent.Num(),1);FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestEqual(TEXT("selected instance sent"),M.Intent.EquipItem->InstanceId.Value,uint64(1));TestEqual(TEXT("selection key retained"),M.Intent.EquipItem->IdempotencyKey,D->RetryKey());
 TestFalse(TEXT("pending disables action"),D->EquipControl()->GetIsEnabled());FIntentRejected R;R.Seq=N->GetLastSentSeq();R.Detail=TEXT("item refused");S->ApplyRejected(R);TestTrue(TEXT("rejection restores action"),D->EquipControl()->GetIsEnabled());
 S->Reset();TestEqual(TEXT("disconnect clears owner grid"),D->GridControl()->GetChildrenCount(),0);TestFalse(TEXT("unknown blocks equip"),D->EquipControl()->GetIsEnabled());N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemOperationUiTest,"Nightfall.Item.UI.EnchantCrystallizeBlacksmithRetry",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemOperationUiTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();auto Socket=ItemUiTest::Connect(N);ItemUiTest::Seed(N,S);
 auto* D=NewObject<UNightfallItemOperationDialog>(Scope.GameInstance);D->EnsureLayout();D->BindState(S,false,1);D->IngredientControl()->SetSelectedIndex(1);TestTrue(TEXT("received selections permit enchant"),D->CanConfirm());const FString Key=D->RetryKey();D->ConfirmControl()->OnClicked.Broadcast();TestEqual(TEXT("actual enchant button sends"),Socket->Sent.Num(),1);
 FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestEqual(TEXT("scroll instance"),M.Intent.EnchantItem->ScrollInstanceId.Value,uint64(2));TestEqual(TEXT("target instance"),M.Intent.EnchantItem->TargetInstanceId.Value,uint64(1));TestEqual(TEXT("same retry key"),M.Intent.EnchantItem->IdempotencyKey,Key);
 FGrpcNightfallV1InventoryChanged C;C.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);C.Incarnation=1;C.Tick=11;C.InventoryRevision=2;C.PreviousRevision=1;C.RemovedIds.Add(2);auto I=ItemUiTest::Item(1,6);I.Enchant=1;C.Updated.Add(I);S->ApplyChanged(C);
 FGrpcNightfallV1EnchantResult E;E.EntityId=C.EntityId;E.Incarnation=1;E.Tick=11;E.InventoryRevision=2;E.ReceiptId=Key;E.TargetInstanceId=1;E.NewEnchant=1;E.Outcome=EGrpcNightfallV1EnchantOutcome::ENCHANT_OUTCOME_SUCCESS;FWorldEvent Event;Event.EnchantResult=E;FServerMessage Server;Server.Event=Event;N->DispatchServerMessage(Server);
 TestTrue(TEXT("received outcome coherent"),S->EnchantConsistent());D->ConfirmControl()->OnClicked.Broadcast();TestEqual(TEXT("retry sends after scroll consumed"),Socket->Sent.Num(),2);FGrpcNightfallV1ClientMessage Retry;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Retry);TestEqual(TEXT("frozen exact body"),Retry.Intent.EnchantItem->ToJsonString(false),M.Intent.EnchantItem->ToJsonString(false));
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
 auto* PC=Scope.GameInstance->GetWorld()->SpawnActor<ANightfallPlayerController>();if(!TestNotNull(TEXT("controller"),PC))return false;PC->InputComponent=NewObject<UInputComponent>(PC);PC->BindSkillKeys();auto* B=PC->InputComponent->KeyBindings.FindByPredicate([](const auto& K){return K.Chord.Key==EKeys::F1 && K.KeyEvent==IE_Pressed;});if(!TestNotNull(TEXT("actual F1 key bound"),B))return false;B->KeyDelegate.Execute(EKeys::F1);FGrpcNightfallV1ClientMessage M;NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),M);TestTrue(TEXT("toggle request on"),M.Intent.ToggleAutoShot->Enabled);TestFalse(TEXT("no optimistic toggle"),S->AutoShot(6));
 S->SetHotbarConfigForTesting(Config);TestEqual(TEXT("character hotbar persisted"),S->Hotbar()[0].Key,uint32(6));IFileManager::Get().Delete(*Config);N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FItemPresentationTest,"Nightfall.Item.Presentation.PublicGearEmptyAndGlow",EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter)
bool FItemPresentationTest::RunTest(const FString&)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<UInventoryStateSubsystem>();ItemUiTest::Seed(N,S);
 auto* P=NewObject<UEquipmentPresentationComponent>(Scope.GameInstance);P->Bind(S,ItemUiTest::Owner);TestFalse(TEXT("empty gear hides weapon"),P->WeaponControl()->IsVisible());
 FGrpcNightfallV1EquipmentChanged E;E.EntityId=NightfallItems::UuidBytes(ItemUiTest::Owner);E.Incarnation=1;E.Tick=11;FGrpcNightfallV1VisibleEquipment V;V.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND;V.TemplateId=6;V.GlowTier=1;E.Visible.Add(V);auto Chest=V;Chest.Slot=EGrpcNightfallV1EquipSlot::EQUIP_SLOT_CHEST;Chest.TemplateId=425;E.Visible.Add(Chest);S->ApplyEquipment(E);TestTrue(TEXT("public chest displays armour placeholder"),P->ArmourControl()->IsVisible());TestTrue(TEXT("public template displays mesh"),P->WeaponControl()->IsVisible());TestEqual(TEXT("received glow tier"),P->GlowTier(),uint32(1));TestEqual(TEXT("public template cannot alter private inventory"),*S->Count(6),uint64(1));
 E.Tick=12;E.Visible[0].TemplateId=1;E.Visible[0].GlowTier=2;auto* Previous=P->WeaponControl()->GetStaticMesh();S->ApplyEquipment(E);TestTrue(TEXT("template swaps placeholder mesh"),Previous!=P->WeaponControl()->GetStaticMesh());
 E.Tick=13;E.Visible[0].TemplateId=0;S->ApplyEquipment(E);TestFalse(TEXT("explicit tombstone clears mesh"),P->WeaponControl()->IsVisible());S->ApplyEquipment(E);TestFalse(TEXT("repeat tombstone stable"),P->WeaponControl()->IsVisible());return true;
}
#endif
