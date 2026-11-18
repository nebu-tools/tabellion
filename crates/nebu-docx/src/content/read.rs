//! Reads the JSON form of the content model as untrusted input.
//!
//! The bytes are parsed into a JSON value first (serde_json stops at 128
//! levels of nesting, so no input can exhaust the stack), then turned into
//! the model by hand, so that every error names the JSON path it is at and
//! says what was wrong there. Unknown keys are errors. The model is then
//! checked as content built in code is ([`super::check`]).

use serde_json::{Map, Value};

use super::{
    Align, Block, Border, Cell, CellBorders, Color, Content, Image, Inline, LineStyle, LinkTarget,
    List, ListItem, Marks, Math, Padding, Row, RowLook, Source, Table, TableBorders, TableStyleDef,
    VerticalAlign,
};
use crate::Error;

/// Bounds on what reading or writing content may cost. The defaults suit
/// far more content than a document holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentLimits {
    /// The size of the JSON form in bytes.
    pub max_bytes: usize,
    /// How deep blocks may nest: a list in a list, a table in a cell, the
    /// content of a footnote, a link's content.
    pub max_depth: usize,
    /// Blocks, inlines, list items, rows and cells together.
    pub max_items: usize,
}

impl Default for ContentLimits {
    fn default() -> ContentLimits {
        ContentLimits {
            max_bytes: 256 << 20,
            max_depth: 32,
            max_items: 4_000_000,
        }
    }
}

pub(super) fn read(json: &[u8], limits: &ContentLimits) -> Result<Content, Error> {
    if json.len() > limits.max_bytes {
        return Err(Error::Invalid(format!(
            "the content is {} bytes of JSON, more than ContentLimits::max_bytes ({})",
            json.len(),
            limits.max_bytes
        )));
    }
    let value: Value = serde_json::from_slice(json).map_err(|e| {
        Error::Invalid(format!(
            "the content is not JSON: {e}{}",
            if e.to_string().contains("recursion") {
                " (nested more than 128 levels)"
            } else {
                ""
            }
        ))
    })?;
    let mut r = Reader { limits, items: 0 };
    let content = r.content(&value)?;
    super::check::content(&content, limits)?;
    Ok(content)
}

/// A path into the JSON value, for errors.
#[derive(Clone)]
struct Path(String);

impl Path {
    fn key(&self, k: &str) -> Path {
        if self.0.is_empty() {
            Path(k.to_string())
        } else {
            Path(format!("{}.{k}", self.0))
        }
    }

    fn at(&self, i: usize) -> Path {
        Path(format!("{}[{i}]", self.0))
    }

    fn err(&self, what: impl std::fmt::Display) -> Error {
        let at = if self.0.is_empty() {
            "the top"
        } else {
            &self.0
        };
        Error::Invalid(format!("{at}: {what}"))
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// An object whose keys are taken one by one; [`Obj::done`] refuses the
/// keys nobody took.
struct Obj<'v> {
    map: &'v Map<String, Value>,
    path: Path,
    taken: Vec<&'static str>,
}

impl<'v> Obj<'v> {
    fn new(v: &'v Value, path: &Path) -> Result<Obj<'v>, Error> {
        match v {
            Value::Object(map) => Ok(Obj {
                map,
                path: path.clone(),
                taken: vec![],
            }),
            other => Err(path.err(format!("expected an object, found {}", kind(other)))),
        }
    }

    fn get(&mut self, k: &'static str) -> Option<(&'v Value, Path)> {
        self.taken.push(k);
        self.map
            .get(k)
            .filter(|v| !v.is_null())
            .map(|v| (v, self.path.key(k)))
    }

    fn need(&mut self, k: &'static str) -> Result<(&'v Value, Path), Error> {
        self.get(k)
            .ok_or_else(|| self.path.err(format!("{k} is missing")))
    }

    fn string(&mut self, k: &'static str) -> Result<String, Error> {
        let (v, p) = self.need(k)?;
        string(v, &p)
    }

    fn opt_string(&mut self, k: &'static str) -> Result<Option<String>, Error> {
        self.get(k).map(|(v, p)| string(v, &p)).transpose()
    }

    fn bool(&mut self, k: &'static str) -> Result<bool, Error> {
        match self.get(k) {
            None => Ok(false),
            Some((Value::Bool(b), _)) => Ok(*b),
            Some((v, p)) => Err(p.err(format!("expected true or false, found {}", kind(v)))),
        }
    }

    fn count(&mut self, k: &'static str, default: usize) -> Result<usize, Error> {
        match self.get(k) {
            None => Ok(default),
            Some((v, p)) => v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| p.err(format!("expected a whole number, found {v}"))),
        }
    }

    fn number(&mut self, k: &'static str) -> Result<Option<f64>, Error> {
        match self.get(k) {
            None => Ok(None),
            Some((v, p)) => match v.as_f64() {
                Some(n) if n.is_finite() => Ok(Some(n)),
                _ => Err(p.err(format!("expected a number, found {}", kind(v)))),
            },
        }
    }

    fn color(&mut self, k: &'static str) -> Result<Option<Color>, Error> {
        match self.get(k) {
            None => Ok(None),
            Some((v, p)) => {
                let s = string(v, &p)?;
                Color::new(&s).map(Some).map_err(|e| p.err(inner(e)))
            }
        }
    }

    fn done(self) -> Result<(), Error> {
        let mut extra: Vec<&String> = self
            .map
            .keys()
            .filter(|k| !self.taken.contains(&k.as_str()))
            .collect();
        extra.sort();
        match extra.first() {
            None => Ok(()),
            Some(k) => {
                let known: Vec<&str> = self
                    .taken
                    .iter()
                    .copied()
                    .filter(|k| *k != "type")
                    .collect();
                Err(self.path.err(format!(
                    "unknown key {k:?}; the keys here are {}",
                    if known.is_empty() {
                        "type".to_string()
                    } else {
                        known.join(", ")
                    }
                )))
            }
        }
    }
}

/// The message of an `Invalid` error without its variant.
fn inner(e: Error) -> String {
    match e {
        Error::Invalid(s) => s,
        e => e.to_string(),
    }
}

fn string(v: &Value, p: &Path) -> Result<String, Error> {
    match v {
        Value::String(s) => Ok(s.clone()),
        other => Err(p.err(format!("expected a string, found {}", kind(other)))),
    }
}

fn array<'v>(v: &'v Value, p: &Path) -> Result<&'v Vec<Value>, Error> {
    match v {
        Value::Array(a) => Ok(a),
        other => Err(p.err(format!("expected an array, found {}", kind(other)))),
    }
}

struct Reader<'l> {
    limits: &'l ContentLimits,
    items: usize,
}

impl Reader<'_> {
    fn item(&mut self, p: &Path) -> Result<(), Error> {
        self.items += 1;
        if self.items > self.limits.max_items {
            return Err(p.err(format!(
                "the content has more than {} blocks, inlines, items, rows and cells; raise ContentLimits::max_items",
                self.limits.max_items
            )));
        }
        Ok(())
    }

    fn deeper(&self, depth: usize, p: &Path) -> Result<usize, Error> {
        if depth >= self.limits.max_depth {
            return Err(p.err(format!(
                "content nests deeper than {} levels; raise ContentLimits::max_depth",
                self.limits.max_depth
            )));
        }
        Ok(depth + 1)
    }

    fn content(&mut self, v: &Value) -> Result<Content, Error> {
        let top = Path(String::new());
        // A bare array is a list of blocks.
        if let Value::Array(_) = v {
            return Ok(Content {
                blocks: self.blocks(v, &top, 0)?,
                table_styles: vec![],
            });
        }
        let mut o = Obj::new(v, &top)?;
        let (b, p) = o.need("blocks")?;
        let blocks = self.blocks(b, &p, 0)?;
        let mut table_styles = vec![];
        if let Some((s, p)) = o.get("table_styles") {
            for (i, d) in array(s, &p)?.iter().enumerate() {
                table_styles.push(table_style(d, &p.at(i))?);
            }
        }
        o.done()?;
        Ok(Content {
            blocks,
            table_styles,
        })
    }

    fn blocks(&mut self, v: &Value, p: &Path, depth: usize) -> Result<Vec<Block>, Error> {
        array(v, p)?
            .iter()
            .enumerate()
            .map(|(i, b)| self.block(b, &p.at(i), depth))
            .collect()
    }

    fn block(&mut self, v: &Value, p: &Path, depth: usize) -> Result<Block, Error> {
        self.item(p)?;
        let mut o = Obj::new(v, p)?;
        let t = o.string("type")?;
        let b = match t.as_str() {
            "paragraph" => Block::Paragraph {
                style: o.opt_string("style")?,
                align: o.get("align").map(|(v, p)| align(v, &p)).transpose()?,
                content: self.inlines_of(&mut o, depth)?,
            },
            "heading" => Block::Heading {
                level: {
                    let n = o.count("level", 0)?;
                    if !(1..=9).contains(&n) {
                        return Err(p
                            .key("level")
                            .err(format!("{n} is not a heading level; use 1 to 9")));
                    }
                    n as u8
                },
                content: self.inlines_of(&mut o, depth)?,
                label: o.opt_string("label")?,
            },
            "list" => Block::List(self.list_body(&mut o, depth)?),
            "table" => Block::Table(self.table(&mut o, depth)?),
            "figure" => {
                let (i, ip) = o.need("image")?;
                let image = image(i, &ip)?;
                let (c, cp) = o.need("caption")?;
                Block::Figure {
                    image,
                    caption: self.inlines(c, &cp, depth)?,
                    label: o.opt_string("label")?,
                }
            }
            "image" => {
                let (i, ip) = o.need("image")?;
                Block::Image {
                    image: image(i, &ip)?,
                }
            }
            "equation" => {
                let (m, mp) = o.need("math")?;
                Block::Equation {
                    math: math(m, &mp)?,
                    label: o.opt_string("label")?,
                }
            }
            "page_break" => Block::PageBreak,
            "raw" => Block::Raw {
                xml: o.string("xml")?,
            },
            other => {
                return Err(p.key("type").err(format!(
                    "{other:?} is not a block; the blocks are paragraph, heading, list, table, figure, image, equation, page_break and raw"
                )));
            }
        };
        o.done()?;
        Ok(b)
    }

    fn inlines_of(&mut self, o: &mut Obj<'_>, depth: usize) -> Result<Vec<Inline>, Error> {
        match o.get("content") {
            Some((v, p)) => self.inlines(v, &p, depth),
            None => Ok(vec![]),
        }
    }

    fn list_body(&mut self, o: &mut Obj<'_>, depth: usize) -> Result<List, Error> {
        let ordered = o.bool("ordered")?;
        let (items, ip) = o.need("items")?;
        let mut out = vec![];
        for (i, it) in array(items, &ip)?.iter().enumerate() {
            let p = ip.at(i);
            self.item(&p)?;
            let mut io = Obj::new(it, &p)?;
            let content = self.inlines_of(&mut io, depth)?;
            let list = match io.get("list") {
                Some((l, lp)) => {
                    let d = self.deeper(depth, &lp)?;
                    let mut lo = Obj::new(l, &lp)?;
                    let list = self.list_body(&mut lo, d)?;
                    lo.done()?;
                    Some(Box::new(list))
                }
                None => None,
            };
            io.done()?;
            out.push(ListItem { content, list });
        }
        Ok(List {
            ordered,
            items: out,
        })
    }

    fn table(&mut self, o: &mut Obj<'_>, depth: usize) -> Result<Table, Error> {
        let (rows, rp) = o.need("rows")?;
        let mut out = vec![];
        for (i, r) in array(rows, &rp)?.iter().enumerate() {
            let p = rp.at(i);
            self.item(&p)?;
            let mut ro = Obj::new(r, &p)?;
            let (cells, cp) = ro.need("cells")?;
            let mut list = vec![];
            for (k, c) in array(cells, &cp)?.iter().enumerate() {
                list.push(self.cell(c, &cp.at(k), depth)?);
            }
            ro.done()?;
            out.push(Row { cells: list });
        }
        let widths = match o.get("widths") {
            Some((w, wp)) => Some(
                array(w, &wp)?
                    .iter()
                    .enumerate()
                    .map(|(i, x)| match x.as_f64() {
                        Some(n) if n.is_finite() && n > 0.0 => Ok(n),
                        _ => Err(wp
                            .at(i)
                            .err(format!("{x} is not a width; give a number above 0"))),
                    })
                    .collect::<Result<Vec<f64>, Error>>()?,
            ),
            None => None,
        };
        let caption = match o.get("caption") {
            Some((c, cp)) => Some(self.inlines(c, &cp, depth)?),
            None => None,
        };
        Ok(Table {
            rows: out,
            header_rows: o.count("header_rows", 0)?,
            first_column: o.bool("first_column")?,
            style: o.opt_string("style")?,
            widths,
            caption,
            label: o.opt_string("label")?,
        })
    }

    fn cell(&mut self, v: &Value, p: &Path, depth: usize) -> Result<Cell, Error> {
        self.item(p)?;
        let mut o = Obj::new(v, p)?;
        let content = match o.get("content") {
            Some((c, cp)) => {
                let d = self.deeper(depth, &cp)?;
                self.blocks(c, &cp, d)?
            }
            None => vec![],
        };
        let cell = Cell {
            content,
            row_span: o.count("row_span", 1)?,
            column_span: o.count("column_span", 1)?,
            align: o.get("align").map(|(v, p)| align(v, &p)).transpose()?,
            vertical_align: o
                .get("vertical_align")
                .map(|(v, p)| vertical(v, &p))
                .transpose()?,
            shading: o.color("shading")?,
            borders: match o.get("borders") {
                Some((b, bp)) => {
                    let mut bo = Obj::new(b, &bp)?;
                    let borders = CellBorders {
                        top: border_of(&mut bo, "top")?,
                        bottom: border_of(&mut bo, "bottom")?,
                        left: border_of(&mut bo, "left")?,
                        right: border_of(&mut bo, "right")?,
                    };
                    bo.done()?;
                    Some(borders)
                }
                None => None,
            },
        };
        o.done()?;
        Ok(cell)
    }

    fn inlines(&mut self, v: &Value, p: &Path, depth: usize) -> Result<Vec<Inline>, Error> {
        array(v, p)?
            .iter()
            .enumerate()
            .map(|(i, x)| self.inline(x, &p.at(i), depth))
            .collect()
    }

    fn inline(&mut self, v: &Value, p: &Path, depth: usize) -> Result<Inline, Error> {
        self.item(p)?;
        // A bare string is plain text.
        if let Value::String(s) = v {
            return Ok(Inline::text(s));
        }
        let mut o = Obj::new(v, p)?;
        let t = o.string("type")?;
        let i = match t.as_str() {
            "text" => Inline::Text {
                text: o.string("text")?,
                marks: match o.get("marks") {
                    Some((m, mp)) => marks(m, &mp)?,
                    None => Marks::default(),
                },
            },
            "footnote" => {
                let (c, cp) = o.need("content")?;
                let d = self.deeper(depth, &cp)?;
                Inline::Footnote {
                    content: self.blocks(c, &cp, d)?,
                }
            }
            "reference" => Inline::Reference {
                label: o.string("label")?,
            },
            "link" => {
                let (t, tp) = o.need("target")?;
                let target = link_target(t, &tp)?;
                let content = match o.get("content") {
                    Some((c, cp)) => {
                        let d = self.deeper(depth, &cp)?;
                        self.inlines(c, &cp, d)?
                    }
                    None => vec![],
                };
                Inline::Link { target, content }
            }
            "math" => {
                let (m, mp) = o.need("math")?;
                Inline::Math {
                    math: math(m, &mp)?,
                }
            }
            "line_break" => Inline::LineBreak,
            "field" => Inline::Field {
                code: o.string("code")?,
                result: o.opt_string("result")?.unwrap_or_default(),
            },
            "image" => {
                let (m, mp) = o.need("image")?;
                Inline::Image {
                    image: image(m, &mp)?,
                }
            }
            "raw" => Inline::Raw {
                xml: o.string("xml")?,
            },
            other => {
                return Err(p.key("type").err(format!(
                    "{other:?} is not an inline; the inlines are text, footnote, reference, link, math, line_break, field, image and raw"
                )));
            }
        };
        o.done()?;
        Ok(i)
    }
}

fn align(v: &Value, p: &Path) -> Result<Align, Error> {
    match string(v, p)?.as_str() {
        "left" => Ok(Align::Left),
        "center" => Ok(Align::Center),
        "right" => Ok(Align::Right),
        "justify" => Ok(Align::Justify),
        s => Err(p.err(format!(
            "{s:?} is not an alignment; use left, center, right or justify"
        ))),
    }
}

fn vertical(v: &Value, p: &Path) -> Result<VerticalAlign, Error> {
    match string(v, p)?.as_str() {
        "top" => Ok(VerticalAlign::Top),
        "center" => Ok(VerticalAlign::Center),
        "bottom" => Ok(VerticalAlign::Bottom),
        s => Err(p.err(format!(
            "{s:?} is not a vertical alignment; use top, center or bottom"
        ))),
    }
}

fn marks(v: &Value, p: &Path) -> Result<Marks, Error> {
    let mut o = Obj::new(v, p)?;
    let m = Marks {
        bold: o.bool("bold")?,
        italic: o.bool("italic")?,
        underline: o.bool("underline")?,
        strike: o.bool("strike")?,
        superscript: o.bool("superscript")?,
        subscript: o.bool("subscript")?,
        small_caps: o.bool("small_caps")?,
        code: o.bool("code")?,
        color: o.color("color")?,
        highlight: o.color("highlight")?,
        size: o.number("size")?,
    };
    o.done()?;
    Ok(m)
}

fn link_target(v: &Value, p: &Path) -> Result<LinkTarget, Error> {
    let mut o = Obj::new(v, p)?;
    let url = o.opt_string("url")?;
    let bookmark = o.opt_string("bookmark")?;
    let label = o.opt_string("label")?;
    o.done()?;
    match (url, bookmark, label) {
        (Some(u), None, None) => Ok(LinkTarget::Url(u)),
        (None, Some(b), None) => Ok(LinkTarget::Bookmark(b)),
        (None, None, Some(l)) => Ok(LinkTarget::Label(l)),
        _ => Err(p.err("give exactly one of url, bookmark and label")),
    }
}

fn math(v: &Value, p: &Path) -> Result<Math, Error> {
    let mut o = Obj::new(v, p)?;
    let m = Math {
        omml: o.string("omml")?,
    };
    o.done()?;
    Ok(m)
}

fn source_of(o: &mut Obj<'_>) -> Result<Option<Source>, Error> {
    let data = o.get("data");
    let file = o.opt_string("file")?;
    match (data, file) {
        (Some((d, dp)), None) => {
            let s = string(d, &dp)?;
            Ok(Some(Source::Data(
                base64_decode(&s).map_err(|e| dp.err(e))?,
            )))
        }
        (None, Some(f)) => Ok(Some(Source::File(f))),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(o.path.err("give data or file, not both")),
    }
}

fn image(v: &Value, p: &Path) -> Result<Image, Error> {
    let mut o = Obj::new(v, p)?;
    let source =
        source_of(&mut o)?.ok_or_else(|| p.err("give the picture as data (base64) or file"))?;
    let fallback = match o.get("fallback") {
        Some((f, fp)) => {
            let mut fo = Obj::new(f, &fp)?;
            let s = source_of(&mut fo)?.ok_or_else(|| fp.err("give data (base64) or file"))?;
            fo.done()?;
            Some(s)
        }
        None => None,
    };
    let image = Image {
        source,
        fallback,
        width: o.number("width")?,
        height: o.number("height")?,
        alt: o.opt_string("alt")?,
        title: o.opt_string("title")?,
    };
    o.done()?;
    Ok(image)
}

fn border(v: &Value, p: &Path) -> Result<Border, Error> {
    let mut o = Obj::new(v, p)?;
    let width = o
        .number("width")?
        .ok_or_else(|| p.err("width is missing"))?;
    let color = o.color("color")?.ok_or_else(|| p.err("color is missing"))?;
    let style = match o.opt_string("style")?.as_deref() {
        None | Some("single") => LineStyle::Single,
        Some("double") => LineStyle::Double,
        Some("dashed") => LineStyle::Dashed,
        Some("dotted") => LineStyle::Dotted,
        Some("none") => LineStyle::None,
        Some(s) => {
            return Err(p.key("style").err(format!(
                "{s:?} is not a line; use single, double, dashed, dotted or none"
            )));
        }
    };
    o.done()?;
    Ok(Border {
        width,
        color,
        style,
    })
}

fn border_of(o: &mut Obj<'_>, k: &'static str) -> Result<Option<Border>, Error> {
    o.get(k).map(|(v, p)| border(v, &p)).transpose()
}

fn row_look(v: &Value, p: &Path) -> Result<RowLook, Error> {
    let mut o = Obj::new(v, p)?;
    let look = RowLook {
        fill: o.color("fill")?,
        color: o.color("color")?,
        bold: o.bool("bold")?,
        border: border_of(&mut o, "border")?,
    };
    o.done()?;
    Ok(look)
}

fn table_style(v: &Value, p: &Path) -> Result<TableStyleDef, Error> {
    let mut o = Obj::new(v, p)?;
    let borders = match o.get("borders") {
        Some((b, bp)) => {
            let mut bo = Obj::new(b, &bp)?;
            let borders = TableBorders {
                top: border_of(&mut bo, "top")?,
                bottom: border_of(&mut bo, "bottom")?,
                left: border_of(&mut bo, "left")?,
                right: border_of(&mut bo, "right")?,
                inside_h: border_of(&mut bo, "inside_h")?,
                inside_v: border_of(&mut bo, "inside_v")?,
            };
            bo.done()?;
            borders
        }
        None => TableBorders::default(),
    };
    let padding = match o.get("padding") {
        Some((x, xp)) => {
            let mut po = Obj::new(x, &xp)?;
            let mut side =
                |k: &'static str| -> Result<f64, Error> { Ok(po.number(k)?.unwrap_or(0.0)) };
            let pad = Padding {
                top: side("top")?,
                right: side("right")?,
                bottom: side("bottom")?,
                left: side("left")?,
            };
            po.done()?;
            Some(pad)
        }
        None => None,
    };
    let d = TableStyleDef {
        name: o.string("name")?,
        font: o.opt_string("font")?,
        size: o.number("size")?,
        borders,
        padding,
        header: o.get("header").map(|(v, p)| row_look(v, &p)).transpose()?,
        band_fill: o.color("band_fill")?,
        first_column: o
            .get("first_column")
            .map(|(v, p)| row_look(v, &p))
            .transpose()?,
    };
    o.done()?;
    Ok(d)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 with padding.
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Base64, standard or URL-safe alphabet, padding optional, white space
/// ignored.
pub(crate) fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut pad = false;
    for (i, c) in s.bytes().enumerate() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => {
                pad = true;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return Err(format!("not base64: {:?} at character {i}", c as char)),
        };
        if pad {
            return Err(format!(
                "not base64: data after the padding at character {i}"
            ));
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits >= 6 {
        return Err("not base64: it ends in the middle of a byte".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_both_ways() {
        for s in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0, 255, 128, 7, 9]] {
            assert_eq!(base64_decode(&base64_encode(s)).unwrap(), s);
        }
        assert_eq!(base64_encode(b"Wren"), "V3Jlbg==");
        assert!(base64_decode("V3J*").is_err());
        assert!(base64_decode("V").is_err());
    }
}
