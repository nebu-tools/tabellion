//! Document handles for R: open, save, read back and append content.
//!
//! The plain functions below work on [`Document`] values so they stay
//! testable without R; the `#[extendr]` wrappers translate them to R
//! objects for the package to call.

use std::cell::RefCell;

use extendr_api::prelude::*;
use extendr_ffi::{R_ExternalPtrProtected, R_ExternalPtrTag, R_SetExternalPtrProtected};
use nebu_docx::content::{Block, Inline};
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

extendr_module! {
    mod tabellion;
    fn doc_open;
    fn doc_save;
    fn doc_blocks;
    fn doc_text;
    fn doc_add_paragraph;
    fn doc_add_typst;
}

#[cfg(test)]
mod tests {
    use super::*;
    use extendr_api::prelude::{GetSexp, Robj};

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

    /// A foreign external pointer is rejected with an error, never
    /// dereferenced: only [`doc_open`] output passes [`checked_handle`].
    #[test]
    fn rejects_foreign_handles() {
        use extendr_ffi::{R_MakeExternalPtr, R_NilValue};
        extendr_api::test! {
            // Not an external pointer at all.
            let err = checked_handle(&Robj::from(42)).unwrap_err();
            assert!(matches!(err, Error::Docx(_)), "{err:?}");
            // A foreign pointer: an address no `Box<dyn Any>` lives at,
            // with no tag and no marker. Checking it must not touch the
            // address, so this cannot crash.
            let foreign = unsafe {
                Robj::from_sexp(R_MakeExternalPtr(
                    0x1234 as *mut std::ffi::c_void,
                    R_NilValue,
                    R_NilValue,
                ))
            };
            let err = checked_handle(&foreign).unwrap_err();
            assert!(matches!(err, Error::Docx(_)), "{err:?}");
            // The right tag but the wrong marker is still rejected.
            let tag = Robj::from(std::any::type_name::<RefCell<Document>>());
            let wrong_prot = unsafe {
                Robj::from_sexp(R_MakeExternalPtr(
                    0x1234 as *mut std::ffi::c_void,
                    tag.get(),
                    R_NilValue,
                ))
            };
            let err = checked_handle(&wrong_prot).unwrap_err();
            assert!(matches!(err, Error::Docx(_)), "{err:?}");
            // A null address with the right tag and marker fails cleanly.
            let marker = Robj::from(HANDLE_MARKER);
            let nulled = unsafe {
                Robj::from_sexp(R_MakeExternalPtr(
                    std::ptr::null_mut(),
                    tag.get(),
                    marker.get(),
                ))
            };
            let err = checked_handle(&nulled).unwrap_err();
            assert!(matches!(err, Error::Docx(_)), "{err:?}");
        }
    }

    /// What [`doc_open`] makes passes [`checked_handle`].
    #[test]
    fn accepts_own_handles() {
        extendr_api::test! {
            let bytes = sample();
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("in.docx");
            std::fs::write(&input, &bytes).unwrap();
            let handle = doc_open(input.to_str().unwrap()).unwrap();
            let back = checked_handle(handle.as_robj()).unwrap();
            assert_eq!(back.borrow().sha256(), handle.borrow().sha256());
        }
    }
}
