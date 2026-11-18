//! A .docx engine that keeps what it does not touch.
//!
//! [`Document`] opens a package, exposes its body as blocks, and edits it in
//! place: new content from the [`content`] model written in the document's
//! own styles, text replaced as tracked changes, table styles defined, field
//! results brought up to date. Saving copies every unchanged part as the same
//! zip entry and every unchanged XML node as the same bytes.

pub mod content;
pub mod document;
pub mod fields;
pub mod package;
pub mod revise;
pub mod styles;
pub mod table_style;
pub mod write;
pub mod xml;

pub use content::{Block, Cell, Figure, FigureKind, Image, Inline, Marks, Math, Table};
pub use document::{BlockInfo, BlockKind, Document, Revision};
pub use package::Package;
pub use styles::{Style, StyleKind, Styles};
pub use table_style::{TableStyle, TableStyleDef};
pub use write::{Place, Roles, WriteOptions};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("{0}: {1}")]
    Xml(String, xml::ParseError),
    #[error("the package has no part {0}")]
    MissingPart(String),
    #[error("pending tracked changes by {} in the target; accept or reject them first", .authors.join(", "))]
    ForeignRevision { authors: Vec<String> },
    #[error("{0}")]
    Invalid(String),
    #[error("not supported: {0}")]
    Unsupported(String),
}
