#pragma once
#include "CoreMinimal.h"
#include "Components/SceneComponent.h"
#include "SNightfallV1/ItemsMessage.h"
#include "EquipmentPresentationComponent.generated.h"
class UInventoryStateSubsystem;
class UStaticMeshComponent;
/**
 * Cosmetic placeholders consume only the public equipment projection (design §1.5/I6).
 * The three placeholder meshes are transient runtime children created on registration (or on the
 * first ApplyVisible for an unregistered component), not default subobjects of this component:
 * nested component subobjects attached to `this` in the constructor re-parent against the Blueprint
 * CDO's template when a Blueprint subclass (BP_RemoteEntity_C) is instanced and trip SceneComponent's
 * "Template Mismatch during attachment" ensure (integration defect T2).
 */
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
 UStaticMeshComponent* OffhandControl() const { return Offhand; }
 uint32 WeaponTemplate() const { return Template; }
 uint32 GlowTier() const { return Glow; }
protected:
 virtual void OnRegister() override;
 /** Creates any missing placeholder (never on a template) and, in a world, attaches and registers it. */
 void EnsurePlaceholders();
 virtual void EndPlay(const EEndPlayReason::Type Reason) override;
private:
 UPROPERTY(Transient) TObjectPtr<UStaticMeshComponent> Weapon;
 UPROPERTY(Transient) TObjectPtr<UStaticMeshComponent> Offhand;
 UPROPERTY(Transient) TObjectPtr<UStaticMeshComponent> Armour;
 UPROPERTY() TObjectPtr<UInventoryStateSubsystem> State;
 FString Entity;
 uint32 Template=0,Glow=0,LeftTemplate=0,LeftGlow=0,ArmourTemplate=0,ArmourGlow=0;
 FDelegateHandle Changed;
};
