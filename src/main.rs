use clap::Parser;
use console::style;
use dialoguer::Confirm;
use directories::ProjectDirs;
use indicatif::{ProgressBar, ProgressStyle};
use netcorehost::{nethost, pdcstr, pdcstring::PdCString};
use rayon::prelude::*;
use std::ffi::{CStr, CString};
use std::io::IsTerminal;
use std::io::{BufRead, BufReader};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;
use std::{
    fs::DirEntry,
    path::{Path, PathBuf},
};

#[derive(serde::Serialize)]
struct CompileRequest<'a> {
    plugin: String,
    references: &'a [String],
    preprocessor: &'a [String],
}
#[derive(serde::Deserialize)]
struct CompileResponse {
    success: bool,
    errored: bool,
    errors: Vec<String>,
}

#[derive(Parser, Debug)]
#[command(version, about = "Oxide plugin compile checker")]
struct Args {
    #[arg(value_name = "PATH")]
    path: PathBuf,

    // TODO: future `--output json` / website export (like Spark)
    #[arg(short, long, default_value_t = false)]
    yes: bool,

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
    author: String, // TODO: Add author print.
}

impl Plugin {
    fn from_path(path: PathBuf) -> std::io::Result<Plugin> {
        let contents = std::fs::read_to_string(&path)?;

        let info_line = contents.lines().find(|line| line.contains("[Info("));

        let (name, author) = info_line
            .and_then(|line| {
                let parts: Vec<&str> = line.split('"').collect();
                Some((parts.get(1)?.to_string(), parts.get(3)?.to_string()))
            })
            .unwrap_or_else(|| {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                (stem.to_string(), "Unknown".to_string())
            });

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

    fn preprocessor_symbols(&self) -> Vec<String> {
        let branch = match self {
            Branch::Release => "PUBLIC",
            Branch::Staging => "STAGING",
        };
        vec![
            "OXIDE".into(),
            "OXIDEMOD".into(),
            "RUST".into(),
            format!("RUST_{branch}"),
            "OXIDE_PUBLICIZED".into(),
        ]
    }
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

fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Args = Args::parse();

    if !std::io::stdin().is_terminal() {
        eprintln!("oxidize must be run from the cli");
        return Ok(ExitCode::FAILURE);
    }

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

    ensure_libraries(&dirs, &branch, args.yes)?;
    ensure_oxide(&dirs, &branch)?;
    ensure_compiler(&dirs)?;

    if !args.path.exists() {
        eprintln!("path not found: {}", args.path.display());
        return Ok(ExitCode::FAILURE);
    }

    let plugins = if args.path.is_file() {
        if args.path.extension().and_then(|e| e.to_str()) != Some("cs") {
            eprintln!("not a .cs plugin: {}", args.path.display());
            return Ok(ExitCode::FAILURE);
        }
        vec![args.path.clone()]
    } else {
        traverse(&args.path, "cs", false)?
    };

    let libraries = traverse(&library_path, "dll", true)?;

    if plugins.is_empty() {
        eprintln!("no plugins found in {}", args.path.display());
        return Ok(ExitCode::SUCCESS);
    }

    let mut parsed = Vec::new();

    for path in plugins {
        let plugin = Plugin::from_path(path)?;
        parsed.push(plugin);
    }

    let installed_compiler = compiler_dir(&dirs);
    let symbols = branch.preprocessor_symbols();

    let (mut ok, mut fail, mut err) = (0, 0, 0);

    for (name, author, result) in compile_all(&parsed, &libraries, &installed_compiler, &symbols)? {
        match result {
            CompileResult::Success => {
                ok += 1;
                println!(
                    "{} {name} by {author}",
                    style(format!("{:<5}", "OK")).green()
                );
            }
            CompileResult::Failure(errors) => {
                fail += 1;
                println!(
                    "{} {name} by {author} ({} errors)",
                    style(format!("{:<5}", "FAIL")).yellow(),
                    errors.len()
                );
                for e in &errors {
                    println!("        {e}");
                }
            }
            CompileResult::Errored(msg) => {
                err += 1;
                println!(
                    "{} {name} by {author}: {msg}",
                    style(format!("{:<5}", "ERROR")).red()
                )
            }
        }
    }

    println!(
        "{} OK  {} FAIL  {} ERROR",
        style(ok).green(),
        style(fail).yellow(),
        style(err).red(),
    );

    if fail + err > 0 {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
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
    let exe_name = format!("DepotDownloader{}", std::env::consts::EXE_SUFFIX);
    let tool_dirs = dirs.data_local_dir().join("tools");
    let depot = tool_dirs.join(exe_name);

    let progress = ProgressBar::new_spinner();
    progress.set_message("Checking prerequisites...");
    progress.enable_steady_tick(Duration::from_millis(100));

    if !depot.exists() {
        progress.finish_and_clear();
        eprintln!("DepotDownloader not found at {}", depot.display());
        eprintln!("Download it from: https://github.com/SteamRE/DepotDownloader/releases");
        eprintln!("and place it there, then re-run.");
        std::process::exit(1);
    }
    progress.set_message("DepotDownloader is installed");

    let dotnet = Command::new("dotnet").arg("--version").output();

    match dotnet {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout);
            progress.set_message(format!(".NET {} detected", version.trim()));
        }
        _ => {
            progress.finish_and_clear();
            eprintln!(" .NET SDK not found. Install it from https://dotnet.microsoft.com/download");
            std::process::exit(1);
        }
    }

    progress.finish_with_message("Prerequisites working!");
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

fn ensure_libraries(dirs: &ProjectDirs, branch: &Branch, yes: bool) -> std::io::Result<()> {
    let library_path = dirs.data_local_dir().join(branch.folder_name());
    let assembly = library_path.join("RustDedicated_Data/Managed/Assembly-CSharp.dll");

    let progress = ProgressBar::new_spinner();
    progress.set_message("Checking for Rust Assemblies...");
    progress.enable_steady_tick(Duration::from_millis(100));

    if assembly.exists() {
        let msg = match installed_manifest(&library_path) {
            Some(manifest) => format!("Libraries installed (manifest {manifest})"),
            None => "Libraries installed".to_string(),
        };
        progress.finish_with_message(msg);
    } else {
        if !yes {
            let confirmation = progress
                .suspend(|| {
                    Confirm::new()
                        .with_prompt("Rust reference assemblies are missing. Would you like to download it now?")
                        .default(true)
                        .interact()
                })
                .map_err(std::io::Error::other)?;

            if !confirmation {
                progress.finish_and_clear();
                return Err(std::io::Error::other("download declined by user"));
            }
        }

        progress.set_message("Libraries missing. Installing now.");
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
            .arg(&filelist)
            .stdout(Stdio::piped());

        if let Branch::Staging = branch {
            depot_cmd.arg("-branch").arg("staging");
        }

        progress.set_message("Downloading Rust Assemblies...");

        let mut child = depot_cmd.spawn()?;

        let stdout = child.stdout.take().expect("stdout was piped");
        let reader = BufReader::new(stdout);
        let mut log: Vec<String> = Vec::new();

        for line in reader.lines() {
            let line = line?;
            progress.set_message(line.clone());
            log.push(line);
        }

        let status = child.wait()?;
        if !status.success() {
            progress.finish_and_clear();
            for line in &log {
                eprintln!("{line}");
            }
            return Err(std::io::Error::other("DepotDownloader failed"));
        }

        progress.finish_with_message("Rust assemblies installed successfully");
    }
    Ok(())
}

fn ensure_oxide(dirs: &ProjectDirs, branch: &Branch) -> Result<(), Box<dyn std::error::Error>> {
    let library_path = dirs.data_local_dir().join(branch.folder_name());
    let assembly = library_path.join("RustDedicated_Data/Managed/Oxide.Rust.dll");

    let progress = ProgressBar::new_spinner();
    progress.set_message("Checking for Oxide...");
    progress.enable_steady_tick(Duration::from_millis(100));

    if assembly.exists() {
        progress.finish_with_message("Oxide installed");
    } else {
        progress.set_message("Downloading Oxide...");
        let mut resp =
            ureq::get("https://api.github.com/repos/OxideMod/Oxide.Rust/releases/latest")
                .header("User-Agent", "oxidize")
                .call()?;
        let release: Release = resp.body_mut().read_json()?;
        progress.set_message(format!("Downloading Oxide {}…", release.tag_name));

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == "Oxide.Rust.zip")
            .ok_or("Oxide.Rust.zip not found")?;

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

        progress.finish_with_message("Oxide installed successfully");
    }

    Ok(())
}

fn compiler_dir(dirs: &ProjectDirs) -> PathBuf {
    dirs.data_local_dir().join("tools").join("compiler")
}

fn ensure_compiler(dirs: &ProjectDirs) -> Result<(), Box<dyn std::error::Error>> {
    let compiler_path = compiler_dir(dirs);
    let assembly = compiler_path.join("OxideCompiler.dll");

    let progress = ProgressBar::new_spinner();
    progress.set_message("Checking for OxideCompiler...");
    progress.enable_steady_tick(Duration::from_millis(100));

    if assembly.exists() {
        progress.finish_with_message("OxideCompiler installed");
    } else {
        progress.set_message("Downloading OxideCompiler...");
        std::fs::create_dir_all(&compiler_path)?;

        let mut resp = ureq::get("https://api.github.com/repos/ItzPabz/oxidize/releases/latest")
            .header("User-Agent", "oxidize")
            .call()?;
        let release: Release = resp.body_mut().read_json()?;
        progress.set_message(format!("Downloading OxideCompiler {}…", release.tag_name));

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == "OxideCompiler.zip")
            .ok_or("OxideCompiler.zip not found")?;

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

        progress.finish_with_message("OxideCompiler installed successfully");
    }

    Ok(())
}

fn compile_all(
    plugins: &[Plugin],
    references: &[PathBuf],
    compiler_dir: &Path,
    preprocessor: &[String],
) -> Result<Vec<(String, String, CompileResult)>, Box<dyn std::error::Error>> {
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

    let progress = ProgressBar::new(plugins.len() as u64);
    progress.set_style(ProgressStyle::with_template(
        "[{pos}/{len}] {bar:40.166/white} {msg}",
    )?);

    // The netcorehost delegates are Send + Sync but not Copy. Deref them once into
    // plain fn pointers (which are Copy + Send + Sync) so the parallel closure can
    // call them freely without capturing the !Sync loader/context.
    let compile_fn: extern "system" fn(*const u8) -> *mut u8 = *compile;
    let free_fn: extern "system" fn(*mut u8) = *free_result;

    // Each Roslyn compilation is independent and CPU-bound, so compile across cores.
    // Cap concurrency: every in-flight compilation holds its own working set
    // (symbols + emit buffers over all references), so unbounded parallelism would
    // spike memory on high-core machines for little extra speed.
    let jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(jobs).build()?;

    // par_iter over a slice is order-preserving, so results still line up with plugins.
    let results: Vec<(String, String, CompileResult)> = pool.install(|| {
        plugins
            .par_iter()
            .map(|plugin| {
                progress.set_message(plugin.name.clone());
                let request = CompileRequest {
                    plugin: plugin.path.to_string_lossy().into_owned(),
                    references: &refs,
                    preprocessor,
                };
                let result = compile_one(compile_fn, free_fn, &request);
                progress.inc(1);
                (plugin.name.clone(), plugin.author.clone(), result)
            })
            .collect()
    });
    progress.finish();
    Ok(results)
}

// Compiles a single plugin via the C# bridge. Infallible by design: any
// marshaling/parse failure becomes CompileResult::Errored so one bad plugin can't
// abort the whole batch, and the C# allocation is always freed before any fallible
// step so the error path can't leak it.
fn compile_one(
    compile_fn: extern "system" fn(*const u8) -> *mut u8,
    free_fn: extern "system" fn(*mut u8),
    request: &CompileRequest<'_>,
) -> CompileResult {
    let json = match serde_json::to_string(request).map(CString::new) {
        Ok(Ok(json)) => json,
        Ok(Err(e)) => return CompileResult::Errored(e.to_string()),
        Err(e) => return CompileResult::Errored(e.to_string()),
    };

    let ptr = compile_fn(json.as_ptr() as *const u8);
    // Copy the response out before freeing; never hold the borrow across free_fn.
    let resp_json = unsafe { CStr::from_ptr(ptr as *const i8) }
        .to_str()
        .map(str::to_owned);
    free_fn(ptr);

    let resp_json = match resp_json {
        Ok(s) => s,
        Err(e) => return CompileResult::Errored(e.to_string()),
    };

    match serde_json::from_str::<CompileResponse>(&resp_json) {
        Err(e) => CompileResult::Errored(e.to_string()),
        Ok(resp) if resp.errored => CompileResult::Errored(resp.errors.join("; ")),
        Ok(resp) if resp.success => CompileResult::Success,
        Ok(resp) => CompileResult::Failure(resp.errors),
    }
}
