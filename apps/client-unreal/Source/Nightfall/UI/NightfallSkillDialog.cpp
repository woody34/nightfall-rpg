#include "NightfallSkillDialog.h"
#include "Skills/SkillStateSubsystem.h"
#include "Blueprint/WidgetTree.h"
#include "Components/Border.h"
#include "Components/Button.h"
#include "Components/ScrollBox.h"
#include "Components/SizeBox.h"
#include "Components/TextBlock.h"
#include "Components/VerticalBox.h"
#include "Engine/GameInstance.h"
#include "Misc/Guid.h"
void UNightfallSkillChoice::Click() { if (Dialog.IsValid()) Dialog->Select(Id, Rank); }
UTextBlock* UNightfallSkillDialog::Text(UPanelWidget* Parent, const FString& Label)
{
 auto* T = WidgetTree->ConstructWidget<UTextBlock>(); T->SetText(FText::FromString(Label)); T->SetAutoWrapText(true);
 if (Parent) Parent->AddChild(T); return T;
}
UButton* UNightfallSkillDialog::Button(UPanelWidget* Parent, const FString& Label)
{
 auto* B = WidgetTree->ConstructWidget<UButton>(); auto* T = Text(nullptr, Label); T->SetColorAndOpacity(FSlateColor(FLinearColor::Black));
 B->AddChild(T); if (Parent) Parent->AddChild(B); return B;
}
void UNightfallSkillDialog::EnsureLayout()
{
 if (!WidgetTree) WidgetTree = NewObject<UWidgetTree>(this, TEXT("WidgetTree"), RF_Transient);
 if (WidgetTree->RootWidget) return;
 auto* Root = WidgetTree->ConstructWidget<UBorder>(); Root->SetBrushColor(FLinearColor(0.02f,0.025f,0.04f,0.97f)); Root->SetPadding(FMargin(24));
 Root->SetHorizontalAlignment(HAlign_Center); Root->SetVerticalAlignment(VAlign_Center); WidgetTree->RootWidget = Root;
 auto* Size = WidgetTree->ConstructWidget<USizeBox>(); Size->SetWidthOverride(700); Size->SetHeightOverride(540); Root->SetContent(Size);
 auto* Column = WidgetTree->ConstructWidget<UVerticalBox>(); Size->AddChild(Column);
 Text(Column,TEXT("Skill trainer")); Status=Text(Column,TEXT("Loading server choices..."));
 auto* ScrollSize=WidgetTree->ConstructWidget<USizeBox>(); ScrollSize->SetHeightOverride(350); Column->AddChild(ScrollSize);
 auto* Scroll=WidgetTree->ConstructWidget<UScrollBox>(); ScrollSize->AddChild(Scroll); Rows=WidgetTree->ConstructWidget<UVerticalBox>(); Scroll->AddChild(Rows);
 Selection=Text(Column,TEXT("Choose an eligible rank to learn.")); ConfirmButton=Button(Column,TEXT("Learn selected rank / retry"));
 ConfirmButton->OnClicked.AddDynamic(this,&UNightfallSkillDialog::Confirm);
 Button(Column,TEXT("Refresh server choices"))->OnClicked.AddDynamic(this,&UNightfallSkillDialog::Refresh);
 Button(Column,TEXT("Close"))->OnClicked.AddDynamic(this,&UNightfallSkillDialog::Close);
 ConfirmButton->SetIsEnabled(false);
}
void UNightfallSkillDialog::NativeOnInitialized()
{
 Super::NativeOnInitialized(); EnsureLayout(); BindState(GetGameInstance()?GetGameInstance()->GetSubsystem<USkillStateSubsystem>():nullptr);
}
void UNightfallSkillDialog::NativeDestruct() { if(State) State->OnChanged.Remove(Changed); Super::NativeDestruct(); }
TOptional<FUIInputConfig> UNightfallSkillDialog::GetDesiredInputConfig() const { return FUIInputConfig(ECommonInputMode::Menu,EMouseCaptureMode::NoCapture); }
void UNightfallSkillDialog::BindState(USkillStateSubsystem* S,bool bRefresh)
{
 if(State)State->OnChanged.Remove(Changed); State=S; if(State)Changed=State->OnChanged.AddUObject(this,&UNightfallSkillDialog::Render);
 if(bRefresh)Refresh();else Render();
}
void UNightfallSkillDialog::Refresh()
{
 if(!State || State->IsBusy())return;
 State->LoadLearnable([Weak=TWeakObjectPtr<UNightfallSkillDialog>(this)](const FNetResult& R){if(Weak.IsValid()){Weak->Status->SetText(FText::FromString(R.IsOk()?TEXT("Server requirements checked."):R.Message));Weak->Render();}});
 Render();
}
void UNightfallSkillDialog::Render()
{
 if(!Rows || !State)return; Rows->ClearChildren(); Choices.Reset();
 if(State->HasLearnable())for(const auto& R:State->GetLearnable().Skills)
 {
  const auto* Catalog=State->FindSkill(R.Skill.SkillId.Value);
  FString Name=Catalog?Catalog->DisplayName:FString::Printf(TEXT("Skill %u"),R.Skill.SkillId.Value);
  FString Label=FString::Printf(TEXT("%s rank %u — SP %llu, level %u%s%s"),*Name,R.Skill.SkillLevel.Value,R.Skill.SpCost.Value,R.Skill.RequiredLevel.Value,R.TrainerRequired?TEXT(" — trainer required"):TEXT(""),R.Learned?TEXT(" — learned"):TEXT(""));
  if(!R.Unmet.IsEmpty())Label+=TEXT(" — ")+FString::Join(R.Unmet,TEXT(", "));
  auto* B=Button(Rows,Label); B->SetIsEnabled(R.Eligible && !State->IsBusy());
  auto* H=NewObject<UNightfallSkillChoice>(this); H->Id=R.Skill.SkillId.Value; H->Rank=R.Skill.SkillLevel.Value; H->Dialog=this; Choices.Add(H); B->OnClicked.AddDynamic(H,&UNightfallSkillChoice::Click);
 }
 if(!State->HasLearnable())Text(Rows,State->IsBusy()?TEXT("Waiting for the server..."):TEXT("Choices unavailable. Refresh to retry."));
 ConfirmButton->SetIsEnabled(CanConfirm());
}
bool UNightfallSkillDialog::CanConfirm() const
{
 return State && !State->IsBusy() && State->HasLearnable() && State->GetLearnable().Skills.ContainsByPredicate([this](const auto& R){return R.Eligible && R.Skill.SkillId.Value==SelectedId && R.Skill.SkillLevel.Value==SelectedRank;});
}
void UNightfallSkillDialog::Select(uint32 Id,uint32 Rank)
{
 if(!State || State->IsBusy())return;
 if(SelectedId!=Id || SelectedRank!=Rank){RetryKey=FGuid::NewGuid().ToString(EGuidFormats::DigitsWithHyphensLower);SelectedId=Id;SelectedRank=Rank;}
 Selection->SetText(FText::FromString(FString::Printf(TEXT("Learn skill %u rank %u?"),Id,Rank)));Render();
}
void UNightfallSkillDialog::Confirm()
{
 if(!CanConfirm())return;
 State->Learn(SelectedId,SelectedRank,[Weak=TWeakObjectPtr<UNightfallSkillDialog>(this)](const FNetResult& R)
 {
  if(!Weak.IsValid())return;
  Weak->Status->SetText(FText::FromString(R.IsOk()?TEXT("Learned. Waiting for authoritative world state."):R.Message));
  if(R.IsOk()){Weak->SelectedId=Weak->SelectedRank=0;Weak->RetryKey.Reset();Weak->Refresh();}else Weak->Render();
 },RetryKey);Render();
}
void UNightfallSkillDialog::Close(){RemoveFromParent();}
