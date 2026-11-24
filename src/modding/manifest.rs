//! `mod.toml` format. Relative paths resolve against the manifest's directory.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const TEMPLATE: &str = r#"[mod]
name = "example-mod"
version = "0.1.0"
description = "Example mod"
target_archive = "common.rpf"

# One table per replaced entry: index from `forge9 list`, file relative to this manifest.
[[replace]]
entry = 6
file = "mods/my_config.xml"
"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(rename = "mod")]
    pub info: ModInfo,
    #[serde(default)]
    pub replace: Vec<Replace>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub target_archive: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replace {
    pub entry: usize,
    pub file: PathBuf,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    /// Loads a manifest and makes its paths relative to the manifest's directory.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut m = Self::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new(""));
        m.info.target_archive = base.join(&m.info.target_archive);
        for r in &mut m.replace { r.file = base.join(&r.file); }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let m = Manifest::parse(TEMPLATE).unwrap();
        assert_eq!(m.info.name, "example-mod");
        assert_eq!((m.replace[0].entry, m.replace[0].file.as_path()), (6, Path::new("mods/my_config.xml")));
        assert!(Manifest::parse("[mod]\nname = \"x\"").is_err());
    }
}
