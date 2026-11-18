//! Times inserting chapters into a document: each a heading with a label,
//! paragraphs with a footnote and cross-references, a captioned table and
//! a list. Prints the median of five runs.
//!
//!     cargo run --release -p nebu-docx --example chapters -- samples/report-template.docx 300
use std::time::Instant;

use nebu_docx::content::{Block, Cell, Inline, List, ListItem, Row, Table};
use nebu_docx::{Document, Place, WriteOptions};

fn para(content: Vec<Inline>) -> Block {
    Block::Paragraph {
        style: None,
        align: None,
        content,
    }
}

fn chapters(n: usize) -> Vec<Block> {
    let mut out = vec![];
    for i in 0..n {
        out.push(Block::Heading {
            level: 1,
            content: vec![Inline::text(&format!("Site {i}"))],
            label: Some(format!("sec-{i}")),
        });
        for _ in 0..3 {
            out.push(para(vec![
                Inline::text("Wrens sang at the reed beds; the counts are in "),
                Inline::Reference {
                    label: format!("tab-{i}"),
                },
                Inline::text("."),
            ]));
        }
        out.push(para(vec![
            Inline::text("Counted from song at dawn"),
            Inline::Footnote {
                content: vec![para(vec![Inline::text("Between 05:00 and 08:00.")])],
            },
            Inline::text("."),
        ]));
        let row = |a: &str| Row {
            cells: (0..4)
                .map(|k| Cell::new(vec![para(vec![Inline::text(&format!("{a} {k}"))])]))
                .collect(),
        };
        out.push(Block::Table(Table {
            rows: (0..5)
                .map(|r| row(if r == 0 { "Visit" } else { "12" }))
                .collect(),
            header_rows: 1,
            first_column: false,
            style: None,
            widths: None,
            caption: Some(vec![Inline::text("Counts by visit")]),
            label: Some(format!("tab-{i}")),
        }));
        out.push(Block::List(List {
            ordered: true,
            items: ["Walk the transect", "Count the pairs", "Note the weather"]
                .iter()
                .map(|t| ListItem {
                    content: vec![Inline::text(t)],
                    list: None,
                })
                .collect(),
        }));
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1])?;
    let n: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
    let blocks = chapters(n);
    let mut times = vec![];
    let mut size = 0;
    for _ in 0..5 {
        let mut doc = Document::from_bytes(bytes.clone())?;
        let t = Instant::now();
        doc.insert(Place::End, &blocks, &WriteOptions::default())?;
        times.push(t.elapsed());
        size = doc.read_part("word/document.xml")?.len();
    }
    times.sort();
    println!(
        "{n} chapters, {} blocks, document.xml {size} bytes: insert {:?} (median of 5)",
        blocks.len(),
        times[2]
    );
    Ok(())
}
