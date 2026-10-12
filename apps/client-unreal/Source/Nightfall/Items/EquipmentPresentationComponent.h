#pragma once
#include "CoreMinimal.h"
#include "Components/SceneComponent.h"
#include "SNightfallV1/ItemsMessage.h"
#include "EquipmentPresentationComponent.generated.h"
class UInventoryStateSubsystem;
class UStaticMeshComponent;
/** Cosmetic placeholders consume only the public equipment projection (design §1.5/I6). */
UCLASS(ClassGroup=(Nightfall),meta=(BlueprintSpawnableComponent))
class NIGHTFALL_API UEquipmentPresentationComponent : public USceneComponent
{
 GENERATED_BODY()
public:
 UEquipmentPresentationComponent();
 void Bind(UInventoryStateSubsystem* State,const FString& Entity);
 void ApplyVisible(const TArray<FGrpcNightfallV1VisibleEquipment>& Rows);
 void Refresh();
 static FLinearColor GlowColor(uint32 Tier);
 UStaticMeshComponent* WeaponControl() const { return Weapon; }
 UStaticMeshComponent* ArmourControl() const {return Armour;}
 uint32 WeaponTemplate() const { return Template; }
 uint32 GlowTier() const { return Glow; }
protected:
 virtual void OnRegister() override;
 virtual void EndPlay(const EEndPlayReason::Type Reason) override;
private:
 UPROPERTY() TObjectPtr<UStaticMeshComponent> Weapon;
 UPROPERTY() TObjectPtr<UStaticMeshComponent> Offhand;
 UPROPERTY() TObjectPtr<UStaticMeshComponent> Armour;
 UPROPERTY() TObjectPtr<UInventoryStateSubsystem> State;
 FString Entity;
 uint32 Template=0,Glow=0,LeftTemplate=0,LeftGlow=0,ArmourTemplate=0,ArmourGlow=0;
 FDelegateHandle Changed;
};
