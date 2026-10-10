#pragma once

#include "CoreMinimal.h"
#include "CommonActivatableWidget.h"
#include "NightfallClassDialog.generated.h"

class UPanelWidget;
class UClassStateSubsystem;
class UButton;
class UTextBlock;
class UVerticalBox;
class UNightfallClassDialog;

UCLASS()
class NIGHTFALL_API UNightfallClassChoiceHandler : public UObject
{
	GENERATED_BODY()
public:
	uint32 ClassId = 0;
	TWeakObjectPtr<UNightfallClassDialog> Dialog;
	UFUNCTION() void HandleClicked();
};

/** Catalogue-rendered path and master options. Only the server decides transfer eligibility. */
UCLASS()
class NIGHTFALL_API UNightfallClassDialog : public UCommonActivatableWidget
{
	GENERATED_BODY()
public:
	void EnsureLayout();
	void BindState(UClassStateSubsystem* InState, bool bLoadOptions = true);
	bool CanConfirm() const;
	FString ConfirmationText() const;
	FString SkillDetailsText() const;
	void Refresh();
	void Select(uint32 ClassId);
	void Render();
	int32 NumChoices() const { return Handlers.Num(); }
protected:
	virtual void NativeOnInitialized() override;
	virtual void NativeDestruct() override;
	virtual TOptional<FUIInputConfig> GetDesiredInputConfig() const override;
private:
	UTextBlock* Text(UPanelWidget* Parent, const FString& Label, int32 Size = 14);
	UButton* Button(UPanelWidget* Parent, const FString& Label);
	UFUNCTION() void HandleConfirm();
	UFUNCTION() void HandleRefresh();
	UFUNCTION() void HandleClose();
	UPROPERTY() TObjectPtr<UTextBlock> Summary;
	UPROPERTY() TObjectPtr<UTextBlock> Status;
	UPROPERTY() TObjectPtr<UTextBlock> Confirmation;
	UPROPERTY() TObjectPtr<UTextBlock> SkillDetails;
	UPROPERTY() TObjectPtr<UVerticalBox> Tree;
	UPROPERTY() TObjectPtr<UButton> ConfirmButton;
	UPROPERTY() TArray<TObjectPtr<UNightfallClassChoiceHandler>> Handlers;
	UPROPERTY() TObjectPtr<UClassStateSubsystem> BoundState;
	UClassStateSubsystem* GetState() const;
	TOptional<uint32> SelectedTarget;
	FDelegateHandle StateHandle;
};
