#include "EquipmentPresentationComponent.h"
#include "InventoryStateSubsystem.h"
#include "Components/StaticMeshComponent.h"
#include "Engine/StaticMesh.h"
#include "Materials/MaterialInstanceDynamic.h"
#include "Materials/MaterialInterface.h"
#include "Misc/PackageName.h"
UEquipmentPresentationComponent::UEquipmentPresentationComponent()
{
 Weapon=CreateDefaultSubobject<UStaticMeshComponent>(TEXT("WeaponPlaceholder"));Weapon->SetupAttachment(this);Weapon->SetCollisionEnabled(ECollisionEnabled::NoCollision);Weapon->SetRelativeLocation(FVector(0,50,80));Weapon->SetRelativeScale3D(FVector(0.12f,0.12f,0.8f));Weapon->SetVisibility(false);
 Armour=CreateDefaultSubobject<UStaticMeshComponent>(TEXT("ArmourPlaceholder"));Armour->SetupAttachment(this);Armour->SetCollisionEnabled(ECollisionEnabled::NoCollision);Armour->SetRelativeLocation(FVector(0,0,110));Armour->SetRelativeScale3D(FVector(0.65f,0.65f,0.4f));Armour->SetVisibility(false);
 Offhand=CreateDefaultSubobject<UStaticMeshComponent>(TEXT("OffhandPlaceholder"));Offhand->SetupAttachment(this);Offhand->SetCollisionEnabled(ECollisionEnabled::NoCollision);Offhand->SetRelativeLocation(FVector(0,-50,80));Offhand->SetRelativeScale3D(FVector(0.12f,0.5f,0.5f));Offhand->SetVisibility(false);
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
 Super::OnRegister();if(GetOwner()){if(!Weapon->IsRegistered())Weapon->RegisterComponent();if(!Offhand->IsRegistered())Offhand->RegisterComponent();if(!Armour->IsRegistered())Armour->RegisterComponent();}
}
