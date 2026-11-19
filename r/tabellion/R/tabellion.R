#' @useDynLib tabellion, .registration = TRUE
NULL

#' Open a Word document.
#'
#' Opens a template or an existing `.docx` file. The result is a
#' document handle for the other functions; functions take it first
#' and return it invisibly, so calls pipe.
#'
#' @param path Path to a `.docx` file.
#' @return A `tabellion_doc` handle, invisibly for the other verbs.
#' @export
#' @examples
#' \dontrun{
#' doc <- open_docx("report.docx")
#' docx_blocks(doc)
#' }
open_docx <- function(path) {
  doc <- .Call("wrap__doc_open", path, PACKAGE = "tabellion")
  class(doc) <- "tabellion_doc"
  doc
}

#' @export
#' @method print tabellion_doc
print.tabellion_doc <- function(x, ...) {
  cat("<tabellion document>\n")
  invisible(x)
}

#' Write a document to a file.
#'
#' Untouched parts stay byte-identical.
#'
#' @param doc A document handle, as [`open_docx()`] makes.
#' @param path Where to write the `.docx` file.
#' @return The document, invisibly.
#' @export
save_docx <- function(doc, path) {
  .Call("wrap__doc_save", doc, path, PACKAGE = "tabellion")
  invisible(doc)
}

#' Tabulate a document's top-level blocks.
#'
#' @param doc A document handle, as [`open_docx()`] makes.
#' @return A data frame with one row per block: `id` (`p:…`, `t:…` or
#'   `#<index>`), `kind` (`paragraph`, `table`, `content-control`,
#'   `section`, `bookmark` or `other`), `style` (the style id),
#'   `level` (the outline level for headings) and `text`.
#' @export
docx_blocks <- function(doc) {
  .Call("wrap__doc_blocks", doc, PACKAGE = "tabellion")
}

#' Read a document's text.
#'
#' @param doc A document handle, as [`open_docx()`] makes.
#' @return A character vector, one element per extracted text line.
#'   Embedded line breaks split the output; a terminal line break adds
#'   no empty element.
#' @export
docx_text <- function(doc) {
  strsplit(.Call("wrap__doc_text", doc, PACKAGE = "tabellion"), "\n", fixed = TRUE)[[1]]
}

#' Add a paragraph at the end.
#'
#' @param doc A document handle, as [`open_docx()`] makes.
#' @param text The paragraph text.
#' @param style A paragraph style by id or name, or `NULL` for the
#'   body style.
#' @return The document, invisibly.
#' @export
add_paragraph <- function(doc, text, style = NULL) {
  .Call("wrap__doc_add_paragraph", doc, text, style, PACKAGE = "tabellion")
  invisible(doc)
}

#' Add Typst source at the end.
#'
#' Headings, paragraphs, lists, tables, figures and math become Word
#' content. Anything without a Word form is an error.
#'
#' @param doc A document handle, as [`open_docx()`] makes.
#' @param source Typst source.
#' @return The document, invisibly.
#' @export
add_typst <- function(doc, source) {
  .Call("wrap__doc_add_typst", doc, source, PACKAGE = "tabellion")
  invisible(doc)
}
