//! Writes the content model into a document.
//!
//! [`Document::insert`] puts blocks at a [`Place`]: before, after or
//! instead of a block that the views name, or at the end of the body. The
//! place may be in the body, a table cell, a content control, a header or
//! footer, a footnote or endnote, or a text box; a block that cannot stand
//! there (a footnote in a footnote, a page break in a cell) is an error
//! that says why. Nothing is written unless everything can be.
//!
//! The writer uses the document's own styles through [`Roles`], adds what
//! the content needs and the document lacks (a style as Word defines it, a
//! numbering definition, the footnotes part, a media part), and leaves
//! every other part and every other node as it was. Captions are numbered
//! with `SEQ` fields and cross-references are `REF` fields; their results
//! are written as Word would compute them, and later captions of the same
//! sequence are renumbered, so the document reads right without Word.

mod build;
mod fields;
mod ids;
mod media;
mod notes;
mod numbering;
mod place;
mod roles;
mod session;
mod tables;
mod xml;

use std::path::PathBuf;

use crate::content::{Block, Content, ContentLimits, TableStyleDef};
use crate::xml::NodeId;
use crate::{Document, Error};

pub use roles::{Role, RoleSource, Roles};

/// Where content goes.
///
/// The node is a block's [`Block::node`](crate::Block::node) from the
/// views, or a paragraph's [`Paragraph::node`](crate::Paragraph::node) for
/// a place inside a table cell, a content control, a header or footer, a
/// note or a text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Before the block.
    Before(NodeId),
    /// After the block.
    After(NodeId),
    /// Instead of the block.
    Replace(NodeId),
    /// At the end of the body, before its final section properties.
    End,
}

/// The words of captions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captions {
    /// The label word of a figure caption and its `SEQ` identifier.
    pub figure: String,
    /// The label word of a table caption.
    pub table: String,
    /// The label word of a numbered equation, which cross-references
    /// show before its number.
    pub equation: String,
    /// What stands between the number and the caption text.
    pub separator: String,
}

impl Default for Captions {
    fn default() -> Captions {
        Captions {
            figure: "Figure".into(),
            table: "Table".into(),
            equation: "Equation".into(),
            separator: ": ".into(),
        }
    }
}

/// How content is written.
#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    /// Which style plays which part. The default detects Word's built-in
    /// styles in the document.
    pub roles: Roles,
    /// Table styles defined with the content, found by name before the
    /// document's own.
    pub table_styles: Vec<TableStyleDef>,
    /// The label words of captions.
    pub captions: Captions,
    /// The directory that pictures given as files are relative to. Without
    /// it the writer reads no file, and a picture given as a file is an
    /// error.
    pub files: Option<PathBuf>,
    /// Bounds on the content.
    pub limits: ContentLimits,
}

impl WriteOptions {
    /// Options for content read from JSON: its table styles, and pictures
    /// read relative to `files`.
    pub fn for_content(content: &Content, files: Option<PathBuf>) -> WriteOptions {
        WriteOptions {
            table_styles: content.table_styles.clone(),
            files,
            ..WriteOptions::default()
        }
    }
}

/// What an insertion did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inserted {
    /// The elements written, in order, for each place.
    pub nodes: Vec<Vec<NodeId>>,
    /// The roles the content used and where their styles came from:
    /// the caller's choice, Word's built-in style, a fallback, or Word's
    /// definition added to the document.
    pub roles: Vec<(Role, RoleSource)>,
    /// The styles added to the document, by name.
    pub styles_added: Vec<String>,
    /// The table styles the tables use, by id.
    pub table_styles: Vec<String>,
    /// Labels written, with the bookmark that holds each.
    pub labels: Vec<(String, String)>,
    /// Fields outside the insertion whose results changed: later captions
    /// of a sequence and the cross-references to them.
    pub renumbered: usize,
}

impl Document {
    /// Writes blocks at a place. See [`Document::insert_all`].
    pub fn insert(
        &mut self,
        place: Place,
        blocks: &[Block],
        opts: &WriteOptions,
    ) -> Result<Inserted, Error> {
        self.insert_all(&[(place, blocks)], opts)
    }

    /// Writes blocks at several places in one pass.
    ///
    /// The content is checked whole first, then every place; an error
    /// leaves the document as it was. The cost is one reading of the views,
    /// the content written, and one pass over the body for the field
    /// results.
    pub fn insert_all(
        &mut self,
        items: &[(Place, &[Block])],
        opts: &WriteOptions,
    ) -> Result<Inserted, Error> {
        // Check everything before the first change.
        for (i, (_, blocks)) in items.iter().enumerate() {
            let c = Content {
                blocks: blocks.to_vec(),
                table_styles: if i == 0 {
                    opts.table_styles.clone()
                } else {
                    vec![]
                },
            };
            c.check(&opts.limits)?;
        }
        // A failure half-way would leave a part changed: work on a copy.
        let backup = self.snapshot();
        match session::run(self, items, opts) {
            Ok(r) => Ok(r),
            Err(e) => {
                self.restore(backup);
                Err(e)
            }
        }
    }

    /// Adds a table style, or finds one the document has, and returns its
    /// id.
    ///
    /// A definition whose name the document already has is not written;
    /// the document's style is used, so a later run never undoes a change
    /// a Word user made to it. A name alone must name a table style of the
    /// document; otherwise the error lists the table styles it has.
    pub fn ensure_table_style(&mut self, style: &TableStyle) -> Result<String, Error> {
        let backup = self.snapshot();
        match roles::ensure_table_style(self, style) {
            Ok(r) => Ok(r),
            Err(e) => {
                self.restore(backup);
                Err(e)
            }
        }
    }
}

/// A table style to find or add.
#[derive(Debug, Clone, PartialEq)]
pub enum TableStyle {
    /// A style the document has, by id or name.
    Named(String),
    /// A definition, used by name if the document has a style of that
    /// name and added otherwise.
    Defined(Box<TableStyleDef>),
}
