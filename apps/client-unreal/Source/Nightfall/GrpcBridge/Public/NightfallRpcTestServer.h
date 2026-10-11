#pragma once
#include "CoreMinimal.h"

#if WITH_DEV_AUTOMATION_TESTS
/** Owned loopback status fixture; destroyed after each offline transport test. */
class INightfallRpcTestServer
{
public:
 virtual ~INightfallRpcTestServer() = default;
 virtual FString Endpoint() const = 0;
};

/** Catalogue fails UNAVAILABLE or FAILED_PRECONDITION; LearnSkill fails UNAVAILABLE. */
TURBOLINKGRPC_API TUniquePtr<INightfallRpcTestServer> MakeNightfallRpcTestServer(bool bCatalogUnavailable);
#endif
