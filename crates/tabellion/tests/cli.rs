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
    assert_eq!(o.status.code(), Some(2));
    let stderr = String::from_utf8(o.stderr).unwrap();
    assert!(
        stderr.starts_with("tabellion: ") && stderr.contains("not a readable .docx file"),
        "{stderr}"
    );
}
