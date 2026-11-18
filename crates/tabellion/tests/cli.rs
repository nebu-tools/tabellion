//! The command line as a user runs it.

use std::process::Command;

fn tabellion() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tabellion"))
}

const SAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../samples/report-template.docx"
);

#[test]
fn roundtrip_reports_an_identical_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("copy.docx");
    let o = tabellion()
        .arg("roundtrip")
        .arg(SAMPLE)
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let stdout = String::from_utf8(o.stdout).unwrap();
    let size = std::fs::metadata(SAMPLE).unwrap().len();
    assert!(stdout.ends_with(&format!("all {size} bytes\n")), "{stdout}");
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(SAMPLE).unwrap());
}

#[test]
fn roundtrip_reports_a_file_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.docx");
    std::fs::write(&bad, b"not a zip").unwrap();
    let o = tabellion()
        .arg("roundtrip")
        .arg(&bad)
        .arg(dir.path().join("out.docx"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
    let stderr = String::from_utf8(o.stderr).unwrap();
    assert!(
        stderr.starts_with("tabellion: ") && stderr.contains("not a readable .docx file"),
        "{stderr}"
    );
}

fn corpus(name: &str) -> String {
    format!("{}/../../corpus/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn run(args: &[&str]) -> (Option<i32>, String, String) {
    let o = tabellion().args(args).output().unwrap();
    (
        o.status.code(),
        String::from_utf8(o.stdout).unwrap(),
        String::from_utf8(o.stderr).unwrap(),
    )
}

#[test]
fn inspect_prints_an_outline() {
    let (code, out, err) = run(&["inspect", &corpus("birds.docx")]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.starts_with("birds.docx: 27 blocks in 2 sections\n"),
        "{out}"
    );
    let line = |kind: &str, text: &str| {
        out.lines()
            .any(|l| l.starts_with(kind) && l.ends_with(text))
    };
    assert!(line("heading 1", "Survey sites"));
    assert!(line("table", "Table 1: Sites, visits and counts (5 × 4)"));
    assert!(line("figure", "Figure 1: A wren at the north site"));
    assert!(out.ends_with(
        "2 tables, 2 images, 13 fields, 0 revisions, 0 comments, 1 footnote, 1 endnote, 5 bookmarks\n"
    ), "{out}");
}

#[test]
fn every_view_prints_as_text_and_as_json() {
    let views = [
        "blocks",
        "styles",
        "tables",
        "fields",
        "comments",
        "revisions",
        "images",
        "bookmarks",
        "notes",
        "sections",
        "numbering",
        "content-controls",
    ];
    for doc in ["parts.docx", "birds.docx", "edge.docx"] {
        for v in views {
            let (code, out, err) = run(&["inspect", &corpus(doc), v]);
            assert_eq!(code, Some(0), "{doc} {v}: {err}");
            assert!(!out.is_empty(), "{doc} {v}");
            let (code, out, err) = run(&["inspect", &corpus(doc), v, "--json"]);
            assert_eq!(code, Some(0), "{doc} {v}: {err}");
            let json: serde_json::Value = serde_json::from_str(&out).unwrap();
            let key = v.replace('-', "_");
            assert_eq!(json["sha256"].as_str().unwrap().len(), 64);
            assert!(!json[&key].is_null(), "{doc} {v}");
        }
    }
}

#[test]
fn text_output_fits_the_terminal() {
    let o = tabellion()
        .args(["inspect", &corpus("parts.docx"), "revisions"])
        .env("COLUMNS", "60")
        .output()
        .unwrap();
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(out.lines().all(|l| l.chars().count() <= 60), "{out}");
}

#[test]
fn text_follows_the_view_of_revisions() {
    let row = "S-1\tSeal kit\t9.50";
    let (_, accepted, _) = run(&["text", &corpus("parts.docx")]);
    let (_, rejected, _) = run(&["text", &corpus("parts.docx"), "--revisions", "rejected"]);
    let (_, current, _) = run(&["text", &corpus("parts.docx"), "--revisions", "current"]);
    assert!(!accepted.contains(row) && rejected.contains(row) && current.contains(row));
    assert!(accepted.contains("The P-100 pump quietly moves"));
    assert!(rejected.contains("The P-100 pump moves"));
}

#[test]
fn text_adds_headers_notes_and_comments_on_request() {
    let (_, plain, _) = run(&["text", &corpus("birds.docx")]);
    assert!(!plain.contains("Alder Marshes survey: methods"));
    let (_, full, _) = run(&[
        "text",
        &corpus("birds.docx"),
        "--headers-footers",
        "--notes",
    ]);
    assert!(
        full.starts_with("Breeding bird survey, first page\n"),
        "{full}"
    );
    assert!(
        full.contains("Page 1\nAlder Marshes survey: methods\n\nMethods\n"),
        "{full}"
    );
    assert!(full.contains("Wrens sang at every site[1], and reed warblers at two[i]."));
    assert!(full.contains("[1] Counted from song between 05:00 and 08:00."));
    let (_, c, _) = run(&["text", &corpus("parts.docx"), "--comments"]);
    assert!(
        c.contains(
            "[Reply by Pip Tanager on \u{201C}P-100\u{201D}] Yes, until the end of the year."
        ),
        "{c}"
    );
    let (_, no_boxes, _) = run(&["text", &corpus("birds.docx"), "--no-text-boxes"]);
    assert!(plain.contains("Field note:") && !no_boxes.contains("Field note:"));
}

#[test]
fn bad_input_gives_one_line_and_a_status() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.docx");
    std::fs::write(&bad, b"PK\x03\x04 not really").unwrap();
    for args in [
        vec!["inspect".to_string(), bad.display().to_string()],
        vec!["text".to_string(), bad.display().to_string()],
        vec![
            "inspect".to_string(),
            dir.path().join("missing.docx").display().to_string(),
            "blocks".into(),
        ],
    ] {
        let o = tabellion().args(&args).output().unwrap();
        assert_eq!(o.status.code(), Some(3), "{args:?}");
        let err = String::from_utf8(o.stderr).unwrap();
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(
            err.starts_with("tabellion: ") && !err.contains("panicked"),
            "{err}"
        );
    }
    let (code, _, err) = run(&["inspect", &corpus("parts.docx"), "--json"]);
    assert_eq!(code, Some(2), "{err}");
    let (code, _, _) = run(&["inspect", &corpus("parts.docx"), "chapters"]);
    assert_eq!(code, Some(2));
}
