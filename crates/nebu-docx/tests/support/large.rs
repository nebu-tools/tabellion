//! A large document made from the sample template, for tests, timings and
//! checks in Word: chapters with headings, text with cross-references,
//! captioned tables with merged header cells, footnotes and bullet lists.
//! 300 chapters give 271 pages in Word and a 5.9 MB `word/document.xml`.
//!
//! The XML is written by hand and inserted with the engine's own tree
//! edits, so the generator needs nothing beyond the engine core.

use nebu_docx::{Document, Error, ns};

const WORDS: &[&str] = &[
    "nest",
    "count",
    "transect",
    "plot",
    "season",
    "visit",
    "species",
    "pair",
    "territory",
    "estimate",
    "total",
    "share",
    "interval",
    "baseline",
    "trend",
    "series",
    "observer",
    "rate",
    "index",
    "habitat",
    "range",
    "model",
];

/// A pseudo-random sentence of `n` words.
fn sentence(seed: &mut u64, n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let w = WORDS[(*seed >> 33) as usize % WORDS.len()];
        if i == 0 {
            let mut c = w.chars();
            s.extend(c.next().map(|c| c.to_ascii_uppercase()));
            s.push_str(c.as_str());
        } else {
            s.push(' ');
            s.push_str(w);
        }
    }
    s.push('.');
    s
}

fn run(text: &str) -> String {
    format!("<w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
}

fn para(style: &str, runs: &str) -> String {
    format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>{runs}</w:p>")
}

/// A complex field with its cached result.
fn field(code: &str, result: &str) -> String {
    format!(
        "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> {code} </w:instrText></w:r>\
         <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>"
    )
}

fn cell(width: u32, text: &str, props: &str, right: bool) -> String {
    let jc = if right { "<w:jc w:val=\"right\"/>" } else { "" };
    format!(
        "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>{props}</w:tcPr><w:p><w:pPr><w:pStyle w:val=\"TableText\"/>{jc}</w:pPr>{}</w:p></w:tc>",
        if text.is_empty() {
            String::new()
        } else {
            run(text)
        }
    )
}

fn table(chapter: usize) -> String {
    let widths = [2016u32, 1296, 1296, 1296, 1296, 1872];
    let mut x = String::from(
        "<w:tbl><w:tblPr><w:tblStyle w:val=\"ReportTable\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/>\
         <w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"1\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>",
    );
    for w in widths {
        x.push_str(&format!("<w:gridCol w:w=\"{w}\"/>"));
    }
    x.push_str("</w:tblGrid>");
    let header = "<w:trPr><w:tblHeader/></w:trPr>";
    x.push_str(&format!(
        "<w:tr>{header}{}{}{}{}</w:tr>",
        cell(widths[0], "Site", "<w:vMerge w:val=\"restart\"/>", false),
        cell(
            widths[1] + widths[2],
            "Season A",
            "<w:gridSpan w:val=\"2\"/>",
            false
        ),
        cell(
            widths[3] + widths[4],
            "Season B",
            "<w:gridSpan w:val=\"2\"/>",
            false
        ),
        cell(widths[5], "Change", "<w:vMerge w:val=\"restart\"/>", false),
    ));
    x.push_str(&format!(
        "<w:tr>{header}{}{}{}{}{}{}</w:tr>",
        cell(widths[0], "", "<w:vMerge/>", false),
        cell(widths[1], "Pairs", "", true),
        cell(widths[2], "Mean", "", true),
        cell(widths[3], "Pairs", "", true),
        cell(widths[4], "Mean", "", true),
        cell(widths[5], "", "<w:vMerge/>", false),
    ));
    for r in 0..12 {
        let v = (chapter * 37 + r * 11) % 97;
        x.push_str(&format!(
            "<w:tr>{}{}{}{}{}{}</w:tr>",
            cell(widths[0], &format!("Site {}", r + 1), "", false),
            cell(widths[1], &format!("{}", 100 + v), "", true),
            cell(widths[2], &format!("{:.2}", v as f64 / 7.0), "", true),
            cell(widths[3], &format!("{}", 120 + v), "", true),
            cell(widths[4], &format!("{:.2}", v as f64 / 6.0), "", true),
            cell(
                widths[5],
                &format!("{:+.1} %", (v as f64 - 48.0) / 3.0),
                "",
                true
            ),
        ));
    }
    x.push_str("</w:tbl>");
    x
}

/// The body XML of one chapter and the text of its footnote.
fn chapter(n: usize, seed: &mut u64, bookmark: u32) -> (String, String) {
    let label = format!("_Ref_tab_{n}");
    let mut x = para("Heading1", &run(&format!("Chapter {n}")));
    for _ in 0..3 {
        let mut runs = run(&format!("{} {} ", sentence(seed, 14), sentence(seed, 18)));
        runs.push_str("<w:r><w:rPr><w:i/></w:rPr><w:t xml:space=\"preserve\">See </w:t></w:r>");
        runs.push_str(&field(&format!("REF {label} \\h"), &format!("Table {n}")));
        runs.push_str(&run(&format!(". {}", sentence(seed, 22))));
        x.push_str(&para("BodyText", &runs));
    }
    let caption = format!(
        "<w:bookmarkStart w:id=\"{bookmark}\" w:name=\"{label}\"/>{}{}<w:bookmarkEnd w:id=\"{bookmark}\"/>{}",
        run("Table "),
        field("SEQ Table \\* ARABIC", &n.to_string()),
        run(&format!(": Breeding pairs by site, chapter {n}")),
    );
    x.push_str(&para("Caption", &caption));
    x.push_str(&table(n));
    x.push_str(&para("Heading2", &run(&format!("Notes on chapter {n}"))));
    let note_ref = format!(
        "<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteReference w:id=\"{n}\"/></w:r>"
    );
    x.push_str(&para(
        "BodyText",
        &format!(
            "{}{note_ref}{}",
            run(&sentence(seed, 25)),
            run(&format!(" {}", sentence(seed, 20)))
        ),
    ));
    for _ in 0..3 {
        x.push_str(&para("ListBullet", &run(&sentence(seed, 8))));
    }
    (x, sentence(seed, 9))
}

/// Builds a document of `chapters` chapters on the sample template.
pub fn generate(template: &[u8], chapters: usize) -> Result<Vec<u8>, Error> {
    let mut doc = Document::from_bytes(template)?;
    let mut seed = 7u64;
    let mut body_xml = String::new();
    let mut notes_xml = String::new();
    for n in 1..=chapters {
        let (x, note) = chapter(n, &mut seed, 1000 + n as u32);
        body_xml.push_str(&x);
        notes_xml.push_str(&format!(
            "<w:footnote w:id=\"{n}\"><w:p><w:pPr><w:pStyle w:val=\"FootnoteText\"/></w:pPr>\
             <w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteRef/></w:r>{}</w:p></w:footnote>",
            run(&format!(" {note}"))
        ));
    }
    let w = [("w", ns::W)];

    let main = doc.main_part().to_string();
    let t = doc.xml(&main)?;
    let body = t
        .child(t.root_element(), Some(ns::W), "body")
        .unwrap()
        .expect("the template has a body");
    let sect = t
        .child(body, Some(ns::W), "sectPr")
        .unwrap()
        .expect("the template has section properties");
    for node in t.parse_fragment(&body_xml, &w)? {
        t.insert_before(sect, node)?;
    }

    let t = doc.xml("word/footnotes.xml")?;
    let root = t.root_element();
    for node in t.parse_fragment(&notes_xml, &w)? {
        t.append(root, node)?;
    }
    doc.to_bytes()
}
