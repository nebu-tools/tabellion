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
fn tables_writes_one_csv_per_table() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tables");
    let (code, stdout, stderr) = run(&[
        "tables",
        &corpus("birds.docx"),
        out.to_str().unwrap(),
        "--format",
        "csv",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("wrote 2 tables"), "{stdout}");
    let read = |name: &str| {
        let bytes = std::fs::read(out.join(name)).unwrap();
        csv::ReaderBuilder::new()
            .has_headers(false)
            .from_reader(bytes.as_slice())
            .records()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|record| record.iter().map(str::to_string).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    let first = read("table-1.csv");
    assert_eq!(first.len(), 5, "{first:?}");
    assert!(first.iter().all(|row| row.len() == 4), "{first:?}");
    assert_eq!(first[0], ["Site", "Counts", "", "Notes"]);
    assert_eq!(first[2], ["North", "12", "41", "Reed beds"]);
    // A merged cell leaves its grid positions empty.
    assert_eq!(first[1][0], "");
    let second = read("table-2.csv");
    assert_eq!(second, [["Wren", "4"], ["Reed warbler", "7"]]);
}

#[test]
fn tables_refuses_what_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let (code, _, stderr) = run(&["tables", "no/such.docx", dir.path().to_str().unwrap()]);
    assert_eq!(code, Some(3), "{stderr}");
    let (code, _, _) = run(&[
        "tables",
        &corpus("birds.docx"),
        dir.path().to_str().unwrap(),
        "--format",
        "tsv",
    ]);
    assert_eq!(code, Some(2));
}

#[test]
fn tables_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tables");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("table-1.csv"), "precious\n").unwrap();
    let (code, _, stderr) = run(&["tables", &corpus("birds.docx"), out.to_str().unwrap()]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("refusing to overwrite"), "{stderr}");
    assert_eq!(
        std::fs::read(out.join("table-1.csv")).unwrap(),
        b"precious\n"
    );
    assert!(!out.join("table-2.csv").exists());
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
        "paragraphs",
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

#[test]
fn a_wrong_command_line_gives_a_hint_on_standard_error() {
    for args in [
        vec!["inspect"],
        vec!["inspect", "x.docx", "chapters"],
        vec!["text", "x.docx", "--revisions", "some"],
        vec!["frobnicate"],
    ] {
        let (code, out, err) = run(&args);
        assert_eq!(code, Some(2), "{args:?}");
        assert!(out.is_empty(), "{args:?}: {out}");
        assert!(err.starts_with("error: "), "{args:?}: {err}");
        assert!(
            err.contains("Usage: tabellion") || err.contains("[possible values: "),
            "{args:?}: {err}"
        );
        assert!(
            err.ends_with("For more information, try '--help'.\n"),
            "{args:?}: {err}"
        );
        assert!(err.lines().count() > 1, "{args:?}: {err}");
    }
}

#[test]
fn a_file_that_cannot_be_read_gives_one_line_on_standard_error() {
    let (code, out, err) = run(&["text", "no/such/file.docx"]);
    assert_eq!(code, Some(3));
    assert!(out.is_empty());
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(
        err.starts_with("tabellion: cannot read no/such/file.docx"),
        "{err}"
    );
}

#[test]
fn inspect_names_blocks_that_share_a_paragraph_id() {
    let (code, out, err) = run(&["inspect", &corpus("edge.docx")]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.contains(
            "2 blocks share a paragraph id with another block and are numbered instead: #"
        ),
        "{out}"
    );
}

#[test]
fn fields_updates_results_and_reports_the_count() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.docx");
    let out_s = out.display().to_string();
    let (code, stdout, err) = run(&["fields", SAMPLE, &out_s]);
    assert_eq!(code, Some(0), "{err}");
    assert!(stdout.contains("updated 0 fields"), "{stdout}");
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(SAMPLE).unwrap());
}

#[test]
fn insert_takes_typst_source() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.docx");
    let typ = dir.path().join("part.typ");
    std::fs::write(
        &typ,
        "= Methods\n\nWe measured *growth*.\n\n- first\n- second\n",
    )
    .unwrap();
    let (out_s, typ_s) = (out.display().to_string(), typ.display().to_string());
    let (code, stdout, err) = run(&["insert", SAMPLE, &out_s, "--typst", &typ_s]);
    assert_eq!(code, Some(0), "{err}");
    assert!(stdout.contains("inserted 3 blocks"), "{stdout}");
    let (_, text, _) = run(&["text", &out_s]);
    for line in ["Methods", "We measured growth.", "first", "second"] {
        assert!(text.contains(line), "{line}: {text}");
    }
    // What has no Word form is one line on stderr with status 3.
    std::fs::write(&typ, "#table(columns: 2, [a])\n").unwrap();
    let (code, _, err) = run(&["insert", SAMPLE, &out_s, "--typst", &typ_s]);
    assert_eq!(code, Some(3), "{err}");
    assert!(err.contains("has no Word form"), "{err}");
    // The two sources exclude each other: a wrong command line is 2.
    let (code, _, _) = run(&[
        "insert",
        SAMPLE,
        &out_s,
        "--content",
        "c.json",
        "--typst",
        &typ_s,
    ]);
    assert_eq!(code, Some(2));
}

#[test]
fn insert_takes_typst_tables() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.docx");
    let typ = dir.path().join("tab.typ");
    std::fs::write(
        &typ,
        "= Results\n\n#table(columns: 2, table.header([Site], [Pairs]), [North], [12])\n",
    )
    .unwrap();
    let (out_s, typ_s) = (out.display().to_string(), typ.display().to_string());
    let (code, stdout, err) = run(&["insert", SAMPLE, &out_s, "--typst", &typ_s]);
    assert_eq!(code, Some(0), "{err}");
    assert!(stdout.contains("inserted 2 blocks"), "{stdout}");
    let (_, text, _) = run(&["text", &out_s]);
    for line in ["Results", "Site\tPairs", "North\t12"] {
        assert!(text.contains(line), "{line}: {text}");
    }
    // A grid has no table form.
    std::fs::write(&typ, "#grid(columns: 2, [a])\n").unwrap();
    let (code, _, err) = run(&["insert", SAMPLE, &out_s, "--typst", &typ_s]);
    assert_eq!(code, Some(3), "{err}");
    assert!(err.contains("has no Word form"), "{err}");
}

#[test]
fn insert_takes_typst_math() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.docx");
    let typ = dir.path().join("math.typ");
    std::fs::write(&typ, "The value $x + 1$ grows.\n\n$ E = m c^2 $\n").unwrap();
    let (out_s, typ_s) = (out.display().to_string(), typ.display().to_string());
    let (code, stdout, err) = run(&["insert", SAMPLE, &out_s, "--typst", &typ_s]);
    assert_eq!(code, Some(0), "{err}");
    assert!(stdout.contains("inserted 2 blocks"), "{stdout}");
    let (_, text, _) = run(&["text", &out_s]);
    assert!(text.contains("The value x+1 grows."), "{text}");
    // A table call has no math form.
    std::fs::write(&typ, "$table(columns: 1, [a])$\n").unwrap();
    let (code, _, err) = run(&["insert", SAMPLE, &out_s, "--typst", &typ_s]);
    assert_eq!(code, Some(3), "{err}");
    assert!(err.contains("has no Word form"), "{err}");
}

#[test]
fn insert_takes_typst_rowspan_tables() {
    // A rowspan covering trailing rows lists the covered rows empty,
    // so the document opens.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.docx");
    let typ = dir.path().join("span.typ");
    std::fs::write(&typ, "#table(columns: 1, table.cell(rowspan: 2)[A])\n").unwrap();
    let (out_s, typ_s) = (out.display().to_string(), typ.display().to_string());
    let (code, stdout, err) = run(&["insert", SAMPLE, &out_s, "--typst", &typ_s]);
    assert_eq!(code, Some(0), "{err}");
    assert!(stdout.contains("inserted 1 block"), "{stdout}");
    let (_, text, _) = run(&["text", &out_s]);
    assert!(text.contains("A\n"), "{text}");
}

#[test]
fn insert_takes_typst_figures() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.docx");
    let typ = dir.path().join("fig.typ");
    std::fs::write(
        dir.path().join("wren.png"),
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../samples/wren.png"
        ))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &typ,
        "#figure(image(\"wren.png\", alt: \"A wren\"), caption: [A wren]) <fig-wren>\n",
    )
    .unwrap();
    let (out_s, typ_s) = (out.display().to_string(), typ.display().to_string());
    let (code, stdout, err) = run(&["insert", SAMPLE, &out_s, "--typst", &typ_s]);
    assert_eq!(code, Some(0), "{err}");
    assert!(stdout.contains("inserted 1 block"), "{stdout}");
    let (_, text, _) = run(&["text", &out_s]);
    assert!(text.contains("A wren"), "{text}");
}

#[test]
fn insert_takes_a_place_and_reports_errors_with_a_status() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.docx");
    let content = dir.path().join("c.json");
    std::fs::write(
        &content,
        r#"[{"type": "paragraph", "content": ["A wren sang."]}]"#,
    )
    .unwrap();
    let o = |args: &[&str]| run(args);
    let (out_s, content_s) = (out.display().to_string(), content.display().to_string());
    for place in [
        vec![],
        vec!["--before", "heading:Methods"],
        vec!["--after", "#3"],
        vec!["--replace", "bookmark:summary"],
        vec!["--at", "results"],
    ] {
        let mut args = vec!["insert", SAMPLE, &out_s, "--content", &content_s];
        args.extend(place.iter());
        let (code, stdout, err) = o(&args);
        assert_eq!(code, Some(0), "{place:?}: {err}");
        assert!(stdout.contains("inserted 1 block"), "{stdout}");
        let (_, text, _) = o(&["text", &out_s]);
        assert!(text.contains("A wren sang.\n"), "{place:?}: {text}");
    }
    // Content that cannot go there, a place that matches nothing, JSON that
    // is not content: one line and status 3.
    std::fs::write(&content, r#"[{"type": "heading", "level": 12}]"#).unwrap();
    let (code, _, err) = o(&["insert", SAMPLE, &out_s, "--content", &content_s]);
    assert_eq!(code, Some(3));
    assert!(
        err.contains("[0].level") && err.lines().count() == 1,
        "{err}"
    );
    std::fs::write(&content, "[]").unwrap();
    let (code, _, err) = o(&[
        "insert",
        SAMPLE,
        &out_s,
        "--content",
        &content_s,
        "--before",
        "heading:Nowhere",
    ]);
    assert_eq!(code, Some(3));
    assert!(err.contains("no block matches"), "{err}");
    // Two places, or no content, is a wrong command line.
    let (code, _, _) = o(&[
        "insert",
        SAMPLE,
        &out_s,
        "--content",
        &content_s,
        "--at",
        "results",
        "--before",
        "#1",
    ]);
    assert_eq!(code, Some(2));
    let (code, _, _) = o(&["insert", SAMPLE, &out_s]);
    assert_eq!(code, Some(2));
}

#[test]
fn images_writes_one_file_per_embedded_picture() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("images");
    let (code, stdout, stderr) = run(&["images", &corpus("birds.docx"), out.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("wrote 2 images"), "{stdout}");
    for name in ["image1.png", "image2.png"] {
        let bytes = std::fs::read(out.join(name)).unwrap();
        assert!(bytes.len() > 8, "{name}");
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n", "{name}");
    }
}

#[test]
fn images_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("images");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("image1.png"), "precious\n").unwrap();
    let (code, _, stderr) = run(&["images", &corpus("birds.docx"), out.to_str().unwrap()]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("refusing to overwrite"), "{stderr}");
    assert_eq!(
        std::fs::read(out.join("image1.png")).unwrap(),
        b"precious\n"
    );
    assert!(!out.join("image2.png").exists());
}

#[test]
fn images_disambiguates_drawings_sharing_a_part() {
    let dir = tempfile::tempdir().unwrap();
    let patched = dir.path().join("shared.docx");
    std::fs::copy(corpus("birds.docx"), &patched).unwrap();
    {
        let mut doc = nebu_docx::Document::open(&patched).unwrap();
        let main = doc.main_part().to_string();
        let tree = doc.xml(&main).unwrap();
        let root = tree.root_element();
        let body = tree
            .child(root, Some(nebu_docx::ns::W), "body")
            .unwrap()
            .expect("a body");
        const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
        const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let blips: Vec<_> = tree
            .descendants(body)
            .unwrap()
            .filter(|id| tree.is(*id, Some(A), "blip").unwrap_or(false))
            .collect();
        assert!(blips.len() >= 2, "{}", blips.len());
        let mut first: Option<String> = None;
        for blip in blips {
            let rid = tree
                .attribute(blip, Some(R), "embed")
                .unwrap()
                .expect("an embed")
                .into_owned();
            if first.is_none() {
                first = Some(rid);
            }
            tree.set_attribute(blip, Some(R), "embed", first.as_deref().unwrap())
                .unwrap();
        }
        doc.save(dir.path().join("shared2.docx")).unwrap();
    }
    let out = dir.path().join("images");
    let (code, stdout, stderr) = run(&[
        "images",
        dir.path().join("shared2.docx").to_str().unwrap(),
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("wrote 2 images"), "{stdout}");
    let mut names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names.len(), 2, "{names:?}");
    let plain = names
        .iter()
        .find(|n| !n.contains("-2."))
        .expect("a base name");
    let (stem, ext) = plain.rsplit_once('.').expect("an extension");
    assert!(names.contains(&format!("{stem}-2.{ext}")), "{names:?}");
    // Both drawings show the same part, so the bytes match.
    assert_eq!(
        std::fs::read(out.join(&names[0])).unwrap(),
        std::fs::read(out.join(&names[1])).unwrap()
    );
}

const LABELLED_TABLE_JSON: &str = r#"{"blocks": [{"type": "table", "header_rows": 1, "caption": [{"type": "text", "text": "Sites."}], "label": "tab-sites", "rows": [{"cells": [{"content": [{"type": "paragraph", "content": [{"type": "text", "text": "site"}]}]}, {"content": [{"type": "paragraph", "content": [{"type": "text", "text": "count"}]}]}]}, {"cells": [{"content": [{"type": "paragraph", "content": [{"type": "text", "text": "North"}]}]}, {"content": [{"type": "paragraph", "content": [{"type": "text", "text": "12"}]}]}]}]}]}"#;

fn labelled_doc(dir: &std::path::Path) -> std::path::PathBuf {
    let template = dir.join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let content = dir.join("table.json");
    std::fs::write(&content, LABELLED_TABLE_JSON).unwrap();
    let labeled = dir.join("labeled.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        labeled.to_str().unwrap(),
        "--content",
        content.to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    labeled
}

#[test]
fn table_replaces_a_labelled_table_from_csv() {
    let dir = tempfile::tempdir().unwrap();
    let labeled = labelled_doc(dir.path());
    let csv = dir.path().join("new.csv");
    std::fs::write(&csv, "a,b\nc,d\n").unwrap();
    let out = dir.path().join("replaced.docx");
    let (code, stdout, stderr) = run(&[
        "table",
        labeled.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "tab-sites",
        "--csv",
        csv.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("replaced table tab-sites (2 × 2)"),
        "{stdout}"
    );
    // The caption stays; the body round-trips through extract.
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("Table 1: Sites."), "{text}");
    let back = dir.path().join("back");
    let (code, _, stderr) = run(&["tables", out.to_str().unwrap(), back.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{stderr}");
    let bytes = std::fs::read(back.join("table-1.csv")).unwrap();
    let rows: Vec<Vec<String>> = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(bytes.as_slice())
        .records()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .iter()
        .map(|record| record.iter().map(str::to_string).collect())
        .collect();
    assert_eq!(rows, [["a", "b"], ["c", "d"]]);
}

#[test]
fn table_refuses_what_it_cannot_replace() {
    let dir = tempfile::tempdir().unwrap();
    let labeled = labelled_doc(dir.path());
    let out = dir.path().join("out.docx");
    let csv = dir.path().join("new.csv");
    std::fs::write(&csv, "a,b\nc,d\n").unwrap();
    // No such label.
    let (code, _, stderr) = run(&[
        "table",
        labeled.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "tab-nowhere",
        "--csv",
        csv.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("no block matches"), "{stderr}");
    // A ragged file names the row.
    let ragged = dir.path().join("ragged.csv");
    std::fs::write(&ragged, "a,b\nc\n").unwrap();
    let (code, _, stderr) = run(&[
        "table",
        labeled.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "tab-sites",
        "--csv",
        ragged.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("row 2 has 1 cells, expected 2"), "{stderr}");
    // An empty file has no rows.
    let empty = dir.path().join("empty.csv");
    std::fs::write(&empty, "").unwrap();
    let (code, _, stderr) = run(&[
        "table",
        labeled.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "tab-sites",
        "--csv",
        empty.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("no rows"), "{stderr}");
    // A missing file never reaches the document.
    let (code, _, stderr) = run(&[
        "table",
        labeled.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "tab-sites",
        "--csv",
        dir.path().join("no-such.csv").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("cannot read"), "{stderr}");
    // A label on a figure names no table.
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/wren.png"),
        dir.path().join("wren.png"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("figure.json"),
        r#"{"blocks": [{"type": "figure", "image": {"file": "wren.png"}, "caption": [{"type": "text", "text": "A wren."}], "label": "fig-wren"}]}"#,
    )
    .unwrap();
    let figured = dir.path().join("figured.docx");
    let (code, _, stderr) = run(&[
        "insert",
        labeled.to_str().unwrap(),
        figured.to_str().unwrap(),
        "--content",
        dir.path().join("figure.json").to_str().unwrap(),
        "--at",
        "author",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let (code, _, stderr) = run(&[
        "table",
        figured.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-wren",
        "--csv",
        csv.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("no table labelled fig-wren"), "{stderr}");
}

#[test]
fn table_refuses_a_label_that_names_no_table() {
    let dir = tempfile::tempdir().unwrap();
    // A labelled figure followed by an unlabelled table: the bookmark
    // resolves, but no table carries the label.
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/wren.png"),
        dir.path().join("wren.png"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("figure.json"),
        r#"{"blocks": [{"type": "figure", "image": {"file": "wren.png"}, "caption": [{"type": "text", "text": "A wren."}], "label": "fig-wren"}]}"#,
    )
    .unwrap();
    let template = dir.path().join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let figured = dir.path().join("figured.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        figured.to_str().unwrap(),
        "--content",
        dir.path().join("figure.json").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    std::fs::write(
        dir.path().join("plain.json"),
        r#"{"blocks": [{"type": "table", "rows": [{"cells": [{"content": [{"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]}]}]}]}"#,
    )
    .unwrap();
    let both = dir.path().join("both.docx");
    let (code, _, stderr) = run(&[
        "insert",
        figured.to_str().unwrap(),
        both.to_str().unwrap(),
        "--content",
        dir.path().join("plain.json").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let csv = dir.path().join("new.csv");
    std::fs::write(&csv, "a,b\n").unwrap();
    let out = dir.path().join("out.docx");
    let (code, _, stderr) = run(&[
        "table",
        both.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-wren",
        "--csv",
        csv.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("no table labelled fig-wren"), "{stderr}");
    assert!(!out.exists());
}

#[test]
fn table_accepts_a_lowercase_seq_caption() {
    let dir = tempfile::tempdir().unwrap();
    let labeled = labelled_doc(dir.path());
    let lowered = dir.path().join("lowered.docx");
    {
        let mut doc = nebu_docx::Document::open(&labeled).unwrap();
        let main = doc.main_part().to_string();
        let tree = doc.xml(&main).unwrap();
        let root = tree.root_element();
        let body = tree
            .child(root, Some(nebu_docx::ns::W), "body")
            .unwrap()
            .expect("a body");
        let mut changed = 0;
        let mut blips: Vec<_> = Vec::new();
        for id in tree.descendants(body).unwrap() {
            if tree
                .is(id, Some(nebu_docx::ns::W), "fldSimple")
                .unwrap_or(false)
            {
                blips.push(id);
            }
        }
        for id in blips {
            let instr = tree
                .attribute(id, Some(nebu_docx::ns::W), "instr")
                .unwrap()
                .map(|instr| instr.into_owned());
            if let Some(instr) = instr.filter(|instr| instr.contains("SEQ Table")) {
                tree.set_attribute(
                    id,
                    Some(nebu_docx::ns::W),
                    "instr",
                    &instr.replace("SEQ Table", "seq table"),
                )
                .unwrap();
                changed += 1;
            }
        }
        assert!(changed > 0, "a SEQ Table caption");
        doc.save(&lowered).unwrap();
    }
    let csv = dir.path().join("new.csv");
    std::fs::write(&csv, "a,b\n").unwrap();
    let out = dir.path().join("out.docx");
    let (code, stdout, stderr) = run(&[
        "table",
        lowered.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "tab-sites",
        "--csv",
        csv.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("replaced table tab-sites (1 × 2)"),
        "{stdout}"
    );
}

#[test]
fn image_replaces_a_labelled_figure_with_same_kind_bytes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/wren.png"),
        dir.path().join("wren.png"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("figure.json"),
        r#"{"blocks": [{"type": "figure", "image": {"file": "wren.png"}, "caption": [{"type": "text", "text": "A wren."}], "label": "fig-wren"}]}"#,
    )
    .unwrap();
    let template = dir.path().join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let figured = dir.path().join("figured.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        figured.to_str().unwrap(),
        "--content",
        dir.path().join("figure.json").to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    // Different bytes, same kind: flip a byte well past the header.
    let mut alt = std::fs::read(dir.path().join("wren.png")).unwrap();
    alt[100] ^= 0xff;
    std::fs::write(dir.path().join("alt.png"), &alt).unwrap();
    let out = dir.path().join("out.docx");
    let (code, stdout, stderr) = run(&[
        "image",
        figured.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-wren",
        "--file",
        dir.path().join("alt.png").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("replaced image fig-wren"), "{stdout}");
    let back = dir.path().join("back");
    let (code, _, stderr) = run(&["images", out.to_str().unwrap(), back.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{stderr}");
    let files: Vec<_> = std::fs::read_dir(&back)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(std::fs::read(&files[0]).unwrap(), alt);
    // The caption still numbers the figure.
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("Figure 1: A wren."), "{text}");
}

#[test]
fn image_refuses_what_it_cannot_replace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/wren.png"),
        dir.path().join("wren.png"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("figure.json"),
        r#"{"blocks": [{"type": "figure", "image": {"file": "wren.png"}, "caption": [{"type": "text", "text": "A wren."}], "label": "fig-wren"}]}"#,
    )
    .unwrap();
    let template = dir.path().join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let figured = dir.path().join("figured.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        figured.to_str().unwrap(),
        "--content",
        dir.path().join("figure.json").to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let out = dir.path().join("out.docx");
    // No such label.
    let (code, _, stderr) = run(&[
        "image",
        figured.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-nowhere",
        "--file",
        dir.path().join("wren.png").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("no block matches"), "{stderr}");
    assert!(!out.exists());
    // A label on a table names no picture.
    std::fs::write(
        dir.path().join("table.json"),
        r#"{"blocks": [{"type": "table", "caption": [{"type": "text", "text": "Sites."}], "label": "tab-sites", "rows": [{"cells": [{"content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}]}]}]}"#,
    )
    .unwrap();
    let tabulated = dir.path().join("tabulated.docx");
    let (code, _, stderr) = run(&[
        "insert",
        figured.to_str().unwrap(),
        tabulated.to_str().unwrap(),
        "--content",
        dir.path().join("table.json").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let (code, _, stderr) = run(&[
        "image",
        tabulated.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "tab-sites",
        "--file",
        dir.path().join("wren.png").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("no picture labelled tab-sites"), "{stderr}");
    assert!(!out.exists());
    // A picture of another kind does not take its place.
    std::fs::write(
        dir.path().join("vector.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\"/>",
    )
    .unwrap();
    let (code, _, stderr) = run(&[
        "image",
        figured.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-wren",
        "--file",
        dir.path().join("vector.svg").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("cannot take the place of"), "{stderr}");
    assert!(!out.exists());
    // A missing file never reaches the document.
    let (code, _, stderr) = run(&[
        "image",
        figured.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-wren",
        "--file",
        dir.path().join("no-such.png").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("cannot read"), "{stderr}");
}

fn figured_doc(dir: &std::path::Path, caption: &str, label: &str) -> std::path::PathBuf {
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/wren.png"),
        dir.join("wren.png"),
    )
    .unwrap();
    std::fs::write(
        dir.join("figure.json"),
        format!(
            r#"{{"blocks": [{{"type": "figure", "image": {{"file": "wren.png"}}, "caption": [{{"type": "text", "text": "{caption}"}}], "label": "{label}"}}]}}"#
        ),
    )
    .unwrap();
    let template = dir.join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let figured = dir.join("figured.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        figured.to_str().unwrap(),
        "--content",
        dir.join("figure.json").to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    figured
}

#[test]
fn image_refuses_a_part_shared_by_two_figures() {
    let dir = tempfile::tempdir().unwrap();
    let first = figured_doc(dir.path(), "First.", "fig-first");
    let second = dir.path().join("second.docx");
    // The same bytes insert under a second label; the writer stores the
    // picture once, so both figures point at one part.
    std::fs::write(
        dir.path().join("figure2.json"),
        r#"{"blocks": [{"type": "figure", "image": {"file": "wren.png"}, "caption": [{"type": "text", "text": "Second."}], "label": "fig-second"}]}"#,
    )
    .unwrap();
    let (code, _, stderr) = run(&[
        "insert",
        first.to_str().unwrap(),
        second.to_str().unwrap(),
        "--content",
        dir.path().join("figure2.json").to_str().unwrap(),
        "--at",
        "author",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let out = dir.path().join("out.docx");
    let (code, _, stderr) = run(&[
        "image",
        second.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-first",
        "--file",
        dir.path().join("wren.png").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("is shared by 2 pictures"), "{stderr}");
    assert!(!out.exists());
}

#[test]
fn image_refuses_an_svg_with_a_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/wren.png"),
        dir.path().join("wren.png"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("art.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\"/>",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("figure.json"),
        r#"{"blocks": [{"type": "figure", "image": {"file": "art.svg", "fallback": {"file": "wren.png"}}, "caption": [{"type": "text", "text": "A drawing."}], "label": "fig-svg"}]}"#,
    )
    .unwrap();
    let template = dir.path().join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let figured = dir.path().join("figured.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        figured.to_str().unwrap(),
        "--content",
        dir.path().join("figure.json").to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let out = dir.path().join("out.docx");
    let (code, _, stderr) = run(&[
        "image",
        figured.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-svg",
        "--file",
        dir.path().join("wren.png").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("more than one picture"), "{stderr}");
    assert!(!out.exists());
}

#[test]
fn image_refuses_a_paragraph_with_a_vml_picture_too() {
    let dir = tempfile::tempdir().unwrap();
    let template = dir.path().join("template.docx");
    std::fs::copy(corpus("birds.docx"), &template).unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/wren.png"),
        dir.path().join("wren.png"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("figure.json"),
        r#"{"blocks": [{"type": "figure", "image": {"file": "wren.png"}, "caption": [{"type": "text", "text": "A wren."}], "label": "fig-wren"}]}"#,
    )
    .unwrap();
    let figured = dir.path().join("figured.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        figured.to_str().unwrap(),
        "--content",
        dir.path().join("figure.json").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    // A VML picture reference in the figure paragraph: one DrawingML
    // picture and one VML picture, so two references to replace.
    {
        use nebu_docx::{Document, ns};
        const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
        const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        const VML: &str = "urn:schemas-microsoft-com:vml";
        let mut doc = Document::open(&figured).unwrap();
        let main = doc.main_part().to_string();
        let caption = {
            let views = doc.views().unwrap();
            let label: nebu_docx::Locator = "bookmark:_Ref_fig-wren".parse().unwrap();
            views.locate(&label).unwrap().index
        };
        let tree = doc.xml(&main).unwrap();
        let root = tree.root_element();
        let body = tree
            .child(root, Some(ns::W), "body")
            .unwrap()
            .expect("a body");
        let kids: Vec<_> = tree.children(body).unwrap().collect();
        let para = kids[caption - 1];
        let mut own: Vec<String> = Vec::new();
        for id in tree.descendants(para).unwrap() {
            if tree.is(id, Some(A), "blip").unwrap_or(false)
                && let Some(rid) = tree.attribute(id, Some(R), "embed").unwrap()
            {
                own.push(rid.into_owned());
            }
        }
        assert!(!own.is_empty(), "a blip");
        let mut other: Option<String> = None;
        for id in tree.descendants(body).unwrap() {
            if tree.is(id, Some(A), "blip").unwrap_or(false)
                && let Some(rid) = tree
                    .attribute(id, Some(R), "embed")
                    .unwrap()
                    .map(|v| v.into_owned())
                && !own.contains(&rid)
            {
                other = Some(rid);
                break;
            }
        }
        let other = other.expect("another blip");
        // The VML reference points at a different part than the figure.
        let pict = tree.create_element(Some(ns::W), "pict").unwrap();
        let shape = tree.create_element(Some(VML), "shape").unwrap();
        let imagedata = tree.create_element(Some(VML), "imagedata").unwrap();
        tree.set_attribute(imagedata, Some(R), "id", &other)
            .unwrap();
        tree.append(shape, imagedata).unwrap();
        tree.append(pict, shape).unwrap();
        tree.append(para, pict).unwrap();
        doc.save(&figured).unwrap();
    }
    let out = dir.path().join("out.docx");
    let (code, _, stderr) = run(&[
        "image",
        figured.to_str().unwrap(),
        out.to_str().unwrap(),
        "--target",
        "fig-wren",
        "--file",
        dir.path().join("wren.png").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("more than one picture"), "{stderr}");
    assert!(!out.exists());
}

const REPLACE_JSON: &str = r#"{"blocks": [
    {"type": "paragraph", "content": [{"type": "text", "text": "A draft plan"}]},
    {"type": "paragraph", "content": [{"type": "text", "text": "Another draft, two drafts"}]},
    {"type": "table", "rows": [
        {"cells": [{"content": [{"type": "paragraph", "content": [{"type": "text", "text": "draft tally"}]}]}]}
    ]}
]}"#;

/// A document with findable text in two paragraphs and a table cell.
fn drafted_doc(dir: &std::path::Path) -> std::path::PathBuf {
    let template = dir.join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let content = dir.join("draft.json");
    std::fs::write(&content, REPLACE_JSON).unwrap();
    let drafted = dir.join("drafted.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        drafted.to_str().unwrap(),
        "--content",
        content.to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    drafted
}

#[test]
fn replace_swaps_text_across_paragraphs_and_cells() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let out = dir.path().join("replaced.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    // Two paragraphs change; the fourth match is a substring of "drafts".
    assert!(
        stdout.contains(r#"replaced "draft" with "final" (4 occurrences in 3 paragraphs)"#),
        "{stdout}"
    );
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("A final plan"), "{text}");
    assert!(text.contains("Another final, two finals"), "{text}");
    let back = dir.path().join("back");
    let (code, _, stderr) = run(&["tables", out.to_str().unwrap(), back.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{stderr}");
    let bytes = std::fs::read(back.join("table-1.csv")).unwrap();
    assert!(String::from_utf8(bytes).unwrap().contains("final tally"));
}

/// The id of the block whose text holds the needle.
fn block_with(doc: &str, needle: &str) -> String {
    let (code, out, err) = run(&["inspect", doc, "blocks", "--json"]);
    assert_eq!(code, Some(0), "{err}");
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    json["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["text"].as_str().is_some_and(|t| t.contains(needle)))
        .and_then(|b| b["id"].as_str())
        .expect("a block holds the text")
        .to_string()
}

#[test]
fn replace_scopes_to_the_named_block() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let id = block_with(drafted.to_str().unwrap(), "Another draft");
    let out = dir.path().join("replaced.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--block",
        &id,
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains(&format!(
            r#"replaced "draft" with "final" in {id} (2 occurrences in 1 paragraph)"#
        )),
        "{stdout}"
    );
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("Another final, two finals"), "{text}");
    assert!(text.contains("A draft plan"), "{text}");
}

#[test]
fn replace_refuses_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let template = dir.path().join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let content = dir.path().join("field.json");
    std::fs::write(
        &content,
        r#"{"blocks": [
            {"type": "paragraph", "content": [{"type": "text", "text": "plain draft here"}]},
            {"type": "table", "rows": [{"cells": [
                {"content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "See "},
                    {"type": "field", "code": "REF a \\h", "result": "draft"},
                    {"type": "text", "text": " A"}
                ]}]},
                {"content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "See "},
                    {"type": "field", "code": "REF b \\h", "result": "draft"},
                    {"type": "text", "text": " B"}
                ]}]}
            ]}]}
        ]}"#,
    )
    .unwrap();
    let doc = dir.path().join("fielded.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        doc.to_str().unwrap(),
        "--content",
        content.to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let out = dir.path().join("out.docx");
    let (code, _, stderr) = run(&[
        "replace",
        doc.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    // The body paragraph succeeds, then both cells refuse: the sweep
    // lists each refusal distinctly instead of stopping at the first.
    assert!(stderr.contains("refused 2 paragraphs"), "{stderr}");
    let listed = stderr
        .split_once("refused 2 paragraphs: ")
        .expect("the refusals")
        .1;
    let mut entries: Vec<&str> = listed.split(", ").collect();
    assert_eq!(entries.len(), 2, "{stderr}");
    assert!(entries.iter().all(|e| e.contains("(field)")), "{stderr}");
    entries.sort_unstable();
    entries.dedup();
    assert_eq!(entries.len(), 2, "distinct paragraphs: {stderr}");
    assert!(!out.exists());
}

#[test]
fn replace_names_the_flag_for_bad_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    // A line ending in --with is an argument no paragraph can take: one
    // diagnostic, not one refusal per paragraph.
    for (flag, value) in [("--find", "dra\tft"), ("--with", "fin\nal")] {
        let out = dir.path().join("out.docx");
        let (code, _, stderr) = run(&[
            "replace",
            drafted.to_str().unwrap(),
            out.to_str().unwrap(),
            "--find",
            if flag == "--find" { value } else { "draft" },
            "--with",
            if flag == "--with" { value } else { "final" },
        ]);
        assert_eq!(code, Some(3), "{stderr}");
        assert!(stderr.contains(flag), "{stderr}");
        assert!(!stderr.contains("refused"), "{stderr}");
        assert!(!out.exists());
    }
    // A character XML cannot write names --with too.
    let out = dir.path().join("out.docx");
    let (code, _, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "a\x01b",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("--with"), "{stderr}");
    assert!(stderr.contains("U+0001"), "{stderr}");
    assert!(!out.exists());
}

#[test]
fn replace_reports_no_occurrences() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let out = dir.path().join("out.docx");
    let (code, _, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "nightingale",
        "--with",
        "thrush",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(
        stderr.contains(r#"no occurrences of "nightingale""#),
        "{stderr}"
    );
    assert!(!out.exists());
}

#[test]
fn replace_rejects_an_empty_find() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let out = dir.path().join("out.docx");
    let (code, _, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "",
        "--with",
        "final",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(
        stderr.contains("cannot replace an empty string"),
        "{stderr}"
    );
    assert!(!out.exists());
}

#[test]
fn replace_deletes_with_an_empty_with() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let out = dir.path().join("deleted.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft ",
        "--with",
        "",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("(2 occurrences in 2 paragraphs)"),
        "{stdout}"
    );
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("A plan"), "{text}");
}

#[test]
fn replace_sweeps_headers_and_leaves_comments_alone() {
    let dir = tempfile::tempdir().unwrap();
    // "Marshes" stands once in the body and twice in headers.
    let out = dir.path().join("headers.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        &corpus("birds.docx"),
        out.to_str().unwrap(),
        "--find",
        "Marshes",
        "--with",
        "Fens",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("(3 occurrences in 3 paragraphs)"),
        "{stdout}"
    );
    let (_, text, _) = run(&["text", out.to_str().unwrap(), "--headers-footers"]);
    assert_eq!(text.matches("Fens").count(), 3, "{text}");
    // "Both" stands only in a comment of changes.docx ("dawn" also
    // stands in a footnote's insertion): nothing to replace. The source
    // comment still holds the word, so the check cannot pass vacuously.
    let (code, out, err) = run(&["inspect", &corpus("changes.docx"), "comments", "--json"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("Both"), "{out}");
    let silent = dir.path().join("silent.docx");
    let (code, _, stderr) = run(&[
        "replace",
        &corpus("changes.docx"),
        silent.to_str().unwrap(),
        "--find",
        "Both",
        "--with",
        "Each",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains(r#"no occurrences of "Both""#), "{stderr}");
    assert!(!silent.exists());
}

#[test]
fn replace_tracks_the_replacement_for_an_author() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let out = dir.path().join("tracked.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "Lark",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    // The human line keeps its shape and names the tracker.
    assert!(
        stdout.contains(
            r#"replaced "draft" with "final" (4 occurrences in 3 paragraphs, tracked by Lark)"#
        ),
        "{stdout}"
    );
    // The file holds Lark's pending revisions; the accepted text reads
    // through them.
    let (code, revisions, err) = run(&["inspect", out.to_str().unwrap(), "revisions", "--json"]);
    assert_eq!(code, Some(0), "{err}");
    let revisions: serde_json::Value = serde_json::from_str(&revisions).unwrap();
    let authors: Vec<&str> = revisions["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["author"].as_str().unwrap())
        .collect();
    assert!(!authors.is_empty(), "{revisions}");
    assert!(authors.iter().all(|a| *a == "Lark"), "{revisions}");
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("A final plan"), "{text}");
}

#[test]
fn replace_rejects_a_bad_tracking_author() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let out = dir.path().join("out.docx");
    // A control character names --author, not one refusal per paragraph.
    let (code, _, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "La\x01rk",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("tracking author"), "{stderr}");
    assert!(!stderr.contains("refused"), "{stderr}");
    assert!(!out.exists());
}

#[test]
fn replace_reports_json_for_programs() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let out = dir.path().join("tracked.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "Lark",
        "--json",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["command"], "replace");
    assert_eq!(report["find"], "draft");
    assert_eq!(report["with"], "final");
    assert_eq!(report["tracked_by"], "Lark");
    assert_eq!(report["occurrences"], 4);
    assert_eq!(report["paragraphs"], 3);
    assert_eq!(report["refused"].as_array().unwrap().len(), 0);
}

#[test]
fn replace_json_names_each_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let template = dir.path().join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let content = dir.path().join("field.json");
    std::fs::write(
        &content,
        r#"{"blocks": [
            {"type": "paragraph", "content": [{"type": "text", "text": "plain draft here"}]},
            {"type": "table", "rows": [{"cells": [
                {"content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "See "},
                    {"type": "field", "code": "REF a \\h", "result": "draft"}
                ]}]}
            ]}]}
        ]}"#,
    )
    .unwrap();
    // A field result the sweep cannot cut through.
    let fielded = dir.path().join("fielded.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        fielded.to_str().unwrap(),
        "--content",
        content.to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let out = dir.path().join("out.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        fielded.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--json",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    // Programs get the same refusals as people: the JSON report still
    // prints on stdout while the diagnostic stays on stderr.
    assert!(stderr.contains("refused 1 paragraph"), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let refused = report["refused"].as_array().unwrap();
    assert_eq!(refused.len(), 1, "{report}");
    assert_eq!(refused[0]["reason"], "field", "{report}");
    assert!(!out.exists());
}

#[test]
fn replace_json_reports_failed_validation() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    // An empty find, a bad tracking author, a missing input and an
    // unresolved block all fail before any sweep: with --json each
    // still prints one parseable zeros report on stdout, keeps the
    // diagnostic on stderr, exits 3 and writes nothing.
    let missing = dir.path().join("missing.docx");
    let cases: Vec<(Vec<String>, &str)> = vec![
        (
            vec!["--find".into(), "".into(), "--with".into(), "final".into()],
            "cannot replace an empty string",
        ),
        (
            vec![
                "--find".into(),
                "draft".into(),
                "--with".into(),
                "final".into(),
                "--author".into(),
                "La\x01rk".into(),
            ],
            "tracking author",
        ),
        (
            vec![
                "--find".into(),
                "draft".into(),
                "--with".into(),
                "final".into(),
                "--block".into(),
                "p:00000000".into(),
            ],
            "no block matches",
        ),
    ];
    for (flags, diagnostic) in &cases {
        let out = dir.path().join("out.docx");
        let mut args = vec![
            "replace".to_string(),
            drafted.to_str().unwrap().to_string(),
            out.to_str().unwrap().to_string(),
        ];
        args.extend(flags.clone());
        args.push("--json".to_string());
        let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (code, stdout, stderr) = run(&args);
        assert_eq!(code, Some(3), "{stderr}");
        assert!(stderr.contains(diagnostic), "{stderr}");
        let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(report["command"], "replace", "{report}");
        assert_eq!(report["occurrences"], 0, "{report}");
        assert_eq!(report["paragraphs"], 0, "{report}");
        assert_eq!(report["refused"].as_array().unwrap().len(), 0, "{report}");
        assert!(!out.exists());
    }
    // The author echo survives validation failure.
    let out = dir.path().join("author.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "La\x01rk",
        "--json",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["tracked_by"], "La\x01rk", "{report}");
    assert!(!out.exists());
    // A missing input reports zeros too.
    let out = dir.path().join("gone.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        missing.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--json",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["occurrences"], 0, "{report}");
    assert!(!out.exists());
}

#[test]
fn replace_json_pins_foreign_and_multiword_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let pip = dir.path().join("pip.docx");
    let (code, _, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        pip.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "Pip",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    // Another author's tracked text refuses as a foreign revision.
    let out = dir.path().join("out.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        pip.to_str().unwrap(),
        out.to_str().unwrap(),
        "--find",
        "final",
        "--with",
        "ultimate",
        "--author",
        "Max",
        "--json",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let refused = report["refused"].as_array().unwrap();
    assert!(!refused.is_empty(), "{report}");
    assert!(
        refused.iter().all(|r| r["reason"] == "foreign-revision"),
        "{report}"
    );
    assert!(stderr.contains("pending change"), "{stderr}");
    assert!(!out.exists());
    // Without tracking the same text refuses by obstruction name, with
    // spaces as dashes.
    let plain = dir.path().join("plain.docx");
    let (code, stdout, stderr) = run(&[
        "replace",
        pip.to_str().unwrap(),
        plain.to_str().unwrap(),
        "--find",
        "final",
        "--with",
        "ultimate",
        "--json",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let refused = report["refused"].as_array().unwrap();
    assert!(!refused.is_empty(), "{report}");
    assert!(
        refused.iter().all(|r| r["reason"] == "inserted-revision"),
        "{report}"
    );
}

/// A document with a single Lark move pair between its two body
/// paragraphs, with range markers: the source holds the moveFrom
/// record, the destination the moveTo record.
fn moved_doc(dir: &std::path::Path) -> std::path::PathBuf {
    use nebu_docx::xml::NodeId;
    let drafted = drafted_doc(dir);
    let bytes = std::fs::read(&drafted).unwrap();
    let mut doc = nebu_docx::Document::from_bytes(bytes).unwrap();
    let (source, dest) = {
        let views = doc.views().unwrap();
        let node = |needle: &str| {
            views
                .paragraphs()
                .iter()
                .find(|p| p.text.contains(needle))
                .unwrap_or_else(|| panic!("no paragraph holds {needle:?}"))
                .node
        };
        (node("A draft plan"), node("Another draft"))
    };
    let set_runs = |doc: &mut nebu_docx::Document, para: NodeId, xml: &str| {
        let main = doc.main_part().to_string();
        let tree = doc.xml(&main).unwrap();
        let runs: Vec<NodeId> = tree
            .elements(para)
            .unwrap()
            .filter(|&n| tree.is(n, Some(nebu_docx::ns::W), "r").unwrap_or(false))
            .collect();
        for r in runs {
            tree.detach(r).unwrap();
        }
        for n in tree
            .parse_fragment(xml, &[("w", nebu_docx::ns::W)])
            .unwrap()
        {
            tree.append(para, n).unwrap();
        }
    };
    set_runs(
        &mut doc,
        source,
        concat!(
            "<w:moveFromRangeStart w:id=\"10\" w:name=\"m1\"/>",
            "<w:moveFrom w:id=\"11\" w:author=\"Lark\" w:date=\"2026-11-20T12:00:00Z\" w:name=\"m1\">",
            "<w:r><w:t>rover</w:t></w:r></w:moveFrom>",
            "<w:moveFromRangeEnd w:id=\"10\" w:name=\"m1\"/>",
            "<w:r><w:t xml:space=\"preserve\"> stays</w:t></w:r>",
        ),
    );
    set_runs(
        &mut doc,
        dest,
        concat!(
            "<w:r><w:t xml:space=\"preserve\">goes </w:t></w:r>",
            "<w:moveToRangeStart w:id=\"20\" w:name=\"m1\"/>",
            "<w:moveTo w:id=\"21\" w:author=\"Lark\" w:date=\"2026-11-20T12:00:00Z\" w:name=\"m1\">",
            "<w:r><w:t>rover</w:t></w:r></w:moveTo>",
            "<w:moveToRangeEnd w:id=\"20\" w:name=\"m1\"/>",
        ),
    );
    let moved = dir.join("moved.docx");
    std::fs::write(&moved, doc.to_bytes().unwrap()).unwrap();
    moved
}

/// The decompressed word/document.xml of a saved document.
fn document_xml(doc: &std::path::Path) -> String {
    use std::io::Read;
    let bytes = std::fs::read(doc).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

#[test]
fn accept_settles_a_move_pair() {
    let dir = tempfile::tempdir().unwrap();
    let moved = moved_doc(dir.path());
    let out = dir.path().join("accepted.docx");
    let (code, stdout, stderr) = run(&[
        "accept",
        moved.to_str().unwrap(),
        out.to_str().unwrap(),
        "--author",
        "Lark",
        "--json",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    // The pair counts its two records.
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["command"], "accept");
    assert_eq!(report["author"], "Lark");
    assert_eq!(report["settled"], 2, "{report}");
    // The destination keeps the text, the source loses it, and the
    // records with their range markers are gone.
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("goes rover"), "{text}");
    assert!(!text.contains("rover stays"), "{text}");
    let (code, revisions, err) = run(&["inspect", out.to_str().unwrap(), "revisions", "--json"]);
    assert_eq!(code, Some(0), "{err}");
    let revisions: serde_json::Value = serde_json::from_str(&revisions).unwrap();
    assert_eq!(revisions["revisions"].as_array().unwrap().len(), 0);
    let xml = document_xml(&out);
    assert!(!xml.contains("moveFrom"), "{xml}");
    assert!(!xml.contains("moveTo"), "{xml}");
}

#[test]
fn reject_restores_a_move_pair() {
    let dir = tempfile::tempdir().unwrap();
    let moved = moved_doc(dir.path());
    let out = dir.path().join("rejected.docx");
    let (code, stdout, stderr) = run(&["reject", moved.to_str().unwrap(), out.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("rejected 2 revisions"), "{stdout}");
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("rover stays"), "{text}");
    assert!(!text.contains("goes rover"), "{text}");
    let xml = document_xml(&out);
    assert!(!xml.contains("moveFrom"), "{xml}");
    assert!(!xml.contains("moveTo"), "{xml}");
}

#[test]
fn settle_moves_respect_the_author() {
    let dir = tempfile::tempdir().unwrap();
    let moved = moved_doc(dir.path());
    // Another author's move stays whole: nothing settles and the file
    // keeps both records with the text where the move left it.
    let out = dir.path().join("wren.docx");
    let (code, stdout, stderr) = run(&[
        "accept",
        moved.to_str().unwrap(),
        out.to_str().unwrap(),
        "--author",
        "Wren",
        "--json",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["settled"], 0, "{report}");
    let (code, revisions, err) = run(&["inspect", out.to_str().unwrap(), "revisions", "--json"]);
    assert_eq!(code, Some(0), "{err}");
    let revisions: serde_json::Value = serde_json::from_str(&revisions).unwrap();
    assert_eq!(revisions["revisions"].as_array().unwrap().len(), 2);
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("goes rover"), "{text}");
    // Lark's own accept settles the pair.
    let lark = dir.path().join("lark.docx");
    let (code, stdout, stderr) = run(&[
        "accept",
        out.to_str().unwrap(),
        lark.to_str().unwrap(),
        "--author",
        "Lark",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("accepted 2 revisions"), "{stdout}");
}

#[test]
fn settle_json_reports_a_failed_save() {
    let dir = tempfile::tempdir().unwrap();
    let moved = moved_doc(dir.path());
    // Saving into an existing directory fails: with --json the report
    // still prints, saying nothing was saved, and the diagnostic stays
    // on stderr.
    let out = dir.path().join("existing");
    std::fs::create_dir(&out).unwrap();
    for command in ["accept", "reject"] {
        let (code, stdout, stderr) = run(&[
            command,
            moved.to_str().unwrap(),
            out.to_str().unwrap(),
            "--json",
        ]);
        assert_ne!(code, Some(0), "{stderr}");
        let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(report["command"], command, "{report}");
        assert_eq!(report["settled"], 0, "{report}");
        assert!(!stderr.is_empty(), "the diagnostic stays");
    }
}

#[test]
fn accept_settles_tracked_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let tracked = dir.path().join("tracked.docx");
    let (code, _, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        tracked.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "Lark",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let out = dir.path().join("accepted.docx");
    let (code, stdout, stderr) = run(&["accept", tracked.to_str().unwrap(), out.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{stderr}");
    // Four matches make four deletions and four insertions.
    assert!(stdout.contains("accepted 8 revisions"), "{stdout}");
    let (code, revisions, err) = run(&["inspect", out.to_str().unwrap(), "revisions", "--json"]);
    assert_eq!(code, Some(0), "{err}");
    let revisions: serde_json::Value = serde_json::from_str(&revisions).unwrap();
    assert_eq!(revisions["revisions"].as_array().unwrap().len(), 0);
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("A final plan"), "{text}");
}

#[test]
fn reject_restores_the_original_text() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let tracked = dir.path().join("tracked.docx");
    let (code, _, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        tracked.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "Lark",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let out = dir.path().join("rejected.docx");
    let (code, stdout, stderr) = run(&[
        "reject",
        tracked.to_str().unwrap(),
        out.to_str().unwrap(),
        "--author",
        "Lark",
        "--json",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["command"], "reject");
    assert_eq!(report["author"], "Lark");
    assert_eq!(report["settled"], 8);
    let (_, text, _) = run(&["text", out.to_str().unwrap()]);
    assert!(text.contains("A draft plan"), "{text}");
    assert!(!text.contains("final"), "{text}");
}

#[test]
fn accept_scopes_to_one_author_and_tolerates_a_clean_document() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    // Lark tracks "draft" and Wren tracks "plan": disjoint matches, so
    // each replacement settles one author's revisions only.
    let lark = dir.path().join("lark.docx");
    let (code, _, stderr) = run(&[
        "replace",
        drafted.to_str().unwrap(),
        lark.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "Lark",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let both = dir.path().join("both.docx");
    let (code, _, stderr) = run(&[
        "replace",
        lark.to_str().unwrap(),
        both.to_str().unwrap(),
        "--find",
        "plan",
        "--with",
        "scheme",
        "--author",
        "Wren",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let out = dir.path().join("lark-accepted.docx");
    let (code, stdout, stderr) = run(&[
        "accept",
        both.to_str().unwrap(),
        out.to_str().unwrap(),
        "--author",
        "Lark",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("accepted 8 revisions"), "{stdout}");
    // Wren's revisions stay pending.
    let (code, revisions, err) = run(&["inspect", out.to_str().unwrap(), "revisions", "--json"]);
    assert_eq!(code, Some(0), "{err}");
    let revisions: serde_json::Value = serde_json::from_str(&revisions).unwrap();
    let authors: Vec<&str> = revisions["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["author"].as_str().unwrap())
        .collect();
    assert!(!authors.is_empty(), "{revisions}");
    assert!(authors.iter().all(|a| *a == "Wren"), "{revisions}");
    // Wren's insertion is still pending: the rejected text holds the
    // pre-move source word it replaced.
    let (_, rejected, _) = run(&["text", out.to_str().unwrap(), "--revisions", "rejected"]);
    assert!(rejected.contains("A final plan"), "{rejected}");
    // Accepting again settles nothing and still succeeds.
    let clean = dir.path().join("clean.docx");
    let (code, stdout, stderr) = run(&[
        "accept",
        out.to_str().unwrap(),
        clean.to_str().unwrap(),
        "--author",
        "Lark",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("no revisions to accept"), "{stdout}");
}

#[test]
fn bench_reports_timings_without_writing() {
    // The 40 ms gate has three orders of magnitude of headroom on these
    // fixtures (single replacements time in microseconds), so the exit
    // code does not meaningfully depend on the wall clock here; the
    // release-only perf test guards the real budget.
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let (code, stdout, stderr) = run(&[
        "bench",
        drafted.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains(r#"bench "draft" with "final" (4 occurrences in 3 paragraphs"#),
        "{stdout}"
    );
    assert!(stdout.contains("against a 20ms budget"), "{stdout}");
}

#[test]
fn bench_tracks_with_an_author() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let (code, stdout, stderr) = run(&[
        "bench",
        drafted.to_str().unwrap(),
        "--find",
        "draft",
        "--with",
        "final",
        "--author",
        "Bench",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("(4 occurrences in 3 paragraphs"),
        "{stdout}"
    );
}

#[test]
fn bench_reports_no_occurrences() {
    let dir = tempfile::tempdir().unwrap();
    let drafted = drafted_doc(dir.path());
    let (code, _, stderr) = run(&[
        "bench",
        drafted.to_str().unwrap(),
        "--find",
        "nightingale",
        "--with",
        "thrush",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(
        stderr.contains(r#"no occurrences of "nightingale""#),
        "{stderr}"
    );
}

#[test]
fn bench_refuses_like_replace() {
    // A field holds the only match: bench reports the refusal exactly
    // as replace does, without timing anything. The 40 ms gate never
    // runs here, so this test does not depend on the wall clock.
    let dir = tempfile::tempdir().unwrap();
    let template = dir.path().join("template.docx");
    std::fs::copy(SAMPLE, &template).unwrap();
    let content = dir.path().join("field.json");
    std::fs::write(
        &content,
        r#"{"blocks": [{"type": "paragraph", "content": [
            {"type": "text", "text": "See "},
            {"type": "field", "code": "REF tab-sites \\h", "result": "draft table"},
            {"type": "text", "text": " now."}
        ]}]}"#,
    )
    .unwrap();
    let doc = dir.path().join("fielded.docx");
    let (code, _, stderr) = run(&[
        "insert",
        template.to_str().unwrap(),
        doc.to_str().unwrap(),
        "--content",
        content.to_str().unwrap(),
        "--at",
        "results",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let (code, _, stderr) = run(&[
        "bench",
        doc.to_str().unwrap(),
        "--find",
        "draft table",
        "--with",
        "final table",
    ]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("refused 1 paragraph"), "{stderr}");
    assert!(stderr.contains("(field)"), "{stderr}");
}
