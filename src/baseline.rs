use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::timeline::Bin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineProfile {
    pub version: u8,
    pub bin_count: usize,
    pub window_secs: u64,
    pub bins: Vec<Bin>,
    pub top_tokens: Vec<TokenCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCount {
    pub token: String,
    pub count: u64,
}

impl BaselineProfile {
    pub fn new(
        bin_count: usize,
        window_secs: u64,
        bins: Vec<Bin>,
        top_tokens: Vec<TokenCount>,
    ) -> Self {
        Self {
            version: 1,
            bin_count,
            window_secs,
            bins,
            top_tokens,
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data = fs::read_to_string(path)?;
        let profile: BaselineProfile = serde_json::from_str(&data)?;
        Ok(profile)
    }
}
