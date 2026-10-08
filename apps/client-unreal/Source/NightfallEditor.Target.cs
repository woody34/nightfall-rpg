using UnrealBuildTool;
using System.Collections.Generic;

public class NightfallEditorTarget : TargetRules
{
	public NightfallEditorTarget(TargetInfo Target) : base(Target)
	{
		Type = TargetType.Editor;
		DefaultBuildSettings = BuildSettingsVersion.Latest;
		IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
		CppStandard = CppStandardVersion.Cpp20;
		ExtraModuleNames.Add("Nightfall");
	}
}
