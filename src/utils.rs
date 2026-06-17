use std::fs::DirEntry;
use std::path::{Path, PathBuf};

pub fn traverse(path: &Path, ext: &str, recurse: bool) -> std::io::Result<Vec<PathBuf>> {
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
