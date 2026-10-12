#include "NightfallItemOperationDialog.h"
#include "Items/InventoryStateSubsystem.h"
#include "Blueprint/WidgetTree.h"
#include "Components/VerticalBox.h"
#include "Components/ComboBoxString.h"
#include "Components/TextBlock.h"
#include "Components/Button.h"
void UNightfallItemOperationDialog::EnsureLayout()
{
 if(!WidgetTree)WidgetTree=NewObject<UWidgetTree>(this);if(WidgetTree->RootWidget)return;
 auto* Column=WidgetTree->ConstructWidget<UVerticalBox>();WidgetTree->RootWidget=Column;
 auto Text=[&](const TCHAR* S){auto* T=WidgetTree->ConstructWidget<UTextBlock>();T->SetText(FText::FromString(S));T->SetAutoWrapText(true);Column->AddChild(T);return T;};
 Text(TEXT("Item operation — server chooses the result"));Text(TEXT("Action"));Action=WidgetTree->ConstructWidget<UComboBoxString>();Column->AddChild(Action);
 Text(TEXT("Target item"));Target=WidgetTree->ConstructWidget<UComboBoxString>();Column->AddChild(Target);
 Text(TEXT("Scroll / life stone / special ability recipe"));Ingredient=WidgetTree->ConstructWidget<UComboBoxString>();Column->AddChild(Ingredient);
 Text(TEXT("Optional enchant support"));Support=WidgetTree->ConstructWidget<UComboBoxString>();Column->AddChild(Support);
 for(auto* P:{Action.Get(),Target.Get(),Ingredient.Get(),Support.Get()})P->OnSelectionChanged.AddDynamic(this,&UNightfallItemOperationDialog::SelectionChanged);
 Status=Text(TEXT("Select received items. Ingredients and charges are checked by the server."));
 ConfirmButton=WidgetTree->ConstructWidget<UButton>();auto* Label=WidgetTree->ConstructWidget<UTextBlock>();Label->SetText(FText::FromString(TEXT("Confirm / retry same operation")));ConfirmButton->AddChild(Label);Column->AddChild(ConfirmButton);ConfirmButton->OnClicked.AddDynamic(this,&UNightfallItemOperationDialog::Confirm);
 auto* CloseButton=WidgetTree->ConstructWidget<UButton>();auto* CloseLabel=WidgetTree->ConstructWidget<UTextBlock>();CloseLabel->SetText(FText::FromString(TEXT("Close")));CloseButton->AddChild(CloseLabel);Column->AddChild(CloseButton);CloseButton->OnClicked.AddDynamic(this,&UNightfallItemOperationDialog::Close);
}
void UNightfallItemOperationDialog::BindState(UInventoryStateSubsystem* S,bool Blacksmith,uint64 Selected)
{
 EnsureLayout();if(State)State->OnChanged.Remove(Changed);State=S;bBlacksmith=Blacksmith;bRendering=true;
 Action->ClearOptions();for(const TCHAR* A:Blacksmith?TArray<const TCHAR*>{TEXT("Refine"),TEXT("CancelRefine"),TEXT("ExchangeSA")}:TArray<const TCHAR*>{TEXT("Enchant"),TEXT("Crystallize")})Action->AddOption(A);
 Action->SetSelectedIndex(0);bRendering=false;Key=FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);FrozenTokens.Reset();
 if(State)Changed=State->OnChanged.AddUObject(this,&UNightfallItemOperationDialog::Render);Render();
 for(const auto& P:Instances)if(P.Value==Selected){Target->SetSelectedOption(P.Key);break;}
}
void UNightfallItemOperationDialog::NativeDestruct(){if(State)State->OnChanged.Remove(Changed);Super::NativeDestruct();}
TOptional<FUIInputConfig> UNightfallItemOperationDialog::GetDesiredInputConfig() const{return FUIInputConfig(ECommonInputMode::Menu,EMouseCaptureMode::NoCapture);}
void UNightfallItemOperationDialog::Render()
{
 if(!Target || !State)return;bRendering=true;const FString T=Target->GetSelectedOption(),I=Ingredient->GetSelectedOption(),P=Support->GetSelectedOption();
 Target->ClearOptions();Ingredient->ClearOptions();Support->ClearOptions();Support->AddOption(TEXT("None"));Instances.Reset();Recipes.Reset();
 TArray<uint64> Ids;State->GetItems().GetKeys(Ids);Ids.Sort();
 for(uint64 Id:Ids){const auto* Item=State->Find(Id);const auto* Def=State->Definition(Item->TemplateId);const FString Label=FString::Printf(TEXT("%s #%llu x%llu +%u"),Def?*Def->Name:TEXT("Item"),Id,Item->Count.Value,Item->Enchant.Value);Instances.Add(Label,Id);Target->AddOption(Label);Support->AddOption(Label);if(Action->GetSelectedOption()!=TEXT("ExchangeSA"))Ingredient->AddOption(Label);}
 if(Action->GetSelectedOption()==TEXT("ExchangeSA"))for(const auto& Recipe:State->GetCatalog().Recipes){const auto* Product=State->Definition(Recipe.ProductTemplate);const FString Label=FString::Printf(TEXT("%s (recipe %llu)"),Product?*Product->Name:TEXT("Special ability"),Recipe.RecipeId.Value);Ingredient->AddOption(Label);Recipes.Add(Label,Recipe.RecipeId);}
 if(Instances.Contains(T))Target->SetSelectedOption(T);if(Instances.Contains(I) || Recipes.Contains(I))Ingredient->SetSelectedOption(I);Support->SetSelectedOption(Instances.Contains(P)?P:TEXT("None"));
 bRendering=false;ConfirmButton->SetIsEnabled(CanConfirm());FString Text=State->Status();
 if(State->GetEnchant().IsSet()){const auto& E=*State->GetEnchant();Text+=FString::Printf(TEXT("\nReceived enchant outcome %u: +%u → +%u"),uint8(E.Outcome),E.OldEnchant.Value,E.NewEnchant.Value);for(const auto& R:E.Refunds)Text+=FString::Printf(TEXT("\nRefund template %u x%llu"),R.TemplateId.Value,R.Count.Value);}
 if(State->GetRefine().IsSet())Text+=State->GetRefine()->Outcome==EGrpcNightfallV1RefineOutcome::REFINE_OUTCOME_APPLIED?TEXT("\nAugmentation applied"):TEXT("\nAugmentation removed");
 if(Action->GetSelectedOption()==TEXT("ExchangeSA"))for(const auto& R:State->GetCatalog().Recipes)if(R.RecipeId.Value==Recipes.FindRef(Ingredient->GetSelectedOption())){Text+=TEXT("\nRequired ingredients:");for(const auto& Input:R.Ingredients){const auto* D=State->Definition(Input.Template);Text+=FString::Printf(TEXT("\n%s x%llu"),D?*D->Name:TEXT("Item"),Input.Count.Value);}Text+=R.MaintainEnchantment?TEXT("\nRetains enchantment"):TEXT("\nResets enchantment");}
 if(bBlacksmith && State->Blacksmith().IsEmpty())Text+=TEXT("\nWaiting for the Blacksmith to be visible");Status->SetText(FText::FromString(Text));
}
bool UNightfallItemOperationDialog::CanConfirm() const
{
 if(!State || !State->HasBaseline() || State->IsBusy() || !State->HasCatalog())return false;
 if(!FrozenTokens.IsEmpty())return true;
 if(!Target || !Instances.Contains(Target->GetSelectedOption()) || (bBlacksmith && State->Blacksmith().IsEmpty()))return false;
 const FString A=Action->GetSelectedOption();return A==TEXT("Crystallize") || A==TEXT("CancelRefine") || (A==TEXT("ExchangeSA")?Recipes.Contains(Ingredient->GetSelectedOption()):Instances.Contains(Ingredient->GetSelectedOption()));
}
void UNightfallItemOperationDialog::SelectionChanged(FString,ESelectInfo::Type)
{
 if(bRendering)return;Key=FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);FrozenTokens.Reset();Render();
}
void UNightfallItemOperationDialog::Confirm()
{
 if(!CanConfirm())return;
 if(FrozenTokens.IsEmpty()){
  const FString A=Action->GetSelectedOption();FrozenTokens={TEXT("nf.")+A};auto Instance=[](uint64 Id){return FString::Printf(TEXT("instance:%llu"),Id);};
  if(A==TEXT("Enchant"))FrozenTokens.Add(Instance(Instances.FindRef(Ingredient->GetSelectedOption())));
  FrozenTokens.Add(Instance(Instances.FindRef(Target->GetSelectedOption())));
  if(A==TEXT("Refine"))FrozenTokens.Add(Instance(Instances.FindRef(Ingredient->GetSelectedOption())));
  if(A==TEXT("ExchangeSA"))FrozenTokens.Add(FString::Printf(TEXT("%llu"),Recipes.FindRef(Ingredient->GetSelectedOption())));
  if(A==TEXT("Enchant") && Support->GetSelectedOption()!=TEXT("None"))FrozenTokens.Add(TEXT("support:")+Instance(Instances.FindRef(Support->GetSelectedOption())));
  FrozenTokens.Add(Key);
 }
 State->Execute(FrozenTokens);Render();
}
void UNightfallItemOperationDialog::Close(){DeactivateWidget();RemoveFromParent();}
