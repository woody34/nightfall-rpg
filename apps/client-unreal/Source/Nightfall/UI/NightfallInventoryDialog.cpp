#include "NightfallInventoryDialog.h"
#include "NightfallItemOperationDialog.h"
#include "Items/InventoryStateSubsystem.h"
#include "Blueprint/WidgetTree.h"
#include "Components/VerticalBox.h"
#include "Components/HorizontalBox.h"
#include "Components/UniformGridPanel.h"
#include "Components/SizeBox.h"
#include "Components/ScrollBox.h"
#include "Components/ComboBoxString.h"
#include "Components/TextBlock.h"
#include "Components/Button.h"
#include "Components/ProgressBar.h"
#include "Engine/GameInstance.h"
#include "Engine/Texture2D.h"
#include "Components/Image.h"
#include "Misc/PackageName.h"
void UNightfallItemChoice::Click(){if(Dialog.IsValid()){if(Slot.IsEmpty())Dialog->Select(Id);else Dialog->SelectSlot(Slot);}}
void UNightfallInventoryDialog::EnsureLayout()
{
 if(!WidgetTree)WidgetTree=NewObject<UWidgetTree>(this);if(WidgetTree->RootWidget)return;
 auto* Column=WidgetTree->ConstructWidget<UVerticalBox>();WidgetTree->RootWidget=Column;
 auto Text=[&](const TCHAR* S){auto* T=WidgetTree->ConstructWidget<UTextBlock>();T->SetText(FText::FromString(S));T->SetAutoWrapText(true);Column->AddChild(T);return T;};
 auto Button=[&](const TCHAR* S){auto* B=WidgetTree->ConstructWidget<UButton>();auto* T=WidgetTree->ConstructWidget<UTextBlock>();T->SetText(FText::FromString(S));B->AddChild(T);Column->AddChild(B);return B;};
 Text(TEXT("Inventory and equipment"));Summary=Text(TEXT("Waiting for inventory"));Weight=WidgetTree->ConstructWidget<UProgressBar>();Column->AddChild(Weight);Status=Text(TEXT(""));
 auto* Size=WidgetTree->ConstructWidget<USizeBox>();Size->SetHeightOverride(260);Column->AddChild(Size);auto* Scroll=WidgetTree->ConstructWidget<UScrollBox>();Size->AddChild(Scroll);Grid=WidgetTree->ConstructWidget<UUniformGridPanel>();Scroll->AddChild(Grid);
 auto* DollSize=WidgetTree->ConstructWidget<USizeBox>();DollSize->SetHeightOverride(160);Column->AddChild(DollSize);auto* DollScroll=WidgetTree->ConstructWidget<UScrollBox>();DollSize->AddChild(DollScroll);Paperdoll=WidgetTree->ConstructWidget<UVerticalBox>();DollScroll->AddChild(Paperdoll);
 Selection=Text(TEXT("Select an item or equipment slot"));EquipButton=Button(TEXT("Equip selected"));EquipButton->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::Equip);UnequipButton=Button(TEXT("Unequip selected slot"));UnequipButton->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::Unequip);UseButton=Button(TEXT("Use selected"));UseButton->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::Use);
 EnchantButton=Button(TEXT("Enchant / crystallize"));EnchantButton->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::Enchant);BlacksmithButton=Button(TEXT("Blacksmith: augment / cancel / special ability"));BlacksmithButton->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::Blacksmith);
 Text(TEXT("Item hotbar (F1–F9)"));SlotPicker=WidgetTree->ConstructWidget<UComboBoxString>();Column->AddChild(SlotPicker);for(int32 I=1;I<=9;++I)SlotPicker->AddOption(FString::FromInt(I));SlotPicker->SetSelectedIndex(0);
 ModePicker=WidgetTree->ConstructWidget<UComboBoxString>();Column->AddChild(ModePicker);ModePicker->AddOption(TEXT("Manual use"));ModePicker->AddOption(TEXT("Auto-shot toggle"));ModePicker->SetSelectedIndex(0);
 Button(TEXT("Assign selected item"))->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::Assign);Button(TEXT("Clear hotbar slot"))->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::ClearSlot);
 Button(TEXT("Refresh catalogue / detached inventory"))->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::Refresh);Button(TEXT("Close"))->OnClicked.AddDynamic(this,&UNightfallInventoryDialog::Close);
}
void UNightfallInventoryDialog::NativeOnInitialized(){Super::NativeOnInitialized();EnsureLayout();BindState(GetGameInstance()?GetGameInstance()->GetSubsystem<UInventoryStateSubsystem>():nullptr);}
void UNightfallInventoryDialog::NativeDestruct(){if(State)State->OnChanged.Remove(Changed);Super::NativeDestruct();}
TOptional<FUIInputConfig> UNightfallInventoryDialog::GetDesiredInputConfig() const{return FUIInputConfig(ECommonInputMode::Menu,EMouseCaptureMode::NoCapture);}
void UNightfallInventoryDialog::BindState(UInventoryStateSubsystem* S,bool RefreshState){EnsureLayout();if(State)State->OnChanged.Remove(Changed);State=S;if(State)Changed=State->OnChanged.AddUObject(this,&UNightfallInventoryDialog::Render);if(RefreshState)Refresh();else Render();}
void UNightfallInventoryDialog::ShowDetached(const FString& Character){DetachedCharacter=Character;Selected=0;SelectedSlot.Reset();Key.Reset();if(State)State->LoadDetached(Character);Render();}
FString UNightfallInventoryDialog::Tooltip(uint32 Template) const
{
 const auto* D=State?State->Definition(Template):nullptr;if(!D)return TEXT("Item catalogue unavailable");
 FString T=FString::Printf(TEXT("%s\nGrade %u • Weight %u\n%s"),*D->Name,uint8(D->Grade),D->Weight.Value,D->Usable?TEXT("Server validates use"):*FString::Join(D->Blockers,TEXT(", ")));
 for(const auto& S:D->Stats)T+=FString::Printf(TEXT("\n%s %s %lld/%lld (order %u)"),*S.Stat,*S.Operation,S.Numerator,S.Denominator,S.Order.Value);
 for(const auto& S:D->Skills)T+=FString::Printf(TEXT("\nSkill %u rank %u"),S.Id.Value,S.Rank.Value);return T;
}
FString UNightfallInventoryDialog::ItemLabel(uint64 Id) const
{
 const auto* I=State?State->Find(Id):nullptr;if(!I)return {};const auto* D=State->Definition(I->TemplateId);
 return FString::Printf(TEXT("%s\nx%llu  +%u%s"),D?*D->Name:*FString::Printf(TEXT("Item %u"),I->TemplateId.Value),I->Count.Value,I->Enchant.Value,I->_augment_id._augment_idCase==EGrpcNightfallV1OwnedItem_augment_id::AugmentId?TEXT("  ◆ augment"):TEXT(""));
}
void UNightfallInventoryDialog::Render()
{
 if(!Grid || !State)return;Grid->ClearChildren();Paperdoll->ClearChildren();Choices.Reset();const bool Detached=!DetachedCharacter.IsEmpty();const bool Live=!Detached && State->HasBaseline();
 const auto* View=Detached?State->Detached(DetachedCharacter):Live?&State->GetInventory():nullptr;
 Status->SetText(FText::FromString(State->Status()));
 if(View){Summary->SetText(FText::FromString(FString::Printf(TEXT("%s • %d items / %u ordinary + %u quest\nWeight %lld / %lld (%lld/10%%) • overload %u • grade weapon %u / armor %u"),Detached?TEXT("Out of world"):TEXT("Live inventory"),View->Items.Num(),View->OrdinaryCapacity.Value,View->QuestCapacity.Value,View->Weight.Weight,View->Weight.MaxLoad,View->Weight.PerMille,View->Penalties.OverloadLevel.Value,View->Penalties.WeaponLevel.Value,View->Penalties.ArmorLevel.Value)));Weight->SetPercent(FMath::Clamp(float(View->Weight.PerMille)/1000.f,0.f,1.f));Weight->SetFillColorAndOpacity(View->Penalties.Overloaded?FLinearColor::Red:FLinearColor(0.2f,0.7f,0.3f));}
 else {Summary->SetText(FText::FromString(TEXT("Inventory state unknown")));Weight->SetPercent(0);}
 auto Choice=[&](const FString& Label,uint64 Id,const FString& Slot){auto* B=WidgetTree->ConstructWidget<UButton>();auto* L=WidgetTree->ConstructWidget<UTextBlock>();L->SetText(FText::FromString(Label));L->SetAutoWrapText(true);B->AddChild(L);auto* H=NewObject<UNightfallItemChoice>(this);H->Id=Id;H->Slot=Slot;H->Dialog=this;Choices.Add(H);B->OnClicked.AddDynamic(H,&UNightfallItemChoice::Click);B->SetIsEnabled(Live);return B;};
 if(View){int32 Index=0;for(const auto& I:View->Items){const auto* D=State->Definition(I.TemplateId);const FString Label=Live?ItemLabel(I.InstanceId):FString::Printf(TEXT("%s x%llu +%u"),D?*D->Name:TEXT("Item"),I.Count.Value,I.Enchant.Value);auto* B=Choice(Label,I.InstanceId,{});
 if(D && FPackageName::DoesPackageExist(TEXT("/Game/Items/T_ItemPlaceholderAtlas")))if(auto* Atlas=LoadObject<UTexture2D>(nullptr,TEXT("/Game/Items/T_ItemPlaceholderAtlas.T_ItemPlaceholderAtlas"))){auto* Cell=WidgetTree->ConstructWidget<UVerticalBox>();auto* LabelWidget=B->GetChildAt(0);B->RemoveChild(LabelWidget);auto* Icon=WidgetTree->ConstructWidget<UImage>();FSlateBrush Brush;Brush.SetResourceObject(Atlas);Brush.ImageSize=FVector2D(48,48);const int32 Kind=FMath::Clamp(int32(D->Kind)-1,0,2);Brush.SetUVRegion(FBox2D(FVector2D(Kind/3.f,0),FVector2D((Kind+1)/3.f,1)));Icon->SetBrush(Brush);Cell->AddChild(Icon);Cell->AddChild(LabelWidget);B->AddChild(Cell);}
 B->SetToolTipText(FText::FromString(Tooltip(I.TemplateId)));Grid->AddChildToUniformGrid(B,Index/5,Index%5);++Index;}}
 for(uint8 I=1;I<=25;++I){const auto Slot=EGrpcNightfallV1EquipSlot(I);const auto* Held=Live?State->Equipped(Slot):nullptr;if(Detached && View)for(const auto& Item:View->Items)if(Item.Slot==Slot){Held=&Item;break;}const FString Name=NightfallItems::SlotName(Slot);Paperdoll->AddChild(Choice(Name+TEXT(": ")+(Held?(Live?ItemLabel(Held->InstanceId):FString::Printf(TEXT("Item %u +%u"),Held->TemplateId.Value,Held->Enchant.Value)):TEXT("empty")),Held?Held->InstanceId.Value:0,Name));}
 const auto* Held=Live?State->Find(Selected):nullptr;const auto* Def=Held?State->Definition(Held->TemplateId):nullptr;const bool Ready=Live && !State->IsBusy();
 Selection->SetText(FText::FromString(Held?ItemLabel(Selected):TEXT("Select a current item")));EquipButton->SetIsEnabled(Ready && Def && Def->Kind!=EGrpcNightfallV1ItemKind::ITEM_KIND_ETC);UseButton->SetIsEnabled(Ready && Def && Def->Usable);UnequipButton->SetIsEnabled(Ready && !SelectedSlot.IsEmpty() && State->Equipped(NightfallItems::Slot(SelectedSlot)));EnchantButton->SetIsEnabled(Ready && State->HasCatalog());BlacksmithButton->SetIsEnabled(Ready && State->HasCatalog() && !State->Blacksmith().IsEmpty());
}
void UNightfallInventoryDialog::Select(uint64 Id){if(Selected!=Id){Key.Reset();Selected=Id;}SelectedSlot.Reset();Render();}
void UNightfallInventoryDialog::SelectSlot(const FString& Slot){if(SelectedSlot!=Slot)Key.Reset();SelectedSlot=Slot;const auto* I=State?State->Equipped(NightfallItems::Slot(Slot)):nullptr;Selected=I?I->InstanceId.Value:0;Render();}
void UNightfallInventoryDialog::Equip(){if(!State || !EquipButton->GetIsEnabled())return;Key=FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);State->Execute({TEXT("nf.Equip"),FString::Printf(TEXT("instance:%llu"),Selected),Key});}
void UNightfallInventoryDialog::Unequip(){if(!State || !UnequipButton->GetIsEnabled())return;Key=FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);State->Execute({TEXT("nf.Unequip"),SelectedSlot,Key});}
void UNightfallInventoryDialog::Use(){if(!State || !UseButton->GetIsEnabled())return;State->Execute({TEXT("nf.UseItem"),FString::Printf(TEXT("instance:%llu"),Selected)});}
void UNightfallInventoryDialog::Enchant(){if(!EnchantButton || !EnchantButton->GetIsEnabled())return;auto* D=CreateWidget<UNightfallItemOperationDialog>(GetOwningPlayer());if(D){D->BindState(State,false,Selected);D->AddToViewport(60);D->ActivateWidget();}}
void UNightfallInventoryDialog::Blacksmith(){if(!BlacksmithButton || !BlacksmithButton->GetIsEnabled())return;auto* D=CreateWidget<UNightfallItemOperationDialog>(GetOwningPlayer());if(D){D->BindState(State,true,Selected);D->AddToViewport(60);D->ActivateWidget();}}
void UNightfallInventoryDialog::Assign(){if(!DetachedCharacter.IsEmpty())return;const auto* I=State?State->Find(Selected):nullptr;if(I)State->AssignHotbar(SlotPicker->GetSelectedIndex(),I->TemplateId,ModePicker->GetSelectedIndex()==1);}
void UNightfallInventoryDialog::ClearSlot(){if(!DetachedCharacter.IsEmpty())return;if(State)State->AssignHotbar(SlotPicker->GetSelectedIndex(),0,false);}
void UNightfallInventoryDialog::Refresh(){if(State){State->LoadCatalog();if(!DetachedCharacter.IsEmpty())State->LoadDetached(DetachedCharacter);}Render();}
void UNightfallInventoryDialog::Close(){DeactivateWidget();RemoveFromParent();}
