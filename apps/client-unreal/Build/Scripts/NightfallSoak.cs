using System;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Text.Json;
using Gauntlet;

namespace Nightfall
{
    // The actual shipped Development client runs every assertion. Gauntlet owns role lifetimes,
    // process deadlines, crash/ensure detection and collection of each role's engine log.
    public class NightfallSoak : UnrealTestNode<UnrealTestConfiguration>
    {
        private DateTime Started;
        private int Clients;
        private int Seconds;
        private string Output;

        public NightfallSoak(UnrealTestContext context) : base(context) { }

        public override UnrealTestConfiguration GetConfiguration()
        {
            var config = base.GetConfiguration();
            Clients = Globals.Params.ParseValue("SoakClients", 8);
            Seconds = Globals.Params.ParseValue("SoakSeconds", 1200);
            Output = Path.GetFullPath(Globals.Params.ParseValue("SoakOutput", "Saved/Soak"));
            string fixtures = Path.GetFullPath(Globals.Params.ParseValue("SoakFixtures", "Saved/Soak/fixtures"));
            if (Clients < 1 || Clients > 8 || Seconds < 1)
                throw new ArgumentException("SoakClients must be 1..8 and SoakSeconds positive");
            string manifestPath = Globals.Params.ParseValue("SoakRoleManifest", "");
            var lanes = ReadRoles(manifestPath, Clients);
            config.MaxDuration = Seconds + (lanes != null ? 210 : 180); // boot plus the final complete 90-second scenario
            config.AllRolesExit = true;
            config.FailOnEnsures = true;
            var roles = config.RequireRoles(UnrealTargetRole.Client, Clients);
            for (int i = 0; i < roles.Count; ++i)
            {
                string dir = Path.Combine(Output, $"client-{i + 1:00}");
                Directory.CreateDirectory(dir);
                roles[i].CommandLineParams.AddRawCommandline("-ini:Game:[/Script/Nightfall.NetSettings]:GrpcEndpoint=" + Globals.Params.ParseValue("SoakGrpc", "localhost:15051"));
                roles[i].CommandLineParams.Add("nullrhi");
                roles[i].CommandLineParams.Add("nosound");
                roles[i].CommandLineParams.Add("unattended");
                string scenario = lanes != null ? lanes[i].Scenario : Path.Combine(fixtures, $"client-{i + 1:00}", "Scenarios", "1-kill-one-monster.nfs");
                roles[i].CommandLineParams.Add("BotScenario", $"\"{scenario}\"");
                if (lanes != null) roles[i].CommandLineParams.Add("DevTokenFile", $"\"{lanes[i].Token}\"");
                roles[i].CommandLineParams.Add("BotDataDir", $"\"{Path.Combine(fixtures, "data")}\"");
                roles[i].CommandLineParams.Add("BotOutDir", $"\"{dir}\"");
                roles[i].CommandLineParams.Add("BotLoop", Seconds.ToString());
                // A separate user dir prevents the clients racing over UE user settings and logs.
                roles[i].CommandLineParams.Add("UserDir", $"\"{Path.Combine(dir, "User")}\"");
            }
            return config;
        }

        private sealed class Lane
        {
            public string Scenario;
            public string Token;
        }

        // Fail before launching any role. Tokens are paths only and never read or logged here.
        private static List<Lane> ReadRoles(string path, int clients)
        {
            if (String.IsNullOrEmpty(path)) return null;
            using var document = JsonDocument.Parse(File.ReadAllText(Path.GetFullPath(path)));
            var root = document.RootElement;
            if (root.GetProperty("schema_version").GetInt32() != 1 || !root.GetProperty("casting").GetBoolean())
                throw new ArgumentException("SoakRoleManifest requires schema_version 1 and casting true");
            var roles = root.GetProperty("roles");
            if (roles.GetArrayLength() != clients) throw new ArgumentException("SoakRoleManifest role count must match SoakClients");
            var paths = new HashSet<string>(StringComparer.Ordinal);
            var result = new List<Lane>();
            int index = 0;
            foreach (var role in roles.EnumerateArray())
            {
                ++index;
                if (role.GetProperty("name").GetString() != $"client-{index:00}")
                    throw new ArgumentException("SoakRoleManifest role order must be client-01, client-02, ...");
                string scenario = role.GetProperty("scenario").GetString();
                string token = role.GetProperty("token_file").GetString();
                foreach (string value in new[] { scenario, token })
                {
                    if (String.IsNullOrEmpty(value) || !Path.IsPathFullyQualified(value) || value.Contains('"') || value.Contains('\n') || value.Contains('\r') || !File.Exists(value) || !paths.Add(Path.GetFullPath(value)))
                        throw new ArgumentException("Manifest paths must be unique existing absolute paths without command-line delimiters");
                }
                if (Path.GetFileName(scenario) != "1-kill-one-monster.nfs")
                    throw new ArgumentException("Casting scenario must retain the stable report filename");
                if (!OperatingSystem.IsWindows())
                {
                    var mode = File.GetUnixFileMode(token);
                    var exposed = UnixFileMode.GroupRead | UnixFileMode.GroupWrite | UnixFileMode.GroupExecute | UnixFileMode.OtherRead | UnixFileMode.OtherWrite | UnixFileMode.OtherExecute;
                    if ((mode & exposed) != 0 || (mode & UnixFileMode.UserRead) == 0)
                        throw new ArgumentException("DevTokenFile must be private and owner-readable");
                }
                result.Add(new Lane { Scenario = Path.GetFullPath(scenario), Token = Path.GetFullPath(token) });
            }
            return result;
        }

        public override bool StartTest(int pass, int numPasses)
        {
            Started = DateTime.UtcNow;
            return base.StartTest(pass, numPasses);
        }

        public override void TickTest()
        {
            base.TickTest();
            foreach (var role in TestInstance.RunningRoles)
            {
                if (role.AppInstance.HasExited && role.AppInstance.ExitCode != 0)
                {
                    ReportError("Soak client exited with code {0}", role.AppInstance.ExitCode);
                    MarkTestComplete();
                    SetUnrealTestResult(TestResult.Failed);
                    return;
                }
                if (!role.AppInstance.HasExited && (DateTime.UtcNow - Started).TotalSeconds > Seconds + 180)
                {
                    ReportError("Soak client exceeded its {0}s process budget", Seconds + 180);
                    MarkTestComplete();
                    SetUnrealTestResult(TestResult.TimedOut);
                    return;
                }
            }
        }

        protected override UnrealProcessResult GetExitCodeAndReason(StopReason reason, UnrealLog log,
            UnrealRoleArtifacts artifacts, out string exitReason, out int exitCode)
        {
            var result = base.GetExitCodeAndReason(reason, log, artifacts, out exitReason, out exitCode);
            if (result != UnrealProcessResult.ExitOk) return result;
            if (artifacts.AppInstance.WasKilled || artifacts.AppInstance.ExitCode != 0)
            {
                exitCode = artifacts.AppInstance.ExitCode != 0 ? artifacts.AppInstance.ExitCode : 1;
                exitReason = "Soak requires every process to exit naturally with code zero";
                return UnrealProcessResult.TestFailure;
            }
            return result;
        }
    }
}
