//! No committed fixture or reading names a real person, machine or path.
//!
//! The test reads every file under `corpus/` (the documents the writer
//! made and Word saved in `corpus/written/` too), `samples/` and `tools/`,
//! the tables under `crates/nebu-docx/src/view/names/`, the built-in styles
//! the writer adds, the pictures of the writer's tests, the schemas, the fuzz
//! README and the two documents at the top, as git lists them (committed or
//! added, not ignored). A .docx is read part by part. It fails on:
//!
//! - a user's path: `C:\Users\…`, `/home/…`, `/Users/…`, `/mnt/c/Users/…`,
//!   a `file:` URL or a network path `\\host\…`;
//! - a machine name: one of the form Windows gives (`DESKTOP-…`,
//!   `LAPTOP-…`), or the name of the machine the test runs on;
//! - an e-mail address outside the domains `example.org`, `example.com`
//!   and `example.net`;
//! - the name of the user the test runs as, or the name and address git is
//!   configured with;
//! - in a .docx, a person outside the invented ones (Wren Ashby, Pip
//!   Tanager) in the fields Word fills in: creator, last author, company,
//!   manager, revision and comment authors, initials, user ids and identity
//!   providers; and a template with a path or a revision session id that is
//!   not a number;
//! - in a reading (`.word.json`), an author or initials outside the
//!   invented ones.
//!
//! It cannot tell a real name in running text from an invented one; the
//! corpus scripts use the invented names only.

use std::path::{Path, PathBuf};
use std::process::Command;

use nebu_docx::Document;
use nebu_docx::xml::Tree;

/// The invented people of the corpus, and the tool that writes the sample.
const NAMES: [&str; 3] = ["Wren Ashby", "Pip Tanager", "tabellion"];
const INITIALS: [&str; 2] = ["WA", "PT"];
const MAIL_DOMAINS: [&str; 3] = ["example.org", "example.com", "example.net"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const SCANNED: [&str; 10] = [
    "corpus/",
    "samples/",
    "tools/",
    "crates/nebu-docx/src/view/names/",
    "crates/nebu-docx/src/writer/builtin.xml",
    "crates/nebu-docx/tests/data/",
    "crates/nebu-docx/schema/",
    "fuzz/README.md",
    "README.md",
    "DESIGN.md",
];

/// The files git lists under the scanned paths, committed or added.
fn fixtures() -> Vec<PathBuf> {
    let o = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(SCANNED)
        .current_dir(root())
        .output()
        .expect("git runs");
    assert!(o.status.success(), "git ls-files failed");
    let mut out: Vec<PathBuf> = o
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| root().join(String::from_utf8_lossy(s).as_ref()))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Words that name the person or machine running the test, which no
/// fixture may hold.
fn local_names() -> Vec<String> {
    let mut out = vec![];
    for var in ["USER", "USERNAME", "LOGNAME"] {
        if let Ok(v) = std::env::var(var) {
            out.push(v);
        }
    }
    for file in ["/etc/hostname", "/proc/sys/kernel/hostname"] {
        if let Ok(v) = std::fs::read_to_string(file) {
            out.push(v.trim().to_string());
        }
    }
    for key in ["user.name", "user.email"] {
        if let Ok(o) = Command::new("git")
            .args(["config", key])
            .current_dir(root())
            .output()
        {
            out.push(String::from_utf8_lossy(&o.stdout).trim().to_string());
        }
    }
    // Names too short or too common to mean anyone.
    out.retain(|v| {
        v.len() >= 4
            && !["root", "runner", "user", "localhost", "tabellion"]
                .contains(&v.to_lowercase().as_str())
    });
    out.sort();
    out.dedup();
    out
}

/// What in a text names a person, a machine or a path.
fn scan_text(what: &str, s: &str, local: &[String], bad: &mut Vec<String>) {
    let lower = s.to_lowercase();
    for marker in [":\\users\\", "/home/", "/users/", "file://", "\\\\?\\"] {
        if lower.contains(marker) {
            bad.push(format!("{what}: a path ({marker})"));
        }
    }
    // A network path: two backslashes, a host name, a backslash.
    for (i, _) in s.match_indices("\\\\") {
        let rest = &s[i + 2..];
        let host = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || ".-".contains(c)))
            .unwrap_or(rest.len());
        let before = s[..i].chars().next_back();
        if host > 0
            && rest[host..].starts_with('\\')
            && before.is_none_or(|c| c.is_whitespace() || "\"'=(".contains(c))
        {
            bad.push(format!("{what}: a network path"));
            break;
        }
    }
    for prefix in ["DESKTOP-", "LAPTOP-"] {
        if s.contains(prefix) {
            bad.push(format!("{what}: a machine name ({prefix}…)"));
        }
    }
    for name in local {
        if lower.contains(&name.to_lowercase()) {
            bad.push(format!("{what}: the name of this user or machine"));
        }
    }
    for mail in mail_addresses(s) {
        let domain = mail.rsplit('@').next().unwrap_or_default().to_lowercase();
        if !MAIL_DOMAINS.contains(&domain.as_str()) {
            bad.push(format!("{what}: an e-mail address at {domain}"));
        }
    }
}

/// Words of the form `local@domain.tld`.
fn mail_addresses(s: &str) -> Vec<String> {
    let ok = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let mut out = vec![];
    for (i, _) in s.match_indices('@') {
        let start = s[..i].rfind(|c: char| !ok(c)).map_or(0, |j| j + 1);
        let end = s[i + 1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || ".-".contains(c)))
            .map_or(s.len(), |j| i + 1 + j);
        let (local, domain) = (&s[start..i], s[i + 1..end].trim_end_matches('.'));
        if !local.is_empty() && domain.contains('.') {
            out.push(format!("{local}@{domain}"));
        }
    }
    out
}

/// The text of every element with this local name, in any namespace.
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

fn scan_docx(name: &str, path: &Path, local: &[String], bad: &mut Vec<String>) {
    let mut doc = Document::open(path).unwrap();
    let parts: Vec<String> = doc.parts().map(str::to_string).collect();
    for part in &parts {
        let bytes = doc.read_part(part).unwrap();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let what = format!("{name}, {part}");
        scan_text(&what, text, local, bad);
        if !(part.ends_with(".xml") || part.ends_with(".rels")) {
            continue;
        }
        let t = doc.xml(part).unwrap();
        let mut person = |field: &str, v: &str, allowed: &[&str]| {
            if !v.is_empty() && !allowed.contains(&v) {
                bad.push(format!("{what}: {field} {v:?}"));
            }
        };
        for local in ["creator", "lastModifiedBy", "Company", "Manager"] {
            for v in element_text(t, local) {
                person(local, &v, &NAMES);
            }
        }
        for v in attribute_values(t, "author") {
            person("author", &v, &NAMES);
        }
        for v in attribute_values(t, "userId") {
            person("user id", &v, &NAMES);
        }
        for v in attribute_values(t, "initials") {
            person("initials", &v, &INITIALS);
        }
        for v in attribute_values(t, "providerId") {
            person("identity provider", &v, &["None"]);
        }
        for v in element_text(t, "Template") {
            if v.contains(['/', '\\', ':']) {
                bad.push(format!("{what}: a template with a path"));
            }
        }
        // Revision session ids are hexadecimal numbers and nothing else.
        for n in t.descendants(t.document()).unwrap() {
            for a in t.attributes(n).unwrap() {
                if a.name.local.starts_with("rsid")
                    && !a.value.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    bad.push(format!("{what}: rsid {:?}", a.value));
                }
            }
        }
    }
}

/// Authors and initials anywhere in a reading.
fn scan_reading(name: &str, v: &serde_json::Value, bad: &mut Vec<String>) {
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            serde_json::Value::Object(m) => {
                for (k, x) in m {
                    if let Some(s) = x.as_str() {
                        let allowed: &[&str] = match k.as_str() {
                            "author" => &NAMES,
                            "initials" => &INITIALS,
                            _ => continue,
                        };
                        if !s.is_empty() && !allowed.contains(&s) {
                            bad.push(format!("{name}: {k} {s:?}"));
                        }
                    } else {
                        stack.push(x);
                    }
                }
            }
            serde_json::Value::Array(a) => stack.extend(a),
            _ => {}
        }
    }
}

#[test]
fn fixtures_name_no_real_person_machine_or_path() {
    let files = fixtures();
    let docx = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "docx"))
        .count();
    assert!(docx >= 9 && files.len() >= 30, "found only {files:?}");
    let local = local_names();
    let mut bad = vec![];
    for f in &files {
        let name = f.strip_prefix(root()).unwrap_or(f).display().to_string();
        if name.ends_with(".docx") {
            scan_docx(&name, f, &local, &mut bad);
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        if name.ends_with(".json") {
            // The strings as they read, not as JSON escapes them.
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            let mut stack = vec![&v];
            while let Some(x) = stack.pop() {
                match x {
                    serde_json::Value::String(t) => scan_text(&name, t, &local, &mut bad),
                    serde_json::Value::Array(a) => stack.extend(a),
                    serde_json::Value::Object(m) => {
                        for (k, x) in m {
                            scan_text(&name, k, &local, &mut bad);
                            stack.push(x);
                        }
                    }
                    _ => {}
                }
            }
            scan_reading(&name, &v, &mut bad);
        } else {
            scan_text(&name, &text, &local, &mut bad);
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

#[test]
fn the_scan_finds_what_it_looks_for() {
    let local = vec!["Somebody".to_string()];
    let mut bad = vec![];
    for s in [
        r"C:\Users\x\report.docx",
        "/home/x/a",
        "ask somebody",
        "mail a.b@mail.test",
        r"\\server\share\a",
        "DESKTOP-AB12CD",
    ] {
        scan_text(s, s, &local, &mut bad);
    }
    assert_eq!(bad.len(), 6, "{bad:?}");
    let mut fine = vec![];
    scan_text(
        "ok",
        "write to a@example.org; Wren Ashby",
        &local,
        &mut fine,
    );
    assert!(fine.is_empty(), "{fine:?}");
    let mut people = vec![];
    scan_reading(
        "r",
        &serde_json::json!({"revisions": [{"author": "Wren Ashby"}, {"author": "A Real Person", "initials": "ARP"}]}),
        &mut people,
    );
    assert_eq!(people.len(), 2, "{people:?}");
}
