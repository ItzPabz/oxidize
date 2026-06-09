use clap::{Parser, ValueEnum};
use directories::ProjectDirs;
use netcorehost::{nethost, pdcstr, pdcstring::PdCString};
use std::process::Command;
use std::{
    fs::DirEntry,
    path::{Path, PathBuf},
};
use std::ffi::{CStr, CString};


#[derive(serde::Serialize)]
struct CompileRequest {
    plugin: String,
    references: Vec<String>,
}
#[derive(serde::Deserialize)]
struct CompileResponse {
    success: bool,
    errored: bool,
    errors: Vec<String>,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum Format {
    Human,
    Json,
}

#[derive(Parser, Debug)]
#[command(version, about = "Oxide plugin compile checker")]
struct Args {
    // TODO: See if PathBuf allows for single file and dir
    #[arg(value_name = "PATH")]
    path: PathBuf,

    #[arg(short, long, value_enum, default_value_t = Format::Human)]
    output: Format,

    #[arg(short, long, default_value_t = false)]
    staging: bool,

    // TODO: Make a watch
    #[arg(short, long, default_value_t = false)]
    watch: bool,
}

#[derive(Debug)]
enum CompileResult {
    Success,
    Failure(Vec<String>),
    Errored(String),
}

#[derive(Debug)]
struct Plugin {
    path: PathBuf,
    name: String,
    author: String,
}

impl Plugin {
    fn from_path(path: PathBuf) -> std::io::Result<Plugin> {
        let contents = std::fs::read_to_string(&path)?;

        let info_line = contents.lines().find(|line| line.contains("[Info("));

        let (name, author) = match info_line {
            Some(line) => {
                let parts: Vec<&str> = line.split('"').collect();
                (parts[1].to_string(), parts[3].to_string())
            }
            None => {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                (stem.to_string(), "Unknown".to_string())
            }
        };

        Ok(Plugin { path, name, author })
    }
}

#[derive(Debug)]
enum Branch {
    Release,
    Staging,
}

impl Branch {
    fn folder_name(&self) -> &str {
        match self {
            Branch::Release => "release",
            Branch::Staging => "staging",
        }
    }
}

#[derive(Debug)]
enum LibraryStatus {
    NotInstalled,
    UpToDate,
    Outdated { have: String, latest: String },
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}
#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Args = Args::parse();
    let branch: Branch = if args.staging {
        Branch::Staging
    } else {
        Branch::Release
    };
    let dirs = ProjectDirs::from("com", "ItzPabz", "oxidize").expect("failed to find home dir");
    prereq_checks(&dirs);

    let library_path = dirs.data_local_dir().join(branch.folder_name());
    let custom_path = library_path.join("custom");

    std::fs::create_dir_all(&custom_path)?;

    ensure_libraries(&dirs, &branch)?;
    ensure_oxide(&dirs, &branch)?;
    ensure_compiler(&dirs)?;

    let plugins = traverse(&args.path, "cs", false)?;
    let libraries = traverse(&library_path, "dll", true)?;

    println!(
        "Found {} plugin(s) and {} library file(s)",
        plugins.len(),
        libraries.len()
    );

    let mut parsed = Vec::new();

    for path in plugins {
        let plugin = Plugin::from_path(path)?;
        parsed.push(plugin);
    }

    let installed_compiler = compiler_dir(&dirs);
    for (name, result) in compile_all(&parsed, &libraries, &installed_compiler)? {
        match result {
            CompileResult::Success => println!("OK    {name}"),
            CompileResult::Failure(errors) => {
                println!("FAIL  {name} ({} errors)", errors.len());
                for e in &errors {
                    println!("        {e}");
                }
            }
            CompileResult::Errored(msg) => println!("ERROR {name}: {msg}"),
        }
    }

    Ok(())
}

fn traverse(path: &Path, ext: &str, recurse: bool) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();

    for file in path.read_dir()? {
        let entry: DirEntry = file?;
        let entry_path: PathBuf = entry.path();

        if entry_path.is_dir() {
            if entry_path.file_name().and_then(|n| n.to_str()) == Some(".DepotDownloader") {
                continue;
            }
            if recurse {
                found.extend(traverse(&entry_path, ext, recurse)?);
            }
        } else if entry_path.extension().and_then(|e| e.to_str()) == Some(ext) {
            found.push(entry_path);
        }
    }
    Ok(found)
}

fn prereq_checks(dirs: &ProjectDirs) {
    println!("Running dependency checks");

    let exe_name = format!("DepotDownloader{}", std::env::consts::EXE_SUFFIX);
    let tool_dirs = dirs.data_local_dir().join("tools");
    let depot = tool_dirs.join(exe_name);

    if !depot.exists() {
        eprintln!("DepotDownloader not found at {}", depot.display());
        eprintln!("Download it from: https://github.com/SteamRE/DepotDownloader/releases");
        eprintln!("and place it there, then re-run.");
        std::process::exit(1);
    }
    println!("  DepotDownloader is installed");

    let dotnet = Command::new("dotnet").arg("--version").output();

    match dotnet {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout);
            println!("  .NET {} detected", version.trim());
        }
        _ => {
            eprintln!(" .NET SDK not found. Install it from https://dotnet.microsoft.com/download");
            std::process::exit(1);
        }
    }
}

fn installed_manifest(library_path: &Path) -> Option<String> {
    let state = library_path.join(".DepotDownloader");
    for entry in state.read_dir().ok()? {
        let path = entry.ok()?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("manifest") {
            let stem = path.file_stem()?.to_str()?;
            return stem.split('_').nth(1).map(|s| s.to_string());
        }
    }
    None
}

fn ensure_libraries(dirs: &ProjectDirs, branch: &Branch) -> std::io::Result<()> {
    let library_path = dirs.data_local_dir().join(branch.folder_name());
    let assembly = library_path.join("RustDedicated_Data/Managed/Assembly-CSharp.dll");

    if assembly.exists() {
        println!("Libraries installed.");
    } else {
        println!("Libraries missing. Installing now.");
        let exe_name = format!("DepotDownloader{}", std::env::consts::EXE_SUFFIX);
        let tool_dirs = dirs.data_local_dir().join("tools");
        let depot = tool_dirs.join(exe_name);

        let filelist = tool_dirs.join("managed.filelist");
        std::fs::write(&filelist, "regex:^RustDedicated_Data/Managed/.*\\.dll$")?;

        let mut depot_cmd = Command::new(depot);

        depot_cmd
            .arg("-app")
            .arg("258550")
            .arg("-depot")
            .arg("258551")
            .arg("-dir")
            .arg(&library_path)
            .arg("-filelist")
            .arg(&filelist);

        if let Branch::Staging = branch {
            depot_cmd.arg("-branch").arg("staging");
        }

        depot_cmd.status()?;
    }
    Ok(())
}

fn ensure_oxide(dirs: &ProjectDirs, branch: &Branch) -> Result<(), Box<dyn std::error::Error>> {
    let library_path = dirs.data_local_dir().join(branch.folder_name());
    let assembly = library_path.join("RustDedicated_Data/Managed/Oxide.Rust.dll");
    println!("Checking for Oxide");

    if assembly.exists() {
        println!("  Oxide installed.");
    } else {
        eprintln!("  Oxide missing.");
        println!("  Installing Oxide.");
        let mut resp =
            ureq::get("https://api.github.com/repos/OxideMod/Oxide.Rust/releases/latest")
                .header("User-Agent", "oxidize")
                .call()?;
        let release: Release = resp.body_mut().read_json()?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == "Oxide.Rust.zip")
            .ok_or("Oxide.Rust.zip not found")?;
        println!("Fetching Oxide {}", release.tag_name);

        let zip = dirs.data_local_dir().join("tools").join("Oxide.Rust.zip");

        let mut download = ureq::get(&asset.browser_download_url)
            .header("User-Agent", "oxidize")
            .call()?;
        let mut reader = download.body_mut().as_reader();
        let mut out = std::fs::File::create(&zip)?;
        std::io::copy(&mut reader, &mut out)?;

        let file = std::fs::File::open(&zip)?;
        let mut archive = zip::ZipArchive::new(file)?;
        archive.extract(&library_path)?;
    }

    Ok(())
}


fn compiler_dir(dirs: &ProjectDirs) -> PathBuf {
    dirs.data_local_dir().join("tools").join("compiler")
}

fn ensure_compiler(dirs: &ProjectDirs) -> Result<(), Box<dyn std::error::Error>> {
    let compiler_path = compiler_dir(dirs);
    let assembly = compiler_path.join("OxideCompiler.dll");
    println!("Checking for OxideCompiler");

    if assembly.exists() {
        println!("  OxideCompiler installed.");
    } else {
        eprintln!("  OxideCompiler missing.");
        println!("  Installing OxideCompiler.");
        std::fs::create_dir_all(&compiler_path)?;

        let mut resp =
            ureq::get("https://api.github.com/repos/ItzPabz/oxidize/releases/latest")
                .header("User-Agent", "oxidize")
                .call()?;
        let release: Release = resp.body_mut().read_json()?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == "OxideCompiler.zip")
            .ok_or("OxideCompiler.zip not found")?;
        println!("Fetching OxideCompiler {}", release.tag_name);

        let zip = dirs
            .data_local_dir()
            .join("tools")
            .join("OxideCompiler.zip");

        let mut download = ureq::get(&asset.browser_download_url)
            .header("User-Agent", "oxidize")
            .call()?;
        let mut reader = download.body_mut().as_reader();
        let mut out = std::fs::File::create(&zip)?;
        std::io::copy(&mut reader, &mut out)?;

        let file = std::fs::File::open(&zip)?;
        let mut archive = zip::ZipArchive::new(file)?;
        archive.extract(&compiler_path)?;
    }

    Ok(())
}

fn compile_all(
    plugins: &[Plugin],
    references: &[PathBuf],
    compiler_dir: &Path,
) -> Result<Vec<(String, CompileResult)>, Box<dyn std::error::Error>> {
    let config = PdCString::from_os_str(compiler_dir.join("OxideCompiler.runtimeconfig.json"))?;
    let dll = PdCString::from_os_str(compiler_dir.join("OxideCompiler.dll"))?;

    let hostfxr = nethost::load_hostfxr()?;
    let context = hostfxr.initialize_for_runtime_config(&config)?;
    let loader = context.get_delegate_loader_for_assembly(dll)?;

    let compile = loader.get_function_with_unmanaged_callers_only::<fn(*const u8) -> *mut u8>(
        pdcstr!("OxideCompiler.Bridge, OxideCompiler"),
        pdcstr!("Compile"),
    )?;
    let free_result = loader.get_function_with_unmanaged_callers_only::<fn(*mut u8)>(
        pdcstr!("OxideCompiler.Bridge, OxideCompiler"),
        pdcstr!("FreeResult"),
    )?;
    let publicize = loader.get_function_with_unmanaged_callers_only::<fn(*const u8) -> i32>(
        pdcstr!("OxideCompiler.Bridge, OxideCompiler"),
        pdcstr!("Publicize"),
    )?;

    if let Some(acs) = references
        .iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("Assembly-CSharp.dll"))
    {
        let c = CString::new(acs.to_string_lossy().into_owned())?;
        if publicize(c.as_ptr() as *const u8) != 0 {
            eprintln!("warning: failed to publicize Assembly-CSharp.dll");
        }
    }

    let refs: Vec<String> = references
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    let mut results = Vec::new();
    for plugin in plugins {
        let request = CompileRequest {
            plugin: plugin.path.to_string_lossy().into_owned(),
            references: refs.clone(),
        };
        let json = CString::new(serde_json::to_string(&request)?)?;

        let ptr = compile(json.as_ptr() as *const u8);
        let resp_json = unsafe { CStr::from_ptr(ptr as *const i8) }
            .to_str()?
            .to_owned();
        free_result(ptr);

        let resp: CompileResponse = serde_json::from_str(&resp_json)?;
        let result = if resp.errored {
            CompileResult::Errored(resp.errors.join("; "))
        } else if resp.success {
            CompileResult::Success
        } else {
            CompileResult::Failure(resp.errors)
        };
        results.push((plugin.name.clone(), result));
    }
    Ok(results)
}