//! Writes the large test document: chapters with headings, text with
//! cross-references, captioned tables, footnotes and lists, on a template.
//!
//!   cargo run --release -p nebu-docx --example generate -- TEMPLATE CHAPTERS OUT
//!
//! 300 chapters on `samples/report-template.docx` give 271 pages in Word.

#[path = "../tests/support/large.rs"]
mod large;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [template, chapters, out] = args.as_slice() else {
        eprintln!("usage: generate TEMPLATE CHAPTERS OUT");
        std::process::exit(2);
    };
    let run = || -> Result<(), Box<dyn std::error::Error>> {
        let chapters: usize = chapters.parse()?;
        let bytes = large::generate(&std::fs::read(template)?, chapters)?;
        std::fs::write(out, bytes)?;
        Ok(())
    };
    if let Err(e) = run() {
        eprintln!("generate: {e}");
        std::process::exit(1);
    }
}
