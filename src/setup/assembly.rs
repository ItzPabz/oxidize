use crate::ProjectDirs;
use crate::branch::Branch;
use dialoguer::Confirm;
use indicatif::ProgressBar;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn ensure_assemblies(dirs: &ProjectDirs, branch: &Branch, yes: bool) -> std::io::Result<()> {
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
