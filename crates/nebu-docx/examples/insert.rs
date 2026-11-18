//! Inserts content from a JSON file into a document.
//!
//!     cargo run -p nebu-docx --example insert -- in.docx content.json out.docx
use nebu_docx::{Content, Document, Place, WriteOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let mut doc = Document::open(&a[1])?;
    let content = Content::from_json(&std::fs::read(&a[2])?)?;
    let dir = std::path::Path::new(&a[2])
        .parent()
        .map(|p| p.to_path_buf());
    let opts = WriteOptions::for_content(&content, dir);
    let r = doc.insert(Place::End, &content.blocks, &opts)?;
    eprintln!("{r:#?}");
    doc.save(&a[3])?;
    Ok(())
}
