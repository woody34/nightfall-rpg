using UnrealBuildTool;

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
			"HTTP",         // OIDC device flow and token refresh (Auth/AuthSubsystem)
			"TurboLinkGrpc", // gRPC channel + generated nightfall.v1 classes (Scripts/gen-proto.sh)
			"DeveloperSettings",
			"Json",
			"JsonUtilities",
		});

		PrivateDependencyModuleNames.AddRange(new string[]
		{
			"Slate",
			"SlateCore",
			"CommonInput",      // FUIInputConfig for the login screen
			"ApplicationCore",  // clipboard (copy verification URL)
			"NavigationSystem",
			"EngineSettings",   // UGameMapsSettings (content wiring test)
			"AIModule",     // SimpleMoveToLocation for local click-to-move preview
		});

		// Generated/ and GrpcBridge/ carry a .ubtignore: they are compiled inside the TurboLinkGrpc
		// module (Scripts/setup-turbolink.sh links them there), never here. This module reaches
		// them through TurboLinkGrpc's public headers and must not link protobuf or gRPC itself.
		PublicIncludePaths.Add(ModuleDirectory);   // so "Net/..." and "Nightfall.h" resolve from subfolders

		// Warnings are errors, matching the Rust side's -D warnings policy.
		bWarningsAsErrors = true;
	}
}
