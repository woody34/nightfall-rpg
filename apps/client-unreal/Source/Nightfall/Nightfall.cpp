#include "Nightfall.h"
#include "Modules/ModuleManager.h"

DEFINE_LOG_CATEGORY(LogNightfall);

void FNightfallModule::StartupModule()
{
	UE_LOG(LogNightfall, Log, TEXT("Nightfall module started"));
}

void FNightfallModule::ShutdownModule()
{
}

IMPLEMENT_PRIMARY_GAME_MODULE(FNightfallModule, Nightfall, "Nightfall");
