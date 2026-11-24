//! Patch operations on one entry's payload, and deriving them from an edited copy (`mod make`).

use anyhow::{bail, ensure, Context, Result};
use std::collections::HashMap;
use std::ops::Range;
use crate::modding::manifest::Op;

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    to_hex(&sha2::Sha256::digest(data))
}

/// Applies `op` to `data`, failing if its precondition does not hold.
pub fn apply(op: &Op, data: &[u8]) -> Result<Vec<u8>> {
    match op {
        Op::Text { find, replace, count } => {
            ensure!(!find.is_empty(), "text: empty find");
            let hits = matches(data, find.as_bytes());
            ensure!(hits.len() == *count, "text: expected {count} match(es) of {:?}, found {}", short(find), hits.len());
            let mut out = Vec::with_capacity(data.len());
            let mut at = 0;
            for h in hits {
                out.extend(&data[at..h]);
                out.extend(replace.as_bytes());
                at = h + find.len();
            }
            out.extend(&data[at..]);
            Ok(out)
        }
        Op::Xml { path, attr, value, expect } => {
            let r = xml_value(data, path, attr.as_deref())?;
            if let Some(e) = expect {
                let have = &data[r.clone()];
                ensure!(have == e.as_bytes(), "xml {}: expected {e:?}, found {:?}", label(path, attr), String::from_utf8_lossy(have));
            }
            Ok(splice(data, r, value.as_bytes()))
        }
        Op::Bytes { offset, expect, data: new } => {
            let (expect, new) = (hex(expect)?, hex(new)?);
            let r = *offset..offset + expect.len();
            ensure!(data.get(r.clone()) == Some(&expect[..]), "bytes at {offset:#x}: original bytes differ");
            Ok(splice(data, r, &new))
        }
    }
}

/// One line for `mod verify`.
pub fn describe(op: &Op) -> String {
    match op {
        Op::Text { find, replace, count } => format!("text {:?} -> {:?} ({count}x)", short(find), short(replace)),
        Op::Xml { path, attr, value, .. } => format!("xml {} = {:?}", label(path, attr), short(value)),
        Op::Bytes { offset, expect, data } => format!("bytes at {offset:#x}: {} -> {} bytes", expect.len() / 2, data.len() / 2),
    }
}

/// Ops turning `old` into `new`: XML value sets when only element text and attribute values
/// differ, else one text (or, for binary, bytes) op over the changed span.
pub fn make(old: &[u8], new: &[u8]) -> Result<Vec<Op>> {
    ensure!(old != new, "payloads are identical");
    let replay = |ops: &[Op]| ops.iter().try_fold(old.to_vec(), |d, op| apply(op, &d));
    let ops = xml_ops(old, new).filter(|ops| replay(ops).is_ok_and(|r| r == new)).unwrap_or_else(|| vec![span_op(old, new)]);
    ensure!(replay(&ops)? == new, "generated ops do not reproduce the edited payload");
    Ok(ops)
}

fn xml_ops(old: &[u8], new: &[u8]) -> Option<Vec<Op>> {
    let (a, b) = (elements(old).ok()?, elements(new).ok()?);
    let same = |x: &Elem, y: &Elem| (&x.name, x.parent, x.index, x.attrs.len()) == (&y.name, y.parent, y.index, y.attrs.len());
    if a.len() != b.len() || !a.iter().zip(&b).all(|(x, y)| same(x, y)) { return None; }
    let text = |d: &[u8], r: &Range<usize>| String::from_utf8(d[r.clone()].to_vec()).ok();
    let mut ops = vec![];
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        let mut set = |attr: Option<&str>, r: &Range<usize>, s: &Range<usize>| -> Option<()> {
            if old[r.clone()] != new[s.clone()] {
                ops.push(Op::Xml { path: path_of(&a, i), attr: attr.map(String::from), value: text(new, s)?, expect: Some(text(old, r)?) });
            }
            Some(())
        };
        for ((n, r), (m, s)) in x.attrs.iter().zip(&y.attrs) {
            if n != m { return None; }
            set(Some(n), r, s)?;
        }
        if let (Some(r), Some(s)) = (&x.text, &y.text) { set(None, r, s)?; }
    }
    Some(ops)
}

/// One op replacing the span between the common prefix and suffix.
fn span_op(old: &[u8], new: &[u8]) -> Op {
    let p = old.iter().zip(new).take_while(|(x, y)| x == y).count();
    let s = old.iter().rev().zip(new.iter().rev()).take_while(|(x, y)| x == y).count().min(old.len().min(new.len()) - p);
    if std::str::from_utf8(old).is_err() || std::str::from_utf8(new).is_err() {
        return Op::Bytes { offset: p, expect: to_hex(&old[p..old.len() - s]), data: to_hex(&new[p..new.len() - s]) };
    }
    // ponytail: one span, so far-apart edits carry everything between them; a line diff would split them
    let (mut a, mut b) = (p, old.len() - s);
    loop {
        let find = std::str::from_utf8(&old[a..b]);
        let replace = std::str::from_utf8(&new[a..new.len() - (old.len() - b)]);
        if let (Ok(f), Ok(r)) = (find, replace) {
            if !f.is_empty() && matches(old, f.as_bytes()).len() == 1 || (a, b) == (0, old.len()) {
                return Op::Text { find: f.into(), replace: r.into(), count: 1 };
            }
        }
        // Widen to whole lines, then one more line each side, until `find` is unique.
        if a > 0 { a = old[..a - 1].iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1); }
        if b < old.len() { b = old[b..].iter().position(|&c| c == b'\n').map_or(old.len(), |i| b + i + 1); }
    }
}

/// Start offsets of the non-overlapping occurrences of `needle`.
fn matches(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(p) = hay[i..].windows(needle.len()).position(|w| w == needle) {
        out.push(i + p);
        i += p + needle.len();
    }
    out
}

fn splice(data: &[u8], r: Range<usize>, with: &[u8]) -> Vec<u8> {
    [&data[..r.start], with, &data[r.end..]].concat()
}

fn hex(s: &str) -> Result<Vec<u8>> {
    let d: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    ensure!(d.len() % 2 == 0, "odd hex length in {s:?}");
    d.chunks(2).map(|c| u8::from_str_radix(std::str::from_utf8(c)?, 16).with_context(|| format!("bad hex in {s:?}"))).collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn short(s: &str) -> String {
    if s.chars().count() <= 40 { s.into() } else { format!("{}...", s.chars().take(40).collect::<String>()) }
}

fn label(path: &str, attr: &Option<String>) -> String {
    attr.as_ref().map_or(path.into(), |a| format!("{path}@{a}"))
}

struct Elem {
    name: String,
    parent: Option<usize>,
    /// Position among the parent's children of the same name, and how many there are.
    index: usize,
    siblings: usize,
    /// Attribute value ranges, between the quotes.
    attrs: Vec<(String, Range<usize>)>,
    /// Content range of an element with an end tag and no child elements.
    text: Option<Range<usize>>,
}

/// Elements of `doc` in document order. Not a validating parser: just enough to locate values.
fn elements(doc: &[u8]) -> Result<Vec<Elem>> {
    let find = |from: usize, s: &[u8]| doc[from..].windows(s.len()).position(|w| w == s).map(|p| from + p + s.len());
    let mut out: Vec<Elem> = vec![];
    // (element, content start, has child elements)
    let mut open: Vec<(usize, usize, bool)> = vec![];
    let mut counts: HashMap<(Option<usize>, String), usize> = HashMap::new();
    let mut i = 0;
    while let Some(p) = doc[i..].iter().position(|&c| c == b'<').map(|p| i + p) {
        let rest = &doc[p..];
        let skip = [(&b"<!--"[..], &b"-->"[..]), (b"<![CDATA[", b"]]>"), (b"<?", b"?>"), (b"<!", b">")];
        if let Some((_, end)) = skip.iter().find(|(start, _)| rest.starts_with(start)) {
            i = find(p, end).context("unterminated markup")?;
            continue;
        }
        if rest.starts_with(b"</") {
            let (e, start, child) = open.pop().context("unmatched end tag")?;
            if !child { out[e].text = Some(start..p); }
            i = find(p, b">").context("unterminated end tag")?;
            continue;
        }
        let ws = |c: &u8| c.is_ascii_whitespace();
        let mut j = p + 1 + doc[p + 1..].iter().position(|c| ws(c) || b"/>".contains(c)).context("unterminated tag")?;
        let name = String::from_utf8_lossy(&doc[p + 1..j]).into_owned();
        let mut attrs = vec![];
        let closed = loop {
            while doc.get(j).is_some_and(ws) { j += 1; }
            match doc.get(j) {
                Some(b'>') => break false,
                Some(b'/') if doc.get(j + 1) == Some(&b'>') => { j += 1; break true; }
                None => bail!("unterminated tag <{name}"),
                _ => {}
            }
            let eq = j + doc[j..].iter().position(|&c| c == b'=').with_context(|| format!("attribute without value in <{name}"))?;
            let attr = String::from_utf8_lossy(doc[j..eq].trim_ascii()).into_owned();
            let q = eq + 1 + doc[eq + 1..].iter().position(|c| !ws(c)).context("unterminated tag")?;
            ensure!(matches!(doc[q], b'"' | b'\''), "unquoted attribute {attr} in <{name}");
            let end = q + 1 + doc[q + 1..].iter().position(|&c| c == doc[q]).context("unterminated attribute")?;
            attrs.push((attr, q + 1..end));
            j = end + 1;
        };
        let parent = open.last_mut().map(|o| { o.2 = true; o.0 });
        let n = counts.entry((parent, name.clone())).or_default();
        out.push(Elem { name, parent, index: *n, siblings: 0, attrs, text: None });
        *n += 1;
        if !closed { open.push((out.len() - 1, j + 1, false)); }
        i = j + 1;
    }
    ensure!(open.is_empty(), "unclosed element <{}>", out[open[0].0].name);
    for e in &mut out { e.siblings = counts[&(e.parent, e.name.clone())]; }
    Ok(out)
}

/// The value range at `path` (`a/b[1]/c`, `[i]` counting same-name siblings from 0): the element's
/// text, or its attribute `attr`. A step without `[i]` matches any; the path must match one element.
fn xml_value(doc: &[u8], path: &str, attr: Option<&str>) -> Result<Range<usize>> {
    let steps = path.split('/').map(|s| match s.strip_suffix(']').and_then(|s| s.split_once('[')) {
        Some((n, i)) => Ok((n, Some(i.parse::<usize>().with_context(|| format!("bad index in {s:?}"))?))),
        None => Ok((s, None)),
    }).collect::<Result<Vec<_>>>()?;
    let elems = elements(doc).context("parsing XML")?;
    let at = |mut cur: Option<usize>| {
        for (name, idx) in steps.iter().rev() {
            let Some(e) = cur.map(|c| &elems[c]) else { return false };
            if e.name != *name || idx.is_some_and(|i| i != e.index) { return false; }
            cur = e.parent;
        }
        cur.is_none()
    };
    let hits: Vec<&Elem> = (0..elems.len()).filter(|&i| at(Some(i))).map(|i| &elems[i]).collect();
    let e = match hits[..] {
        [e] => e,
        [] => bail!("xml {path}: no such element"),
        _ => bail!("xml {path}: {} elements match; pick one with name[i]", hits.len()),
    };
    match attr {
        Some(a) => e.attrs.iter().find(|(n, _)| n == a).map(|(_, r)| r.clone()).with_context(|| format!("xml {path}: no attribute {a}")),
        None => e.text.clone().with_context(|| format!("xml {path}: element has child elements or no end tag")),
    }
}

/// A path that `xml_value` resolves to element `i` alone.
fn path_of(elems: &[Elem], i: usize) -> String {
    let mut parts = vec![];
    let mut cur = Some(i);
    while let Some(e) = cur.map(|c| &elems[c]) {
        parts.push(if e.siblings > 1 { format!("{}[{}]", e.name, e.index) } else { e.name.clone() });
        cur = e.parent;
    }
    parts.reverse();
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\u{feff}<?xml version=\"1.0\"?>\n<!-- c <x> -->\n<Root a='1'>\n  <Items>\n    <Item><v>1</v></Item>\n    <Item><v>2</v><w value=\"a>b\" /></Item>\n  </Items>\n  <Name><![CDATA[x]]></Name>\n</Root>\n";

    fn xml(path: &str, attr: Option<&str>, value: &str, expect: Option<&str>) -> Op {
        Op::Xml { path: path.into(), attr: attr.map(Into::into), value: value.into(), expect: expect.map(Into::into) }
    }

    fn run(op: Op) -> Result<String> {
        Ok(String::from_utf8(apply(&op, DOC.as_bytes())?).unwrap())
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn text_requires_exact_count() {
        let op = |count| Op::Text { find: "<v>".into(), replace: "<v >".into(), count };
        assert_eq!(run(op(2)).unwrap().matches("<v >").count(), 2);
        assert!(run(op(1)).unwrap_err().to_string().contains("expected 1 match(es) of \"<v>\", found 2"));
    }

    #[test]
    fn xml_sets_text_and_attributes_in_place() {
        let out = run(xml("Root/Items/Item[1]/v", None, "20", Some("2"))).unwrap();
        assert_eq!(out, DOC.replace("<v>2</v>", "<v>20</v>"));
        let out = run(xml("Root/Items/Item/w", Some("value"), "c", Some("a>b"))).unwrap();
        assert_eq!(out, DOC.replace("a>b", "c"));
        assert_eq!(run(xml("Root", Some("a"), "2", None)).unwrap(), DOC.replace("'1'", "'2'"));
        assert_eq!(run(xml("Root/Name", None, "y", None)).unwrap(), DOC.replace("<![CDATA[x]]>", "y"));
    }

    #[test]
    fn xml_refuses_bad_preconditions() {
        let err = |op| run(op).unwrap_err().to_string();
        assert!(err(xml("Root/Items/Item/v", None, "3", None)).contains("2 elements match"));
        assert!(err(xml("Root/Items/Item[0]/v", None, "3", Some("9"))).contains("expected \"9\", found \"1\""));
        assert!(err(xml("Root/Nope", None, "3", None)).contains("no such element"));
        assert!(err(xml("Root/Items", None, "3", None)).contains("child elements"));
        assert!(err(xml("Root", Some("b"), "3", None)).contains("no attribute b"));
    }

    #[test]
    fn bytes_check_original() {
        let op = |expect: &str| Op::Bytes { offset: 2, expect: expect.into(), data: "ff ee dd".into() };
        assert_eq!(apply(&op("0203"), &[0, 1, 2, 3, 4]).unwrap(), [0, 1, 0xff, 0xee, 0xdd, 4]);
        assert!(apply(&op("0303"), &[0, 1, 2, 3, 4]).is_err());
        assert!(apply(&op("02030405"), &[0, 1, 2, 3, 4]).is_err());
    }

    #[test]
    fn make_derives_ops_that_replay() {
        let edited = DOC.replace("<v>2</v>", "<v>5</v>").replace("'1'", "'7'");
        assert_eq!(make(DOC.as_bytes(), edited.as_bytes()).unwrap(), [
            xml("Root", Some("a"), "7", Some("1")),
            xml("Root/Items/Item[1]/v", None, "5", Some("2")),
        ]);
        // A structural edit falls back to one text op with enough context to be unique.
        let edited = DOC.replace("<v>1</v>", "<v>1</v><u/>");
        let ops = make(DOC.as_bytes(), edited.as_bytes()).unwrap();
        assert!(matches!(&ops[..], [Op::Text { find, count: 1, .. }] if find == "    <Item><v>1</v></Item>\n"), "{ops:?}");
        let ops = make(&[0, 1, 2, 0xff, 4], &[0, 1, 9, 9, 0xff, 4]).unwrap();
        assert_eq!(ops, [Op::Bytes { offset: 2, expect: "02".into(), data: "0909".into() }]);
        assert!(make(b"a", b"a").is_err());
    }
}
