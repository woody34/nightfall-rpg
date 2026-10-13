#include "EquipmentPresentationComponent.h"
#include "InventoryStateSubsystem.h"
#include "Components/StaticMeshComponent.h"
#include "Engine/StaticMesh.h"
#include "Materials/MaterialInstanceDynamic.h"
#include "Materials/MaterialInterface.h"
#include "Misc/PackageName.h"
UEquipmentPresentationComponent::UEquipmentPresentationComponent()=default;
void UEquipmentPresentationComponent::EnsurePlaceholders()
{
 if(IsTemplate())return;
 auto Make=[this](TObjectPtr<UStaticMeshComponent>& Mesh,const TCHAR* Name,const FVector& Location,const FVector& Scale){
  if(!IsValid(Mesh))
  {
   // Outer is this component, so the owning actor (when there is one) owns the placeholder too.
   Mesh=NewObject<UStaticMeshComponent>(this,MakeUniqueObjectName(this,UStaticMeshComponent::StaticClass(),Name),RF_Transient);
   Mesh->SetCollisionEnabled(ECollisionEnabled::NoCollision);Mesh->SetRelativeLocation(Location);Mesh->SetRelativeScale3D(Scale);Mesh->SetVisibility(false);
   Mesh->SetupAttachment(this);
  }
  if(IsRegistered() && GetOwner() && !Mesh->IsRegistered()){if(Mesh->GetAttachParent()!=this)Mesh->SetupAttachment(this);Mesh->RegisterComponent();}
 };
 Make(Weapon,TEXT("WeaponPlaceholder"),FVector(0,50,80),FVector(0.12f,0.12f,0.8f));
 Make(Armour,TEXT("ArmourPlaceholder"),FVector(0,0,110),FVector(0.65f,0.65f,0.4f));
 Make(Offhand,TEXT("OffhandPlaceholder"),FVector(0,-50,80),FVector(0.12f,0.5f,0.5f));
}
FLinearColor UEquipmentPresentationComponent::GlowColor(uint32 Tier)
{
 // Placeholder colours, not source client materials. Tier is already projected by the server.
 return Tier>=2?FLinearColor(1.f,0.2f,0.05f):Tier==1?FLinearColor(0.15f,0.45f,1.f):FLinearColor(0.6f,0.6f,0.6f);
}
void UEquipmentPresentationComponent::Bind(UInventoryStateSubsystem* S,const FString& Id)
{
 if(State)State->OnChanged.Remove(Changed);State=S;Entity=Id;if(State)Changed=State->OnChanged.AddUObject(this,&UEquipmentPresentationComponent::Refresh);Refresh();
}
void UEquipmentPresentationComponent::Refresh(){const auto* V=State?State->Visible(Entity):nullptr;ApplyVisible(V?*V:TArray<FGrpcNightfallV1VisibleEquipment>());}
void UEquipmentPresentationComponent::ApplyVisible(const TArray<FGrpcNightfallV1VisibleEquipment>& Rows)
{
 EnsurePlaceholders();if(!Weapon)return;
 uint32 T=0,G=0,L=0,LG=0,A=0,AG=0;
 for(const auto& R:Rows){if(R.Slot==EGrpcNightfallV1EquipSlot::EQUIP_SLOT_RHAND){T=R.TemplateId;G=R.GlowTier;}if(R.Slot==EGrpcNightfallV1EquipSlot::EQUIP_SLOT_LHAND){L=R.TemplateId;LG=R.GlowTier;}if(R.Slot==EGrpcNightfallV1EquipSlot::EQUIP_SLOT_CHEST){A=R.TemplateId;AG=R.GlowTier;}}
 auto Apply=[&](UStaticMeshComponent* Mesh,uint32 Id,uint32 Tier,uint32& Held,uint32& HeldGlow,const TCHAR* Path){
  const bool ChangedMesh=Id!=Held;if(ChangedMesh){Held=Id;Mesh->SetStaticMesh(Id?LoadObject<UStaticMesh>(nullptr,Path):nullptr);}Mesh->SetVisibility(Id!=0);
  if(Tier!=HeldGlow || ChangedMesh){HeldGlow=Tier;if(FPackageName::DoesPackageExist(TEXT("/Game/Items/M_ItemGlow")))if(auto* M=LoadObject<UMaterialInterface>(nullptr,TEXT("/Game/Items/M_ItemGlow.M_ItemGlow"))){auto* D=UMaterialInstanceDynamic::Create(M,this);D->SetVectorParameterValue(TEXT("Tint"),GlowColor(Tier));Mesh->SetMaterial(0,D);}}
 };
 Apply(Weapon,T,G,Template,Glow,T%2?TEXT("/Engine/BasicShapes/Cube.Cube"):TEXT("/Engine/BasicShapes/Cylinder.Cylinder"));
 Apply(Offhand,L,LG,LeftTemplate,LeftGlow,TEXT("/Engine/BasicShapes/Cube.Cube"));
 Apply(Armour,A,AG,ArmourTemplate,ArmourGlow,TEXT("/Engine/BasicShapes/Cube.Cube"));
}
void UEquipmentPresentationComponent::EndPlay(const EEndPlayReason::Type Reason){if(State)State->OnChanged.Remove(Changed);Super::EndPlay(Reason);}

void UEquipmentPresentationComponent::OnRegister()
{
 Super::OnRegister();EnsurePlaceholders();
}
