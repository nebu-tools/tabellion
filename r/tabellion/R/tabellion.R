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

#' Read a document's tables.
#'
#' @param doc A document handle, as [`open_docx()`] makes.
#' @return A list of data frames, one per table in document order; a
#'   table nested in a cell is its own entry. Each frame holds the grid
#'   text row by row, with `NA` where a merged cell covers the grid.
#'   The first row is data, not names; columns are named by position
#'   (`V1`, `V2`, ...). Empty cells read back as empty strings.
#' @export
docx_tables <- function(doc) {
  .Call("wrap__doc_tables", doc, PACKAGE = "tabellion")
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

#' Recompute a document's field results.
#'
#' Recomputes `SEQ` fields and bare `REF` fields (those without switches)
#' in the document body. Other field results, including page numbers,
#' dates, and `REF` fields with switches, are left unchanged.
#'
#' @param doc A document handle, as [`open_docx()`] makes.
#' @return The document, invisibly.
#' @export
update_fields <- function(doc) {
  .Call("wrap__doc_update_fields", doc, PACKAGE = "tabellion")
  invisible(doc)
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

#' Add a data frame as a table at the end.
#'
#' The column names become the header row, which repeats on each page.
#' Columns keep their order; every value becomes text, and missing
#' values stay empty.
#'
#' @param doc A document handle, as [`open_docx()`] makes.
#' @param data A data frame with at least one column. Columns become
#'   text with `as.character()`: each column must yield one string per
#'   row, so nested data frames are rejected. Columns with their own
#'   `as.character` methods are taken at face value.
#' @param caption The caption text after the label and number, or `NULL`.
#' @param label A label that cross-references name; needs a caption, or
#'   `NULL`.
#' @return The document, invisibly.
#' @export
add_table <- function(doc, data, caption = NULL, label = NULL) {
  stopifnot(is.data.frame(data))
  # Name offenders for display, falling back to positions: names can be
  # missing or blank, and NULL names must not bypass the guards.
  describe <- function(i) {
    nm <- names(data)[i]
    if (length(nm) == 0L || is.na(nm) || nm == "") paste0("column ", i) else nm
  }
  nested <- which(vapply(data, is.data.frame, logical(1)))
  if (length(nested)) {
    stop(
      "add_table: nested data-frame column(s) have no single row text: ",
      paste(vapply(nested, describe, character(1)), collapse = ", "),
      call. = FALSE
    )
  }
  coerced <- lapply(data, as.character)
  changed <- which(lengths(coerced) != nrow(data))
  if (length(changed)) {
    stop(
      "add_table: as.character() changes the length of column(s): ",
      paste(vapply(changed, describe, character(1)), collapse = ", "),
      call. = FALSE
    )
  }
  data[] <- coerced
  .Call("wrap__doc_add_table", doc, data, caption, label, PACKAGE = "tabellion")
  invisible(doc)
}
