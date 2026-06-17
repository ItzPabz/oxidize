use crate::branch::Branch;
use directories::ProjectDirs;
use indicatif::ProgressBar;
use std::time::Duration;

#[derive(serde::Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

pub fn ensure_oxide(dirs: &ProjectDirs, branch: &Branch) -> Result<(), Box<dyn std::error::Error>> {
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
