pub mod assembly;
pub mod compiler;
pub mod oxide;
use std::{process::Command, time::Duration};

use directories::ProjectDirs;
use indicatif::ProgressBar;

pub fn prereq_checks(dirs: &ProjectDirs) {
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
