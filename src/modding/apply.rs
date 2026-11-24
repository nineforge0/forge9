//! Applying and verifying a mod manifest against its target archive.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use crate::modding::manifest::Manifest;
use crate::rpf9::crypto::CryptoContext;
use crate::rpf9::reader::Archive;

/// Rebuilds `archive` (default: the manifest's target) with every replacement and writes `output`.
pub fn apply(m: &Manifest, archive: Option<&Path>, crypto: &CryptoContext, output: &Path) -> Result<()> {
    let target = archive.unwrap_or(&m.info.target_archive);
    let raw = std::fs::read(target).with_context(|| format!("reading {}", target.display()))?;
    let mut replacements = HashMap::new();
    for r in &m.replace {
        let data = std::fs::read(&r.file).with_context(|| format!("reading {}", r.file.display()))?;
        if replacements.insert(r.entry, data).is_some() { bail!("entry {} replaced twice", r.entry); }
    }
    crate::rpf9::writer::repack(&raw, crypto, &replacements, output)
}

/// Checks replacement files and, when the archive and crypto are available,
/// entry indices and sizes. Returns the number of problems found.
pub fn verify(m: &Manifest, archive: Option<&Path>, crypto: Option<&CryptoContext>) -> Result<usize> {
    let target = archive.unwrap_or(&m.info.target_archive);
    println!("{} {} → {}", m.info.name, m.info.version, target.display());
    let raw = std::fs::read(target).ok();
    let opened = match (&raw, crypto) {
        (Some(raw), Some(c)) => Some(Archive::open(raw, c)?),
        _ => {
            println!("  (archive or crypto unavailable — checking files only)");
            None
        }
    };
    let mut problems = 0;
    let mut seen = std::collections::HashSet::new();
    for r in &m.replace {
        let mut issue = |msg: String| { problems += 1; println!("  ✗ #{:<4} {}: {msg}", r.entry, r.file.display()); };
        if !seen.insert(r.entry) { issue("entry replaced twice".into()); continue; }
        let Ok(meta) = std::fs::metadata(&r.file) else { issue("file missing".into()); continue; };
        let size = meta.len();
        if size == 0 { issue("file is empty".into()); continue; }
        if size > u64::from(u32::MAX) { issue(format!("{size} bytes exceeds 4 GiB")); continue; }
        let Some(a) = &opened else { println!("  ✓ #{:<4} {} ({size} bytes)", r.entry, r.file.display()); continue; };
        let Some(e) = a.entries().get(r.entry) else { issue(format!("index out of range (archive has {})", a.entries().len())); continue; };
        // Stored size is compressed for zlib entries, so this is only a rough guide.
        let ratio = size as f64 / e.size.max(1) as f64;
        let note = if !(0.1..=10.0).contains(&ratio) { "  ⚠ size differs a lot from original" } else { "" };
        println!("  ✓ #{:<4} {} ({size} bytes, original {} bytes .{}){note}", r.entry, r.file.display(), e.size, a.sniff(e));
    }
    Ok(problems)
}
