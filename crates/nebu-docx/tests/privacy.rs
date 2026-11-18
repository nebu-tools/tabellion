//! No committed .docx names a real person, company, machine or file path.
//!
//! The documents of the corpus are made by Word, pandoc or a script under
//! invented names. Word fills in the user, the company, the template and
//! more on its own, so this test reads every committed .docx and fails on
//! any name outside a fixed list.

use std::path::{Path, PathBuf};
use std::process::Command;

use nebu_docx::Document;
use nebu_docx::xml::Tree;

/// The invented people of the corpus, and the tool that writes the sample.
const NAMES: [&str; 3] = ["Wren Ashby", "Pip Tanager", "tabellion"];
const INITIALS: [&str; 2] = ["WA", "PT"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The .docx files git tracks, or, without git, those under `corpus/` and
/// `samples/`.
fn committed() -> Vec<PathBuf> {
    let out = Command::new("git")
        .args(["ls-files", "-z", "--", "*.docx"])
        .current_dir(root())
        .output();
    if let Ok(o) = out
        && o.status.success()
    {
        let list: Vec<PathBuf> = o
            .stdout
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| root().join(String::from_utf8_lossy(s).as_ref()))
            .collect();
        // Files added but not yet committed count too.
        let mut all = list;
        all.extend(walk(&root().join("corpus")));
        all.sort();
        all.dedup();
        return all;
    }
    let mut all = walk(&root().join("corpus"));
    all.extend(walk(&root().join("samples")));
    all
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "docx"))
                .collect()
        })
        .unwrap_or_default()
}

/// The text of the first element with this local name, in any namespace.
fn element_text(t: &Tree, local: &str) -> Vec<String> {
    let mut out = vec![];
    for n in t.descendants(t.document()).unwrap() {
        if t.name(n).unwrap().is_some_and(|m| m.local == local) {
            out.push(t.text_content(n).unwrap());
        }
    }
    out
}

/// Every value of attributes with this local name, in any namespace.
fn attribute_values(t: &Tree, local: &str) -> Vec<String> {
    let mut out = vec![];
    for n in t.descendants(t.document()).unwrap() {
        for a in t.attributes(n).unwrap() {
            if a.name.local == local {
                out.push(a.value.into_owned());
            }
        }
    }
    out
}

#[test]
fn committed_documents_name_only_invented_people() {
    let files = committed();
    assert!(files.len() >= 7, "found only {files:?}");
    let mut failures = vec![];
    for f in files {
        let name = f.strip_prefix(root()).unwrap_or(&f).display().to_string();
        let mut doc = Document::open(&f).unwrap();
        let parts: Vec<String> = doc
            .parts()
            .filter(|p| p.ends_with(".xml") || p.ends_with(".rels"))
            .map(str::to_string)
            .collect();
        let mut bad = |what: &str, v: &str| failures.push(format!("{name}: {what} {v:?}"));
        for part in &parts {
            let t = doc.xml(part).unwrap();
            let lower = part.to_ascii_lowercase();
            if lower == "docprops/core.xml" {
                for local in ["creator", "lastModifiedBy"] {
                    for v in element_text(t, local) {
                        if !v.is_empty() && !NAMES.contains(&v.as_str()) {
                            bad(local, &v);
                        }
                    }
                }
            }
            if lower == "docprops/app.xml" {
                for local in ["Company", "Manager"] {
                    for v in element_text(t, local) {
                        if !v.is_empty() && !NAMES.contains(&v.as_str()) {
                            bad(local, &v);
                        }
                    }
                }
                for v in element_text(t, "Template") {
                    if v.contains(['/', '\\', ':']) {
                        bad("template path", &v);
                    }
                }
            }
            for v in attribute_values(t, "author") {
                if !NAMES.contains(&v.as_str()) {
                    bad("author", &v);
                }
            }
            for v in attribute_values(t, "initials") {
                if !INITIALS.contains(&v.as_str()) {
                    bad("initials", &v);
                }
            }
            for v in attribute_values(t, "userId") {
                if !NAMES.contains(&v.as_str()) {
                    bad("user id", &v);
                }
            }
            for v in attribute_values(t, "providerId") {
                if v != "None" {
                    bad("identity provider", &v);
                }
            }
            // Revision session ids are hexadecimal numbers and nothing else.
            for n in t.descendants(t.document()).unwrap() {
                for a in t.attributes(n).unwrap() {
                    if a.name.local.starts_with("rsid")
                        && !a.value.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        bad("rsid", &a.value);
                    }
                }
            }
            for v in attribute_values(t, "Target") {
                if v.starts_with("file:") || v.contains(":\\") || v.contains("\\\\") {
                    bad("relationship target", &v);
                }
            }
            // No path of a machine anywhere in the text of a part.
            let bytes = doc.read_part(part).unwrap();
            let s = String::from_utf8_lossy(&bytes);
            for marker in [":\\", "\\Users\\", "/home/", "/Users/", "file:///"] {
                if s.contains(marker) {
                    bad("a path in", part);
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
