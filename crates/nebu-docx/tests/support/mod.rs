//! Shared by the integration tests: zip helpers and a corpus of documents
//! with unusual but legal XML.

#![allow(dead_code)]

pub mod edits;
pub mod jsonschema;
pub mod large;
pub mod xsd;
pub mod zipraw;

use std::io::{Cursor, Read, Write};
use std::sync::OnceLock;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// The sample template from `samples/`.
pub fn sample() -> Vec<u8> {
    include_bytes!("../../../../samples/report-template.docx").to_vec()
}

/// The large generated document, made once per test binary.
pub fn large() -> &'static [u8] {
    static LARGE: OnceLock<Vec<u8>> = OnceLock::new();
    LARGE.get_or_init(|| large::generate(&sample(), 300).expect("the generator works"))
}

/// The decompressed entries in archive order.
pub fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut a = ZipArchive::new(Cursor::new(bytes)).expect("a zip archive");
    (0..a.len())
        .map(|i| {
            let mut f = a.by_index(i).expect("an entry");
            let mut v = vec![];
            f.read_to_end(&mut v).expect("entry bytes");
            (f.name().expect("a name").to_string(), v)
        })
        .collect()
}

pub fn entry(bytes: &[u8], name: &str) -> Vec<u8> {
    entries(bytes)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, b)| b)
        .expect("the entry exists")
}

/// A zip archive of the given entries, deflated unless `stored`.
pub fn zip(files: &[(&str, &[u8])], stored: bool) -> Vec<u8> {
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    let method = if stored {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    };
    for (name, data) in files {
        if name.ends_with('/') {
            w.add_directory(*name, SimpleFileOptions::default())
                .expect("a directory entry");
            continue;
        }
        w.start_file(
            *name,
            SimpleFileOptions::default().compression_method(method),
        )
        .expect("an entry");
        w.write_all(data).expect("entry data");
    }
    w.finish().expect("a finished zip").into_inner()
}

/// The sample with some parts replaced or added (`Some`) or removed (`None`).
pub fn sample_with(changes: &[(&str, Option<Vec<u8>>)]) -> Vec<u8> {
    let mut files = entries(&sample());
    for (name, data) in changes {
        match (files.iter().position(|(n, _)| n == name), data) {
            (Some(i), Some(d)) => files[i].1 = d.clone(),
            (Some(i), None) => {
                files.remove(i);
            }
            (None, Some(d)) => files.push((name.to_string(), d.clone())),
            (None, None) => {}
        }
    }
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    zip(&refs, false)
}

fn sample_part(name: &str) -> String {
    String::from_utf8(entry(&sample(), name)).expect("UTF-8")
}

fn utf16(s: &str, big_endian: bool) -> Vec<u8> {
    let mut v = if big_endian {
        vec![0xFE, 0xFF]
    } else {
        vec![0xFF, 0xFE]
    };
    for u in s.encode_utf16() {
        v.extend_from_slice(&if big_endian {
            u.to_be_bytes()
        } else {
            u.to_le_bytes()
        });
    }
    v
}

/// Rewrites element and attribute prefixes `w:` as `to:`.
pub fn reprefix(xml: &str, to: &str) -> String {
    xml.replace("<w:", &format!("<{to}:"))
        .replace("</w:", &format!("</{to}:"))
        .replace(" w:", &format!(" {to}:"))
        .replace("xmlns:w=", &format!("xmlns:{to}="))
}

/// Documents with unusual but legal XML, each a complete .docx.
pub fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    let doc = sample_part("word/document.xml");
    let styles = sample_part("word/styles.xml");
    let app = sample_part("docProps/app.xml");
    let core = sample_part("docProps/core.xml");

    // WordprocessingML bound to another prefix.
    let prefixed = reprefix(&doc, "x");

    // WordprocessingML as the default namespace for elements; attributes
    // still need a prefix.
    let default_ns = doc
        .replace("<w:", "<")
        .replace("</w:", "</")
        .replace("xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"",
            "xmlns=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"");

    // Comments, processing instructions, CDATA, references, both quote
    // styles, odd white space inside tags and CRLF line ends.
    let markup = doc
        .replace("standalone=\"yes\"?>", "standalone='yes' ?>\r\n<!-- generated for a test -->\r\n<?test-pi some data?>")
        .replace("<w:body>", "<w:body >\r\n<!-- the body -->\r\n")
        .replace(
            "<w:t>The summary goes here.</w:t>",
            "<w:t xml:space='preserve'>Birds &amp; nests &#169; &#x263A; &lt;counted&gt; &quot;twice&quot; &apos;once&apos; <![CDATA[<raw> & ]] text]]></w:t>",
        )
        .replace("<w:pStyle w:val=\"Heading1\"/>", "<w:pStyle\r\n  w:val = 'Heading1' />")
        .replace("</w:body>", "<?keep this?></w:body\r\n>");

    let custom = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- custom XML -->\n<data xmlns=\"urn:example:survey\" xmlns:s=\"urn:example:sites\">\n  <s:site s:id='1' name=\"North &amp; East\"><![CDATA[a < b]]></s:site>\n  <plain xmlns=\"\"><?pi inside?>text</plain>\n</data>\n";
    let custom_rels_type =
        "<Override PartName=\"/customXml/item1.xml\" ContentType=\"application/xml\"/>";
    let types = sample_part("[Content_Types].xml")
        .replace("</Types>", &format!("{custom_rels_type}</Types>"));

    let mut bom_styles = b"\xEF\xBB\xBF".to_vec();
    bom_styles.extend_from_slice(styles.as_bytes());

    let app16 = utf16(
        &app.replace("encoding=\"UTF-8\"", "encoding=\"UTF-16\""),
        false,
    );
    let core16 = utf16(
        &core.replace("encoding=\"UTF-8\"", "encoding=\"UTF-16\""),
        true,
    );

    let stored = {
        let mut files = entries(&sample());
        files.insert(1, ("word/".to_string(), vec![]));
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        zip(&refs, true)
    };

    vec![
        ("sample", sample()),
        ("large", large().to_vec()),
        (
            "prefixed",
            sample_with(&[("word/document.xml", Some(prefixed.into_bytes()))]),
        ),
        (
            "default-namespace",
            sample_with(&[("word/document.xml", Some(default_ns.into_bytes()))]),
        ),
        (
            "markup",
            sample_with(&[("word/document.xml", Some(markup.into_bytes()))]),
        ),
        (
            "custom-xml",
            sample_with(&[
                ("customXml/item1.xml", Some(custom.as_bytes().to_vec())),
                ("[Content_Types].xml", Some(types.into_bytes())),
            ]),
        ),
        ("bom", sample_with(&[("word/styles.xml", Some(bom_styles))])),
        (
            "utf16",
            sample_with(&[
                ("docProps/app.xml", Some(app16)),
                ("docProps/core.xml", Some(core16)),
            ]),
        ),
        ("stored", stored),
    ]
}

/// A PNG of a bird on a plain ground, `w` by `h` pixels at 96 per inch, as
/// `corpus/build/png.py` draws it.
pub fn png(w: u32, h: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend(crc32fast::hash(&c).to_be_bytes());
    }
    let mut raw = vec![];
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let (fx, fy) = (x as f64, y as f64);
            let (w, h) = (w as f64, h as f64);
            let body = ((fx - w * 0.45) / (w * 0.28)).powi(2)
                + ((fy - h * 0.6) / (h * 0.2)).powi(2)
                <= 1.0;
            let head = ((fx - w * 0.72) / (w * 0.1)).powi(2)
                + ((fy - h * 0.42) / (h * 0.12)).powi(2)
                <= 1.0;
            raw.extend(if body || head {
                [40, 40, 40]
            } else {
                [200, 225, 190]
            });
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&raw).unwrap();
    let data = z.finish().unwrap();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &data);
    chunk(&mut out, b"IEND", b"");
    out
}

/// Base64 with padding, for pictures in the JSON form.
pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            out.push(if k <= c.len() {
                A[((n >> (18 - 6 * k)) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

pub fn assert_records_kept(before: &[u8], after: &[u8], changed: &[&str]) {
    zipraw::assert_records_kept(before, after, changed)
}
