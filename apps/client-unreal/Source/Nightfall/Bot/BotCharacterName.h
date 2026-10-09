#pragma once

#include "CoreMinimal.h"
#include "Misc/Guid.h"
#include "Misc/Parse.h"

namespace BotCharacterName
{
	/** Server-valid 16-letter name from 52 random bits, including for timestamp-prefixed UUIDv7. */
	inline FString FromGuid(const FGuid& Guid)
	{
		const FString Hex = Guid.ToString(EGuidFormats::Digits);
		FString Name = TEXT("Bot");
		for (int32 I = Hex.Len() - 13; I < Hex.Len(); ++I)
		{
			Name.AppendChar(TEXT('a') + static_cast<TCHAR>(FParse::HexDigit(Hex[I])));
		}
		return Name;
	}
}
