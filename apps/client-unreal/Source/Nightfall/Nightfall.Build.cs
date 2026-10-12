using UnrealBuildTool;

public class Nightfall : ModuleRules
{
	public Nightfall(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;
		CppStandard = CppStandardVersion.Cpp20;
		// Tests and commands use file-local helpers; keep their translation units isolated.
		bUseUnity = false;

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
			"NavigationSystem",   // nav mesh presence check before the local preview
			"AIModule",     // SimpleMoveToLocation for local click-to-move preview
			"XmlParser",    // Bot JUnit shape test parses the report back
		});

		// Generated/ and GrpcBridge/ carry a .ubtignore: they are compiled inside the TurboLinkGrpc
		// module (Scripts/setup-turbolink.sh links them there), never here. This module reaches
		// them through TurboLinkGrpc's public headers and must not link protobuf or gRPC itself.
		PublicIncludePaths.Add(ModuleDirectory);   // so "Net/..." and "Nightfall.h" resolve from subfolders

		// Warnings are errors, matching the Rust side's -D warnings policy.
		bWarningsAsErrors = true;

		// Each .cpp is its own translation unit. Tests and console commands share anonymous-namespace
		// names (Flags, FRecordingSocket, CreateCommand...), which collide once UBT's adaptive unity
		// build merges committed files; it only hid this while those files were in the git working set.
		bUseUnity = false;
	}
}
