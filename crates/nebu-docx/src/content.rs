//! The content model that every authoring path produces: Typst, Markdown, R
//! data frames, JSON. It says what the content is, not how it looks; the
//! writer in [`crate::write`] turns it into WordprocessingML in the target
//! document's own styles.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph {
        #[serde(default)]
        inlines: Vec<Inline>,
        /// A paragraph style (id, name or role such as `body`). None: the body role.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        align: Option<Align>,
    },
    Heading {
        level: u8,
        inlines: Vec<Inline>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Table(Table),
    Figure(Figure),
    Image(Image),
    /// Display math.
    Equation {
        math: Math,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    PageBreak,
    /// A WordprocessingML fragment, inserted as is. The escape hatch.
    Raw { xml: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Marks {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strike: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sub: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sup: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub small_caps: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub code: bool,
    /// RGB hex without '#'.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_pt: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// A character style (id or name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inline {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "is_default")]
        marks: Marks,
    },
    Footnote { blocks: Vec<Block> },
    /// A cross-reference to a label: becomes a REF field.
    Ref { target: String },
    Link { url: String, inlines: Vec<Inline> },
    Math { math: Math },
    LineBreak,
    /// Any field. `result` is the text shown until fields update.
    Field { code: String, result: String },
    Image(Image),
    Raw { xml: String },
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

impl Inline {
    pub fn text(s: impl Into<String>) -> Inline {
        Inline::Text { text: s.into(), marks: Marks::default() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Table {
    /// A table style: id or name in the document, or the name of a definition
    /// passed alongside. None: the pack's default table style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Relative column widths. Empty: equal.
    #[serde(default)]
    pub widths: Vec<f32>,
    /// Rows that repeat on every page and take the style's header look.
    #[serde(default)]
    pub header_rows: usize,
    /// Rows as in HTML: a cell spanning rows is left out of the rows below.
    pub rows: Vec<Vec<Cell>>,
    /// Columns whose text aligns right (numbers).
    #[serde(default)]
    pub align: Vec<Option<Align>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cell {
    pub blocks: Vec<Block>,
    #[serde(default = "one")]
    pub colspan: usize,
    #[serde(default = "one")]
    pub rowspan: usize,
    /// Shading, RGB hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
}

fn one() -> usize {
    1
}

impl Cell {
    pub fn text(s: impl Into<String>) -> Cell {
        Cell::new(vec![Block::Paragraph { inlines: vec![Inline::text(s)], style: None, align: None }])
    }

    pub fn new(blocks: Vec<Block>) -> Cell {
        Cell { blocks, colspan: 1, rowspan: 1, fill: None, align: None }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FigureKind {
    Figure,
    Table,
    Equation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Figure {
    pub kind: FigureKind,
    pub body: Vec<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<Vec<Inline>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Image {
    /// File name hint, e.g. `plot.png`; the extension picks the format.
    pub name: String,
    #[serde(with = "base64_bytes")]
    pub data: Vec<u8>,
    /// Size in EMU. Zero: from the image's pixel size and resolution.
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
    #[serde(default)]
    pub alt: String,
    /// A PNG fallback for vector formats that older readers cannot show.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "opt_base64")]
    pub fallback_png: Option<Vec<u8>>,
}

/// Office Math, in the shape OMML has.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Math {
    Row { items: Vec<Math> },
    /// A run of identifiers, numbers or operators. `normal`: upright text.
    Run { text: String, #[serde(default)] normal: bool },
    Frac { num: Box<Math>, den: Box<Math> },
    Scripts { base: Box<Math>, sub: Option<Box<Math>>, sup: Option<Box<Math>> },
    Root { index: Option<Box<Math>>, radicand: Box<Math> },
    Delim { open: String, close: String, body: Box<Math> },
    Accent { accent: String, base: Box<Math> },
    /// A big operator such as a sum or integral with limits.
    Nary { op: String, sub: Option<Box<Math>>, sup: Option<Box<Math>>, body: Box<Math> },
    Matrix { rows: Vec<Vec<Math>> },
    /// Something OMML cannot express, embedded as a picture.
    Image { image: Image },
}

mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::b64::encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        super::b64::decode(&s).map_err(serde::de::Error::custom)
    }
}

mod opt_base64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => s.serialize_some(&super::b64::encode(b)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        s.map(|x| super::b64::decode(&x).map_err(serde::de::Error::custom)).transpose()
    }
}

pub(crate) mod b64 {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    pub fn encode(v: &[u8]) -> String {
        let mut s = String::with_capacity(v.len().div_ceil(3) * 4);
        for c in v.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }
    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        let mut out = vec![];
        let mut buf = 0u32;
        let mut bits = 0;
        for ch in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
            let v = A.iter().position(|&a| a == ch).ok_or("invalid base64")? as u32;
            buf = buf << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        Ok(out)
    }
}
