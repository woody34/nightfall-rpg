#pragma once

#include "CoreMinimal.h"
#include "SNightfallV1/GameMessage.h"

namespace NightfallCreation
{
	/** Shared by the form and development console; the server validates every field. */
	inline FGrpcNightfallV1CreateCharacterRequest Request(const FString& Name, uint32 Race, uint32 BaseClass, uint32 Sex, uint32 HairStyle = 0, uint32 HairColor = 0, uint32 Face = 0)
	{
		FGrpcNightfallV1CreateCharacterRequest Out;
		Out.Name = Name;
		Out.Race = static_cast<EGrpcNightfallV1Race>(Race);
		Out._base_class_id._base_class_idCase = EGrpcNightfallV1CreateCharacterRequest_base_class_id::BaseClassId;
		Out._base_class_id.BaseClassId = BaseClass;
		Out.Sex = static_cast<EGrpcNightfallV1Sex>(Sex);
		Out.HairStyle = HairStyle;
		Out.HairColor = HairColor;
		Out.Face = Face;
		return Out;
	}
}
