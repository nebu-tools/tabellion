//! Generates a large neutral document from a template, for benchmarks and
//! Word checks: chapters with headings, body text, captioned tables with
//! merged header cells, cross-references, bullet lists and footnotes.
//!
//!   cargo run --release -p nebu-docx --example generate -- TEMPLATE CHAPTERS OUT

use nebu_docx::{Block, Cell, Document, Figure, FigureKind, Inline, Marks, Place, Roles, Table, WriteOptions};

const WORDS: &[&str] = &[
    "sample", "measure", "region", "period", "average", "change", "value", "group", "result", "estimate", "total",
    "share", "interval", "baseline", "trend", "series", "count", "rate", "index", "level", "range", "model",
];

fn sentence(seed: &mut u64, n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let w = WORDS[(*seed >> 33) as usize % WORDS.len()];
        if i == 0 {
            let mut c = w.chars();
            s.push_str(&c.next().unwrap().to_uppercase().collect::<String>());
            s.push_str(c.as_str());
        } else {
            s.push(' ');
            s.push_str(w);
        }
    }
    s.push('.');
    s
}

fn para(inlines: Vec<Inline>) -> Block {
    Block::Paragraph { inlines, style: None, align: None }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (tpl, n, out) = (&a[1], a[2].parse::<usize>().unwrap(), &a[3]);
    let t0 = std::time::Instant::now();
    let mut doc = Document::open(tpl).unwrap();
    let roles = Roles::detect(doc.styles().unwrap());
    let o = WriteOptions { roles, ..Default::default() };
    let mut seed = 7u64;
    let mut blocks = vec![];
    for ch in 1..=n {
        let label = format!("tab-{ch}");
        blocks.push(Block::Heading { level: 1, inlines: vec![Inline::text(format!("Chapter {ch}"))], label: None });
        for _ in 0..3 {
            let mut inl = vec![Inline::text(format!("{} {} ", sentence(&mut seed, 14), sentence(&mut seed, 18)))];
            inl.push(Inline::Text { text: "See ".into(), marks: Marks { italic: true, ..Default::default() } });
            inl.push(Inline::Ref { target: label.clone() });
            inl.push(Inline::text(format!(". {}", sentence(&mut seed, 22))));
            blocks.push(para(inl));
        }
        let mut rows = vec![vec![
            Cell { rowspan: 2, ..Cell::text("Region") },
            Cell { colspan: 2, ..Cell::text("Period A") },
            Cell { colspan: 2, ..Cell::text("Period B") },
            Cell { rowspan: 2, ..Cell::text("Change") },
        ]];
        rows.push(vec![Cell::text("n"), Cell::text("Mean"), Cell::text("n"), Cell::text("Mean")]);
        for r in 0..12 {
            let v = (ch * 37 + r * 11) % 97;
            rows.push(vec![
                Cell::text(format!("Region {}", r + 1)),
                Cell::text(format!("{}", 100 + v)),
                Cell::text(format!("{:.2}", v as f64 / 7.0)),
                Cell::text(format!("{}", 120 + v)),
                Cell::text(format!("{:.2}", v as f64 / 6.0)),
                Cell::text(format!("{:+.1} %", (v as f64 - 48.0) / 3.0)),
            ]);
        }
        let mut align = vec![Some(nebu_docx::content::Align::Right); 6];
        align[0] = None;
        blocks.push(Block::Figure(Figure {
            kind: FigureKind::Table,
            body: vec![Block::Table(Table { style: Some("Report Table".into()), widths: vec![2.0, 1.0, 1.0, 1.0, 1.0, 1.2], header_rows: 2, rows, align })],
            caption: Some(vec![Inline::text(format!("Values by region, chapter {ch}"))]),
            label: Some(label.clone()),
        }));
        blocks.push(Block::Heading { level: 2, inlines: vec![Inline::text(format!("Notes on chapter {ch}"))], label: None });
        blocks.push(para(vec![
            Inline::text(sentence(&mut seed, 25)),
            Inline::Footnote { blocks: vec![para(vec![Inline::text(sentence(&mut seed, 9))])] },
            Inline::text(format!(" {}", sentence(&mut seed, 20))),
        ]));
        blocks.push(Block::List {
            ordered: false,
            items: (0..3).map(|_| vec![para(vec![Inline::text(sentence(&mut seed, 8))])]).collect(),
        });
    }
    doc.insert(Place::End, &blocks, &o).unwrap();
    doc.save(out).unwrap();
    eprintln!("{n} chapters in {:.2?}", t0.elapsed());
}
