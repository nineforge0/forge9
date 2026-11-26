use forge9::rpf9::{crypto::CryptoContext, reader::Archive};
use forge9::formats::drawable;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let crypto = CryptoContext::load(Path::new("data"))?;
    let raw = std::fs::read(&args[1])?;
    let outer = Archive::open(&raw, &crypto)?;
    let idx: usize = args[2].parse()?;
    let nested_raw = outer.read_entry(&outer.entries()[idx])?;
    let inner = Archive::open(&nested_raw, &crypto)?;
    eprintln!("{} entries in nested #{idx}", inner.entries().len());

    let keywords = ["head","uppr","lowr","feet","hair","accs","hand","teef",
        "task","jbib","legs","player","lucia","jason","ghost","p_zero","p_one"];

    for entry in inner.entries() {
        let blocks = match inner.read_entry_blocks(entry) {
            Ok(b) => b, Err(_) => continue,
        };
        if blocks.is_empty() { continue; }
        let sys = &blocks[0];
        if !drawable::looks_like(sys) && drawable::dictionary(sys).is_none() { continue; }
        let labels = drawable::labels(sys);
        for label in &labels {
            let low = label.to_lowercase();
            let star = if keywords.iter().any(|w| low.contains(w)) { "*** " } else { "" };
            println!("{star}#{}\t{label}", entry.index);
        }
    }
    Ok(())
}
