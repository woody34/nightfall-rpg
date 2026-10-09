#include "Misc/AutomationTest.h"
#include "Bot/BotCharacterName.h"

#if WITH_DEV_AUTOMATION_TESTS
IMPLEMENT_SIMPLE_AUTOMATION_TEST(FBotCharacterNameTest, "Nightfall.Bot.CharacterName",
	EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)

bool FBotCharacterNameTest::RunTest(const FString& Parameters)
{
	FGuid First, Second;
	FGuid::Parse(TEXT("01a122f0-699b-7042-bba6-8b9664905c31"), First);
	FGuid::Parse(TEXT("01a122f0-699b-7042-bba6-8b9664905c32"), Second);
	const FString A = BotCharacterName::FromGuid(First);
	const FString B = BotCharacterName::FromGuid(Second);
	TestTrue(TEXT("same UUIDv7 timestamp retains distinct random identities"), A != B);
	TestEqual(TEXT("maximum server name length"), A.Len(), 16);
	for (TCHAR C : A)
	{
		TestTrue(TEXT("ASCII letters only"), (C >= TEXT('a') && C <= TEXT('z')) || (C >= TEXT('A') && C <= TEXT('Z')));
	}
	return true;
}
#endif
