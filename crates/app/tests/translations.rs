//! Every string passed to `tr(...)` in the app must be translated in every language
//! listed in po/LINGUAS. Fails the build when someone adds UI text without a translation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// Reads a Rust/PO string literal starting right after the opening quote.
fn literal(src: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

fn source_msgids() -> BTreeSet<String> {
    let src_dir = Path::new(ROOT).join("crates/app/src");
    let mut ids = BTreeSet::new();
    for entry in std::fs::read_dir(src_dir).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        for (i, _) in text.match_indices("tr(") {
            // Skip identifiers that merely end in "tr", like `attr(`.
            let prev = text[..i].chars().last();
            if prev.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            // rustfmt may put the literal on the next line.
            let rest = text[i + 3..].trim_start();
            if let Some(lit) = rest.strip_prefix('"') {
                ids.insert(literal(lit).expect("terminated literal"));
            }
        }
    }
    ids
}

#[derive(Default)]
struct Po {
    entries: BTreeMap<String, String>,
    fuzzy: BTreeSet<String>,
}

/// Minimal .po reader: msgid/msgstr with continuation lines, and fuzzy flags.
fn parse_po(text: &str) -> Po {
    let mut po = Po::default();
    let (mut id, mut msgstr, mut in_id, mut fuzzy) = (None::<String>, String::new(), false, false);
    let mut flush = |id: &mut Option<String>, msgstr: &mut String, fuzzy: &mut bool| {
        // Only an actual entry consumes the pending fuzzy flag.
        if let Some(i) = id.take() {
            if !i.is_empty() {
                if *fuzzy {
                    po.fuzzy.insert(i.clone());
                }
                po.entries.insert(i, std::mem::take(msgstr));
            }
            *fuzzy = false;
        }
        msgstr.clear();
    };
    for line in text.lines().map(str::trim) {
        // Blank lines and comments start a new entry, so close the previous one first.
        if line.is_empty() || line.starts_with('#') {
            flush(&mut id, &mut msgstr, &mut fuzzy);
            fuzzy |= line.starts_with("#,") && line.contains("fuzzy");
        } else if let Some(rest) = line.strip_prefix("msgid \"") {
            flush(&mut id, &mut msgstr, &mut fuzzy);
            id = literal(rest);
            in_id = true;
        } else if let Some(rest) = line.strip_prefix("msgstr \"") {
            msgstr = literal(rest).unwrap();
            in_id = false;
        } else if let Some(rest) = line.strip_prefix('"') {
            let more = literal(rest).unwrap();
            match (&mut id, in_id) {
                (Some(i), true) => i.push_str(&more),
                _ => msgstr.push_str(&more),
            }
        }
    }
    flush(&mut id, &mut msgstr, &mut fuzzy);
    po
}

fn languages() -> Vec<String> {
    std::fs::read_to_string(Path::new(ROOT).join("po/LINGUAS"))
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

#[test]
fn every_ui_string_is_translated() {
    let ids = source_msgids();
    assert!(ids.len() > 20, "extractor found too few strings: {ids:?}");
    for lang in languages() {
        let text = std::fs::read_to_string(Path::new(ROOT).join(format!("po/{lang}.po"))).unwrap();
        let po = parse_po(&text);
        let missing: Vec<_> = ids
            .iter()
            .filter(|id| po.entries.get(*id).is_none_or(String::is_empty) || po.fuzzy.contains(*id))
            .collect();
        assert!(
            missing.is_empty(),
            "{lang}: missing translations for {missing:#?}"
        );
        let stale: Vec<_> = po.entries.keys().filter(|k| !ids.contains(*k)).collect();
        assert!(
            stale.is_empty(),
            "{lang}: translations no longer used: {stale:#?}"
        );
    }
}

#[test]
fn po_reader_handles_continuations_and_fuzzy() {
    let po = parse_po(
        "msgid \"\"\nmsgstr \"header\"\n\nmsgid \"a \"\n\"b\"\nmsgstr \"x\"\n\"y\"\n\n#, fuzzy\nmsgid \"c\"\nmsgstr \"z\"\n",
    );
    assert_eq!(po.entries.get("a b").map(String::as_str), Some("xy"));
    assert!(po.fuzzy.contains("c"));
    assert!(!po.entries.contains_key(""));
}
