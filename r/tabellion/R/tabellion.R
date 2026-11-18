# The R interface. Every function that changes a document changes it in
# place and returns it invisibly, so calls can be piped.

#' Open a Word document or template.
open_docx <- function(path) {
  path <- normalizePath(path, mustWork = TRUE)
  structure(list(ptr = Docx$open(path), path = path), class = "tabellion_docx")
}

#' Write the document. Untouched parts are copied byte for byte.
save_docx <- function(doc, path) {
  doc$ptr$save(file.path(normalizePath(dirname(path), mustWork = TRUE), basename(path)))
  invisible(path)
}

#' @export
print.tabellion_docx <- function(x, ...) {
  b <- docx_blocks(x)
  cat(sprintf("<docx> %s: %d blocks, %d tables, %d headings\n", basename(x$path), nrow(b),
              sum(b$kind == "table"), sum(!is.na(b$heading_level))))
  invisible(x)
}

#' Name a place in the document: a placeholder `{{name}}`, a bookmark, a
#' heading, a block id from docx_blocks(), or the first paragraph holding
#' some text. at() alone is the end of the document.
at <- function(placeholder = NULL, bookmark = NULL, heading = NULL, block = NULL, text = NULL) {
  given <- Filter(Negate(is.null), list(placeholder = placeholder, bookmark = bookmark, heading = heading,
                                        block = block, text = text))
  if (!length(given)) return("end")
  if (length(given) > 1L) stop("name one place")
  k <- names(given); v <- given[[1]]
  if (k == "placeholder") sprintf("{{%s}}", v) else paste0(k, ":", v)
}

#' A rule (border line) for table_style(): width in points, colour as hex.
rule <- function(width = 0.5, color = "000000") list(width = width, color = sub("^#", "", color))

#' A table style defined in R. It is written into the document as a real
#' Word table style, so Word users can change it later.
table_style <- function(name, font_size = NULL, font = NULL, header = list(bold = TRUE), band_fill = NULL,
                        top = NULL, bottom = NULL, left = NULL, right = NULL, inside_h = NULL, inside_v = NULL,
                        padding = NULL, first_column = NULL) {
  hex <- function(x) if (is.null(x)) NULL else sub("^#", "", x)
  header$fill <- hex(header$fill); header$color <- hex(header$color)
  s <- list(name = name, font_size_pt = font_size, font = font, header = header, band_fill = hex(band_fill),
            top = top, bottom = bottom, left = left, right = right, inside_h = inside_h, inside_v = inside_v,
            padding_pt = padding, first_column = first_column)
  structure(Filter(Negate(is.null), s), class = "tabellion_table_style")
}

#' A table style defined in Typst: the dictionary bound to `name` in a Typst
#' file or source string, written with `table-style()` from the tabellion
#' Typst helpers.
typst_table_style <- function(source, name) {
  root <- "."
  if (length(source) == 1L && file.exists(source)) {
    root <- dirname(normalizePath(source))
    source <- paste(readLines(source, encoding = "UTF-8"), collapse = "\n")
  }
  s <- jsonlite::fromJSON(typst_table_style_json(source, name, root), simplifyVector = FALSE)
  structure(s, class = "tabellion_table_style")
}

format_column <- function(x, digits) {
  out <- if (is.integer(x)) {
    as.character(x)
  } else if (is.numeric(x)) {
    if (is.null(digits)) format(x, trim = TRUE, scientific = FALSE) else formatC(x, format = "f", digits = digits)
  } else {
    as.character(x)
  }
  out[is.na(x)] <- ""
  out
}

#' Add a data frame as a Word table.
#'
#' `style` is the name of a table style in the document (for example one of
#' the template's), or a definition from table_style() or
#' typst_table_style(). With `caption`, the table gets a numbered caption
#' that Word updates, and `label` makes it a target for cross-references.
add_table <- function(doc, data, style = NULL, at = "end", mode = NULL, caption = NULL, label = NULL,
                      author = NULL, digits = NULL) {
  data <- as.data.frame(data, check.names = FALSE)
  cols <- lapply(data, format_column, digits = digits)
  numeric <- vapply(data, is.numeric, NA)
  style_name <- ""; style_def <- ""
  if (inherits(style, "tabellion_table_style")) {
    style_def <- as.character(jsonlite::toJSON(unclass(style), auto_unbox = TRUE, null = "null"))
  } else if (!is.null(style)) {
    style_name <- style
  }
  doc$ptr$add_table(names(data), unname(cols), unname(numeric), style_name, style_def, at, mode %||% "",
                    caption %||% "", label %||% "", author %||% "")
  invisible(doc)
}

#' Add a plot as a captioned figure. `plot` is a ggplot object or a function
#' that draws with base graphics. Width and height are in centimetres.
add_plot <- function(doc, plot, width = 16, height = 9, dpi = 300, caption = NULL, label = NULL, alt = "",
                     at = "end", mode = NULL, author = NULL) {
  png_file <- tempfile(fileext = ".png")
  on.exit(unlink(png_file))
  grDevices::png(png_file, width = width, height = height, units = "cm", res = dpi)
  if (is.function(plot)) plot() else print(plot)
  grDevices::dev.off()
  doc$ptr$add_image(png_file, width, alt, at, mode %||% "", caption %||% "", label %||% "", author %||% "")
  invisible(doc)
}

#' Add Typst content, for example generated in R. Headings, lists, tables,
#' figures, footnotes, references and equations become native Word content in
#' the document's styles. Anything else becomes a picture and a warning.
add_typst <- function(doc, source, at = "end", mode = NULL, author = NULL, root = ".") {
  w <- doc$ptr$add_typst(paste(source, collapse = "\n"), normalizePath(root), at, mode %||% "", author %||% "")
  for (x in w) warning(x, call. = FALSE)
  invisible(doc)
}

#' The document's blocks: id, kind, style, heading level and text.
docx_blocks <- function(doc) {
  b <- jsonlite::fromJSON(doc$ptr$blocks_json(), simplifyVector = TRUE)
  for (k in c("style_id", "style_name", "heading_level")) if (is.null(b[[k]])) b[[k]] <- NA
  b[, c("id", "kind", "style_id", "style_name", "heading_level", "text")]
}

#' Every table as a data frame; the first row becomes the column names and
#' columns that read as numbers become numbers.
docx_tables <- function(doc, header = TRUE, convert = TRUE) {
  t <- jsonlite::fromJSON(doc$ptr$tables_json(), simplifyVector = FALSE)
  out <- lapply(t, function(x) {
    rows <- lapply(x$rows, unlist)
    n <- max(lengths(rows))
    m <- do.call(rbind, lapply(rows, function(r) c(r, rep("", n - length(r)))))
    if (header && nrow(m) > 1) {
      df <- as.data.frame(m[-1, , drop = FALSE], stringsAsFactors = FALSE)
      names(df) <- make.unique(m[1, ])
    } else {
      df <- as.data.frame(m, stringsAsFactors = FALSE)
    }
    if (convert) df[] <- lapply(df, utils::type.convert, as.is = TRUE)
    df
  })
  names(out) <- vapply(t, `[[`, "", "block")
  out
}

`%||%` <- function(a, b) if (is.null(a)) b else a
