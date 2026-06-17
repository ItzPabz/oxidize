#[derive(Debug)]
pub enum Branch {
    Release,
    Staging,
}

impl Branch {
    pub fn folder_name(&self) -> &str {
        match self {
            Branch::Release => "release",
            Branch::Staging => "staging",
        }
    }

    pub fn preprocessor_symbols(&self) -> Vec<String> {
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
