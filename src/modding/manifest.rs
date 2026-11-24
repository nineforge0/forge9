//! `mod.toml` format (see docs/mod_format.md). Relative paths resolve against the manifest's directory.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const TEMPLATE: &str = r#"[mod]
name = "example-mod"
version = "0.1.0"
description = "Example mod"
target_archive = "common.rpf"

# One table per patched entry: path from `forge9 list --recursive`, sha256 of its original
# inflated payload (apply refuses any other version), then operations applied in order.
[[patch]]
path = "6"
base_sha256 = "0000000000000000000000000000000000000000000000000000000000000000"

[[patch.op]]
type = "xml"
path = "Config/Limits/MaxCount"
attr = "value"
value = "64"

[[patch.op]]
type = "text"
find = "<Enabled>false</Enabled>"
replace = "<Enabled>true</Enabled>"
count = 1
"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(rename = "mod")]
    pub info: ModInfo,
    /// v1 whole-entry replacements; `parse` moves them into `patch`.
    #[serde(default)]
    replace: Vec<Replace>,
    #[serde(default)]
    pub patch: Vec<Patch>,
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
struct Replace {
    entry: usize,
    file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    /// Entry path, `a/b` descending into nested archives.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha256: Option<String>,
    /// Whole new payload, for content the mod author created; ops then apply to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    #[serde(default, rename = "op", skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Op {
    /// Replace every occurrence of `find`, which must occur exactly `count` times.
    Text { find: String, replace: String, #[serde(default = "one")] count: usize },
    /// Set the text (or `attr`) of the one element at `path`, e.g. `Root/Items/Item[2]/value`.
    Xml {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attr: Option<String>,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect: Option<String>,
    },
    /// Replace the hex bytes `expect` at `offset` with the hex bytes `data`.
    Bytes { offset: usize, expect: String, data: String },
}

fn one() -> usize { 1 }

impl Manifest {
    pub fn parse(text: &str) -> Result<Self> {
        let mut m: Self = toml::from_str(text)?;
        let v1 = std::mem::take(&mut m.replace).into_iter()
            .map(|r| Patch { path: r.entry.to_string(), base_sha256: None, file: Some(r.file), ops: vec![] });
        m.patch.splice(0..0, v1);
        Ok(m)
    }

    /// Loads a manifest and makes its paths relative to the manifest's directory.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut m = Self::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new(""));
        m.info.target_archive = base.join(&m.info.target_archive);
        for f in m.patch.iter_mut().filter_map(|p| p.file.as_mut()) { *f = base.join(&*f); }
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
        assert_eq!((m.patch[0].path.as_str(), m.patch[0].ops.len()), ("6", 2));
        assert!(matches!(&m.patch[0].ops[1], Op::Text { count: 1, .. }));
        assert!(Manifest::parse("[mod]\nname = \"x\"").is_err());
    }

    #[test]
    fn v1_replace_becomes_a_file_patch() {
        let m = Manifest::parse("[mod]\nname = \"x\"\nversion = \"1\"\ntarget_archive = \"a.rpf\"\n[[replace]]\nentry = 6\nfile = \"f.xml\"\n").unwrap();
        assert_eq!(m.patch, [Patch { path: "6".into(), base_sha256: None, file: Some("f.xml".into()), ops: vec![] }]);
    }
}
