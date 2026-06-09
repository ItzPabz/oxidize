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
                    method.Attributes = (method.Attributes & ~MethodAttributes.MemberAccessMask) | MethodAttributes.Public;

                foreach (var field in type.Fields)
                    field.Attributes = (field.Attributes & ~FieldAttributes.FieldAccessMask) | FieldAttributes.Public;
            }

            module.Write(path);
            return 0;
        }
        catch
        {
            return 1;
        }
    }

    private static CompileResponse Run(CompileRequest req)
    {
        string source = File.ReadAllText(req.plugin);
        var tree = CSharpSyntaxTree.ParseText(source);

        var references = req.references
            .Where(File.Exists)
            .Where(IsManagedAssembly)                                  // skip native DLLs -> CS0009
            .Where(r => !Excluded.Contains(Path.GetFileName(r)))       // skip dupes -> CS0433
            .Select(r => (MetadataReference)MetadataReference.CreateFromFile(r))
            .ToList();

        var compilation = CSharpCompilation.Create(
            assemblyName: Path.GetFileNameWithoutExtension(req.plugin),
            syntaxTrees: new[] { tree },
            references: references,
            options: new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));

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
}

internal sealed class CompileResponse
{
    public bool success { get; set; }
    public bool errored { get; set; }
    public string[] errors { get; set; } = Array.Empty<string>();
}
