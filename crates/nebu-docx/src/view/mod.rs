//! Read-only views of a document: blocks, paragraphs, tables, fields,
//! revisions, comments, notes, bookmarks, images, styles, numbering and
//! sections.
//!
//! [`Document::views`] reads every view in one walk over the stories
//! ([`walk`]) and keeps them until an edit changes a tree. Each item names
//! where it is (a block id, a part) and gives what a reader needs (text,
//! cells, codes, authors). The walk records the text of every story as
//! pieces ([`text`]), and every text a view gives is a range of them
//! rendered by one function, so visible text has one definition: field
//! results and not field codes, no hidden text, and revisions as the
//! [`Revisions`] option says.
//!
//! Every view type serialises to the JSON shape that
//! `crates/nebu-docx/schema/views.json` describes.

mod collect;
mod locate;
mod names;
mod numbering;
mod props;
mod reader;
mod styles;
mod text;
mod walk;

#[cfg(test)]
mod linear;

use serde::Serialize;

use crate::xml::NodeId;
use crate::{Document, Error};

pub use locate::{BlockId, Locator};
pub use numbering::{
    LevelOverride, Numbering, NumberingDefinition, NumberingInstance, NumberingLevel,
};
pub use reader::Views;
pub use styles::{LocalName, Style, StyleKind, Styles, Toggle, Toggles};

/// Which text tracked changes leave in the document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Revisions {
    /// As if every change were accepted: inserted text in, deleted text out.
    #[default]
    Accepted,
    /// As if every change were rejected: inserted text out, deleted text in.
    Rejected,
    /// Everything the file holds: inserted and deleted text both.
    Current,
}

impl std::str::FromStr for Revisions {
    type Err = Error;
    fn from_str(s: &str) -> Result<Revisions, Error> {
        match s {
            "accepted" => Ok(Revisions::Accepted),
            "rejected" => Ok(Revisions::Rejected),
            "current" => Ok(Revisions::Current),
            _ => Err(Error::Invalid(format!(
                "{s:?} is not a view of revisions; use accepted, rejected or current"
            ))),
        }
    }
}

/// What a tracked change does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionKind {
    /// Content was inserted.
    Insertion,
    /// Content was deleted.
    Deletion,
    /// Content was moved away from here.
    MoveFrom,
    /// Content was moved here.
    MoveTo,
    /// Properties changed: formatting of text, a paragraph, a table, a row,
    /// a cell or a section.
    Formatting,
}

/// What a tracked change applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionTarget {
    /// Text and inline content.
    Text,
    /// The mark at the end of a paragraph.
    ParagraphMark,
    /// A table row.
    Row,
    /// A table cell.
    Cell,
    /// Run properties.
    Run,
    /// Paragraph properties.
    Paragraph,
    /// Table properties.
    Table,
    /// Section properties.
    Section,
}

/// The kind of story text belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryKind {
    /// The body of the document.
    Main,
    /// A header.
    Header,
    /// A footer.
    Footer,
    /// A footnote.
    Footnote,
    /// An endnote.
    Endnote,
    /// A comment.
    Comment,
    /// A text box, in whatever story holds it.
    TextBox,
}

/// Where an item is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    /// The kind of story.
    pub story: StoryKind,
    /// The part that holds it.
    pub part: String,
    /// The id of the top-level block of the body that holds it, for the
    /// main story and its text boxes.
    pub block: Option<BlockId>,
}

/// What a top-level block of the body is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    /// A paragraph.
    Paragraph,
    /// A table.
    Table,
    /// A block-level content control.
    ContentControl,
    /// The section properties at the end of the body.
    Section,
    /// The start or the end of a bookmark between blocks.
    Bookmark,
    /// Anything else, such as custom XML or an element no view knows.
    Other,
}

/// A list a paragraph or style belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NumberingRef {
    /// The numbering instance (`w:numId`).
    pub id: String,
    /// The level, from 0.
    pub level: u8,
}

/// A top-level block of the body.
#[derive(Debug, Clone, Serialize)]
pub struct Block {
    /// `p:<paraId>` for a paragraph with a `w14:paraId`, `t:<paraId>` for a
    /// table whose first paragraph has one, `#<index>` otherwise. A
    /// `#<index>` id holds only for the file it was read from.
    pub id: BlockId,
    /// True for a paragraph or table whose `w14:paraId` another block of the
    /// body shares, so that it has a `#<index>` id instead.
    pub shared_para_id: bool,
    /// The position among the blocks, from 0.
    pub index: usize,
    /// What the block is.
    pub kind: BlockKind,
    /// The style id of a paragraph or table.
    pub style: Option<String>,
    /// The English name Word shows for that style.
    pub style_name: Option<String>,
    /// The outline level of a paragraph, 1 to 9: a heading level.
    pub level: Option<u8>,
    /// The list a paragraph belongs to.
    pub numbering: Option<NumberingRef>,
    /// The text a reader sees, with revisions accepted. A table gives its
    /// rows as lines of tab-separated cells; paragraphs in a cell are
    /// joined by spaces.
    pub text: String,
    /// The element's name as written, for kinds `bookmark` and `other`.
    pub element: Option<String>,
    /// What a block-level content control is.
    pub control: Option<ContentControl>,
    /// A paragraph's runs and the inline objects between them.
    pub content: Vec<Inline>,
    /// The block's element in the main part's tree.
    #[serde(skip)]
    pub node: NodeId,
}

/// Direct formatting of a run. Unset properties come from styles.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Format {
    /// The character style id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub italic: Option<bool>,
    /// The underline style, such as `single`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub double_strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub superscript: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub subscript: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub small_caps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub all_caps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub hidden: Option<bool>,
    /// The font for Latin text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The size in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// The colour as six hex digits, or `auto`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The highlight colour name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
}

/// A tracked change that text is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// Insertion, deletion, or one side of a move.
    pub kind: RevisionKind,
    /// Who made it.
    pub author: String,
    /// When, as written in the file.
    pub date: Option<String>,
}

/// What a content control holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentControlKind {
    #[allow(missing_docs)]
    RichText,
    #[allow(missing_docs)]
    PlainText,
    #[allow(missing_docs)]
    Picture,
    #[allow(missing_docs)]
    Date,
    #[allow(missing_docs)]
    ComboBox,
    #[allow(missing_docs)]
    DropDownList,
    #[allow(missing_docs)]
    CheckBox,
    /// A building block gallery or a document part.
    BuildingBlock,
    #[allow(missing_docs)]
    Group,
    #[allow(missing_docs)]
    RepeatingSection,
    #[allow(missing_docs)]
    RepeatingSectionItem,
    #[allow(missing_docs)]
    Citation,
    #[allow(missing_docs)]
    Bibliography,
    #[allow(missing_docs)]
    Equation,
}

/// A content control's kind, tag and title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentControl {
    /// What it holds.
    pub kind: ContentControlKind,
    /// Its tag.
    pub tag: Option<String>,
    /// Its title.
    pub title: Option<String>,
    /// True while it shows its placeholder text.
    pub placeholder: bool,
}

/// A content control with the text it holds.
#[derive(Debug, Clone, Serialize)]
pub struct Control {
    /// Its kind, tag and title.
    #[serde(flatten)]
    pub control: ContentControl,
    /// Its text, with revisions accepted; paragraphs are separated by
    /// `\n`.
    pub text: String,
    /// Where it begins.
    pub location: Location,
}

/// What a drawing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingKind {
    #[allow(missing_docs)]
    Picture,
    #[allow(missing_docs)]
    Chart,
    /// SmartArt.
    Diagram,
    #[allow(missing_docs)]
    Shape,
    #[allow(missing_docs)]
    TextBox,
    #[allow(missing_docs)]
    Group,
    #[allow(missing_docs)]
    Canvas,
    #[allow(missing_docs)]
    Ink,
    /// An embedded or linked object (OLE).
    Object,
    /// A drawing of a kind the views do not know.
    Other,
}

/// How a drawing sits in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// In the line of text, like a character.
    Inline,
    /// Anchored to a paragraph and placed on the page.
    Floating,
}

/// A footnote or an endnote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    #[allow(missing_docs)]
    Footnote,
    #[allow(missing_docs)]
    Endnote,
}

/// The content of a paragraph: runs of text and the objects between them.
/// Containers (fields, hyperlinks, content controls, bookmarks, comment
/// ranges) come as a start and an end around their content.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inline {
    /// Text of one run. Field codes are not text; deleted and hidden text
    /// is, marked by its revision and its toggles.
    Text {
        /// The text, with tabs and line breaks as `\t` and `\n`.
        text: String,
        /// The run's direct formatting.
        format: Format,
        /// The toggle properties that are on after the style cascade:
        /// `hidden` for text a reader does not see, `bold`, `italic` and
        /// the others.
        toggles: Toggles,
        /// The tracked change the text is in.
        revision: Option<Change>,
    },
    /// The result of a field begins. The field is
    /// `Document::fields()[field]`.
    FieldStart {
        /// The index in the list of fields.
        field: usize,
        /// The field's code.
        code: String,
    },
    /// The result of a field ends.
    FieldEnd {
        /// The index in the list of fields.
        field: usize,
    },
    /// A hyperlink begins.
    HyperlinkStart {
        /// The address, for a link out of the document.
        target: Option<String>,
        /// The bookmark, for a link into the document.
        anchor: Option<String>,
    },
    /// A hyperlink ends.
    HyperlinkEnd,
    /// A content control begins.
    ContentControlStart {
        /// What it holds.
        kind: ContentControlKind,
        /// Its tag.
        tag: Option<String>,
        /// Its title.
        title: Option<String>,
        /// True while it shows its placeholder text.
        placeholder: bool,
    },
    /// A content control ends.
    ContentControlEnd,
    /// A bookmark begins.
    BookmarkStart {
        /// Its name.
        name: String,
    },
    /// A bookmark ends.
    BookmarkEnd {
        /// Its name, if its start was seen.
        name: Option<String>,
    },
    /// The text a comment is about begins.
    CommentStart {
        /// The comment's id.
        comment: String,
    },
    /// The text a comment is about ends.
    CommentEnd {
        /// The comment's id.
        comment: String,
    },
    /// The mark that shows a comment.
    CommentReference {
        /// The comment's id.
        comment: String,
    },
    /// The mark that calls a footnote or an endnote.
    NoteReference {
        /// Footnote or endnote.
        note: NoteKind,
        /// The note's id.
        id: String,
        /// The number Word shows, for an automatically numbered note.
        number: Option<usize>,
    },
    /// A picture, chart, shape or text box.
    Drawing {
        /// What it is.
        kind: DrawingKind,
        /// Inline or floating.
        placement: Placement,
        /// For a picture, its index in `Document::images()`.
        image: Option<usize>,
        /// Its alternative text.
        alt: Option<String>,
        /// The text of the text boxes it holds, with revisions accepted.
        text: Option<String>,
    },
    /// An equation.
    Math {
        /// Its text, symbol by symbol.
        text: String,
    },
    /// A page or column break.
    Break {
        /// `page` or `column`.
        kind: String,
    },
    /// An element the views do not know, kept in place.
    Opaque {
        /// Its name as written.
        element: String,
    },
}

/// A table on its grid.
#[derive(Debug, Clone, Serialize)]
pub struct Table {
    /// The position in `Document::tables()`, from 0.
    pub index: usize,
    /// The block id, for a top-level table of the body.
    pub id: Option<BlockId>,
    /// Where the table is.
    pub location: Location,
    /// For a table inside a cell: that table's index, and the cell's row
    /// and column.
    pub parent: Option<CellRef>,
    /// The table style id.
    pub style: Option<String>,
    /// The number of rows.
    pub row_count: usize,
    /// The number of grid columns.
    pub column_count: usize,
    /// The cells, each once, in reading order.
    pub cells: Vec<Cell>,
    /// The text of each grid position, row by row: a cell's text at its
    /// top left position and `null` where a merged cell covers the grid.
    pub rows: Vec<Vec<Option<String>>>,
}

/// A position in a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CellRef {
    /// The table's index.
    pub table: usize,
    /// The row, from 0.
    pub row: usize,
    /// The grid column, from 0.
    pub column: usize,
}

/// A cell of a table.
#[derive(Debug, Clone, Serialize)]
pub struct Cell {
    /// The row it starts in, from 0.
    pub row: usize,
    /// The grid column it starts in, from 0.
    pub column: usize,
    /// The rows it covers.
    pub row_span: usize,
    /// The grid columns it covers.
    pub column_span: usize,
    /// Its text, paragraphs separated by `\n`, without the tables nested
    /// in it.
    pub text: String,
    /// The indexes of the tables nested in it.
    pub tables: Vec<usize>,
}

/// A field.
#[derive(Debug, Clone, Serialize)]
pub struct Field {
    /// The position in `Document::fields()`, from 0.
    pub index: usize,
    /// The first word of the code in capitals: `REF`, `SEQ`, `TOC`.
    pub kind: String,
    /// The code. A field nested in the code appears as `{ code }`.
    pub code: String,
    /// The result as the file stores it, with revisions accepted.
    pub result: String,
    /// True for a simple field (`w:fldSimple`).
    pub simple: bool,
    /// The field this one is nested in.
    pub parent: Option<usize>,
    /// Where the field begins.
    pub location: Location,
}

/// A tracked change.
#[derive(Debug, Clone, Serialize)]
pub struct Revision {
    /// The position in `Document::revisions()`, from 0.
    pub index: usize,
    /// What it does.
    pub kind: RevisionKind,
    /// What it applies to.
    pub target: RevisionTarget,
    /// Who made it.
    pub author: String,
    /// When, as written in the file.
    pub date: Option<String>,
    /// The text it affects, deleted text included. A paragraph mark is
    /// `\n`.
    pub text: String,
    /// Where it is.
    pub location: Location,
}

/// A comment.
#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    /// The id the document refers to it by.
    pub id: String,
    /// Who wrote it.
    pub author: String,
    /// The author's initials.
    pub initials: Option<String>,
    /// When, as written in the file.
    pub date: Option<String>,
    /// What it says.
    pub text: String,
    /// The text it is about, with revisions accepted.
    pub anchor: Option<String>,
    /// The id of the comment it answers.
    pub parent: Option<String>,
    /// True if it is marked as resolved.
    pub done: bool,
    /// Where its mark is.
    pub location: Option<Location>,
}

/// A footnote or endnote.
#[derive(Debug, Clone, Serialize)]
pub struct Note {
    /// Footnote or endnote.
    pub kind: NoteKind,
    /// The id the reference uses.
    pub id: String,
    /// The number Word shows, counted by references in the body. `None`
    /// for a note with its own mark or without a reference.
    pub number: Option<usize>,
    /// Its text.
    pub text: String,
    /// Where its reference is.
    pub location: Option<Location>,
}

/// A bookmark.
#[derive(Debug, Clone, Serialize)]
pub struct Bookmark {
    /// Its name.
    pub name: String,
    /// The id that pairs its start and end.
    pub id: String,
    /// True for a name that starts with `_`, which Word hides.
    pub hidden: bool,
    /// The text it encloses, with revisions accepted. Paragraphs are
    /// separated by `\n` and table cells by tabs.
    pub text: String,
    /// Where it starts.
    pub location: Location,
}

/// A picture.
#[derive(Debug, Clone, Serialize)]
pub struct Image {
    /// The position in `Document::images()`, from 0.
    pub index: usize,
    /// Inline or floating.
    pub placement: Placement,
    /// The relationship id that names the picture.
    pub relationship: Option<String>,
    /// The part that holds the picture, for an embedded picture.
    pub part: Option<String>,
    /// The address, for a linked picture.
    pub link: Option<String>,
    /// The content type of the part.
    pub content_type: Option<String>,
    /// The width in points, as placed.
    pub width: Option<f64>,
    /// The height in points, as placed.
    pub height: Option<f64>,
    /// The drawing's name.
    pub name: Option<String>,
    /// Its title.
    pub title: Option<String>,
    /// Its alternative text.
    pub alt: Option<String>,
    /// Where it is.
    pub location: Location,
}

/// A header or footer of a section.
#[derive(Debug, Clone, Serialize)]
pub struct HeaderFooter {
    /// `default`, `first` or `even`.
    pub kind: String,
    /// The part.
    pub part: String,
    /// True if the section takes it from an earlier section.
    pub inherited: bool,
    /// Its text.
    pub text: String,
}

/// Page margins in points.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[allow(missing_docs)]
pub struct Margins {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
    pub header: f64,
    pub footer: f64,
    pub gutter: f64,
}

/// A section of the document.
#[derive(Debug, Clone, Serialize)]
pub struct Section {
    /// The position, from 0.
    pub index: usize,
    /// The id of its first block.
    pub first_block: Option<BlockId>,
    /// The id of its last block.
    pub last_block: Option<BlockId>,
    /// How it begins: `next_page`, `continuous`, `even_page`, `odd_page` or
    /// `next_column`.
    pub start: String,
    /// The page width in points.
    pub page_width: Option<f64>,
    /// The page height in points.
    pub page_height: Option<f64>,
    /// `portrait` or `landscape`.
    pub orientation: String,
    /// The margins.
    pub margins: Margins,
    /// The number of text columns.
    pub columns: u32,
    /// The width between the margins, in points.
    pub text_width: Option<f64>,
    /// True if the first page has its own header and footer.
    pub title_page: bool,
    /// Its headers.
    pub headers: Vec<HeaderFooter>,
    /// Its footers.
    pub footers: Vec<HeaderFooter>,
}

/// What [`Document::text`] includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOptions {
    /// Which text tracked changes leave.
    pub revisions: Revisions,
    /// Each section's headers before its text and its footers after.
    pub headers_footers: bool,
    /// Note marks in the text as `[1]`, and the notes at the end.
    pub notes: bool,
    /// The comments at the end.
    pub comments: bool,
    /// The text of text boxes after the paragraph they belong to.
    pub text_boxes: bool,
}

impl Default for TextOptions {
    fn default() -> TextOptions {
        TextOptions {
            revisions: Revisions::Accepted,
            headers_footers: false,
            notes: false,
            comments: false,
            text_boxes: true,
        }
    }
}

/// The views of a document as read, with the stamp of the trees they were
/// read from.
pub(crate) struct Cache {
    stamp: Vec<(u64, u64)>,
    views: Views,
}

impl Document {
    /// Every view of the document.
    ///
    /// The first call reads them all in one walk over every story (the
    /// body, headers and footers, notes, comments and text boxes), which
    /// costs time in proportion to the size of those parts. Later calls
    /// return the same views at no cost until an edit changes a tree or a
    /// part is added; then the next call reads them again.
    pub fn views(&mut self) -> Result<&Views, Error> {
        // The parts the views need were parsed when they were read, so an
        // equal stamp means nothing changed since.
        let fresh = self.views.as_ref().is_some_and(|c| c.stamp == self.stamp());
        if !fresh {
            let views = reader::Reader::new(self)?.read()?;
            let stamp = self.stamp();
            self.views = Some(Box::new(Cache { stamp, views }));
        }
        match &self.views {
            Some(c) => Ok(&c.views),
            None => Err(Error::Invalid("the views could not be read".into())),
        }
    }
}
