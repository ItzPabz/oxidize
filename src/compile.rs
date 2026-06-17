use std::{
    ffi::{CStr, CString},
    path::{Path, PathBuf},
};

use indicatif::{ProgressBar, ProgressStyle};
use netcorehost::{nethost, pdcstr, pdcstring::PdCString};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::plugin::Plugin;

#[derive(serde::Serialize)]
pub struct CompileRequest<'a> {
    plugin: String,
    references: &'a [String],
    preprocessor: &'a [String],
}
#[derive(serde::Deserialize)]
pub struct CompileResponse {
    success: bool,
    errored: bool,
    errors: Vec<String>,
}

#[derive(Debug)]
pub enum CompileResult {
    Success,
    Failure(Vec<String>),
    Errored(String),
}

pub fn compile_all(
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
pub fn compile_one(
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
