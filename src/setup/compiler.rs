use crate::setup::oxide;
use directories::ProjectDirs;
use indicatif::ProgressBar;
use oxide::Release;
use std::path::PathBuf;
use std::time::Duration;

pub fn compiler_dir(dirs: &ProjectDirs) -> PathBuf {
    dirs.data_local_dir().join("tools").join("compiler")
}

pub fn ensure_compiler(dirs: &ProjectDirs) -> Result<(), Box<dyn std::error::Error>> {
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
