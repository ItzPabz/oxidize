use std::path::PathBuf;

#[derive(Debug)]
pub struct Plugin {
    pub path: PathBuf,
    pub name: String,
    pub author: String,
}

impl Plugin {
    pub fn from_path(path: PathBuf) -> std::io::Result<Plugin> {
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
