//! R1-01: `docs/notes/formats.md` is the list of everything versioned between builds. From 0.1.0 on
//! a protocol name, signing domain or on-disk layout may only change on purpose, so this test fails
//! when the source and the list disagree.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `shoebox/<name>/<version>` in `text` (version: digits, optionally after a `v`).
fn names(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(i) = rest.find("shoebox/") {
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '/' || c == '-'))
            .unwrap_or(tail.len());
        let candidate = &tail[..end];
        let parts: Vec<&str> = candidate.split('/').collect();
        if parts.len() == 3
            && !parts[1].is_empty()
            && parts[2]
                .strip_prefix('v')
                .unwrap_or(parts[2])
                .parse::<u32>()
                .is_ok()
        {
            out.insert(candidate.to_string());
        }
        rest = &tail[end.max(1)..];
    }
    out
}

/// Rust files under a `src/` directory (tests may use made-up names on purpose).
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            if name == "target" || name == "node_modules" || name == "gen" || name == "tests" {
                continue;
            }
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && p.components().any(|c| c.as_os_str() == "src")
        {
            out.push(p);
        }
    }
}

#[test]
fn every_versioned_name_in_the_source_is_listed_and_nothing_else() {
    let root = workspace();
    let mut files = vec![];
    for d in ["crates", "apps", "tools"] {
        sources(&root.join(d), &mut files);
    }
    let mut in_source = BTreeSet::new();
    for f in &files {
        in_source.extend(names(&std::fs::read_to_string(f).unwrap()));
    }
    let doc = std::fs::read_to_string(root.join("docs/notes/formats.md")).unwrap();
    let listed = names(&doc);
    let missing: Vec<_> = in_source.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&in_source).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "docs/notes/formats.md is out of step with the source.\n\
         In the source but not listed: {missing:?}\n\
         Listed but not in the source: {stale:?}\n\
         A changed protocol needs the old name served for a release (CONTRIBUTING.md)."
    );
    assert!(
        in_source.contains("shoebox/loro/0"),
        "the scan found the library protocol"
    );
}

#[test]
fn the_listed_layout_is_the_one_this_build_writes() {
    let doc = std::fs::read_to_string(workspace().join("docs/notes/formats.md")).unwrap();
    let heading = format!(
        "## On disk: layout {}",
        shoebox_node::format::CURRENT_FORMAT
    );
    assert!(
        doc.contains(&heading),
        "formats.md must describe layout {} (expected the heading `{heading}`)",
        shoebox_node::format::CURRENT_FORMAT
    );
}
