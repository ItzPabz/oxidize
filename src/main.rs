use clap::Parser;
use console::style;
use directories::ProjectDirs;
use std::io::IsTerminal;
use std::path::{PathBuf};
use std::process::ExitCode;
mod plugin;
use plugin::Plugin;
mod branch;
use branch::Branch;
mod setup;
use setup::{
    assembly::ensure_assemblies, compiler::ensure_compiler, oxide::ensure_oxide, prereq_checks,
};
mod utils;
use utils::traverse;
mod compile;
use compile::{CompileResult, compile_all};

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

    ensure_assemblies(&dirs, &branch, args.yes)?;
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

    let installed_compiler = setup::compiler::compiler_dir(&dirs);
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
