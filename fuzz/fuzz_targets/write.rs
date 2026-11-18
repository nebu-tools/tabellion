//! The writer on content made from any bytes, put into the sample template
//! at a place the bytes choose: nothing may panic, an error leaves the
//! document as it was, and a document written opens again and gives every
//! view.
//!
//! The bytes drive a small generator of the content model, which makes
//! tables with any spans, labels and references that may or may not match,
//! raw OOXML that may not fit, footnotes in footnotes and the like, so that
//! the checks and the writer meet what a random JSON file would rarely
//! reach.
#![no_main]

use libfuzzer_sys::fuzz_target;
use nebu_docx::content::{
    Align, Block, Border, Cell, CellBorders, Color, Image, Inline, LineStyle, LinkTarget, List,
    ListItem, Marks, Math, Row, Table, TableStyleDef,
};
use nebu_docx::{Content, Document, Place, TextOptions, WriteOptions};

const SAMPLE: &[u8] = include_bytes!("../../samples/report-template.docx");

struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        match self.0.split_first() {
            Some((b, rest)) => {
                self.0 = rest;
                *b
            }
            None => 0,
        }
    }

    fn below(&mut self, n: u8) -> u8 {
        self.byte() % n.max(1)
    }

    fn text(&mut self) -> String {
        const WORDS: &[&str] = &["wren", "heron", " ", "\t", "\n", "<&>", "]]>", "\r", "\u{1}", "é", ""];
        (0..self.below(4)).map(|_| WORDS[self.below(WORDS.len() as u8) as usize]).collect()
    }

    fn label(&mut self) -> String {
        ["lab-a", "lab-b", "x y", "", "summary", "9lives"][self.below(6) as usize].to_string()
    }
}

fn png(w: u8, h: u8) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend(u32::from(w).to_be_bytes());
    b.extend(u32::from(h).to_be_bytes());
    b.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    b
}

fn inline(b: &mut Bytes<'_>, depth: u8) -> Inline {
    match b.below(11) {
        0 | 1 => Inline::Text {
            text: b.text(),
            marks: Marks {
                bold: b.below(2) == 0,
                superscript: b.below(3) == 0,
                subscript: b.below(3) == 0,
                code: b.below(4) == 0,
                color: (b.below(3) == 0).then(|| Color::new("385723").unwrap()),
                size: (b.below(4) == 0).then(|| f64::from(b.byte())),
                ..Marks::default()
            },
        },
        2 if depth < 3 => Inline::Footnote { content: blocks(b, depth + 1) },
        3 => Inline::Reference { label: b.label() },
        4 if depth < 3 => Inline::Link {
            target: match b.below(3) {
                0 => LinkTarget::Url("https://example.org/a b".into()),
                1 => LinkTarget::Bookmark(b.label()),
                _ => LinkTarget::Label(b.label()),
            },
            content: (0..b.below(3)).map(|_| inline(b, depth + 1)).collect(),
        },
        5 => Inline::Math {
            math: Math::omml(["<m:oMath><m:r><m:t>x</m:t></m:r></m:oMath>", "<m:oMath><w:p/></m:oMath>", "<m:r/>"][b.below(3) as usize]),
        },
        6 => Inline::LineBreak,
        7 => Inline::Field { code: b.text(), result: b.text() },
        8 => Inline::Image {
            image: Image {
                width: (b.below(3) == 0).then(|| f64::from(b.byte())),
                ..Image::new(match b.below(4) {
                    0 => png(b.byte(), b.byte()),
                    1 => b"GIF89a\x10\0\x10\0".to_vec(),
                    2 => br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"/>"#.to_vec(),
                    _ => vec![0xFF, 0xD8, 0xFF, b.byte(), b.byte()],
                })
            },
        },
        9 => Inline::Raw {
            xml: ["<w:r><w:t>r</w:t></w:r>", "<w:p/>", "<w:r>", "<w:bookmarkStart w:id=\"9\" w:name=\"b\"/>"][b.below(4) as usize].into(),
        },
        _ => Inline::text("plain"),
    }
}

fn inlines(b: &mut Bytes<'_>, depth: u8) -> Vec<Inline> {
    (0..b.below(4)).map(|_| inline(b, depth)).collect()
}

fn list(b: &mut Bytes<'_>, depth: u8) -> List {
    List {
        ordered: b.below(2) == 0,
        items: (0..b.below(4))
            .map(|_| ListItem {
                content: inlines(b, depth),
                list: (depth < 10 && b.below(3) == 0).then(|| Box::new(list(b, depth + 1))),
            })
            .collect(),
    }
}

fn table(b: &mut Bytes<'_>, depth: u8) -> Table {
    let rows = b.below(4);
    Table {
        rows: (0..rows)
            .map(|_| Row {
                cells: (0..b.below(4))
                    .map(|_| Cell {
                        row_span: usize::from(b.below(3)),
                        column_span: usize::from(b.below(3)),
                        align: (b.below(3) == 0).then_some(Align::Center),
                        shading: (b.below(3) == 0).then(|| Color::new("E2EFDA").unwrap()),
                        borders: (b.below(4) == 0).then(|| CellBorders {
                            top: Some(Border { width: f64::from(b.byte()) / 16.0, color: Color::new("000000").unwrap(), style: LineStyle::Dashed }),
                            ..CellBorders::default()
                        }),
                        ..Cell::new(if depth < 3 { blocks(b, depth + 1) } else { vec![] })
                    })
                    .collect(),
            })
            .collect(),
        header_rows: usize::from(b.below(3)),
        first_column: b.below(2) == 0,
        style: match b.below(4) {
            0 => Some("Field Green".into()),
            1 => Some("Nonesuch".into()),
            2 => Some("Report Table".into()),
            _ => None,
        },
        widths: (b.below(4) == 0).then(|| vec![1.0, 2.0]),
        caption: (b.below(2) == 0).then(|| inlines(b, depth)),
        label: (b.below(2) == 0).then(|| b.label()),
    }
}

fn block(b: &mut Bytes<'_>, depth: u8) -> Block {
    match b.below(9) {
        0 | 1 => Block::Paragraph { style: (b.below(5) == 0).then(|| "Body Text".to_string()), align: None, content: inlines(b, depth) },
        2 => Block::Heading { level: b.below(11), content: inlines(b, depth), label: (b.below(2) == 0).then(|| b.label()) },
        3 => Block::List(list(b, depth)),
        4 => Block::Table(table(b, depth)),
        5 => Block::Figure { image: Image::new(png(b.byte(), b.byte())), caption: inlines(b, depth), label: (b.below(2) == 0).then(|| b.label()) },
        6 => Block::Equation { math: Math::omml("<m:oMath><m:r><m:t>a</m:t></m:r></m:oMath>"), label: (b.below(2) == 0).then(|| b.label()) },
        7 => Block::PageBreak,
        _ => Block::Raw { xml: ["<w:p/>", "<w:tbl/>", "<w:r/>", "text", "<w:p><w:r><w:t>x</w:t></w:r></w:p>"][b.below(5) as usize].into() },
    }
}

fn blocks(b: &mut Bytes<'_>, depth: u8) -> Vec<Block> {
    (0..b.below(5)).map(|_| block(b, depth)).collect()
}

fuzz_target!(|data: &[u8]| {
    let mut b = Bytes(data);
    let content = Content {
        blocks: blocks(&mut b, 0),
        table_styles: vec![TableStyleDef { name: "Field Green".into(), band_fill: Some(Color::new("E2EFDA").unwrap()), ..TableStyleDef::default() }],
    };
    let mut doc = Document::from_bytes(SAMPLE.to_vec()).unwrap();
    let (where_, at) = (b.below(4), b.byte());
    let place = {
        let v = doc.views().unwrap();
        let paragraphs = v.paragraphs();
        match (where_, paragraphs.get(usize::from(at) % paragraphs.len().max(1))) {
            (0, Some(p)) => Place::Before(p.node),
            (1, Some(p)) => Place::After(p.node),
            (2, Some(p)) => Place::Replace(p.node),
            _ => Place::End,
        }
    };
    let before = doc.to_bytes().unwrap();
    match doc.insert(place, &content.blocks, &WriteOptions::for_content(&content, None)) {
        Ok(_) => {
            let out = doc.to_bytes().unwrap();
            let mut again = Document::from_bytes(out).expect("a written document opens");
            let v = again.views().expect("a written document gives its views");
            v.text(&TextOptions::default());
        }
        Err(_) => assert_eq!(doc.to_bytes().unwrap(), before, "a failed insertion changed the document"),
    }
});
