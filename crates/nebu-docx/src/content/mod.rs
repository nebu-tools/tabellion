//! The content model: what content is, before it is written into a
//! document.
//!
//! Every authoring path (JSON, and later Typst, Markdown and R) produces
//! [`Content`]: blocks ([`Block`]) that hold inlines ([`Inline`]), and the
//! table styles the blocks name. The model says what the content is, not
//! how Word stores it: a heading has a level, a table has rows of cells
//! with spans, a caption has a label. [`Document::insert`] writes it in the
//! target document's own styles.
//!
//! The model has a JSON form. [`Content::from_json`] reads it as untrusted
//! input, within [`ContentLimits`], and an error names the JSON path and
//! what was wrong there. Serialising a [`Content`] gives the same form, and
//! `crates/nebu-docx/schema/content.json` describes it as a JSON Schema.
//!
//! [`Document::insert`]: crate::Document::insert

mod check;
mod read;

use serde::Serialize;

pub use read::ContentLimits;

/// Content to insert: blocks, and the table styles they name.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Content {
    /// The blocks, in order.
    pub blocks: Vec<Block>,
    /// Table styles defined with the content. A table that names one of
    /// these by name gets it; see [`TableStyleDef`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub table_styles: Vec<TableStyleDef>,
}

impl Content {
    /// Reads content from its JSON form, within the default limits.
    pub fn from_json(json: &[u8]) -> Result<Content, crate::Error> {
        Content::from_json_with_limits(json, &ContentLimits::default())
    }

    /// Reads content from its JSON form within the given limits. An error
    /// is [`Error::Invalid`](crate::Error::Invalid) and names the JSON path,
    /// such as `blocks[2].rows[0].cells[1]`, and what is wrong there.
    pub fn from_json_with_limits(
        json: &[u8],
        limits: &ContentLimits,
    ) -> Result<Content, crate::Error> {
        read::read(json, limits)
    }

    /// Checks what the JSON reader checks, for content built in code: the
    /// limits, table grids, colours, labels, math and raw OOXML.
    pub fn check(&self, limits: &ContentLimits) -> Result<(), crate::Error> {
        check::content(self, limits)
    }

    /// The JSON form.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// A block of content.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    /// A paragraph in the body style or a named paragraph style.
    Paragraph {
        /// A paragraph style by id or name. Without one, the body role.
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        /// The alignment, over the style's.
        #[serde(skip_serializing_if = "Option::is_none")]
        align: Option<Align>,
        /// What it says.
        content: Vec<Inline>,
    },
    /// A heading.
    Heading {
        /// 1 to 9.
        level: u8,
        /// What it says.
        content: Vec<Inline>,
        /// A label that cross-references name; a hidden bookmark
        /// `_Ref_<label>` marks the heading's text.
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A bulleted or numbered list.
    List(List),
    /// A table.
    Table(Table),
    /// A picture with a caption.
    Figure {
        /// The picture.
        image: Image,
        /// The caption text after the label and number.
        caption: Vec<Inline>,
        /// A label that cross-references name.
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A picture in a paragraph of its own, without a caption.
    Image {
        /// The picture.
        image: Image,
    },
    /// A display equation; with a label it is numbered `(n)` at the right
    /// margin.
    Equation {
        /// The equation.
        math: Math,
        /// A label that cross-references name.
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The next block starts a new page.
    PageBreak,
    /// WordprocessingML written as given: one or more block-level elements
    /// such as `w:p`, `w:tbl` or `w:sdt`.
    Raw {
        /// The XML. The prefixes `w`, `r`, `m`, `wp`, `a`, `pic`, `w14`,
        /// `wps` and `mc` are bound; the XML may bind others itself.
        xml: String,
    },
}

/// A list: items, each with its text and an optional list inside it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct List {
    /// Numbered (true) or bulleted (false).
    pub ordered: bool,
    /// The items.
    pub items: Vec<ListItem>,
}

/// An item of a list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListItem {
    /// What it says.
    pub content: Vec<Inline>,
    /// A list one level deeper, after the item's text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<Box<List>>,
}

/// Horizontal alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

/// Vertical alignment in a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum VerticalAlign {
    Top,
    Center,
    Bottom,
}

/// A table: rows of cells on a grid.
///
/// Each row lists the cells that begin in it, from left to right; a cell
/// takes the first grid position that no cell above covers. Every row must
/// cover the same number of grid columns, and no two cells may cover one
/// position.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Table {
    /// The rows.
    pub rows: Vec<Row>,
    /// How many rows at the top are header rows, repeated on each page.
    #[serde(skip_serializing_if = "is_zero")]
    pub header_rows: usize,
    /// The first column has the style's first-column look.
    #[serde(skip_serializing_if = "is_false")]
    pub first_column: bool,
    /// A table style by name: one defined with the content, else one the
    /// document has, by id or name. Without one, the default table style
    /// role, then Table Grid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Relative column widths, one per grid column. Without them, equal
    /// columns. The table spans the text width either way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widths: Option<Vec<f64>>,
    /// The caption text after the label and number, written above the
    /// table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<Vec<Inline>>,
    /// A label that cross-references name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A row of a table.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// The cells that begin in this row.
    pub cells: Vec<Cell>,
}

/// A cell of a table. Its shading, alignment and borders are direct
/// formatting over the table style.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cell {
    /// What it holds: paragraphs, lists, tables, pictures, equations.
    pub content: Vec<Block>,
    /// The rows it covers, from 1.
    #[serde(skip_serializing_if = "is_one")]
    pub row_span: usize,
    /// The grid columns it covers, from 1.
    #[serde(skip_serializing_if = "is_one")]
    pub column_span: usize,
    /// Horizontal alignment of its paragraphs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// Vertical alignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<VerticalAlign>,
    /// The fill colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<Color>,
    /// Borders of this cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<CellBorders>,
}

impl Cell {
    /// A cell of one grid position holding these blocks.
    pub fn new(content: Vec<Block>) -> Cell {
        Cell {
            content,
            row_span: 1,
            column_span: 1,
            align: None,
            vertical_align: None,
            shading: None,
            borders: None,
        }
    }
}

/// The borders of a cell.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[allow(missing_docs)]
pub struct CellBorders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Border>,
}

/// A border line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Border {
    /// The width in points, 0.25 to 6. Word draws widths in eighths of a
    /// point.
    pub width: f64,
    /// The colour.
    pub color: Color,
    /// The line.
    #[serde(skip_serializing_if = "LineStyle::is_single")]
    pub style: LineStyle,
}

/// How a border is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum LineStyle {
    #[default]
    Single,
    Double,
    Dashed,
    Dotted,
    /// No line, to take away a line the table style draws.
    None,
}

impl LineStyle {
    fn is_single(&self) -> bool {
        *self == LineStyle::Single
    }
}

/// A colour as six hexadecimal digits, `RRGGBB`. The JSON form also takes
/// a leading `#`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Color(String);

impl Color {
    /// A colour from `RRGGBB` or `#RRGGBB`, in either case.
    pub fn new(s: &str) -> Result<Color, crate::Error> {
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Color(hex.to_ascii_uppercase()))
        } else {
            Err(crate::Error::Invalid(format!(
                "{s:?} is not a colour; write six hexadecimal digits such as \"385723\""
            )))
        }
    }

    /// The six digits, in capitals.
    pub fn hex(&self) -> &str {
        &self.0
    }
}

/// A picture: PNG, JPEG, GIF or SVG.
///
/// The size comes from the picture's pixel size and resolution, limited to
/// the text width of the section it lands in, unless `width` or `height`
/// say otherwise; one of them alone keeps the proportions. An SVG needs a
/// PNG `fallback`, which Word shows where it cannot draw SVG and which
/// gives the size if the SVG states none.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Image {
    /// Where the bytes come from.
    #[serde(flatten)]
    pub source: Source,
    /// The PNG Word shows in place of an SVG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Source>,
    /// The width in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// The height in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// The alternative text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    /// The title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Image {
    /// A picture from its bytes, sized from them.
    pub fn new(bytes: Vec<u8>) -> Image {
        Image {
            source: Source::Data(bytes),
            fallback: None,
            width: None,
            height: None,
            alt: None,
            title: None,
        }
    }
}

/// Where the bytes of a picture come from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The bytes themselves; base64 in the JSON form.
    #[serde(serialize_with = "base64_out")]
    Data(Vec<u8>),
    /// A file, read only when [`WriteOptions::files`] names the directory
    /// it is relative to. The engine reads no file otherwise.
    ///
    /// [`WriteOptions::files`]: crate::WriteOptions::files
    File(String),
}

fn base64_out<S: serde::Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&read::base64_encode(bytes))
}

/// An equation.
///
/// For now an equation is Office Math XML: one `m:oMath` element. The
/// Typst mapping will produce the same type from Typst math, through
/// another constructor, so nothing that takes a [`Math`] changes then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Math {
    /// One `m:oMath` element, with the `m` prefix bound.
    pub omml: String,
}

impl Math {
    /// An equation from Office Math XML: one `m:oMath` element. The XML is
    /// checked when the content is checked or written.
    pub fn omml(xml: &str) -> Math {
        Math {
            omml: xml.to_string(),
        }
    }
}

/// Inline content.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inline {
    /// Text with marks. A tab is `\t`, a line break `\n`.
    Text {
        /// The text.
        text: String,
        /// Its formatting.
        #[serde(skip_serializing_if = "Marks::is_empty")]
        marks: Marks,
    },
    /// A footnote, with its own blocks.
    Footnote {
        /// What the note says: paragraphs, lists, tables.
        content: Vec<Block>,
    },
    /// A cross-reference to a labelled caption, heading or equation. It
    /// shows the target's label word and number ("Table 3"), the heading's
    /// text, or the equation's "(n)".
    Reference {
        /// The label.
        label: String,
    },
    /// A hyperlink.
    Link {
        /// Where it leads.
        target: LinkTarget,
        /// What it shows.
        content: Vec<Inline>,
    },
    /// An equation in the line of text.
    Math {
        /// The equation.
        math: Math,
    },
    /// A line break.
    LineBreak,
    /// A field written as given, with a result to show until Word updates
    /// it.
    Field {
        /// The code, such as `DATE \@ "d MMMM yyyy"`.
        code: String,
        /// The result.
        #[serde(skip_serializing_if = "String::is_empty")]
        result: String,
    },
    /// A picture in the line of text.
    Image {
        /// The picture.
        image: Image,
    },
    /// WordprocessingML written as given: run-level elements such as
    /// `w:r`, `w:hyperlink`, `w:fldSimple`, `w:sdt` or `m:oMath`.
    Raw {
        /// The XML, with the prefixes of [`Block::Raw`] bound.
        xml: String,
    },
}

impl Inline {
    /// Plain text.
    pub fn text(s: &str) -> Inline {
        Inline::Text {
            text: s.to_string(),
            marks: Marks::default(),
        }
    }
}

/// Where a link leads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkTarget {
    /// An address outside the document.
    Url(String),
    /// A bookmark in the document, by name.
    Bookmark(String),
    /// A label of the content or the document: the bookmark `_Ref_<label>`.
    Label(String),
}

/// The formatting of text.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Marks {
    #[serde(skip_serializing_if = "is_false")]
    #[allow(missing_docs)]
    pub bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    #[allow(missing_docs)]
    pub italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    #[allow(missing_docs)]
    pub underline: bool,
    #[serde(skip_serializing_if = "is_false")]
    #[allow(missing_docs)]
    pub strike: bool,
    #[serde(skip_serializing_if = "is_false")]
    #[allow(missing_docs)]
    pub superscript: bool,
    #[serde(skip_serializing_if = "is_false")]
    #[allow(missing_docs)]
    pub subscript: bool,
    #[serde(skip_serializing_if = "is_false")]
    #[allow(missing_docs)]
    pub small_caps: bool,
    /// Code: the code role's character style.
    #[serde(skip_serializing_if = "is_false")]
    pub code: bool,
    /// The text colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// A fill behind the text, any colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Color>,
    /// The size in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
}

impl Marks {
    /// True if the text has no formatting of its own.
    pub fn is_empty(&self) -> bool {
        *self == Marks::default()
    }
}

/// A table style defined with the content.
///
/// The engine writes it as a real table style (`w:style w:type="table"`),
/// which Word lists in its table design gallery. A document that already
/// has a style of this name keeps its own, which is used and not
/// overwritten.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct TableStyleDef {
    /// The name Word shows.
    pub name: String,
    /// The font of the table's text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The size of the table's text in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// The borders of the table.
    #[serde(skip_serializing_if = "TableBorders::is_empty")]
    pub borders: TableBorders,
    /// The space between a cell's border and its text, in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    /// The look of the header rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<RowLook>,
    /// The fill of every other body row, starting with the first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band_fill: Option<Color>,
    /// The look of the first column, for tables that ask for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_column: Option<RowLook>,
}

/// The outer and inner borders of a table.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[allow(missing_docs)]
pub struct TableBorders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Border>,
    /// Between rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside_h: Option<Border>,
    /// Between columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside_v: Option<Border>,
}

impl TableBorders {
    fn is_empty(&self) -> bool {
        *self == TableBorders::default()
    }
}

/// Cell padding in points.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(missing_docs)]
pub struct Padding {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

/// The look of a header row or the first column.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct RowLook {
    /// The fill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Color>,
    /// The text colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Bold text.
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    /// A line below the header rows, or right of the first column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn is_one(n: &usize) -> bool {
    *n == 1
}

pub(crate) use check::{RAW_PREFIXES, check_label, grid};
