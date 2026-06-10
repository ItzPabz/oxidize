using System.Collections.Concurrent;
using System.Linq;
using System.Reflection.PortableExecutable;
using System.Runtime.InteropServices;
using System.Text.Json;
using AsmResolver.DotNet;
using AsmResolver.PE.DotNet.Metadata.Tables;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace OxideCompiler;

public static class Bridge
{
    // Smoke test (kept for sanity checks).
    [UnmanagedCallersOnly]
    public static int Ping(int value) => value + 1;

    // Takes a pointer to a null-terminated UTF-8 JSON request, returns a pointer to a
    // null-terminated UTF-8 JSON response. Caller must release it with FreeResult.
    [UnmanagedCallersOnly]
    public static unsafe byte* Compile(byte* requestUtf8)
    {
        string responseJson;
        try
        {
            string requestJson = Marshal.PtrToStringUTF8((IntPtr)requestUtf8) ?? "{}";
            var req = JsonSerializer.Deserialize<CompileRequest>(requestJson) ?? new CompileRequest();
            responseJson = JsonSerializer.Serialize(Run(req));
        }
        catch (Exception ex)
        {
            // Host-level failure (bad request, file missing, Roslyn blew up) -> maps to CompileResult::Errored
            responseJson = JsonSerializer.Serialize(new CompileResponse
            {
                success = false,
                errored = true,
                errors = new[] { ex.Message },
            });
        }
        return (byte*)Marshal.StringToCoTaskMemUTF8(responseJson);
    }

    [UnmanagedCallersOnly]
    public static unsafe void FreeResult(byte* ptr) => Marshal.FreeCoTaskMem((IntPtr)ptr);

    // Rewrites an assembly so every type/method/field is public. Real Rust plugins poke
    // protected/internal game internals; without this they hit CS0122. Returns 0 on success.
    // Types are always publicized; members are skipped if they're compiler-generated or
    // extension methods, matching Oxide's Publicizer so we expose the same surface.
    [UnmanagedCallersOnly]
    public static unsafe int Publicize(byte* pathUtf8)
    {
        try
        {
            string path = Marshal.PtrToStringUTF8((IntPtr)pathUtf8) ?? "";
            var module = ModuleDefinition.FromBytes(File.ReadAllBytes(path));

            foreach (var type in module.GetAllTypes())
            {
                type.Attributes = type.IsNested
                    ? (type.Attributes & ~TypeAttributes.VisibilityMask) | TypeAttributes.NestedPublic
                    : (type.Attributes & ~TypeAttributes.VisibilityMask) | TypeAttributes.Public;

                foreach (var method in type.Methods)
                {
                    if (SkipPublicize(method)) continue;
                    method.Attributes = (method.Attributes & ~MethodAttributes.MemberAccessMask) | MethodAttributes.Public;
                }

                foreach (var field in type.Fields)
                {
                    if (SkipPublicize(field)) continue;
                    field.Attributes = (field.Attributes & ~FieldAttributes.FieldAccessMask) | FieldAttributes.Public;
                }
            }

            module.Write(path);
            return 0;
        }
        catch
        {
            return 1;
        }
    }

    // Oxide's Publicizer leaves compiler-generated members and extension methods alone
    // (matched by attribute-name suffix). We mirror that so a plugin can't reference a
    // member we exposed but a real server wouldn't have.
    private static bool SkipPublicize(IHasCustomAttribute member) =>
        member.CustomAttributes.Any(a =>
        {
            string? name = a.Constructor?.DeclaringType?.FullName;
            return name != null
                && (name.EndsWith("CompilerGeneratedAttribute", StringComparison.Ordinal)
                    || name.EndsWith("CompilerServices.ExtensionAttribute", StringComparison.Ordinal));
        });

    private static CompileResponse Run(CompileRequest req)
    {
        string source = File.ReadAllText(req.plugin);

        // Match Oxide.Compiler's parse config: preview language + the server's
        // preprocessor symbols (RUST, OXIDE, ...) so #if blocks resolve the same way.
        var parseOptions = new CSharpParseOptions(
            LanguageVersion.Preview,
            preprocessorSymbols: req.preprocessor);

        var tree = CSharpSyntaxTree.ParseText(source, parseOptions);

        // References are identical for every plugin in a run, so resolving them
        // (disk read + PE parse + metadata load) per plugin was the dominant cost.
        // RefCache memoizes the resolved MetadataReference (or a null "skip"
        // decision) per path so each DLL is loaded exactly once per process.
        var references = req.references
            .Select(r => RefCache.GetOrAdd(r, BuildReference))
            .Where(r => r is not null)
            .Select(r => r!)
            .ToList();

        var compilation = CSharpCompilation.Create(
            assemblyName: Path.GetFileNameWithoutExtension(req.plugin),
            syntaxTrees: new[] { tree },
            references: references,
            options: CompilationOptions);

        using var ms = new MemoryStream();
        var result = compilation.Emit(ms);

        var errors = result.Diagnostics
            .Where(d => d.Severity == DiagnosticSeverity.Error)
            .Select(d => d.ToString())
            .ToArray();

        return new CompileResponse { success = result.Success, errors = errors };
    }

    // Oxide.References bundles its own copy of these, so the game's standalone copies
    // would collide (CS0433 "exists in both ..."). Prefer Oxide's versions.
    private static readonly HashSet<string> Excluded = new(StringComparer.OrdinalIgnoreCase)
    {
        "Newtonsoft.Json.dll",
    };

    // Resolved references, keyed by path. A null value means "skip this path"
    // (missing / native / excluded), so negative decisions are memoized too.
    // MetadataReference is immutable and safe to share across concurrent
    // compilations, so this cache also serves the parallel compile path.
    private static readonly ConcurrentDictionary<string, MetadataReference?> RefCache =
        new(StringComparer.OrdinalIgnoreCase);

    // Match Oxide.Compiler's CSharpCompilationOptions. allowUnsafe stops false
    // CS0227s; the desktop identity comparer keeps game/Oxide version skew a
    // warning (CS1701) instead of a hard error (CS1702). Immutable, so it's
    // built once and shared by every compilation.
    private static readonly CSharpCompilationOptions CompilationOptions =
        new CSharpCompilationOptions(
                OutputKind.DynamicallyLinkedLibrary,
                platform: Platform.AnyCpu,
                allowUnsafe: true,
                deterministic: true,
                optimizationLevel: OptimizationLevel.Debug)
            .WithAssemblyIdentityComparer(DesktopAssemblyIdentityComparer.Default);

    // Resolves a reference path to a MetadataReference, or null if it should be
    // skipped. Invoked once per unique path via RefCache.GetOrAdd.
    private static MetadataReference? BuildReference(string path)
    {
        if (!File.Exists(path)) return null;
        if (!IsManagedAssembly(path)) return null;                   // skip native DLLs -> CS0009
        if (Excluded.Contains(Path.GetFileName(path))) return null;  // skip dupes -> CS0433
        return MetadataReference.CreateFromFile(path);
    }

    // A managed assembly has a CLI/CorHeader in its PE; native DLLs don't.
    private static bool IsManagedAssembly(string path)
    {
        try
        {
            using var fs = File.OpenRead(path);
            using var pe = new PEReader(fs);
            return pe.HasMetadata && pe.PEHeaders.CorHeader != null;
        }
        catch
        {
            return false;
        }
    }
}

internal sealed class CompileRequest
{
    public string plugin { get; set; } = "";
    public string[] references { get; set; } = Array.Empty<string>();
    public string[] preprocessor { get; set; } = Array.Empty<string>();
}

internal sealed class CompileResponse
{
    public bool success { get; set; }
    public bool errored { get; set; }
    public string[] errors { get; set; } = Array.Empty<string>();
}
