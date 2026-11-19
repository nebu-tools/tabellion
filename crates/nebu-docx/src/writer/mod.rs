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
pub(crate) mod media;
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

/// Who a tracked insertion belongs to.
///
/// `Track::now` stamps it with the current UTC time in the form Word
/// writes (`YYYY-MM-DDTHH:MM:SSZ`). The writer puts every new block's runs
/// inside `w:ins` with this author and date, and marks the paragraph mark
/// the same way, so the views read the insertion back as a revision and
/// `text` with `Revisions::Rejected` leaves it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// The revision author, as Word shows it.
    pub author: String,
    /// The revision date, as written in the file.
    pub date: String,
}

impl Track {
    /// A track record for `author` stamped with the current time.
    pub fn now(author: &str) -> Track {
        Track {
            author: author.to_string(),
            date: utc_now(),
        }
    }
}

fn utc_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days to civil date (Howard Hinnant's algorithm).
    let days = (secs / 86_400) as i64;
    let time = (secs % 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as i64;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as i64;
    y += if m <= 2 { 1 } else { 0 };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        time / 3_600,
        (time % 3_600) / 60,
        time % 60
    )
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
    /// When present, new blocks are written as a tracked insertion by this
    /// author: runs inside `w:ins`, with the paragraph mark flagged the
    /// same way. A replacement still removes the old block and writes the
    /// new one as an insertion; keeping the old text as a tracked deletion
    /// comes with the editing calls.
    pub track: Option<Track>,
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

    /// Recomputes every `SEQ` and bare `REF` result in the body and
    /// returns how many distinct fields changed. Fields Word alone can
    /// compute (page numbers, dates, external includes) and `REF`s with
    /// switches keep their results. `SEQ` sequences that restart at a
    /// heading level start again after each heading of that level.
    pub fn update_fields(&mut self) -> Result<usize, Error> {
        let backup = self.snapshot();
        match fields::update_all(self) {
            // No counter advance: refreshing field results allocates no
            // document ids.
            Ok(n) => Ok(n),
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
