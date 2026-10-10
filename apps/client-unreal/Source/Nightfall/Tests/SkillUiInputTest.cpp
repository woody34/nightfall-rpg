#include "Misc/AutomationTest.h"
#include "Tests/TestGameInstance.h"
#include "Skills/SkillStateSubsystem.h"
#include "Effects/EffectStateSubsystem.h"
#include "Net/NetClientSubsystem.h"
#include "NightfallPlayerController.h"
#include "NightfallWire.h"
#include "UI/NightfallHud.h"
#include "Components/InputComponent.h"
#include "Components/Button.h"
#include "InputCoreTypes.h"
#include "IWebSocket.h"
#if WITH_DEV_AUTOMATION_TESTS
namespace SkillUiTests
{
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
 constexpr auto Flags=EAutomationTestFlags::EditorContext|EAutomationTestFlags::ProductFilter;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillCatalogueAdmissionTest,"Nightfall.Skill.DelayedCatalogueAdmission",SkillUiTests::Flags)
bool FSkillCatalogueAdmissionTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();N->SetOwnEntityId(TEXT("owner"));
 USessionClient::FSkillCatalogCallback Pending;S->SetCatalogProviderForTesting([&](auto Callback){Pending=MoveTemp(Callback);});
 int32 Completed=0;S->LoadCatalog([&](const auto&){++Completed;});TestTrue(TEXT("catalogue pending"),S->IsBusy());
 FEntitySpawn Spawn;Spawn.EntityId=TEXT("owner");Spawn.LifeIncarnation=1;Spawn.StateTick=10;S->ApplySpawn(Spawn);
 TestTrue(TEXT("owner admission cannot clear catalogue busy state"),S->IsBusy());
 FGrpcNightfallV1GetSkillCatalogResponse Catalog;auto& Row=Catalog.Skills.AddDefaulted_GetRef();Row.SkillId=3;Row.EffectImplemented=true;
 Pending({},Catalog);TestTrue(TEXT("delayed successful catalogue retained after admission"),S->HasCatalog());TestNotNull(TEXT("received skill visible"),S->FindSkill(3));TestEqual(TEXT("callback completes once"),Completed,1);TestFalse(TEXT("catalogue completed"),S->IsBusy());
 S->LoadCatalog([&](const auto&){++Completed;});auto Old=MoveTemp(Pending);S->Reset();S->LoadCatalog([&](const auto&){++Completed;});
 Old({},Catalog);TestTrue(TEXT("old response cannot clear newer request"),S->IsBusy());Pending({},Catalog);TestFalse(TEXT("new request completes after reset"),S->IsBusy());TestEqual(TEXT("reset discards old completion"),Completed,2);
 S->SetCatalogProviderForTesting(nullptr);return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillCorpseInputTest,"Nightfall.Skill.CorpseClickAndNumberKey",SkillUiTests::Flags)
bool FSkillCorpseInputTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();auto Socket=MakeShared<SkillUiTests::FSocket>();
 N->SetSocketFactoryForTesting([Socket](const auto&)->TSharedRef<IWebSocket>{return Socket;});N->SetSchedulerForTesting([](float,TFunction<void()>){ });N->Connect(TEXT("ws://test"),TEXT("test"));Socket->Connected.Broadcast();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};FWorldEvent E;
 FEntitySpawn Spawn;Spawn.EntityId=TEXT("owner");Spawn.Kind=1;Spawn.LifeIncarnation=1;Spawn.SessionGeneration=1;Spawn.StateTick=10;Spawn.Hp=100;E.Spawn=Spawn;Send(E);
 Spawn.EntityId=TEXT("corpse");Spawn.Hp=0;Spawn.bDead=true;E.Spawn=Spawn;Send(E);
 FSkillStateSync Sync;Sync.Entity=TEXT("owner");Sync.Incarnation=1;Sync.Tick=Sync.BaselineTick=10;Sync.bBaseline=true;Sync.Learned.Add({1016,1});E={};E.SkillStateSync=Sync;Send(E);
 FGrpcNightfallV1GetSkillCatalogResponse Catalog;auto& Row=Catalog.Skills.AddDefaulted_GetRef();Row.SkillId=1016;Row.EffectImplemented=true;S->ApplyCatalog(Catalog,{{1016,TEXT("A1")}});
 auto* PC=Scope.GameInstance->GetWorld()->SpawnActor<ANightfallPlayerController>();if(!TestNotNull(TEXT("controller"),PC))return false;
 PC->InputComponent=NewObject<UInputComponent>(PC);PC->BindSkillKeys();
 TestTrue(TEXT("mouse proxy resolver accepts PC_BODY"),PC->ClickEntityProxy(TEXT("corpse")));TestEqual(TEXT("corpse click sends no attack/SetTarget/MoveTo"),Socket->Sent.Num(),0);
 auto* Binding=PC->InputComponent->KeyBindings.FindByPredicate([](const auto& B){return B.Chord.Key==EKeys::One && B.KeyEvent==IE_Pressed;});if(!TestNotNull(TEXT("actual key1 delegate installed"),Binding))return false;
 if(!TestTrue(TEXT("key delegate bound"),Binding->KeyDelegate.IsBound()))return false;Binding->KeyDelegate.Execute(EKeys::One);if(!TestEqual(TEXT("key sends one cast"),Socket->Sent.Num(),1))return false;
 FGrpcNightfallV1ClientMessage Cast;if(!TestTrue(TEXT("outgoing cast decode"),NightfallWire::DecodeClientMessage(Socket->Sent.Last().GetData(),Socket->Sent.Last().Num(),Cast)))return false;
 TestEqual(TEXT("key1 casts received resurrection skill"),Cast.Intent.CastSkill->SkillId.Value,1016u);TestEqual(TEXT("explicit PC_BODY UUID"),Cast.Intent.CastSkill->_target.Target,FString(TEXT("corpse")));
 FEntityDespawn Despawn;Despawn.EntityId=TEXT("corpse");E={};E.Despawn=Despawn;Send(E);TestFalse(TEXT("despawn clears local corpse"),S->CorpseTarget().IsSet());
 Spawn.StateTick=12;Spawn.LifeIncarnation=2;E={};E.Spawn=Spawn;Send(E);PC->ClickEntityProxy(TEXT("corpse"));TestTrue(TEXT("new corpse identity selectable"),S->CorpseTarget().IsSet());
 FEntityRespawned Respawn;Respawn.Entity=TEXT("corpse");Respawn.Incarnation=3;Respawn.Tick=13;E={};E.EntityRespawned=Respawn;Send(E);TestFalse(TEXT("revival invalidates corpse selection"),S->CorpseTarget().IsSet());
 Spawn.StateTick=14;Spawn.LifeIncarnation=4;E={};E.Spawn=Spawn;Send(E);PC->ClickEntityProxy(TEXT("corpse"));Spawn.SessionGeneration=2;Spawn.StateTick=15;E.Spawn=Spawn;Send(E);TestFalse(TEXT("replacement generation invalidates selection"),S->CorpseTarget().IsSet());
 N->Disconnect();return true;
}
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FSkillHudProjectionTest,"Nightfall.Skill.HudProgressCountdownConsent",SkillUiTests::Flags)
bool FSkillHudProjectionTest::RunTest(const FString& Parameters)
{
 FScopedTestGameInstance Scope;auto* N=Scope.Get<UNetClientSubsystem>();auto* S=Scope.Get<USkillStateSubsystem>();auto Socket=MakeShared<SkillUiTests::FSocket>();
 N->SetSocketFactoryForTesting([Socket](const auto&)->TSharedRef<IWebSocket>{return Socket;});N->SetSchedulerForTesting([](float,TFunction<void()>){ });N->Connect(TEXT("ws://test"),TEXT("test"));Socket->Connected.Broadcast();N->SetOwnEntityId(TEXT("owner"));
 auto Send=[&](const FWorldEvent& E){FServerMessage M;M.Event=E;N->DispatchServerMessage(M);};FWorldEvent E;
 FEntitySpawn Spawn;Spawn.EntityId=TEXT("owner");Spawn.Kind=1;Spawn.LifeIncarnation=1;Spawn.StateTick=10;Spawn.Hp=Spawn.MaxHp=100;E.Spawn=Spawn;Send(E);
 auto* Hud=CreateWidget<UNightfallHud>(Scope.GameInstance,UNightfallHud::StaticClass());if(!TestNotNull(TEXT("HUD"),Hud))return false;Hud->RefreshPresentation();TestEqual(TEXT("neutral white"),Hud->OwnFlagColor(),FLinearColor::White);
 FSkillCastStarted Cast;Cast.Caster=TEXT("owner");Cast.CasterIncarnation=1;Cast.CastId=1;Cast.SkillId=3;Cast.Tick=Cast.StartTick=11;Cast.CompleteTick=21;E={};E.SkillCastStarted=Cast;Send(E);
 FEffectApplied Effect;Effect.Target=TEXT("owner");Effect.TargetIncarnation=1;Effect.EffectId=1;Effect.SkillId=3;Effect.SkillLevel=1;Effect.Pool=2;Effect.Tick=11;Effect.EndTick=31;E={};E.EffectApplied=Effect;Send(E);Hud->RefreshPresentation();TestEqual(TEXT("cast starts at zero"),Hud->CastProgress(),0.f);TestTrue(TEXT("received pool/deadline shown"),Hud->EffectLabel().Contains(TEXT("Song/dance")) && Hud->EffectLabel().Contains(TEXT("20 ticks")));
 FServerMessage Close;Close.Ack=FAck{1,16};N->DispatchServerMessage(Close);Hud->RefreshPresentation();TestEqual(TEXT("cast progresses on received tick"),Hud->CastProgress(),0.5f);TestTrue(TEXT("effect countdown advances"),Hud->EffectLabel().Contains(TEXT("15 ticks")));
 FPvpStateChanged P;P.Entity=TEXT("owner");P.Incarnation=1;P.Tick=17;P.FlagUntilTick=40;E={};E.PvpStateChanged=P;Send(E);Hud->RefreshPresentation();TestEqual(TEXT("flagged purple"),Hud->OwnFlagColor(),FLinearColor(0.7f,0.25f,1.f));
 P.Tick=18;P.Karma=1;E.PvpStateChanged=P;Send(E);Hud->RefreshPresentation();TestEqual(TEXT("karma red"),Hud->OwnFlagColor(),FLinearColor::Red);
 FSkillCastCancelled Cancel;Cancel.Caster=TEXT("owner");Cancel.CasterIncarnation=1;Cancel.CastId=1;Cancel.Tick=19;Cancel.Reason=2;E={};E.SkillCastCancelled=Cancel;Send(E);Hud->RefreshPresentation();TestEqual(TEXT("cancel clears cast bar"),Hud->CastProgress(),0.f);TestEqual(TEXT("cancel terminal label"),Hud->CastStatus(),FString(TEXT("Cast cancelled")));
 Cast.CastId=2;Cast.StartTick=Cast.Tick=20;Cast.CompleteTick=21;E={};E.SkillCastStarted=Cast;Send(E);FSkillCastResult Result;Result.Caster=TEXT("owner");Result.CasterIncarnation=1;Result.CastId=2;Result.Tick=21;E={};E.SkillCastResult=Result;Send(E);Hud->RefreshPresentation();TestEqual(TEXT("result clears cast bar"),Hud->CastProgress(),0.f);TestEqual(TEXT("result terminal label"),Hud->CastStatus(),FString(TEXT("Cast completed")));
 FEntityDied Death;Death.Entity=TEXT("owner");Death.Incarnation=1;Death.Tick=22;E={};E.EntityDied=Death;Send(E);Hud->RefreshPresentation();TestEqual(TEXT("no offer hides consent"),Hud->AcceptControl()->GetVisibility(),ESlateVisibility::Collapsed);
 FResurrectOffer Offer;Offer.Target=TEXT("owner");Offer.TargetIncarnation=1;Offer.OfferId=7;Offer.State=1;Offer.Tick=23;Offer.ExpiryTick=30;E={};E.ResurrectOffer=Offer;Send(E);Hud->RefreshPresentation();TestEqual(TEXT("received pending offer shows both choices"),Hud->DeclineControl()->GetVisibility(),ESlateVisibility::Visible);
 Hud->AcceptControl()->OnClicked.Broadcast();Hud->DeclineControl()->OnClicked.Broadcast();if(!TestEqual(TEXT("consent sends two explicit responses"),Socket->Sent.Num(),2))return false;
 for(int32 I=0;I<2;++I){FGrpcNightfallV1ClientMessage Response;NightfallWire::DecodeClientMessage(Socket->Sent[I].GetData(),Socket->Sent[I].Num(),Response);TestEqual(TEXT("received offer id"),Response.Intent.AcceptResurrect->OfferId.Value,uint64(7));TestEqual(TEXT("correct consent choice"),Response.Intent.AcceptResurrect->Accept,I==0);}
 Close.Ack=FAck{2,31};N->DispatchServerMessage(Close);Hud->RefreshPresentation();TestEqual(TEXT("expired effect disappears"),Hud->EffectLabel(),FString());TestEqual(TEXT("expired offer hides controls"),Hud->AcceptControl()->GetVisibility(),ESlateVisibility::Collapsed);Hud->AcceptControl()->OnClicked.Broadcast();TestEqual(TEXT("expired callback cannot send consent"),Socket->Sent.Num(),2);
 N->Disconnect();return true;
}
#endif
