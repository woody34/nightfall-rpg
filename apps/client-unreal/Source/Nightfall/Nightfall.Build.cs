using UnrealBuildTool;
using System.IO;

public class Nightfall : ModuleRules
{
	public Nightfall(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;
		CppStandard = CppStandardVersion.Cpp20;

		PublicDependencyModuleNames.AddRange(new string[]
		{
			"Core",
			"CoreUObject",
			"Engine",
			"InputCore",
			"EnhancedInput",
			"UMG",
			"CommonUI",
			"WebSockets",   // IWebSocket: the real-time channel to the Rust server
			"HTTP",         // health/ready probes; gRPC comes later via a dedicated module
			"Json",
			"JsonUtilities",
		});

		PrivateDependencyModuleNames.AddRange(new string[]
		{
			"Slate",
			"SlateCore",
			"NavigationSystem",
			"AIModule",     // SimpleMoveToLocation for local click-to-move preview
		});

		// Generated protobuf code (Scripts/gen-proto.sh) and the vendored protobuf-lite runtime.
		// Until the ThirdParty module exists, ProtoCodec.cpp carries a minimal hand-written wire
		// codec for the envelope so the client links without protobuf. See Net/ProtoCodec.h.
		PublicIncludePaths.Add(ModuleDirectory);   // so "Net/..." and "Nightfall.h" resolve from subfolders
		PublicIncludePaths.Add(Path.Combine(ModuleDirectory, "Generated"));

		// Warnings are errors, matching the Rust side's -D warnings policy.
		bWarningsAsErrors = true;
	}
}
