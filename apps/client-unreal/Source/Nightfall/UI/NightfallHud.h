#pragma once

#include "CoreMinimal.h"
#include "CommonUserWidget.h"
#include "Combat/CombatStateSubsystem.h"
#include "NightfallHud.generated.h"

class UBorder;
class UButton;
class UCanvasPanel;
class UCanvasPanelSlot;
class UProgressBar;
class UTextBlock;
class UVerticalBox;

/**
 * The in-world combat HUD (E5.2 / E5.3), built in code like the login screen: own HP / MP bars,
 * level and XP; the target frame; the status line; floating HP bars over NPC proxies and remote
 * players; floating damage numbers; and the dead overlay with its Respawn button.
 *
 * It owns no game state. Every frame it reads UCombatStateSubsystem::BuildHudModel() and projects
 * proxies to the screen; damage numbers are spawned from UCombatStateSubsystem::OnDamageNumber,
 * which fires once per AttackResult.
 */
UCLASS()
class NIGHTFALL_API UNightfallHud : public UCommonUserWidget
{
	GENERATED_BODY()

public:
	/** Seconds a damage number floats before it is removed. */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall|HUD")
	float DamageNumberSeconds = 1.2f;

	/** Seconds the status line stays visible after the last change. */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall|HUD")
	float StatusSeconds = 6.f;

	/** Height above a proxy's origin where its floating bar sits (cm). */
	UPROPERTY(EditDefaultsOnly, Category = "Nightfall|HUD")
	float BarHeightCm = 230.f;

	/** Builds the code layout if no Blueprint supplied one. NativeOnInitialized calls it; tests call it without a player. */
	void EnsureLayout();

	int32 NumDamageNumbers() const { return Numbers.Num(); }
	int32 NumFloatingBars() const { return Bars.Num(); }

protected:
	virtual void NativeOnInitialized() override;
	virtual void NativeConstruct() override;
	virtual void NativeDestruct() override;
	virtual void NativeTick(const FGeometry& MyGeometry, float InDeltaTime) override;

private:
	struct FFloatingBar
	{
		UVerticalBox* Box = nullptr;
		UTextBlock* Name = nullptr;
		UProgressBar* Bar = nullptr;
	};
	struct FFloatingNumber
	{
		UTextBlock* Text = nullptr;
		FVector Anchor = FVector::ZeroVector;
		float Age = 0.f;
	};

	void BuildLayout();
	UTextBlock* MakeText(UPanelWidget* Parent, const FString& Text, int32 Size);
	UProgressBar* MakeBar(UPanelWidget* Parent, const FLinearColor& Fill, float Width, float Height);
	void ApplyModel(const FCombatHudModel& Model);
	void UpdateFloatingBars();
	void UpdateNumbers(float DeltaSeconds);
	void SpawnNumber(const FDamageNumber& Number);
	bool ProjectToCanvas(const FVector& World, FVector2D& OutPosition) const;

	UFUNCTION() void HandleStatus(const FString& Status);
	UFUNCTION() void HandleRespawnClicked();
	UFUNCTION() void HandleClassesClicked();

	UPROPERTY() TObjectPtr<UCanvasPanel> Root;
	UPROPERTY() TObjectPtr<UCanvasPanel> FloatLayer;
	UPROPERTY() TObjectPtr<UVerticalBox> OwnPanel;
	UPROPERTY() TObjectPtr<UTextBlock> OwnLabel;
	UPROPERTY() TObjectPtr<UProgressBar> OwnHpBar;
	UPROPERTY() TObjectPtr<UTextBlock> OwnHpText;
	UPROPERTY() TObjectPtr<UProgressBar> OwnMpBar;
	UPROPERTY() TObjectPtr<UTextBlock> OwnMpText;
	UPROPERTY() TObjectPtr<UProgressBar> OwnCpBar;
	UPROPERTY() TObjectPtr<UTextBlock> OwnCpText;
	UPROPERTY() TObjectPtr<UTextBlock> ClassText;
	UPROPERTY() TObjectPtr<UTextBlock> XpText;
	UPROPERTY() TObjectPtr<UTextBlock> AttackText;
	UPROPERTY() TObjectPtr<UVerticalBox> TargetPanel;
	UPROPERTY() TObjectPtr<UTextBlock> TargetLabel;
	UPROPERTY() TObjectPtr<UProgressBar> TargetHpBar;
	UPROPERTY() TObjectPtr<UTextBlock> TargetHpText;
	UPROPERTY() TObjectPtr<UTextBlock> StatusText;
	UPROPERTY() TObjectPtr<UBorder> DeadOverlay;
	UPROPERTY() TObjectPtr<UButton> RespawnButton;
	UPROPERTY() TObjectPtr<UTextBlock> RespawnLabel;

	UPROPERTY() TMap<FString, TObjectPtr<UVerticalBox>> BarBoxes;   // keeps the pooled widgets alive
	TMap<FString, FFloatingBar> Bars;
	TArray<FFloatingNumber> Numbers;
	float StatusAge = 0.f;
	FDelegateHandle DamageHandle;
};
