//! The content model: random content has the shape `schema/content.json`
//! gives, survives its JSON form, and reads back through the views after
//! the writer has put it into a document. The same insertion gives the
//! same bytes, and every document written is valid against the schemas.

mod support;

use std::collections::BTreeSet;
use std::path::Path;

use nebu_docx::content::{
    Align, Block, Border, Cell, CellBorders, Color, Content, Image, Inline, LineStyle, LinkTarget,
    List, ListItem, Marks, Math, Padding, Row, RowLook, Source, Table, TableBorders, TableStyleDef,
    VerticalAlign,
};
use nebu_docx::{Document, Place, StoryKind, WriteOptions};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use serde_json::Value;

fn schema() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/content.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn corpus(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus")
            .join(name),
    )
    .unwrap()
}

const WORDS: &[&str] = &[
    "wren",
    "robin",
    "heron",
    "teal",
    "reed",
    "warbler",
    "marsh",
    "dawn",
    "nest",
    "pairs",
    "<count>",
    "A & B",
    "\u{e9}t\u{e9}",
    "\u{2013}",
    "\"quoted\"",
];

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(WORDS), 1..5).prop_map(|w| w.join(" "))
}

fn color() -> impl Strategy<Value = Color> {
    prop::sample::select(vec!["385723", "E2EFDA", "7F6000", "FFF2CC", "2F5496"])
        .prop_map(|c| Color::new(c).unwrap())
}

fn marks() -> impl Strategy<Value = Marks> {
    (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        0u8..3,
        any::<bool>(),
        any::<bool>(),
        prop::option::of(color()),
        prop::option::of(color()),
        prop::option::of(prop::sample::select(vec![9.0, 11.0, 14.5])),
    )
        .prop_map(
            |(
                bold,
                italic,
                underline,
                strike,
                script,
                small_caps,
                code,
                color,
                highlight,
                size,
            )| Marks {
                bold,
                italic,
                underline,
                strike,
                superscript: script == 1,
                subscript: script == 2,
                small_caps,
                code,
                color,
                highlight,
                size,
            },
        )
}

fn math() -> Math {
    Math::omml(
        "<m:oMath><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath>",
    )
}

/// Inlines that may stand anywhere; `notes` allows footnotes.
fn inline(notes: bool) -> BoxedStrategy<Inline> {
    let mut options: Vec<BoxedStrategy<Inline>> = vec![
        (text(), marks())
            .prop_map(|(text, marks)| Inline::Text { text, marks })
            .boxed(),
        Just(Inline::text("tab\tand line\nbreak")).boxed(),
        Just(Inline::LineBreak).boxed(),
        text()
            .prop_map(|r| Inline::Field {
                code: "DATE \\@ \"d MMMM yyyy\"".into(),
                result: r,
            })
            .boxed(),
        Just(Inline::Math { math: math() }).boxed(),
        (text(), prop::bool::ANY)
            .prop_map(|(t, b)| Inline::Link {
                target: LinkTarget::Url(format!(
                    "https://example.org/{}",
                    if b { "birds" } else { "marsh" }
                )),
                content: vec![Inline::text(&t)],
            })
            .boxed(),
        Just(Inline::Raw {
            xml: "<w:r><w:t>raw</w:t></w:r>".into(),
        })
        .boxed(),
    ];
    if notes {
        options.push(
            text()
                .prop_map(|t| Inline::Footnote {
                    content: vec![Block::Paragraph {
                        style: None,
                        align: None,
                        content: vec![Inline::text(&t)],
                    }],
                })
                .boxed(),
        );
    }
    prop::strategy::Union::new(options).boxed()
}

fn inlines(notes: bool) -> impl Strategy<Value = Vec<Inline>> {
    prop::collection::vec(inline(notes), 1..5)
}

fn paragraph(notes: bool) -> impl Strategy<Value = Block> {
    (
        inlines(notes),
        prop::option::of(prop::sample::select(vec![
            Align::Left,
            Align::Center,
            Align::Right,
            Align::Justify,
        ])),
    )
        .prop_map(|(content, align)| Block::Paragraph {
            style: None,
            align,
            content,
        })
}

fn list(depth: u32) -> BoxedStrategy<List> {
    let item = if depth == 0 {
        inlines(false)
            .prop_map(|content| ListItem {
                content,
                list: None,
            })
            .boxed()
    } else {
        (inlines(false), prop::option::of(list(depth - 1)))
            .prop_map(|(content, l)| ListItem {
                content,
                list: l.map(Box::new),
            })
            .boxed()
    };
    (any::<bool>(), prop::collection::vec(item, 1..4))
        .prop_map(|(ordered, items)| List { ordered, items })
        .boxed()
}

fn border() -> impl Strategy<Value = Border> {
    (
        prop::sample::select(vec![0.5, 1.0, 2.25]),
        color(),
        prop::sample::select(vec![
            LineStyle::Single,
            LineStyle::Double,
            LineStyle::Dashed,
            LineStyle::Dotted,
            LineStyle::None,
        ]),
    )
        .prop_map(|(width, color, style)| Border {
            width,
            color,
            style,
        })
}

/// A valid grid: cells in reading order, each spanning free positions.
fn table(captioned: bool) -> impl Strategy<Value = Table> {
    (
        1usize..5,
        1usize..4,
        prop::collection::vec((1usize..3, 1usize..3, any::<u8>()), 40),
        text(),
        any::<bool>(),
        0usize..2,
    )
        .prop_map(move |(rows, cols, spans, caption, first_column, header)| {
            let mut cover = vec![vec![false; cols]; rows];
            let mut out: Vec<Row> = (0..rows).map(|_| Row { cells: vec![] }).collect();
            let mut k = 0;
            for r in 0..rows {
                for c in 0..cols {
                    if cover[r][c] {
                        continue;
                    }
                    let (rs, cs, f) = spans[k % spans.len()];
                    k += 1;
                    let mut cs = cs.min(cols - c);
                    while (c..c + cs).any(|x| cover[r][x]) {
                        cs -= 1;
                    }
                    let mut rs = rs.min(rows - r);
                    while (r..r + rs).any(|y| (c..c + cs).any(|x| cover[y][x])) {
                        rs -= 1;
                    }
                    for row in cover.iter_mut().skip(r).take(rs) {
                        for x in row.iter_mut().skip(c).take(cs) {
                            *x = true;
                        }
                    }
                    let mut cell = Cell::new(vec![Block::Paragraph {
                        style: None,
                        align: None,
                        content: vec![Inline::text(&format!(
                            "{} {r}.{c}",
                            WORDS[(f as usize) % 10]
                        ))],
                    }]);
                    cell.row_span = rs;
                    cell.column_span = cs;
                    if f % 3 == 0 {
                        cell.shading = Some(Color::new("E2EFDA").unwrap());
                    }
                    if f % 4 == 0 {
                        cell.align = Some(Align::Right);
                        cell.vertical_align = Some(VerticalAlign::Center);
                    }
                    if f % 5 == 0 {
                        cell.borders = Some(CellBorders {
                            bottom: Some(Border {
                                width: 1.0,
                                color: Color::new("385723").unwrap(),
                                style: LineStyle::Single,
                            }),
                            ..CellBorders::default()
                        });
                    }
                    if f % 7 == 0 {
                        cell.content.push(Block::List(List {
                            ordered: true,
                            items: vec![ListItem {
                                content: vec![Inline::text("in a cell")],
                                list: None,
                            }],
                        }));
                    }
                    out[r].cells.push(cell);
                }
            }
            Table {
                rows: out,
                header_rows: header.min(rows),
                first_column,
                style: if first_column {
                    Some("Generated Look".into())
                } else {
                    None
                },
                widths: None,
                caption: captioned.then(|| vec![Inline::text(&caption)]),
                label: None,
            }
        })
}

fn style_def() -> impl Strategy<Value = TableStyleDef> {
    (
        prop::option::of(border()),
        prop::option::of(border()),
        prop::option::of(color()),
        any::<bool>(),
        prop::option::of(color()),
    )
        .prop_map(|(top, inside, band, bold, fill)| TableStyleDef {
            name: "Generated Look".into(),
            font: Some("Arial".into()),
            size: Some(9.0),
            borders: TableBorders {
                top,
                inside_h: inside,
                ..TableBorders::default()
            },
            padding: Some(Padding {
                top: 1.0,
                right: 5.4,
                bottom: 1.0,
                left: 5.4,
            }),
            header: Some(RowLook {
                fill,
                color: Some(Color::new("FFFFFF").unwrap()),
                bold,
                border: None,
            }),
            band_fill: band,
            first_column: Some(RowLook {
                bold: true,
                ..RowLook::default()
            }),
        })
}

fn block() -> impl Strategy<Value = Block> {
    let png = support::png(24, 16);
    prop_oneof![
        3 => paragraph(true),
        1 => (1u8..5, inlines(false)).prop_map(|(level, content)| Block::Heading { level, content, label: None }),
        1 => list(2).prop_map(Block::List),
        1 => (table(true), any::<bool>()).prop_map(|(t, c)| Block::Table(if c { t } else { Table { caption: None, ..t } })),
        1 => (text(), any::<bool>()).prop_map(move |(c, alt)| Block::Figure {
            image: Image { alt: alt.then(|| "A wren".to_string()), ..Image::new(png.clone()) },
            caption: vec![Inline::text(&c)],
            label: None,
        }),
        1 => Just(Block::Equation { math: math(), label: None }),
        1 => Just(Block::Raw { xml: "<w:p><w:r><w:t>raw block</w:t></w:r></w:p>".into() }),
        1 => Just(Block::PageBreak),
    ]
}

/// Content with labels on some captions and headings, and references to
/// each of them at the end.
fn content() -> impl Strategy<Value = Content> {
    (prop::collection::vec(block(), 1..8), style_def()).prop_map(|(mut blocks, def)| {
        let mut labels = vec![];
        for (i, b) in blocks.iter_mut().enumerate() {
            let l = format!("lab-{i}");
            match b {
                Block::Table(t) if t.caption.is_some() => t.label = Some(l.clone()),
                Block::Figure { label, .. }
                | Block::Heading { label, .. }
                | Block::Equation { label, .. } => *label = Some(l.clone()),
                _ => continue,
            }
            labels.push(l);
        }
        if !labels.is_empty() {
            let mut content = vec![Inline::text("See ")];
            for l in &labels {
                content.push(Inline::Reference { label: l.clone() });
                content.push(Inline::text(", "));
            }
            blocks.push(Block::Paragraph {
                style: None,
                align: None,
                content,
            });
        }
        Content {
            blocks,
            table_styles: vec![def],
        }
    })
}

fn plain(list: &[Inline]) -> String {
    let mut s = String::new();
    for i in list {
        match i {
            Inline::Text { text, .. } => s.push_str(text),
            Inline::Link { content, .. } => s.push_str(&plain(content)),
            Inline::Field { result, .. } => s.push_str(result),
            Inline::LineBreak => s.push('\n'),
            Inline::Raw { .. } => s.push_str("raw"),
            Inline::Math { .. } => s.push_str("ab"),
            _ => {}
        }
    }
    s
}

/// The keys of every object in a JSON value.
fn keys(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                out.insert(k.clone());
                keys(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| keys(x, out)),
        _ => {}
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn content_has_the_schema_shape_and_survives_json(c in content()) {
        let root = schema();
        let v: Value = serde_json::from_str(&c.to_json()).unwrap();
        let mut errors = vec![];
        support::jsonschema::validate(&root, &root, &v, "content", &mut errors);
        prop_assert!(errors.is_empty(), "{}", errors.join("\n"));
        let again = Content::from_json(c.to_json().as_bytes()).unwrap();
        prop_assert_eq!(&again, &c);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn written_content_reads_back(c in content(), which in 0usize..3) {
        let source = [support::sample(), corpus("birds.docx"), corpus("cascade.docx")][which].clone();
        let opts = WriteOptions::for_content(&c, None);
        let mut doc = Document::from_bytes(source.clone()).unwrap();
        doc.insert(Place::End, &c.blocks, &opts).unwrap();
        let out = doc.to_bytes().unwrap();
        // The same call gives the same bytes.
        let mut twin = Document::from_bytes(source).unwrap();
        twin.insert(Place::End, &c.blocks, &opts).unwrap();
        prop_assert!(twin.to_bytes().unwrap() == out, "two insertions gave other bytes");
        let errors = support::xsd::validate_docx(&out);
        prop_assert!(errors.is_empty(), "{}", errors.join("\n"));
        let mut back = Document::from_bytes(out).unwrap();
        let v = back.views().unwrap();
        let texts: Vec<&str> = v.paragraphs().iter().map(|p| p.text.as_str()).collect();
        let (mut tables, mut captions, mut notes, mut pictures) = (0, 0, 0, 0);
        for b in &c.blocks {
            match b {
                Block::Paragraph { content, .. } => {
                    notes += content.iter().filter(|i| matches!(i, Inline::Footnote { .. })).count();
                    if !content.iter().any(|i| matches!(i, Inline::Reference { .. } | Inline::Footnote { .. })) {
                        // A paragraph's text has no line end at its end.
                        let want = plain(content);
                        let want = want.trim_end_matches('\n');
                        prop_assert!(texts.contains(&want), "no paragraph {:?}", want);
                    }
                }
                Block::Heading { level, content, .. } => {
                    let want = plain(content);
                    let want = want.trim_end_matches('\n');
                    prop_assert!(
                        v.blocks().iter().any(|x| x.level == Some(*level) && x.text == want),
                        "no heading {} {:?}", level, want
                    );
                }
                Block::Table(t) => {
                    tables += 1;
                    captions += usize::from(t.caption.is_some());
                }
                Block::Figure { .. } => {
                    captions += 1;
                    pictures += 1;
                }
                Block::Equation { label, .. } => captions += usize::from(label.is_some()),
                _ => {}
            }
        }
        // Tables: the last ones of the body, on the same grid.
        let body: Vec<_> = v.tables().iter().filter(|t| t.location.story == StoryKind::Main && t.parent.is_none()).collect();
        let ours = &body[body.len() - tables..];
        let mut k = 0;
        for b in &c.blocks {
            if let Block::Table(t) = b {
                let got = ours[k];
                k += 1;
                prop_assert_eq!(got.row_count, t.rows.len());
                let cells: usize = t.rows.iter().map(|r| r.cells.len()).sum();
                prop_assert_eq!(got.cells.len(), cells);
                for (r, row) in t.rows.iter().enumerate() {
                    for cell in &row.cells {
                        let want = match &cell.content[0] {
                            Block::Paragraph { content, .. } => plain(content),
                            _ => unreachable!(),
                        };
                        prop_assert!(got.cells.iter().any(|g| g.row == r && g.text.starts_with(&want)
                            && g.row_span == cell.row_span && g.column_span == cell.column_span),
                            "row {}: no cell {:?}", r, want);
                    }
                }
            }
        }
        // Captions are SEQ fields after those the document had; labels are
        // bookmarks; each reference shows its target's text.
        let seq = v.fields().iter().filter(|f| f.kind == "SEQ").count();
        prop_assert!(seq >= captions);
        for b in v.bookmarks().iter().filter(|b| b.name.starts_with("_Ref_lab-")) {
            prop_assert!(b.hidden);
            let refs: Vec<_> = v.fields().iter().filter(|f| f.code == format!("REF {} \\h", b.name)).collect();
            prop_assert_eq!(refs.len(), 1, "{}", b.name);
            prop_assert_eq!(&refs[0].result, &b.text);
        }
        let new_notes = v.notes().iter().filter(|n| n.kind == nebu_docx::NoteKind::Footnote && n.text.split(' ').all(|w| !w.is_empty())).count();
        prop_assert!(new_notes >= notes);
        prop_assert!(v.images().len() >= pictures);
    }
}

#[test]
fn every_property_of_the_schema_is_written_by_the_code() {
    // Many generated contents together use every key the schema names.
    let root = schema();
    let mut seen = BTreeSet::new();
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = content();
    for _ in 0..400 {
        let c = strategy.new_tree(&mut runner).unwrap().current();
        let v: Value = serde_json::from_str(&c.to_json()).unwrap();
        keys(&v, &mut seen);
    }
    // What the generator leaves out, written by hand.
    let extra = Content {
        blocks: vec![
            Block::Paragraph {
                style: Some("Body Text".into()),
                align: None,
                content: vec![
                    Inline::Link {
                        target: LinkTarget::Bookmark("summary".into()),
                        content: vec![],
                    },
                    Inline::Link {
                        target: LinkTarget::Label("x".into()),
                        content: vec![],
                    },
                ],
            },
            Block::Image {
                image: Image {
                    source: Source::File("bird.svg".into()),
                    fallback: Some(Source::Data(vec![1])),
                    width: Some(10.0),
                    height: Some(5.0),
                    alt: None,
                    title: Some("t".into()),
                },
            },
            Block::Table(Table {
                rows: vec![],
                header_rows: 0,
                first_column: false,
                style: None,
                widths: Some(vec![1.0]),
                caption: None,
                label: None,
            }),
        ],
        table_styles: vec![TableStyleDef {
            borders: TableBorders {
                bottom: Some(Border {
                    width: 1.0,
                    color: Color::new("000000").unwrap(),
                    style: LineStyle::Single,
                }),
                left: Some(Border {
                    width: 1.0,
                    color: Color::new("000000").unwrap(),
                    style: LineStyle::Single,
                }),
                right: Some(Border {
                    width: 1.0,
                    color: Color::new("000000").unwrap(),
                    style: LineStyle::Single,
                }),
                inside_v: Some(Border {
                    width: 1.0,
                    color: Color::new("000000").unwrap(),
                    style: LineStyle::Single,
                }),
                ..TableBorders::default()
            },
            first_column: Some(RowLook {
                fill: Some(Color::new("000000").unwrap()),
                color: None,
                bold: false,
                border: Some(Border {
                    width: 1.0,
                    color: Color::new("000000").unwrap(),
                    style: LineStyle::Single,
                }),
            }),
            ..TableStyleDef::default()
        }],
    };
    let cell = Cell {
        borders: Some(CellBorders {
            top: Some(Border {
                width: 1.0,
                color: Color::new("000000").unwrap(),
                style: LineStyle::Single,
            }),
            left: Some(Border {
                width: 1.0,
                color: Color::new("000000").unwrap(),
                style: LineStyle::Single,
            }),
            right: Some(Border {
                width: 1.0,
                color: Color::new("000000").unwrap(),
                style: LineStyle::Single,
            }),
            bottom: None,
        }),
        ..Cell::new(vec![])
    };
    let v: Value = serde_json::to_value(&extra).unwrap();
    keys(&v, &mut seen);
    keys(&serde_json::to_value(&cell).unwrap(), &mut seen);
    let mut missing = vec![];
    for (def, p) in support::jsonschema::properties(&root) {
        if !seen.contains(&p) {
            missing.push(format!("{def}.{p}"));
        }
    }
    assert!(
        missing.is_empty(),
        "the code writes no {}",
        missing.join(", ")
    );
}
