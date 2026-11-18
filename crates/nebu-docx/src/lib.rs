//! Open, edit and save .docx files without changing what an edit does not
//! touch.
//!
//! A [`Document`] is a Word file held in memory. A caller opens it, reaches
//! the XML tree of a part, changes nodes, and saves:
//!
//! ```no_run
//! use nebu_docx::{Document, ns};
//!
//! let mut doc = Document::open("report.docx")?;
//! let main = doc.main_part().to_string();
//! let tree = doc.xml(&main)?;
//! let body = tree.child(tree.root_element(), Some(ns::W), "body").expect("a body");
//! let p = tree.parse_fragment(r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p>"#, &[("w", ns::W)])?;
//! tree.insert(body, 0, p[0])?;
//! doc.save("out.docx")?;
//! # Ok::<(), nebu_docx::Error>(())
//! ```
//!
//! Saving keeps what did not change. A part nobody changed is copied into
//! the new file as the same compressed zip entry. In a part that changed,
//! every node outside the change is copied as the bytes it was read from
//! (see [`xml`]).
//!
//! Files from strangers are safe to open: [`Limits`] bounds the work and the
//! memory a file can ask for, and no input makes the engine panic.

mod document;
mod package;
pub mod xml;

pub use document::{Document, Relationship, Target};

/// Namespaces that callers of this crate meet.
pub mod ns {
    /// WordprocessingML, the main namespace of a Word document.
    pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    /// Package relationships (`.rels` parts).
    pub const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    /// The content types part (`[Content_Types].xml`).
    pub const CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
}

/// Bounds on what opening a file may cost. The defaults suit documents far
/// larger than people write; a caller that trusts its files can raise them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Entries in the zip archive.
    pub max_entries: usize,
    /// Decompressed size of one part, in bytes.
    pub max_part_size: u64,
    /// Decompressed size of all parts read, in bytes.
    pub max_total_size: u64,
    /// Nesting depth of elements in an XML part.
    pub max_depth: usize,
    /// Nodes in one XML part: elements, text, comments and the like. Bounds
    /// the memory a part of many tiny elements can take.
    pub max_xml_nodes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_entries: 10_000,
            max_part_size: 256 << 20,
            max_total_size: 1 << 30,
            max_depth: 256,
            max_xml_nodes: 10_000_000,
        }
    }
}

/// Everything that can go wrong. Each message says what was wrong with the
/// file or the call, so it can be shown to the person who supplied them.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A file could not be read or written.
    #[error("cannot {action} {path}: {source}")]
    Io {
        /// "read" or "write".
        action: &'static str,
        /// The file.
        path: std::path::PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The bytes are not a zip archive, or the archive is damaged.
    #[error("not a readable .docx file: {0}")]
    Zip(String),
    /// The archive is a zip file but not a Word document.
    #[error("not a Word document: {0}")]
    NotWord(String),
    /// A part that the call or a relationship names does not exist.
    #[error("the document has no part {0}")]
    MissingPart(String),
    /// A part to be added exists already.
    #[error("the document already has a part {0}")]
    PartExists(String),
    /// A zip entry or a part to be added has a name that a package may not
    /// use, such as one with `..` or a backslash.
    #[error("{name:?} is not a valid part name: {reason}")]
    PartName {
        /// The name.
        name: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// The file asks for more than [`Limits`] allows.
    #[error(
        "the file has {what}, more than allowed; if the file is trusted, raise Limits::{field} (now {limit})"
    )]
    Limit {
        /// What exceeded the limit.
        what: String,
        /// The [`Limits`] field to raise.
        field: &'static str,
        /// Its current value.
        limit: u64,
    },
    /// An XML part is not well-formed.
    #[error("{}{message}", location(part, *line, *column))]
    Xml {
        /// The part, when known.
        part: Option<String>,
        /// The line, from 1; 0 if unknown.
        line: usize,
        /// The column in characters, from 1; 0 if unknown.
        column: usize,
        /// What is wrong.
        message: String,
    },
    /// The file uses something this version does not handle.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// An edit would make the tree or the package invalid.
    #[error("invalid edit: {0}")]
    Edit(String),
}

fn location(part: &Option<String>, line: usize, column: usize) -> String {
    match (part, line) {
        (Some(p), 0) => format!("{p}: "),
        (Some(p), l) => format!("{p}, line {l}, column {column}: "),
        (None, 0) => String::new(),
        (None, l) => format!("line {l}, column {column}: "),
    }
}

impl Error {
    /// Names the part an XML error is in.
    pub(crate) fn in_part(self, name: &str) -> Error {
        match self {
            Error::Xml {
                part: None,
                line,
                column,
                message,
            } => Error::Xml {
                part: Some(name.to_string()),
                line,
                column,
                message,
            },
            other => other,
        }
    }
}
