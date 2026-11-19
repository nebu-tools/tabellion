//! Document handles for R: open, save, read back and append content.
//!
//! The plain functions below work on [`Document`] values so they stay
//! testable without R; the `#[extendr]` wrappers translate them to R
//! objects for the package to call.

use std::cell::RefCell;

use extendr_api::prelude::*;
use extendr_ffi::{R_ExternalPtrProtected, R_ExternalPtrTag, R_SetExternalPtrProtected};
use nebu_docx::content::{Block, Cell, Inline, Row, Table};
use nebu_docx::{BlockKind, Document, Place, TextOptions, WriteOptions};
use thiserror::Error;

/// A fallible result with a one-line error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// The document or the content is not valid.
    #[error("{0}")]
    Docx(String),
    /// Typst source has no content form.
    #[error("{0}")]
    Typst(String),
}

impl From<nebu_docx::Error> for Error {
    fn from(e: nebu_docx::Error) -> Self {
        Error::Docx(e.to_string())
    }
}

impl From<extendr_api::Error> for Error {
    fn from(e: extendr_api::Error) -> Self {
        Error::Docx(format!("{e:?}"))
    }
}

/// A fallible result with a one-line error.
///
/// `extendr` brings its own one-argument `Result` into scope, so the
/// standard two-argument form goes by name here.
pub type Fallible<T> = std::result::Result<T, Error>;

/// One row of `docx_blocks`: what a top-level block is and says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRow {
    /// `p:1A2B3C4D`, `t:…` or `#<index>`.
    pub id: String,
    /// `paragraph`, `table`, `content-control`, `section`, `bookmark` or
    /// `other`.
    pub kind: &'static str,
    /// The style id, if any.
    pub style: Option<String>,
    /// The outline level, 1 to 9, for headings.
    pub level: Option<u8>,
    /// The text a reader sees.
    pub text: String,
}

/// Opens a template or document.
pub fn open_docx(path: &str) -> Fallible<Document> {
    Ok(Document::open(path)?)
}

/// Writes the document; untouched parts stay byte-identical.
pub fn save_docx(doc: &Document, path: &str) -> Fallible<()> {
    Ok(doc.save(path)?)
}

/// The top-level blocks as rows: id, kind, style, level, text.
pub fn docx_blocks(doc: &mut Document) -> Fallible<Vec<BlockRow>> {
    Ok(doc
        .views()?
        .blocks()
        .iter()
        .map(|b| BlockRow {
            id: b.id.to_string(),
            kind: match b.kind {
                BlockKind::Paragraph => "paragraph",
                BlockKind::Table => "table",
                BlockKind::ContentControl => "content-control",
                BlockKind::Section => "section",
                BlockKind::Bookmark => "bookmark",
                BlockKind::Other => "other",
            },
            style: b.style.clone(),
            level: b.level,
            text: b.text.clone(),
        })
        .collect())
}

/// The text a reader sees, one line per paragraph.
pub fn docx_text(doc: &mut Document) -> Fallible<String> {
    Ok(doc.views()?.text(&TextOptions::default()))
}

/// Appends one body-style paragraph, or one in a named style.
/// Reports how many nodes were written.
pub fn add_paragraph(doc: &mut Document, text: &str, style: Option<&str>) -> Fallible<usize> {
    let blocks = vec![Block::Paragraph {
        style: style.map(str::to_string),
        align: None,
        content: vec![Inline::text(text)],
    }];
    let done = doc.insert(Place::End, &blocks, &WriteOptions::default())?;
    Ok(done.nodes.iter().map(Vec::len).sum())
}

/// Appends Typst source as Word content. Reports how many nodes were
/// written.
pub fn add_typst(doc: &mut Document, source: &str) -> Fallible<usize> {
    let content = tabellion_typst::parse(source).map_err(|e| Error::Typst(e.to_string()))?;
    let done = doc.insert(Place::End, &content.blocks, &WriteOptions::default())?;
    Ok(done.nodes.iter().map(Vec::len).sum())
}

/// A cell holding one paragraph of plain text.
fn text_cell(text: &str) -> Cell {
    Cell::new(vec![Block::Paragraph {
        style: None,
        align: None,
        content: vec![Inline::text(text)],
    }])
}

/// Appends columns of text as a table with a header row of the column
/// names. Missing cells stay empty; every column must be as long as the
/// first. Reports how many nodes were written.
pub fn add_table(
    doc: &mut Document,
    headers: &[String],
    columns: &[Vec<Option<String>>],
    caption: Option<&str>,
    label: Option<&str>,
) -> Fallible<usize> {
    if headers.is_empty() {
        return Err(Error::Docx("a table needs at least one column".to_string()));
    }
    if columns.len() != headers.len() {
        return Err(Error::Docx(
            "table columns must match the column names".to_string(),
        ));
    }
    let height = columns.first().map_or(0, Vec::len);
    if columns.iter().any(|column| column.len() != height) {
        return Err(Error::Docx(
            "table columns must all be the same length".to_string(),
        ));
    }
    let mut rows = Vec::with_capacity(height + 1);
    rows.push(Row {
        cells: headers.iter().map(|header| text_cell(header)).collect(),
    });
    for i in 0..height {
        rows.push(Row {
            cells: columns
                .iter()
                .map(|column| text_cell(column[i].as_deref().unwrap_or("")))
                .collect(),
        });
    }
    let table = Table {
        rows,
        header_rows: 1,
        first_column: false,
        style: None,
        widths: None,
        caption: caption.map(|text| vec![Inline::text(text)]),
        label: label.map(str::to_string),
    };
    let done = doc.insert(Place::End, &[Block::Table(table)], &WriteOptions::default())?;
    Ok(done.nodes.iter().map(Vec::len).sum())
}

/// A document shared with R, interior-mutable behind one handle.
pub type DocHandle = ExternalPtr<RefCell<Document>>;

/// Marks the `prot` slot of every handle this package creates.
///
/// [`ExternalPtr::try_from`] reads the address as a `Box<dyn Any>` before
/// checking its type, so converting a foreign external pointer would
/// dereference it with the wrong layout. R code cannot set this slot, so a
/// handle carrying it was made by [`doc_open`] and has that layout;
/// anything else is rejected before its address is touched.
const HANDLE_MARKER: &str = "tabellion-document-handle-v1";

/// The handle behind an R object, checked before it is touched.
///
/// Accepts only what [`doc_open`] made, by its type tag and marker.
/// Anything else is an error, so a foreign external pointer is never
/// dereferenced.
fn checked_handle(obj: &Robj) -> Fallible<DocHandle> {
    if !obj.is_external_pointer() {
        return Err(Error::Docx(
            "a document handle is an external pointer".to_string(),
        ));
    }
    // Both reads only touch the pointer's tag and protection, never its
    // address, so they are safe for foreign pointers too.
    let tag_ok = single_threaded(|| unsafe {
        Robj::from_sexp(R_ExternalPtrTag(obj.get())).as_str()
            == Some(std::any::type_name::<RefCell<Document>>())
    });
    let prot_ok = single_threaded(|| unsafe {
        Robj::from_sexp(R_ExternalPtrProtected(obj.get())).as_str() == Some(HANDLE_MARKER)
    });
    if !(tag_ok && prot_ok) {
        return Err(Error::Docx(
            "not a document handle from this package".to_string(),
        ));
    }
    Ok(DocHandle::try_from(obj)?)
}

/// Opens a template or document behind a handle.
#[extendr]
fn doc_open(path: &str) -> Fallible<DocHandle> {
    let handle = ExternalPtr::new(RefCell::new(open_docx(path)?));
    single_threaded(|| unsafe {
        R_SetExternalPtrProtected(handle.get(), Robj::from(HANDLE_MARKER).get());
    });
    Ok(handle)
}

/// Writes the document behind a handle.
#[extendr]
fn doc_save(doc: Robj, path: &str) -> Fallible<()> {
    save_docx(&checked_handle(&doc)?.borrow(), path)
}

/// The top-level blocks as a data frame.
#[extendr]
fn doc_blocks(doc: Robj) -> Fallible<Robj> {
    let rows = docx_blocks(&mut checked_handle(&doc)?.borrow_mut())?;
    let n = rows.len();
    let mut ids = Vec::with_capacity(n);
    let mut kinds = Vec::with_capacity(n);
    let mut styles: Vec<Option<String>> = Vec::with_capacity(n);
    let mut levels: Vec<Option<i32>> = Vec::with_capacity(n);
    let mut texts = Vec::with_capacity(n);
    for row in rows {
        ids.push(row.id);
        kinds.push(row.kind.to_string());
        styles.push(row.style);
        levels.push(row.level.map(i32::from));
        texts.push(row.text);
    }
    let mut frame = list!(
        id = ids,
        kind = kinds,
        style = styles,
        level = levels,
        text = texts,
    );
    frame.set_class(["data.frame"])?;
    let rownames: Vec<String> = (1..=n).map(|i| i.to_string()).collect();
    frame.set_attrib("row.names", rownames)?;
    Ok(frame.into())
}

/// The text a reader sees, one line per paragraph.
#[extendr]
fn doc_text(doc: Robj) -> Fallible<String> {
    docx_text(&mut checked_handle(&doc)?.borrow_mut())
}

/// Appends one body-style paragraph, or one in a named style.
/// Reports how many nodes were written.
#[extendr]
fn doc_add_paragraph(doc: Robj, text: &str, style: Option<&str>) -> Fallible<i32> {
    Ok(add_paragraph(&mut checked_handle(&doc)?.borrow_mut(), text, style)? as i32)
}

/// Appends Typst source as Word content. Reports how many nodes were
/// written.
#[extendr]
fn doc_add_typst(doc: Robj, source: &str) -> Fallible<i32> {
    Ok(add_typst(&mut checked_handle(&doc)?.borrow_mut(), source)? as i32)
}

/// Reads a character column, keeping missing values as missing.
fn strings_or_none(column: &Robj) -> Fallible<Vec<Option<String>>> {
    let strings = column
        .as_str_iter()
        .ok_or_else(|| Error::Docx("table columns must be character vectors".to_string()))?;
    Ok(strings
        .map(|s| if s.is_na() { None } else { Some(s.to_string()) })
        .collect())
}

/// Appends a data frame as a table with a header row of its names.
/// Missing cells stay empty. Reports how many nodes were written.
#[extendr]
fn doc_add_table(
    doc: Robj,
    frame: Robj,
    caption: Option<&str>,
    label: Option<&str>,
) -> Fallible<i32> {
    if !frame.is_list() || !frame.inherits("data.frame") {
        return Err(Error::Docx("a table needs a data frame".to_string()));
    }
    let names = frame
        .get_attrib("names")
        .ok_or_else(|| Error::Docx("a data frame has column names".to_string()))?;
    let mut headers = Vec::new();
    for name in names
        .as_str_iter()
        .ok_or_else(|| Error::Docx("a data frame has column names".to_string()))?
    {
        if name.is_na() {
            return Err(Error::Docx("column names must not be missing".to_string()));
        }
        headers.push(name.to_string());
    }
    let list = List::try_from(&frame)?;
    let mut columns = Vec::with_capacity(headers.len());
    for value in list.values() {
        columns.push(strings_or_none(&value)?);
    }
    Ok(add_table(
        &mut checked_handle(&doc)?.borrow_mut(),
        &headers,
        &columns,
        caption,
        label,
    )? as i32)
}

extendr_module! {
    mod tabellion;
    fn doc_open;
    fn doc_save;
    fn doc_blocks;
    fn doc_text;
    fn doc_add_paragraph;
    fn doc_add_typst;
    fn doc_add_table;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = root
            .ancestors()
            .nth(4)
            .expect("the workspace root")
            .join("samples")
            .join("report-template.docx");
        std::fs::read(path).expect("the sample document")
    }

    #[test]
    fn opens_reads_appends_and_saves() {
        let bytes = sample();
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.docx");
        let output = dir.path().join("out.docx");
        std::fs::write(&input, &bytes).unwrap();
        let mut doc = open_docx(input.to_str().unwrap()).unwrap();
        let before = docx_blocks(&mut doc).unwrap().len();
        assert_eq!(add_paragraph(&mut doc, "A wren sang.", None).unwrap(), 1);
        assert_eq!(add_typst(&mut doc, "= Results\n\nCounted.\n").unwrap(), 2);
        let after = docx_blocks(&mut doc).unwrap();
        assert_eq!(after.len(), before + 3, "{after:?}");
        let text = docx_text(&mut doc).unwrap();
        assert!(text.contains("A wren sang.\n"), "{text}");
        assert!(text.contains("Results\nCounted.\n"), "{text}");
        save_docx(&doc, output.to_str().unwrap()).unwrap();
        assert!(output.exists());
        // The saved document opens again with the same text.
        let mut reopened = open_docx(output.to_str().unwrap()).unwrap();
        assert_eq!(docx_text(&mut reopened).unwrap(), text);
    }

    #[test]
    fn reports_missing_files_and_bad_content() {
        assert!(open_docx("/no/such/file.docx").is_err());
        let bytes = sample();
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.docx");
        std::fs::write(&input, &bytes).unwrap();
        let mut doc = open_docx(input.to_str().unwrap()).unwrap();
        let err = add_typst(&mut doc, "#grid(columns: 2, [a])\n").unwrap_err();
        assert!(matches!(err, Error::Typst(_)), "{err:?}");
    }

    #[test]
    fn appends_tables_with_empty_missing_cells() {
        let bytes = sample();
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.docx");
        std::fs::write(&input, &bytes).unwrap();
        let mut doc = open_docx(input.to_str().unwrap()).unwrap();
        let headers = vec!["site".to_string(), "count".to_string()];
        let columns = vec![
            vec![Some("A".to_string()), Some("B".to_string())],
            vec![Some("3".to_string()), None],
        ];
        assert!(add_table(&mut doc, &headers, &columns, None, None).is_ok());
        let text = docx_text(&mut doc).unwrap();
        assert!(text.contains("site\tcount\n"), "{text}");
        assert!(text.contains("A\t3\n"), "{text}");
        assert!(text.contains("B\t\n"), "{text}");
        // No columns, mismatched names, and ragged columns are errors.
        let err = add_table(&mut doc, &[], &[], None, None).unwrap_err();
        assert!(matches!(err, Error::Docx(_)), "{err:?}");
        let short = vec![vec![Some("A".to_string())]];
        let err = add_table(&mut doc, &headers, &short, None, None).unwrap_err();
        assert!(matches!(err, Error::Docx(_)), "{err:?}");
        let ragged = vec![
            vec![Some("A".to_string()), Some("B".to_string())],
            vec![Some("3".to_string())],
        ];
        let err = add_table(&mut doc, &headers, &ragged, None, None).unwrap_err();
        assert!(matches!(err, Error::Docx(_)), "{err:?}");
    }
}
